// Package config loads runtime configuration from environment variables.
// No config files in v1; env is the entire surface.
package config

import (
	"fmt"
	"net/mail"
	"os"
	"strconv"
	"strings"
	"time"
)

// IdP describes one upstream identity universe the console manages.
// A site can have two: tenant IdP (Kratos 4434, Hydra 4445) and
// operator IdP (Kratos 4436, Hydra 4447). The Key field is the URL-safe
// identifier used in the ?idp= query parameter.
type IdP struct {
	Key            string
	Label          string
	KratosAdminURL string
	HydraAdminURL  string
}

type Config struct {
	ListenAddr          string
	IdPs                map[string]IdP // keyed by IdP.Key
	IdPOrder            []string       // stable ordering for UI
	DefaultIdP          string
	PermissionsYAMLPath string
	// HoldRecordsPath is the path to the lifecycle hold-records JSONL
	// file (ops/lifecycle/schema/hold-record.v1.schema.json). Absent
	// path or unreadable file ⇒ the Access section on the
	// user-detail page renders "hold records unavailable" and names
	// the cause; it never refuses console startup.
	HoldRecordsPath string
	// MembershipScopePath is the installed warrant scope declaration for this
	// goatnet. MembershipAuthority must explicitly select "ungoverned" or
	// "scope"; absent or invalid configuration suppresses live release.
	MembershipScopePath string
	MembershipAuthority string
	// PeerHandshakesPath is the management host's per-peer WireGuard
	// handshake summary (roles/mesh-health, mounted read-only). Backs the
	// enrolled card's control-only flag; unset or stale ⇒ the flag stays off.
	PeerHandshakesPath string
	// CoalitionFrameworksPath is the bind-mounted ops/identity/coalition-frameworks.yaml
	// (same read-only mount pattern as PermissionsYAMLPath). It backs the
	// identity-detail view's coalition-standing row: standing is nation-derived
	// and deliberately absent from both the cert and the Ory claim, so without
	// this file the console cannot show an operator what standing a person holds.
	// Unset/absent ⇒ the row renders "unresolved" and names the missing file;
	// it never guesses a standing.
	CoalitionFrameworksPath string
	// CoalitionFramework is this goatnet's ACTIVE framework id (ADR 1077). Empty
	// ⇒ the registry's own default_framework. Set it per-goatnet when the
	// deployment is not in the default AOR — standing is meaningless without
	// naming which framework it is relative to.
	CoalitionFramework string
	// OperatingSovereign is the ISO 3166-1 alpha-3 nation OPERATING this
	// goatnet (ADR 1151). It is the other half of the `organic` derivation:
	// an affiliation is organic when its nation equals this one, which is why
	// the same org reads organic on one goatnet and foreign on another.
	//
	// Empty ⇒ organic is UNKNOWN and every surface must say so. It must NOT
	// default to any nation: a hardcoded sovereign is precisely the bug ADR 1151
	// names in the old `organic` definition, and defaulting would silently
	// reclassify a nation's own units as somebody else's. Unknown is a safe,
	// visible state; a wrong answer is not.
	OperatingSovereign string
	StepUpReturnToBase string
	AAL2FreshWindow    time.Duration
	// StepUpEnforced is the per-goatnet policy switch for whether the AAL2
	// step-up gate on sensitive mutations is actually enforced. Fail-closed
	// default true (prod). Declared false ONLY on a sandbox goatnet
	// where per-action re-auth is not required — there the gated
	// actions proceed and are still audited, instead of 403-ing every admin
	// because no AAL2 signal is wired. NOT the DevBypassAuth dev-only hack:
	// this is a deliberate, documented deployment policy (env STEP_UP_ENFORCED).
	StepUpEnforced bool
	// StepUpSSOIdP is the IdP realm the console's own oauth2-proxy SSO
	// authenticates admins against — the only realm where the forwarded actor
	// identity ID (the OIDC sub on X-Forwarded-User) resolves. The step-up gate
	// resolves the ACTOR's AAL posture here, never against the ?idp=
	// administered-realm selector: an admin administering the other identity
	// plane is still an SSO-realm identity. Env STEP_UP_SSO_IDP; defaults to
	// the default IdP.
	StepUpSSOIdP  string
	LogLevel      string
	AuditBackend  string // stderr (ring only) | file (ring + durable JSONL)
	AuditFilePath string

	// Onboarding engine (ADR 1062/1071, extracted from the data-fabric portal into
	// this console — docs/design/onboarding-extraction-plan.md). The console owns
	// the proposal + invite store, the mint, the review/decide, and native
	// attribution. OnboardDBDSN is the Postgres database holding that store
	// (env ONBOARD_DB_DSN, required: the store is always opened — onboarding is
	// a first-class console workflow now, not a portal proxy).
	//
	// ADR 1165 step A: the console no longer reads SQLite. ONBOARD_DB_PATH and
	// ORG_DB_PATH are read only by cmd/store-migrate, which imports the old
	// files into Postgres and exports them back for a rollback.
	OnboardDBDSN string
	// OrgDBDSN is the first-class org registry (ADR 1112,
	// docs/design/org-hierarchy-and-dedup.md): orgs + effective-dated parent
	// edges + aliases, in its OWN Postgres database beside the onboarding
	// store's (env ORG_DB_DSN, required).
	OrgDBDSN string
	// OrgMatchEnabled turns on the onboard-time org match-or-create
	// affordance (ADR 1112 increment 2): the fuzzy suggest endpoint + the
	// review-page candidate panel. Default OFF — the §13.2 rollback lever:
	// off = today's free-typed intake, byte-identical behavior.
	OrgMatchEnabled bool

	// RequireRegisteredOrg refuses an ADMIN APPROVAL while the candidate's
	// affiliation.id does not resolve in the org registry (ADR 1171 decision 7).
	//
	// DEFAULT FALSE, and armed per goatnet only after that goatnet's registry
	// carries the orgs its live manifests already name. Armed against a registry
	// that does not, it refuses every onboarding. The flag exists so arming is a
	// per-goatnet decision taken when that goatnet is ready, not a fleet-wide flip.
	//
	// Independent of OrgMatchEnabled above: matching is how an admin FINDS or
	// creates the org, this is whether an unregistered one may be approved at
	// all. A goatnet can match without requiring.
	RequireRegisteredOrg bool
	// OnboardOperatorAPIKeys is the MACHINE realm's PER-HANDLE operator-key map,
	// mirroring the portal's PORTAL_OPERATOR_API_KEYS. Parsed from
	// ONBOARD_OPERATOR_API_KEYS in the form "handle1:key1,handle2:key2"
	// (comma-separated handle:key pairs); keyed by KEY → handle for O(1) resolve.
	// The runner + `goat admin invite` present their own per-handle keys. Empty →
	// the machine-token routes 503 (not configured). Operator keys are
	// operator-equivalent (ADR 1071 authorized, no tier cap); an unknown key is
	// treated as a participant key at the resolve seam (portal delegation, wired
	// later).
	OnboardOperatorAPIKeys map[string]string
	// IntakeBaseURL is the PUBLIC front-desk base a candidate-onboarding one-time
	// link is built against (e.g. https://intake.example.net). Empty → the
	// invite mint returns a relative intake path only.
	IntakeBaseURL string
	// ConsolePublicURL is THE CONSOLE'S OWN base (admin.auth.<site>.<zone>) — the
	// host that actually serves /ssh-key and the admin pages. Distinct from
	// PortalPublicURL below, which is the PUBLIC INTAKE front desk and serves
	// neither. Conflating them produces a dead /ssh-key link. Falls back to
	// StepUpReturnToBase, which is the same
	// host and is already set on every goatnet.
	ConsolePublicURL string
	// PortalPublicURL is the public INTAKE base (intake.<zone>), used to build the
	// nomination link the sponsor can hand over today (the /public/onboard.html
	// form). Empty → nomination link is relative.
	PortalPublicURL string
	// PortalInternalURL is the mesh-internal base of the data-fabric portal
	// (goat-zenoh-portal) the console calls read-only for participant-key
	// delegation: a machine-realm key that is NOT a known operator key is a
	// participant's own portal key, resolved via GET {PortalInternalURL}/api/whoami
	// (→ org handle + ADR 1071 sponsor cap). Empty → participant-key mint is
	// unavailable (the resolve seam stays fail-closed); operator-key mint still works.
	// CollabDoorURL / PublicDoorURL and their idm_admin passwords back the
	// /setup-code self-service page: the two Kanidm sign-on planes, which are
	// SEPARATE from the mesh Ory/Kratos plane. Empty → that door is simply not
	// offered, which is the right default (most goatnets have no collab plane).
	//
	// The password is the same idm_admin credential the onboard-runner already
	// holds on this host for collab provisioning, so this adds no new secret to
	// a new blast radius. It is admin-scoped because Kanidm has no lesser right
	// that can mint a credential-update intent.
	CollabDoorURL     string
	CollabDoorAdminPW string
	PublicDoorURL     string
	PublicDoorAdminPW string
	PortalInternalURL string
	// PortalLookupToken is the bearer this console presents to the portal's
	// sponsorship-by-email lookup (ADR 1072 sponsor mode). Defaults to the
	// NOTIFY_TOKEN shared secret (same console↔portal trust pair); empty
	// disables sponsor mode entirely (non-admin sessions stay refused).
	PortalLookupToken string

	// SMTP (AWS SES) for emailing onboarding invite links. Empty host disables
	// email — the invite link is still shown for manual hand-over. Recovery
	// emails go through the Kratos courier separately (config on the Kratos side).
	SMTPHost     string
	SMTPPort     string
	SMTPUsername string
	SMTPPassword string
	SMTPFrom     string
	// SMTPReplyTo is the monitored mailbox an invite's Reply-To points at. The
	// From identity is send-only (no MX on its zone), so a blank value means the
	// invite says a reply is not delivered rather than routing it nowhere.
	SMTPReplyTo string

	// SES v2 API access for the sandbox-aware invite email controls
	// (production-access detection + recipient verification). All three empty
	// = feature off (the console hides the sandbox controls). The access key
	// is the same IAM user the SMTP credentials derive from (the goatnet's
	// Terraform ses.tf), policy-scoped to GetAccount + Create/GetEmailIdentity.
	SESAPIRegion          string
	SESAPIAccessKeyID     string
	SESAPISecretAccessKey string
	// Candidate stage-notification intake (onboarding-notifications.md
	// increment 3). The portal POSTs /api/notify server-to-server (it cannot
	// present oauth2-proxy group headers), authenticated by this shared
	// bearer; empty disables the endpoint. The three URLs are the
	// candidate-facing links the final "you're onboarded" email renders —
	// per-goatnet display values owned here so the portal payload stays
	// minimal (event + identity + setup key only).
	NotifyToken   string
	NotifyMgmtURL string // mesh enrollment URL, e.g. https://netbird.example.net
	// Enrollment watcher (internal/enrollwatch): flips an onboarded proposal to
	// `enrolled` once the person actually consumes their mesh setup key. Reuses
	// NotifyMgmtURL as the NetBird API base; the TOKEN is what arms it. Empty
	// token ⇒ watcher disabled, every row stays at `onboarded` (default-off).
	// SSHKeyTTLDays is the expiry RECORDED on a submitted key. Enforcement is
	// deliberately NOT here: silently expiring a shell mid-incident is a real
	// hazard, so the sweep that acts on it is a separate, default-off choice.
	// 0 = record no expiry at all rather than invent one.
	SSHKeyTTLDays int

	// OperatorKeyDir is the on-host directory the PORTAL reads for operator
	// API keys, shared with this console as a bind mount. Approving a request
	// writes a file here and the portal picks it up on its next reload, so
	// issuing an attributable key needs no envrc edit and no restart. Empty =
	// the console records requests but grants nothing, and says so rather than
	// letting an approval look successful.
	OperatorKeyDir      string
	EnrollWatchToken    string
	EnrollWatchInterval time.Duration
	// InviteArchiveAfter is how long an invite stays on the working list after
	// it dies (revoked, or past its TTL) before the sweep files it into the
	// folded archive. 0 disables the sweep entirely — dead invites then pile up
	// on the working list exactly as they did before this existed. Only the
	// env word forms off / never / disabled produce 0 here; "0", a blank or an
	// unparseable ONBOARD_INVITE_ARCHIVE_AFTER falls back to the 7-day default.
	InviteArchiveAfter time.Duration
	// ClaimRequeueAfter is how long a proposal may sit claimed (onboarding /
	// offboarding) before the sweep puts it back on the runner's queue,
	// recovering a row a dead runner stranded (goat#977). 0 disables the sweep
	// (stranded rows stay stuck). Only off / never / disabled produce 0; a
	// blank or unparseable ONBOARD_CLAIM_REQUEUE_AFTER uses the 10-minute default.
	ClaimRequeueAfter time.Duration
	NotifyGetURL      string // mesh download surface, e.g. https://get.site.netbird.example.net
	NotifyPortalURL   string // mesh portal, e.g. https://portal.site.netbird.example.net
	// GuideRepoBaseURL is the org base the onboarding guide composes worked-example
	// links from, e.g. "https://github.com/usareur-af". Repositories get renamed and
	// move between organizations, so the guide stores SLUGS and composes links here;
	// blank renders the names without links plus instructions to ask, which is the
	// honest failure mode rather than a page of 404s.
	GuideRepoBaseURL string
	// PlatformRepoURL is the platform source repository the welcome package
	// and the membership-authority panel link to (admin guide, git clone,
	// warrant membership declarations), e.g. "https://github.com/example/goat".
	PlatformRepoURL string
	// CollabURL is the collaboration environment's front door named in the
	// welcome package's collab block (NOTIFY_COLLAB_URL).
	CollabURL string
	// NOTIFY_RELAY_HEALTH_URL — public relay-health dashboard surfaced in the welcome
	// package so new onboardees learn they have access. Off-mesh zone, not derivable
	// from the portal base, so it is configured explicitly. Empty = not surfaced.
	NotifyRelayHealthURL string
	// NOTIFY_MAP_URL — the geographic map surface, on goatnets that carry a live
	// geographic feed. Configured rather than derived from the portal base
	// because most goatnets have no such feed, and a derived link would point
	// confidently at a host that does not exist there. Empty = the guide renders
	// "not configured on this goatnet". Same pattern as NOTIFY_RELAY_HEALTH_URL.
	NotifyMapURL string
	// NotifyRelayAddresses + NotifyStunHosts are the goatnet's relay_addresses
	// and stun_hosts group_vars, so the guide's firewall topic lists what a
	// device on THIS goatnet actually dials. Empty renders no address table.
	NotifyRelayAddresses []string
	NotifyStunHosts      []string
	// NOTIFY_MINT_ON_COLLECT — templated from the goatnet's
	// onboard_runner_defer_credential so ONE fact drives the runner's behavior
	// and the guide's description of it. Empty/false = credentials are minted
	// at provisioning and delivered by the welcome email.
	NotifyMintOnCollect bool
	// NotifyRosenpass, when set, appends --enable-rosenpass --rosenpass-permissive
	// to the mesh-enrollment command this goatnet hands new peers, so their
	// WireGuard sessions are post-quantum-secured (ADR 0602). Per-goatnet toggle,
	// default off — the staged-rollout switch: flip it per goatnet (canary first).
	// Permissive means an enabled peer still connects to not-yet-enabled peers, so
	// turning it on never breaks connectivity.
	NotifyRosenpass bool
	// NotifyGoatnetTagline / NotifyGoatnetPurpose identify WHICH environment a
	// person is being onboarded to and what it is for — the guard against
	// "which goatnet am I on?" confusion once multiple goatnets onboard the
	// same people. Tagline is a short descriptor rendered in parentheses after
	// the goatnet name in email subjects/titles (e.g. "integration gateway");
	// Purpose is a sentence-or-three paragraph rendered as an "About this
	// environment" block in email bodies. Per-goatnet display values; both
	// empty = today's name-only rendering, never a fabricated description.
	// NotifyGoatnet is the goatnet this console SERVES (its site_id). It is the
	// fallback identity for emails to people who have
	// no onboarding proposal row — a ported / hand-created / pre-buildout
	// identity. Without it those emails render "the data fabric" instead of
	// the environment's name, because the proposal row is the only other source.
	NotifyGoatnet        string
	NotifyGoatnetTagline string
	NotifyGoatnetPurpose string

	// OnboardAdminNotifyEmails is who gets told a candidate has been vouched
	// for and is waiting on the ADMIN gate. There is no admin "group address"
	// derivable from the identity plane — the ADMINS group is a login-group
	// name, not a mailbox — so the list is configured. Empty = the admin
	// notification is off, and the queue stays pull-only.
	OnboardAdminNotifyEmails []string

	// SectorsNongovEnabled exposes + accepts the demand-gated non-governmental
	// affiliation.sector cluster (ngo / media / academic / commercial) at intake
	// (docs/adr/1077, ops/identity/grammar.yaml gated_values). Per-goatnet toggle,
	// default OFF — the governmental cluster (military / law-enforcement /
	// civil-government / intelligence) is always live. Flip on demand when a
	// goatnet onboards non-gov participants it wants to sector-classify.
	SectorsNongovEnabled bool

	// --- SPIFFE mTLS on the mint-capable onboarding leg (runner → console) ---
	// Additive, default OFF. When enabled, the console starts a SECOND listener
	// (MTLSOnboardAddr) that requires a client SVID whose SPIFFE ID is in
	// MTLSOnboardAllowedClientIDs — cryptographic transport auth for the
	// onboard-runner, replacing "membership on the mgmt bridge + a static
	// bearer". The plaintext :8080 listener is unchanged; the bearer still
	// carries per-handle attribution. SVID files are written by a
	// spiffe-helper sidecar (ADR 0412, SPIRE Phase A). See the tee-up
	// docs/design/runner-console-mtls.md.
	MTLSOnboardEnabled          bool
	MTLSOnboardAddr             string   // e.g. ":9443"
	MTLSOnboardCertFile         string   // console SVID cert (spiffe-helper output)
	MTLSOnboardKeyFile          string   // console SVID key
	MTLSOnboardBundleFile       string   // SPIRE trust bundle
	MTLSOnboardAllowedClientIDs []string // runner SPIFFE ID(s) allowed to mint

	// RepoAccessConfigPath points at the declarative role/identity → GitHub repo
	// collaborator policy (docs/design/onboarding-github-repo-grants.md). The
	// console reads it fresh at each provision. Absent file = feature off.
	RepoAccessConfigPath string
	// GitHubOnboardToken is the fine-grained PAT (Administration: write on the
	// listed repos) used to add a collaborator at onboarding. Empty → repo grants
	// are skipped (logged); the onboard proceeds. Never in a committed file.
	GitHubOnboardToken string

	// Dev-only flags. Refuse to start if any are true outside dev.
	DevBypassAuth bool
}

func Load() (*Config, error) {
	c := &Config{
		ListenAddr:          envOr("LISTEN_ADDR", ":8080"),
		IdPs:                map[string]IdP{},
		PermissionsYAMLPath: envOr("PERMISSIONS_YAML_PATH", "/etc/goat-id-console/group-permissions.yml"),
		HoldRecordsPath:     envOr("HOLD_RECORDS_PATH", "/var/lib/peerview/hold-records.jsonl"),
		MembershipScopePath: strings.TrimSpace(os.Getenv("MEMBERSHIP_SCOPE_PATH")),
		MembershipAuthority: strings.TrimSpace(os.Getenv("MEMBERSHIP_AUTHORITY")),
		PeerHandshakesPath:  os.Getenv("PEER_HANDSHAKES_PATH"),
		CoalitionFrameworksPath: envOr("COALITION_FRAMEWORKS_PATH",
			"/etc/goat-id-console/coalition-frameworks.yaml"),
		CoalitionFramework: os.Getenv("COALITION_FRAMEWORK"),
		OperatingSovereign: strings.ToUpper(strings.TrimSpace(os.Getenv("OPERATING_SOVEREIGN"))),
		StepUpReturnToBase: envOr("STEP_UP_RETURN_TO_BASE", "https://admin.auth.goat.example"),
		StepUpEnforced:     envBoolDefault("STEP_UP_ENFORCED", true),
		LogLevel:           envOr("LOG_LEVEL", "info"),
		AuditBackend:       envOr("AUDIT_BACKEND", "stderr"),
		AuditFilePath:      envOr("AUDIT_FILE_PATH", "/var/lib/goat-id-console/audit.jsonl"),
		DevBypassAuth:      envBool("DEV_BYPASS_AUTH"),
	}

	freshSec, err := strconv.Atoi(envOr("AAL2_FRESH_WINDOW_SEC", "300"))
	if err != nil {
		return nil, fmt.Errorf("AAL2_FRESH_WINDOW_SEC: %w", err)
	}
	c.AAL2FreshWindow = time.Duration(freshSec) * time.Second

	// Tenant IdP is always present — every site has at least one Kratos.
	tenant := IdP{
		Key:            "tenant",
		Label:          envOr("IDP_TENANT_LABEL", "Site users"),
		KratosAdminURL: envOr("IDP_TENANT_KRATOS_ADMIN_URL", "http://127.0.0.1:4434"),
		HydraAdminURL:  envOr("IDP_TENANT_HYDRA_ADMIN_URL", "http://127.0.0.1:4445"),
	}
	c.IdPs[tenant.Key] = tenant
	c.IdPOrder = []string{tenant.Key}
	c.DefaultIdP = tenant.Key

	// Operator IdP is OPTIONAL — gated on IDP_OPERATOR_KRATOS_ADMIN_URL being
	// set explicitly. Per ADR 0842, the operator IdP runs on Orin-C in the
	// three-Orin topology; single-node trial sites don't have it.
	// Omitting the env var hides the dropdown entry instead of surfacing a
	// 502 from an unresolvable hostname.
	if v := os.Getenv("IDP_OPERATOR_KRATOS_ADMIN_URL"); v != "" {
		operator := IdP{
			Key:            "operator",
			Label:          envOr("IDP_OPERATOR_LABEL", "Operators"),
			KratosAdminURL: v,
			HydraAdminURL:  envOr("IDP_OPERATOR_HYDRA_ADMIN_URL", "http://127.0.0.1:4447"),
		}
		c.IdPs[operator.Key] = operator
		c.IdPOrder = append(c.IdPOrder, operator.Key)
	}

	c.StepUpSSOIdP = envOr("STEP_UP_SSO_IDP", c.DefaultIdP)
	if _, ok := c.IdPs[c.StepUpSSOIdP]; !ok {
		return nil, fmt.Errorf("STEP_UP_SSO_IDP %q is not a configured IdP", c.StepUpSSOIdP)
	}

	c.OnboardDBDSN = strings.TrimSpace(os.Getenv("ONBOARD_DB_DSN"))
	c.OrgDBDSN = strings.TrimSpace(os.Getenv("ORG_DB_DSN"))
	c.OrgMatchEnabled = envBoolDefault("ORG_MATCH_ENABLED", false)
	c.RequireRegisteredOrg = envBoolDefault("ONBOARD_REQUIRE_REGISTERED_ORG", false)
	c.OnboardOperatorAPIKeys = parseHandleKeyPairs(envOr("ONBOARD_OPERATOR_API_KEYS", ""))
	c.IntakeBaseURL = envOr("INTAKE_BASE_URL", "")
	c.ConsolePublicURL = envOr("CONSOLE_PUBLIC_URL", "")
	c.PortalPublicURL = envOr("PORTAL_PUBLIC_URL", "")
	c.CollabDoorURL = envOr("COLLAB_KANIDM_URL", "")
	c.CollabDoorAdminPW = envOr("COLLAB_KANIDM_IDM_ADMIN_PW", "")
	c.PublicDoorURL = envOr("PUBLIC_DOOR_KANIDM_URL", "")
	// The role writes GOAT_IDM_ADMIN_PW whatever name the deployment gives the
	// variable on its controllers (public_door_idm_admin_pw_env).
	c.PublicDoorAdminPW = envOr("GOAT_IDM_ADMIN_PW", "")
	c.PortalInternalURL = envOr("PORTAL_INTERNAL_URL", "")
	c.PortalLookupToken = envOr("PORTAL_LOOKUP_TOKEN", envOr("NOTIFY_TOKEN", ""))

	c.SMTPHost = envOr("SMTP_HOST", "")
	c.SMTPPort = envOr("SMTP_PORT", "587")
	c.SMTPUsername = envOr("SMTP_USERNAME", "")
	c.SMTPPassword = envOr("SMTP_PASSWORD", "")
	// Trimmed here, not only in mailer.New: validating the raw value would
	// judge a string the mailer never sends. A padded address would be refused
	// though it sends fine trimmed, and a whitespace-only value would pass as
	// "set" though the mailer reads it as unset.
	c.SMTPFrom = strings.TrimSpace(envOr("SMTP_FROM", ""))
	c.SMTPReplyTo = strings.TrimSpace(envOr("SMTP_REPLY_TO", ""))
	// Both go raw into a header line, and the Reply-To is printed in the
	// invite body as the place questions go. A value that is not one address
	// would ship a malformed header on every invite and defeat the reply
	// route, so refuse to start on it; a typo is caught at deploy, not by the
	// first candidate who hits Reply.
	for _, v := range []struct{ name, value string }{
		{"SMTP_FROM", c.SMTPFrom},
		{"SMTP_REPLY_TO", c.SMTPReplyTo},
	} {
		if v.value == "" {
			continue
		}
		if _, err := mail.ParseAddress(v.value); err != nil {
			return nil, fmt.Errorf("%s %q is not a single RFC 5322 address: %w", v.name, v.value, err)
		}
	}

	c.SESAPIRegion = envOr("SES_API_REGION", "")
	c.SESAPIAccessKeyID = envOr("SES_API_ACCESS_KEY_ID", "")
	c.SESAPISecretAccessKey = envOr("SES_API_SECRET_ACCESS_KEY", "")
	c.RepoAccessConfigPath = envOr("REPO_ACCESS_CONFIG", "/etc/goat-id-console/repo-access.yaml")
	c.GitHubOnboardToken = os.Getenv("GITHUB_ONBOARD_TOKEN")
	c.NotifyToken = envOr("NOTIFY_TOKEN", "")
	c.NotifyMgmtURL = envOr("NOTIFY_MGMT_URL", "")
	c.SSHKeyTTLDays = envIntOr("SSH_KEY_TTL_DAYS", 90)
	c.OperatorKeyDir = os.Getenv("OPERATOR_KEY_DIR")
	c.EnrollWatchToken = envOr("ENROLL_WATCH_NB_TOKEN", "")
	c.EnrollWatchInterval = envDurationOr("ENROLL_WATCH_INTERVAL", 5*time.Minute)
	// Default ON at seven days. This only moves a DEAD link between two lists
	// in the console's own UI — it revokes nothing, sends nothing and changes
	// no access — so it needs no opt-in. `off` (or `never`) stops it.
	switch strings.ToLower(strings.TrimSpace(os.Getenv("ONBOARD_INVITE_ARCHIVE_AFTER"))) {
	case "off", "never", "disabled":
		c.InviteArchiveAfter = 0
	default:
		c.InviteArchiveAfter = envDurationOr("ONBOARD_INVITE_ARCHIVE_AFTER", 7*24*time.Hour) // = onboardstore.DefaultInviteArchiveAfter
	}
	// Default ON at ten minutes. This only recovers a row a runner failed to
	// finish claiming — it mints or revokes nothing itself — so it needs no
	// opt-in. `off` (or `never`) stops it.
	switch strings.ToLower(strings.TrimSpace(os.Getenv("ONBOARD_CLAIM_REQUEUE_AFTER"))) {
	case "off", "never", "disabled":
		c.ClaimRequeueAfter = 0
	default:
		c.ClaimRequeueAfter = envDurationOr("ONBOARD_CLAIM_REQUEUE_AFTER", 10*time.Minute) // = onboardstore.DefaultClaimRequeueAfter
	}
	c.NotifyGetURL = envOr("NOTIFY_GET_URL", "")
	c.NotifyPortalURL = envOr("NOTIFY_PORTAL_URL", "")
	c.GuideRepoBaseURL = envOr("GUIDE_REPO_BASE_URL", "")
	c.PlatformRepoURL = strings.TrimRight(envOr("PLATFORM_REPO_URL", "https://github.com/usareur-af/goatcore"), "/")
	c.CollabURL = strings.TrimRight(envOr("NOTIFY_COLLAB_URL", "https://collab.goat.example"), "/")
	c.NotifyRelayHealthURL = envOr("NOTIFY_RELAY_HEALTH_URL", "")
	c.NotifyMapURL = envOr("NOTIFY_MAP_URL", "")
	c.NotifyRelayAddresses = parseCommaList(envOr("NOTIFY_RELAY_ADDRESSES", ""))
	c.NotifyStunHosts = parseCommaList(envOr("NOTIFY_STUN_HOSTS", ""))
	c.NotifyMintOnCollect = envBool("NOTIFY_MINT_ON_COLLECT")
	c.NotifyRosenpass = envBool("NOTIFY_ROSENPASS")
	c.NotifyGoatnet = envOr("NOTIFY_GOATNET", "")
	c.NotifyGoatnetTagline = envOr("NOTIFY_GOATNET_TAGLINE", "")
	c.NotifyGoatnetPurpose = envOr("NOTIFY_GOATNET_PURPOSE", "")
	c.OnboardAdminNotifyEmails = parseCommaList(envOr("ONBOARD_ADMIN_NOTIFY_EMAILS", ""))
	c.SectorsNongovEnabled = envBool("ONBOARD_SECTORS_NONGOV_ENABLED")

	c.MTLSOnboardEnabled = envBool("MTLS_ONBOARD_ENABLED")
	c.MTLSOnboardAddr = envOr("MTLS_ONBOARD_ADDR", ":9443")
	c.MTLSOnboardCertFile = envOr("MTLS_ONBOARD_CERT_FILE", "/svid/goat-id-console-cert.pem")
	c.MTLSOnboardKeyFile = envOr("MTLS_ONBOARD_KEY_FILE", "/svid/goat-id-console-key.pem")
	c.MTLSOnboardBundleFile = envOr("MTLS_ONBOARD_BUNDLE_FILE", "/svid/spire-bundle.pem")
	c.MTLSOnboardAllowedClientIDs = parseCommaList(os.Getenv("MTLS_ONBOARD_ALLOWED_CLIENT_IDS"))

	return c, nil
}

func (c *Config) IdP(key string) (IdP, bool) {
	idp, ok := c.IdPs[key]
	return idp, ok
}

func envOr(k, fallback string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return fallback
}

func envBool(k string) bool {
	v := os.Getenv(k)
	return v == "1" || v == "true" || v == "yes"
}

// envBoolDefault reads a boolean env var that defaults to def when unset.
// Used for fail-closed policy flags (e.g. STEP_UP_ENFORCED) where absence
// must mean the safe value, not false.
func envBoolDefault(k string, def bool) bool {
	v := os.Getenv(k)
	if v == "" {
		return def
	}
	return v == "1" || v == "true" || v == "yes"
}

// parseCommaList splits "a, b ,c" into ["a","b","c"], dropping blanks.
func parseCommaList(raw string) []string {
	var out []string
	for _, p := range strings.Split(raw, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}

// parseHandleKeyPairs parses "handle1:key1,handle2:key2" into a map keyed by KEY
// → handle (the resolve direction: a presented bearer key yields its handle).
// Blank pairs, pairs missing the ':', and empty handle/key are skipped. Mirrors
// the portal's PORTAL_OPERATOR_API_KEYS parsing.
func parseHandleKeyPairs(raw string) map[string]string {
	out := map[string]string{}
	for _, pair := range strings.Split(raw, ",") {
		pair = strings.TrimSpace(pair)
		if pair == "" {
			continue
		}
		handle, key, ok := strings.Cut(pair, ":")
		handle = strings.TrimSpace(handle)
		key = strings.TrimSpace(key)
		if !ok || handle == "" || key == "" {
			continue
		}
		out[key] = handle
	}
	return out
}

// envDurationOr reads a Go duration string (e.g. "5m", "30s"). An unset or
// unparseable value falls back rather than failing boot: this only paces a
// reveal-only watcher, and a typo'd interval must not keep the console down.
func envDurationOr(k string, fallback time.Duration) time.Duration {
	v := os.Getenv(k)
	if v == "" {
		return fallback
	}
	d, err := time.ParseDuration(v)
	if err != nil || d <= 0 {
		return fallback
	}
	return d
}

// envIntOr reads a non-negative integer, falling back rather than failing boot:
// this only sets a recorded expiry window, and a typo must not keep the console
// down.
func envIntOr(k string, fallback int) int {
	v := strings.TrimSpace(os.Getenv(k))
	if v == "" {
		return fallback
	}
	n, err := strconv.Atoi(v)
	if err != nil || n < 0 {
		return fallback
	}
	return n
}
