package handlers

// The welcome package generator. An onboarded enrollee needs a
// complete, tier-correct set of instructions — mesh join, login, CLI, surfaces,
// publish — and email is often unavailable (SES sandbox). This endpoint renders
// that package from the proposal + persisted setup key + a freshly-minted login
// link, so an operator hands over ONE artifact instead of hand-assembling it and
// dropping pieces. The tier→capability decision lives in recipientFromProposal;
// the rendering lives in the pure internal/welcome package.

import (
	"encoding/json"
	"log/slog"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/welcome"
)

// bundleLink builds the enrollee-facing mesh download link for the fabric bundle:
// the mesh portal base + the runner-reported token path. "" when either is absent
// (→ the welcome package falls back to the OOB hand-off line).
func (s *Server) bundleLink(bundlePath string) string {
	base := strings.TrimRight(s.Cfg.NotifyPortalURL, "/")
	if base == "" || bundlePath == "" {
		return ""
	}
	if !strings.HasPrefix(bundlePath, "/") {
		bundlePath = "/" + bundlePath
	}
	return base + bundlePath
}

// welcomeCatalog builds the per-goatnet service catalog from console config —
// the same NotifyMgmt/Get/Portal URLs the onboarded email already uses.
// emailGoatnet resolves WHICH goatnet an email should name. The proposal row
// is authoritative when there is one; otherwise fall back to the goatnet this
// console serves, so an email to an identity without one (ported, hand-created,
// pre-buildout) does not call the environment "the data fabric" — the generic
// string DisplayName returns for an empty goatnet.
func (s *Server) emailGoatnet(env string) string {
	if strings.TrimSpace(env) != "" {
		return env
	}
	return strings.TrimSpace(s.Cfg.NotifyGoatnet)
}

func (s *Server) welcomeCatalog(env string) welcome.Catalog {
	env = s.emailGoatnet(env)
	cat := welcome.DeriveCatalog(env, s.Cfg.NotifyMgmtURL, s.Cfg.NotifyGetURL, s.Cfg.NotifyPortalURL)
	cat.RosenpassEnabled = s.Cfg.NotifyRosenpass
	cat.RelayHealth = s.Cfg.NotifyRelayHealthURL
	cat.Map = s.Cfg.NotifyMapURL
	cat.MintOnCollect = s.Cfg.NotifyMintOnCollect
	cat.Tagline = s.Cfg.NotifyGoatnetTagline
	cat.Purpose = s.Cfg.NotifyGoatnetPurpose
	cat.RepoURL = s.Cfg.PlatformRepoURL
	cat.CollabURL = s.Cfg.CollabURL
	if n, ok := s.networkRequirements(); ok {
		cat.Network = &n
	}
	return cat
}

// welcomeManifest is the subset of the onboard manifest the welcome package reads.
type welcomeManifest struct {
	Identity struct {
		Name   string `json:"name"`
		GitHub string `json:"github"`
	} `json:"identity"`
	Affiliation struct {
		Kind string `json:"kind"`
	} `json:"affiliation"`
	Netbird struct {
		Enabled    bool     `json:"enabled"`
		AutoGroups []string `json:"auto_groups"`
		// HandsetLimit is netbird.handset_limit (ADR 1175). A pointer so an
		// absent field (default 1) is told apart from an explicit 0.
		HandsetLimit *int `json:"handset_limit"`
	} `json:"netbird"`
	Zenoh struct {
		Enabled bool `json:"enabled"`
	} `json:"zenoh"`
}

// recipientFromProposal resolves the tier→capability decision once, from the
// proposal's tier + manifest, so welcome.Render stays a pure formatter.
func recipientFromProposal(p *onboardstore.Proposal) welcome.Recipient {
	var m welcomeManifest
	_ = json.Unmarshal([]byte(p.ManifestJSON), &m) //nolint:errcheck // malformed legacy manifests degrade to proposal-level fields
	r := welcome.Recipient{
		Name:            strings.TrimSpace(m.Identity.Name),
		Handle:          p.Handle,
		Email:           p.Email,
		Tier:            p.Tier,
		Groups:          m.Netbird.AutoGroups,
		GitHubUsername:  strings.TrimSpace(m.Identity.GitHub),
		AffiliationKind: strings.TrimSpace(m.Affiliation.Kind),
		NoHandset:       m.Netbird.HandsetLimit != nil && *m.Netbird.HandsetLimit == 0,
	}
	// FabricAccess = has a Zenoh identity (cert bundle) at all. Publisher = that
	// PLUS a role that writes (DEVELOPER); a pure OBSERVER reaches the fabric
	// read-only (identified subscribe) but is not a publisher.
	r.FabricAccess = m.Zenoh.Enabled
	r.Publisher = m.Zenoh.Enabled && containsFold(m.Netbird.AutoGroups, "DEVELOPER")
	// Admin console + operator setup are granted to the escalated admin tiers only.
	r.AdminConsole = p.Tier == "iam-admin" || p.Tier == "platform-admin"
	r.AdminOperator = r.AdminConsole
	// Repo-collaborator grants recorded at provision (fail-open), surfaced in the
	// "Repository access" section. '' / '[]' → none.
	if grants := parseRepoGrants(p.RepoGrantsJSON); len(grants) > 0 {
		r.RepoGrants = grants
	}
	return r
}

// needsSetupKey reports whether this proposal's welcome package is incomplete
// without a NetBird enrollment key — i.e. the manifest enrolls the person on the
// mesh. Zenoh-only / login-only manifests never mint a key, so their packages
// are complete without one and must not be held.
func needsSetupKey(p *onboardstore.Proposal) bool {
	var m struct {
		Netbird *struct {
			Enabled bool `json:"enabled"`
		} `json:"netbird"`
	}
	if err := json.Unmarshal([]byte(p.ManifestJSON), &m); err != nil || m.Netbird == nil {
		// An unreadable manifest, or a historical index row such as {}, cannot
		// prove the person needs no mesh key. Require one rather than deliver a
		// package that silently omits it (#2027).
		return true
	}
	return m.Netbird.Enabled
}

// parseRepoGrants decodes the persisted repo-grant outcomes into welcome
// RepoGrant values (nil on empty / malformed — the section just renders nothing).
func parseRepoGrants(raw string) []welcome.RepoGrant {
	raw = strings.TrimSpace(raw)
	if raw == "" || raw == "[]" {
		return nil
	}
	var gs []welcome.RepoGrant
	if err := json.Unmarshal([]byte(raw), &gs); err != nil {
		return nil
	}
	return gs
}

func containsFold(xs []string, want string) bool {
	for _, x := range xs {
		if strings.EqualFold(x, want) {
			return true
		}
	}
	return false
}

// handleWelcomeDownload serves the generated welcome package for an onboarded
// proposal as a plaintext attachment — the deliverable an operator hands the
// enrollee when email is unavailable. Admin-gated (onboardGate). Mints a fresh
// login recovery link best-effort so the package is complete end to end.
func (s *Server) handleWelcomeDownload(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, 503, "onboarding is not configured")
		return
	}
	id := r.PathValue("id")
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if err != nil {
		s.renderError(w, r, 404, "proposal not found")
		return
	}
	if p.Status != onboardstore.ProposalOnboarded {
		s.renderError(w, r, 409, "the welcome package is available once the enrollee is onboarded (status: "+p.Status+")")
		return
	}
	if needsSetupKey(p) && strings.TrimSpace(p.SetupKey) == "" {
		s.renderError(w, r, http.StatusConflict, "this enrollee requires a mesh enrollment key; supply it as `setup_key` through welcome resend before downloading the package")
		return
	}
	cat := s.welcomeCatalog(p.Env)
	rcpt := recipientFromProposal(p)
	// CollabAccount rides along so the download names the collaboration
	// account the mailed package named. The one-time set-up link is not stored,
	// so the renderer falls to the no-link branch and points at /setup-code.
	art := welcome.Artifacts{SetupKey: p.SetupKey, BundleURL: s.bundleLink(p.BundlePath),
		CollabAccount: p.CollabAccount}
	if link := s.mintRecoveryForEmail(r, p.Email); link != nil {
		art.RecoveryLink = link.RecoveryLink
		art.RecoveryCode = link.RecoveryCode
		art.RecoveryExpiresAt = link.ExpiresAt
	}
	body := welcome.Render(cat, rcpt, art)

	// This hands over one-time secrets (setup key + recovery link) — audit it.
	s.emitAuditWithInviteToken(r, actorFromCtx(r.Context()), "onboard_welcome_package",
		p.Handle, "", nil, map[string]interface{}{"env": p.Env, "email": p.Email}, false, "ok", "")

	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.Header().Set("Content-Disposition", "attachment; filename=\""+p.Handle+"-welcome.txt\"")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write([]byte(body))
}

// handleWelcomeResend re-sends the welcome package to an already-onboarded
// enrollee, marked as a repeat so a second package in the inbox is not read as a
// second identity. It is the recovery path for a person who received an
// incomplete package (the held-send case above, or a package sent before the key
// existed) — without it the only remedy is hand-assembling the email, which is
// what the generated package exists to prevent.
//
// An optional `setup_key` form/JSON field supplies a key the console never
// learned (e.g. minted outside the runner writeback); it is persisted so the
// stored proposal and the delivered package agree from then on. Admin-gated.
func (s *Server) handleWelcomeResend(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		s.renderError(w, r, 503, "onboarding is not configured")
		return
	}
	id := r.PathValue("id")
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if err != nil {
		s.renderError(w, r, 404, "proposal not found")
		return
	}
	if p.Status != onboardstore.ProposalOnboarded {
		s.renderError(w, r, 409, "a welcome package is re-sendable once the enrollee is onboarded (status: "+p.Status+")")
		return
	}
	// A supplied key wins over the stored one and is persisted, so the next read
	// of this proposal (or its welcome download) carries the same key. It is
	// saved before the mailer check so a console without email can still take
	// the key and serve the download.
	setupKey := strings.TrimSpace(r.FormValue("setup_key"))
	handsetKey := strings.TrimSpace(r.FormValue("handset_key"))
	if (setupKey != "" && setupKey != p.SetupKey) || (handsetKey != "" && handsetKey != p.HandsetKey) {
		// Blank strings + false leave every other stored field untouched (the
		// store only overwrites on non-empty, and collab_activated only latches).
		if err := s.Onboard.SetProposalResult(r.Context(), id, p.Status, setupKey, handsetKey, "", "", "", false); err != nil {
			s.renderError(w, r, 500, "persist setup key: "+err.Error())
			return
		}
		if setupKey != "" {
			p.SetupKey = setupKey
		}
		if handsetKey != "" {
			p.HandsetKey = handsetKey
		}
	}
	if setupKey == "" {
		setupKey = p.SetupKey
	}
	if handsetKey == "" {
		handsetKey = p.HandsetKey
	}
	if s.Mailer == nil || !s.Mailer.Enabled() || strings.TrimSpace(p.Email) == "" {
		msg := "email is not configured for this console"
		if setupKey != "" {
			msg += "; the setup key is saved, so the welcome package can be downloaded"
		}
		s.renderError(w, r, 503, msg)
		return
	}
	if needsSetupKey(p) && setupKey == "" {
		s.renderError(w, r, 409, "this enrollee is mesh-enrolled but the console holds no setup key — "+
			"supply one as `setup_key` (it is on the mint host in ops/operators/out/"+p.Handle+"-bundle.md)")
		return
	}

	outcome, errMsg := s.resendWelcomePackage(r, p, setupKey, handsetKey, actorFromCtx(r.Context()))
	if outcome != "ok" {
		s.renderError(w, r, 502, "send failed: "+errMsg)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]any{"status": "sent", "to": p.Email, "handle": p.Handle})
}

// resendWelcomePackage renders the reissue-marked welcome package with the given
// keys and emails it, auditing the outcome as actor. It is the core of
// handleWelcomeResend and of the handset-key reissue result (ADR 1175), so a
// fresh key reaches the person the same way a corrected package does. Returns
// "ok", "error", or "held" when a required mesh key is missing; "skipped" when no mailer is
// configured or the proposal has no email, so a caller can record that the key
// was minted but not delivered.
func (s *Server) resendWelcomePackage(r *http.Request, p *onboardstore.Proposal, setupKey, handsetKey string, actor Actor) (outcome, errMsg string) {
	if needsSetupKey(p) && strings.TrimSpace(setupKey) == "" {
		s.emitAuditWithInviteToken(r, actor, "onboard_welcome_resend", p.Handle, "", nil, nil, false,
			"held", "a mesh enrollment key is required before delivery")
		return "held", "a mesh enrollment key is required before delivery"
	}
	if s.Mailer == nil || !s.Mailer.Enabled() || strings.TrimSpace(p.Email) == "" {
		return "skipped", "email is not configured for this console, or the person has no email"
	}
	cat := s.welcomeCatalog(p.Env)
	rcpt := recipientFromProposal(p)
	art := welcome.Artifacts{SetupKey: setupKey, HandsetKey: handsetKey, BundleURL: s.bundleLink(p.BundlePath),
		CollabAccount: p.CollabAccount}
	if link := s.mintRecoveryForEmail(r, p.Email); link != nil {
		art.RecoveryLink = link.RecoveryLink
		art.RecoveryCode = link.RecoveryCode
		art.RecoveryExpiresAt = link.ExpiresAt
	}
	pkg := welcome.RenderReissuePackage(cat, rcpt, art)
	goatnet := welcome.DisplayLabel(p.Env, s.Cfg.NotifyGoatnetTagline)
	subject := "Corrected: your " + goatnet + " welcome package (replaces the earlier email)"

	outcome, errMsg = "ok", ""
	if err := s.Mailer.SendMultipart(p.Email, subject, pkg.Text, pkg.HTML); err != nil {
		outcome, errMsg = "error", err.Error()
	}
	s.emitAuditWithInviteToken(r, actor, "onboard_welcome_email_resend",
		p.Handle, "", nil, map[string]interface{}{"to": p.Email, "env": p.Env,
			"setup_key_included": setupKey != "", "handset_key_included": handsetKey != ""}, false, outcome, errMsg)
	return outcome, errMsg
}

// mintRecoveryForEmail resolves the tenant identity by email and mints a recovery
// link. Returns nil (not an error) on any miss — the caller degrades gracefully
// to "your operator sends a login link" rather than failing the whole package.
func (s *Server) mintRecoveryForEmail(r *http.Request, email string) *kratos.RecoveryLink {
	cli := s.Kratos["tenant"]
	if cli == nil || strings.TrimSpace(email) == "" {
		return nil
	}
	ids, err := cli.ListIdentities(r.Context())
	if err != nil {
		slog.Warn("welcome: list identities failed", "err", err)
		return nil
	}
	var idID string
	for _, i := range ids {
		if strings.EqualFold(i.Email(), email) {
			idID = i.ID
			break
		}
	}
	if idID == "" {
		return nil
	}
	link, err := cli.MintRecoveryLink(r.Context(), idID)
	if err != nil {
		slog.Warn("welcome: mint recovery failed", "err", err)
		return nil
	}
	return link
}

// sendWelcomeEmail renders the welcome package for a freshly-onboarded enrollee
// and emails it (best-effort). Called at result time when the setup key is known,
// so onboarding auto-delivers instead of relying on an operator hand-off. The
// private-key cert bundle stays out of band (never emailed); the email carries
// mesh join + login + surfaces + the fabric-bundle instructions. Failures are
// audited, never fatal to the onboarded result.
func (s *Server) sendWelcomeEmail(r *http.Request, p *onboardstore.Proposal, setupKey, handsetKey, collabSetupLink, portalAPIKey string, credentialDeferred bool) {
	if s.Mailer == nil || !s.Mailer.Enabled() || strings.TrimSpace(p.Email) == "" {
		return
	}
	// A mesh-enrolled enrollee with no setup key cannot complete step 1, and the
	// package degrades to "your operator will hand you a key" — which reads as a
	// complete set of instructions and is not one. Sending it strands the person
	// AND consumes the one welcome email they will read carefully. So: hold the
	// send, make the gap loud in the audit trail, and let the key's arrival (a
	// later result writeback, or an operator resend) deliver the real package.
	// Manifests with netbird disabled have no key to wait for — send normally.
	if needsSetupKey(p) && strings.TrimSpace(setupKey) == "" {
		slog.Warn("welcome email HELD: onboarded with netbird enabled but no setup key — "+
			"resend once the key is known (POST /onboarding/{id}/welcome/resend)",
			"handle", p.Handle, "env", p.Env, "email", p.Email, "proposal", p.ID)
		s.emitAuditWithInviteToken(r, Actor{Email: "console-onboard"}, "onboard_welcome_email",
			p.Handle, "", nil, map[string]interface{}{"to": p.Email, "env": p.Env}, false,
			"held", "no setup key at result time — welcome email withheld rather than sent incomplete")
		return
	}
	cat := s.welcomeCatalog(p.Env)
	rcpt := recipientFromProposal(p)
	// collabSetupLink is the one-time collab credential-setup link (passed through
	// from the runner writeback, never stored). When present, welcome.Render adds a
	// "Collaboration environment" section with the username + link.
	art := welcome.Artifacts{SetupKey: setupKey, HandsetKey: handsetKey, BundleURL: s.bundleLink(p.BundlePath),
		CollabAccount: p.CollabAccount, CollabSetupLink: collabSetupLink,
		PortalAPIKey: portalAPIKey, CredentialDeferred: credentialDeferred}
	if link := s.mintRecoveryForEmail(r, p.Email); link != nil {
		art.RecoveryLink = link.RecoveryLink
		art.RecoveryCode = link.RecoveryCode
		art.RecoveryExpiresAt = link.ExpiresAt
	}
	pkg := welcome.RenderPackage(cat, rcpt, art)
	goatnet := welcome.DisplayLabel(p.Env, s.Cfg.NotifyGoatnetTagline)
	subject := "Welcome to " + goatnet + " — your access and next steps"
	outcome, errMsg := "ok", ""
	if err := s.Mailer.SendMultipart(p.Email, subject, pkg.Text, pkg.HTML); err != nil {
		outcome, errMsg = "error", err.Error()
	}
	s.emitAuditWithInviteToken(r, Actor{Email: "console-onboard"}, "onboard_welcome_email",
		p.Handle, "", nil, map[string]interface{}{"to": p.Email, "env": p.Env}, false, outcome, errMsg)
}

// handleOnboardingGuide serves the post-mesh half of the welcome package as an
// HTML page (internal/welcome/guide.go explains the split). UNAUTHENTICATED by
// design and safe to be: the page is identical for everyone at a tier and holds
// no credentials, so there is nothing on it to leak between two people who can
// both already reach the mesh. Reachability is the control — it is published on
// a mesh-only host, so being able to load it at all means being on the mesh.
//
// `tier` selects the view. An unknown or absent tier renders the browse view
// rather than erroring: someone typing the URL should get the least-privileged
// accurate page, never a stack trace.
func (s *Server) handleOnboardingGuide(w http.ResponseWriter, r *http.Request) {
	cat := s.welcomeCatalog(strings.TrimSpace(r.URL.Query().Get("env")))
	q := r.URL.Query()
	// `role` is the current parameter; `tier` is accepted so links minted by
	// earlier welcome emails keep working.
	role := strings.TrimSpace(q.Get("role"))
	if role == "" {
		role = strings.TrimSpace(q.Get("tier"))
	}
	if strings.EqualFold(strings.TrimSpace(q.Get("topic")), "firewall") {
		cat.Egress = guideEgress.hosts(r.Context(), s.Cfg.NotifyMgmtURL, s.Cfg.NotifyRelayAddresses, s.Cfg.NotifyStunHosts)
	}
	page := welcome.BuildGuidePage(cat, role, q.Get("topic"), s.Cfg.GuideRepoBaseURL)
	html, err := welcome.RenderGuidePage(page)
	if err != nil {
		slog.Error("onboarding guide render failed", "err", err)
		http.Error(w, "onboarding guide unavailable", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Cache-Control", "public, max-age=300")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write([]byte(html)) //nolint:gosec // HTML is rendered from trusted templates; XSS is controlled in welcome.RenderGuidePage
}
