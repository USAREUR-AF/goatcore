package handlers

// onboard_public.go — the PUBLIC candidate surface of the three-stage delegated
// onboarding (ADR 1062 stage 1), ported from the portal per the extraction plan.
//
// NO oauth2-proxy ADMINS gate — these are reachable without ADMINS, TOKEN-gated
// in-app by the one-time invite token (the candidate has no mesh/portal identity
// yet). They do nothing without a valid, open token and only ever produce an INERT
// proposal for review. The front-desk collector (intake.<zone>) re-points here.

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"net/http"
	"strconv"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/orgstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/web"
)

// handlePublicOnboardForm serves the candidate nomination form (static HTML,
// self-contained). Public: it renders the form shell; the token gating happens in
// the /public/onboard/preview call the page makes.
func (s *Server) handlePublicOnboardForm(w http.ResponseWriter, r *http.Request) {
	data, err := fs.ReadFile(web.FS, "static/onboard.html")
	if err != nil {
		http.Error(w, "onboarding form unavailable", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_, _ = w.Write(data)
}

type onboardPreviewResponse struct {
	Valid bool `json:"valid"`
	// Network is what the candidate's site network must allow (names + ports).
	Network *previewNetwork `json:"network,omitempty"`
	Env     string          `json:"env,omitempty"`
	Status  string          `json:"status,omitempty"`
	Reason  string          `json:"reason,omitempty"`
	// Sponsor is who minted the invite ("invited by …"). The generic shared
	// operator label is suppressed — a candidate is shown a person, or nothing.
	Sponsor            string `json:"sponsor,omitempty"`
	Escalated          bool   `json:"escalated"`
	TierLocked         bool   `json:"tier_locked"`
	AffiliationLocked  bool   `json:"affiliation_locked"`
	MachineQuotaLocked bool   `json:"machine_quota_locked"`
	// SectorLocked is true when the sponsor prebound affiliation.sector at mint.
	// The intake page then shows the sector instead of asking for it, the same
	// way it treats a locked tier.
	SectorLocked bool `json:"sector_locked"`
	// OrgLocked is true when the invite names a REGISTERED org (ADR 1112). The
	// intake page then shows the candidate their organization instead of asking
	// them to type one. OrgName is that org's display name (its id when the
	// registry carries no display name) — a label a person recognizes, never the
	// raw slug alone.
	OrgLocked              bool                  `json:"org_locked"`
	OrgName                string                `json:"org_name,omitempty"`
	Prebound               onboardstore.Prebound `json:"prebound"`
	SelectableTiers        []intakeTier          `json:"selectable_tiers"`
	SelectableAffiliations []string              `json:"selectable_affiliations"`
	// SelectableSectors is the affiliation.sector menu this goatnet offers: the
	// governmental cluster always, plus the non-governmental cluster when
	// ONBOARD_SECTORS_NONGOV_ENABLED (ADR 1077). The web form renders exactly
	// these, so a gated-off goatnet never shows the non-gov options.
	SelectableSectors []string `json:"selectable_sectors"`
	// RepoTiers is the tiers this goatnet's repo-access policy can grant
	// GitHub repositories to (repo-access.yaml, read fresh). The form warns a
	// candidate on one of these tiers who leaves the GitHub username blank:
	// they may still submit, but without it they get none of the example and
	// partner repositories until they add one.
	RepoTiers []string `json:"repo_tiers"`
	// Submission is the candidate's own submission status, present only when
	// the invite has been consumed. The invite token is the read capability:
	// whoever holds the link may see how far their request has moved, but only
	// as the closed candidate-phase vocabulary — never the raw queue status or
	// any manifest content.
	Submission *onboardSubmissionStatus `json:"submission,omitempty"`
}

// onboardSubmissionStatus is the candidate-facing view of a consumed invite's
// proposal. Phase is a closed vocabulary (pending-sponsor / pending-admin /
// provisioning / complete / rejected / issue) so the public page renders a
// stable set of states regardless of internal queue statuses.
type onboardSubmissionStatus struct {
	Phase       string `json:"phase"`
	Detail      string `json:"detail"`
	SubmittedAt int64  `json:"submitted_at,omitempty"`
	VouchedAt   int64  `json:"vouched_at,omitempty"`
	DecidedAt   int64  `json:"decided_at,omitempty"`
}

// candidateSubmissionStatus folds internal proposal statuses into the
// candidate phases. Failed provisioning is shown as "issue", not "failed":
// the retry lever is admin-side, so the candidate's only action is to wait
// (or nudge their sponsor) — same as any other in-flight phase.
// Bounds on the setup-key device allowance a candidate may ask for. The intake
// form documents "1–100" in its help text; a bound that lives only in help text
// is not a bound, so it is enforced here too. Above the ceiling is clamped, not
// refused — the number is an estimate of how many laptops someone owns, and
// dead-ending a nomination over a fat-fingered 1000 helps nobody.
const (
	minCandidateUsageLimit = 1
	maxCandidateUsageLimit = 100
)

// clampUsageLimit bounds a candidate-supplied device allowance. 0 (no number
// given) passes through as 0, which leaves the intake-form default standing.
func clampUsageLimit(n int) int {
	if n <= 0 {
		return 0
	}
	if n < minCandidateUsageLimit {
		return minCandidateUsageLimit
	}
	if n > maxCandidateUsageLimit {
		return maxCandidateUsageLimit
	}
	return n
}

func candidateSubmissionStatus(p *onboardstore.Proposal) *onboardSubmissionStatus {
	st := &onboardSubmissionStatus{SubmittedAt: p.CreatedAt, VouchedAt: p.SponsorApprovedAt}
	if p.DecidedAt != nil {
		st.DecidedAt = *p.DecidedAt
	}
	switch p.Status {
	case onboardstore.ProposalProposed:
		st.Phase, st.Detail = "pending-sponsor", "your request is waiting for your sponsor to vouch for it"
	case onboardstore.ProposalSponsorApproved:
		st.Phase, st.Detail = "pending-admin", "your sponsor has vouched — an administrator reviews it next"
	case onboardstore.ProposalApproved, onboardstore.ProposalOnboarding:
		st.Phase, st.Detail = "provisioning", "approved — your access is being provisioned"
	case onboardstore.ProposalOnboarded, onboardstore.ProposalEnrolled:
		st.Phase, st.Detail = "complete", "done — your welcome package was sent to the email you submitted"
	case onboardstore.ProposalRejected:
		st.Phase, st.Detail = "rejected", "your request was not approved — contact your sponsor for details"
	case onboardstore.ProposalFailed:
		st.Phase, st.Detail = "issue", "provisioning hit a problem on our side — your sponsor and the administrators have it; nothing is needed from you"
	case onboardstore.ProposalOffboarded:
		st.Phase, st.Detail = "closed", "your access here has ended — contact your sponsor if that is unexpected"
	default:
		st.Phase, st.Detail = "in-review", "your request is in review"
	}
	return st
}

// selectableSectors returns the ordered affiliation.sector menu for the current
// goatnet: the governmental cluster always, then the non-governmental cluster
// only when its demand-gate is on. Order is stable + human-meaningful (not map
// iteration order) so the form renders deterministically.
func selectableSectors(nongovEnabled bool) []string {
	// ADR 1151 target set FIRST, and always offered. It is deliberately not
	// behind the non-gov gate: that gate withholds the ADR 1077 non-governmental
	// CLUSTER, and `commercial` here is not that cluster — it is one of the three
	// institutional sectors. Gating it is what left a foreign commercial vendor
	// with no correct answer to give.
	out := []string{"state", "commercial", "civil-society"}
	// ADR 1077 legacy values stay selectable while the migration window is open,
	// so an in-flight invite that prebound one still resolves.
	out = append(out, "military", "law-enforcement", "civil-government", "intelligence")
	if nongovEnabled {
		out = append(out, "ngo", "media", "academic")
	}
	return out
}

// handleOnboardPreview reports whether a presented invite token is still usable
// and which access fields the enrollee may fill vs the sponsor locked. Token-gated.
// Never returns the locked VALUES (only that a field is locked).
func (s *Server) handleOnboardPreview(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		onboardError(w, http.StatusServiceUnavailable, "not_configured", "onboarding is not configured")
		return
	}
	token := strings.TrimSpace(r.URL.Query().Get("t"))
	if token == "" {
		onboardJSON(w, http.StatusOK, onboardPreviewResponse{Valid: false, Reason: "no token"})
		return
	}
	inv, err := s.Onboard.GetInviteByToken(r.Context(), token)
	if errors.Is(err, onboardstore.ErrNotFound) {
		onboardJSON(w, http.StatusOK, onboardPreviewResponse{Valid: false, Reason: "unknown or invalid link"})
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "preview_error", "could not check the link")
		return
	}
	if inv.Status != onboardstore.InviteInvited {
		reason := "this link is no longer open"
		resp := onboardPreviewResponse{Valid: false, Status: inv.Status}
		switch inv.Status {
		case onboardstore.InviteExpired:
			reason = "this link has expired"
		case onboardstore.InviteSubmitted:
			reason = "this link has already been used"
			// The consumed link doubles as the candidate's status read: resolve
			// the proposal it produced so the page can show how far the request
			// has moved instead of dead-ending them at "ask for a new invite".
			if inv.ProposalID != nil {
				if p, perr := s.Onboard.GetProposal(r.Context(), *inv.ProposalID); perr == nil {
					resp.Submission = candidateSubmissionStatus(p)
				}
			}
		}
		resp.Reason = reason
		onboardJSON(w, http.StatusOK, resp)
		return
	}

	pb, err := inv.Prebound()
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "invalid_invite", "this invite has invalid access metadata")
		return
	}
	// Prefer the human sponsor label (a participant sponsor's Sponsor is its
	// UUID org anchor — never shown to a candidate).
	sponsor := firstNonEmpty(inv.SponsorName, inv.Sponsor)
	if isServiceSponsorLabel(sponsor) {
		sponsor = "" // suppress the service labels: show a person or nothing
	}
	resp := onboardPreviewResponse{
		Network:                s.previewNetwork(),
		Valid:                  true,
		Env:                    inv.Env,
		Status:                 inv.Status,
		Sponsor:                sponsor,
		Escalated:              isEscalatedTier(pb.Tier),
		TierLocked:             strings.TrimSpace(pb.Tier) != "",
		AffiliationLocked:      strings.TrimSpace(pb.Affiliation) != "",
		SectorLocked:           strings.TrimSpace(pb.Sector) != "",
		MachineQuotaLocked:     pb.MachineQuota > 0,
		OrgLocked:              strings.TrimSpace(pb.Org) != "",
		OrgName:                s.preboundOrgLabel(pb.Org),
		Prebound:               pb,
		SelectableTiers:        []intakeTier{},
		SelectableAffiliations: affiliationKinds,
		SelectableSectors:      selectableSectors(s.Cfg.SectorsNongovEnabled),
	}
	if h, herr := loadIntakeHints(); herr == nil {
		// The tier menu is bounded by the minting sponsor's cap (ADR 1071); an
		// operator-minted invite ('' cap) offers every non-escalated tier.
		resp.SelectableTiers = h.selectableTiers(pb.SponsorMaxTier)
	}
	resp.RepoTiers = s.repoGrantingTiers(pb.Tier, resp.SelectableTiers)
	onboardJSON(w, http.StatusOK, resp)
}

// preboundOrgLabel resolves a prebound org id to the label the candidate is
// shown. Falls back to the id itself: the invite named a registered org, so the
// worst case is a person seeing a slug they recognize, never a blank field that
// silently re-opens free-typing. An org that has since been deprecated or
// renamed away still labels as its id rather than failing the whole preview.
func (s *Server) preboundOrgLabel(id string) string {
	id = strings.TrimSpace(id)
	if id == "" {
		return ""
	}
	if s.Orgs != nil {
		if v, err := s.Orgs.GetOrg(id); err == nil && strings.TrimSpace(v.DisplayName) != "" {
			return v.DisplayName
		}
	}
	return id
}

type onboardSubmitRequest struct {
	Token      string          `json:"token"`
	Submission json.RawMessage `json:"submission"`
}

type onboardSubmitResponse struct {
	ProposalID string `json:"proposal_id"`
	Status     string `json:"status"`
	Escalated  bool   `json:"escalated"`
	Complete   bool   `json:"complete"`
	// Warnings are consequences of what was submitted that did not stop it.
	// Today the only one is githubMissingWarning.
	Warnings []string `json:"warnings,omitempty"`
}

// candidateSubmission is what the enrollee sends: their identity, plus any access
// fields the invite left open (or a value for a locked field → a petition).
type candidateSubmission struct {
	Name          string `json:"name"`
	Email         string `json:"email"`
	Handle        string `json:"handle"`
	Organization  string `json:"organization"`
	Title         string `json:"title"`
	Notes         string `json:"notes"`
	PrincipalType string `json:"principal_type"`
	// Nation is the ORGANISATION's controlling nation. The intake form no longer
	// asks a person for it (ADR 1171) — it is an org fact typed by an admin at
	// org registration — but the field stays accepted so an in-flight submission
	// and any non-browser caller keep working.
	Nation string `json:"nation"`
	// Nationality is the PERSON's own citizenship, and is what the form now asks.
	// Distinct from Nation above and never derived from it: a liaison officer or
	// anyone posted to another country's command differs, legitimately.
	Nationality    string `json:"nationality"`
	UnitCode       string `json:"unit_code"`
	UnitCodeScheme string `json:"unit_code_scheme"`
	Tier           string `json:"tier"`
	Affiliation    string `json:"affiliation"`
	// The field is `alliance_standing` (ALLIANCE standing: member = an ally,
	// partner = external to the alliance). The legacy `coalition` key is still
	// accepted; `alliance_standing` wins (ADR 1052 additive-alias method).
	Standing  string `json:"alliance_standing"`
	Coalition string `json:"coalition"`
	Sector    string `json:"sector"`
	// GovBranch was sent by the intake page and had no field here, so the JSON
	// decoder discarded it and affiliation.gov_branch could never be stored on
	// this path. Same class as Standing below: the page sends it, the assembler
	// writes it, and only this struct was missing.
	GovBranch      string `json:"gov_branch"`
	MachineQuota   int    `json:"machine_quota"`
	Site           string `json:"site"`
	SpireFederated bool   `json:"spire_federated"`
	// GitHubUsername is an optional login captured for repo-collaborator grants at
	// onboarding (docs/design/onboarding-github-repo-grants.md). Written to the
	// manifest as identity.github, normalized to a bare login at assembly.
	GitHubUsername string `json:"github_username"`
}

// petition is an enrollee request to change a sponsor-locked field.
type petition struct {
	Field          string `json:"field"`
	SponsorValue   string `json:"sponsor_value"`
	RequestedValue string `json:"requested_value"`
}

var errEnrolleeEscalation = errors.New("escalated roles must be offered by your sponsor")
var errTierExceedsSponsorship = errors.New("the requested role exceeds what your sponsor may authorize")

// validationErrorsForSubmission collects per-field validation failures for a
// candidate submission against one invite's pre-bound set. It runs BEFORE the
// one-time invite is consumed so the candidate can reopen the same link and
// fix everything in one pass. The keys match intake field names; messages are
// the same ones the single-field errors used to carry, and they surface in
// the 422 `validation_failed` `error.fields` payload. Valid GitHub logins are
// normalized in place so the value passed to manifest assembly is canonical.
func validationErrorsForSubmission(inv *onboardstore.Invite, sub *candidateSubmission) (map[string]string, error) {
	fields := map[string]string{}

	// Name + email: required for any submission.
	if strings.TrimSpace(sub.Name) == "" || strings.TrimSpace(sub.Email) == "" {
		msg := "please provide at least your name and email"
		if strings.TrimSpace(sub.Name) == "" {
			fields["name"] = msg
		}
		if strings.TrimSpace(sub.Email) == "" {
			fields["email"] = msg
		}
	}

	pb, err := inv.Prebound()
	if err != nil {
		return nil, err
	}

	// Organisation is required when the invite did not pin a registered org,
	// and it must be able to form an affiliation id (at least one letter or
	// digit), or the manifest would fail the approve-time lint after the
	// invite is spent.
	if strings.TrimSpace(pb.Org) == "" {
		switch org := strings.TrimSpace(sub.Organization); {
		case org == "":
			fields["organization"] = "organization is required"
		case affiliationIDFromOrg(org) == "":
			fields["organization"] = "organization must contain at least one letter or digit"
		}
	}

	// Tier: if the sponsor left it open, the candidate must choose one.
	if strings.TrimSpace(pb.Tier) == "" && strings.TrimSpace(sub.Tier) == "" {
		fields["tier"] = errTierRequired.Error()
	}

	// Handle: derive from email when absent; if that still fails, the address
	// cannot form an account name. A blank email already carries its own
	// error, and the form has no handle input, so the handle is named only
	// when there is an address to blame.
	if strings.TrimSpace(sub.Email) != "" {
		handle := strings.TrimSpace(sub.Handle)
		if handle == "" {
			handle = deriveHandle(sub.Email)
		}
		if handle == "" {
			fields["handle"] = errHandleRequired.Error()
		}
	}

	// Organisation nation (legacy field; the form no longer renders it).
	if n := strings.TrimSpace(sub.Nation); n != "" && !validNationCode(n) {
		fields["nation"] = "nation must be an ISO 3166-1 alpha-3 code (e.g. CAN, USA) or a multinational marker (NATO, EU), not a full country name — got " + strconv.Quote(n)
	}

	// Person nationality: required for a person, optional for a system; both
	// sides share the same nation-code rule. Lowercase codes are refused so a
	// submission with a lowercase nationality joins other invalid fields in the
	// multi-field 422 response.
	isSystem := strings.EqualFold(strings.TrimSpace(sub.PrincipalType), "system")
	nat := strings.TrimSpace(sub.Nationality)
	if nat != "" {
		if !validNationCode(nat) {
			fields["nationality"] = "nationality must be an ISO 3166-1 alpha-3 code (e.g. CAN, USA), not a full country name — got " + strconv.Quote(nat)
		}
	} else if !isSystem {
		fields["nationality"] = "please provide your nationality as an ISO 3166-1 alpha-3 code (e.g. CAN, USA) — it decides which shared data you may be shown, so a request without it cannot be reviewed"
	}

	github, githubErr := normalizeGitHubLogin(sub.GitHubUsername)
	if githubErr != nil {
		fields["github_username"] = githubErr.Message
	} else {
		sub.GitHubUsername = github
	}

	if len(fields) == 0 {
		return nil, nil
	}
	return fields, nil
}

// handleOnboardSubmit consumes an invite with the enrollee's submission and
// creates a proposal. Effective access = sponsor-locked field ⊕ enrollee-filled
// open field; an enrollee-supplied escalated tier is refused. A complete effective
// set assembles a valid, approvable manifest; an incomplete one stores a draft.
func (s *Server) handleOnboardSubmit(w http.ResponseWriter, r *http.Request) {
	if s.Onboard == nil {
		onboardError(w, http.StatusServiceUnavailable, "not_configured", "onboarding is not configured")
		return
	}
	var req onboardSubmitRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_request", "body must be JSON: {token, submission}")
		return
	}
	token := strings.TrimSpace(req.Token)
	if token == "" {
		onboardError(w, http.StatusBadRequest, "no_token", "token is required")
		return
	}
	inv, err := s.Onboard.GetInviteByToken(r.Context(), token)
	switch {
	case errors.Is(err, onboardstore.ErrNotFound):
		onboardError(w, http.StatusNotFound, "invalid_link", "this link is invalid")
		return
	case err != nil:
		onboardError(w, http.StatusInternalServerError, "submit_error", "could not submit your details")
		return
	}
	switch inv.Status {
	case onboardstore.InviteInvited:
		// open — proceed
	case onboardstore.InviteExpired:
		onboardError(w, http.StatusGone, "expired_link", "this link has expired")
		return
	default:
		onboardError(w, http.StatusConflict, "used_link", "this link has already been used")
		return
	}

	sub, derr := decodeSubmittedAnswers(req.Submission)
	var typeErr *json.UnmarshalTypeError
	if errors.As(derr, &typeErr) {
		onboardError(w, http.StatusBadRequest, "bad_field_type", wrongTypeMessage(typeErr))
		return
	}
	if derr != nil {
		onboardError(w, http.StatusBadRequest, "bad_submission", "submission must be a JSON object")
		return
	}
	fields, perr := validationErrorsForSubmission(inv, &sub)
	if perr != nil {
		onboardError(w, http.StatusInternalServerError, "invalid_invite", "this invite has invalid access metadata")
		return
	}
	if len(fields) > 0 {
		onboardValidationError(w, fields)
		return
	}

	// Duplicate-email guard, checked BEFORE consuming the one-time invite so a
	// genuine mistake (double-submit, re-onboard of someone already in-flight) does
	// not burn the link. An in-progress or already-onboarded email is refused; a
	// prior rejected/failed attempt does not block a retry. The runner has its own
	// dedup backstop for the concurrent-submit race (it copies the original's key
	// rather than mint twice).
	// The refusal names the blocking row (id + status) in the operator-facing
	// detail — without it, finding the one row the guard read means SSH +
	// sqlite3 against onboard.db on the management host (#334). A store error
	// skips the guard; the runner's dedup backstops the duplicate.
	if p, found, derr := s.Onboard.ActiveProposalForEmail(r.Context(), sub.Email); derr == nil && found {
		onboardErrorDetail(w, http.StatusConflict, "duplicate_email",
			"an onboarding for this email is already in progress or complete — contact your operator if that's not you",
			"blocked by proposal "+p.ID+" (status: "+p.Status+")")
		return
	}

	hints, herr := loadIntakeHints()
	if herr != nil {
		onboardError(w, http.StatusInternalServerError, "hints_error", "onboarding config unavailable")
		return
	}

	build := func(inv *onboardstore.Invite, pid string) (*onboardstore.Proposal, error) {
		// The fold of sponsor-prebound + candidate-filled into effective inputs
		// lives in intakeEffective so draft completion re-runs exactly this.
		in, petitions, ferr := s.intakeEffective(hints, inv, sub)
		if ferr != nil {
			return nil, ferr
		}
		petJSON, err := json.Marshal(petitions)
		if err != nil {
			return nil, fmt.Errorf("encode access petitions: %w", err)
		}
		manifest, aerr := assembleManifest(hints, in)
		p := &onboardstore.Proposal{
			Handle:    in.Handle,
			Email:     strings.TrimSpace(sub.Email),
			Escalated: isEscalatedTier(in.Tier),
			Petitions: string(petJSON),
			Note:      "via invite " + inv.ID,
			Tier:      strings.ToLower(strings.TrimSpace(in.Tier)),
			// Where affiliation.id came from, recorded at the one point that
			// knows: in.AffiliationID is the invite's pinned org, and it is
			// empty exactly when the id will be derived from what the candidate
			// typed. A completed intake stores the assembled manifest, which
			// keeps no trace of either, and the manifest schema fixes
			// affiliation's key set (additionalProperties: false), so the
			// provenance is kept on the proposal rather than in the document.
			OrgSource: orgSourceFor(in),
		}
		var incomplete *incompleteManifestError
		var unreg string
		if aerr == nil {
			unreg = s.unregisteredOrgAtIntake(in)
		}
		switch {
		case aerr == nil && unreg == "":
			b, err := json.Marshal(manifest)
			_ = err // manifest is built from validated inputs; JSON encode cannot fail here in practice.
			p.ManifestJSON = string(b)
			p.Complete = true
		case aerr == nil || errors.As(aerr, &incomplete):
			// Minimal-burden draft — store the v-draft record for admin completion.
			// Two reasons land here: the effective inputs were incomplete, or
			// they were complete but name an organization the registry does
			// not know (aerr == nil, unreg != ""). The candidate is told
			// nothing either way; the admin registers or attaches the org
			// from the review page, which completes the draft.
			rec, h, e, rerr := onboardstore.AssembleIntakeRecord(inv, req.Submission)
			if rerr != nil {
				return nil, rerr
			}
			p.ManifestJSON = string(rec)
			p.Complete = false
			reason := "unregistered organization " + strconv.Quote(unreg)
			if aerr != nil {
				reason = incomplete.Error()
			}
			p.Note = "via invite " + inv.ID + " — INCOMPLETE (" + reason + "); admin to complete"
			if p.Handle == "" {
				p.Handle = h
			}
			if p.Email == "" {
				p.Email = e
			}
		default:
			return nil, aerr
		}
		if sub.SpireFederated {
			p.Note += " — SPIRE/SPIFFE federation requested (admin follow-up: moon-pod trust domain)"
		}
		return p, nil
	}

	p, err := s.Onboard.ConsumeInvite(r.Context(), token, build)
	switch {
	case err == nil:
		// created — audit the submission so /audit carries the candidate's own
		// event, not just the sponsor's later decision (the actor is the
		// candidate themselves, authenticated only by holding the invite token).
		tokPrefix := token
		if len(tokPrefix) > 8 {
			tokPrefix = tokPrefix[:8]
		}
		s.emitAuditWithInviteToken(r, Actor{Email: p.Email}, "onboard_submit",
			tokPrefix, "", nil,
			map[string]interface{}{"proposal_id": p.ID, "handle": p.Handle, "env": p.Env, "escalated": p.Escalated},
			false, "ok", "")
		// The sponsor is now the waiting human — tell them. Best-effort: the
		// proposal is already stored, so a mail failure never fails the submit.
		s.notifySponsorSubmitted(r, p)
	case errors.Is(err, errEnrolleeEscalation):
		onboardError(w, http.StatusForbidden, "escalation_refused", errEnrolleeEscalation.Error())
		return
	case errors.Is(err, errTierExceedsSponsorship):
		onboardError(w, http.StatusForbidden, "tier_exceeds_sponsorship", errTierExceedsSponsorship.Error())
		return
	case errors.Is(err, errTierRequired):
		onboardValidationError(w, map[string]string{"tier": errTierRequired.Error()})
		return
	case errors.Is(err, errHandleRequired):
		onboardValidationError(w, map[string]string{"handle": errHandleRequired.Error()})
		return
	case errors.Is(err, onboardstore.ErrNotFound):
		onboardError(w, http.StatusNotFound, "invalid_link", "this link is invalid")
		return
	case errors.Is(err, onboardstore.ErrInviteExpired):
		onboardError(w, http.StatusGone, "expired_link", "this link has expired")
		return
	case errors.Is(err, onboardstore.ErrInviteConsumed):
		onboardError(w, http.StatusConflict, "used_link", "this link has already been used")
		return
	default:
		onboardError(w, http.StatusInternalServerError, "submit_error", "could not submit your details")
		return
	}
	resp := onboardSubmitResponse{
		ProposalID: p.ID, Status: onboardstore.InviteSubmitted, Escalated: p.Escalated, Complete: p.Complete,
	}
	if strings.TrimSpace(sub.GitHubUsername) == "" && len(s.repoGrantingTiers(p.Tier, nil)) > 0 {
		// The submit still succeeds: a GitHub username is optional. The page
		// warned before sending; this is the server's own record of the
		// choice, so the warning does not depend on the browser having shown it.
		resp.Warnings = append(resp.Warnings, githubMissingWarning)
		s.emitAudit(r, Actor{Email: p.Email}, "onboard_submit_without_github", nil, "", nil,
			map[string]interface{}{"proposal_id": p.ID, "tier": p.Tier}, false, "ok", "")
	}
	onboardJSON(w, http.StatusCreated, resp)
}

// unregisteredOrgAtIntake decides, at submit time, whether a complete set of
// effective inputs names an organization the org registry does not know.
// It returns the organization as the candidate typed it (falling back to the
// derived org id when nothing was typed, i.e. a prebound org that has since
// left the registry) so the draft note can name it, or "" when the proposal
// should complete as before.
//
// The check is gated on orgMatchActive: parking a draft only makes sense
// when the admin's Attach / Register panel — the one path that completes it
// — renders, and that panel sits behind the same predicate. It reads the
// ORG-derived id (orgIDForIntake), the same id the assembler writes; a
// submission with no organization never reaches it, because intake refuses
// that with a 422 before the invite is consumed (#257). Only
// orgstore.ErrNotFound parks the draft; any other store error falls through
// to the complete proposal, never a refusal or a 500 on the candidate's
// submit. That is deliberate and one-sided: this hint and the review marker
// (orgUnregistered) fail open, the approve gate (requireRegisteredOrg) fails
// closed. Do not align one with the other.
func (s *Server) unregisteredOrgAtIntake(in effectiveInputs) string {
	if !s.orgMatchActive() {
		return ""
	}
	id := orgIDForIntake(in)
	if id == "" {
		return ""
	}
	_, err := s.Orgs.Resolve(id)
	if !errors.Is(err, orgstore.ErrNotFound) {
		return ""
	}
	return firstNonEmpty(strings.TrimSpace(in.Organization), id)
}
