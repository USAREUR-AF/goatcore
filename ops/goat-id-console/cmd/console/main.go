// Command goat-id-console — admin console for the Ory Kratos + Hydra
// identity plane. One front door for platform operators.
//
// Design: docs/design/id-admin-console.md
// ADR:    docs/adr/0860-id-admin-console-separate-tenant.md
// Runbook: docs/operations/id-admin-console-runbook.md
package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/enrollwatch"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/handlers"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/hydra"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/orgstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/permissions"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/spiffemtls"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/store"
)

const version = "0.1.0"

func main() {
	slog.SetDefault(slog.New(slog.NewJSONHandler(os.Stderr, &slog.HandlerOptions{
		Level: slog.LevelInfo,
	})))
	if err := run(); err != nil {
		slog.Error("fatal", slog.String("err", err.Error()))
		os.Exit(1)
	}
}

func run() error {
	cfg, err := config.Load()
	if err != nil {
		return fmt.Errorf("config: %w", err)
	}

	slog.Info("starting goat-id-console",
		slog.String("version", version),
		slog.String("listen", cfg.ListenAddr),
		slog.String("permissions_yaml", cfg.PermissionsYAMLPath),
		slog.Bool("dev_bypass_auth", cfg.DevBypassAuth),
		slog.Bool("step_up_enforced", cfg.StepUpEnforced),
	)
	if !cfg.StepUpEnforced {
		slog.Warn("AAL2 step-up enforcement is DISABLED for this goatnet (STEP_UP_ENFORCED=false)",
			slog.String("effect", "sensitive mutations (delete/revoke-admin/reset-2fa/grant-admin) proceed without a fresh second-factor check; actions are still audited"),
			slog.String("policy", "sandbox-only; prod must wire the real AAL2 flow and set STEP_UP_ENFORCED=true"),
		)
	}

	perms, err := permissions.Load(cfg.PermissionsYAMLPath)
	if err != nil {
		return fmt.Errorf("permissions: %w", err)
	}

	kratosClients := map[string]*kratos.Client{}
	hydraClients := map[string]*hydra.Client{}
	for _, key := range cfg.IdPOrder {
		idp, _ := cfg.IdP(key)
		kratosClients[key] = kratos.NewClient(idp.KratosAdminURL)
		hydraClients[key] = hydra.NewClient(idp.HydraAdminURL)
	}

	invites := store.NewMemory()
	var auditEmitter audit.Emitter
	if cfg.AuditBackend == "file" {
		fe, ferr := audit.NewFileEmitter(200, cfg.AuditFilePath)
		if ferr != nil {
			// Fail loud, not silent: an operator asked for durable audit and
			// didn't get it. Refusing to start beats quietly dropping to the
			// ring (the vanishing-trail class).
			slog.Error("AUDIT_BACKEND=file but the sink could not be opened", "path", cfg.AuditFilePath, "err", ferr)
			os.Exit(1)
		}
		auditEmitter = fe
	} else {
		auditEmitter = audit.NewStderrEmitter(200)
	}

	// Onboarding store (proposals + invites). The console owns the onboarding
	// workflow natively now (docs/design/onboarding-extraction-plan.md), so this
	// is opened unconditionally. Postgres only (ADR 1165 step A): a SQLite path
	// is refused with the way forward rather than silently ignored.
	if err := requireDSN("ONBOARD_DB_DSN", cfg.OnboardDBDSN, "ONBOARD_DB_PATH"); err != nil {
		return err
	}
	onboard, err := onboardstore.Open(cfg.OnboardDBDSN)
	if err != nil {
		return fmt.Errorf("onboard store: %w", err)
	}
	defer func() { _ = onboard.Close() }() //nolint:errcheck // process-exit cleanup cannot change a completed request
	slog.Info("onboarding store opened")

	// Org registry (ADR 1112) — its own database beside the onboarding store's.
	if err := requireDSN("ORG_DB_DSN", cfg.OrgDBDSN, "ORG_DB_PATH"); err != nil {
		return err
	}
	orgs, err := orgstore.Open(cfg.OrgDBDSN)
	if err != nil {
		return fmt.Errorf("org store: %w", err)
	}
	defer func() { _ = orgs.Close() }() //nolint:errcheck // process-exit cleanup cannot change a completed request
	slog.Info("org registry opened")

	srv, err := handlers.NewServer(cfg, kratosClients, hydraClients, invites, auditEmitter, perms, onboard)
	if err != nil {
		return fmt.Errorf("handlers: %w", err)
	}
	srv.Orgs = orgs
	// Name, once, every per-person key in ONBOARD_OPERATOR_API_KEYS the
	// machine-realm gate will refuse (GHSA-mxf4-8rv7-774w).
	srv.WarnUnrosteredOperatorKeys(context.Background())
	// Complete every review-stage draft the current rules can assemble, so the
	// queue's draft flags reflect the rules in force rather than the ones at
	// submit time. Needs the org registry above (a pinned org's nation feeds
	// the derived affiliation.kind). Failure is logged, not fatal: a stuck
	// sweep must not keep the console down, and one failed row does not stop
	// the rest, so the counts are reported alongside any failure.
	done, left, err := srv.CompleteDrafts(context.Background())
	if err != nil {
		slog.Warn("onboarding draft sweep: some rows failed", slog.Any("err", err))
	}
	if done > 0 || left > 0 {
		slog.Info("onboarding draft sweep", slog.Int("completed", done), slog.Int("still_incomplete", left))
	}

	httpServer := &http.Server{
		Addr:              cfg.ListenAddr,
		Handler:           srv.Routes(),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       15 * time.Second,
		WriteTimeout:      30 * time.Second,
		IdleTimeout:       60 * time.Second,
	}

	// Graceful shutdown plumbing.
	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
	defer cancel()

	go func() {
		ticker := time.NewTicker(time.Hour)
		defer ticker.Stop()
		for {
			if err := onboard.DeleteExpiredMeshMoveReviews(ctx, time.Now()); err != nil && ctx.Err() == nil {
				slog.Warn("could not delete expired mesh move reviews", "error", err)
			}
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
			}
		}
	}()

	// Enrollment watcher (internal/enrollwatch): the one OBSERVED transition in
	// the onboarding lifecycle — onboarded → enrolled, when the person actually
	// uses the mesh credential we minted. Default-off: no token ⇒ Run returns
	// immediately and every row stays at `onboarded`, exactly as before.
	enrollWatcher := &enrollwatch.Watcher{
		Store:    onboard,
		MgmtURL:  cfg.NotifyMgmtURL,
		Token:    cfg.EnrollWatchToken,
		Interval: cfg.EnrollWatchInterval,
		Audit:    auditEmitter,
	}
	if enrollWatcher.Enabled() {
		go enrollWatcher.Run(ctx)
	} else {
		slog.Info("enrollment watcher disabled (no ENROLL_WATCH_NB_TOKEN); proposals stay at `onboarded`")
	}

	// Stale-invite sweep: file revoked/expired invites into the folded archive
	// once they have been dead for InviteArchiveAfter (7d default).
	//
	// Deliberately NOT a job inside the enrollment watcher, even though both are
	// periodic. That watcher needs a NetBird token to run at all, gives up early
	// when the API is unreachable, and returns before doing anything when no
	// proposal is pending — so an invite sweep hung off it would quietly stop
	// running in three situations that have nothing to do with invites. This
	// touches only the console's own SQLite.
	if cfg.InviteArchiveAfter > 0 {
		// Say the sweep is armed, once, at boot. Without this line the healthy
		// steady state — nothing stale, nothing moved — is INDISTINGUISHABLE
		// from the sweep never having started, because the only other output is
		// per-archived-row. That ambiguity is how a dead guard goes unnoticed
		// for months; an operator reading the log must be able to see the lane
		// exists before they trust an empty result from it.
		slog.Info("stale-invite sweep armed",
			slog.Duration("archive_after", cfg.InviteArchiveAfter),
			slog.String("moves", "revoked or expired invites -> archived"))
		go func() {
			// Hourly is far finer than a 7-day window needs; it just means a
			// long-running console reaches the boundary within the hour instead
			// of at the next restart.
			const sweepEvery = time.Hour
			sweep := func() {
				moved, err := onboard.ArchiveStaleInvites(ctx, cfg.InviteArchiveAfter)
				if err != nil {
					slog.Warn("stale-invite sweep failed", slog.Any("error", err))
					// Fall through: a partial sweep still reports what it moved.
				}
				for _, inv := range moved {
					slog.Info("invite archived",
						slog.String("invite_id", inv.ID),
						slog.String("sponsor", inv.Sponsor),
						slog.String("env", inv.Env))
				}
				slog.Info("stale-invite sweep pass",
					slog.Int("archived", len(moved)),
					slog.Duration("dead_for_at_least", cfg.InviteArchiveAfter))
			}
			sweep() // once at boot, so a restart reflects the window immediately
			t := time.NewTicker(sweepEvery)
			defer t.Stop()
			for {
				select {
				case <-ctx.Done():
					return
				case <-t.C:
					sweep()
				}
			}
		}()
	} else {
		slog.Info("stale-invite sweep disabled (ONBOARD_INVITE_ARCHIVE_AFTER=off); dead invites stay on the working list")
	}

	// Stale-claim requeue sweep (goat#977): a claim a dead runner abandoned has
	// no other exit, so this puts it back on the runner's queue. See
	// RequeueStaleClaims.
	if cfg.ClaimRequeueAfter > 0 {
		// Announce at boot so a healthy empty steady state is distinguishable
		// from the sweep never starting (same as the invite sweep).
		slog.Info("stale-claim requeue sweep armed",
			slog.Duration("requeue_after", cfg.ClaimRequeueAfter),
			slog.String("moves", "onboarding -> approved, offboarding -> offboard-requested"))
		go func() {
			// Minute granularity: the window is minutes, not days.
			const sweepEvery = time.Minute
			sweep := func() {
				moved, err := onboard.RequeueStaleClaims(ctx, cfg.ClaimRequeueAfter)
				if err != nil {
					slog.Warn("stale-claim requeue sweep failed", slog.Any("error", err))
				}
				for _, p := range moved {
					slog.Info("stale claim re-queued",
						slog.String("proposal_id", p.ID),
						slog.String("env", p.Env),
						slog.String("status", p.Status))
					action := "onboard_requeued"
					before := onboardstore.ProposalOnboarding
					if p.Status == onboardstore.ProposalOffboardRequested {
						action = "offboard_requeued"
						before = onboardstore.ProposalOffboarding
					}
					auditEmitter.Emit(ctx, audit.Event{
						Actor:  audit.Actor{Email: "claim-requeue-sweep"},
						Action: action,
						Target: audit.Target{Email: p.Email},
						Before: map[string]interface{}{"status": before},
						After: map[string]interface{}{"status": p.Status,
							"proposal_id": p.ID, "env": p.Env, "note": p.Note},
						Result: "ok",
					})
				}
				slog.Info("stale-claim requeue sweep pass",
					slog.Int("requeued", len(moved)),
					slog.Duration("unreported_for_at_least", cfg.ClaimRequeueAfter))
			}
			sweep() // once at boot, so a restart reflects the window immediately
			t := time.NewTicker(sweepEvery)
			defer t.Stop()
			for {
				select {
				case <-ctx.Done():
					return
				case <-t.C:
					sweep()
				}
			}
		}()
	} else {
		slog.Info("stale-claim requeue sweep disabled (ONBOARD_CLAIM_REQUEUE_AFTER=off); a claim a runner never reports back on stays stuck")
	}

	errCh := make(chan error, 1)
	go func() {
		slog.Info("http listening", slog.String("addr", httpServer.Addr))
		if err := httpServer.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			errCh <- err
		}
	}()

	// Additive mTLS listener for the mint-capable onboarding leg (runner →
	// console), default OFF. Same routes; transport auth is the client SVID so
	// the console authenticates the runner cryptographically rather than by
	// network position + a static bearer (the bearer still carries F-207
	// per-handle attribution). Fail-closed: a config error aborts startup rather
	// than silently leaving the leg on bearer-only.
	var mtlsServer *http.Server
	if cfg.MTLSOnboardEnabled {
		tlsCfg, err := spiffemtls.ServerConfig(
			cfg.MTLSOnboardCertFile, cfg.MTLSOnboardKeyFile, cfg.MTLSOnboardBundleFile,
			cfg.MTLSOnboardAllowedClientIDs,
		)
		if err != nil {
			return fmt.Errorf("mTLS onboarding listener: %w", err)
		}
		mtlsServer = &http.Server{
			Addr:              cfg.MTLSOnboardAddr,
			Handler:           srv.Routes(),
			TLSConfig:         tlsCfg,
			ReadHeaderTimeout: 5 * time.Second,
			ReadTimeout:       15 * time.Second,
			WriteTimeout:      30 * time.Second,
			IdleTimeout:       60 * time.Second,
		}
		go func() {
			slog.Info("mTLS onboarding listener",
				slog.String("addr", mtlsServer.Addr),
				slog.Any("allowed_client_spiffe_ids", cfg.MTLSOnboardAllowedClientIDs))
			// Certs come from the TLSConfig (SVID files), so ListenAndServeTLS
			// takes empty cert/key paths.
			if err := mtlsServer.ListenAndServeTLS("", ""); err != nil && !errors.Is(err, http.ErrServerClosed) {
				errCh <- err
			}
		}()
	}

	select {
	case <-ctx.Done():
		slog.Info("shutdown signal received")
	case err := <-errCh:
		return err
	}

	shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if mtlsServer != nil {
		if err := mtlsServer.Shutdown(shutdownCtx); err != nil {
			slog.Warn("mTLS listener shutdown", slog.String("err", err.Error()))
		}
	}
	if err := httpServer.Shutdown(shutdownCtx); err != nil {
		return fmt.Errorf("shutdown: %w", err)
	}
	slog.Info("http stopped cleanly")
	return nil
}

// requireDSN refuses to start without a store's Postgres DSN. When only the
// retired SQLite path variable is set, the error says how to move the data.
func requireDSN(dsnKey, dsn, legacyPathKey string) error {
	if dsn != "" {
		return nil
	}
	if os.Getenv(legacyPathKey) != "" {
		return fmt.Errorf("%s is not set and %s is: the console now stores this data in Postgres only (ADR 1165); import the SQLite file with `store-migrate import` and set %s", dsnKey, legacyPathKey, dsnKey)
	}
	return fmt.Errorf("%s is not set: the console needs a Postgres database for this store (ADR 1165)", dsnKey)
}
