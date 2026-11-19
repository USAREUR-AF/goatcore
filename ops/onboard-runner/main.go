// Command onboard-runner is the pull-model auto-executor for the three-stage
// candidate onboarding flow (docs/design/operator-onboard-intake.md §"Fast-follow").
// It runs on a trusted sovereign host (the site Thor), polls the goat-zenoh-portal
// for operator-approved onboarding proposals, atomically claims each, and runs the
// native `goat admin onboard` (~3s, all three surfaces) — so "admin approves → it
// just happens", with the portal staying credential-free.
//
// Credentials (in the process env, NOT flags): ONBOARD_RUNNER_PORTAL_KEY (the
// operator-tier bearer for the portal contract) + whatever `goat admin onboard`
// itself needs (per-env NetBird API token, SSH to the site Thor, the portal
// operator key for the cert mint). This binary is a mint-anything capability — run
// it only on a tightly-controlled host, least-privilege per-env tokens, audited.
//
// Start SAFE: `--dry-run` polls + logs what it would onboard, but never claims,
// mints, or writes back — prove connectivity + auth + the eligible set first, then
// drop --dry-run.
package main

import (
	"context"
	"flag"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"
)

func main() {
	var (
		portalURL   = flag.String("portal", envOr("PORTAL_URL", ""), "portal base URL for the cert-mint leg (goat profile portal_endpoint) — the Zenoh cert/slot registry")
		onboardURL  = flag.String("onboard", envOr("ONBOARD_URL", ""), "onboarding-contract base URL (goat-id-console). Empty ⇒ fall back to --portal (pre-extraction: the proposal store lived in the portal)")
		repoRoot    = flag.String("repo-root", ".", "repo root holding ops/operators/ + the manifest schema")
		goatBin     = flag.String("goat-bin", "goat", "path to the goat binary")
		runnerID    = flag.String("runner-id", "onboard-runner", "GOAT_OPERATOR_HANDLE stamped into the mint audit event")
		auditFB     = flag.String("audit-fallback", envOr("AUDIT_FALLBACK_PATH", ""), "where an operator-action record the audit fabric refused is appended (\"\" = <repo-root>/out/audit/operator-actions.log)")
		onlyEnv     = flag.String("only-env", envOr("ONLY_ENV", ""), "only process proposals for this env (\"\"=all)")
		interval    = flag.Duration("interval", 30*time.Second, "poll interval")
		httpTimeout = flag.Duration("http-timeout", 25*time.Second, "per-request portal timeout (lossy-mesh tolerant)")
		once        = flag.Bool("once", false, "run a single poll cycle then exit (cron/testing)")
		dryRun      = flag.Bool("dry-run", false, "poll + log only; never claim, mint, or write back")
		collabOp    = flag.String("collab", "", "operator collab-account op instead of polling: status|revoke|list|reconcile|backfill (see collabctl.go)")
		collabHnd   = flag.String("handle", "", "with -collab status|revoke: the person handle")
		collabYes   = flag.Bool("yes", false, "with -collab revoke: confirm cutting a real person's collab access; with -collab backfill: write the manifests (default is a dry run)")
		collabJSON  = flag.Bool("json", false, "with -collab reconcile: emit the report as JSON instead of the human table")
		collabEmit  = flag.String("emit-backfill", "", "with -collab reconcile: also write the durable-record patch set to this path")
		collabFile  = flag.String("file", "", "with -collab backfill: the patch set to apply")
	)
	flag.Parse()

	log := slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelInfo}))

	// Operator one-shots run before any onboarding-contract wiring: they need
	// only the collab provider's own config (and, for reconcile/backfill, the
	// repo root holding the manifests + roster), never a portal/console bearer.
	if *collabOp != "" {
		os.Exit(runCollabOp(collabOpts{
			op: *collabOp, handle: *collabHnd, repoRoot: *repoRoot,
			emitBackfill: *collabEmit, file: *collabFile,
			jsonOut: *collabJSON, yes: *collabYes, timeout: *httpTimeout,
		}, log))
	}

	if *portalURL == "" {
		log.Error("missing --portal (or PORTAL_URL) — the cert-mint leg needs the portal endpoint")
		os.Exit(2)
	}

	// The onboarding contract (approved queue / claim / result) and the cert-mint
	// leg are two targets since the onboarding engine moved to goat-id-console
	// (docs/design/onboarding-extraction-plan.md). Prefer the console + its
	// machine-token; fall back to the portal + the runner's portal key so this
	// binary keeps working against a pre-extraction portal until the cutover.
	contractBase := *onboardURL
	contractKey := os.Getenv("ONBOARD_API_TOKEN")
	if contractBase == "" {
		contractBase = *portalURL
		contractKey = os.Getenv("ONBOARD_RUNNER_PORTAL_KEY")
	}
	if contractKey == "" && !*dryRun {
		// dry-run can read the (auth-gated) queue only if the key is present; make
		// the missing-key case a hard error for the minting path.
		log.Error("missing onboarding-contract bearer (ONBOARD_API_TOKEN for --onboard, or ONBOARD_RUNNER_PORTAL_KEY for the --portal fallback)")
		os.Exit(2)
	}

	// Build the onboarding-contract client. Default: plaintext HTTP + bearer
	// (the mgmt-bridge house style). When ONBOARD_MTLS_ENABLED is set, the leg
	// to the console is wrapped in SPIFFE mTLS (docs/design/runner-console-mtls.md):
	// the runner presents its SVID and verifies the console's SVID SPIFFE ID.
	// The bearer is still sent for per-handle attribution. Additive +
	// fail-closed — a misconfigured mTLS mode aborts rather than silently
	// falling back to plaintext on this mint-capable leg.
	var pc *portalClient
	if envBool("ONBOARD_MTLS_ENABLED") {
		hc, err := mtlsClient(
			envOr("ONBOARD_MTLS_CERT_FILE", "/svid/onboard-runner-cert.pem"),
			envOr("ONBOARD_MTLS_KEY_FILE", "/svid/onboard-runner-key.pem"),
			envOr("ONBOARD_MTLS_BUNDLE_FILE", "/svid/spire-bundle.pem"),
			splitCommaEnv("ONBOARD_MTLS_SERVER_ID"),
			*httpTimeout,
		)
		if err != nil {
			log.Error("onboarding-contract mTLS setup failed", "err", err)
			os.Exit(2)
		}
		log.Info("onboarding-contract leg is SPIFFE mTLS", "expected_server_id", os.Getenv("ONBOARD_MTLS_SERVER_ID"))
		pc = newPortalClientWithHTTP(contractBase, contractKey, hc)
	} else {
		pc = newPortalClient(contractBase, contractKey, *httpTimeout)
	}

	// Optional collaboration-environment leg. Nil unless COLLAB_PROVISION_ENABLED
	// is set with a reachable collab identity plane — most goatnets have no collab
	// env and the leg simply no-ops. See docs/design/collab-onboarding-provisioner.md.
	collab := newCollabProviderFromEnv(*httpTimeout, log)

	r := &runner{
		pc:       pc,
		onlyEnv:  *onlyEnv,
		repoRoot: *repoRoot,
		goatBin:  *goatBin,
		runnerID: *runnerID,
		dryRun:   *dryRun,
		// Offboard operator-action records the fabric refuses land here.
		auditFallbackPath: *auditFB,
		// Mint-on-collect, per goatnet. Off unless the deploy explicitly turns it
		// on, so a goatnet whose console does not yet serve the collection page
		// keeps minting at onboard rather than stranding its enrollees.
		deferCredential: envBool("ONBOARD_DEFER_CREDENTIAL"),
		log:             log,
		collab:          collab,
		// The management API as THIS container resolves it — proven before any
		// claim. Same value the native mint calls.
		mgmtURL:  os.Getenv("NETBIRD_MGMT_URL"),
		mgmtHTTP: &http.Client{Timeout: *httpTimeout},
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	log.Info("onboard-runner starting",
		"portal", *portalURL, "onboard_contract", contractBase, "only_env", *onlyEnv, "runner_id", *runnerID,
		"interval", interval.String(), "once", *once, "dry_run", *dryRun)

	if *once {
		if err := r.runOnce(ctx); err != nil {
			log.Error("run failed", "err", err)
			os.Exit(1)
		}
		return
	}

	ticker := time.NewTicker(*interval)
	defer ticker.Stop()
	// Fire once immediately so a fresh start doesn't wait a full interval.
	if err := r.runOnce(ctx); err != nil {
		log.Warn("poll failed", "err", err)
	}
	for {
		select {
		case <-ctx.Done():
			log.Info("shutting down")
			return
		case <-ticker.C:
			if err := r.runOnce(ctx); err != nil {
				log.Warn("poll failed", "err", err)
			}
		}
	}
}

func envOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

func envBool(key string) bool {
	v := os.Getenv(key)
	return v == "1" || v == "true" || v == "yes"
}

// splitCommaEnv reads a comma-separated env var into a slice, dropping blanks.
func splitCommaEnv(key string) []string {
	var out []string
	for _, p := range strings.Split(os.Getenv(key), ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}
