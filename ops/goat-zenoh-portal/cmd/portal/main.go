// Command goat-zenoh-portal is the vendor self-service portal + message
// inspector for the Zenoh trial fabric.
//
// Authoritative design: docs/design/zenoh-trial-fabric.md §"Vendor
// self-service portal + inspector."
//
// Subcommands:
//
//	serve         — run the HTTP service (most users want this)
//	render-acl    — render the ACL config once and exit (one-shot)
//	healthcheck   — probe a running portal's /api/health; exit 0 healthy, 1 not
//	problem-pages — serve the constant problem+json pages the reverse proxy substitutes for its 429
//	version       — print version and exit
package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/api"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/audittail"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/kratosadmin"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/schemastore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/zenohproxy"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/web"
)

// version is the REST contract version the binary serves (internal/api).
const version = api.APIVersion

func main() {
	slog.SetDefault(slog.New(slog.NewJSONHandler(os.Stderr, &slog.HandlerOptions{
		Level: slog.LevelInfo,
	})))

	if len(os.Args) < 2 {
		usage()
		os.Exit(2)
	}
	switch os.Args[1] {
	case "serve":
		if err := cmdServe(os.Args[2:]); err != nil {
			slog.Error("serve failed", "err", err)
			os.Exit(1)
		}
	case "render-acl":
		if err := cmdRenderACL(os.Args[2:]); err != nil {
			slog.Error("render-acl failed", "err", err)
			os.Exit(1)
		}
	case "healthcheck":
		if err := cmdHealthcheck(os.Args[2:]); err != nil {
			// Plain stderr, not slog: this runs as a container healthcheck, whose
			// output docker surfaces verbatim in `inspect`. A JSON log line there
			// is noise an operator has to decode mid-incident.
			fmt.Fprintln(os.Stderr, "unhealthy: "+err.Error())
			os.Exit(1)
		}
	case "problem-pages":
		if err := cmdProblemPages(os.Args[2:]); err != nil {
			slog.Error("problem-pages failed", "err", err)
			os.Exit(1)
		}
	case "version":
		fmt.Println(version)
	case "-h", "--help", "help":
		usage()
	default:
		fmt.Fprintf(os.Stderr, "unknown subcommand %q\n\n", os.Args[1])
		usage()
		os.Exit(2)
	}
}

func usage() {
	fmt.Fprintf(os.Stderr, `goat-zenoh-portal v%s — vendor self-service portal + inspector for the Zenoh trial fabric

usage:
  goat-zenoh-portal serve       --listen ... --db ... --operator-api-key ... [flags]
  goat-zenoh-portal render-acl  --db ... --acl-config ... --acl-storage-dir-prefix ... [flags]
  goat-zenoh-portal healthcheck [--url http://127.0.0.1:8090/api/health] [--timeout 3s]
  goat-zenoh-portal problem-pages [--listen 0.0.0.0:8091]
  goat-zenoh-portal version

design: docs/design/zenoh-trial-fabric.md §Vendor self-service portal + inspector
`, version)
}

// --- serve -----------------------------------------------------------

func cmdServe(args []string) error {
	cfg, err := config.LoadFromFlags(args)
	if err != nil {
		return err
	}
	if err := cfg.Validate(); err != nil {
		return err
	}

	reg, err := registry.Open(cfg.DBDSN)
	if err != nil {
		return fmt.Errorf("open registry: %w", err)
	}
	defer reg.Close()

	// Per-uuid slot-pool mode (ADR 1025): bake the deterministic slot pool into
	// the registry once. Idempotent (no-op once populated). The salt + size MUST
	// match the router template's zenoh_slot_pool_salt / zenoh_slot_pool_size so
	// the portal's anchors equal the router's pre-baked rules. Fail fast on
	// error — a portal that can't allocate slots can't onboard anyone.
	if cfg.IdentityModel == config.IdentityModelPerUUID {
		n, err := reg.InitializeSlotPool(context.Background(), cfg.SlotPoolSalt, cfg.SlotPoolSize)
		if err != nil {
			return fmt.Errorf("initialize slot pool: %w", err)
		}
		slog.Info("per-uuid slot pool ready", "created", n, "size", cfg.SlotPoolSize)
	}

	zp := zenohproxy.New(cfg.ZenohRest, cfg.ZenohRestFake)
	minter := &certmint.Minter{
		BinaryPath:       cfg.CertSignBin,
		BundleCreatePath: cfg.BundleCreateBin,
		Fake:             cfg.CertSignFake,
		CABundleDir:      cfg.CABundleDir,
		AgeIdentityPath:  cfg.AgeIdentityPath,
		CAID:             cfg.CertSignCAID,
		SiteID:           cfg.CertSignSiteID,
		Org:              cfg.CertSignOrg,
		URIPrefix:        cfg.CertSignURIPrefix,
		CNSuffix:         cfg.CertSignCNSuffix,
		DNSSuffix:        cfg.CertSignDNSSuffix,
		CARoot:           cfg.CARootCert,
		BundleCAPubkey:   cfg.BundleCAPubkey,
		OnboardingPacket: cfg.OnboardingPacket,
		PacketName:       cfg.OnboardingPacketName,
		TarballStem:      cfg.OnboardingTarballStem,
		RootCertName:     cfg.OnboardingRootCertName,
		VendorBundleDir:  cfg.VendorBundleDir,
		RecipientKeysDir: cfg.RecipientKeysDir,
		RouterEndpoint:   cfg.RouterEndpoint,
		PortalURL:        cfg.PortalPublicURL,
		OnboardURL:       cfg.OnboardPublicURL,
		TrialSunset:      cfg.SunsetDate,
		FeedbackChannel:  cfg.FeedbackChannel,
		Lex:              cfg.Lexicon,
	}
	aclR := &acl.Renderer{
		ConfigPath:       cfg.ACLConfigPath,
		ContainerName:    cfg.ACLContainerName,
		DockerSocketPath: cfg.ACLDockerSocket,
		StorageDirPrefix: cfg.ACLStorageDirPrefix,
		ReloadFake:       cfg.ACLReloadFake,
	}
	if cfg.ACLAdminRESTURL != "" {
		aclR.AdminREST = &acl.AdminRESTClient{BaseURL: cfg.ACLAdminRESTURL}
	}
	staticFS, err := fs.Sub(web.Static, "static")
	if err != nil {
		return fmt.Errorf("embed static: %w", err)
	}

	// Schema body store (Block 85). Nil-able: when SchemaDir is unset the
	// upload + fetch endpoints respond 503; the rest of the API is
	// unaffected.
	var schemas *schemastore.Store
	if cfg.SchemaDir != "" {
		schemas, err = schemastore.New(cfg.SchemaDir)
		if err != nil {
			return fmt.Errorf("schemastore: %w", err)
		}
	}

	// Kratos admin client (83C.4 / vendor-activity admin page). Nil-able:
	// when KratosAdminURL is unset, /api/admin/vendor-activity renders
	// activity counts without the per-vendor identity-roster column and
	// notes the gap inline. No bearer auth — Kratos admin port is mesh-
	// only on the trial fabric per id-admin-console.md §Architecture.
	var kratos *kratosadmin.Client
	if cfg.KratosAdminURL != "" {
		kratos = kratosadmin.New(cfg.KratosAdminURL)
	}

	srv := api.New(cfg, reg, zp, minter, aclR, schemas, kratos, staticFS)

	httpSrv := &http.Server{
		Addr:    cfg.Listen,
		Handler: srv.Routes(),
		// Block 50A.1 timeout discipline.
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      0, // 0 because SSE long-lived streams need open writes; per-handler timeouts gate the non-SSE routes
		IdleTimeout:       120 * time.Second,
	}

	// Render the ACL once at startup so a fresh process picks up whatever's
	// in the registry. Trial-scope: this is fast (<1s) and idempotent.
	//
	// Per-uuid mode skips this entirely: the slot rules are fully baked by the
	// ansible router template (no per-vendor BEGIN/END markers exist in that
	// config), so a render would find no markers and onboarding never touches
	// the ACL. Rendering here would be a no-op at best and clobber-risk at worst.
	if cfg.IdentityModel != config.IdentityModelPerUUID {
		if err := renderACLFromRegistry(context.Background(), reg, aclR); err != nil {
			// Don't fail startup — log and continue. ACL drift surfaces in
			// the admin UI; operator-recoverable.
			slog.Warn("startup acl render failed", "err", err)
		}
	}

	// Graceful-shutdown wiring.
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	// Block 88 background workers. Both degrade to no-op when their
	// respective env vars are unset — the swimlane just shows "—" for
	// the disabled columns. The netbird poller runs on the server's shared
	// client so its periodic fetches keep the access-COP peer cache warm
	// (netbirdpoll.PeersForAccess), decoupling the access path from mgmt latency.
	srv.NetbirdClient().Run(ctx, reg, parseInterval(cfg.NetbirdPollInterval, 60*time.Second))

	// Operator keys are re-read from their on-host directory while the portal
	// runs, so issuing or revoking one is an ordinary operation rather than an
	// envrc edit plus a redeploy. Logged by HANDLE — an operator reading this
	// line needs to know who gained or lost access, and the key itself must
	// never reach a log. A read failure keeps the last good set and says so.
	warnShadowedOperatorKeys(slog.Default(), cfg.OperatorKeys)
	if cfg.OperatorKeys != nil {
		if every := parseInterval(cfg.OperatorAPIKeysReloadInterval, 30*time.Second); every > 0 && cfg.OperatorAPIKeysDir != "" {
			slog.Info("operator-key directory is live",
				"dir", cfg.OperatorAPIKeysDir, "every", every, "handles", len(cfg.OperatorKeys.Handles()))
			go cfg.OperatorKeys.Watch(ctx, every,
				func(added, removed []string) {
					slog.Info("operator keys changed", "granted", added, "revoked", removed,
						"handles", len(cfg.OperatorKeys.Handles()))
				},
				func(err error) {
					slog.Error("operator-key reload failed; SERVING THE PREVIOUS SET", "err", err,
						"handles", len(cfg.OperatorKeys.Handles()))
				})
		} else {
			slog.Info("operator keys are frozen at boot", "reason", "reload disabled or no directory configured")
		}
	}

	tailer := audittail.New(audittail.Config{
		Endpoint:     cfg.AuditS3Endpoint,
		Bucket:       cfg.AuditS3Bucket,
		Region:       cfg.AuditS3Region,
		AccessKey:    cfg.AuditS3AccessKey,
		SecretKey:    cfg.AuditS3SecretKey,
		SubscriberID: cfg.AuditSubscriberID,
		PollInterval: parseInterval(cfg.AuditTailInterval, 30*time.Second),
	}, reg)
	tailer.Run(ctx)

	// ADR 1102 governance cutover. No-op while the goatnet is bootstrap-open
	// (the kick-start default); once flipped fail-closed it runs the one-time
	// existing-orgs all-members migration (marker-guarded) then seeds the first
	// member of any new zero-admin org, re-checking on the interval so a freshly
	// onboarded org is governed without a restart. See internal/api/govern_seed.go.
	srv.RunGovernReconcile(ctx, parseInterval(cfg.GovernReconcileInterval, 5*time.Minute))

	errCh := make(chan error, 1)
	go func() { errCh <- httpSrv.ListenAndServe() }()
	slog.Info("portal listening",
		"addr", cfg.Listen,
		"zenoh_rest", cfg.ZenohRest,
		"zenoh_rest_fake", cfg.ZenohRestFake,
		"cert_sign_fake", cfg.CertSignFake,
		"acl_reload_fake", cfg.ACLReloadFake,
	)
	select {
	case <-ctx.Done():
		slog.Info("shutdown signal received")
	case err := <-errCh:
		if !errors.Is(err, http.ErrServerClosed) {
			return err
		}
	}
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	return httpSrv.Shutdown(shutdownCtx)
}

// --- healthcheck -----------------------------------------------------

// cmdHealthcheck probes a running portal's /api/health and returns nil only
// when it answers 200 with status "ok".
//
// It exists because the runtime image is distroless
// (the runtime-cc base — no shell, no curl, no wget), so the usual
// `wget -q -O- .../api/health` healthcheck is unrunnable: F-173 records the
// result of writing one into a distroless image — the container reports
// permanently unhealthy while serving fine, which is worse than no healthcheck
// at all. The binary is the only executable in the image, so it probes itself.
//
// Compose:
//
//	healthcheck:
//	  test: ["CMD", "/usr/local/bin/goat-zenoh-portal", "healthcheck",
//	         "--url", "http://127.0.0.1:8090/api/health"]
func cmdHealthcheck(args []string) error {
	fs := flag.NewFlagSet("healthcheck", flag.ContinueOnError)
	url := fs.String("url", "http://127.0.0.1:8090/api/health", "health endpoint to probe")
	timeout := fs.Duration("timeout", 3*time.Second, "probe timeout")
	if err := fs.Parse(args); err != nil {
		return err
	}

	ctx, cancel := context.WithTimeout(context.Background(), *timeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, *url, nil)
	if err != nil {
		return fmt.Errorf("bad --url %q: %w", *url, err)
	}
	resp, err := (&http.Client{Timeout: *timeout}).Do(req)
	if err != nil {
		return fmt.Errorf("GET %s: %w", *url, err)
	}
	defer func() { _ = resp.Body.Close() }()

	body, _ := io.ReadAll(io.LimitReader(resp.Body, 8<<10))
	var payload struct {
		Status string `json:"status"`
		Reason string `json:"reason"`
	}
	_ = json.Unmarshal(body, &payload) // a non-JSON body just leaves both empty

	if resp.StatusCode != http.StatusOK {
		if payload.Reason != "" {
			return fmt.Errorf("%s returned HTTP %d: %s", *url, resp.StatusCode, payload.Reason)
		}
		return fmt.Errorf("%s returned HTTP %d", *url, resp.StatusCode)
	}
	// A 200 whose payload does not say "ok" is treated as unhealthy: the status
	// field is what the endpoint asserts about itself, and trusting the code
	// alone is how a body that says "unhealthy" gets reported as healthy.
	if payload.Status != "ok" {
		if payload.Reason != "" {
			return fmt.Errorf("%s returned 200 but status=%q: %s", *url, payload.Status, payload.Reason)
		}
		return fmt.Errorf("%s returned 200 but status=%q (want \"ok\")", *url, payload.Status)
	}
	return nil
}

// --- render-acl ------------------------------------------------------

func cmdRenderACL(args []string) error {
	cfg, err := config.LoadFromFlags(args)
	if err != nil {
		return err
	}
	// render-acl doesn't need the full validation surface — DB + ACL
	// config + storage dir are enough.
	if err := cfg.RequireDSN(); err != nil {
		return err
	}
	if cfg.ACLConfigPath == "" || cfg.ACLStorageDirPrefix == "" {
		return errors.New("render-acl requires --db-dsn + --acl-config + --acl-storage-dir-prefix")
	}
	reg, err := registry.Open(cfg.DBDSN)
	if err != nil {
		return err
	}
	defer reg.Close()
	aclR2 := &acl.Renderer{
		ConfigPath:       cfg.ACLConfigPath,
		ContainerName:    cfg.ACLContainerName,
		DockerSocketPath: cfg.ACLDockerSocket,
		StorageDirPrefix: cfg.ACLStorageDirPrefix,
		ReloadFake:       cfg.ACLReloadFake,
	}
	if cfg.ACLAdminRESTURL != "" {
		aclR2.AdminREST = &acl.AdminRESTClient{BaseURL: cfg.ACLAdminRESTURL}
	}
	return renderACLFromRegistry(context.Background(), reg, aclR2)
}

func renderACLFromRegistry(ctx context.Context, reg *registry.Registry, aclR *acl.Renderer) error {
	ps, err := reg.ApprovedVendorPrefixes(ctx)
	if err != nil {
		return fmt.Errorf("query approved vendors: %w", err)
	}
	return aclR.Render(ctx, ps)
}

// parseInterval reads a duration string and falls back to the provided
// default when empty or unparseable. Used for background-worker
// intervals where a malformed flag should not crash the portal.
func parseInterval(s string, dflt time.Duration) time.Duration {
	if s == "" {
		return dflt
	}
	d, err := time.ParseDuration(s)
	if err != nil || d <= 0 {
		return dflt
	}
	return d
}

// warnShadowedOperatorKeys logs one WARN line per handle whose directory key is
// overridden by an env key (PORTAL_OPERATOR_API_KEYS). Revoking such a handle
// in the console has no effect while the env pin exists, so the condition must
// be in the startup log rather than discovered after a revocation fails. The
// handle is logged; the key never is. Which key wins is not changed here:
// removing an env pin is a deployment decision.
func warnShadowedOperatorKeys(logger *slog.Logger, store *config.OperatorKeyStore) {
	if store == nil {
		return
	}
	for _, handle := range store.Shadowed() {
		logger.Warn("operator key in the directory is shadowed by an env key; console revocation of this handle has no effect until the env pin is removed",
			"handle", handle, "env", "PORTAL_OPERATOR_API_KEYS")
	}
}
