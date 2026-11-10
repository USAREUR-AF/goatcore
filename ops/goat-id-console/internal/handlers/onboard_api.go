package handlers

// onboard_api.go — the MACHINE (portal-key) onboarding realm (ADR 1062/1071),
// ported from the data-fabric portal per docs/design/onboarding-extraction-plan.md
// (slice 2b). Auth is a PER-HANDLE operator-key map (ONBOARD_OPERATOR_API_KEYS =
// "handle:key,..."), NOT a single shared token — F-207 per-person attribution is
// preserved. The onboard-runner + `goat admin invite` CLI present their own keys.
// JSON in/out is byte-shaped identically to the portal's so they are a drop-in
// re-point.
//
// resolvePrincipal is the resolve SEAM: operator keys resolve locally to their
// handle (operator-equivalent — ADR 1071 authorized, no tier cap); a key not in
// the operator map is a PARTICIPANT's own portal key, resolved read-only against
// the portal (GET /api/whoami with that key as the bearer) for the org handle +
// its ADR 1071 sponsorship cap. When PORTAL_INTERNAL_URL is unset the participant
// seam fails closed (errParticipantKeyUnsupported), never silently open.

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardlint"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// onboardLint is a thin alias so the human decide path + the machine lint endpoint
// share one linter (same signature as the portal's).
func onboardLint(content []byte, format string) onboardlint.Result {
	return onboardlint.Lint(content, format)
}

// onboardPrincipal is the resolved machine-realm caller: the attributed handle,
// and whether it is operator-equivalent (no ADR 1071 tier cap). MaxTier carries
// the participant's sponsorship ceiling once the portal-delegation seam is wired
// (” = unrestricted, which is only ever the case for an operator).
type onboardPrincipal struct {
	Handle     string
	IsOperator bool
	MaxTier    string // ADR 1071 sponsorship cap for a participant ('' for operators)
	// Alias + SponsorEmail are the participant identity's other recorded
	// sponsor forms (from the portal whoami) — the own-invitee scoping
	// (ADR 1072) matches a row's `sponsor` against {handle, alias, email}.
	Alias        string
	SponsorEmail string
	// Name is the human-readable display label for the principal (an operator's
	// per-person handle, or a participant's portal display_name / slot alias).
	// Display-only attribution — Handle stays the machine value (a participant's
	// handle is its UUID org anchor, which ADR 1072 sponsor-scoping matches on).
	Name string
}

// unrestrictedView reports whether this principal sees the full queue:
// operators always; participants only when their cap is an escalated admin
// tier (iam-admin / platform-admin) — the own-invitee restriction applies to
// the non-escalated tiers (ADR 1072).
func (p onboardPrincipal) unrestrictedView() bool {
	return p.IsOperator || isEscalatedTier(p.MaxTier)
}

// allowsSponsor reports whether a recorded invite/proposal sponsor value
// belongs to this principal's scope.
func (p onboardPrincipal) allowsSponsor(sponsor string) bool {
	if p.unrestrictedView() {
		return true
	}
	return sponsorAllows(sponsorKeySet(p.Handle, p.Alias, p.SponsorEmail), sponsor)
}

// errParticipantKeyUnsupported is returned by resolvePrincipal for a key that is
// not a known operator key when portal delegation is not configured
// (PORTAL_INTERNAL_URL unset) — the seam fails closed rather than silently open.
var errParticipantKeyUnsupported = errors.New("participant-key onboarding is unavailable (PORTAL_INTERNAL_URL unset — portal delegation off)")

// errNotSponsorAuthorized is returned when a resolved participant identity is not
// sponsorship-authorized (ADR 1071) — it may not mint invites.
var errNotSponsorAuthorized = errors.New("your identity is not sponsorship-authorized")

// resolvePrincipal resolves a presented bearer key to its principal. Operator
// keys (ONBOARD_OPERATOR_API_KEYS) resolve locally via a constant-time compare;
// the matched handle is attributed to the action. A key that is not a
// known operator key is a PARTICIPANT's own portal key: it is resolved read-only
// against the data-fabric portal (GET /api/whoami with that key as the bearer),
// which returns the org handle + its ADR 1071 sponsorship cap. The key store +
// vendors registry stay the SoT in the portal; the console reads them.
func (s *Server) resolvePrincipal(ctx context.Context, key string) (onboardPrincipal, error) {
	key = strings.TrimSpace(key)
	if key == "" {
		return onboardPrincipal{}, errors.New("no key presented")
	}
	// Operator keys resolve locally. Constant-time compare against every
	// configured key so timing can't distinguish a near-miss.
	matchedHandle := ""
	kb := []byte(key)
	for k, handle := range s.Cfg.OnboardOperatorAPIKeys {
		if subtle.ConstantTimeCompare(kb, []byte(k)) == 1 {
			matchedHandle = handle
		}
	}
	if matchedHandle != "" {
		return onboardPrincipal{Handle: matchedHandle, Name: matchedHandle, IsOperator: true}, nil
	}
	// Participant key → delegate to the portal's read-only whoami.
	if strings.TrimSpace(s.Cfg.PortalInternalURL) == "" {
		return onboardPrincipal{}, errParticipantKeyUnsupported
	}
	return s.resolveParticipantViaPortal(ctx, key)
}

// portalWhoami is the shape of the portal's GET /api/whoami response.
type portalWhoami struct {
	Operator          bool   `json:"operator"`
	Actor             string `json:"actor"`
	Org               string `json:"vendor"`
	DisplayName       string `json:"display_name"`
	SponsorAuthorized bool   `json:"sponsor_authorized"`
	SponsorMaxTier    string `json:"sponsor_max_tier"`
	Alias             string `json:"alias"`
	SponsorEmail      string `json:"sponsor_email"`
}

// resolveParticipantViaPortal calls the portal's whoami with the participant's
// own key and maps the result onto an onboardPrincipal. A vendor principal must
// be sponsorship-authorized (ADR 1071); its SponsorMaxTier caps the invite.
func (s *Server) resolveParticipantViaPortal(ctx context.Context, key string) (onboardPrincipal, error) {
	base := strings.TrimRight(strings.TrimSpace(s.Cfg.PortalInternalURL), "/")
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, base+"/api/whoami", nil)
	if err != nil {
		return onboardPrincipal{}, err
	}
	req.Header.Set("Authorization", "Bearer "+key)
	resp, err := onboardHTTP.Do(req)
	if err != nil {
		return onboardPrincipal{}, fmt.Errorf("portal whoami: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode == http.StatusUnauthorized {
		return onboardPrincipal{}, errors.New("key not recognized by the portal")
	}
	if resp.StatusCode != http.StatusOK {
		return onboardPrincipal{}, fmt.Errorf("portal whoami: HTTP %d", resp.StatusCode)
	}
	var wh portalWhoami
	if err := json.NewDecoder(io.LimitReader(resp.Body, 1<<16)).Decode(&wh); err != nil {
		return onboardPrincipal{}, fmt.Errorf("portal whoami decode: %w", err)
	}
	if wh.Operator {
		// An operator-tier portal key presented as a participant key — treat as
		// operator-equivalent (no cap), attributing to its handle.
		h := firstNonEmpty(wh.Actor, "operator")
		return onboardPrincipal{Handle: h, Name: h, IsOperator: true}, nil
	}
	if wh.Org == "" {
		return onboardPrincipal{}, errors.New("key not recognized by the portal")
	}
	if !wh.SponsorAuthorized {
		return onboardPrincipal{}, errNotSponsorAuthorized
	}
	return onboardPrincipal{
		Handle:       wh.Org,
		Name:         firstNonEmpty(strings.TrimSpace(wh.DisplayName), strings.TrimSpace(wh.Alias), wh.Org),
		IsOperator:   false,
		MaxTier:      strings.ToLower(strings.TrimSpace(wh.SponsorMaxTier)),
		Alias:        wh.Alias,
		SponsorEmail: wh.SponsorEmail,
	}, nil
}

// onboardHTTP is the shared client for the read-only portal delegation call.
var onboardHTTP = &http.Client{Timeout: 20 * time.Second}

// ctxKeyOnboardPrincipal stashes the resolved principal for the handlers.
type onboardCtxKey int

const ctxKeyOnboardPrincipal onboardCtxKey = 0

// onboardAPIGate is the machine-realm middleware: it resolves the presented
// bearer key to a principal via resolvePrincipal and stashes it in context. When
// neither auth path is configured (no operator keys AND no portal delegation) the
// routes 503 — fail-closed, no silent open. An unknown/unresolvable key is 401.
func (s *Server) onboardAPIGate(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if len(s.Cfg.OnboardOperatorAPIKeys) == 0 && strings.TrimSpace(s.Cfg.PortalInternalURL) == "" {
			onboardError(w, http.StatusServiceUnavailable, "not_configured", "onboard API auth not configured")
			return
		}
		presented := strings.TrimSpace(strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer "))
		p, err := s.resolvePrincipal(r.Context(), presented)
		if err != nil {
			// A recognized-but-unauthorized participant is 403; anything else
			// (unknown key, portal unreachable, delegation off) is 401.
			if errors.Is(err, errNotSponsorAuthorized) {
				onboardError(w, http.StatusForbidden, "not_sponsor", "your identity is not sponsorship-authorized")
				return
			}
			onboardError(w, http.StatusUnauthorized, "unauthorized", "invalid or missing onboard API key")
			return
		}
		ctx := context.WithValue(r.Context(), ctxKeyOnboardPrincipal, p)
		next.ServeHTTP(w, r.WithContext(ctx))
	}
}

// onboardPrincipalFromCtx returns the resolved machine-realm principal.
func onboardPrincipalFromCtx(ctx context.Context) onboardPrincipal {
	p, _ := ctx.Value(ctxKeyOnboardPrincipal).(onboardPrincipal)
	return p
}

// isServiceSponsorLabel reports whether a resolved sponsor handle names a
// shared machine credential rather than a person. "operator" is the legacy
// shared key, "onboard-api" the fallback attribution, and "id-console" the
// console's own portal key, which a goatnet with no per-person operator keys
// resolves every operator mint to (#1942). For these, the
// X-Goat-Operator-Actor header names the person, and the candidate preview
// shows no sponsor rather than a service name.
func isServiceSponsorLabel(handle string) bool {
	switch strings.TrimSpace(handle) {
	case "", "operator", "onboard-api", "id-console":
		return true
	}
	return false
}

// onboardAPIActor is the machine-realm attribution: the resolved per-handle
// principal. Falls back to the "onboard-api" label only if the principal
// is somehow unset (the gate always sets it, so this is defensive).
func onboardAPIActor(r *http.Request) string {
	if h := onboardPrincipalFromCtx(r.Context()).Handle; h != "" {
		return h
	}
	return "onboard-api"
}

// --- proposals: list / get -------------------------------------------

func (s *Server) apiListProposals(w http.ResponseWriter, r *http.Request) {
	p := onboardPrincipalFromCtx(r.Context())
	status := strings.TrimSpace(r.URL.Query().Get("status"))
	email := strings.TrimSpace(r.URL.Query().Get("email"))
	if r.URL.Query().Has("email") && email == "" {
		onboardError(w, http.StatusBadRequest, "blank_email_filter", "email filter is blank; pass an address or omit the filter")
		return
	}
	ps, err := s.Onboard.ListProposalsFiltered(r.Context(), status, email)
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "list_error", "could not list proposals")
		return
	}
	// Own-invitee scoping (ADR 1072): a non-escalated participant sponsor
	// sees only proposals carrying their sponsorship.
	out := make([]onboardstore.Proposal, 0, len(ps))
	for _, row := range ps {
		if p.allowsSponsor(row.Sponsor) {
			out = append(out, row)
		}
	}
	onboardJSON(w, http.StatusOK, out)
}

func (s *Server) apiGetProposal(w http.ResponseWriter, r *http.Request) {
	pr := onboardPrincipalFromCtx(r.Context())
	p, err := s.Onboard.GetProposal(r.Context(), r.PathValue("id"))
	if errors.Is(err, onboardstore.ErrNotFound) {
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "get_error", "could not load proposal")
		return
	}
	// Out-of-scope ids read 404, not 403, so ids don't leak existence.
	if !pr.allowsSponsor(p.Sponsor) {
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return
	}
	onboardJSON(w, http.StatusOK, p)
}

// --- proposals: approve / reject (two-gate) --------------------------

// requireOperatorPrincipal refuses non-operator (participant-key) principals
// on the admin-stage and runner-contract actions. Participant sponsors get
// mint + own-invitee list/vouch/revoke; the admin decisions and the mint
// pipeline stay operator-equivalent only (ADR 1072 — before this guard any
// sponsorship-authorized participant key could reach admin approve/claim).
func requireOperatorPrincipal(w http.ResponseWriter, r *http.Request) bool {
	if onboardPrincipalFromCtx(r.Context()).IsOperator {
		return true
	}
	onboardError(w, http.StatusForbidden, "operator_only",
		"this action is operator-only — sponsors approve or decline their own invitees at the sponsor stage")
	return false
}

type proposalDecisionRequest struct {
	Note string `json:"note,omitempty"`
}

func decodeProposalDecision(w http.ResponseWriter, r *http.Request, req *proposalDecisionRequest) bool {
	if err := json.NewDecoder(r.Body).Decode(req); err != nil && !errors.Is(err, io.EOF) {
		onboardError(w, http.StatusBadRequest, "bad_request", "request body must be valid JSON")
		return false
	}
	return true
}

// apiApproveProposal is the stage-3 admin approve (machine realm, operator-
// equivalent). Records `approved` only — the runner claims + mints (the YAML/
// command response the portal returned is dropped, per the extraction plan).
func (s *Server) apiApproveProposal(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	var req proposalDecisionRequest
	if !decodeProposalDecision(w, r, &req) {
		return
	}
	status, err := s.decideProposal(r, r.PathValue("id"), "approve", onboardAPIActor(r), req.Note, "")
	if err != nil {
		s.writeDecideError(w, err)
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{"id": r.PathValue("id"), "status": status})
}

// sponsorVouchGate scopes the stage-2 vouch for participant principals
// (ADR 1071 + 1072): the proposal must carry the caller's own sponsorship
// (foreign ids read 404 — no existence leak) and its tier must sit within the
// caller's cap. Operators pass untouched. Writes the refusal itself.
func (s *Server) sponsorVouchGate(w http.ResponseWriter, r *http.Request, id string) bool {
	pr := onboardPrincipalFromCtx(r.Context())
	if pr.IsOperator {
		return true
	}
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if errors.Is(err, onboardstore.ErrNotFound) {
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return false
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "get_error", "could not load proposal")
		return false
	}
	if !pr.allowsSponsor(p.Sponsor) {
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return false
	}
	if pr.MaxTier != "" && strings.TrimSpace(p.Tier) != "" {
		h, herr := loadIntakeHints()
		if herr != nil {
			onboardError(w, http.StatusInternalServerError, "hints_error", "onboarding config unavailable")
			return false
		}
		if !h.tierWithin(p.Tier, pr.MaxTier) {
			onboardError(w, http.StatusForbidden, "tier_exceeds_sponsorship",
				fmt.Sprintf("you may vouch up to %q — this proposal carries %q", pr.MaxTier, p.Tier))
			return false
		}
	}
	return true
}

// apiSponsorApproveProposal is the stage-2 sponsor vouch (machine realm). Records
// proposed → sponsor_approved. Participant sponsors vouch only their own
// invitees, within their tier cap (sponsorVouchGate).
func (s *Server) apiSponsorApproveProposal(w http.ResponseWriter, r *http.Request) {
	if !s.sponsorVouchGate(w, r, r.PathValue("id")) {
		return
	}
	var req proposalDecisionRequest
	if !decodeProposalDecision(w, r, &req) {
		return
	}
	status, err := s.decideProposal(r, r.PathValue("id"), "sponsor-approve", onboardAPIActor(r), req.Note, "")
	if err != nil {
		s.writeDecideError(w, err)
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{"id": r.PathValue("id"), "status": status})
}

func (s *Server) apiSponsorRejectProposal(w http.ResponseWriter, r *http.Request) {
	if !s.sponsorVouchGate(w, r, r.PathValue("id")) {
		return
	}
	var req proposalDecisionRequest
	if !decodeProposalDecision(w, r, &req) {
		return
	}
	status, err := s.decideProposal(r, r.PathValue("id"), "sponsor-reject", onboardAPIActor(r), req.Note, "")
	if err != nil {
		s.writeDecideError(w, err)
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{"id": r.PathValue("id"), "status": status})
}

func (s *Server) apiRejectProposal(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	var req proposalDecisionRequest
	if !decodeProposalDecision(w, r, &req) {
		return
	}
	status, err := s.decideProposal(r, r.PathValue("id"), "reject", onboardAPIActor(r), req.Note, "")
	if err != nil {
		s.writeDecideError(w, err)
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{"id": r.PathValue("id"), "status": status})
}

func (s *Server) apiOffboardProposal(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	var req proposalDecisionRequest
	if !decodeProposalDecision(w, r, &req) {
		return
	}
	status, err := s.decideProposal(r, r.PathValue("id"), "offboard", onboardAPIActor(r), req.Note, "")
	if err != nil {
		s.writeDecideError(w, err)
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{"id": r.PathValue("id"), "status": status})
}

// writeDecideError maps a decideError's status onto the portal-shaped error body.
func (s *Server) writeDecideError(w http.ResponseWriter, err error) {
	var de *decideError
	if errors.As(err, &de) {
		code := "decide_error"
		switch de.status {
		case http.StatusNotFound:
			code = "not_found"
		case http.StatusConflict:
			code = "not_pending"
		case http.StatusBadRequest:
			code = "bad_request"
		}
		onboardError(w, de.status, code, de.msg)
		return
	}
	onboardError(w, http.StatusInternalServerError, "decide_error", err.Error())
}

// --- proposals: onboarded / claim / result (runner contract) ---------

func (s *Server) apiMarkOnboarded(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	id := r.PathValue("id")
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if errors.Is(err, onboardstore.ErrNotFound) {
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "get_error", "could not load proposal")
		return
	}
	if p.Status != onboardstore.ProposalApproved {
		onboardError(w, http.StatusConflict, "not_approved",
			"proposal is "+p.Status+", not approved — cannot mark onboarded")
		return
	}
	if err := s.Onboard.SetProposalStatus(r.Context(), id, onboardstore.ProposalOnboarded); err != nil {
		onboardError(w, http.StatusInternalServerError, "status_error", "could not update status")
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{"id": id, "status": onboardstore.ProposalOnboarded})
}

// apiClaimProposal atomically transitions an approved proposal to onboarding so
// the onboard-runner can mint it without a second runner racing (exactly one claim
// wins). 409 if the proposal isn't approved.
func (s *Server) apiClaimProposal(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	id := r.PathValue("id")
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if errors.Is(err, onboardstore.ErrNotFound) {
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "get_error", "could not load proposal")
		return
	}
	if err := s.Onboard.ClaimProposal(r.Context(), id); err != nil {
		switch {
		case errors.Is(err, onboardstore.ErrProposalNotApproved):
			onboardError(w, http.StatusConflict, "not_approved",
				"proposal is not in the approved state — already claimed, minted, or rejected")
		case errors.Is(err, onboardstore.ErrNotFound):
			onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		default:
			onboardError(w, http.StatusInternalServerError, "claim_error", "could not claim proposal")
		}
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{
		"id": p.ID, "status": onboardstore.ProposalOnboarding, "handle": p.Handle, "env": p.Env,
	})
}

// apiOffboardClaim atomically transitions an offboard-requested proposal to
// offboarding so the onboard-runner can execute the revoke without a second
// runner racing (exactly one claim wins) — the offboard counterpart of
// apiClaimProposal. 409 when no cut is queued: the runner never cuts anyone
// on its own initiative.
func (s *Server) apiOffboardClaim(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	id := r.PathValue("id")
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if errors.Is(err, onboardstore.ErrNotFound) {
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "get_error", "could not load proposal")
		return
	}
	if err := s.Onboard.ClaimOffboard(r.Context(), id); err != nil {
		switch {
		case errors.Is(err, onboardstore.ErrReissueRunning):
			onboardError(w, http.StatusConflict, "reissue_running", err.Error())
		case errors.Is(err, onboardstore.ErrProposalNotOffboardRequested):
			onboardError(w, http.StatusConflict, "not_offboard_requested",
				"proposal has no queued offboard — not requested, already claimed, or already terminal")
		case errors.Is(err, onboardstore.ErrNotFound):
			onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		default:
			onboardError(w, http.StatusInternalServerError, "claim_error", "could not claim offboard")
		}
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{
		"id": p.ID, "status": onboardstore.ProposalOffboarding, "handle": p.Handle, "env": p.Env,
	})
}

type proposalResultRequest struct {
	Status   string `json:"status"` // "onboarded" / "failed", or "offboarded" / "offboard-failed" from the offboard leg
	Error    string `json:"error,omitempty"`
	SetupKey string `json:"setup_key,omitempty"` // onboarded only — persisted for the welcome package
	// HandsetKey is the OBSERVER-only key for the person's phones and tablets
	// (ADR 1175), persisted beside SetupKey for the welcome package.
	HandsetKey string `json:"handset_key,omitempty"`
	// PortalAPIKey is the scoped `vp_…` portal key the mint issued. Passed
	// through for the welcome email ONLY and never persisted: it is broader and
	// longer-lived than the setup key. Before this existed the email told people
	// to "find it under your identity once you can reach the Portal" — circular,
	// since the key IS how you reach the Portal.
	PortalAPIKey string `json:"portal_api_key,omitempty"`
	BundlePath   string `json:"bundle_path,omitempty"` // onboarded only — the fabric bundle's mesh download path, rendered as a link
	// SlotAnchor is the write anchor (= slot_id = cert CN) the mint REPORTED.
	// Preferred over parsing BundlePath for the person→slot linkage: a slot can
	// legitimately have no bundle (mint-on-collect defers the credential), and the
	// path derivation reads that as "no fabric leg" and skips in silence.
	SlotAnchor string `json:"slot_anchor,omitempty"`
	// CredentialDeferred: the mint deliberately issued no fabric credential
	// (mint-on-collect), so the welcome email points at the collection page
	// instead of printing a key. Stated by the runner, never inferred from an
	// empty PortalAPIKey — that is also what a failed scrape looks like.
	CredentialDeferred bool `json:"credential_deferred,omitempty"`
	// Collaboration-environment fields (onboarded only, when collab.enabled). The
	// account is persisted read-only for mesh awareness; the setup link is
	// credential-bearing + one-time — used for the welcome email, never stored.
	CollabAccount   string `json:"collab_account,omitempty"`
	CollabSetupLink string `json:"collab_setup_link,omitempty"`
	CollabActivated bool   `json:"collab_activated,omitempty"`
}

// apiProposalResult records the terminal outcome of a claimed (onboarding)
// proposal: the runner writes back onboarded or failed after the native mint.
func (s *Server) apiProposalResult(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	id := r.PathValue("id")
	var req proposalResultRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_json", "invalid request body")
		return
	}
	// Each leg's terminal vocabulary is only writable from that leg's claimed
	// state: a crossed writeback is a state conflict, never a lifecycle jump.
	var fromStatus string
	switch req.Status {
	case onboardstore.ProposalOnboarded, onboardstore.ProposalFailed:
		fromStatus = onboardstore.ProposalOnboarding
	case onboardstore.ProposalOffboarded, onboardstore.ProposalOffboardFailed:
		fromStatus = onboardstore.ProposalOffboarding
	default:
		onboardError(w, http.StatusBadRequest, "bad_status",
			`status must be "onboarded", "failed", "offboarded" or "offboard-failed"`)
		return
	}
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if errors.Is(err, onboardstore.ErrNotFound) {
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "get_error", "could not load proposal")
		return
	}
	if p.Status != fromStatus {
		onboardError(w, http.StatusConflict, "not_"+fromStatus,
			"proposal is "+p.Status+", not "+fromStatus+" — cannot record a "+req.Status+" result")
		return
	}
	// Persist the setup key/bundle path alongside the status, and the failure
	// reason (req.Error) as the note on a `failed` result — so the admin queue
	// shows WHY a mint failed instead of discarding it.
	if err := s.Onboard.SetProposalResult(r.Context(), id, req.Status, req.SetupKey, req.HandsetKey, req.BundlePath, req.Error, req.CollabAccount, req.CollabActivated); err != nil {
		onboardError(w, http.StatusInternalServerError, "status_error", "could not update status")
		return
	}
	// Finalize delivery: on a successful mint, auto-email the complete welcome
	// package (mesh join + login + surfaces + fabric bundle download link) to the
	// enrollee. Best-effort — never blocks or fails the onboarded result.
	if req.Status == onboardstore.ProposalOnboarded {
		p.SetupKey = req.SetupKey
		p.HandsetKey = req.HandsetKey
		p.BundlePath = req.BundlePath
		p.CollabAccount = req.CollabAccount
		p.CollabActivated = req.CollabActivated
		// Repo-collaborator grants (docs/design/onboarding-github-repo-grants.md).
		// Fail-open — a GitHub error never fails the onboard or blocks the welcome;
		// grantRepoAccess records the outcomes on the proposal (p.RepoGrantsJSON) so
		// sendWelcomeEmail surfaces them via recipientFromProposal.
		s.grantRepoAccess(r.Context(), p)
		// The one-time collab setup link is passed through for the email only (not
		// persisted — it is credential-bearing).
		// Bind the slot to the person in the portal registry (ADR 1051). Done
		// here because this is the first moment both halves are known: the mint
		// creates the slot before the Ory identity exists (Ory is deliberately
		// last, so its failure degrades rather than blocks), and the runner's
		// writeback carries the slot anchor in the bundle path. Best-effort —
		// see onboard_slot_identity.go.
		s.stampSlotIdentity(r.Context(), p.Handle, p.Email, req.SlotAnchor, req.BundlePath)
		s.sendWelcomeEmail(r, p, req.SetupKey, req.HandsetKey, req.CollabSetupLink, req.PortalAPIKey, req.CredentialDeferred)
	}
	onboardJSON(w, http.StatusOK, map[string]any{"id": id, "status": req.Status})
}

// --- invites: list / get / revoke ------------------------------------

// --- mint invite (machine realm — the `goat admin invite` CLI path) --------

type mintInviteRequest struct {
	Env          string `json:"env"`
	Tier         string `json:"tier"`
	Affiliation  string `json:"affiliation"`
	Site         string `json:"site"`
	MachineQuota int    `json:"machine_quota"`
	TTLHours     int    `json:"ttl_hours"`
	Note         string `json:"note"`
	// Collab pre-binds the collaboration-environment decision, exactly as the web
	// invite page's checkbox does. Without it the machine path could not express
	// the choice at all, so every CLI/API-minted invite silently landed on false
	// while the web page defaulted it true — two mint paths, two different
	// outcomes, no signal to the sponsor. Pointer: absent = inherit the
	// default rather than mean "no".
	Collab *bool `json:"collab"`
	// Email, when set, delivers the one-time nomination link to the candidate
	// via SES right after minting — the machine-path equivalent of the web
	// invite page's "email the link" action, so an automated/operator-key mint
	// is self-contained and doesn't require a logged-in session to reach the
	// candidate. Empty = mint only (caller hands out the link).
	Email string `json:"email"`
}

type mintInviteResponse struct {
	ID             string                `json:"id"`
	Sponsor        string                `json:"sponsor"`
	SponsorName    string                `json:"sponsor_name,omitempty"`
	Env            string                `json:"env"`
	Prebound       onboardstore.Prebound `json:"prebound"`
	ExpiresAt      int64                 `json:"expires_at"`
	Status         string                `json:"status"`
	Token          string                `json:"token"`
	IntakePath     string                `json:"intake_path"`
	IntakeURL      string                `json:"intake_url,omitempty"`
	NominationURL  string                `json:"nomination_url,omitempty"`
	NominationPath string                `json:"nomination_path"`
	Escalated      bool                  `json:"escalated"`
	// EmailedTo is the candidate address the link was delivered to when the mint
	// request carried `email` (empty = not emailed); EmailError is set instead
	// when a requested send failed (the link is still returned so the caller can
	// hand it over manually).
	EmailedTo  string `json:"emailed_to,omitempty"`
	EmailError string `json:"email_error,omitempty"`
}

// apiMintInvite mints a one-time candidate-onboarding invite (machine realm —
// the `goat admin invite` CLI). The sponsor is the resolved per-handle principal;
// an operator is unrestricted (ADR 1071 "operators sponsor by default"), a
// participant is capped at its portal-resolved SponsorMaxTier — the cap is
// server-set onto the invite so preview/submit bound the enrollee's tier too.
// JSON shape mirrors the portal's so the CLI is a drop-in re-point.
func (s *Server) apiMintInvite(w http.ResponseWriter, r *http.Request) {
	p := onboardPrincipalFromCtx(r.Context())
	var req mintInviteRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_request", "body must be JSON")
		return
	}
	env := strings.TrimSpace(req.Env)
	// env is REQUIRED, not optional: a blank env yields a blank-env proposal that a
	// scoped onboard-runner (`--only-env <env>`) silently skips forever — the exact
	// strand the well-oiled-machine work is closing. Fail closed with an actionable
	// message so the CLI/caller passes one.
	if env == "" {
		onboardError(w, http.StatusBadRequest, "env_required",
			`env is required (e.g. "galaxy") — a blank env strands the proposal for a scoped onboard-runner`)
		return
	}
	if !envPattern.MatchString(env) {
		onboardError(w, http.StatusBadRequest, "bad_env", "env must be a lowercase goatnet slug (e.g. galaxy)")
		return
	}
	// Validate the pre-bound affiliation KIND here, at mint.
	//
	// `--affiliation` binds affiliation.kind — a four-value enum — and the
	// candidate supplies affiliation.id themselves at intake. Without a check here,
	// an org shortname would pass straight through and the failure would
	// land at APPROVE on the lint gate: "value must be one of
	// 'vendor', 'organic', 'partner', 'civil'". By then the candidate has
	// filled in the form, and affiliation.kind is outside ADR 1137's closed
	// editable set, so the proposal cannot be corrected. The value is knowable as
	// wrong at the moment it is typed.
	if k := strings.TrimSpace(req.Affiliation); k != "" && !isAffiliationKind(k) {
		onboardError(w, http.StatusBadRequest, "bad_affiliation_kind", fmt.Sprintf(
			"affiliation %q is not a kind — this field is the four-value enum %s. "+
				"It is the TYPE of organisation, not its name: the candidate supplies "+
				"their org id (e.g. %q) themselves at intake. A wrong value here is only "+
				"refused at approval, after the candidate has filled in the form.",
			k, strings.Join(affiliationKinds, " | "), k))
		return
	}

	ttl := req.TTLHours
	if ttl == 0 {
		ttl = defaultInviteTTLHours
	}
	if ttl < minInviteTTLHours || ttl > maxInviteTTLHours {
		onboardError(w, http.StatusBadRequest, "bad_ttl", fmt.Sprintf("ttl_hours must be between %d and %d", minInviteTTLHours, maxInviteTTLHours))
		return
	}

	prebound := onboardstore.Prebound{
		Tier:         strings.TrimSpace(req.Tier),
		Affiliation:  strings.TrimSpace(req.Affiliation),
		Site:         strings.TrimSpace(req.Site),
		MachineQuota: req.MachineQuota,
		// Absent → the web page's default (checked). Parity is the point: the
		// same invite minted from the CLI and from the page must provision the
		// same thing.
		Collab: req.Collab == nil || *req.Collab,
	}

	// ADR 1071 cap. Operators are unrestricted; a participant's ceiling was
	// resolved from the portal at auth. The cap is server-set, never from the
	// request, and stamped onto the invite so preview/submit bound the enrollee.
	maxTier := ""
	if !p.IsOperator {
		maxTier = p.MaxTier
		if maxTier == "" {
			// Authorized participant with no recorded max → least-privileged tier.
			if h, herr := loadIntakeHints(); herr == nil {
				maxTier = h.lowestTier()
			}
		}
		if maxTier != "" && prebound.Tier != "" {
			h, herr := loadIntakeHints()
			if herr != nil {
				onboardError(w, http.StatusInternalServerError, "hints_error", "onboarding config unavailable")
				return
			}
			if !h.tierWithin(prebound.Tier, maxTier) {
				onboardError(w, http.StatusForbidden, "tier_exceeds_sponsorship",
					fmt.Sprintf("you may sponsor up to %q — %q exceeds your sponsorship level", maxTier, prebound.Tier))
				return
			}
		}
	}
	prebound.SponsorMaxTier = maxTier

	preboundJSON, err := json.Marshal(prebound)
	_ = err // Prebound is a plain struct; JSON encode cannot fail here in practice.
	// Sponsor attribution (ADR 1112 §11 / F-207): a mint through the shared
	// legacy operator key resolves to the generic "operator" label — when the
	// caller sent the per-person delegated-attribution header (goat-cli sends
	// it from GOAT_OPERATOR_HANDLE), record the PERSON as sponsor. The header
	// never changes authorization (the key already resolved operator) and
	// never overrides a real per-handle identity — it only replaces the
	// service labels in isServiceSponsorLabel.
	sponsor, sponsorName := p.Handle, firstNonEmpty(p.Name, p.Handle)
	if p.IsOperator {
		if actor := strings.TrimSpace(r.Header.Get("X-Goat-Operator-Actor")); actor != "" &&
			isServiceSponsorLabel(sponsor) {
			sponsor, sponsorName = actor, actor
		}
	}
	inv := &onboardstore.Invite{
		Sponsor:      sponsor,
		SponsorName:  sponsorName,
		Env:          env,
		PreboundJSON: string(preboundJSON),
		ExpiresAt:    time.Now().Add(time.Duration(ttl) * time.Hour).Unix(),
		Note:         strings.TrimSpace(req.Note),
		// Addressed at mint, before the send below is attempted: who the link
		// was meant for is part of the record even when the send fails.
		CandidateEmail: strings.TrimSpace(req.Email),
	}
	if refusal := s.checkInviteRecipient(r, inv.CandidateEmail); refusal != nil {
		onboardError(w, refusal.status, refusal.code, refusal.message)
		return
	}
	token, err := s.Onboard.MintInvite(r.Context(), inv)
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "mint_error", "could not mint invite")
		return
	}

	// Clean candidate-facing path; the intake collector rewrites it to the
	// console's /public/onboard.html. Emitting /public 404s at the front desk.
	nomPath := "/onboard.html?t=" + token
	resp := mintInviteResponse{
		ID:             inv.ID,
		Sponsor:        inv.Sponsor,
		SponsorName:    inv.SponsorName,
		Env:            inv.Env,
		Prebound:       prebound,
		ExpiresAt:      inv.ExpiresAt,
		Status:         inv.Status,
		Token:          token,
		IntakePath:     "/?t=" + token,
		NominationPath: nomPath,
		Escalated:      isEscalatedTier(prebound.Tier),
	}
	if base := strings.TrimRight(s.Cfg.IntakeBaseURL, "/"); base != "" {
		resp.IntakeURL = base + nomPath
	}
	if base := strings.TrimRight(s.Cfg.PortalPublicURL, "/"); base != "" {
		resp.NominationURL = base + nomPath
	}

	// Machine-path delivery: if the caller asked to email the candidate, send the
	// one-time link now via the same Mailer.SendInvite the web invite page uses.
	// Best-effort — a send failure never fails the mint; the link is still
	// returned so the caller can hand it over. Audited as onboard_email_invite,
	// mirroring the web handler.
	if to := strings.TrimSpace(req.Email); to != "" {
		link := firstNonEmpty(resp.IntakeURL, resp.NominationURL, nomPath)
		actor := Actor{Email: p.Handle}
		if s.Mailer == nil || !s.Mailer.Enabled() {
			resp.EmailError = "email sending is not configured on this console"
			s.emitAuditWithInviteToken(r, actor, "onboard_email_invite", inv.ID, "", nil, map[string]interface{}{"to": to}, false, "error", resp.EmailError)
		} else if err := s.Mailer.SendInvite(to, link, firstNonEmpty(inv.SponsorName, inv.Sponsor), inv.Env, s.Cfg.NotifyGoatnetTagline, s.Cfg.NotifyGoatnetPurpose, inv.ExpiresAt); err != nil {
			resp.EmailError = err.Error()
			s.emitAuditWithInviteToken(r, actor, "onboard_email_invite", inv.ID, "", nil, map[string]interface{}{"to": to}, false, "error", err.Error())
		} else {
			resp.EmailedTo = to
			s.recordInviteSend(r, inv.ID, to, onboardstore.SentViaEmail)
			s.emitAuditWithInviteToken(r, actor, "onboard_email_invite", inv.ID, "", nil, map[string]interface{}{"to": to}, false, "ok", "")
		}
	}

	onboardJSON(w, http.StatusCreated, resp)
}

func (s *Server) apiListInvites(w http.ResponseWriter, r *http.Request) {
	pr := onboardPrincipalFromCtx(r.Context())
	status := strings.TrimSpace(r.URL.Query().Get("status"))
	invs, err := s.Onboard.ListInvites(r.Context(), status)
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "list_error", "could not list invites")
		return
	}
	// Own-invitee scoping (ADR 1072): a non-escalated participant sponsor
	// sees only invites they minted.
	out := make([]onboardstore.Invite, 0, len(invs))
	for i := range invs {
		if !pr.allowsSponsor(invs[i].Sponsor) {
			continue
		}
		pb, err := invs[i].Prebound()
		if err != nil {
			slog.ErrorContext(r.Context(), "onboarding API: invite access metadata unreadable",
				slog.String("invite_id", invs[i].ID), slog.String("err", err.Error()))
			invs[i].MetadataInvalid = true
			invs[i].Escalated = false
			out = append(out, invs[i])
			continue
		}
		invs[i].Escalated = isEscalatedTier(pb.Tier)
		out = append(out, invs[i])
	}
	onboardJSON(w, http.StatusOK, out)
}

func (s *Server) apiGetInvite(w http.ResponseWriter, r *http.Request) {
	pr := onboardPrincipalFromCtx(r.Context())
	inv, err := s.Onboard.GetInvite(r.Context(), r.PathValue("id"))
	if errors.Is(err, onboardstore.ErrNotFound) {
		onboardError(w, http.StatusNotFound, "not_found", "invite not found")
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "get_error", "could not load invite")
		return
	}
	if !pr.allowsSponsor(inv.Sponsor) {
		onboardError(w, http.StatusNotFound, "not_found", "invite not found")
		return
	}
	pb, err := inv.Prebound()
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "get_error", "could not decode invite access metadata")
		return
	}
	inv.Escalated = isEscalatedTier(pb.Tier)
	onboardJSON(w, http.StatusOK, inv)
}

func (s *Server) apiRevokeInvite(w http.ResponseWriter, r *http.Request) {
	pr := onboardPrincipalFromCtx(r.Context())
	id := r.PathValue("id")
	if !pr.unrestrictedView() {
		inv, gerr := s.Onboard.GetInvite(r.Context(), id)
		if errors.Is(gerr, onboardstore.ErrNotFound) || (gerr == nil && !pr.allowsSponsor(inv.Sponsor)) {
			onboardError(w, http.StatusNotFound, "not_found", "invite not found")
			return
		}
		if gerr != nil {
			onboardError(w, http.StatusInternalServerError, "get_error", "could not load invite")
			return
		}
	}
	err := s.Onboard.RevokeInvite(r.Context(), id)
	switch {
	case err == nil:
		// ok
	case errors.Is(err, onboardstore.ErrNotFound):
		onboardError(w, http.StatusNotFound, "not_found", "invite not found")
		return
	case errors.Is(err, onboardstore.ErrInviteConsumed):
		onboardError(w, http.StatusConflict, "not_open", "invite is no longer open — cannot revoke")
		return
	default:
		onboardError(w, http.StatusInternalServerError, "revoke_error", "could not revoke invite")
		return
	}
	onboardJSON(w, http.StatusOK, map[string]any{"id": id, "status": onboardstore.InviteRevoked})
}

// --- intake / batch / lint / schema (machine realm) ------------------

type onboardIntakeRequest struct {
	Manifest json.RawMessage `json:"manifest"`
	Env      string          `json:"env"`
}

type onboardIntakeResponse struct {
	ID      string `json:"id"`
	Status  string `json:"status"`
	Handle  string `json:"handle"`
	Sponsor string `json:"sponsor"`
}

// apiOnboardIntake stores a validated onboarding manifest as an INERT proposal
// (ADR 1061). Mints nothing. The sponsor is bound server-side to the machine
// actor. Re-lints before storing so a green proposal passes the onboard gate.
func (s *Server) apiOnboardIntake(w http.ResponseWriter, r *http.Request) {
	var req onboardIntakeRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_request", "body must be JSON: {manifest, env}")
		return
	}
	if len(req.Manifest) == 0 {
		onboardError(w, http.StatusBadRequest, "empty_manifest", "manifest is required")
		return
	}
	env := strings.TrimSpace(req.Env)
	if env != "" && !envPattern.MatchString(env) {
		onboardError(w, http.StatusBadRequest, "bad_env", "env must be a lowercase goatnet slug (e.g. galaxy)")
		return
	}
	var tree map[string]any
	if err := json.Unmarshal(req.Manifest, &tree); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_manifest", "manifest must be a JSON object")
		return
	}
	stampAdded(tree)
	canonical, err := json.Marshal(tree)
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "encode_error", "could not encode manifest")
		return
	}
	res := onboardLint(canonical, "json")
	if !res.OK {
		lintJSON, err := json.Marshal(res)
		_ = err // lint response is always JSON-marshalable; encode failure is not expected here.
		onboardJSON(w, http.StatusBadRequest, json.RawMessage(lintJSON))
		return
	}
	handle, email := manifestHandleEmail(tree)
	if handle == "" {
		onboardError(w, http.StatusBadRequest, "no_handle", "manifest is missing identity.handle")
		return
	}
	lintJSON, err := json.Marshal(res)
	_ = err // lint response is always JSON-marshalable; encode failure is not expected here.
	principal := onboardPrincipalFromCtx(r.Context())
	p := &onboardstore.Proposal{
		Sponsor:      onboardAPIActor(r),
		SponsorName:  firstNonEmpty(principal.Name, onboardAPIActor(r)),
		Env:          env,
		Handle:       handle,
		Email:        email,
		Status:       onboardstore.ProposalProposed,
		ManifestJSON: string(canonical),
		LintJSON:     string(lintJSON),
		Complete:     true,
		// Submitted whole by an API principal: affiliation.id is theirs, not a
		// candidate's typed claim.
		OrgSource: onboardstore.OrgSourceSponsor,
	}
	if err := s.Onboard.CreateProposal(r.Context(), p); err != nil {
		onboardError(w, http.StatusInternalServerError, "store_error", "could not store proposal")
		return
	}
	onboardJSON(w, http.StatusCreated, onboardIntakeResponse{
		ID: p.ID, Status: p.Status, Handle: p.Handle, Sponsor: p.Sponsor,
	})
}

// stampAdded sets identity.added to today when missing/placeholder, so
// required-field validation passes server-side.
func stampAdded(tree map[string]any) {
	id, ok := tree["identity"].(map[string]any)
	if !ok {
		return
	}
	if v, _ := id["added"].(string); v == "" || strings.HasPrefix(v, "<") {
		id["added"] = time.Now().UTC().Format("2006-01-02")
	}
}

func (s *Server) apiOnboardLint(w http.ResponseWriter, r *http.Request) {
	var req onboardLintRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_request", "body must be JSON: {content, format?}")
		return
	}
	if strings.TrimSpace(req.Content) == "" {
		onboardError(w, http.StatusBadRequest, "empty_content", "content is required and must be non-empty")
		return
	}
	res := onboardLint([]byte(req.Content), req.Format)
	onboardJSON(w, http.StatusOK, onboardLintResponse{
		OK: res.OK, Findings: res.Findings, Shape: res.Shape, Detail: res.Detail,
	})
}

type onboardLintRequest struct {
	Content string `json:"content"`
	Format  string `json:"format,omitempty"`
}

type onboardLintResponse struct {
	OK       bool                  `json:"ok"`
	Findings []onboardlint.Finding `json:"findings"`
	Shape    string                `json:"shape,omitempty"`
	Detail   string                `json:"detail,omitempty"`
}

func (s *Server) apiOnboardSchema(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-cache")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write(onboardlint.SchemaJSON())
}

// apiEmailReference (POST /api/onboard/email-reference {"email": ...}) sends
// an existing tenant identity the complete current-role reference email — the
// machine path for catch-up sends (people whose access predates the
// role-change emails, e.g. the pre-buildout cohort). Operator principals only:
// a sponsorship-capped participant has no business emailing arbitrary
// identities.
func (s *Server) apiEmailReference(w http.ResponseWriter, r *http.Request) {
	pr := onboardPrincipalFromCtx(r.Context())
	if !pr.IsOperator {
		onboardError(w, http.StatusForbidden, "forbidden", "operator key required")
		return
	}
	var req struct {
		Email string `json:"email"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil || strings.TrimSpace(req.Email) == "" {
		onboardError(w, http.StatusBadRequest, "bad_request", `body must be {"email": "<address>"}`)
		return
	}
	cli := s.Kratos[tenantIdPKey]
	if cli == nil {
		onboardError(w, http.StatusServiceUnavailable, "not_configured", "tenant realm unavailable")
		return
	}
	ids, err := cli.ListIdentities(r.Context())
	if err != nil {
		onboardError(w, http.StatusBadGateway, "kratos_error", "list identities failed")
		return
	}
	var target *kratos.Identity
	for i := range ids {
		if strings.EqualFold(strings.TrimSpace(ids[i].Email()), strings.TrimSpace(req.Email)) {
			target = &ids[i]
			break
		}
	}
	if target == nil {
		onboardError(w, http.StatusNotFound, "not_found", "no tenant identity with that email")
		return
	}
	if err := s.sendReferenceEmail(r, tenantIdPKey, target, Actor{Email: pr.Handle}); err != nil {
		onboardError(w, http.StatusBadGateway, "send_error", err.Error())
		return
	}
	w.WriteHeader(http.StatusAccepted)
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(map[string]string{"sent_to": target.Email()})
}

// apiRetryProposal (POST /api/onboard/proposals/{id}/retry) returns a FAILED
// proposal to `approved` so the runner re-attempts the mint.
//
// A mint can fail for reasons that have nothing to do with the decision — a
// runner without its mesh token, an unreachable portal, a half-provisioned
// remnant from a previous attempt — and the row must not be terminal at `failed`
// in the UI or the API. The alternative is a hand-written UPDATE against the
// console's SQLite file: precisely the out-of-band surgery onboarding-rails.md
// forbids.
//
// Retry re-runs the MINT, never a decision: both human gates were already
// passed to reach `failed`, and the store guard admits only rows currently in
// that state, so this cannot resurrect a rejected proposal, interrupt a live
// mint, or repeat a completed one.
// apiArchiveProposal puts an onboarded-but-never-redeemed proposal into cold
// storage (ADR 1136). The ADR 1136 sweep revokes the unredeemed setup key on
// the mesh side; this is the console half it always said belonged here and
// which, until now, did not exist — so the sweep could tidy the credential and
// never the record.
//
// Archive rather than delete: the row keeps its sponsor and both gate
// approvals, which is what lets apiRestoreProposal below re-mint without asking
// two people to re-approve somebody they already approved.
func (s *Server) apiArchiveProposal(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	id := r.PathValue("id")
	reason := strings.TrimSpace(r.URL.Query().Get("reason"))
	if reason == "" {
		reason = "setup key never redeemed; revoked by the dormancy sweep"
	}
	ok, err := s.Onboard.ArchiveProposal(r.Context(), id, reason)
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "archive_error", "could not archive proposal")
		return
	}
	if !ok {
		// 409, not 404: the row may well exist, in a state that refuses this.
		// Naming `enrolled` explicitly because it is the refusal that matters —
		// that person joined, and archiving them would be the real damage.
		onboardError(w, http.StatusConflict, "not_onboarded",
			"only an onboarded proposal can be archived — an enrolled person has used their "+
				"credential and must not be shelved, and every other stage has its own meaning")
		return
	}
	s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "onboard_archive_proposal", nil, "",
		map[string]interface{}{"status": onboardstore.ProposalOnboarded},
		map[string]interface{}{"status": onboardstore.ProposalArchived, "proposal_id": id, "reason": reason},
		false, "ok", "")
	onboardJSON(w, http.StatusOK, map[string]any{"id": id, "status": onboardstore.ProposalArchived})
}

// apiRestoreProposal is the single admin action that brings a shelved person
// back: the row returns to `approved`, the runner claims it, mints a FRESH
// setup key, and the console re-sends the welcome package. Neither human gate
// is repeated, because the row never lost the record of them being passed.
func (s *Server) apiRestoreProposal(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	id := r.PathValue("id")
	ok, err := s.Onboard.RestoreProposal(r.Context(), id, onboardAPIActor(r))
	if errors.Is(err, onboardstore.ErrInvalidManifest) {
		onboardError(w, http.StatusConflict, "invalid_manifest", err.Error())
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "restore_error", "could not restore proposal")
		return
	}
	if !ok {
		onboardError(w, http.StatusConflict, "not_archived",
			"only an archived proposal can be restored — check its current stage")
		return
	}
	// Audited at the same weight as approve: a restore re-enters the
	// mint-capable queue and will put a live credential in somebody's inbox.
	s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "onboard_restore_proposal", nil, "",
		map[string]interface{}{"status": onboardstore.ProposalArchived},
		map[string]interface{}{"status": onboardstore.ProposalApproved, "proposal_id": id},
		false, "ok", "")
	onboardJSON(w, http.StatusOK, map[string]any{"id": id, "status": onboardstore.ProposalApproved})
}

func (s *Server) apiRetryProposal(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	id := r.PathValue("id")
	err := s.Onboard.RetryProposal(r.Context(), id)
	switch {
	case errors.Is(err, onboardstore.ErrInvalidManifest):
		onboardError(w, http.StatusConflict, "invalid_manifest", err.Error())
		return
	case errors.Is(err, onboardstore.ErrNotFound):
		onboardError(w, http.StatusNotFound, "not_found", "proposal not found")
		return
	case errors.Is(err, onboardstore.ErrProposalNotFailed):
		// 409, not 400: the request is well-formed, the STATE refuses it.
		onboardError(w, http.StatusConflict, "not_failed",
			"only a failed proposal can be retried — check its current stage")
		return
	case err != nil:
		onboardError(w, http.StatusInternalServerError, "retry_error", "could not retry proposal")
		return
	}
	// Audited: a retry re-enters the mint-capable queue, so it belongs in the
	// operator-action record beside approve/reject.
	s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "onboard_retry_proposal", nil, "",
		map[string]interface{}{"status": onboardstore.ProposalFailed},
		map[string]interface{}{"status": onboardstore.ProposalApproved, "proposal_id": id},
		false, "ok", "")
	onboardJSON(w, http.StatusOK, map[string]any{"id": id, "status": onboardstore.ProposalApproved})
}
