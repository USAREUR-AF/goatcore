// Package welcome renders a complete, tier-correct onboarding "welcome package"
// for a freshly-provisioned enrollee: how to join the mesh, set a login, install
// the CLI, reach every surface their tier grants, and (for publishers) publish to
// the data fabric. It is the single source of truth for "what a tier gets", so no
// human — and no assistant session — hand-assembles this and drops a piece.
//
// Deliberately derived from the same three URLs the onboarded email already uses
// (mgmt / get / portal, console config): a goatnet configures the surfaces it has,
// and every other surface URL is derived from the portal host's shared base. Pure
// and dependency-free so it is trivially testable and reusable by both the email
// path and the console download endpoint.
package welcome

import (
	"net/url"
	"path"
	"strings"
	"time"
)

// Catalog is the per-goatnet set of service URLs a welcome package points at.
type Catalog struct {
	Goatnet      string // display name, e.g. "acme"
	MgmtURL      string // netbird enrollment, e.g. https://netbird.example.net
	GetURL       string // cli download surface, e.g. https://get.site.netbird.example.net
	PortalURL    string // portal home
	AuthURL      string // Kratos public base for the login / recovery flow
	Panoscope    string
	Flashlight   string
	Stethoscope  string
	Sandbox      string
	Docs         string
	IsItUp       string
	Chat         string
	AdminConsole string // admin.auth.<base> — only surfaced for admin tiers
	Microscope   string // per-entity behavior lens — admin-fenced
	Grafana      string // metrics dashboards — admin-fenced at the OAuth handshake
	Dashboard    string // netbird dashboard — only referenced in the operator section
	RelayHealth  string // public relay-health dashboard — surfaced only when configured (NOTIFY_RELAY_HEALTH_URL)
	// MintOnCollect mirrors this goatnet's onboard_runner_defer_credential:
	// true = the person mints their portal key + bundle themselves on
	// /fabric-credential; false = provisioning minted for them and the welcome
	// email carries the key inline plus a one-time bundle link. The guide's
	// credential step and the fabric deep dive both turn on it, so a goatnet
	// cannot describe a flow it does not run. Default false = the older,
	// wider-deployed shape.
	MintOnCollect bool

	// Egress is what a device on this goatnet dials outbound — the management
	// host, the relays and the STUN hosts, with their resolved public
	// addresses. Built from NOTIFY_RELAY_ADDRESSES + NOTIFY_STUN_HOSTS by the
	// handler; empty renders the firewall topic without an address table.
	Egress []EgressHost
	// Network is what the reader's own site network must allow (names and
	// ports only). Nil omits the welcome email's network block.
	Network *NetworkRequirements

	// Map is the geographic map surface. CONFIGURED (NOTIFY_MAP_URL), never
	// derived: only some goatnets carry a feed worth mapping, and deriving it
	// would print a confident link to a host that does not exist on the rest.
	// Same pattern as RelayHealth; blank renders "not configured on this goatnet".
	Map string
	// Guide is the mesh-only onboarding guide page — the post-mesh half of this
	// package (internal/welcome/guide.go). When set, the email drops the sections
	// that are useless before the mesh is up (surfaces tour, CLI install, fabric,
	// operator setup) and points here instead, so the credentials stop competing
	// with a walkthrough for the reader's attention. Blank → the email renders
	// those sections inline exactly as before, so a goatnet that has not deployed
	// the guide is unaffected. That makes this reversible by CONFIG, not a revert.
	Guide string

	// RosenpassEnabled, when true, makes the mesh-enrollment command include
	// --enable-rosenpass --rosenpass-permissive so the peer's WireGuard sessions
	// are post-quantum-secured (ADR 0602). Per-goatnet, default off.
	RosenpassEnabled bool

	// Tagline is the short per-goatnet descriptor rendered in parentheses after
	// the goatnet name (e.g. "integration gateway" → "ACME (integration
	// gateway)") in email subjects and the environment block. Purpose is the
	// sentence-or-three "what this environment is for" paragraph rendered as an
	// "About this environment" block near the top of every package, so a person
	// onboarded to more than one goatnet always
	// knows WHICH one an email is about. Both per-goatnet config; both blank →
	// name-only rendering, never a fabricated description.
	Tagline string
	Purpose string

	// RepoURL is the platform source repository (PLATFORM_REPO_URL): the
	// operator section's admin-guide link and git clone come from it. Blank
	// renders the guide by name and asks the operator for the clone URL.
	RepoURL string
	// CollabURL is the collaboration environment's front door
	// (NOTIFY_COLLAB_URL). Blank renders the collab block without an address.
	CollabURL string
}

// EnrollCommand returns the one-time `sudo netbird up` mesh-enrollment command a
// goatnet hands a new peer. It is the single source of truth for that command
// across the console welcome package and the onboarded-email path, so the
// Rosenpass flags can never be applied on one surface and dropped on the other.
// When rosenpass is true it appends --enable-rosenpass --rosenpass-permissive
// (permissive = still connects to peers that don't yet have Rosenpass, so
// enabling it never breaks connectivity).
func EnrollCommand(mgmtURL, setupKey string, rosenpass bool, goatnet string) string {
	return EnrollCommandNamed(mgmtURL, setupKey, rosenpass, "", goatnet)
}

// EnrollCommandNamed is EnrollCommand plus a canonical peer name.
//
// THREE THINGS THIS FIXES.
//
// 1. NAMING. Without --hostname, NetBird names the peer after the
// machine's own hostname, which is how a goatnet acquires peers called "Mac"
// and "iPhone". That is not untidiness: under setup-key enrollment
// NetBird leaves peer.user_id EMPTY, so the peer NAME is the only link from a
// mesh peer to the principal that owns it (ops/identity/peer-name-grammar.yaml).
// A peer called "Mac" is UNATTRIBUTABLE — the audit cannot place it and a
// revoke cannot find it. namePrefix should be the grammar's
// peer-<handle>[-<org_short_tag>]-<site>-<date>; the machine appends its own
// short hostname, because only the machine knows it.
//
// 2. CLEARING ROSENPASS. When rosenpass is false this now emits
// --enable-rosenpass=false EXPLICITLY rather than simply omitting the flag.
// Omitting it does NOT clear a previously-set flag: the setting persists in the
// client's local config across netbird down/up AND across re-enrollment with a
// fresh setup key. A full down / up --setup-key cycle leaves "Quantum resistance:
// true (permissive)" intact, and only the explicit =false clears it. So a
// revoke-and-remint remediation that relies on omission silently achieves nothing.
//
// 3. NOT REUSING WHATEVER PROFILE IS ACTIVE (PR #1336 review). With no
// --profile, `netbird up` reuses the invoking user's currently active NetBird
// profile and silently overwrites its management URL and setup key — a device
// already pointed at a personal account or a different goatnet loses that
// config with no warning, and (pre-v0.78 clients) a caller running under sudo
// resolves that "active profile" against root's own state rather than the
// invoking user's, so the switch performed a moment earlier as a normal user
// is invisible to it. `--profile <goatnet>` sidesteps both: `up` itself
// switches to (or creates) the named profile, so nothing else is overwritten,
// and running it again is a no-op switch rather than a second,
// ambiguously-named profile (switchOrCreateProfile, NetBird v0.73.1+).
// Whose profile it is depends on the client. From v0.78 `up` resolves the
// sudo invoking user (SUDO_USER); before that it uses the process user, which
// under sudo is root, so the profile lands in root's store
// (/var/lib/netbird/root/) and the enrollee's own `netbird profile list` does
// not show it. The enrollment works either way. Floor: --profile on `up` needs
// v0.52.0+ (unknown flag before), and v0.73.0 alone refuses a profile that
// does not exist yet. This is now the ONLY enrollment command every surface
// emits, so every "run step 5 again" fix elsewhere (note A, note C, the hosted
// guide) inherits it for free.
func EnrollCommandNamed(mgmtURL, setupKey string, rosenpass bool, namePrefix, goatnet string) string {
	cmd := "sudo netbird up --profile " + profileID(goatnet) + " --management-url " + mgmtURL + " --setup-key " + setupKey
	if namePrefix != "" {
		// $(hostname -s) expands on the enrolling machine, which is the only
		// place the machine's own name is known.
		cmd += " --hostname \"" + namePrefix + "-$(hostname -s)\""
	}
	return cmd + rosenpassFlags(rosenpass)
}

// WindowsEnrollCommand is EnrollCommand for an administrator Command Prompt or
// PowerShell: same flags, no `sudo` (neither shell has it — an enrollee on
// had to strip it from six commands by trial and error). This
// unnamed form is the one every email and guide emits; it works in either
// shell.
func WindowsEnrollCommand(mgmtURL, setupKey string, rosenpass bool, goatnet string) string {
	return WindowsEnrollCommandNamed(mgmtURL, setupKey, rosenpass, "", goatnet)
}

// WindowsEnrollCommandNamed is EnrollCommandNamed for Windows, kept for
// symmetry: no production caller passes a prefix today (the same is true of
// EnrollCommandNamed). If one ever does, the machine's short name is
// %COMPUTERNAME%, which cmd.exe expands but PowerShell keeps literal (it
// spells the variable $env:COMPUTERNAME); `$(hostname -s)` is POSIX-shell
// syntax and would be kept literal on both. That is the only reason the
// email points Windows readers at Command Prompt rather than PowerShell.
// --profile carries no sudo/root wrinkle here: an elevated Command Prompt
// still runs as the same account, unlike macOS/Linux sudo.
func WindowsEnrollCommandNamed(mgmtURL, setupKey string, rosenpass bool, namePrefix, goatnet string) string {
	cmd := "netbird up --profile " + profileID(goatnet) + " --management-url " + mgmtURL + " --setup-key " + setupKey
	if namePrefix != "" {
		cmd += " --hostname \"" + namePrefix + "-%COMPUTERNAME%\""
	}
	return cmd + rosenpassFlags(rosenpass)
}

// profileID returns a NetBird profile identifier safe to splice, unquoted,
// into a shell command. A blank goatnet (Catalog{}) would otherwise land in
// the command as an empty --profile argument, and a multi-word one as several;
// renderOperatorSetup's `if env == "" { env = "<env>" }` is the same guard for
// the same reason. Callers resolve the goatnet before this point
// (handlers.emailGoatnet falls back to NOTIFY_GOATNET), because the
// placeholder is a shell input redirect and the pasted command fails on it.
func profileID(goatnet string) string {
	g := strings.TrimSpace(goatnet)
	if g == "" || strings.ContainsAny(g, " \t\n") {
		return "<goatnet>"
	}
	return g
}

func rosenpassFlags(rosenpass bool) string {
	if rosenpass {
		return " --enable-rosenpass --rosenpass-permissive"
	}
	return " --enable-rosenpass=false --rosenpass-permissive=false"
}

// renderEnrollLine prints step 5's enrollment line for one OS block. build is
// EnrollCommand or WindowsEnrollCommand; the key/URL fallbacks are identical
// for both so a keyless or URL-less package degrades the same way everywhere.
func renderEnrollLine(d *doc, cat Catalog, a Artifacts, build func(mgmtURL, setupKey string, rosenpass bool) string) {
	switch {
	case a.SetupKey != "" && cat.MgmtURL != "":
		d.cmd("     " + build(cat.MgmtURL, a.SetupKey, cat.RosenpassEnabled))
	case a.SetupKey != "":
		d.cred("     ", "Your mesh enrollment key", a.SetupKey)
		if cat.MgmtURL != "" {
			d.cmd("     " + build(cat.MgmtURL, a.SetupKey, cat.RosenpassEnabled))
		}
	case cat.MgmtURL != "":
		// No key yet, but we know where the mesh lives — show the enrollment target
		// so a keyless enrollee isn't dead-ended waiting on an operator with no URL.
		d.cont("     Your operator will hand you a one-time mesh enrollment key, then run:")
		d.cmd("       " + build(cat.MgmtURL, "<key>", cat.RosenpassEnabled))
	default:
		d.cont("     Your operator will hand you a one-time mesh enrollment key.")
	}
}

// renderHandsetKeyLine prints the key a phone or tablet enrolls with. The
// handset key is the one to hand over; without it the device key still works
// on a phone, but a phone on that key holds every group the laptop holds, so
// the fallback says so rather than silently putting a handset in an admin's
// groups (the shape panoscope's D5 check alarms on).
func renderHandsetKeyLine(d *doc, cat Catalog, a Artifacts) {
	switch {
	case a.HandsetKey != "":
		d.cred("     ", "Your handset key", a.HandsetKey)
	case a.SetupKey != "":
		d.cred("     ", "Your mesh enrollment key (also works on a phone)", a.SetupKey)
		d.para("     A phone on this key holds the same access as your laptop. If you are a")
		d.para("     platform admin, ask your operator for a handset key instead.")
	case cat.MgmtURL != "":
		d.cont("     Your operator will hand you a handset key; enter it here.")
	default:
		d.cont("     Your operator will hand you a handset key.")
	}
}

// renderPhoneBlock is STEP 1's phone-or-tablet variant: the NetBird app from
// the store, the management server, the handset key. Skipped when the manifest
// declared no handsets and no handset key exists.
func renderPhoneBlock(d *doc, cat Catalog, r Recipient, a Artifacts, goatnet string) {
	if r.NoHandset && a.HandsetKey == "" {
		return
	}
	mgmt := cat.MgmtURL
	if mgmt == "" {
		mgmt = "the " + goatnet + " management server your operator names"
	}
	d.para("Phone or tablet (iPhone, iPad, Android) — no commands:")
	d.blank()
	d.step("1) Install NetBird from the App Store or Google Play.")
	d.step("2) Open the app, choose \"Change Server\", and enter the management URL:")
	d.cmd("     " + mgmt)
	d.step("3) Enter your handset key:")
	renderHandsetKeyLine(d, cat, a)
	d.step("4) Tap Connect and accept the VPN prompt. One VPN at a time; see the phones note below.")
	d.blank()
}

// DeriveCatalog builds a Catalog from the three configured URLs. Every derived
// surface URL comes from the portal host's shared base (portal.<base> →
// <svc>.<base>), so a goatnet only configures the portal/get/mgmt URLs it already
// does. A blank portal URL leaves the derived surfaces blank (rendered as
// "ask your operator"), never a fabricated hostname.
func DeriveCatalog(goatnet, mgmtURL, getURL, portalURL string) Catalog {
	base := baseFromPortal(portalURL)
	svc := func(name string) string {
		if base == "" {
			return ""
		}
		return "https://" + name + "." + base
	}
	c := Catalog{
		Goatnet:     goatnet,
		MgmtURL:     strings.TrimSpace(mgmtURL),
		GetURL:      strings.TrimSpace(getURL),
		PortalURL:   strings.TrimSpace(portalURL),
		AuthURL:     svc("auth"),
		Panoscope:   svc("panoscope"),
		Flashlight:  svc("flashlight"),
		Stethoscope: svc("stethoscope"),
		Sandbox:     svc("sandbox"),
		Docs:        svc("docs"),
		IsItUp:      svc("isitup"),
		// No mesh-hosted chat: the team chat (Zulip) is off-mesh on the collab
		// sign-in and is introduced in the collab paragraph, not as a mesh
		// surface. The Mattermost mesh alias was removed .
		Chat:         "",
		AdminConsole: svc("admin.auth"),
		Microscope:   svc("microscope"),
		Grafana:      svc("grafana"),
		Dashboard:    svc("dashboard"),
		// The guide host serves exactly one path (its Traefik router pins
		// PathPrefix(/public/onboarding-guide)), so the path belongs in the
		// derived URL — a bare host would 404.
		Guide: guideURL(svc("guide")),
	}
	return c
}

// guideURL appends the guide's fixed path to its derived host. Blank host (no
// portal configured) stays blank, so the email renders no pointer rather than a
// link to nowhere.
func guideURL(host string) string {
	if host == "" {
		return ""
	}
	return host + "/public/onboarding-guide"
}

// GuidePointer returns the best URL for the guide on this goatnet.
//
// Preferred is the dedicated guide host, which is unauthenticated: anyone who
// can reach the mesh can read it. Where a goatnet has not got one, the console
// serves the same handler at the same path, so fall back to that rather than
// rendering no pointer — a reader behind the console's login is still better
// off than one sent nowhere. Blank only when neither is configured.
func GuidePointer(c Catalog) string {
	if g := strings.TrimSpace(c.Guide); g != "" {
		return g
	}
	if ac := strings.TrimSpace(c.AdminConsole); ac != "" {
		return strings.TrimRight(ac, "/") + "/public/onboarding-guide"
	}
	return ""
}

// baseFromPortal returns the shared surface base from a portal URL:
// https://portal.site.netbird.example.net → site.netbird.example.net.
func baseFromPortal(portalURL string) string {
	u, err := url.Parse(strings.TrimSpace(portalURL))
	if err != nil || u.Host == "" {
		return ""
	}
	if strings.HasPrefix(u.Host, "portal.") {
		return strings.TrimPrefix(u.Host, "portal.")
	}
	return u.Host
}

// Recipient is the provisioned enrollee the package is for. The booleans are the
// tier→capability decision, resolved once by the caller from the manifest + tier,
// so the renderer stays a pure formatter.
type Recipient struct {
	Name   string
	Handle string
	Email  string
	Tier   string   // browse | developer | iam-admin | platform-admin
	Groups []string // netbird auto_groups (mesh role), display only
	// NoHandset is true when the manifest says netbird.handset_limit: 0 — the
	// person declared no phone or tablet. The phone block is then omitted unless
	// a handset key exists anyway. The zero value (field absent, or a manifest
	// that predates the field) keeps the block: the default limit is 1.
	NoHandset bool
	// AffiliationKind is the manifest's affiliation.kind (organic | vendor |
	// partner | civil), threaded through so the email can point a partner-org
	// member at the guide's partner view. DISPLAY-ONLY routing: affiliation
	// never gates anything (identity-grammar rule), and a blank value renders
	// the ordinary email — never a guess.
	AffiliationKind string
	FabricAccess    bool // has a Zenoh identity (cert bundle) — publisher or read/subscribe
	Publisher       bool // may WRITE to the Zenoh data fabric (implies FabricAccess)
	AdminConsole    bool // tier grants the admin console

	// GitHubUsername is the enrollee's GitHub login (identity.github), when
	// captured — surfaced on the summary line + the repo-access section.
	GitHubUsername string
	// RepoGrants are the GitHub repo-collaborator grants attempted at provision
	// (docs/design/onboarding-github-repo-grants.md), each with its outcome. The
	// "Repository access" section lists them so the person knows what to accept.
	RepoGrants []RepoGrant
	// AdminOperator is set for the escalated admin tiers (iam-admin /
	// platform-admin): they get an extra "Operator setup" section pointing at the
	// out-of-band operator bundle + the admin CLI capabilities.
	AdminOperator bool
	// MgmtHost is the mgmt host an operator SSHes into (their SSH key installed
	// out-of-band). Optional — a blank value renders a generic placeholder rather
	// than a fabricated address.
	MgmtHost string
	// MgmtSSHUser is the SHARED operator account used to SSH the mgmt host (NOT the
	// per-handle login — everyone shares one account, and commands there are not
	// attributed). Per-goatnet, so it is config, never a hardcoded default:
	// a blank value renders a generic "the shared account your operator gives you"
	// instruction rather than fabricating a per-handle login.
	MgmtSSHUser string
}

// RepoGrant is one GitHub repo-collaborator grant surfaced in the welcome
// package: the repo, the permission level, the outcome (added/invited/failed) of
// the provision-time attempt, and the Source that put it on the list
// ("policy" from the on-rails engine, or "audible" from a per-onboardee override).
type RepoGrant struct {
	Owner      string
	Repo       string
	Permission string
	Outcome    string
	// Source is "policy" | "audible" | "" (legacy rows / pre-audible grants,
	// which render with no source tag).
	Source string
	// HTTPStatus is the status GitHub answered a FAILED grant with (0 when the
	// request never got an answer, and on every success). Failure is a short
	// safe class derived from it (repoaccess.FailureClass). Both are persisted
	// with the proposal so the durable record is enough to triage a refusal
	// without the console log (#2107). Neither carries a token or a body.
	HTTPStatus int    `json:",omitempty"`
	Failure    string `json:",omitempty"`
}

// Granted reports whether GitHub confirmed this grant: an invitation was
// created or the person already had access. Anything else, a recorded
// failure or a blank or unknown outcome, is not evidence of access.
func (g RepoGrant) Granted() bool {
	return g.Outcome == "added" || g.Outcome == "invited"
}

// Artifacts are the one-time secrets minted at provisioning. Either may be blank
// (not captured / minted); the renderer degrades to an instruction rather than a
// fabricated value.
type Artifacts struct {
	SetupKey string // NetBird enrollment key
	// HandsetKey is the second, OBSERVER-only NetBird key minted for the
	// person's phones and tablets (ADR 1175). Blank when the mint predates it or
	// the manifest's netbird.handset_limit is 0; the phone block then falls back
	// to the device key with a warning about the access it carries.
	HandsetKey   string
	RecoveryLink string // Kratos recovery link that sets the first password
	RecoveryCode string // the code entered at the recovery link (code strategy)
	// RecoveryExpiresAt is the deadline Kratos itself returned for that
	// link+code pair. Zero = not captured, and the copy then falls back to the
	// 24h the console's own mints get from Kratos's configured lifespan.
	//
	// It exists because the fallback is an ASSUMPTION about someone else's
	// config: the console sends no expires_in, so 24h is whatever kratos.yaml
	// says, and an operator minting out of band can pass any window they like.
	// A package that states a deadline the credential does not have sends the
	// reader away from a live credential, or to a dead one — so when Kratos has
	// told us the answer, print that.
	RecoveryExpiresAt time.Time
	BundleURL         string // mesh-only download link for the fabric cert bundle
	// PortalAPIKey is the scoped `vp_…` key that signs the person in to the
	// Portal. It is minted into their bundle, but until it was threaded through
	// here the email could only say "find it under your identity once you can
	// reach the Portal" — circular, because this key IS how you reach the
	// Portal. Blank for tiers that never mint one; the Portal section then says
	// so plainly instead of sending the person looking for something that does
	// not exist.
	PortalAPIKey string
	// CredentialDeferred says the mint DELIBERATELY issued no fabric credential
	// (mint-on-collect): the person's slot and identity exist, and they mint the
	// key + cert themselves from the console's fabric-credential page. Stated by
	// the mint rather than inferred from an empty PortalAPIKey, because that is
	// ALSO what a failed scrape looks like — and telling someone whose key is
	// sitting in their bundle to go and collect one sends them to a page that
	// refuses them. docs/design/fabric-credential-collection.md.
	CredentialDeferred bool
	// Collaboration environment (separate Dex+Kanidm plane). CollabAccount is the
	// sign-in username; CollabSetupLink is the one-time 24h credential-setup link.
	// Both blank unless the manifest opted in (collab.enabled) and provisioning
	// succeeded. With a link present the section walks the person through
	// set-up; with an account and no link (a download, a re-issue, or a
	// re-onboard of someone who kept their access) it names the account and
	// points at the console's /setup-code page instead.
	CollabAccount   string
	CollabSetupLink string
}

// Upgrade describes an access change applied to an EXISTING identity — the
// console group patch (an escalation like developer→platform-admin, or a
// reduction). PrevGroups/NewGroups are the Ory service-permission groups
// before and after the change. Rendering in upgrade mode keeps every section
// of the welcome package (the person still holds ONE complete reference for
// their role) but reframes it: nothing was re-provisioned, the already-done
// steps say so, and no one-time secrets are minted or embedded.
type Upgrade struct {
	PrevGroups []string
	NewGroups  []string
}

// Added returns the groups present after but not before the change.
func (u Upgrade) Added() []string { return diffMissing(u.NewGroups, u.PrevGroups) }

// Removed returns the groups present before but not after the change.
func (u Upgrade) Removed() []string { return diffMissing(u.PrevGroups, u.NewGroups) }

// diffMissing returns the members of xs absent from ys (case-insensitive,
// order-preserving).
func diffMissing(xs, ys []string) []string {
	var out []string
	for _, x := range xs {
		if !containsFoldTrim(ys, x) {
			out = append(out, x)
		}
	}
	return out
}

func containsFoldTrim(xs []string, want string) bool {
	for _, x := range xs {
		if strings.EqualFold(strings.TrimSpace(x), strings.TrimSpace(want)) {
			return true
		}
	}
	return false
}

// DisplayName is the human-facing label for a goatnet: the config name
// uppercased as a proper goatnet label (e.g. "acme" -> "ACME"), or the neutral
// "the data fabric" when no goatnet is configured. It is the single source of
// truth for how a goatnet is titled across the welcome body and the email
// subject, so the two never disagree on casing. It is display-only — the
// lowercase config value remains the identifier used for paths / `--env`.
func DisplayName(goatnet string) string {
	g := strings.TrimSpace(goatnet)
	if g == "" {
		return "the data fabric"
	}
	return strings.ToUpper(g)
}

// DisplayLabel is DisplayName plus the per-goatnet tagline in parentheses:
// ("acme", "integration gateway") → "ACME (integration gateway)". Single source
// of truth for the labeled form across email subjects and the environment
// block. A blank tagline — or no configured goatnet at all — degrades to
// DisplayName alone.
func DisplayLabel(goatnet, tagline string) string {
	name := DisplayName(goatnet)
	t := strings.TrimSpace(tagline)
	if t == "" || strings.TrimSpace(goatnet) == "" {
		return name
	}
	return name + " (" + t + ")"
}

// splitCredentialLink pulls a one-shot credential link apart into the page a
// reader opens and the code they paste there — the F-315 remedy.
//
// The failure it exists to prevent: a credential-setup link is single-use, and
// enterprise mail-security gateways FETCH links in transit to scan them, often
// with HEAD requests no browser sends. The scanner's fetch spends the one use,
// so the human who clicks afterwards is told the token was "reused in another
// browser" and cannot proceed.
//
// Splitting defeats it because neither half is both fetchable and spendable:
// the page address carries no token, so pre-fetching it does nothing, and the
// code is inert until a human submits it.
//
// Returns ("", "") when raw is not a URL carrying a `token` query parameter,
// which the caller MUST treat as "render the original link unchanged" — a
// credential that cannot be split is still a credential the reader needs.
func splitCredentialLink(raw string) (page, code string) {
	u, err := url.Parse(strings.TrimSpace(raw))
	if err != nil || u.Scheme == "" || u.Host == "" {
		return "", ""
	}
	code = u.Query().Get("token")
	if code == "" {
		return "", ""
	}
	u.RawQuery = ""
	u.Fragment = ""
	return u.String(), code
}

// wrapText greedily wraps s into lines of at most width runs of non-space
// characters joined by single spaces (plaintext email discipline: the purpose
// paragraph arrives as one long config string).
func wrapText(s string, width int) []string {
	var lines []string
	cur := ""
	for _, w := range strings.Fields(s) {
		switch {
		case cur == "":
			cur = w
		case len(cur)+1+len(w) <= width:
			cur += " " + w
		default:
			lines = append(lines, cur)
			cur = w
		}
	}
	if cur != "" {
		lines = append(lines, cur)
	}
	return lines
}

// Render produces the complete plaintext welcome package for a FRESHLY
// PROVISIONED enrollee. Section presence is driven by the Recipient's resolved
// capabilities, so a browse observer and a publishing developer get correctly
// different packages from the same code.
func Render(cat Catalog, r Recipient, a Artifacts) string {
	return render(cat, r, a, nil, false)
}

// RenderReissue is Render for a CORRECTED RE-SEND to someone who already
// received a welcome package. Identical content, plus a banner at the very top
// saying so — a second package landing unannounced reads as a second identity
// or a phishing attempt, and the enrollee cannot tell which of the two to
// follow. It exists because an incomplete package (most consequentially, one
// missing the mesh enrollment key) strands a person at step 1 with no recovery
// path other than a re-send.
func RenderReissue(cat Catalog, r Recipient, a Artifacts) string {
	return render(cat, r, a, nil, true)
}

// RenderUpgrade produces the plaintext package for a role/access change on an
// EXISTING identity: an upgrade acknowledgment + what-changed block, then the
// same complete role-appropriate reference a new enrollee at that level
// receives — from the same section code, so the two paths can never drift.
// One-time secrets never appear: Artifacts are deliberately absent (the person
// already enrolled, already holds a password and any fabric bundle).
func RenderUpgrade(cat Catalog, r Recipient, u Upgrade) string {
	return render(cat, r, Artifacts{}, &u, false)
}

// render is the single section renderer behind all three paths; up != nil
// switches the framing from "do these steps" to "these steps are already done",
// and reissue prepends the supersedes-the-earlier-email banner.
// Package is one welcome email in both presentations: Text is the record and
// the plain-text MIME part; HTML is the same document laid out for a mail
// client. Built from one document, so they cannot disagree.
type Package struct {
	Text string
	HTML string
}

// RenderPackage is Render in both presentations.
func RenderPackage(cat Catalog, r Recipient, a Artifacts) Package {
	d := build(cat, r, a, nil, false)
	return Package{Text: d.Text(), HTML: d.HTML()}
}

// RenderReissuePackage is RenderReissue in both presentations.
func RenderReissuePackage(cat Catalog, r Recipient, a Artifacts) Package {
	d := build(cat, r, a, nil, true)
	return Package{Text: d.Text(), HTML: d.HTML()}
}

// RenderUpgradePackage is RenderUpgrade in both presentations.
func RenderUpgradePackage(cat Catalog, r Recipient, u Upgrade) Package {
	d := build(cat, r, Artifacts{}, &u, false)
	return Package{Text: d.Text(), HTML: d.HTML()}
}

func render(cat Catalog, r Recipient, a Artifacts, up *Upgrade, reissue bool) string {
	return build(cat, r, a, up, reissue).Text()
}

func build(cat Catalog, r Recipient, a Artifacts, up *Upgrade, reissue bool) *doc {
	// goatnet is the DISPLAY name (title + mesh prose + upgrade header), shown
	// uppercased as a proper goatnet label (e.g. "ACME"). The lowercase config
	// value (cat.Goatnet) is still used as the `env` identifier for paths and
	// `--env` in renderOperatorSetup — those must NOT be uppercased.
	goatnet := DisplayName(cat.Goatnet)
	d := newDoc()
	// Troubleshooting notes are COLLECTED while the steps render and emitted
	// once, at the end, so the steps stay a short list of things to do and the
	// caveats (every one earned by a real support case) sit where a reader who
	// hit trouble goes looking. Each note is one block; letters are assigned
	// in emission order and the steps refer to them as "note A".

	// Reissue banner FIRST — above the name, the title, everything. Someone
	// reading only the first screen must learn this replaces what they already
	// have before they act on either copy.
	if reissue {
		d.rule()
		d.h2("REPEAT — THIS REPLACES THE WELCOME EMAIL YOU ALREADY RECEIVED")
		d.rule()
		d.para("This is a corrected re-send of your welcome package, not a second account")
		d.para("and not a change to your access. The earlier email was incomplete; this one")
		d.para("is the full, current version. Use THIS email and discard the earlier one.")
		if strings.TrimSpace(a.SetupKey) != "" {
			d.blank()
			d.para("What was missing: your one-time mesh enrollment key for Step 1. The earlier")
			d.para("email said an operator would hand it to you — it is included below instead,")
			d.para("so Step 1 is now completable on your own.")
		}
		d.blank()
		d.para("Steps you have already completed do not need redoing. If you got stuck part")
		d.para("way through, resume from the first step you could not finish.")
		d.blank()
	}

	name := strings.TrimSpace(r.Name)
	if name == "" {
		name = r.Handle
	}
	title := " — " + goatnet + " Welcome Package"
	if up != nil {
		switch {
		case len(up.Added()) > 0:
			title = " — " + goatnet + " access upgrade"
		case len(up.Removed()) > 0:
			title = " — " + goatnet + " access change"
		default:
			// Empty delta = a catch-up reference send, not a change.
			title = " — " + goatnet + " access reference"
		}
	}
	d.title(name + title)
	tier := r.Tier
	if tier == "" {
		tier = "participant"
	}
	summary := "tier: " + tier
	if strings.TrimSpace(r.Handle) != "" {
		summary = "handle: " + r.Handle + " | " + summary
	}
	if len(r.Groups) > 0 {
		summary += " | mesh: " + strings.Join(r.Groups, "+")
	}
	if r.Publisher {
		summary += " | data fabric: Publisher"
	}
	if gh := strings.TrimSpace(r.GitHubUsername); gh != "" {
		summary += " | github: " + gh
	}
	d.meta(summary)
	d.blank()
	// Environment identity first — before any instruction. A person may hold
	// (or later gain) access to more than one goatnet; every package states
	// which environment it is about and what that environment is FOR, from
	// per-goatnet config. No purpose configured → no block, never a guess.
	if p := strings.TrimSpace(cat.Purpose); p != "" {
		d.para("About this environment — " + DisplayLabel(cat.Goatnet, cat.Tagline) + ":")
		for _, l := range wrapText(p, 78) {
			d.para("  " + l)
		}
		d.blank()
	}
	d.h2("Your handle")
	if handle := strings.TrimSpace(r.Handle); handle != "" {
		d.cred("", "Your handle", handle)
		d.para("Use this exact identifier when asking an administrator to change your account or access. It is not your email address, display name, organisation identifier, password, or API key.")
	} else {
		d.para("No canonical handle is available in this package. Contact an administrator to finish or correct your onboarding record; do not substitute your email address, display name, or organisation name.")
	}
	d.blank()
	d.para("What is \"the mesh\"? A private network, like a VPN, between your computer and")
	d.para(goatnet + ". Everything below only works while NetBird is connected (Step 1).")
	d.blank()
	if up != nil {
		renderUpgradeHeader(d, goatnet, *up)
	}

	// Everything that expires, collected ABOVE the steps. A reader who skims
	// reaches step 5 twenty minutes later; a window mentioned only there has
	// often already closed. Rendered only when this package actually carries a
	// time-limited artifact, so it never cries wolf.
	//
	// SetupKey joined this gate when the mesh key's default life
	// dropped from 90 days to 7. At 90 days it was the one artifact that could
	// genuinely wait, so a package carrying ONLY a key had nothing urgent to
	// say; at 7 days it is the most perishable thing most people receive, and a
	// key-only package would otherwise print no deadline anywhere.
	if up == nil && (strings.TrimSpace(a.RecoveryLink) != "" ||
		strings.TrimSpace(a.RecoveryCode) != "" ||
		strings.TrimSpace(a.BundleURL) != "" ||
		strings.TrimSpace(a.SetupKey) != "") {
		d.rule()
		d.h2("!! TIME-SENSITIVE — read this part first")
		d.rule()
		if strings.TrimSpace(a.RecoveryLink) != "" || strings.TrimSpace(a.RecoveryCode) != "" {
			// 24h, not "15 minutes / 1 hour": live Kratos runs
			// selfservice.flows.recovery.lifespan: 24h, and the link and code come
			// from ONE flow, so they share it. The old numbers under-stated the
			// window by up to 24x and contradicted STEP 2 below, which said 24
			// hours in the same email. Telling someone their code died an hour ago
			// when it has a day left is how a reader gives up on a live credential.
			if !a.RecoveryExpiresAt.IsZero() {
				d.bullet("  * Your login link and code expire at " +
					a.RecoveryExpiresAt.UTC().Format("2006-01-02 15:04") + " UTC.")
				d.cont("    Expired? Ask for a fresh one — your account is not locked.")
			} else {
				d.bullet("  * Your login link and code expire 24 hours after issue.")
				d.cont("    Expired? Ask for a fresh one — your account is not locked.")
			}
		}
		if strings.TrimSpace(a.BundleURL) != "" {
			d.bullet("  * Your fabric bundle link works ONCE and expires 24 hours after issue.")
			d.cont("    Fetch it now, even if you are not ready to build.")
		}
		if strings.TrimSpace(a.SetupKey) != "" {
			d.bullet("  * Your mesh enrollment key expires 7 DAYS after issue and enrolls a")
			d.cont("    limited number of devices. Enroll the machine you work on first.")
		}
		d.blank()
	}

	// STEP 1 — mesh join. Upgrade mode: already enrolled — say so, and never
	// instruct a re-enrollment or dangle a key the person doesn't need.
	d.rule()
	if up != nil {
		d.h2("STEP 1 — join the " + goatnet + " mesh (already done)")
		d.rule()
		d.para("You're already enrolled — nothing to redo here. Enrolling an additional")
		if cat.MgmtURL != "" {
			d.para("device? An operator hands you a one-time key, then:")
			d.para("  macOS / Linux (Terminal):")
			d.cmd("  " + EnrollCommand(cat.MgmtURL, "<key>", cat.RosenpassEnabled, cat.Goatnet))
			d.para("  Windows (administrator Command Prompt, no sudo):")
			d.cmd("  " + WindowsEnrollCommand(cat.MgmtURL, "<key>", cat.RosenpassEnabled, cat.Goatnet))
		} else {
			d.para("device? An operator hands you a one-time key.")
		}
	} else {
		d.h2("STEP 1 — join the " + goatnet + " mesh")
		d.rule()
		d.para("Pick your operating system and follow ONLY that block. Do NOT create a")
		d.para("netbird.io account (note A).")
		d.blank()
		d.para("macOS / Linux — open Terminal:")
		d.blank()
		d.step("1) Install the NetBird client:")
		d.cmd("     macOS     brew install netbird")
		d.cmd("     Linux     curl -fsSL https://pkgs.netbird.io/install.sh | sh")
		d.step("2) Check it installed:  netbird version")
		d.step("3) Install + start the background service (asks for your password):")
		d.cmd("     sudo netbird service install")
		d.cmd("     sudo netbird service start")
		d.step("4) If NetBird was already connected to another network, disconnect first:")
		d.cmd("     sudo netbird down")
		d.step("5) Connect to " + goatnet + ":")
		renderEnrollLine(d, cat, a, func(mgmtURL, setupKey string, rosenpass bool) string {
			return EnrollCommand(mgmtURL, setupKey, rosenpass, cat.Goatnet)
		})
		d.step("6) Confirm you landed on the " + goatnet + " mesh:  netbird status -d")
		d.blank()
		// Windows gets its own block rather than a "drop the sudo" sentence: a
		// reader who has never seen sudo cannot tell prefix from command, and the
		// sentence is out of sight by step 3. The installer registers and starts
		// the service itself, so there is no service step. Command Prompt named
		// (not PowerShell) so the named enrollment form's %COMPUTERNAME% would
		// expand if it is ever emitted; the unnamed form works in either.
		d.para("Windows — the commands have NO \"sudo\":")
		d.blank()
		// NetBird's old /download page returned 404 (#1343). The
		// docs page is the one the hosted guide links for Windows and carries
		// both the x64 installer and the MSI.
		d.step("1) Download and run the installer from NetBird's Windows install page:")
		d.cont("   https://docs.netbird.io/get-started/install/windows")
		d.cont("   Install the WINDOWS client, not one inside WSL (note B).")
		d.step("2) AFTER the installer finishes, open a NEW Command Prompt AS ADMINISTRATOR:")
		d.cont("   press the Windows key, type cmd, right-click \"Command Prompt\", choose")
		d.cont("   \"Run as administrator\". A window opened before the install cannot find")
		d.cont("   netbird (PATH changes only apply to new windows).")
		d.step("3) Check it installed:  netbird version")
		d.step("4) If NetBird was already connected to another network, disconnect first:")
		d.cmd("     netbird down")
		d.step("5) Connect to " + goatnet + ":")
		renderEnrollLine(d, cat, a, func(mgmtURL, setupKey string, rosenpass bool) string {
			return WindowsEnrollCommand(mgmtURL, setupKey, rosenpass, cat.Goatnet)
		})
		d.step("6) Confirm you landed on the " + goatnet + " mesh:  netbird status -d")
		d.blank()
		if cat.MgmtURL != "" {
			d.cont("   On either system it must say \"Management: Connected to " + cat.MgmtURL + "\".")
			d.cont("   Any other server means the WRONG mesh — see note C.")
		} else {
			d.cont("   On either system the Management line must name " + goatnet + "'s server — see note C.")
		}
		d.blank()
		renderNetworkBlock(d, cat)
		renderPhoneBlock(d, cat, r, a, goatnet)

		// The install links point at netbird.io, whose every page funnels to a free
		// SaaS signup. Downloading from there is correct; signing up there is not.
		// A partner on Linux did exactly that — installed the client,
		// made a netbird.io account, registered the device, saw "Connected", and
		// reported success. His key was never used (used_times 0).
		d.note("Do not create a netbird.io account",
			"Its site pushes you to \"sign up free\". You do not need an account; one made",
			"there puts your machine on NetBird's public network, not "+goatnet+". Your",
			"one-time key is the only credential. Already signed up? Run",
			"`netbird down` (with sudo on macOS/Linux), then step 5.")
		// WSL is a separate Linux machine with its own network: enrolling there puts
		// WSL on the mesh and leaves Windows off it — the peer looks healthy to the
		// operator while the browser cannot open a single mesh service, and it
		// spends one of the key's device enrollments.
		d.note("Windows: install the Windows client, not one inside WSL",
			"WSL is a separate machine with its own network. Enrolling there puts WSL",
			"on the mesh, leaves Windows off it, and spends one of your key's device",
			"enrollments. Run every command in an administrator Command Prompt on",
			"Windows itself.")
		// The silent-ignore trap: `netbird up` against a new server while connected
		// prints "Already connected" and changes nothing. Step 5's command carries
		// --profile (see EnrollCommandNamed), so "step 5 again" here is not just a
		// bare retry — `up --profile` always drops a live session before
		// reconnecting, and it switches to the goatnet's own profile, so it also
		// can't silently repoint whatever profile (a personal account, a different
		// goatnet) was active before this note fired.
		d.note("\"Connected\" — but to the wrong mesh",
			"`netbird up` against a new server while already connected is SILENTLY IGNORED",
			"(it prints \"Already connected\"), and `netbird status` says Connected for",
			"ANY network. Fix: `netbird down` (with sudo on macOS/Linux), step 5 again,",
			"then `netbird status -d`: Management must name "+goatnet+"'s server,",
			"Signal: Connected, FQDN ending .netbird.selfhosted, IP starting 100.64.")
		// Named explicitly because "Linux" alone gets asked about (Oracle Linux 8,
		// ). The list is what install.sh dispatches on, read from the
		// script, not assumed.
		d.note("Linux distributions",
			"The one-liner covers Debian/Ubuntu (apt) and RHEL, Rocky, AlmaLinux,",
			"Oracle Linux, Fedora (dnf/yum), adding NetBird's repo so updates come the",
			"normal way. No WireGuard kernel module? The client uses its own; nothing",
			"to install.")
		// A phone runs one VPN at a time: with another VPN switched on, NetBird
		// cannot hold the tunnel and shows zero peers. Android's Private DNS
		// bypasses the tunnel's resolver, so mesh names silently fail there.
		if !r.NoHandset || a.HandsetKey != "" {
			d.note("Phones run one VPN at a time",
				"Turn any other VPN off before connecting NetBird, or it shows zero peers.",
				"Android: Settings → Network & internet → Private DNS → Off, or mesh names",
				"will not resolve. Your phone appears on "+goatnet+" as",
				"peer-"+r.Handle+"-<site>-<date>-<device name>. It reaches the same web",
				"services as a laptop, through the same login; it cannot run the goat",
				"command, publish to the fabric, or receive files.")
		}
	}
	d.blank()

	// STEP 2 — login. Upgrade mode: the password already exists and keeps
	// working; the actionable nuance is the fresh-session group refresh.
	d.rule()
	if up != nil {
		d.h2("STEP 2 — your login password (already set)")
		d.rule()
		d.para("Your existing password keeps working — it is single sign-on for every web")
		if cat.Guide != "" {
			d.para("surface, including anything newly granted. The full list is in the guide")
			d.para("linked at the end of this email. If a new surface doesn't admit you, sign")
			d.para("out and back in so a fresh session carries your new access.")
		} else {
			d.para("surface below, including anything newly granted. If a new surface doesn't")
			d.para("admit you, sign out and back in so a fresh session carries your new access.")
		}
	} else {
		d.h2("STEP 2 — set your login password (same sitting; mesh must be up)")
		d.rule()
		if a.RecoveryLink != "" {
			if a.RecoveryCode != "" {
				// Code before link, deliberately: a one-time link can be
				// spent in transit by mail-security scanning, leaving the reader
				// with an error they cannot act on. The code carries no state
				// until it is typed, so it survives a scanned inbox. The link
				// stays for readers whose mail is not scanned.
				d.para("Open the recovery page below. Enter the code on that page:")
				d.cred("  ", "page", a.RecoveryLink)
				d.cred("  ", "code", a.RecoveryCode)
				d.para("The code expires 24 HOURS after it was issued. If it has expired, ask the")
				d.para("admin to mint a fresh one, then try again.")
				d.note("Login link already used?",
					"Some mail systems open links before you do, spending a one-time link.",
					"Use the code instead. The code still works.")
			} else {
				d.para("Open this recovery link to reach account settings:")
				d.cmd("  " + a.RecoveryLink)
			}
			// Inline, beside the link, not only in the notes at the end: a person
			// who enrolled a headless server in Step 1 reads this on a workstation
			// that is not on the mesh, and the link cannot resolve there (#1586).
			meshOnlyDevicePara(d)
			d.note("Login page won't load?",
				"That is the mesh, not the link: the page only resolves while NetBird is connected",
				"(Step 1).")
		} else if cat.AuthURL != "" {
			d.para("Your login lives at " + cat.AuthURL + " — your operator sends you a one-time")
			d.para("link (or code) to set your first password.")
		} else {
			d.para("Your operator sends you a one-time link to set your first password.")
		}
		d.para("The recovery link or code opens account settings. It does not set your password.")
		d.step("1) Find the Password section in account settings.")
		d.step("2) Enter a new password in that section.")
		d.step("3) Select the save button for the Password section.")
		d.step("4) Wait for confirmation that the password was saved before you continue to STEP 3.")
		d.para("If the page rejects the password, follow its validation message. Enter an acceptable password and save again.")
		d.para("If the link or code has expired, ask your admin for a fresh one. Repeat STEP 2 with it.")
		// "below" is only true when the surfaces are still listed in this email.
		// Once they move to the guide the sentence has to point there instead, or
		// it sends the reader looking down the page for a list that is not there.
		if cat.Guide != "" {
			d.para("This password is single sign-on for every web surface — they are listed in")
			d.para("the guide linked at the end of this email.")
		} else {
			d.para("This password is single sign-on for every web surface below.")
		}
	}
	d.blank()

	// STEP 3 — surfaces + the Portal key. The surface TOUR moves to the guide
	// when one is configured (none of those addresses resolve before the mesh is
	// up, so listing them here only pads the email); the Portal API key block
	// stays, because it is a credential and credentials never leave the email.
	d.rule()
	if cat.Guide != "" {
		d.h2("STEP 3 — your Portal key")
	} else {
		d.h2("STEP 3 — your surfaces")
	}
	d.rule()
	surf := func(label, u string) {
		if u != "" {
			d.cmd("  " + label + ": " + u)
		}
	}
	if cat.Guide == "" {
		d.para("Bookmark these. All of them log in with your Step-2 password (single sign-on)")
		d.para("EXCEPT the Portal — read the Portal note below before you try it.")
		surf("Portal (your identity, certificates, API keys, topics)", cat.PortalURL)
		surf("Panoscope (live data-fabric common operating picture)", cat.Panoscope)
		surf("Flashlight (minimal fabric inspector)", cat.Flashlight)
		surf("Stethoscope (per-target deep inspection)", cat.Stethoscope)
		surf("Sandbox (isolated practice playground — NOT live)", cat.Sandbox)
		surf("Docs", cat.Docs)
		surf("Service status", cat.IsItUp)
		surf("Chat (collaboration)", cat.Chat)
		// Relay-health dashboard — ADMIN-GATED (PLATFORM-ADMIN, IAM-ADMIN in every
		// goatnet's group_vars). Listing it for everyone advertised a surface that
		// refuses most readers, which is the same defect as pointing people at a
		// Portal they have no key for. Service status is the equivalent question
		// for everyone else and needs no special access.
		if r.AdminConsole {
			surf("Relay-health dashboard (is it me or the network?)", cat.RelayHealth)
		}
		if r.AdminConsole {
			surf("Admin console (your tier grants this)", cat.AdminConsole)
		}
		d.blank()
	}
	// Portal is the one surface that does NOT take the Step-2 password — it wants a
	// portal API key pasted in. An admin-tier onboardee has TWO keys, not one
	// instead of the other: the scoped vp_ key their mint produced (which signs
	// them into the Portal exactly like anyone else's) and an operator op_ key for
	// the operator surfaces. Name both: telling them only about the second leaves a
	// platform-admin with nothing to paste and a 401, while the key sits in the
	// artifacts this email is rendered from.
	if cat.PortalURL != "" {
		d.para("Portal login is DIFFERENT — it does NOT take your Step-2 password. The Portal")
		d.para("asks you to paste a portal API key into its \"Sign in\" box (stored only in your")
		d.para("browser's localStorage on that device).")
		switch {
		case r.AdminOperator && a.PortalAPIKey != "":
			d.para("You have TWO keys, and they are not interchangeable.")
			d.blank()
			d.item("  1. Your scoped key — this is what signs you INTO the Portal. Paste this")
			d.cont("     whole string into the Sign in box:")
			d.blank()
			d.cmd("       " + a.PortalAPIKey)
			d.blank()
			d.cont("     A second copy lives inside the goat-cli.cbor in your bundle — but")
			d.cont("     that file is BINARY: do not copy the key out of it by hand (stray")
			d.cont("     bytes ride along and the portal rejects it). If you ever need that")
			d.cont("     copy, run `goat ca add <the -ca.pem>` then `goat profile init")
			d.cont("     <the .cbor>` — goat trusts no CA until you add one — and read")
			d.cont("     contexts/default/portal.token. Do NOT ask for a rotation to get a")
			d.cont("     \"fresh\" key: rotating re-issues your certificate and makes the key")
			d.cont("     above stop working.")
			d.blank()
			d.item("  2. Your OPERATOR key (op_...) is an ADDITIONAL credential, for the")
			d.cont("     operator surfaces and `goat admin` — not for the Portal sign-in box.")
			d.cont("     It is not in your bundle and not in this email. Request it yourself:")
			d.blank()
			if cat.AdminConsole != "" {
				d.cmd("       " + strings.TrimRight(cat.AdminConsole, "/") + "/operator-key")
			} else {
				d.cont("       your goatnet's identity console, page /operator-key")
			}
			d.blank()
			d.cont("     An admin approves the request and you collect the key from that same")
			d.cont("     page, once. Approval IS the grant — nothing is installed and no")
			d.cont("     restart is waited on. See Operator setup below for what to do with it.")
		case r.AdminOperator:
			d.para("There are two kinds:")
			d.item("  - Regular org developers get a scoped key that looks like vp_<prefix>_...")
			d.item("  - You are an operator, so you use your OPERATOR key: it is NOT in your")
			d.cont("    bundle and not yet on your machine. Request it yourself here:")
			d.blank()
			if cat.AdminConsole != "" {
				d.cmd("      " + strings.TrimRight(cat.AdminConsole, "/") + "/operator-key")
			} else {
				d.cont("      your goatnet's identity console, page /operator-key")
			}
			d.blank()
			d.cont("    An admin approves and you collect the key from that same page, once.")
			d.cont("    Then paste it into the Portal sign-in box.")
		case a.CredentialDeferred:
			// Deferred: there is no key to print, and saying "your access level
			// does not include one" (the branch below) would be flatly wrong and
			// send them to an admin. Name the page instead.
			d.para("  Your portal API key is not in this email, and not in anyone else's hands")
			d.para("  either — it does not exist yet. You mint it yourself, once, here:")
			d.blank()
			if cat.AdminConsole != "" {
				d.cmd("    " + strings.TrimRight(cat.AdminConsole, "/") + "/fabric-credential")
			} else {
				d.cont("    your goatnet's identity console, page /fabric-credential")
			}
			d.blank()
			d.para("  Sign in with your Step-2 password first, on a device that is on the mesh.")
			d.para("  A server you enrolled does not count: this page opens only in a browser on")
			d.para("  a machine where NetBird is connected.")
			d.para("  The page shows the key and your certificate bundle ONCE — a portal key can")
			d.para("  never be shown a second time, which is exactly why it waits for you rather")
			d.para("  than traveling in an email.")
		case a.PortalAPIKey != "":
			// The key itself, inline. Anything less sends the person hunting
			// through a tarball or back to an admin, and the admin's habitual
			// fix (`goat admin rotate`) re-issues in place — which invalidates
			// the copy in this person's own bundle and strands the next reader.
			d.para("  Your portal API key (paste this whole string into the Sign in box):")
			d.blank()
			d.cmd("    " + a.PortalAPIKey)
			d.note("Lost your portal key?",
				"A copy is inside goat-cli.cbor in your bundle (binary — never copy it out",
				"by hand). Run `goat ca add <the -ca.pem>`, then `goat profile init <the",
				".cbor>`, and read contexts/default/portal.token. Do NOT ask for a rotation:",
				"it re-issues your certificate and makes the key above stop working.")
		default:
			d.para("Your access level does not include a portal API key, so the Portal will")
			d.para("not admit you — that is expected, not a fault. Every other surface above")
			d.para("uses your Step-2 password. If your work needs the Portal, ask an admin to")
			d.para("change your access level rather than to issue a key by hand.")
		}
		d.blank()
	}

	// Collaboration environment — a SEPARATE sign-in plane (its own username +
	// one-time setup link, NOT the Step-2 mesh password, NOT gated on the mesh).
	// Rendered only for a fresh onboard whose manifest opted in (collab.enabled)
	// and whose collab account was provisioned (a.CollabSetupLink present). Never
	// in upgrade mode (the person already has, or doesn't get, collab).
	if up == nil && strings.TrimSpace(a.CollabSetupLink) != "" {
		d.rule()
		d.h2("COLLABORATION ENVIRONMENT — set up your account")
		d.rule()
		collabIntro(d, cat.CollabURL)
		d.blank()
		if strings.TrimSpace(a.CollabAccount) != "" {
			d.credSep("  ", "Your collab username", ":  ", a.CollabAccount)
		}
		// Deliver the setup as a CODE TO PASTE, never as a one-click link.
		// This credential session is one-shot, and mail-security
		// gateways open links in transit — so a scanner that reaches it first
		// spends it, and the reader gets "IntentTokenConflict" with no way
		// through.
		// A bare page address is safe to pre-fetch (it carries no state); the
		// code does nothing until a human submits it. Falls back to the whole
		// link if the token cannot be split out — never drop the credential.
		if page, code := splitCredentialLink(a.CollabSetupLink); code != "" {
			d.para("  Set up your account (valid 24 hours):")
			d.credSep("    ", "1. Open this page", ":  ", page)
			d.cred("    ", "2. Paste this code", code)
			d.para("  The code is deliberately not a clickable link: some mail systems open")
			d.para("  links automatically, which would spend a one-time link before you got")
			d.para("  to it.")
		} else {
			d.para("  Set up your account (one-time link, valid 24 hours):")
			d.cmd("    " + a.CollabSetupLink)
		}
		d.blank()
		d.para("On that page set your own credentials. Multi-factor auth is required, so choose")
		d.para("EITHER a passkey (Face ID / Touch ID / security key) OR a password plus an")
		if cat.CollabURL != "" {
			d.para("authenticator app (TOTP). Then sign in with your collab username at")
			d.para(cat.CollabURL + " — one login across Outline, Files, and Chat.")
		} else {
			d.para("authenticator app (TOTP). Then sign in with your collab username — one")
			d.para("login across Outline, Files, and Chat.")
		}
		d.blank()
	}

	// Same plane, no one-time link to repeat. The proposal keeps CollabAccount
	// but never CollabSetupLink — a one-shot credential is deliberately not
	// stored — so a download or a re-issue cannot render the block above.
	// Rendering nothing is the worse answer: the mailed package and the
	// re-issued package then disagree about whether the person has a collab
	// account at all, and the admin holding the download has no line to point
	// them at. Name the account and where a fresh
	// set-up code comes from; never invent the credential itself.
	if up == nil && strings.TrimSpace(a.CollabSetupLink) == "" && strings.TrimSpace(a.CollabAccount) != "" {
		d.rule()
		d.h2("COLLABORATION ENVIRONMENT — you have an account")
		d.rule()
		collabIntro(d, cat.CollabURL)
		d.blank()
		d.credSep("  ", "Your collab username", ":  ", a.CollabAccount)
		d.blank()
		d.para("  This package carries no set-up code. That credential is one-time and is not")
		d.para("  kept after the first package goes out, so a second copy cannot repeat it.")
		if page := joinConsolePath(cat.AdminConsole, "/setup-code"); page != "" {
			d.para("  Mint a fresh one for yourself here:")
			d.cmd("    " + page)
		} else {
			d.para("  Ask an admin for a fresh one: this goatnet serves no self-service page.")
		}
		d.blank()
	}

	// STEP 4 — CLI (gated on a configured get surface). Matches the v3
	// "Install the goat CLI" step: install, Windows checksum, PATH, verify,
	// tutor, and the NetBird-still-required note.
	if cat.GetURL != "" && cat.Guide == "" {
		d.rule()
		if up != nil {
			d.h2("STEP 4 — the goat CLI (skip if already installed)")
		} else {
			d.h2("STEP 4 — install the goat CLI")
		}
		d.rule()
		d.step("1) Install: while on the mesh, open the get page — it shows the one-line")
		d.cont("   install command for your OS:")
		d.cmd("     " + cat.GetURL)
		d.cont("   macOS:   it includes a one-time `codesign` re-sign step right after")
		d.cont("            install. Run it — without it the binary can hang on launch.")
		d.cont("   Windows: the page gives a Command Prompt line (curl.exe ships with Windows")
		d.cont("            10+), a PowerShell line, and a direct goat-windows-amd64.exe link.")
		d.cont("            All three are the SAME binary; run ONE of them, not several.")
		d.cont("            Either shell works; Step 1 used Command Prompt.")
		d.step("2) (Windows) Verify the download isn't corrupt: the page lists SHA256SUMS —")
		d.cont("   hash your file and match the goat-windows-amd64.exe line; if it differs,")
		d.cont("   delete it and re-download on a stable connection.")
		d.step("3) Make `goat` runnable from anywhere:")
		d.cont("   macOS/Linux — move it into a PATH directory:  sudo mv goat /usr/local/bin/")
		d.cont("   Windows — goat.exe lands in your user folder, so add that folder to your")
		d.cont("             PATH once, then CLOSE the Command Prompt (or PowerShell) window")
		d.cont("             and open a NEW one (PATH changes only apply to new sessions).")
		d.step("4) Verify (bare `goat`, no path prefix, should print a version): goat --version")
		d.step("5) Learn hands-on now — an interactive, resumable walkthrough against a")
		d.cont("   built-in sandbox; needs no profile or bundle, so it works before Step 5:")
		d.cmd("     goat tutor            start (or resume)")
		d.cmd("     goat tutor --list     show the lessons without starting")
		if cat.Docs != "" {
			d.cred("", "Docs", cat.Docs)
		}
		d.para("NetBird is still required — goat does NOT replace it. On some OSes goat can")
		d.para("wrap NetBird's tunnel; on Windows it can't yet, so keep managing the mesh with")
		d.para("the NetBird client from Step 1.")
		d.blank()
	}

	// STEP 5 — data fabric. The walkthrough moves to the guide, but the bundle
	// download link does NOT: it is one-time and expires in ~24h, so it is a
	// credential and stays in the email under its own short heading.
	if r.FabricAccess && cat.Guide != "" && a.BundleURL != "" {
		d.rule()
		d.h2("YOUR FABRIC CREDENTIALS — fetch this first, the link is one-time")
		d.rule()
		d.para("Your certificate, private key and CLI profile come as one bundle. The link")
		d.para("below works ONCE and expires about 24 hours after it was issued, so fetch it")
		d.para("now even if you are not ready to publish yet:")
		d.cmd("  " + a.BundleURL)
		d.para("What to do with it is in the guide (link at the end of this email).")
		d.blank()
	}
	if r.FabricAccess && cat.Guide == "" {
		d.rule()
		if r.Publisher {
			d.h2("STEP 5 — use the data fabric (you're a Publisher)")
		} else {
			d.h2("STEP 5 — use the data fabric (read / subscribe)")
		}
		d.rule()
		d.para("Your fabric credentials come as a bundle (a .tar.gz): your client cert +")
		d.para("private key, the root CA, your goat-cli config (<id>-goat-cli.cbor), and")
		d.para("runnable, mTLS-wired first-publisher.py + first-subscriber.py examples.")
		if up != nil {
			d.para("You already hold your bundle from onboarding — it is unchanged by this")
			d.para("update and keeps working. Setting up a new device takes two commands from")
			d.para("your bundle, in this order: goat ca add <id>-goat-cli-ca.pem, then goat")
			d.para("profile init <id>-goat-cli.cbor. goat trusts no CA until you add one.")
			d.para("If `goat ca` errors as an unrecognized command on the new device, update")
			d.para("goat first (an older binary has no CA-trust step at all and will reject")
			d.para("the bundle as `wrong CA` even though it's valid):")
			d.cmd("  macOS/Linux    GOAT_GET_ENDPOINT=" + cat.GetURL + " goat update")
			d.cmd("  Windows cmd    set \"GOAT_GET_ENDPOINT=" + cat.GetURL + "\" && goat update")
			d.cmd("  PowerShell     $env:GOAT_GET_ENDPOINT=\"" + cat.GetURL + "\"; goat update")
			d.para("macOS/Linux with goat in /usr/local/bin (a root-owned folder)? Update under")
			d.para("sudo, and carry the variable through with env — sudo drops it otherwise:")
			d.cmd("  macOS/Linux    sudo env GOAT_GET_ENDPOINT=" + cat.GetURL + " /usr/local/bin/goat update")
			d.para("Lost the bundle? An operator re-issues one.")
		} else {
			if a.BundleURL != "" {
				d.step("1) Download it (mesh-only link, personal to you — keep it private):")
				d.cmd("     " + a.BundleURL)
			} else {
				d.step("1) Your operator hands it to you out of band.")
			}
			d.cont("   It holds a PRIVATE key — keep it private; never put it in OneDrive /")
			d.cont("   iCloud / Dropbox or any cloud-synced location. Unpack it and work from")
			d.cont("   INSIDE the unpacked folder (the scripts read the cert/key next to them):")
			d.cmd("     tar -xzf <downloaded-file>.tar.gz")
			d.step("2) Tell goat which CA to trust. It ships trusting NOTHING, so this is")
			d.cont("   not optional — <id>-goat-cli-ca.pem is in your bundle, next to the")
			d.cont("   .cbor it verifies:")
			d.cmd("     goat ca add <id>-goat-cli-ca.pem")
			d.cont("   That prints a fingerprint. Check it against the one your operator")
			d.cont("   gives you before you rely on it: this bundle reached you over a")
			d.cont("   channel, and a channel can be wrong. You do this once per goatnet;")
			d.cont("   `goat ca ls` shows what you trust.")
			d.cont("   If `goat ca` itself errors as an unrecognized command, your binary is")
			d.cont("   too old for this step to exist and will reject a perfectly valid bundle")
			d.cont("   as `E_BUNDLE_INVALID — wrong CA` — that is not a bad bundle, it's a stale")
			d.cont("   binary. Update goat first, THEN come back to this step:")
			d.cmd("     macOS/Linux    GOAT_GET_ENDPOINT=" + cat.GetURL + " goat update")
			d.cmd("     Windows cmd    set \"GOAT_GET_ENDPOINT=" + cat.GetURL + "\" && goat update")
			d.cmd("     PowerShell     $env:GOAT_GET_ENDPOINT=\"" + cat.GetURL + "\"; goat update")
			d.cont("   macOS/Linux with goat in /usr/local/bin (a root-owned folder)? Update under")
			d.cont("   sudo, and carry the variable through with env — sudo drops it otherwise:")
			d.cmd("     macOS/Linux    sudo env GOAT_GET_ENDPOINT=" + cat.GetURL + " /usr/local/bin/goat update")
			d.step("3) Point goat at the fabric as you — <id>-goat-cli.cbor is the actual")
			d.cont("   filename in your bundle (the <id> is your org id; you don't look it up):")
			d.cmd("     goat profile init <id>-goat-cli.cbor")
			d.step("4) Verify reachability (this one needs the mesh): goat doctor")
			d.step("5) Set up Python once (the example scripts need it) — one line for your OS:")
			d.cont("     macOS/Linux  python3 -m venv venv && . venv/bin/activate && pip install eclipse-zenoh protobuf")
			d.cmd("     Windows cmd  python -m venv venv && venv\\Scripts\\activate.bat && pip install eclipse-zenoh protobuf")
			d.cmd("     PowerShell   python -m venv venv; .\\venv\\Scripts\\Activate.ps1; pip install eclipse-zenoh protobuf")
		}
		if r.Publisher {
			if up == nil {
				d.step("6) Prove pub/sub works — ORDER MATTERS: start the subscriber FIRST and")
				d.cont("   leave it running, THEN the publisher (it sends a few messages and exits,")
				d.cont("   so if you start it first you'll see nothing):")
				d.cmd("     Terminal A (first)   python3 first-subscriber.py")
				d.cmd("     Terminal B (then)    python3 first-publisher.py")
			}
			d.para("first-publisher.py publishes to your own namespace on the live fabric — you")
			d.para("publish only to your own namespace; the rest of the fabric is read/observe.")
			d.para("If your app can't run a native Zenoh client, use the rest-pub-bridge, which")
			d.para("holds your cert and forwards writes.")
		} else {
			d.para("Run first-subscriber.py from the bundle — it reads the fabric with your")
			d.para("identity. Your tier is read/observe (no publishing).")
		}
		if cat.Docs != "" {
			d.cred("", "Exact endpoint + steps", cat.Docs)
		}
		d.para("Note: the Sandbox (Step 3) is a separate, disconnected playground for learning")
		d.para("the API — it is NOT the live fabric.")
		d.blank()
	}

	// Repository access (docs/design/onboarding-github-repo-grants.md). Present
	// only when repos were granted, or as a gentle nudge when a code-facing tier
	// gave no github username. Never shown for browse / iam-admin with no username.
	renderRepoAccess(d, r, cat)

	// Admin tiers onboard other people — teach the two-door distinction up
	// front, because identities created through the login-only form are never
	// usable.
	if r.AdminConsole && cat.Guide == "" {
		renderAdminOnboardingGuide(d, cat)
	}

	// Operator setup — the escalated admin tiers get an out-of-band operator
	// bundle + the admin CLI. Sensitive material (keys) is POINTED at, never
	// embedded.
	if r.AdminOperator && cat.Guide == "" {
		renderOperatorSetup(d, cat, r)
	}

	// The pointer to the other half. Last, deliberately: it is the one link the
	// reader needs AFTER finishing the steps above, and putting it here means the
	// email ends by handing them somewhere to go rather than trailing off.
	if cat.Guide != "" {
		d.rule()
		d.h2("EVERYTHING ELSE — the guide")
		d.rule()
		d.para("Installing the goat CLI, the full list of your surfaces, publishing to the")
		d.para("data fabric, and what to do when something won't load are all here:")
		d.blank()
		d.cmd("  " + cat.Guide + "?tier=" + guideTierParam(r.Tier))
		d.blank()
		// Partner-org members get a second pointer, not a different first one:
		// their tier path is still their path (the partner view says so itself),
		// but the data-sharing rules that differ for a partner organization live
		// on the partner view, and an email that never mentions it leaves them
		// to find it by accident.
		if strings.EqualFold(strings.TrimSpace(r.AffiliationKind), "partner") {
			d.para("Your organisation is a foreign government or military organisation on this")
			d.para("goatnet, so read the PARTNER view as well — it covers what is different")
			d.para("where data crosses organisation lines (your namespace, release lanes, and sharing):")
			d.blank()
			d.cmd("  " + cat.Guide + "?role=partner")
			d.blank()
		}
		d.para("It only loads once you are on the mesh (Step 1): if it opens, Step 1 worked.")
		d.para("It holds nothing secret — bookmark it, share it with anyone at your level.")
		d.blank()
	}

	d.notesBlock()

	if !r.AdminConsole {
		d.para("(Admin surfaces — dashboard, mgmt, minio, grafana, compliance — are not part of")
		d.para("your tier and won't admit you; that's expected, not an error.)")
	}
	return d
}

// guideTierParam normalizes a tier for the guide's ?tier= query. A blank or
// unrecognized tier resolves to browse — the guide degrades the same way, so a
// hand-edited link can never show someone MORE than their level.
func guideTierParam(tier string) string {
	// Reuse the guide's own role resolution so the link the email emits can
	// never name a view the guide would not render (and a hand-edited value
	// degrades to browse there too, never to something broader).
	return RoleByKey(tier).Key
}

// codeFacingTier reports whether a tier is expected to want repo access (so a
// missing github username warrants a nudge rather than silence).
func codeFacingTier(tier string) bool {
	return tier == "developer" || tier == "platform-admin"
}

// renderRepoAccess writes the "Repository access" section. Grants GitHub
// confirmed (added / invited) are listed with their github.com link and the
// accept-the-invite note. Grants it refused, or whose outcome is unknown, are
// listed separately as unconfirmed with the next step, never under "You've
// been added" (#2107). When none were attempted but the tier is code-facing
// and no github username was captured, it tells the person what they are
// missing and where to add one: the identity console's /github page, which
// re-runs the grant. Browse / iam-admin with no username: nothing.
func renderRepoAccess(d *doc, r Recipient, cat Catalog) {
	if len(r.RepoGrants) > 0 {
		var granted, notGranted []RepoGrant
		for _, g := range r.RepoGrants {
			if g.Granted() {
				granted = append(granted, g)
			} else {
				notGranted = append(notGranted, g)
			}
		}
		d.rule()
		d.h2("Repository access")
		d.rule()
		if len(granted) > 0 {
			d.para("You've been added to these repositories — accept the invite (if any) to open each:")
			for _, g := range granted {
				d.cmd("  " + repoGrantLine(g) + " — https://github.com/" + g.Owner + "/" + g.Repo)
			}
			d.para("If a repo shows a pending invitation, accept it from your GitHub notifications.")
		}
		if len(notGranted) > 0 {
			if len(granted) > 0 {
				d.blank()
			}
			d.para("Access to these repositories has not been confirmed:")
			for _, g := range notGranted {
				d.cmd("  " + repoGrantLine(g))
			}
			d.para("Ask your sponsor or a goatnet")
			d.para("operator to check the grant outcome; the GitHub page of")
			d.para("the identity console re-runs the grant:")
			if u := joinConsolePath(cat.AdminConsole, "/github"); u != "" {
				d.cmd("  " + u)
			} else {
				d.para("the GitHub page of the identity console (/github).")
			}
		}
		d.blank()
		return
	}
	if strings.TrimSpace(r.GitHubUsername) == "" && codeFacingTier(r.Tier) {
		d.rule()
		d.h2("Repository access")
		d.rule()
		d.para("You gave no GitHub username, so you have not been added to the example and")
		d.para("partner repositories your level includes. Add it yourself, any time, and the")
		d.para("repositories are granted then:")
		if u := joinConsolePath(cat.AdminConsole, "/github"); u != "" {
			d.cmd("  " + u)
		} else {
			d.para("the GitHub page of the identity console (/github).")
		}
		d.blank()
	}
}

// repoGrantLine is "owner/repo (permission) (source)": the permission and the
// reason the repo is on the list (policy default vs a per-onboardee audible).
// Empty values (legacy rows) render nothing.
func repoGrantLine(g RepoGrant) string {
	line := g.Owner + "/" + g.Repo
	if g.Permission != "" {
		line += " (" + g.Permission + ")"
	}
	switch g.Source {
	case "policy":
		line += " (policy)"
	case "audible":
		line += " (audible)"
	}
	return line
}

// renderAdminOnboardingGuide writes the admin-tier "onboarding other people"
// section: the two-door distinction (full onboarding vs bare login), when the
// welcome email actually fires, and the two group vocabularies. Every line
// here answers a misunderstanding a real admin actually hit.
func renderAdminOnboardingGuide(d *doc, cat Catalog) {
	d.rule()
	d.h2("Onboarding other people — the two doors (read this once)")
	d.rule()
	d.para("The admin console has two ways to add a person. They are NOT interchangeable:")
	d.blank()
	d.para("  1) Onboarding → Invite — THE way to onboard someone. One approved invite")
	// What an approved invite actually mints depends on the goatnet: where
	// fabric credentials are minted on collection, onboarding allocates the
	// slot and the person mints their own bundle later. Saying "produces their
	// bundle" there sends the admin looking for something that was never sent.
	if cat.MintOnCollect {
		d.cont("     produces their mesh enrollment key and login, and a welcome email —")
		d.cont("     automatically. Their fabric credentials are NOT minted here: they")
		d.cont("     collect those themselves once they are on the mesh.")
	} else {
		d.cont("     produces their mesh enrollment key, credentials/fabric bundle, login,")
		d.cont("     and a complete welcome email — automatically.")
	}
	d.cont("     The welcome email fires seconds after an ADMIN APPROVES the queued")
	d.cont("     proposal — not when you send the invite, so \"no email yet\" almost")
	d.cont("     always means \"not approved yet\".")
	d.para("  2) Users → New user — a bare LOGIN only: no mesh key, no credentials, no")
	d.cont("     welcome email, and its recovery link is mesh-internal (unreachable for")
	d.cont("     anyone not already connected). Only for someone already on the mesh who")
	d.cont("     just needs a login.")
	d.blank()
	d.para("Two group vocabularies, two planes: mesh roles (DEVELOPER, OBSERVER, ...) are")
	d.para("granted by onboarding and never typed into a login-groups field; login groups")
	d.para("(PLATFORM-ADMIN, IAM-ADMIN, ...) gate web surfaces. The console refuses a")
	d.para("mesh role in a login field — that refusal is protecting you, not blocking you.")
	d.blank()
	// The deep dive lives on the guide, not on the console's own page. That
	// page sat behind the sponsorship gate, so half the people told to
	// bookmark it could not open it.
	if g := GuidePointer(cat); g != "" {
		d.para("The full detail — bringing people on, start to finish:")
		d.cmd("  " + g + "?topic=identity-admin")
		d.blank()
	}
}

// renderOperatorSetup writes the escalated-admin "Operator setup" section: how
// to install the OOB operator bundle, the env vars to set, the admin CLI
// capabilities, the host shell, and the attribution note. It NEVER embeds a
// private key — it points at the out-of-band bundle.
func renderOperatorSetup(d *doc, cat Catalog, r Recipient) {
	env := cat.Goatnet
	if env == "" {
		env = "<env>"
	}
	d.rule()
	d.h2("Operator setup (your tier grants operator power — handle with care)")
	d.rule()
	// A blank handle must not render "Your handle is  — EVERY admin action…",
	// which reads as a broken email and tells the reader nothing (identities with
	// no onboarding row carry no handle). Say the honest thing instead of printing
	// a hole.
	if strings.TrimSpace(r.Handle) != "" {
		d.para("Your handle is " + r.Handle + " — EVERY admin action you take is attributed to it.")
	} else {
		d.para("Your operator handle is assigned with your operator bundle — ask your")
		d.para("operator for it. EVERY admin action you take is attributed to it.")
	}
	if cat.RepoURL != "" {
		d.para("Full guide: " + strings.TrimRight(cat.RepoURL, "/") + "/blob/main/docs/operations/admin-operator-guide.md")
	} else {
		d.para("Full guide: docs/operations/admin-operator-guide.md in the platform repository.")
	}
	d.blank()
	d.step("1) Install your operator certificate.")
	d.cont("   Your operator bundle is hand-delivered out of band — never emailed. It")
	d.cont("   contains a private key: never commit it, never share it.")
	d.cont("     mkdir -p ~/.goat/" + env + "-operator && cd ~/.goat/" + env + "-operator")
	d.cont("     cp /path/to/bundle/{<operator>-cert.pem,<operator>-key.pem," + env + "-ca-root.pem} .")
	d.cont("     chmod 400 <operator>-key.pem")
	d.cont("   This certificate is your personal identity badge on Zenoh; it lets you")
	d.cont("   publish operator/admin data into the shared operations area (ops/**).")
	d.blank()
	d.step("2) Set your operator environment.")
	d.cont("   Add these to your shell startup file — ~/.zshrc on macOS, ~/.bashrc on")
	d.cont("   Ubuntu/WSL — so they work in every terminal. (Windows: use WSL2/Ubuntu;")
	d.cont("   the operator tooling needs a POSIX shell.)")
	d.cmd("     export GOAT_OPERATOR_HANDLE=" + handleOrPlaceholder(r.Handle))
	d.cmd("     export GOAT_OPERATOR_AGE_IDENTITY=~/.config/" + env + "-operator-age.key")
	// Per-goatnet, never hardcoded: each goatnet has its own variable, and telling
	// a new admin to export another goatnet's sends them to the wrong goatnet's
	// portal.
	portalKeyVar := strings.ToUpper(strings.ReplaceAll(env, "-", "_")) + "_PORTAL_OPERATOR_API_KEY"
	d.cmd("     export " + portalKeyVar + "='<op_... key from your operator bundle>'")
	if strings.TrimSpace(r.Handle) != "" {
		d.cont("   Reload the shell, then verify it printed " + r.Handle + ":")
	} else {
		d.cont("   Reload the shell, then verify it printed your handle:")
	}
	d.cont("     exec $SHELL")
	d.cont("     echo $GOAT_OPERATOR_HANDLE")
	d.cont("   What each is for:")
	d.cmd("     GOAT_OPERATOR_HANDLE          Stamps your handle on every admin action.")
	d.cmd("     GOAT_OPERATOR_AGE_IDENTITY    PATH to the age PRIVATE key from your bundle;")
	d.cont("                                   lets you READ the audit log (goat audit ops).")
	d.cmd("     " + portalKeyVar + "  Authenticates you to the portal for admin")
	d.cont("                                   commands (rotate / invite / queue / sponsorship).")
	d.blank()
	d.step("3) Your everyday operator commands (work now, from any directory):")
	d.cmd("     goat admin rotate <anchor|alias>                 re-issue a cert in place")
	d.cmd("     goat admin invite                                mint a one-time onboarding link")
	d.cmd("     goat admin queue ls|approve|reject               the admin gate on the onboarding queue")
	d.cmd("     goat admin sponsorship <id> --max-tier <tier>    grant/revoke sponsorship authority")
	d.cmd("     goat audit ops                                   who-did-what audit tail")
	d.cont("   Web only (no CLI equivalent), on the Portal:")
	d.cont("     sponsor vouching        /admin/onboard-sponsor-review.html")
	d.cont("     list/revoke invites     /admin/onboard-invite.html")
	d.cont("   Admin consoles (MinIO / dashboard / isitup): just log in with your password.")
	d.blank()
	// Host shell — SHARED operator account, config-driven (MgmtSSHUser), never a
	// fabricated per-handle login. Commands there are not attributed.
	sshUser := strings.TrimSpace(r.MgmtSSHUser)
	host := strings.TrimSpace(r.MgmtHost)
	hostRef := "<mgmt-host>"
	if host != "" {
		hostRef = host
	}
	d.step("4) Host shell — SSH into the mgmt host to run playbooks, roll back a deploy,")
	d.cont("   or debug a service. Not needed for the everyday goat admin commands above.")
	if sshUser != "" {
		d.cont("     ssh " + sshUser + "@" + hostRef)
		d.cont("   Everyone shares the " + sshUser + " account (no per-handle login).")
		d.cont("   YOU DO NOT HAVE THIS YET. Host shell is a SEPARATE ceremony from the")
		d.cont("   onboarding that produced this package. Submit your SSH PUBLIC key:")
		sshKeyLine(d, cat)
		d.cont("   An admin approves it, then an install run places it. Approved is not yet")
		d.cont("   access — you can log in once the page says installed. Commands run in this")
		d.cont("   shared shell aren't attributed to you — use goat admin for anything that")
		d.cont("   needs an audit trail.")
	} else {
		d.cont("   Use the SHARED operator account your operator gives you (it is NOT your")
		d.cont("   handle). YOU DO NOT HAVE THIS YET. Host shell is a SEPARATE ceremony from")
		d.cont("   the onboarding that produced this package. Submit your SSH PUBLIC key:")
		sshKeyLine(d, cat)
		d.cont("   An admin approves it, then an install run places it. Approved is not yet")
		d.cont("   access — you can log in once the page says installed. Ask your operator")
		d.cont("   for the account + host. Commands in this shell aren't attributed to you —")
		d.cont("   use goat admin for anything that needs an audit trail.")
	}
	d.blank()
	d.step("5) (OPTIONAL) Onboarding people — only if you'll add or remove participants.")
	d.cont("   This is the one workflow with extra setup: a NetBird token + a local repo")
	d.cont("   clone that holds each person's manifest. Skip it entirely otherwise.")
	d.cont("   Setup:")
	if cat.Dashboard != "" {
		d.cont("     a) Mint a NetBird token (per-person + attributable — never reuse a shared")
		d.cont("        one): " + cat.Dashboard + " -> avatar -> Profile Settings -> Access")
		d.cont("        Tokens -> Create Access Token. Then add it alongside your step-2 exports:")
	} else {
		d.cont("     a) Mint your OWN NetBird token (per-person + attributable — never reuse a")
		d.cont("        shared one) from the NetBird dashboard's Access Tokens, then export it:")
	}
	d.cmd("            export " + strings.ToUpper(strings.ReplaceAll(env, "-", "_")) + "_NB_API_TOKEN='<the token you just minted>'")
	d.cont("     b) Clone the repo — goat admin onboard must run from inside it (it reads the")
	d.cont("        manifest); the step-3 commands do NOT need it:")
	if cat.RepoURL != "" {
		repo := strings.TrimRight(cat.RepoURL, "/")
		d.cmd("            git clone " + repo + ".git && cd " + path.Base(repo))
	} else {
		d.cmd("            git clone <platform repository URL from your operator> && cd <repo>")
	}
	d.cont("     c) Onboarding writes an encrypted audit record — put the roster recipients")
	d.cont("        (from your operator) in the clone's gitignored, direnv-loaded")
	d.cmd("        .envrc.local.d/40-dev-ca-operators.envrc:")
	d.cmd("            export GOAT_OPERATOR_AGE_RECIPIENTS='age1...,age1...'")
	d.cont("   To onboard someone:")
	d.cont("     1) Create their manifest — this folder starts EMPTY:")
	d.cont("            cp ops/operators/example-vendor.yml.example ops/operators/" + env + "/<handle>.yml")
	d.cont("            # edit the copy (see fields below), then validate:")
	d.cont("            ops/operators/validate.py ops/operators/" + env + "/<handle>.yml")
	d.cont("     2) Run it (from inside the clone):")
	d.cmd("            goat admin onboard <handle> --env " + env + "           add (--dry-run to preview)")
	d.cmd("            goat admin onboard <handle> --env " + env + " --revoke  revoke / offboard (cuts mesh peers, login, slot, and marks the console proposal offboarded)")
	d.cont("   Manifest fields — name/email are free text; the rest must be an exact value")
	d.cont("   from these closed lists (validate.py rejects anything else):")
	d.cont("     netbird.auto_groups   OBSERVER (read-only baseline) | VENDOR (publisher) |")
	d.cont("                           DEVELOPER (publisher, affiliation-neutral) | MONITOR |")
	d.cont("                           OPERATOR | ADMIN (grant sparingly) — one or more")
	d.cont("     netbird.role          bootstrap|admin|operator|peer (cred TTL 1d/7d/30d/7d;")
	d.cont("                           use peer for people)")
	d.cont("     identity.status       active | inactive | revoked (active for a new person)")
	d.cont("     identity.type         person | system (omit / person for people)")
	d.cont("     affiliation.kind      vendor | organic | partner | civil")
	d.cont("     sponsorship.max_tier  browse|developer|iam-admin|platform-admin (only if you")
	d.cont("                           grant them sponsor authority)")
	d.blank()
	d.para("SENSITIVE MATERIAL — operator-key.pem, your age key, and the portal operator")
	d.para("key are in the OUT-OF-BAND bundle, hand-delivered. This message only points at")
	d.para("it; it never contains a private key. Keep the bundle private; do not commit it.")
	d.blank()
}

// renderUpgradeHeader writes the upgrade acknowledgment + what-changed block:
// the explicit "this is not a new enrollment" framing, then the group delta
// with a one-line meaning per known group.
func renderUpgradeHeader(d *doc, goatnet string, u Upgrade) {
	added, removed := u.Added(), u.Removed()
	d.rule()
	if len(added) == 0 && len(removed) == 0 {
		// Catch-up reference: nothing changed right now — the person's access
		// was set up or upgraded before these emails existed (or they asked
		// for a re-send), so hand them the complete current reference.
		d.para("This is the complete, current reference for your EXISTING " + goatnet)
		d.para("identity and role — sent as a catch-up because your access was set up or")
		d.para("changed before these reference emails existed (or on request). Nothing")
		d.para("changed just now and nothing was re-provisioned: your mesh connection,")
		d.para("login password, CLI profile, and any fabric credentials keep working as-is.")
		d.blank()
	} else {
		d.para("This is a change to your EXISTING " + goatnet + " identity — not a new")
		d.para("enrollment. Nothing was re-provisioned: your mesh connection, login")
		d.para("password, CLI profile, and any fabric credentials keep working as-is.")
		d.blank()
		d.para("What changed (your web/console access groups):")
		for _, g := range added {
			d.para("  + " + g + blurbSuffix(g))
		}
		for _, g := range removed {
			d.item("  - " + g + blurbSuffix(g))
		}
		d.blank()
	}
	d.para("Your complete reference for this role follows — the same package a new")
	d.para("enrollee at your access level receives — so everything is in one place.")
	d.blank()
}

// blurbSuffix wraps a known group's meaning in parens; "" for unknown groups
// (rendered bare rather than guessed at).
func blurbSuffix(g string) string {
	if b := groupBlurb(g); b != "" {
		return " (" + b + ")"
	}
	return ""
}

// groupBlurb gives the one-line human meaning of a known platform group or
// data-grammar value (SoT ops/permissions/group-permissions.yml +
// data-access-roles.yaml); "" for anything unrecognized.
func groupBlurb(g string) string {
	switch strings.ToUpper(strings.TrimSpace(g)) {
	case "PLATFORM-ADMIN", "ADMINS":
		return "platform admin: admin consoles + full data visibility"
	case "IAM-ADMIN":
		return "identity administration on the admin console"
	case "NETBIRD-ADMIN":
		return "writes to the NetBird dashboard from its admin address (break-glass)"
	case "COMPLIANCE":
		return "compliance dashboard access"
	case "GRAFANA-READONLY":
		return "read-only metrics dashboards"
	case "OBS-ADMIN":
		return "observability admin: Grafana, and the MinIO console where wired"
	case "OBS-READONLY":
		return "read-only observability dashboards"
	}
	l := strings.ToLower(strings.TrimSpace(g))
	switch {
	case l == "data.view.all":
		return "view every producer's data on the web surfaces"
	case strings.HasPrefix(l, "data.view.org:"):
		return "view that org's data on the web surfaces"
	case strings.HasPrefix(l, "data.view.producer:"):
		return "view that producer's data on the web surfaces"
	case strings.HasPrefix(l, "data.write.producer:"):
		return "write grant for that producer namespace"
	}
	return ""
}

// handleOrPlaceholder keeps a copy-pasteable shell line honest when the handle
// is unknown: `export GOAT_OPERATOR_HANDLE=` pasted as-is silently exports an
// EMPTY handle, and every subsequent admin action would be attributed to
// nobody. A visible placeholder fails loudly at the human instead.
func handleOrPlaceholder(h string) string {
	if strings.TrimSpace(h) == "" {
		return "<your-handle>"
	}
	return h
}

// sshKeyLine emits the self-service submit URL, or the fallback for a goatnet
// with no console. It exists so the two host-shell branches cannot drift: both
// used to say "send your key to your operator", which was the manual path
// before the submit page existed and turned every new admin into a support
// request.
func sshKeyLine(d *doc, cat Catalog) {
	if cat.AdminConsole != "" {
		d.cmd("     " + strings.TrimRight(cat.AdminConsole, "/") + "/ssh-key")
		return
	}
	d.cont("   ask your operator where to submit it — this goatnet has no console.")
}

// TierGrantsAdminConsole reports whether a tier grants the admin console (and,
// mirroring it, operator setup). It is the ONE home for that rule.
//
// Both the console's recipientFromProposal and tools/reissue-preview — the
// out-of-band renderer an operator reaches for precisely when the console cannot
// send — need it. Without it an escalated-tier package rendered there tells a
// platform-admin that admin surfaces "are not part of your tier" and omits the
// operator-key self-service link, which is the one thing that tier most needs
// (the Portal will not admit them without it).
func TierGrantsAdminConsole(tier string) bool {
	switch strings.ToLower(strings.TrimSpace(tier)) {
	case "iam-admin", "platform-admin":
		return true
	}
	return false
}

// SubjectFresh and SubjectReissue are the welcome-package subject lines. Shared
// for the same reason as TierGrantsAdminConsole: the console owned both strings
// privately, so reissue-preview's -subject could only ever print the reissue
// form — an operator rendering a FIRST-time package out of band got a subject
// announcing a correction to an email that was never sent.
//
// goatnet is the DisplayLabel form (e.g. "ACME (production data fabric)").
func SubjectFresh(goatnet string) string {
	return "Welcome to " + goatnet + " — your access and next steps"
}

// SubjectReissue is the subject for a corrected package replacing an earlier one.
func SubjectReissue(goatnet string) string {
	return "Corrected: your " + goatnet + " welcome package (replaces the earlier email)"
}

// meshOnlyDevicePara says, next to the login link, that the link opens only on
// a device connected to the mesh — and that enrolling a server does not make it
// open on the workstation the email was read on: the console can show a person
// as enrolled whose one enrolled device is a headless server, and the link will
// not resolve on their workstation (#1586).
func meshOnlyDevicePara(d *doc) {
	d.para("Open it on a device that is connected to the mesh (Step 1). If you enrolled")
	d.para("only a server, this link will not open on your workstation: enroll the computer")
	d.para("you browse from too, with the same setup key (it enrolls more than one device).")
}

// renderNetworkBlock prints, right after the enrolment check, what the site
// network must allow and what a blocked site looks like. A device behind a
// firewall that passes only HTTPS registers and reports "Connected" yet
// reaches nothing; the check above does not show that.
func renderNetworkBlock(d *doc, cat Catalog) {
	if cat.Network == nil {
		return
	}
	for _, l := range cat.Network.EmailLines(cat.Goatnet, "") {
		d.cont(l)
	}
	d.cont("  `netbird status -d` shows it: \"Peers count: 0/N Connected\" and relays")
	d.cont("  \"context deadline exceeded\". The guide's firewall page has the full list.")
	d.blank()
}

// collabIntro is the opening of both collaboration-environment blocks.
func collabIntro(d *doc, collabURL string) {
	d.para("You also have access to the cloud collaboration environment (Outline knowledge")
	if collabURL != "" {
		d.para("base, file sharing, and chat) at " + collabURL + ". It has its OWN sign-in,")
	} else {
		d.para("base, file sharing, and chat). It has its OWN sign-in,")
	}
	d.para("separate from the mesh password above, and does NOT need NetBird.")
}
