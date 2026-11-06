// Package handlers wires the HTTP routes for goat-id-console.
//
// Design contract: docs/design/id-admin-console.md + ADR 0860.
//
// Every request flows through:
//
//  1. auth middleware — reads X-Forwarded-{Email,Groups,User} from
//     oauth2-proxy (the unforgeable, sidecar-set family; NOT X-Auth-Request-*,
//     which is client-forgeable in reverse-proxy mode — F-244), and the
//     matrix-driven adminGate refuses callers no group grants. In dev (env
//     DEV_BYPASS_AUTH=1) this is short-circuited.
//  2. idp-selector middleware — reads ?idp=<key> (default tenant) and
//     stuffs the resolved config.IdP into the request context. All
//     Kratos / Hydra calls pick up the IdP from context.
//  3. handler.
//
// Sensitive mutations (per permissions YAML step_up_required) check the
// actor's AAL — refuse with 403 if not aal2 within freshness window. v1
// does not implement the redirect-through-Kratos AAL2 flow; the operator
// re-authenticates manually and retries. v1.1 hardening.
package handlers

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"html/template"
	"io/fs"
	"log/slog"
	"net/http"
	"net/mail"
	"net/url"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/coalition"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/hydra"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kanidmdoor"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/mailer"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/meshstate"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/orgstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/permissions"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/sesapi"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/store"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/welcome"
	"github.com/usareur-af/goatcore/ops/goat-id-console/web"
)

// Server is the HTTP entrypoint. Wires together the per-IdP Kratos and
// Hydra clients, the invite store, and the audit emitter.
type Server struct {
	Cfg         *config.Config
	Kratos      map[string]*kratos.Client // keyed by IdP.Key
	Hydra       map[string]*hydra.Client  // keyed by IdP.Key
	Invites     *store.Memory
	Audit       audit.Emitter
	Permissions *permissions.Matrix
	Templates   *template.Template
	Onboard     *onboardstore.Store // the local onboarding store (proposals + invites); owns the workflow natively
	// Orgs is the first-class org registry (ADR 1112). Set by main.go after
	// NewServer (and by tests that exercise the org-registry routes); nil →
	// the org-registry endpoints answer 503 fail-closed.
	Orgs *orgstore.Store
	// Coalition is the framework registry backing the identity-detail
	// coalition-standing row. Nil when the file is absent or unparseable — the
	// page then says so (CoalitionLoadErr) rather than rendering a standing it
	// cannot justify.
	Coalition        *coalition.Registry
	CoalitionLoadErr string
	// CollabDoor and PublicDoor mint credential-setup codes on the two Kanidm
	// sign-on doors, for the /setup-code self-service page. Nil-safe: a goatnet
	// without a collab plane leaves CollabDoor unconfigured and the page simply
	// does not offer that row.
	CollabDoor *kanidmdoor.Client
	PublicDoor *kanidmdoor.Client
	Mailer     *mailer.Mailer // SMTP (SES) sender for invite + candidate stage emails; disabled when SMTP_HOST unset
	SES        *sesapi.Client // SES v2 API (sandbox detection + recipient verification); nil = feature off
	// StepUps holds live one-shot AAL2 step-up challenges (#174). See
	// stepup.go for why the AAL signal is read from Kratos rather than a
	// forwarded claim header.
	StepUps *stepUpStore
	// runnerLateAfter is the wait after which the onboarding queue's runner
	// band calls a row late; zero means runnerLateAfter. Tests shorten
	// it so a freshly queued row renders as late without a clock.
	runnerLateAfter time.Duration
	// stepUpCache backs the DISPLAY copy of the step-up posture only (the
	// banner + badges). Gates always re-resolve live. See stepup.go.
	stepUpCacheMu sync.Mutex
	stepUpCache   map[string]cachedState
	// stepUpLocks records an operator's deliberate "lock now": the moment they
	// ended their own freshness window early. Kratos never retracts an
	// authentication_methods[] entry, so re-locking cannot be expressed in the
	// upstream ledger — it has to live here. See relockedAt in stepup.go.
	stepUpLockMu sync.Mutex
	stepUpLocks  map[string]time.Time
	csrfNonce    string // server-instance CSRF nonce; rotated on restart

	// testCompleteDraftOverride is a test-only hook: when set by tests,
	// completeDraft delegates to it so handlers can exercise the errDraftChanged
	// mapping without relying on timing races. Nil in production.
	testCompleteDraftOverride func(ctx context.Context, p *onboardstore.Proposal, by draftCompleter) (draftOutcome, string, error)

	// One-time rotate nonces (goat#580). The rotate hand-over page IS the
	// POST response, so a refresh / back-then-forward / double-click
	// re-POSTs the same form — and the static csrfNonce cannot tell that
	// replay from a fresh submission. Minted per user-detail render,
	// consumed by the rotate handler; a replay 409s instead of re-keying.
	rotateNonceMu sync.Mutex
	rotateNonces  map[string]time.Time

	// probationSnapshot is an immutable, IP-indexed NetBird snapshot for the
	// ForwardAuth path. Requests never scan or copy the fleet. Once populated,
	// an expired snapshot is refreshed asynchronously and remains usable for
	// ordinary decisions during a management outage. Explainer and petition
	// paths synchronously require a binding within the hard maximum age.
	probationMu           sync.Mutex
	probationSnapshot     *probationPeerSnapshot
	probationRefreshAfter time.Time
	probationRetryAfter   time.Time
	probationRefreshErr   error
	probationRefreshing   bool
	probationRefreshDone  chan struct{}
	// probationHoldSnapshot is indexed by site + peer and refreshed whenever
	// the path's identity, size, or modification time changes. The mutex also
	// serializes refreshes so a changed JSONL file is parsed only once.
	probationHoldMu       sync.Mutex
	probationHoldSnapshot *probationHoldSnapshot
	// Test seam; production uses meshstate.Client.Peers.
	probationPeerSource func(context.Context) ([]meshstate.Peer, error)
}

func NewServer(
	cfg *config.Config,
	kratosClients map[string]*kratos.Client,
	hydraClients map[string]*hydra.Client,
	invites *store.Memory,
	auditEmitter audit.Emitter,
	perms *permissions.Matrix,
	onboard *onboardstore.Store,
) (*Server, error) {
	tmpl, err := loadTemplates()
	if err != nil {
		return nil, err
	}
	nonce := mintNonce()
	// The framework registry is a DISPLAY input, so a load failure must not
	// refuse startup — an operator locked out of the whole console because a
	// standing row cannot render is a worse outcome than the row saying it is
	// unresolved. Keep the reason and show it on the page.
	var coalReg *coalition.Registry
	var coalErr string
	if p := strings.TrimSpace(cfg.CoalitionFrameworksPath); p != "" {
		reg, cerr := coalition.Load(p)
		if cerr != nil {
			coalErr = cerr.Error()
		} else {
			coalReg = reg
		}
	} else {
		coalErr = "COALITION_FRAMEWORKS_PATH is unset"
	}
	return &Server{
		Cfg: cfg, Kratos: kratosClients, Hydra: hydraClients,
		Invites: invites, Audit: auditEmitter, Permissions: perms,
		Templates: tmpl, csrfNonce: nonce,
		rotateNonces:     map[string]time.Time{},
		StepUps:          newStepUpStore(),
		Onboard:          onboard,
		Coalition:        coalReg,
		CoalitionLoadErr: coalErr,
		Mailer:           newMailer(cfg),
		SES:              sesapi.New(cfg.SESAPIRegion, cfg.SESAPIAccessKeyID, cfg.SESAPISecretAccessKey),
		CollabDoor:       kanidmdoor.New(cfg.CollabDoorURL, cfg.CollabDoorAdminPW),
		PublicDoor:       kanidmdoor.New(cfg.PublicDoorURL, cfg.PublicDoorAdminPW),
	}, nil
}

// loadTemplates parses every templates/*.html as a single set so the
// per-page {{define "content"}} blocks compose with layout.html.
func loadTemplates() (*template.Template, error) {
	entries, err := fs.ReadDir(web.FS, "templates")
	if err != nil {
		return nil, err
	}
	t := template.New("layout").Funcs(template.FuncMap{
		// skipAnnotation renders the installer skip annotation for an SSH key
		// in canonical wording ("skipped by <handle> — …"). The
		// endpoint is a closed enum, so the reason text is derived from it and
		// never from installer-supplied prose.
		"skipAnnotation": skipAnnotation,
		// dict builds the argument map for a {{template}} call that needs
		// more than one value ({{define "offboard_form"}} in onboarding.html).
		"dict": dict,
		// The hold vocabulary (Kind + State badges) is rendered by ONE
		// template define, "hold_badges", on the Holds page and on the
		// member page's Access section, from meshstate's label tables;
		// neither page carries its own wording or classes.
		"holdKindLabel":  holdKindLabel,
		"holdStateLabel": holdStateLabel,
		"holdKindClass":  holdKindClass,
		"holdStateClass": holdStateClass,
	})
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".html") {
			continue
		}
		data, err := fs.ReadFile(web.FS, "templates/"+e.Name())
		if err != nil {
			return nil, err
		}
		_, err = t.Parse(string(data))
		if err != nil {
			return nil, fmt.Errorf("parse %s: %w", e.Name(), err)
		}
	}
	return t, nil
}

// dict turns alternating key/value arguments into a map for a template
// call. It errors on an odd number of arguments or a non-string key so a
// mis-wired call site fails the render instead of silently dropping a field.
func dict(kv ...any) (map[string]any, error) {
	if len(kv)%2 != 0 {
		return nil, fmt.Errorf("dict: want key/value pairs, got %d arguments", len(kv))
	}
	m := make(map[string]any, len(kv)/2)
	for i := 0; i < len(kv); i += 2 {
		k, ok := kv[i].(string)
		if !ok {
			return nil, fmt.Errorf("dict: key %d is %T, want string", i/2, kv[i])
		}
		m[k] = kv[i+1]
	}
	return m, nil
}

func mintNonce() string {
	buf := make([]byte, 32)
	_, _ = rand.Read(buf)
	return base64.RawURLEncoding.EncodeToString(buf)
}

// skipAnnotation renders the installer skip annotation for one SSH key in the
// canonical wording shown in the admin queue: "skipped by
// <handle> — no operator manifest" / "role resolved to no host". It is
// display-only: a missing or partial annotation renders as an empty string.
func skipAnnotation(k onboardstore.SSHKey) string {
	if strings.TrimSpace(k.SkipReason) == "" || strings.TrimSpace(k.SkipReportedBy) == "" || k.SkipReportedAt == 0 {
		return ""
	}
	// A skip only describes an approved key waiting on the installer. If the row
	// has since moved on, the annotation is stale by definition — never render it
	// beside an installed/revoked/rejected status, where it would read as a live
	// failure. Belt-and-braces for the non-transactional clear in
	// MarkSSHKeyInstalled: a clear that fails cannot leave a false signal.
	if k.Status != onboardstore.SSHKeyApproved {
		return ""
	}
	date := time.Unix(k.SkipReportedAt, 0).UTC().Format("2006-01-02") + " UTC"
	var why string
	switch k.SkipReason {
	case "no_manifest":
		why = "no operator manifest"
	case "no_scope":
		why = "role resolved to no host"
	case "env_mismatch":
		why = "recorded for a different goatnet"
	default:
		// The endpoint is a closed enum, so this branch is unreachable through
		// the API *today*. It is reachable across a version skew: image pins are
		// per goatnet (ADR 1120), so a newer installer reporting a reason this
		// console binary predates is expected, not anomalous. Never echo the raw
		// value — it must not render beside the paste-target panel as free text —
		// but do not render nothing either: an empty annotation is byte-identical
		// to "never skipped", which is the false signal this feature exists to
		// remove. Say that something happened and that this console cannot name it.
		why = "reason not recognized by this console"
	}
	return fmt.Sprintf("skipped %s by %s — %s", date, k.SkipReportedBy, why)
}

// Routes builds the HTTP routing tree.
func (s *Server) Routes() http.Handler {
	mux := http.NewServeMux()

	// Unauthenticated.
	mux.HandleFunc("GET /healthz", s.handleHealthz)
	// What this console's onboarding API can do, for a caller that must
	// choose a path before it acts (the intake smoke checks sponsor_withdraw
	// before it vouches, ADR 1207). Unauthenticated: it names capabilities,
	// not data. A console without this route predates every listed one.
	mux.HandleFunc("GET /api/onboard/capabilities", s.handleOnboardCapabilities)
	mux.HandleFunc("GET /internal/probation/ready", s.handleProbationReady)
	mux.Handle("GET /static/", http.StripPrefix("/", http.FileServer(http.FS(web.FS))))
	// Internal Traefik ForwardAuth decision and the cross-host petition target.
	// Neither trusts browser identity headers; both bind solely to the source
	// mesh address Traefik derives from the connection.
	mux.HandleFunc("/internal/probation/auth", s.handleProbationAuth)
	mux.HandleFunc("POST /__goat/probation/request", s.handleProbationRequest)

	// Invite-accept is "the URL is the secret" — token-bound, no admin auth.
	mux.HandleFunc("GET /invites/{token}/accept", s.handleInviteAcceptForm)
	mux.HandleFunc("POST /invites/{token}/accept", s.handleInviteAcceptSubmit)

	// --- PUBLIC onboarding realm (ADR 1062 stage 1) ---
	// NO oauth2-proxy ADMINS gate — these must be reachable without ADMINS,
	// token-gated in-app by the invite one-time token. The front-desk collector
	// (intake.<zone>) re-points here; only these three paths are exposed. Traefik
	// routes them AROUND oauth2-proxy-id-console (the admin paths stay behind it).
	mux.HandleFunc("GET /public/onboard.html", s.handlePublicOnboardForm)
	// The post-mesh half of the welcome package, as a page. Unauthenticated by
	// design (no credentials on it, identical per tier) and published on a
	// mesh-only host, so reachability is the control. See handleOnboardingGuide.
	mux.HandleFunc("GET /public/onboarding-guide", s.handleOnboardingGuide)
	mux.HandleFunc("GET /public/onboard/preview", s.handleOnboardPreview)
	// What a site network must allow before anyone enrolls; public, names + ports only.
	mux.HandleFunc("GET /public/network-requirements", s.handleNetworkRequirements)
	mux.HandleFunc("POST /public/onboard/submit", s.handleOnboardSubmit)

	// Candidate stage-notification intake (server-to-server; NOTIFY_TOKEN bearer,
	// not the oauth2-proxy admin gate). Vestigial now the console owns transitions
	// and fires stage emails directly, but kept for an external notify source;
	// 404s when NOTIFY_TOKEN is unset.
	mux.HandleFunc("POST /api/notify", s.handleNotify)

	// --- MACHINE onboarding realm ---
	// Per-handle keys (ONBOARD_OPERATOR_API_KEYS) + participant-key portal
	// delegation, operator-equivalent. The onboard-runner
	// (claim/result/list/get) + the `goat admin invite` CLI carry their own keys.
	// JSON in/out identical to the portal's shapes so the runner is a drop-in
	// re-point.
	m := s.onboardAPIGate
	mux.HandleFunc("GET /api/onboard/proposals", m(s.apiListProposals))
	// The queue-badge poll target. Deliberately NOT on the machine gate: it is
	// read by the browser session already on /onboarding, so it takes the same
	// door that page does (admins + sponsors, sponsor-scoped). The literal
	// `counts` segment outranks the `{id}` wildcard above it.
	mux.HandleFunc("GET /api/onboard/proposals/counts", s.onboardGate(s.handleOnboardCounts))
	mux.HandleFunc("GET /api/onboard/proposals/{id}", m(s.apiGetProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/sponsor-approve", m(s.apiSponsorApproveProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/sponsor-reject", m(s.apiSponsorRejectProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/approve", m(s.apiApproveProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/reject", m(s.apiRejectProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/onboarded", m(s.apiMarkOnboarded))
	// SSH keys — self-service submit, admin-gated install (operator-only realm).
	// SSH keys, browser realm (sshkeys_page.go).
	//
	// The self-service pair is deliberately NOT behind onboardGate/adminGate:
	// the people who most need it are ordinary members with no admin rights and
	// no sponsorship annotation, and a gate they cannot pass would send them
	// back to mailing a key around out-of-band — the exact path the rails
	// exist to close. It is still authenticated (oauth2-proxy fronts every
	// route), and it can only ever act on the SIGNED-IN person's handle.
	//
	// They MUST still be wrapped in selfServiceAuth. "Not gated" means not
	// AUTHORIZED, not unidentified: the actor only reaches a handler through a
	// middleware that puts it in the request context, so an unwrapped route
	// reads the zero Actor and every one of these pages answers "no signed-in
	// identity on this request" — to everybody, forever.
	mux.HandleFunc("GET /ssh-key", s.selfServiceAuth(s.handleSSHKeySelf))
	mux.HandleFunc("POST /ssh-key", s.selfServiceAuth(s.handleSSHKeySelfSubmit))
	mux.HandleFunc("GET /identity", s.selfServiceAuth(s.handleIdentitySelf))
	// The queue and the decisions are operator-only. The decisions live at
	// /ssh-keys/... rather than under /onboarding/: as a four-segment pattern
	// `POST /onboarding/ssh-keys/{id}/{action}` is ambiguous against the
	// existing `POST /onboarding/{id}/welcome/resend` — neither is more
	// specific, so ServeMux panics at registration.
	// Fabric-credential collection (mint-on-collect,
	// docs/design/fabric-credential-collection.md). Self-service for the same
	// reason as the pair above, and wrapped in selfServiceAuth for the same
	// reason too. The POST mints — there is no separate confirm step,
	// because the confirmation IS the page that shows the secret once.
	// Sign-on setup codes for the Kanidm doors — self-service, because the
	// codes expire in 24 h (a server constant we cannot raise) and Kanidm has
	// no user-initiated recovery of its own. F-351.
	mux.HandleFunc("GET /setup-code", s.selfServiceAuth(s.handleSetupCodeSelf))
	mux.HandleFunc("POST /setup-code", s.selfServiceAuth(s.handleSetupCodeMint))
	// A person's own GitHub username, any time after intake: storing it re-runs
	// the repo-access grant, so a username added late still gets the
	// repositories an on-time one would have (github_retro.go).
	mux.HandleFunc("GET /github", s.selfServiceAuth(s.handleGitHubSelf))
	mux.HandleFunc("POST /github", s.selfServiceAuth(s.handleGitHubSelfSubmit))
	mux.HandleFunc("GET /fabric-credential", s.selfServiceAuth(s.handleFabricCredentialSelf))
	mux.HandleFunc("POST /fabric-credential", s.selfServiceAuth(s.handleFabricCredentialCollect))
	mux.HandleFunc("GET /operator-key", s.selfServiceAuth(s.handleOperatorKeySelf))
	mux.HandleFunc("POST /operator-key", s.selfServiceAuth(s.handleOperatorKeyRequest))
	mux.HandleFunc("GET /operator-key/collect/{id}", s.selfServiceAuth(s.handleOperatorKeyCollect))
	// The operator-key queue + decisions are ADMIN-ONLY (#210) — adminGate,
	// not the onboardGate the other /onboarding pages take. Approval here IS
	// the grant (the file lands and the portal hot-reloads), so this surface
	// is a credential mint, not an onboarding step: the sponsors onboardGate
	// admits have no business on it. The requester ≠ decider rule is enforced
	// in the store; the gate is just the outer wall.
	mux.HandleFunc("GET /onboarding/operator-keys", s.adminGate(s.handleOperatorKeysQueue))
	mux.HandleFunc("POST /operator-keys/{id}/{action}", s.adminGate(s.handleOperatorKeyDecision))
	mux.HandleFunc("GET /onboarding/ssh-keys", s.onboardGate(s.handleSSHKeysQueue))
	mux.HandleFunc("POST /ssh-keys/{id}/{action}", s.onboardGate(s.handleSSHKeyDecision))
	mux.HandleFunc("GET /api/onboard/operator-keys", m(s.apiListOperatorKeys))
	mux.HandleFunc("POST /api/onboard/operator-keys", m(s.apiRequestOperatorKey))
	mux.HandleFunc("GET /api/onboard/operator-keys/{id}/collect", m(s.apiCollectOperatorKey))
	mux.HandleFunc("POST /api/onboard/operator-keys/{id}/approve", m(s.apiApproveOperatorKey))
	mux.HandleFunc("POST /api/onboard/operator-keys/{id}/reject", m(s.apiRejectOperatorKey))
	mux.HandleFunc("POST /api/onboard/operator-keys/{id}/revoke", m(s.apiRevokeOperatorKey))
	mux.HandleFunc("POST /api/onboard/operator-keys/reconcile", m(s.apiReconcileOperatorKeys))
	mux.HandleFunc("GET /api/onboard/ssh-keys", m(s.apiListSSHKeys))
	mux.HandleFunc("POST /api/onboard/ssh-keys", m(s.apiSubmitSSHKey))
	mux.HandleFunc("POST /api/onboard/ssh-keys/{id}/approve", m(s.apiApproveSSHKey))
	mux.HandleFunc("POST /api/onboard/ssh-keys/{id}/reject", m(s.apiRejectSSHKey))
	mux.HandleFunc("POST /api/onboard/ssh-keys/{id}/revoke", m(s.apiRevokeSSHKey))
	mux.HandleFunc("POST /api/onboard/ssh-keys/{id}/installed", m(s.apiMarkSSHKeyInstalled))
	mux.HandleFunc("POST /api/onboard/ssh-keys/{id}/skip-report", m(s.apiReportSSHKeySkip))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/retry", m(s.apiRetryProposal))
	// ADR 1136 cold storage + the one-action way back out of it.
	// F-357: the CLI mint path and the roster backfill both register here, so
	// the console store covers everybody and not only the review queue.
	mux.HandleFunc("POST /api/onboard/register", m(s.apiRegisterMintedProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/archive", m(s.apiArchiveProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/restore", m(s.apiRestoreProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/claim", m(s.apiClaimProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/result", m(s.apiProposalResult))
	// The follow-up action queue (onboard_actions): the runner lists queued
	// actions, claims one exactly once, and writes the outcome back. Same
	// machine gate as the proposals queue.
	mux.HandleFunc("GET /api/onboard/actions", m(s.apiListActions))
	mux.HandleFunc("POST /api/onboard/actions/{id}/claim", m(s.apiClaimAction))
	mux.HandleFunc("POST /api/onboard/actions/{id}/result", m(s.apiActionResult))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/offboard", m(s.apiOffboardProposal))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/offboard-claim", m(s.apiOffboardClaim))
	// Per-onboardee repo-access "audibles" (docs/design/onboarding-github-repo-grants.md
	// §2/§4): GET the effective set (policy ⊕ overrides), POST the full desired
	// override set (replaces the stored one). Read-scoped like the proposal GET.
	mux.HandleFunc("GET /api/onboard/proposals/{id}/repo-access", m(s.apiGetProposalRepoAccess))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/repo-access", m(s.apiSetProposalRepoAccess))
	// GitHub username at any stage; post-mint it re-runs the repo grant.
	mux.HandleFunc("POST /api/onboard/proposals/{id}/github", m(s.apiSetProposalGitHub))
	mux.HandleFunc("POST /api/onboard/proposals/{id}/github-request", m(s.apiRequestProposalGitHub))
	mux.HandleFunc("POST /api/onboard/github-requests", m(s.apiRequestGitHubMissing))
	// Review-gate field edits (ADR 1137). The machine-realm door is the
	// substantive one: without it a correction can only be made from a browser,
	// which invites hand-editing the store on the mgmt host.
	mux.HandleFunc("POST /api/onboard/proposals/{id}/fields", m(s.apiFieldEdits))
	// The loaded on-rails repo-access policy (the rules the effective set starts
	// from) — the `goat access repo policy` view.
	mux.HandleFunc("GET /api/onboard/repo-access/policy", m(s.apiRepoAccessPolicy))
	mux.HandleFunc("POST /api/onboard/invite", m(s.apiMintInvite))
	mux.HandleFunc("GET /api/onboard/invites", m(s.apiListInvites))
	mux.HandleFunc("GET /api/onboard/invites/{id}", m(s.apiGetInvite))
	mux.HandleFunc("POST /api/onboard/invites/{id}/revoke", m(s.apiRevokeInvite))
	mux.HandleFunc("POST /api/onboard/email-reference", m(s.apiEmailReference))
	// Machine siblings of the admin-gated web routes POST /users/{id}/groups and
	// POST /users/{id}/state — the same applyGroupChange / patch_state logic
	// (audit + last-admin guard + role-change email), operator-key attributed.
	// This is what makes a `goat admin` role change behave exactly like a console
	// one. See docs/design/onboarding-notifications.md §"CLI/web parity".
	mux.HandleFunc("POST /api/users/groups", m(s.apiPatchUserGroups))
	mux.HandleFunc("POST /api/users/state", m(s.apiPatchUserState))
	mux.HandleFunc("POST /api/users/reinstatement-check", m(s.apiCheckReinstatement))
	// Org-registry (ADR 1112): reads for any machine-realm principal,
	// mutations operator-only (enforced in the handlers).
	mux.HandleFunc("GET /api/org-registry/orgs", m(s.apiOrgList))
	mux.HandleFunc("GET /api/org-registry/orgs/{id}/children", m(s.apiOrgChildren))
	mux.HandleFunc("GET /api/org-registry/orgs/{id}/descendants", m(s.apiOrgDescendants))
	mux.HandleFunc("GET /api/org-registry/suggest", m(s.apiOrgSuggest))

	mux.HandleFunc("GET /api/org-registry/export", m(s.apiOrgExport))
	mux.HandleFunc("POST /api/org-registry/orgs", m(s.apiOrgCreate))
	mux.HandleFunc("POST /api/org-registry/orgs/{id}/parent", m(s.apiOrgSetParent))
	mux.HandleFunc("POST /api/org-registry/orgs/{id}/update", m(s.apiOrgUpdate))
	mux.HandleFunc("POST /api/org-registry/orgs/{id}/aliases", m(s.apiOrgAddAlias))
	mux.HandleFunc("POST /api/org-registry/orgs/{id}/merge", m(s.apiOrgMerge))
	mux.HandleFunc("POST /api/org-registry/orgs/{id}/unmerge", m(s.apiOrgUnmerge))
	mux.HandleFunc("POST /api/onboard/intake", m(s.apiOnboardIntake))
	mux.HandleFunc("POST /api/onboard/intake/batch", m(s.apiOnboardBatch))
	mux.HandleFunc("POST /api/onboard/lint", m(s.apiOnboardLint))
	mux.HandleFunc("GET /api/onboard/schema", m(s.apiOnboardSchema))

	// Everything else is admin-gated.
	admin := s.adminGate

	// Step-up (#174). Both legs are admin-gated like everything else — a
	// challenge is only meaningful for someone already through the front door.
	mux.HandleFunc("GET /step-up", admin(s.handleStepUpChallenge))
	mux.HandleFunc("GET /step-up/return", admin(s.handleStepUpReturn))
	// Ending your own freshness window can only tighten the gate, so it is
	// not itself step-up-gated — that would be a lock you need a second factor
	// to engage.
	mux.HandleFunc("POST /step-up/lock", admin(s.handleStepUpLock))

	// {$} matches the root only. A bare "GET /" is a catch-all, and it served
	// the Users page with a 200 for every unrouted path (#1500).
	mux.HandleFunc("GET /{$}", admin(s.handleUsersList))
	// Holds are a mesh fact, per goatnet: the page ignores the identity-plane
	// selector, so the query is stripped before the gate resolves an IdP.
	mux.HandleFunc("GET /holds", s.ignoreIdP(admin(s.handleHolds)))
	mux.HandleFunc("GET /reactivation-requests", admin(s.handleReactivationRequests))
	mux.HandleFunc("GET /reactivation-requests/{id}/decline", admin(s.handleReactivationDeclinePreview))
	mux.HandleFunc("POST /reactivation-requests/{id}/decline", admin(s.handleReactivationDecline))
	mux.HandleFunc("GET /holds/{peer}/release", s.ignoreIdP(admin(s.handleReleaseHoldPreview)))
	mux.HandleFunc("POST /holds/{peer}/release", s.ignoreIdP(admin(s.handleReleaseHoldConfirm)))
	mux.HandleFunc("GET /users/new", admin(s.handleUserNewForm))
	mux.HandleFunc("POST /users/new", admin(s.handleUserCreate))
	mux.HandleFunc("GET /users/{id}", admin(s.handleUserDetail))
	mux.HandleFunc("POST /users/{id}/groups", admin(s.handleUserPatchGroups))
	mux.HandleFunc("POST /users/{id}/revoke-sessions", admin(s.handleUserRevokeSessions))
	mux.HandleFunc("POST /users/{id}/reset-2fa", admin(s.handleUserReset2FA))
	mux.HandleFunc("POST /users/{id}/rotate-credential", admin(s.handleUserRotateCredential))
	mux.HandleFunc("POST /users/{id}/delete", admin(s.handleUserDelete))
	mux.HandleFunc("POST /users/{id}/recovery-link", admin(s.handleUserMintRecovery))
	mux.HandleFunc("POST /users/{id}/email-reference", admin(s.handleUserEmailReference))
	mux.HandleFunc("POST /users/{id}/state", admin(s.handleUserPatchState))

	mux.HandleFunc("GET /invites", admin(s.handleInvitesList))
	mux.HandleFunc("POST /invites", admin(s.handleInviteCreate))
	mux.HandleFunc("POST /invites/{token}/revoke", admin(s.handleInviteRevoke))

	// Offboard-verification view (goat#319): read-only proof-of-absence
	// across the four identity planes. Admin-gated — it reads across every
	// plane, the same altitude as the portal's operator-only roster.
	mux.HandleFunc("GET /offboard-verify", admin(s.handleOffboardVerify))

	mux.HandleFunc("GET /permissions", admin(s.handlePermissions))
	mux.HandleFunc("GET /audit", admin(s.handleAudit))
	// Legacy redirect to the guide. Deliberately UNGATED: it holds nothing,
	// and the whole point is that an old bookmark or an already-sent email
	// lands somewhere useful. Under the old onboardGate it 403'd for anyone
	// who was neither a matrix-admin nor a portal-resolved sponsor.
	mux.HandleFunc("GET /docs/onboarding", s.handleDocsOnboarding)

	// Onboarding review (ADR 1062 — HUMAN admin realm, oauth2-proxy ADMINS gated).
	// An oauth2-proxy admin is operator-equivalent → ADR 1071 authorized with no
	// tier cap. Backed by the local store (s.Onboard), attribution native.
	// The human onboarding surface is gated by onboardGate, not adminGate
	// (ADR 1072): an ADMINS session gets the full admin queue; a non-admin
	// session is admitted only when the portal resolves their login email to
	// a sponsorship annotation, and is then scoped to their OWN invitees at
	// their annotated tier cap.
	mux.HandleFunc("GET /onboarding", s.onboardGate(s.handleOnboarding))
	// Device details are admin-only and fetched separately from page rendering
	// so a slow NetBird does not block the candidate list or detail card (#2384).
	mux.HandleFunc("GET /onboarding/{id}/devices", s.adminGate(s.handleOnboardingDevices))
	mux.HandleFunc("GET /onboarding/invite", s.onboardGate(s.handleOnboardInviteForm))
	mux.HandleFunc("POST /onboarding/invite", s.onboardGate(s.handleOnboardInviteCreate))
	// Email-delivery controls on the invite-created page (the one-time link
	// lives only in that page's form state — never stored server-side).
	mux.HandleFunc("POST /onboarding/invite/email/verify", s.onboardGate(s.handleOnboardInviteEmailVerify))
	mux.HandleFunc("POST /onboarding/invite/email/send", s.onboardGate(s.handleOnboardInviteEmailSend))
	mux.HandleFunc("POST /onboarding/invite/email/eml", s.onboardGate(s.handleOnboardInviteEml))
	// Revoke an open invite from the queue's Invited tab (the human-web leg of
	// the machine realm's POST /api/onboard/invites/{id}/revoke).
	mux.HandleFunc("POST /onboarding/invite/{id}/revoke", s.onboardGate(s.handleOnboardInviteRevoke))
	// Org match-or-create at review (ADR 1112 increment 2). More-specific
	// patterns win over the {action} route below; admin-only inside (a
	// sponsor session gets 403 — org structure is not a vouching power).
	mux.HandleFunc("GET /onboarding/org-suggest", s.onboardGate(s.handleOnboardOrgSuggest))
	// Live derivation for the review form: what alliance standing and
	// ours-vs-foreign WILL be, from a nation the operator has typed but not
	// submitted. SESSION-gated like its sibling above, not API-key gated — it is
	// read by the page, and an API-key route would 401 the browser silently.
	// Read-only; authorizes nothing (ADR 1077 §5).
	mux.HandleFunc("GET /onboarding/affiliation-derive", s.onboardGate(s.apiAffiliationDerive))
	mux.HandleFunc("POST /onboarding/{id}/org-attach", s.onboardGate(s.handleOrgAttach))
	mux.HandleFunc("POST /onboarding/{id}/fields", s.onboardGate(s.handleFieldEdits))
	mux.HandleFunc("POST /onboarding/{id}/org-new", s.onboardGate(s.handleOrgNew))
	mux.HandleFunc("POST /onboarding/{id}/{action}", s.onboardGate(s.handleOnboardingDecide))
	// The Archived card's way back (#1037): admin-only inside the handler,
	// same store write + audit event as the operator API's .../restore.
	mux.HandleFunc("POST /onboarding/{id}/restore", s.onboardGate(s.handleOnboardingRestore))
	// GitHub username on the Onboarded / Enrolled card (admin-only inside the
	// handler), and the browser read of the repo-access set the card draws.
	mux.HandleFunc("POST /onboarding/{id}/github", s.onboardGate(s.handleOnboardingGitHub))
	mux.HandleFunc("POST /onboarding/{id}/github-request", s.onboardGate(s.handleOnboardingGitHubRequest))
	mux.HandleFunc("GET /onboarding/{id}/repo-access", s.onboardGate(s.handleOnboardingRepoAccessView))
	mux.HandleFunc("POST /onboarding/{id}/repo-access", s.onboardGate(s.handleOnboardingRepoAccessSet))
	// The generated welcome package — a complete, tier-correct
	// hand-over artifact for an onboarded enrollee, downloadable when email is
	// unavailable.
	mux.HandleFunc("GET /onboarding/{id}/welcome", s.onboardGate(s.handleWelcomeDownload))
	// Re-send that package, banner-marked as a repeat. The recovery path when the
	// first one went out incomplete (no setup key) — four segments, so it stays
	// distinct from the three-segment {action} decide route above.
	mux.HandleFunc("POST /onboarding/{id}/welcome/resend", s.onboardGate(s.handleWelcomeResend))
	// Queue a follow-up for an onboarded person (ADR 1175: reissue the handset
	// key for a lost phone). Four segments for the same reason as the resend
	// route above. The runner claims it from /api/onboard/actions below.
	mux.HandleFunc("POST /onboarding/{id}/handset-key/reissue", s.onboardGate(s.handleHandsetKeyReissue))
	// Same queue: a fresh device setup key with a chosen reuse count, for a
	// person whose key expired or was used up.
	mux.HandleFunc("POST /onboarding/{id}/device-key/reissue", s.onboardGate(s.handleDeviceKeyReissue))

	return s.requestLogger(mux)
}

// --- middleware --------------------------------------------------------

// selfServiceAuth identifies the caller WITHOUT authorizing them: it resolves
// the actor from the unforgeable X-Forwarded-* family (F-244, same read as
// every gate) and puts it in the request context, then admits any signed-in
// person. No group check, no sponsorship check — that is the whole point of a
// self-service route, whose users are ordinary members holding neither.
//
// "Ungated" still needs this middleware, because the actor reaches a handler
// ONLY through a middleware that stores it. Without one, /ssh-key and
// /operator-key read the zero Actor and refuse every visitor with "no signed-in
// identity on this request", including people who just authenticated
// successfully.
//
// Fail-closed: no resolved identity is 401 here rather than a nil-actor handler
// deeper in, so a future self-service route cannot quietly serve an anonymous
// caller. DevBypassAuth is honored because actorFromHeaders applies it.
func (s *Server) selfServiceAuth(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		actor := s.actorFromHeaders(r)
		if strings.TrimSpace(actor.Email) == "" {
			s.renderError(w, r, http.StatusUnauthorized, "no signed-in identity on this request")
			return
		}
		next(w, r.WithContext(context.WithValue(r.Context(), ctxKeyActor, actor)))
	}
}

// adminGate admits any group the canonical matrix grants `admin` on
// this console (ADR 1073: gates consult ops/permissions/group-permissions.yml,
// never a hardcoded group literal — so ADMINS and IAM-ADMIN both pass
// today, and future grants are a YAML edit). Threads the resolved IdP
// into context. Fail-closed: nil/unloaded matrix admits nobody.
func (s *Server) adminGate(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		actor := s.actorFromHeaders(r)
		if !s.Cfg.DevBypassAuth {
			if !s.Permissions.AnyGroupGrants(actor.Groups, "id-admin-console", "admin") {
				http.Error(w, "forbidden: no group with admin access to id-admin-console", http.StatusForbidden)
				return
			}
		}
		ctx := context.WithValue(r.Context(), ctxKeyActor, actor)

		idpKey := r.URL.Query().Get("idp")
		if idpKey == "" {
			idpKey = s.Cfg.DefaultIdP
		}
		idp, ok := s.Cfg.IdP(idpKey)
		if !ok {
			http.Error(w, "unknown idp: "+idpKey, http.StatusBadRequest)
			return
		}
		ctx = context.WithValue(ctx, ctxKeyIdP, idp)

		next.ServeHTTP(w, r.WithContext(ctx))
	}
}

// requestLogger is a thin access-log shim.
func (s *Server) requestLogger(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		ww := &statusRecorder{ResponseWriter: w, status: 200}
		next.ServeHTTP(ww, r)
		slog.Info("http",
			slog.String("method", r.Method),
			slog.String("path", r.URL.Path),
			slog.String("query", r.URL.RawQuery),
			slog.Int("status", ww.status),
			slog.Duration("dur", time.Since(start)),
			slog.String("remote", r.RemoteAddr),
		)
	})
}

type statusRecorder struct {
	http.ResponseWriter
	status int
}

func (s *statusRecorder) WriteHeader(c int) {
	s.status = c
	s.ResponseWriter.WriteHeader(c)
}

// --- actor / IdP plumbing ---------------------------------------------

type ctxKey int

const (
	ctxKeyActor ctxKey = iota
	ctxKeyIdP
)

// Actor is the operator making this request. Identity fields are derived from
// the oauth2-proxy forwarded headers.
//
// AAL is the exception: it does not come from a header at all. It is resolved
// live from the Kratos session ledger at the step-up-gated call sites and
// stamped onto a local copy of the Actor purely so the audit record reflects
// what actually backed the action. Treat a zero AAL as "not established",
// never as "aal1". See stepup.go.
type Actor struct {
	Email      string
	Username   string
	IdentityID string
	Groups     []string
	AAL        string
	SourceIP   string
}

func (s *Server) actorFromHeaders(r *http.Request) Actor {
	// SECURITY: read identity ONLY from the X-Forwarded-* family. Our
	// sidecar is oauth2-proxy as a reverse-proxy (--pass-user-headers), which
	// STRIPS any client-supplied X-Forwarded-* and sets them from the validated
	// session — so they cannot be forged by the browser. It does NOT strip a
	// client-supplied X-Auth-Request-* (that family is only SET on the
	// /oauth2/auth subrequest RESPONSE for nginx auth_request), so reading it
	// let a non-admin forge X-Auth-Request-Groups: ADMINS and pass adminGate.
	// Every field below (esp. Groups, used by adminGate, and Email, used by
	// onboardGate sponsorship) must trust only the unforgeable X-Forwarded-*.
	a := Actor{
		Email:    r.Header.Get("X-Forwarded-Email"),
		Username: r.Header.Get("X-Forwarded-User"),
		// The identity ID rides X-Forwarded-User: oauth2-proxy sets it from
		// the OIDC `sub` claim, which Kratos-as-OAuth2-provider fills with the
		// identity UUID. The previous source, X-Forwarded-Preferred-Username,
		// was NEVER set on this stack (goat#247): oauth2-proxy only emits it
		// from a `preferred_username` claim, the consent UI only mints that
		// claim from identity.traits.username, and the identity schema has no
		// username trait — so every actor arrived with an empty identity ID
		// and the step-up gate failed closed for everyone.
		//
		// `sub` is the identity UUID only for subject_type=public OAuth2
		// clients (all current clients). A pairwise client would deliver an
		// obfuscated per-client subject here — identityIDFromSubject rejects
		// anything that is not a UUID so that value is never sent to Kratos
		// as an identity ID.
		IdentityID: identityIDFromSubject(r.Header.Get("X-Forwarded-User")),
		// AAL is deliberately NOT read from a request header. The
		// X-Forwarded-Auth read that used to live here was dead code: nothing
		// upstream ever set it (Kratos emits no acr claim, and oauth2-proxy
		// cannot map an arbitrary claim to a header — see stepup.go), so
		// actor.AAL was permanently "" and every gated action refused. The
		// live AAL now comes from Kratos via resolveStepUpState; the field is
		// filled in at the gated call sites purely so the audit record shows
		// what actually backed the action.
		SourceIP: firstSet(r.Header.Get("X-Real-IP"), r.Header.Get("X-Forwarded-For"), r.RemoteAddr),
	}
	g := r.Header.Get("X-Forwarded-Groups")
	if g != "" {
		a.Groups = splitCSV(g)
	}
	if s.Cfg.DevBypassAuth && a.Email == "" {
		a.Email = "dev@bypass.local"
		a.Groups = []string{platformAdminGroup}
		a.AAL = "aal2"
	}
	return a
}

// identityUUIDRe matches a Kratos identity UUID. Deliberately case-tolerant:
// Kratos emits lower-case, but the check exists to keep non-UUID subjects out
// of admin-API paths, not to enforce a canonical form.
var identityUUIDRe = regexp.MustCompile(`^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$`)

// identityIDFromSubject returns the forwarded OIDC subject if and only if it
// is shaped like a Kratos identity UUID; anything else (empty header, a
// pairwise-obfuscated subject, an email) yields "" so downstream code treats
// the identity as unresolved rather than querying Kratos with a non-ID.
// Deliberately silent: this runs on every request, so the "subject is not a
// UUID" diagnosis is logged where it matters, in resolveStepUpState.
func identityIDFromSubject(sub string) string {
	if identityUUIDRe.MatchString(sub) {
		return sub
	}
	return ""
}

func actorFromCtx(ctx context.Context) Actor {
	a, _ := ctx.Value(ctxKeyActor).(Actor)
	return a
}

func idpFromCtx(ctx context.Context) config.IdP {
	idp, _ := ctx.Value(ctxKeyIdP).(config.IdP)
	return idp
}

// requireAAL2 is the NON-REDIRECTING step-up predicate: it answers "may this
// mutation proceed", and refuses with an error the caller turns into a 403.
// It is what machine/API callers get, since there is no browser to send into
// a re-authentication flow.
//
// Browser-driven console mutations should call requireStepUp instead, which
// redirects the operator into the explain-then-verify flow rather than
// dead-ending them (#174 Story B).
//
// The AAL is resolved LIVE from Kratos (see stepup.go for why it is not a
// forwarded claim header) and must satisfy the freshness window — a session
// that reached aal2 hours ago no longer authorizes a high-blast mutation.
//
// STEP_UP_ENFORCED=false is a per-goatnet policy escape (#1858), distinct from
// the dev-only DevBypassAuth: on a declared sandbox goatnet the gated actions
// proceed and are still audited instead of refusing every admin. The
// fail-closed default (true) keeps prod requiring a real step-up. Callers must
// NOT infer "a second factor was presented" from a nil return — ask
// stepUpVerified for that.
func (s *Server) requireAAL2(r *http.Request, actor Actor) error {
	if s.Cfg.DevBypassAuth {
		return nil
	}
	if !s.Cfg.StepUpEnforced {
		return nil
	}
	if s.stepUpVerified(r, actor) {
		return nil
	}
	return fmt.Errorf("step-up AAL2 required: this action needs a second factor confirmed within the last %d minutes; re-authenticate with your authenticator app (TOTP) or passkey and retry", int(s.Cfg.AAL2FreshWindow/time.Minute))
}

// stepUpVerified reports whether a GENUINE, FRESH second-factor
// authentication backs this request. It is the single source of truth for the
// audit record's step_up_verified field — never "the gate ran", never "policy
// let it through", never DevBypassAuth.
//
// Fail-closed: an unreachable Kratos yields false.
func (s *Server) stepUpVerified(r *http.Request, actor Actor) bool {
	if s.Cfg.DevBypassAuth {
		return false
	}
	return s.resolveStepUpState(r.Context(), actor).Fresh
}

// --- last-ADMIN guard --------------------------------------------------

// guardLastAdmin refuses the operation if it would leave `idpKey` with
// zero identities carrying ADMINS.
//
// targetCurrentlyAdmin tells us whether the target identity currently
// holds ADMINS — relevant when patching groups (the new groups list may
// drop ADMINS) and when deleting.
func (s *Server) guardLastAdmin(ctx context.Context, idpKey string, targetID string, targetCurrentlyAdmin bool, willTargetStillBeAdmin bool) error {
	if !targetCurrentlyAdmin {
		return nil // can't be the last admin if you weren't one
	}
	if willTargetStillBeAdmin {
		return nil
	}
	cli := s.Kratos[idpKey]
	// Dual-window note: counts the canonical name only. An identity carrying
	// ONLY the legacy name would be uncounted, which makes this guard refuse
	// more, never less (all live admins are mirrored; new grants are canonical).
	n, err := cli.CountIdentitiesWithGroup(ctx, platformAdminGroup)
	if err != nil {
		return fmt.Errorf("last-admin guard query failed: %w", err)
	}
	if n <= 1 {
		return fmt.Errorf("refused: removing platform-admin from this identity would leave 0 admins in %q; promote a second admin first", idpKey)
	}
	return nil
}

// --- render helpers ----------------------------------------------------

type pageVars struct {
	Message     string
	Title       string
	IdP         config.IdP
	AllIdPs     []config.IdP
	Actor       Actor
	Flashes     []flash
	RequestPath string
	// RunnerQueue is the onboarding queue's runner band: rows the
	// onboard-runner has left waiting, with what the runner has and has not
	// done since. Admin sessions only; nil when nothing is late.
	RunnerQueue *runnerQueueVars
	CSRFToken   string
	// StepUp drives the layout's persistent step-up banner and the AAL2
	// badges on gated controls (#174). Populated by render for every page,
	// so a gated control is never rendered without its badge.
	StepUp *stepUpVars
	// StepUpChallenge / StepUpResume are the two step-up pages' payloads.
	StepUpChallenge *stepUpChallengeVars
	StepUpResume    *stepUpResumeVars
	// per-page payloads
	Users []userRow
	User  *userDetail
	// Offboard-verification page (goat#319).
	OffboardVerify *offboardVerifyVars
	// Reset2FAOK drives the post-reset success banner on the user-detail page,
	// set from the ?reset2fa=ok redirect the reset handler issues.
	Reset2FAOK          bool
	Sessions            []sessionRow
	SessionsUnavailable bool
	ConsentsUnavailable bool
	ConsentSessions     []consentRow
	Invites             []inviteRow
	Matrix              *permissions.Matrix
	AccessByGroup       map[string]map[string]string
	Events              []eventRow
	RingCap             int
	AuditDurable        bool
	AuditFilter         string
	AAL2FreshWindow     time.Duration
	// ssh-key page. SSHKeySelf switches the one template between the
	// self-service view (your own key, no decision controls) and the operator
	// queue (everyone's, with approve/reject/revoke) — same rows, and the
	// decision controls exist in exactly one place.
	SSHKeys      []onboardstore.SSHKey
	SSHKeySelf   bool
	SSHKeyHandle string
	SSHKeyStatus string
	// ssh-key install panel (operator realm). Visible only to admins whose
	// session email already has an installed key for this console's goatnet;
	// sponsors see neither the panel nor the hint.
	SSHKeyInstallPanel bool
	SSHKeyInstallHint  bool
	// SSHKeyInstallEnv is the goatnet slug for prose; SSHKeyInstallEnvQ is the
	// same value single-quoted for the rendered shell commands.
	SSHKeyInstallEnv        string
	SSHKeyInstallEnvQ       string
	SSHKeyInstallQueueDepth int
	SSHKeyInstallQueueError string

	// Fabric-credential collection (mint-on-collect). FabricCredAPIKey and
	// FabricCredBundle are shown ONCE, on the post-collect render only — the
	// portal keeps a bcrypt hash and the bundle URL is single-use, so neither
	// can be re-read by anyone, including an admin.
	// Sign-on setup codes (/setup-code). SetupCodeValue is shown ONCE, on the
	// post-mint render only, and is never emailed or logged.
	// GitHubSelf is the /github self-service page (github_retro.go).
	GitHubSelf          *githubSelfView
	Identity            *identitySelfView
	SetupCodeDoors      []setupCodeDoor
	SetupCodeIssued     bool
	SetupCodeIsReset    bool
	SetupCodePlane      string
	SetupCodeLabel      string
	SetupCodeHandle     string
	SetupCodeValue      string
	SetupCodeResetPage  string
	SetupCodeExpires    string
	FabricCredSelf      bool
	FabricCredCollected bool
	FabricCredAnchor    string
	FabricCredHandle    string
	FabricCredAPIKey    string
	FabricCredBundle    string
	// FabricCredBundleLink: the bundle URL is absolute (the portal knows its
	// public base), so the page may hyperlink it. A relative URL is rendered as
	// copy-text only — hyperlinking it would resolve against the CONSOLE's host
	// and 404, burning the single-use link.
	FabricCredBundleLink bool
	FabricCredURISAN     string

	// Admin credential rotation (goat#320): the one-time hand-over page after
	// POST /users/{id}/rotate-credential. Same show-once discipline as the
	// fabric-credential page — none of these are recoverable after render.
	RotatedForEmail string
	RotateAnchor    string
	RotateHandle    string
	RotateAPIKey    string
	RotateBundleURL string
	// RotateNonce rides the user-detail rotate form: one-time, per render,
	// so a re-POSTed hand-over page 409s instead of re-keying (goat#580).
	RotateNonce string

	OperatorKeys []*onboardstore.OperatorKey
	// OperatorKeyRows is the ADMIN QUEUE's row set: each row carries whether
	// it is the viewing admin's own request, so the template can swap that
	// row's approve/reject for plain text — the same predicate the store
	// enforces (#210). The self-service view keeps OperatorKeys.
	OperatorKeyRows []operatorKeyRow
	// The Operator-keys dashboard tab (#210): admin sessions only — a pending
	// count is not a sponsor's information, so for them the tab is absent
	// from the markup, not hidden. The visibility predicate is the sponsor
	// resolution the gate already performed, so tab and gate cannot disagree.
	OperatorKeysTab     bool
	OperatorKeysPending int
	OperatorKeySelf     bool
	// OperatorKeyEligible — this viewer holds a console-admin group, so the
	// request form is theirs to use. False renders the explanation of
	// which credential they actually want instead of a form that would be
	// refused on submit.
	OperatorKeyEligible  bool
	OperatorKeyIneligMsg string
	OperatorKeyHandle    string
	OperatorKeyStatus    string
	OperatorKeySecret    string // shown ONCE, on the collect page only
	OperatorKeyDirPath   string
	OperatorKeyStale     []string // we revoked it, the file is still there — access we said was taken away
	OperatorKeyUnmanaged []string // a file for a handle we have no row for — somebody else's key, reported only
	OperatorKeyMissing   []string // active rows with no file — a key the portal will refuse
	// Live keys nobody has ever collected. The person believes they have no
	// credential; the fabric disagrees. Rendered with the age of the
	// approval, because a day-old one is a person who missed the mail and a
	// month-old one is a key that should probably be revoked.
	OperatorKeyUncollected []struct {
		Handle string
		Age    string
	}
	// invite-related
	AcceptURL    string
	PresetGroups []string
	ExpiresAt    string
	Token        string
	TTLHuman     string
	// recovery-link payload
	Email        string
	IdentityID   string
	RecoveryLink string
	RecoveryCode string
	// new-user form
	SuggestedGroups string
	// GroupOptions is the valid login-group vocabulary (group-permissions.yml)
	// rendered as checkboxes on the create/edit forms — tenant realm only.
	GroupOptions []permissions.Group
	// onboarding (three-stage review, ADR 1062 — local store, native attribution)
	OnboardEnabled bool
	// InviteArchiveSweepEnabled reflects whether dead invites auto-archive after
	// a window; the window itself renders in onboarding copy.
	InviteArchiveSweepEnabled bool
	InviteArchiveWindow       string
	// GuideURL is the role-differentiated guide on its own unauthenticated
	// host. The nav links it directly rather than via /docs/onboarding so the
	// menu does not bounce through a redirect on every click.
	GuideURL string
	// GuideRole is appended to the nav's Guide link as `&role=`, so a viewer
	// the server already knows is escalated (that's what gated the admin nav
	// showing at all) lands on their own role's path instead of the guide's
	// default `browse` view. Empty for anyone else — the guide's own
	// RoleByKey("") fallback is unchanged.
	GuideRole string
	// Sponsor mode (ADR 1072 — non-ADMINS session admitted via the portal
	// sponsorship binding): the chrome narrows to the onboarding pages and
	// the sponsor-stage actions; the store-level scoping enforces the rest.
	SponsorMode    bool
	SponsorMaxTier string
	Proposals      []proposalView
	Proposal       *proposalView
	ProposalJSON   template.JS // the selected proposal serialized for identity-card.js
	OnboardStatus  string      // active list filter
	OnboardStages  []onboardStage
	// The pre-submission funnel stage (the "Invited" tab): minted invites whose
	// candidate has not submitted yet, plus expired/revoked ones so a lapsed
	// link is visible instead of vanishing.
	OnboardInvites []onboardInviteView
	// OnboardInvitesArchived is the folded archive under the working list:
	// invites that have been dead (revoked or expired) long enough for the
	// sweep to file them. Same rows, same sponsor scoping — kept separate so
	// the list an operator scans holds live work.
	OnboardInvitesArchived []onboardInviteView
	OnboardInvite          *onboardInviteView
	// Cross-stage find index (#1306): every candidate the session may see, in
	// every stage, rendered hidden and filtered in the browser. Scoped before
	// it is built; the template never decides who is in it.
	OnboardFindRows      []onboardFindRow
	OnboardFindTruncated bool // the index stopped at findIndexMax rows; the page says so
	OnboardFindPartial   bool // invites could not be read; the index covers proposals only
	// sponsor-a-candidate (invite mint)
	InviteResult    *inviteMintedView
	InviteEmailedTo string // candidate address the link was emailed to ("" = not emailed)
	InviteEmailErr  string // non-empty when the email attempt failed (link still shown)
	MailerEnabled   bool
	// Sandbox-aware email delivery (SES sandbox: recipients must verify first).
	InviteCandidateEmail string // candidate address for the delivery panel
	InviteSandbox        bool   // SES account in sandbox → verification controls shown
	InviteVerifySent     bool   // a verification email was just triggered
	InviteVerified       bool   // recipient completed SES verification

	// Holds is the Mesh holds page (/holds): every peer on hold on this
	// goatnet, read-only.
	Holds *holdsPage
	// ReactivationRequests is the read-only admin queue for dormancy petitions.
	ReactivationRequests *reactivationRequestsPage
	ReactivationDecline  *reactivationDeclinePage
	ReleaseHold          *releaseHoldPreview

	// RequireRegisteredOrg mirrors ONBOARD_REQUIRE_REGISTERED_ORG so the
	// review page can disable Approve on an unregistered org up front
	// instead of letting the server's 409 be the first the admin hears of it.
	RequireRegisteredOrg bool
}

// proposalView is the template-facing shape of an onboarding proposal. It flattens
// onboardstore.Proposal's *string DecidedBy so the template can render it without
// dereferencing a pointer.
type proposalView struct {
	ID        string
	Handle    string
	Email     string
	Env       string
	Status    string
	Escalated bool
	Petitions string
	DecidedBy string
	// RestoredBy/RestoredAt (RFC3339 UTC, "" when never restored) render the
	// "Restored by X at T" mark on the Approved, Provisioning and Onboarded
	// cards, so a person brought back from Archived is not mistaken for a
	// fresh approval (#1037).
	RestoredBy string
	RestoredAt string
	// Created is the proposal's created_at (unix seconds), rendered as a
	// relative age on the row. A queue with no timestamps hides its own
	// backlog: a row that has waited a week looks exactly like one that
	// arrived a minute ago.
	Created   int64
	DecidedAt string
	// DecidedAtUnix is the ADMIN DECISION clock (decided_at), the only clock
	// the "runner?" staleness badge may key on. The badge asks "has the
	// onboard-runner failed to claim an APPROVED proposal", and a healthy
	// runner claims within seconds — so the interval that matters starts at
	// the approval, not at submission. Keyed on Created it fired on every
	// proposal that had waited out its two human gates, which is nearly all
	// of them: the badge was on before the runner had a chance to be late.
	DecidedAtUnix int64
	// Note is the runner's writeback on failed / offboard-failed rows — the
	// WHY an admin acts on. The failed panel referenced it before the view
	// carried it, which was a template exec error that truncated the page on
	// exactly the rows that needed the reason shown.
	Note string
	// OffboardRequestedBy/At attribute a real cut (goat#372): the admin whose
	// step-up-verified click queued it, and when (RFC3339, "" when never).
	// DecidedBy stays the final ONBOARDING approver on runner-cut rows, so
	// the offboard panels must name this field, not that one.
	OffboardRequestedBy string
	OffboardRequestedAt string
	// OffboardRequestedAtUnix is the clock the row's "runner?" badge keys on
	// for a queued cut, as DecidedAtUnix is for an approved mint.
	OffboardRequestedAtUnix int64
	// Org match-or-create panel (ADR 1112, ORG_MATCH_ENABLED only).
	// OrgTyped = the manifest's affiliation.id as typed/slugified;
	// OrgCandidates = ranked registry candidates (empty when the typed org
	// already resolves, or the affordance is off). Never auto-applied.
	OrgTyped      string
	OrgCandidates []orgstore.Candidate
	// OrgTypedName is the organization as the candidate wrote it (drafts
	// only), for the panel heading; OrgTyped stays the id.
	OrgTypedName string
	// OrgClaimed marks a panel raised over an org that IS registered: the
	// candidate typed a name that resolves, and no sponsor or admin has stood
	// behind it. The heading and the available actions differ from the
	// unregistered case — there is nothing to create, only a membership to
	// confirm or refuse.
	OrgClaimed bool
	// OrgUnregistered marks a review-stage proposal whose org id the registry
	// does not know. Set for admin AND sponsor sessions (the sponsor gets one
	// informational line; the admin gets the panel and the approve guard).
	OrgUnregistered bool
	// OrgID is the EDITOR's org prefill, read from the manifest by
	// fillEditable. Separate from OrgTyped because that one is the match
	// panel's business and goes empty whenever the panel has nothing to
	// decide. The editor's box was previously composed from OrgTyped with an
	// OrgResolved fallback, which errored the whole page — `index` on an
	// empty slice — exactly on the paths orgMatchesForProposal and
	// orgChainForProposal document as degrading quietly: org matching
	// disabled, a matcher hiccup, an org store read failure.
	OrgID string
	// OrgKind / OrgNation prefill the create-new form's admin-confirmable
	// fact fields from the manifest — the admin confirms or corrects them
	// instead of the create silently inheriting a candidate's
	// self-description (the `origin` kind mislabel).
	OrgKind   string
	OrgNation string
	// OrgNationFromOrg is the registered org id OrgNation was seeded from when
	// the manifest carried no nation (#1122); empty when the manifest supplied
	// it. The box then holds a value Apply will WRITE, so the card names it.
	OrgNationFromOrg string
	// CandidateNationality is identity.nationality, the PERSON's citizenship. It
	// renders beside the create-org Nation box as a labeled reference and is
	// never a value in it (ADR 1171 Decision 6).
	CandidateNationality string
	// OrgSector / OrgGovBranch were the ADR 1151 prefill on the create-new form:
	// the manifest's values, rendered preselected for the admin to confirm.
	// ADR 1171 removed the prefill — an organisation's facts are typed by the
	// admin, not inherited from whichever candidate happened to trigger the
	// create — so these carried a value nothing read, which is worse than
	// carrying none: the form would show a selection the server then ignored.
	// Deleted rather than left populated and unread.
	// StandingConflict is set when this proposal's manifest SEEDED an
	// affiliation.alliance_standing that disagrees with the value derived from
	// its nation (ADR 1151 step 2). The derived value is authoritative
	// (ADR 1077 §5); this exists so a reviewer sees the stale one before minting
	// rather than after.
	StandingConflict string
	// OrgResolved renders when the manifest's org IS registered: the org
	// itself plus its ancestor chain nearest-first — the proof-of-capture
	// the review card otherwise never shows. Read-only, all stages, both
	// realms (structure CHANGES stay admin-only; seeing structure is not
	// a mutation).
	OrgResolved []orgstore.OrgView
	// Relationship is what the admin gate asks about this membership, decided
	// from the organisation (relationship_scope.go): asked for a governmental
	// org, implied `assigned` for a commercial or civil-society one,
	// `constituent` offered only on a multinational body.
	Relationship relationshipChoice
	// Editable turns on the review-gate field editor (ADR 1137): pre-mint
	// stages, ADMIN sessions only — the same posture as the org panel above,
	// for the same reason (correcting org facts is not vouching for a person).
	Editable bool
	// OrgDisplayName / GitHub prefill the editor with the CURRENT values so an
	// admin sees what is there before changing it. A blank GitHub box is not
	// cosmetic: blank means repo grants skip the person at provision.
	OrgDisplayName string
	GitHub         string
	// GitHubPostMint turns on the GitHub username form on the Onboarded and
	// Enrolled cards (admin sessions): adding one after onboarding re-runs the
	// repo-access grant, so the person gets the repositories they missed.
	GitHubPostMint bool
	// Devices lists the mesh peers this person owns, with each one's OS, on
	// the Onboarded and Enrolled cards (admin sessions). ServerOnly flags a
	// person whose only peers are servers: enrolled, yet unable to open the
	// mesh-only login link from a workstation (#1586). Nil when not loaded.
	Devices *meshstate.DeviceSummary
	// DevicesErr is why Devices could not be loaded, shown instead of a list.
	DevicesErr string
	// DevicesURL defers the selected admin card's mesh lookup until after the
	// page loads. Empty for list rows, sponsors, and other proposal stages.
	DevicesURL string
}

// onboardInviteView is the template-facing shape of an onboarding invite on the
// queue's "Invited" tab. State is the DISPLAY state: an 'invited' row past its
// TTL reads "expired" here even before the lazy-expire write touches it
// (ListInvites does not lazy-expire; only the Get paths do).
type onboardInviteView struct {
	ID            string
	Sponsor       string // SponsorName when set, else the raw sponsor key
	Env           string
	Tier          string // pre-bound ("" = enrollee's choice)
	Affiliation   string
	Sector        string // pre-bound affiliation.sector ("" = candidate declares it)
	Site          string
	MachineQuota  int
	Collab        bool
	SponsorCap    string // server-set tier ceiling ("" = unrestricted minter)
	Escalated     bool
	State         string // open | submitted | expired | revoked | archived
	MetadataError string // safe operator-facing remedy; never the malformed value
	Note          string
	Created       string // RFC3339 UTC
	CreatedAgo    string // humanized ("2h ago")
	Expires       string // humanized ("in 6d" / "2d ago")
	ExpiresAt     string // RFC3339 UTC — the absolute deadline
	TTL           string // total lifetime the sponsor chose ("72h" / "7d")
	// Lifecycle: when the link died and when it archives.
	DeadAt               string // RFC3339 UTC when the invite died (expiry or revocation), "" when still open
	DeadAtShort          string // YYYY-MM-DD, for lifecycle prose
	DeadAgo              string // humanized since DeadAt ("2d 3h ago"), "" when still open or unknown
	ArchivesAt           string // RFC3339 UTC when this invite will archive, "" when sweep is disabled or already archived
	ArchivesAtShort      string // YYYY-MM-DD, for lifecycle prose
	ArchivesIn           string // humanized until archive ("4d" / "1d 3h"), "" when ArchivesAt is not set or already due
	ArchivesDueNextSweep bool   // true when archive time has already passed and the invite is waiting on the next sweep
	ArchivedAt           string // RFC3339 UTC when this invite was archived, "" when not archived
	ArchivedAgo          string // humanized since archive ("2d 3h ago"), "" when not archived
	// Delivery: who the link was addressed to, and when/how the console last
	// released it. CandidateEmail "" = handed over out of band (or a row minted
	// before the console recorded recipients). Sent "" = the console never sent.
	CandidateEmail string
	Sent           string // RFC3339 UTC
	SentAgo        string // humanized ("2h ago")
	SentVia        string // email | eml
	// Consumed / Proposal record the submit that ended the invite (only set on
	// a 'submitted' row, which the Invited tab excludes but a direct id reads).
	Consumed   string
	ProposalID string
}

// inviteMintedView is the template-facing mint result (the one-time link to hand
// the candidate).
type inviteMintedView struct {
	ID            string
	Sponsor       string
	Env           string
	IntakeURL     string // public front-desk link (preferred)
	NominationURL string // mesh nomination link
	Escalated     bool
	// ExpiresAt (unix seconds) rides the minted-link page and its email forms so
	// the confirmation states the real deadline, and so a later send from that
	// page can put the same deadline in the candidate's mail.
	ExpiresAt int64
	// ExpiresLabel is ExpiresAt rendered for the page: "in 3 days — 2026-08-21
	// 18:51 UTC". Shown next to the link so a wrong TTL is caught at mint time,
	// not when the candidate reports a dead link.
	ExpiresLabel string
	// ShortTTL flags an expiry under shortInviteTTL — the page warns rather than
	// silently handing over a link that dies the same working day.
	ShortTTL bool
}

type onboardStage struct {
	Key, Label string
	Count      int
}

type flash struct {
	Level   string // info | success | warn | error
	Message string
}

func (s *Server) render(w http.ResponseWriter, r *http.Request, page string, vars *pageVars) {
	s.renderStatus(w, r, page, vars, http.StatusOK)
}

func (s *Server) renderStatus(w http.ResponseWriter, r *http.Request, page string, vars *pageVars, status int) {
	idp := idpFromCtx(r.Context())
	if vars.IdP.Key == "" {
		vars.IdP = idp
	}
	if vars.AllIdPs == nil {
		for _, k := range s.Cfg.IdPOrder {
			if p, ok := s.Cfg.IdP(k); ok {
				vars.AllIdPs = append(vars.AllIdPs, p)
			}
		}
	}
	if vars.Actor.Email == "" {
		vars.Actor = actorFromCtx(r.Context())
	}
	if vars.RequestPath == "" {
		vars.RequestPath = r.URL.Path
	}
	if vars.CSRFToken == "" {
		vars.CSRFToken = s.csrfNonce
	}
	// Step-up posture rides on every page so the banner and the per-control
	// AAL2 badges are always truthful. Cached per-actor for a few seconds
	// (stepUpStateTTL) so ordinary navigation does not add a Kratos call per
	// render; the gate itself never reads this cache.
	if vars.StepUp == nil {
		vars.StepUp = s.stepUpVars(s.cachedStepUpState(r.Context(), vars.Actor))
	}
	// Onboarding nav is visible on every page — the console owns the workflow.
	vars.OnboardEnabled = s.Onboard != nil
	// Guide nav. Empty on a goatnet with no guide host configured, and the
	// template drops the item rather than linking nowhere.
	if vars.GuideURL == "" {
		vars.GuideURL = welcome.GuidePointer(s.welcomeCatalog(""))
	}
	if vars.GuideRole == "" {
		vars.GuideRole = guideRoleForGroups(vars.Actor.Groups)
	}
	// Sponsor sessions get the narrowed chrome on every onboarding page.
	if sp := sponsorFromCtx(r.Context()); sp != nil {
		vars.SponsorMode = true
		vars.SponsorMaxTier = sp.MaxTier
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Referrer-Policy", "no-referrer")
	// Use the per-page template directly — html/template lets us pass the
	// chosen content block as the entrypoint and the layout.html define
	// pulls in {{template "content" .}}.
	t, err := s.Templates.Clone()
	if err != nil {
		s.renderFailure(w, r, page, err)
		return
	}
	// Replace the "content" definition with the requested page.
	// We do this by parsing the chosen page template name as content.
	// html/template's name resolution picks the most-recently-defined "content"
	// — and our templates each define content. To select one, we re-parse
	// just that file's "content" into the clone.
	data, err := fs.ReadFile(web.FS, "templates/"+page+".html")
	if err != nil {
		s.renderFailure(w, r, page, err)
		return
	}
	if _, err := t.New(page).Parse(string(data)); err != nil {
		s.renderFailure(w, r, page, err)
		return
	}
	// Execute into a buffer and write only on success. Templates fail at
	// EXECUTE time, and executing straight into the ResponseWriter would ship
	// the half written so far as a 200: a page that ends after its heading, with
	// no note, no Retry and no </html>. The error text names template
	// internals, so it goes to the log and the operator gets a plain 500.
	var buf bytes.Buffer
	if err := t.ExecuteTemplate(&buf, "layout", vars); err != nil {
		slog.Error("template execute", slog.String("page", page), slog.String("err", err.Error()))
		s.renderFailure(w, r, page, err)
		return
	}
	w.WriteHeader(status)
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = buf.WriteTo(w)
}

func (s *Server) renderFailure(w http.ResponseWriter, r *http.Request, page string, err error) {
	slog.ErrorContext(r.Context(), "console template failed", "page", page, "error", err)
	if page == "error" {
		http.Error(w, "This page failed to render. Try again later.", http.StatusInternalServerError)
		return
	}
	s.renderError(w, r, http.StatusInternalServerError, "This page failed to render. Try again later.")
}

func (s *Server) renderError(w http.ResponseWriter, r *http.Request, status int, msg string) {
	s.renderErrorMessage(w, r, status, msg, false)
}

// renderOperatorError retains the rotation workflow's existing operator remedies.
// Its portal refusal messages are part of that admin-only contract.
func (s *Server) renderOperatorError(w http.ResponseWriter, r *http.Request, status int, msg string) {
	s.renderErrorMessage(w, r, status, msg, true)
}

func (s *Server) renderErrorMessage(w http.ResponseWriter, r *http.Request, status int, msg string, operatorMessage bool) {
	slog.WarnContext(r.Context(), "console request failed", "status", status, "error", msg)
	// Upstream bodies and addresses belong in the server log. Retain local
	// validation guidance, which tells an operator how to correct their input.
	if (status >= 500 && !operatorMessage) || strings.Contains(msg, "://") {
		msg = "The request could not be completed. Try again later."
		if status == http.StatusNotFound {
			msg = "The requested identity or resource could not be found."
		}
	}
	s.renderStatus(w, r, "error", &pageVars{Title: http.StatusText(status), Message: msg}, status)
}

func (s *Server) validateCSRF(r *http.Request) error {
	tok := r.FormValue("csrf")
	if tok == "" || tok != s.csrfNonce {
		return errors.New("CSRF token mismatch — refresh the page and retry")
	}
	return nil
}

// --- handlers: users list / detail / mutations -------------------------

// handleOnboardCapabilities lists the onboarding API's optional behaviours.
func (s *Server) handleOnboardCapabilities(w http.ResponseWriter, r *http.Request) {
	onboardJSON(w, http.StatusOK, map[string]any{
		// A sponsor may withdraw a vouch until the admin decides (ADR 1207).
		"sponsor_withdraw": true,
	})
}

func (s *Server) handleHealthz(w http.ResponseWriter, r *http.Request) {
	w.WriteHeader(http.StatusOK)
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write([]byte("ok"))
}

type userRow struct {
	ID              string
	Email           string
	Name            string
	Groups          []string
	State           string
	LastSessionAt   string
	CreatedAt       string
	OrgShortTag     string
	OrgDisplayName  string
	HasSecondFactor bool
}

func (s *Server) handleUsersList(w http.ResponseWriter, r *http.Request) {
	idp := idpFromCtx(r.Context())
	cli := s.Kratos[idp.Key]
	ids, err := cli.ListIdentities(r.Context())
	if err != nil {
		s.renderError(w, r, 502, fmt.Sprintf("Kratos list failed: %v", err))
		return
	}
	// Single fleet-wide /admin/sessions?active=true call, bucket by identity_id,
	// max issued_at per identity. Solves the N+1 the per-row LastActiveSession
	// would otherwise hit. Failure here is non-fatal — the column just renders
	// "—" for everyone if Kratos rejects the call (e.g. permission error).
	lastByID, lastErr := cli.LastSessionByIdentity(r.Context())
	if lastErr != nil {
		slog.WarnContext(r.Context(), "last-session lookup failed; column will render as '—' for all rows",
			slog.String("err", lastErr.Error()))
		lastByID = map[string]time.Time{}
	}
	rows := make([]userRow, 0, len(ids))
	for _, i := range ids {
		last := "—"
		if t, ok := lastByID[i.ID]; ok && !t.IsZero() {
			last = kratos.FormatExpiresIn(t) // renders "Nm ago" / "Nh ago" / "Nd ago"
		}
		vTag, vName := i.Org()
		rows = append(rows, userRow{
			ID: i.ID, Email: i.Email(), Name: i.Name(),
			Groups: i.Groups(), State: i.State,
			CreatedAt:      i.CreatedAt.UTC().Format(time.DateOnly),
			LastSessionAt:  last,
			OrgShortTag:    vTag,
			OrgDisplayName: vName,
			// Factor state comes inline from the include_credential-augmented
			// ListIdentities (Story 1) — no per-row Kratos call, no N+1.
			HasSecondFactor: i.HasSecondFactor(),
		})
	}
	s.render(w, r, "users_list", &pageVars{
		Title: "Users — " + idp.Label,
		Users: rows,
	})
}

type userDetail struct {
	ID              string
	Email           string
	Name            string
	State           string
	CreatedAt       string
	Groups          []string
	GroupsCSV       string
	OrgShortTag     string
	OrgDisplayName  string
	HasSecondFactor bool
	Aff             affiliationView
	// Access is the mesh hold + setup-key view rendered in the
	// Access section of the user-detail page.
	Access    meshstate.AccessView
	Authority membershipAuthority
}

// affiliationView is the identity-grammar affiliation dimension as an operator
// needs to read it: the recorded facts (kind, nation) plus the DERIVED coalition
// standing, each labeled with where it came from. Standing has no home on the
// identity itself — it is nation-derived against the goatnet's active framework
// — so if this view does not resolve it, nothing on the page tells an operator
// whether a person is a coalition member or a partner.
type affiliationView struct {
	// Known reports whether anything at all was resolvable. False ⇒ the whole
	// block is hidden (operator-shape identities have no affiliation).
	Known bool
	ID    string // affiliation.id (the org slug)
	Kind  string // vendor | organic | partner | civil
	// KindSource / NationSource name the layer each value came from, because
	// they can disagree: the Ory aff claim is frozen at mint (born-as) while
	// the org registry is currently-is (ADR 1051).
	KindSource string
	// Sector / GovBranch are the ADR 1151 target block, read from the org
	// registry row. They are what the page SHOWS; `kind` is derived and
	// deprecated and is no longer rendered anywhere an operator looks.
	Sector       string
	GovBranch    string
	Nation       string
	NationName   string
	NationSource string
	// Standing is the derived coalition standing (member|partner), blank when
	// unresolvable. Basis is the one-sentence why. Unresolved names what is
	// missing instead of guessing.
	Standing      string
	Basis         string
	Framework     string
	FrameworkName string
	MemberSince   string
	MemberUntil   string
	AsOf          string
	Unresolved    string

	// --- ADR 1151 step 2: derived, never stored ------------------------------
	// Organic answers "is this org ours?" RELATIVE TO THE OPERATING SOVEREIGN,
	// which is why it is computed here rather than carried on the identity.
	// OrganicKnown false ⇒ render "unknown", NEVER "foreign": with no declared
	// operating sovereign the question has no answer, and defaulting it would
	// silently reclassify a nation's own units as somebody else's.
	Organic      bool
	OrganicKnown bool
	// OrganicBasis is the one-sentence why, in the same spirit as Basis.
	OrganicBasis string
}

// resolveAffiliation assembles the affiliation view for one identity. Facts come
// from two layers, most-authoritative-for-that-fact first:
//
//   - traits.aff on the Kratos identity (born-as; present only where the
//     goatnet enabled the affiliation trait), then
//   - the console's own org registry row (currently-is; ADR 1112), which is
//     where nation actually lives for the live population today.
//
// Standing is then DERIVED from nation, never read from either layer.
func (s *Server) resolveAffiliation(identity kratos.Identity) affiliationView {
	av := affiliationView{}
	affID, kind, nation := identity.Aff()
	if affID != "" || kind != "" || nation != "" {
		av.Known = true
		av.ID, av.Kind = affID, kind
		av.Nation = strings.ToUpper(strings.TrimSpace(nation))
		if kind != "" {
			av.KindSource = "Ory aff claim"
		}
		if av.Nation != "" {
			av.NationSource = "Ory aff claim"
		}
	}

	// Fall back to — and fill gaps from — the org registry. That row is the only
	// place nation exists on a goatnet that has not enabled the aff trait, which
	// is the default posture.
	shortTag, _ := identity.Org()
	if av.ID == "" {
		av.ID = shortTag
	}
	if s.Orgs != nil && shortTag != "" {
		if org, err := s.Orgs.GetOrg(shortTag); err == nil {
			if av.Kind == "" && org.Kind != "" {
				av.Known = true
				av.Kind, av.KindSource = org.Kind, "org registry"
			}
			if av.Nation == "" && org.Nation != "" {
				av.Known = true
				av.Nation = strings.ToUpper(strings.TrimSpace(org.Nation))
				av.NationSource = "org registry"
			}
			if org.Sector != "" {
				av.Known = true
				av.Sector, av.GovBranch = org.Sector, org.GovBranch
			}
		}
	}

	if !av.Known && av.ID == "" {
		return av // operator-shape identity: no affiliation to show
	}
	av.Known = true

	if s.Coalition == nil {
		av.Unresolved = "the coalition framework registry did not load"
		if s.CoalitionLoadErr != "" {
			av.Unresolved += " — " + s.CoalitionLoadErr
		}
		return av
	}

	asOf := time.Now().UTC()
	res := s.Coalition.Resolve(av.Nation, s.Cfg.CoalitionFramework, asOf)
	av.Standing = res.Standing
	av.Basis = res.Basis
	av.Framework = res.Framework
	av.FrameworkName = res.FrameworkName
	av.NationName = res.NationName
	av.MemberSince = res.MemberSince
	av.MemberUntil = res.MemberUntil
	av.AsOf = asOf.Format(coalition.DateFormat)

	// ADR 1151 step 2. Organic is DERIVED at read time and stored nowhere. The
	// seeded-standing reconcile is NOT done here: this layer has no seeded value
	// to compare against — the org registry row carries neither sector nor
	// alliance_standing, and the Ory aff claim carries neither. The seeded value
	// exists in the operator manifest, so the reconcile lives on the manifest
	// path (onboard_support.go) where it can actually see one.
	av.Organic, av.OrganicKnown = coalition.Organic(av.Nation, s.Cfg.OperatingSovereign)
	switch {
	case !av.OrganicKnown && strings.TrimSpace(s.Cfg.OperatingSovereign) == "":
		av.OrganicBasis = "this goatnet declares no operating sovereign (OPERATING_SOVEREIGN is unset), so \"ours vs foreign\" has no answer here — it is relative to the operator, not to one fixed nation"
	case !av.OrganicKnown:
		av.OrganicBasis = "no nation is recorded on this affiliation, so it cannot be compared with the operating sovereign"
	case av.Organic:
		av.OrganicBasis = fmt.Sprintf("%s is this goatnet's operating sovereign — organic", av.Nation)
	default:
		av.OrganicBasis = fmt.Sprintf("%s is not %s, this goatnet's operating sovereign — foreign", av.Nation, strings.ToUpper(strings.TrimSpace(s.Cfg.OperatingSovereign)))
	}
	return av
}

type sessionRow struct {
	IssuedHuman   string
	ExpiresHuman  string
	AAL           string
	DeviceSummary string
}

type consentRow struct {
	ClientID     string
	ClientName   string
	HandledHuman string
	ScopesCSV    string
	AudienceCSV  string
}

// resolveAccess assembles the mesh Access view for one identity:
// handle derivation via the onboarding store, hold-records load, and
// live NetBird peers + setup keys. All I/O failures degrade to
// "unavailable" states rendered on the page; they never fail the
// user-detail view.
func (s *Server) resolveAccess(ctx context.Context, identity *kratos.Identity) meshstate.AccessView {
	av := meshstate.AccessView{Site: strings.TrimSpace(s.Cfg.NotifyGoatnet)}

	if identity == nil {
		av.HandleError = "identity is nil; handle cannot be derived"
		return av
	}
	if s.Onboard == nil {
		av.HandleError = "onboarding store is not configured; handle cannot be derived"
		return av
	}

	email := strings.TrimSpace(identity.Email())
	prop, ok, err := s.Onboard.ActiveProposalForEmail(ctx, email)
	if err != nil {
		av.HandleError = "could not resolve onboarding proposal for this identity: " + err.Error()
		return av
	}
	if !ok {
		av.HandleError = "handle cannot be derived — no onboarding proposal row for this email"
		return av
	}

	goatnet := strings.TrimSpace(s.Cfg.NotifyGoatnet)
	if goatnet == "" {
		av.HandleError = "handle cannot be derived — NOTIFY_GOATNET is unset"
		return av
	}
	if strings.TrimSpace(prop.Env) != goatnet {
		av.HandleError = fmt.Sprintf("handle cannot be derived — proposal env %q does not match this console's goatnet %q", prop.Env, goatnet)
		return av
	}
	handle := strings.TrimSpace(prop.Handle)
	if handle == "" {
		av.HandleError = "handle cannot be derived — onboarding proposal carries no handle"
		return av
	}

	av.Handle = handle

	// Name universe from the onboarding store for OwnsName. A read
	// failure is carried onto the view: a partial roster weakens name
	// ownership in the dangerous direction, so the page says so.
	known, knownErr := meshstate.KnownHandles(ctx, s.Onboard, nil)

	// Hold records half: absent path or unreadable file degrades to the
	// "hold records unavailable" state, never refuses startup.
	var (
		records meshstate.Records
		recErr  error
	)
	path := strings.TrimSpace(s.Cfg.HoldRecordsPath)
	if path == "" {
		recErr = fmt.Errorf("HOLD_RECORDS_PATH is unset")
	} else {
		records, recErr = meshstate.LoadHoldRecords(path)
		if recErr != nil {
			// Name the cause for the Access section copy but do not fail the page.
			recErr = fmt.Errorf("load hold records from %s: %w", path, recErr)
		} else if records.Skipped > 0 {
			// Contract: bad lines are skipped and counted, never fatal
			// (ops/lifecycle/README.md). The section shows the count; the
			// log names the lines so an operator can repair the file.
			slog.WarnContext(ctx, "hold-records file has bad lines; skipped them, the affected peers read as no-record",
				slog.String("path", path),
				slog.Int("skipped", records.Skipped),
				slog.Any("lines", records.SkippedLines))
		}
	}

	// NetBird half — reuse NotifyMgmtURL + EnrollWatchToken. When either
	// is empty, the Access section renders mesh state as unavailable and
	// Resolve sees a PeersErr/KeysErr; no HTTP call is made.
	var (
		peers    []meshstate.Peer
		peersErr error
		keys     []meshstate.SetupKey
		keysErr  error
	)
	mgmtURL := strings.TrimSpace(s.Cfg.NotifyMgmtURL)
	token := strings.TrimSpace(s.Cfg.EnrollWatchToken)
	if mgmtURL == "" || token == "" {
		cause := "NOTIFY_MGMT_URL is unset"
		if mgmtURL != "" {
			cause = "ENROLL_WATCH_NB_TOKEN is unset"
		}
		peersErr = fmt.Errorf("NetBird mesh state unavailable — %s", cause)
		keysErr = peersErr
	} else {
		client := &meshstate.Client{MgmtURL: mgmtURL, Token: token}
		peers, peersErr = client.Peers(ctx)
		if peersErr != nil {
			slog.WarnContext(ctx, "Access section NetBird peer list failed; rendering mesh state as unavailable",
				slog.String("identity_id", identity.ID),
				slog.String("err", peersErr.Error()))
		}
		keys, keysErr = client.SetupKeys(ctx)
		if keysErr != nil {
			slog.WarnContext(ctx, "Access section NetBird setup-keys list failed; rendering key state as unavailable",
				slog.String("identity_id", identity.ID),
				slog.String("err", keysErr.Error()))
		}
	}

	input := meshstate.ResolveInput{
		Handle:     handle,
		Site:       goatnet,
		Known:      known,
		KnownErr:   knownErr,
		Records:    records,
		RecordsErr: recErr,
		Peers:      peers,
		PeersErr:   peersErr,
		Keys:       keys,
		KeysErr:    keysErr,
		Now:        time.Now().UTC(),
	}
	av = meshstate.Resolve(input)
	authority := s.membershipAuthority()
	if authority.AllowsLiveRelease() && peersErr == nil {
		cli := &meshstate.Client{MgmtURL: mgmtURL, Token: token}
		if groups, err := cli.Groups(ctx); err == nil {
			knownGroups := map[string]bool{}
			for _, g := range groups {
				knownGroups[strings.ToLower(strings.TrimSpace(g.Name))] = true
			}
			for i := range av.Peers {
				p := &av.Peers[i]
				if !p.HasRecord || p.Kind != meshstate.HoldKindDormancy ||
					(p.State != meshstate.HoldStateHeld && p.State != meshstate.HoldStateReleasing &&
						(p.State != meshstate.HoldStateStale || !equalGroupSet(p.Groups, p.PriorGroups))) {
					continue
				}
				allKnown := true
				for _, g := range p.PriorGroups {
					if !knownGroups[strings.ToLower(strings.TrimSpace(g))] {
						allKnown = false
					}
				}
				entered, parseErr := time.Parse(time.RFC3339, p.EnteredAt)
				if allKnown && parseErr == nil {
					q := url.Values{"entered_at": []string{strconv.FormatInt(entered.Unix(), 10)}}
					p.ReleaseURL = "/holds/" + url.PathEscape(p.ID) + "/release?" + q.Encode()
				}
			}
		}
	}
	return av
}

func (s *Server) handleUserDetail(w http.ResponseWriter, r *http.Request) {
	idp := idpFromCtx(r.Context())
	id := r.PathValue("id")
	cli := s.Kratos[idp.Key]
	identity, err := cli.GetIdentity(r.Context(), id)
	if err != nil {
		s.renderError(w, r, 502, fmt.Sprintf("Kratos get failed: %v", err))
		return
	}
	vTag, vName := identity.Org()
	ud := &userDetail{
		ID: identity.ID, Email: identity.Email(), Name: identity.Name(),
		State: identity.State, CreatedAt: identity.CreatedAt.UTC().Format(time.RFC3339),
		Groups: identity.Groups(), GroupsCSV: strings.Join(identity.Groups(), ", "),
		OrgShortTag: vTag, OrgDisplayName: vName,
		// GetIdentity is include_credential-augmented (Story 1) so this reflects
		// the live factor state; flips to false right after a reset.
		HasSecondFactor: identity.HasSecondFactor(),
		Aff:             s.resolveAffiliation(*identity),
		Authority:       s.membershipAuthority(),
	}
	ud.Access = s.resolveAccess(r.Context(), identity)
	sessions, sessErr := cli.ListIdentitySessions(r.Context(), id)
	if sessErr != nil {
		// Keep failed reads distinct from an authoritative empty result.
		slog.WarnContext(r.Context(), "identity sessions lookup failed; rendering unavailable",
			slog.String("identity_id", id),
			slog.String("err", sessErr.Error()))
		sessions = nil
	}
	sRows := make([]sessionRow, 0, len(sessions))
	for _, sess := range sessions {
		dev := "—"
		if len(sess.Devices) > 0 {
			dev = sess.Devices[0].IPAddress
			if sess.Devices[0].UserAgent != "" {
				dev += " · " + truncate(sess.Devices[0].UserAgent, 60)
			}
		}
		sRows = append(sRows, sessionRow{
			IssuedHuman:   sess.IssuedAt.UTC().Format(time.RFC3339),
			ExpiresHuman:  kratos.FormatExpiresIn(sess.ExpiresAt),
			AAL:           sess.AuthenticatorAssuranceLevel,
			DeviceSummary: dev,
		})
	}
	consentSessions, consentErr := s.Hydra[idp.Key].ListConsentSessions(r.Context(), id)
	if consentErr != nil {
		// Keep the rest of the page usable while identifying the failed read.
		slog.WarnContext(r.Context(), "hydra consent sessions lookup failed; rendering unavailable",
			slog.String("identity_id", id),
			slog.String("err", consentErr.Error()))
		consentSessions = nil
	}
	cRows := make([]consentRow, 0, len(consentSessions))
	for _, cs := range consentSessions {
		cRows = append(cRows, consentRow{
			ClientID:     cs.ConsentRequest.Client.ClientID,
			ClientName:   firstSet(cs.ConsentRequest.Client.ClientName, cs.ConsentRequest.Client.ClientID),
			HandledHuman: cs.HandledAt.UTC().Format(time.RFC3339),
			ScopesCSV:    strings.Join(cs.GrantScope, ", "),
			AudienceCSV:  strings.Join(cs.GrantAccessTokenAudience, ", "),
		})
	}
	s.render(w, r, "user_detail", &pageVars{
		Title:               identity.Email() + " — " + idp.Label,
		User:                ud,
		Sessions:            sRows,
		SessionsUnavailable: sessErr != nil,
		ConsentsUnavailable: consentErr != nil,
		ConsentSessions:     cRows,
		GroupOptions:        s.groupOptions(idp.Key),
		Reset2FAOK:          r.URL.Query().Get("reset2fa") == "ok",
		RotateNonce:         s.mintRotateNonce(),
	})
}

func (s *Server) handleUserPatchGroups(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	id := r.PathValue("id")
	newGroups := splitCSV(r.FormValue("groups"))
	revokeSessions := r.FormValue("revoke_sessions") == "1"
	actor := actorFromCtx(r.Context())

	// The web UI is one of the two surfaces that funnel through applyGroupChange
	// (the other is the operator-key POST /api/users/groups). enforceStepUp:true
	// because this actor carries a browser-session AAL that an admin grant/revoke
	// must satisfy.
	if _, err := s.applyGroupChange(w, r, idp.Key, id, newGroups, actor,
		groupChangeOpts{enforceStepUp: true, revokeSessions: revokeSessions}); err != nil {
		if errors.Is(err, errStepUpHandled) {
			// The gate redirected the operator into step-up; the response is
			// already written.
			return
		}
		status := http.StatusBadGateway
		var gce *groupChangeError
		if errors.As(err, &gce) {
			status = gce.status
		}
		s.renderError(w, r, status, err.Error())
		return
	}

	http.Redirect(w, r, fmt.Sprintf("/users/%s?idp=%s", id, idp.Key), http.StatusSeeOther)
}

func (s *Server) handleUserRevokeSessions(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	id := r.PathValue("id")
	cli := s.Kratos[idp.Key]
	target, err := cli.GetIdentity(r.Context(), id)
	if err != nil {
		var httpErr *kratos.HTTPError
		if errors.As(err, &httpErr) && httpErr.StatusCode == http.StatusNotFound {
			s.renderError(w, r, 404, "identity not found")
			return
		}
		s.renderError(w, r, 502, fmt.Sprintf("Kratos get failed: %v", err))
		return
	}
	actor := actorFromCtx(r.Context())

	stepUpAction := ""
	if target != nil && (target.HasGroup(platformAdminGroup) || target.HasGroup(legacyAdminGroup)) {
		stepUpAction = "revoke_sessions_of_admin"
	}
	stepUpVerified := false
	if stepUpAction != "" && s.stepUpRequired(stepUpAction) {
		out := s.requireStepUp(w, r, actor, "revoke_sessions")
		if out.Handled {
			s.emitAudit(r, actor, "revoke_sessions", target, idp.Key, nil, nil, false, "refused", stepUpChallengeAudit)
			return
		}
		// Honesty: step_up_verified reflects whether a fresh second factor
		// actually backed this call — NOT merely that the action is gated.
		// The old `stepUpAction != ""` claimed a factor on every admin
		// revoke, including under STEP_UP_ENFORCED=false (#174 Story C).
		stepUpVerified = out.Verified
		actor.AAL = aalLabel(stepUpVerified)
	}

	if err := cli.RevokeIdentitySessions(r.Context(), id); err != nil {
		s.emitAudit(r, actor, "revoke_sessions", target, idp.Key, nil, nil, stepUpVerified, "error", err.Error())
		s.renderError(w, r, 502, fmt.Sprintf("Kratos revoke-sessions failed: %v", err))
		return
	}
	s.emitAudit(r, actor, "revoke_sessions", target, idp.Key, nil, nil, stepUpVerified, "ok", "")
	http.Redirect(w, r, fmt.Sprintf("/users/%s?idp=%s", id, idp.Key), http.StatusSeeOther)
}

func (s *Server) handleUserDelete(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	id := r.PathValue("id")
	confirm := r.FormValue("confirm_email")
	cli := s.Kratos[idp.Key]
	target, err := cli.GetIdentity(r.Context(), id)
	if err != nil {
		s.renderError(w, r, 502, fmt.Sprintf("Kratos get failed: %v", err))
		return
	}
	if confirm != target.Email() {
		s.renderError(w, r, 400, "confirm_email mismatch — must exactly match the identity's email")
		return
	}
	actor := actorFromCtx(r.Context())
	stepUpVerified := false
	if s.Permissions.IsStepUpAction("delete_identity") {
		out := s.requireStepUp(w, r, actor, "delete_identity")
		if out.Handled {
			s.emitAudit(r, actor, "delete_identity", target, idp.Key, nil, nil, false, "refused", stepUpChallengeAudit)
			return
		}
		stepUpVerified = out.Verified
		actor.AAL = aalLabel(stepUpVerified)
	}
	if err := s.guardLastAdmin(r.Context(), idp.Key, id, target.HasGroup(platformAdminGroup) || target.HasGroup(legacyAdminGroup), false); err != nil {
		s.emitAudit(r, actor, "delete_identity", target, idp.Key, nil, nil, stepUpVerified, "refused", err.Error())
		s.renderError(w, r, 409, err.Error())
		return
	}
	if err := cli.DeleteIdentity(r.Context(), id); err != nil {
		s.emitAudit(r, actor, "delete_identity", target, idp.Key, nil, nil, stepUpVerified, "error", err.Error())
		s.renderError(w, r, 502, fmt.Sprintf("Kratos delete failed: %v", err))
		return
	}
	s.emitAudit(r, actor, "delete_identity", target, idp.Key, map[string]interface{}{"groups": target.Groups()}, nil, stepUpVerified, "ok", "")
	http.Redirect(w, r, "/?idp="+idp.Key, http.StatusSeeOther)
}

func (s *Server) handleUserNewForm(w http.ResponseWriter, r *http.Request) {
	idp := idpFromCtx(r.Context())
	s.render(w, r, "user_new", &pageVars{
		Title:           "New user — " + idp.Label,
		SuggestedGroups: r.URL.Query().Get("groups"),
		GroupOptions:    s.groupOptions(idp.Key),
	})
}

// groupOptions returns the valid login-group vocabulary for the create/edit
// forms — the matrix groups, tenant realm only (other realms carry their own
// schema enums; the form falls back to free text there).
func (s *Server) groupOptions(idpKey string) []permissions.Group {
	if idpKey != tenantIdPKey || s.Permissions == nil {
		return nil
	}
	return s.Permissions.Groups
}

func (s *Server) handleUserCreate(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	email := strings.TrimSpace(r.FormValue("email"))
	name := strings.TrimSpace(r.FormValue("name"))
	// The form submits vocabulary checkboxes (name="groups", one value each)
	// plus a free-text extras field for data-access grammar values; legacy /
	// non-tenant realms still submit one CSV "groups" input. Both shapes land
	// here: split every value, merge, dedup.
	var groups []string
	seen := map[string]bool{}
	for _, raw := range append(append([]string{}, r.PostForm["groups"]...), r.FormValue("groups_extra")) {
		for _, g := range splitCSV(raw) {
			if !seen[g] {
				seen[g] = true
				groups = append(groups, g)
			}
		}
	}
	if email == "" {
		s.renderError(w, r, 400, "email required")
		return
	}
	actor := actorFromCtx(r.Context())
	// Vocabulary fence (tenant realm): refuse mesh-role names + unknown groups
	// before they land in Kratos, where the schema would silently accept them.
	if idp.Key == tenantIdPKey {
		if err := s.validateGroupVocabulary(groups); err != nil {
			s.emitAudit(r, actor, "create_identity", nil, idp.Key, nil, map[string]interface{}{"email": email, "groups": groups}, false, "refused", err.Error())
			s.renderError(w, r, 400, err.Error())
			return
		}
	}
	stepUpVerified := false
	if isAdminGroupList(groups) && s.stepUpRequired("grant_ADMINS") {
		out := s.requireStepUp(w, r, actor, "create_identity")
		if out.Handled {
			s.emitAudit(r, actor, "create_identity", nil, idp.Key, nil, map[string]interface{}{"email": email, "groups": groups}, false, "refused", stepUpChallengeAudit)
			return
		}
		stepUpVerified = out.Verified
		actor.AAL = aalLabel(stepUpVerified)
	}
	traits := map[string]interface{}{"email": email}
	if name != "" {
		traits["name"] = name
	}
	if len(groups) > 0 {
		traits["groups"] = groups
	}
	cli := s.Kratos[idp.Key]
	// F-475: import the address verified; the admin-minted welcome link never
	// verifies it, and fences that check email_verified refuse the identity.
	identity, err := cli.CreateIdentity(r.Context(), kratos.CreateIdentityRequest{
		Traits:              traits,
		VerifiableAddresses: kratos.VerifiedEmail(email),
	})
	if err != nil {
		s.emitAudit(r, actor, "create_identity", nil, idp.Key, nil, map[string]interface{}{"email": email, "groups": groups}, stepUpVerified, "error", err.Error())
		s.renderError(w, r, 502, fmt.Sprintf("Kratos create failed: %v", err))
		return
	}
	rl, err := cli.MintRecoveryLink(r.Context(), identity.ID)
	if err != nil {
		s.emitAudit(r, actor, "mint_recovery_link", identity, idp.Key, nil, nil, false, "error", err.Error())
		s.renderError(w, r, 502, fmt.Sprintf("Kratos recovery-link failed: %v", err))
		return
	}
	s.emitAudit(r, actor, "create_identity", identity, idp.Key, nil, map[string]interface{}{"email": email, "groups": groups}, stepUpVerified, "ok", "")
	s.emitAudit(r, actor, "mint_recovery_link", identity, idp.Key, nil, nil, false, "ok", "")

	s.render(w, r, "recovery_link", &pageVars{
		Title:        "Recovery link minted",
		Email:        identity.Email(),
		IdentityID:   identity.ID,
		RecoveryLink: rl.RecoveryLink,
		RecoveryCode: rl.RecoveryCode,
		ExpiresAt:    rl.ExpiresAt.UTC().Format(time.RFC3339),
	})
}

func (s *Server) handleUserMintRecovery(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	id := r.PathValue("id")
	cli := s.Kratos[idp.Key]
	target, err := cli.GetIdentity(r.Context(), id)
	if err != nil {
		s.renderError(w, r, 502, fmt.Sprintf("Kratos get failed: %v", err))
		return
	}
	rl, err := cli.MintRecoveryLink(r.Context(), id)
	if err != nil {
		s.emitAudit(r, actorFromCtx(r.Context()), "mint_recovery_link", target, idp.Key, nil, nil, false, "error", err.Error())
		s.renderError(w, r, 502, fmt.Sprintf("Kratos recovery-link failed: %v", err))
		return
	}
	s.emitAudit(r, actorFromCtx(r.Context()), "mint_recovery_link", target, idp.Key, nil, nil, false, "ok", "")
	s.render(w, r, "recovery_link", &pageVars{
		Title:        "Recovery link minted",
		Email:        target.Email(),
		IdentityID:   target.ID,
		RecoveryLink: rl.RecoveryLink,
		RecoveryCode: rl.RecoveryCode,
		ExpiresAt:    rl.ExpiresAt.UTC().Format(time.RFC3339),
	})
}

// handleUserPatchState flips identity.state between active and inactive.
// Toggle action via POST form field `state=active|inactive`. The reactivate
// path is the common one — Kratos's passwordless-webauthn mode auto-flips
// freshly-minted password-only identities to inactive within minutes (see
// F-150). Disabling (active → inactive) is the other direction; treated as
// a sensitive mutation since it locks the user out, so it requires step-up
// AAL2 + audit + last-ADMIN guard for the same reason delete does.
// handleDocsOnboarding redirects to the role-differentiated guide, which
// replaced it. This route survives only because welcome emails already sent
// carry the old URL and bookmarks exist; there is no content here.
//
// The page it used to render was a second, hand-maintained copy of the guide's
// admin material. It drifted — it still described fabric bundles being minted
// at onboarding after that stopped, and a clickable password link after F-315
// replaced it with a code — because nothing made the two move together. Its
// unique sections now live in the guide's identity-admin topic.
func (s *Server) handleDocsOnboarding(w http.ResponseWriter, r *http.Request) {
	target := welcome.GuidePointer(s.welcomeCatalog(r.URL.Query().Get("env")))
	if target == "" {
		// Neither a guide host nor a console base configured: this console
		// still serves the guide handler itself, so go there relatively.
		target = "/public/onboarding-guide"
	}
	target += "?topic=identity-admin"
	http.Redirect(w, r, target, http.StatusFound) //nolint:gosec // target is derived from a curated guide pointer, not user input
}

// handleUserEmailReference (POST, admin) sends the identity the complete
// current-role reference email — the catch-up path for people whose access
// predates the role-change emails, or a re-send.
func (s *Server) handleUserEmailReference(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	id := r.PathValue("id")
	identity, err := s.Kratos[idp.Key].GetIdentity(r.Context(), id)
	if err != nil {
		s.renderError(w, r, 502, fmt.Sprintf("Kratos get failed: %v", err))
		return
	}
	if err := s.sendReferenceEmail(r, idp.Key, identity, actorFromCtx(r.Context())); err != nil {
		s.renderError(w, r, 400, "reference email not sent: "+err.Error())
		return
	}
	http.Redirect(w, r, fmt.Sprintf("/users/%s?idp=%s", id, idp.Key), http.StatusSeeOther)
}

func (s *Server) handleUserPatchState(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	id := r.PathValue("id")
	newState := r.FormValue("state")
	if newState != "active" && newState != "inactive" {
		s.renderError(w, r, 400, "state must be 'active' or 'inactive'")
		return
	}
	cli := s.Kratos[idp.Key]
	target, err := cli.GetIdentity(r.Context(), id)
	if err != nil {
		s.renderError(w, r, 502, fmt.Sprintf("Kratos get failed: %v", err))
		return
	}
	actor := actorFromCtx(r.Context())

	// Disabling an ADMINS-bearing identity is sensitive — same risk class
	// as delete (locks them out). Step-up + last-ADMIN guard.
	stepUpVerified := false
	if newState == "inactive" && (target.HasGroup(platformAdminGroup) || target.HasGroup(legacyAdminGroup)) {
		if s.Permissions.IsStepUpAction("delete_identity") {
			out := s.requireStepUp(w, r, actor, "patch_state")
			if out.Handled {
				s.emitAudit(r, actor, "patch_state", target, idp.Key,
					map[string]interface{}{"state": target.State},
					map[string]interface{}{"state": newState}, false, "refused", stepUpChallengeAudit)
				return
			}
			stepUpVerified = out.Verified
			actor.AAL = aalLabel(stepUpVerified)
		}
		if err := s.guardLastAdmin(r.Context(), idp.Key, id, true, false); err != nil {
			s.emitAudit(r, actor, "patch_state", target, idp.Key,
				map[string]interface{}{"state": target.State},
				map[string]interface{}{"state": newState}, stepUpVerified, "refused", err.Error())
			s.renderError(w, r, 409, err.Error())
			return
		}
	}

	after, err := cli.PatchState(r.Context(), id, newState)
	if err != nil {
		s.emitAudit(r, actor, "patch_state", target, idp.Key,
			map[string]interface{}{"state": target.State},
			map[string]interface{}{"state": newState}, stepUpVerified, "error", err.Error())
		s.renderError(w, r, 502, fmt.Sprintf("Kratos patch failed: %v", err))
		return
	}
	_ = after
	s.emitAudit(r, actor, "patch_state", target, idp.Key,
		map[string]interface{}{"state": target.State},
		map[string]interface{}{"state": newState}, stepUpVerified, "ok", "")

	http.Redirect(w, r, fmt.Sprintf("/users/%s?idp=%s", id, idp.Key), http.StatusSeeOther)
}

// --- handlers: invites -------------------------------------------------

type inviteRow struct {
	Token        string
	TokenPrefix  string
	PresetGroups []string
	Status       string
	TTLHuman     string
	CreatedBy    string
	CreatedHuman string
	UsedByEmail  string
	Email        string // bound at mint, or ""
}

func (s *Server) handleInvitesList(w http.ResponseWriter, r *http.Request) {
	idp := idpFromCtx(r.Context())
	all, err := s.Invites.List()
	if err != nil {
		// Best-effort: an invite-store failure renders as an empty list
		// rather than erroring the whole page.
		slog.WarnContext(r.Context(), "invite list failed; rendering as empty",
			slog.String("err", err.Error()))
		all = nil
	}
	rows := make([]inviteRow, 0, len(all))
	for _, inv := range all {
		if inv.IdPKey != idp.Key {
			continue
		}
		rows = append(rows, inviteRowFromInvite(inv))
	}
	s.render(w, r, "invites_list", &pageVars{
		Title:   "Invites — " + idp.Label,
		Invites: rows,
	})
}

func inviteRowFromInvite(inv store.Invite) inviteRow {
	prefix := inv.Token
	if len(prefix) > 8 {
		prefix = prefix[:8]
	}
	used := ""
	if inv.UsedAt != nil {
		used = inv.UsedByEmail
	}
	return inviteRow{
		Token: inv.Token, TokenPrefix: prefix,
		PresetGroups: inv.PresetGroups, Status: inv.Status(),
		TTLHuman:  kratos.FormatExpiresIn(inv.TTLAt),
		CreatedBy: inv.CreatedBy, CreatedHuman: inv.CreatedAt.UTC().Format(time.RFC3339),
		UsedByEmail: used,
		Email:       inv.Email,
	}
}

func (s *Server) handleInviteCreate(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	groups := splitCSV(r.FormValue("preset_groups"))
	if len(groups) == 0 {
		s.renderError(w, r, 400, "preset_groups must contain at least one group")
		return
	}
	// Vocabulary fence (tenant realm), same as create/patch — a bad preset
	// fails HERE at mint time, not as a cryptic schema error in the invitee's
	// face at accept time. Live precedent: an admin pasted the
	// invitees' EMAIL ADDRESSES into this field.
	if idp.Key == tenantIdPKey {
		if err := s.validateGroupVocabulary(groups); err != nil {
			s.emitAuditWithInviteToken(r, actorFromCtx(r.Context()), "mint_invite", "", idp.Key, nil, map[string]interface{}{"preset_groups": groups}, false, "refused", err.Error())
			s.renderError(w, r, 400, err.Error())
			return
		}
	}
	ttlH, err := strconv.Atoi(r.FormValue("ttl_hours"))
	if err != nil || ttlH <= 0 || ttlH > 168 {
		s.renderError(w, r, 400, "ttl_hours must be 1-168")
		return
	}
	// F-475: the admin may BIND the invite to the invitee's address. The
	// accept page then shows it read-only and the login is minted with the
	// address verified, because an admin asserted it. Left empty, the
	// redeemer types their own address at accept and it is minted
	// unverified: a self-asserted address is never imported verified.
	email := strings.TrimSpace(r.FormValue("email"))
	if email != "" {
		parsed, err := mail.ParseAddress(email)
		if err != nil {
			s.renderError(w, r, 400, "email must be one address, e.g. person@example.org")
			return
		}
		email = parsed.Address
	}
	details := map[string]interface{}{"preset_groups": groups, "ttl_hours": ttlH}
	if email != "" {
		details["email"] = email
	}
	actor := actorFromCtx(r.Context())
	// Step-up if the preset elevates to ADMINS.
	stepUpVerified := false
	if isAdminGroupList(groups) && s.stepUpRequired("grant_ADMINS") {
		out := s.requireStepUp(w, r, actor, "mint_invite")
		if out.Handled {
			s.emitAuditWithInviteToken(r, actor, "mint_invite", "", idp.Key, nil, details, false, "refused", stepUpChallengeAudit)
			return
		}
		stepUpVerified = out.Verified
		actor.AAL = aalLabel(stepUpVerified)
	}
	inv, err := s.Invites.Create(idp.Key, actor.Email, email, groups, time.Duration(ttlH)*time.Hour)
	if err != nil {
		s.renderError(w, r, 500, err.Error())
		return
	}
	tokPrefix := inv.Token
	if len(tokPrefix) > 8 {
		tokPrefix = tokPrefix[:8]
	}
	s.emitAuditWithInviteToken(r, actor, "mint_invite", tokPrefix, idp.Key, nil, details, stepUpVerified, "ok", "")

	acceptURL := fmt.Sprintf("%s/invites/%s/accept", s.Cfg.StepUpReturnToBase, inv.Token)
	s.render(w, r, "invite_minted", &pageVars{
		Title:        "Invite minted",
		AcceptURL:    acceptURL,
		PresetGroups: inv.PresetGroups,
		Email:        inv.Email,
		ExpiresAt:    inv.TTLAt.UTC().Format(time.RFC3339),
	})
}

func (s *Server) handleInviteRevoke(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	token := r.PathValue("token")
	actor := actorFromCtx(r.Context())
	tokPrefix := token
	if len(tokPrefix) > 8 {
		tokPrefix = tokPrefix[:8]
	}
	if err := s.Invites.Revoke(token, actor.Email); err != nil {
		s.emitAuditWithInviteToken(r, actor, "revoke_invite", tokPrefix, idp.Key, nil, nil, false, "error", err.Error())
		s.renderError(w, r, 400, err.Error())
		return
	}
	s.emitAuditWithInviteToken(r, actor, "revoke_invite", tokPrefix, idp.Key, nil, nil, false, "ok", "")
	http.Redirect(w, r, "/invites?idp="+idp.Key, http.StatusSeeOther)
}

// --- handlers: permissions / audit ------------------------------------

func (s *Server) handlePermissions(w http.ResponseWriter, r *http.Request) {
	s.render(w, r, "permissions", &pageVars{
		Title:           "Permissions",
		Matrix:          s.Permissions,
		AccessByGroup:   s.Permissions.AccessByServiceByGroup(),
		AAL2FreshWindow: s.Cfg.AAL2FreshWindow,
	})
}

type eventRow struct {
	TSHuman        string
	Actor          audit.Actor
	Action         string
	Target         audit.Target
	Result         string
	ErrorMessage   string
	StepUpVerified bool
	Before         map[string]interface{}
	After          map[string]interface{}
	DiffJSON       string
}

func (s *Server) handleAudit(w http.ResponseWriter, r *http.Request) {
	// Onboarding is console-native now (the extraction), and its decisions are
	// emitted to this console's own audit — which survives redeploys via the
	// AUDIT_BACKEND=file durable JSONL sink. So the page needs no portal read
	// (the old portal-durable-onboarding merge is obsolete: the portal no longer
	// records onboarding audit).
	// ?target= narrows to one candidate/identity: matched against the
	// event target's invite/proposal token prefix and email. This is how
	// the review card's "console actions" link lands here pre-filtered.
	targetFilter := strings.TrimSpace(r.URL.Query().Get("target"))
	events := s.Audit.Recent(200)
	rows := make([]eventRow, 0, len(events))
	for _, e := range events {
		if targetFilter != "" &&
			e.Target.InviteTokenPrefix != targetFilter &&
			e.Target.Email != targetFilter &&
			!strings.HasPrefix(e.Target.InviteTokenPrefix, targetFilter) {
			continue
		}
		diff := map[string]interface{}{}
		if e.Before != nil {
			diff["before"] = e.Before
		}
		if e.After != nil {
			diff["after"] = e.After
		}
		djs, err := json.MarshalIndent(diff, "", "  ")
		_ = err // diff JSON is best-effort; an encode error renders an empty diff
		rows = append(rows, eventRow{
			TSHuman: e.TS.UTC().Format(time.RFC3339),
			Actor:   e.Actor, Action: e.Action, Target: e.Target,
			Result: e.Result, ErrorMessage: e.ErrorMessage,
			StepUpVerified: e.StepUpVerified,
			Before:         e.Before, After: e.After,
			DiffJSON: string(djs),
		})
	}
	s.render(w, r, "audit", &pageVars{
		Title:        "Audit",
		Events:       rows,
		RingCap:      200,
		AuditDurable: s.Audit.Durable(),
		AuditFilter:  targetFilter,
	})
}

// --- audit helpers -----------------------------------------------------

func (s *Server) emitAudit(r *http.Request, actor Actor, action string, target *kratos.Identity, idpKey string, before, after map[string]interface{}, stepUp bool, result, errMsg string) {
	t := audit.Target{IdPKey: idpKey}
	if target != nil {
		t.IdentityID = target.ID
		t.Email = target.Email()
	}
	s.Audit.Emit(r.Context(), audit.Event{
		Actor: audit.Actor{
			Email: actor.Email, IdentityID: actor.IdentityID, IdPKey: idpKey,
			AAL: actor.AAL, SourceIP: actor.SourceIP,
		},
		Action: action, Target: t, Before: before, After: after,
		RequestID: requestID(r), StepUpVerified: stepUp, Result: result, ErrorMessage: errMsg,
	})
}

func (s *Server) emitAuditWithInviteToken(r *http.Request, actor Actor, action, tokenPrefix, idpKey string, before, after map[string]interface{}, stepUp bool, result, errMsg string) {
	s.Audit.Emit(r.Context(), audit.Event{
		Actor: audit.Actor{
			Email: actor.Email, IdentityID: actor.IdentityID, IdPKey: idpKey,
			AAL: actor.AAL, SourceIP: actor.SourceIP,
		},
		Action: action, Target: audit.Target{IdPKey: idpKey, InviteTokenPrefix: tokenPrefix},
		Before: before, After: after,
		RequestID: requestID(r), StepUpVerified: stepUp, Result: result, ErrorMessage: errMsg,
	})
}

// --- helpers -----------------------------------------------------------

func splitCSV(s string) []string {
	if s == "" {
		return nil
	}
	parts := strings.Split(s, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		t := strings.TrimSpace(p)
		if t != "" {
			out = append(out, t)
		}
	}
	return out
}

// Platform-admin group names. PLATFORM-ADMIN is canonical (ADR 1043 §8);
// ADMINS is the legacy alias accepted until the Ory-plane contract removes
// the old membership — delete legacyAdminGroup (and the grant_ADMINS /
// revoke_ADMINS step-up aliases below) in that contract batch.
const platformAdminGroup = "PLATFORM-ADMIN"
const legacyAdminGroup = "ADMINS"

// isAdminGroupList reports whether a group list carries platform-admin
// under either the canonical or the legacy name.
func isAdminGroupList(groups []string) bool {
	return containsString(groups, platformAdminGroup) || containsString(groups, legacyAdminGroup)
}

// guideRoleForGroups picks the Guide role key (welcome.RoleByKey) that best
// matches a viewer's own Ory login groups, so the nav's Guide link can send
// an already-identified admin straight to their own path instead of the
// guide's default `browse` view. Deliberately narrow: it only recognizes the
// two escalated admin tiers, both of which the admin console already checks
// group membership for elsewhere (adminGate, isAdminGroupList) — extending it
// to every onboarding tier would need a broader Ory-group-to-tier mapping
// this console doesn't otherwise carry for its own (non-onboarding) sessions.
// Empty return leaves the guide's own RoleByKey("") -> browse fallback
// unchanged, exactly as it behaves today for everyone this doesn't match.
func guideRoleForGroups(groups []string) string {
	if isAdminGroupList(groups) {
		return "platform-admin"
	}
	if containsString(groups, "IAM-ADMIN") {
		return "iam-admin"
	}
	return ""
}

// stepUpRequired accepts either spelling of the admin step-up action so the
// matrix can rename grant_ADMINS -> grant_PLATFORM-ADMIN without another
// console release.
func (s *Server) stepUpRequired(action string) bool {
	if s.Permissions.IsStepUpAction(action) {
		return true
	}
	alias := map[string]string{
		"grant_ADMINS":          "grant_PLATFORM-ADMIN",
		"revoke_ADMINS":         "revoke_PLATFORM-ADMIN",
		"grant_PLATFORM-ADMIN":  "grant_ADMINS",
		"revoke_PLATFORM-ADMIN": "revoke_ADMINS",
	}
	if a, ok := alias[action]; ok {
		return s.Permissions.IsStepUpAction(a)
	}
	return false
}

func containsString(haystack []string, needle string) bool {
	for _, h := range haystack {
		if h == needle {
			return true
		}
	}
	return false
}

func firstSet(s ...string) string {
	for _, v := range s {
		if v != "" {
			return v
		}
	}
	return ""
}

func truncate(s string, n int) string {
	if len(s) <= n {
		return s
	}
	return s[:n] + "…"
}

// requestID is the id every audit row written for r carries. A caller that
// sends X-Request-ID names it; otherwise the first call mints one and stamps it
// on the request, so a later call in the same request (a draft completion and
// the decision that caused it) returns the same id and the rows join on it.
func requestID(r *http.Request) string {
	if id := r.Header.Get("X-Request-ID"); id != "" {
		return id
	}
	buf := make([]byte, 12)
	_, _ = rand.Read(buf)
	id := base64.RawURLEncoding.EncodeToString(buf)
	r.Header.Set("X-Request-ID", id)
	return id
}

// URLPathFor returns a URL string given a path + idp key — used by the
// few places we construct redirects with the IdP context preserved.
func URLPathFor(path string, idp string) string {
	u := &url.URL{Path: path, RawQuery: "idp=" + url.QueryEscape(idp)}
	return u.String()
}
