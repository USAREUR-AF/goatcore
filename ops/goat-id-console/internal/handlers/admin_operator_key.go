package handlers

// Admin rules on the operator-key (machine) path (GHSA-7788).
//
// A per-person operator key never grants or revokes an admin-bearing group,
// never acts on an identity that holds one, and never advances an onboarding
// proposal that would mint one (proposalApprovalFence7788), whoever its owner
// resolves to: those changes need the browser console with step-up, or a service key
// (ADR 1071, isServiceOperatorKeyHandle7788). Owner resolution is not used
// for any decision here, because the onboarding rows it would read can be
// filed by the key holder. Per-person keys keep every non-admin action
// (non-admin group changes, offboarding a non-admin member).
//
// The fenced machine routes parse their body exactly once, strictly
// (decodeMachineBody7788: no unknown fields, no trailing data), and hand the
// handler a canonical re-encoding of the value the fence checked, so the
// fence and the handler cannot read different requests. Step-up does not
// apply on this path: there is no browser session to step up.

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"os"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

const perPersonAdminRefusal = "refused: a per-person operator key may not change admin groups or act on an identity holding one; use the console with step-up, or a service key"

// perPersonOperatorKey7788 reports whether a machine request is made with a
// per-person operator key (anything that is not a service key).
func perPersonOperatorKey7788(r *http.Request) bool {
	pr := onboardPrincipalFromCtx(r.Context())
	return !pr.IsOperator || !isServiceOperatorKeyHandle7788(pr.Handle)
}

// targetMetadataGroups7788 returns a target identity's admin-managed
// metadata_public.groups; known is false when the identity has none (not yet
// migrated off self-editable traits, F-572 / ADR 1188) or they are not a
// list of strings. Traits are never read here.
func targetMetadataGroups7788(t *kratos.Identity) (groups []string, known bool) {
	if t == nil {
		return nil, false
	}
	raw, present := t.MetadataPublic["groups"]
	if !present {
		return nil, false
	}
	arr, ok := raw.([]interface{})
	if !ok {
		return nil, false
	}
	for _, g := range arr {
		s, ok := g.(string)
		if !ok {
			return nil, false
		}
		groups = append(groups, s)
	}
	return groups, true
}

// targetAdminRank7788 is adminBearingRank of a target's metadata groups; an
// unmigrated identity ranks as the platform tier (unknown: only a
// PLATFORM-ADMIN acts on it).
func (s *Server) targetAdminRank7788(t *kratos.Identity) int {
	groups, known := targetMetadataGroups7788(t)
	if !known {
		return 2
	}
	return s.adminBearingRank(groups)
}

// targetAdminBearing7788 reports whether a target holds an admin-bearing
// group by its metadata groups; an unmigrated identity counts as one.
func (s *Server) targetAdminBearing7788(t *kratos.Identity) bool {
	groups, known := targetMetadataGroups7788(t)
	return !known || s.isAdminBearingGroupList(groups)
}

// adminChangeAboveTierRank7788 is adminChangeAboveTier with the identity's
// current rank already resolved (targetAdminRank7788).
func (s *Server) adminChangeAboveTierRank7788(actor Actor, beforeRank int, after []string) bool {
	own := s.adminBearingRank(actor.Groups)
	return beforeRank > own || s.adminBearingRank(after) > own
}

// decodeMachineBody7788 decodes one JSON value into v, refusing unknown
// fields and anything after the value.
func decodeMachineBody7788(raw []byte, v interface{}) error {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		return err
	}
	if err := dec.Decode(&struct{}{}); !errors.Is(err, io.EOF) {
		return errors.New("trailing data after the JSON body")
	}
	return nil
}

// machineTargetBody7788 is the fenced route's request; a route accepts only
// its own fields (machineTargetFields7788).
type machineTargetBody7788 struct {
	Email string `json:"email,omitempty"`
	ID    string `json:"id,omitempty"`
	State string `json:"state,omitempty"`
}

var machineTargetFields7788 = map[string][]string{
	"POST /api/users/state": {"email", "id", "state"},
}

// machineAdminTargetFence7788 fronts the machine route that acts on one
// tenant identity named in the body (POST /api/users/state): a per-person
// key may not act on an identity holding an admin-bearing group.
// reinstatement-check is not fenced: it only audits a request and changes
// nothing; the state change it precedes goes through this fence.
func (s *Server) machineAdminTargetFence7788(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		raw, err := io.ReadAll(io.LimitReader(r.Body, 1<<20))
		if err != nil {
			onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body")
			return
		}
		var allowed map[string]bool
		if f, ok := machineTargetFields7788[r.Pattern]; ok {
			allowed = map[string]bool{}
			for _, k := range f {
				allowed[k] = true
			}
		}
		var probe map[string]json.RawMessage
		var req machineTargetBody7788
		if err := decodeMachineBody7788(raw, &probe); err != nil {
			onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body: "+err.Error())
			return
		}
		for k := range probe {
			if allowed != nil && !allowed[k] {
				onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body: unknown field "+k)
				return
			}
		}
		if err := decodeMachineBody7788(raw, &req); err != nil {
			onboardError(w, http.StatusBadRequest, "bad_request", "invalid JSON body: "+err.Error())
			return
		}
		if onboardPrincipalFromCtx(r.Context()).IsOperator && perPersonOperatorKey7788(r) {
			if _, target, gErr := s.resolveTenantIdentity(r.Context(), req.ID, req.Email); gErr == nil && s.targetAdminBearing7788(target) {
				onboardError(w, http.StatusForbidden, "forbidden", perPersonAdminRefusal)
				return
			}
		} else if onboardPrincipalFromCtx(r.Context()).IsOperator && strings.TrimSpace(req.State) == "active" {
			// ADR 1071 as amended: a service key may not return a
			// platform-tier login to service. Deactivation (offboarding)
			// is unchanged. A target that cannot be read is refused.
			_, target, gErr := s.resolveTenantIdentity(r.Context(), req.ID, req.Email)
			if gErr != nil || (s.targetAdminRank7788(target) >= 2 && target.State != "active") {
				s.emitAudit(r, Actor{Email: onboardPrincipalFromCtx(r.Context()).Handle}, "patch_state", target, tenantIdPKey, nil, nil, false, "refused", operatorKeyPlatformRefusal7788)
				onboardError(w, http.StatusForbidden, "forbidden", operatorKeyPlatformRefusal7788)
				return
			}
		}
		canon, _ := json.Marshal(req) //nolint:errcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		r.Body = io.NopCloser(bytes.NewReader(canon))
		r.ContentLength = int64(len(canon))
		next(w, r)
	}
}

// isServiceOperatorKeyHandle7788 reports whether an operator key's handle names a service rather
// than a person: "operator", "onboard-api", "id-console", the
// onboarding runner (onboard-runner, onboard-runner-<site>), or a handle
// listed in ONBOARD_SERVICE_OPERATOR_HANDLES (comma-separated). A service
// key is uncapped (ADR 1071). The exact rule of GHSA-mxf4's
// isServiceOperatorKeyHandle (trimmed, exact match, no case folding; an
// empty handle is not a service), copied
// under its own name so this patch applies alone and stacks.
func isServiceOperatorKeyHandle7788(handle string) bool {
	h := strings.TrimSpace(handle)
	switch h {
	case "":
		// An empty handle is an unset principal, never a service.
		return false
	case "operator", "onboard-api", "id-console", "onboard-runner":
		return true
	}
	if strings.HasPrefix(h, "onboard-runner-") {
		return true
	}
	for _, v := range strings.Split(os.Getenv("ONBOARD_SERVICE_OPERATOR_HANDLES"), ",") {
		if v = strings.TrimSpace(v); v != "" && v == h {
			return true
		}
	}
	return false
}

// operatorKeyPlatformRefusal7788 is the ADR 1071 amendment's refusal: the
// platform tier is invited, vouched for, approved, queued (restore, retry),
// reactivated and granted only from a browser PLATFORM-ADMIN session with
// step-up. No operator key does any of these: not a service key, not a
// per-person key. The runner (a service key) still claims a platform-tier
// proposal and posts its result, and a service key still deactivates and
// removes the platform tier and registers an already-minted login.
const operatorKeyPlatformRefusal7788 = "refused: the platform-admin tier is invited, approved, re-queued, reactivated and granted only in the console by a PLATFORM-ADMIN with step-up; no operator key (service or per-person) may do this"

// platformTierInvite7788 reports whether an invite pre-bound to tier would
// grant the platform tier (PLATFORM-ADMIN or ADMINS in the tier preset's
// groups). A tier the table cannot place, or an unreadable table, counts as
// the platform tier.
func platformTierInvite7788(tier string) bool {
	tier = strings.TrimSpace(tier)
	if tier == "" {
		return false
	}
	h, err := loadIntakeHints()
	if err != nil {
		return true
	}
	for _, t := range h.Tiers {
		if t.ID == tier {
			return isAdminGroupList(tierOryGroups(t))
		}
	}
	return true
}

const servicePlatformGrantRefusal7788 = "refused: no operator key, service keys included, may grant the platform-admin tier (PLATFORM-ADMIN, ADMINS) through the API; grant it in the console with step-up"

// grantsPlatformTier7788 reports whether after adds a platform-tier group
// (PLATFORM-ADMIN or the legacy ADMINS) that before does not hold.
func grantsPlatformTier7788(before, after []string) bool {
	for _, g := range []string{platformAdminGroup, legacyAdminGroup} {
		if containsString(after, g) && !containsString(before, g) {
			return true
		}
	}
	return false
}

const perPersonApprovalRefusal7788 = "refused: this proposal would create an identity holding an admin group, which a per-person operator key may not approve; approve it in the console, or with a service key"

// proposalApprovalFence7788 fronts the decisions that advance an onboarding
// proposal toward a minted login: sponsor-approve and approve, on the
// machine routes and on the browser decide route. The runner writes the
// manifest's ory_site.groups to the new identity (operator-onboard.yml), so
// approving a proposal that carries an admin-bearing group creates an admin
// login, whatever the proposal's tier slug says.
//   - A per-person operator key may not advance a proposal that would create
//     an admin-bearing identity, at any tier (a service key is uncapped, ADR
//     1071).
//   - A browser admin may not advance one whose groups rank above their own
//     admin tier (adminBearingRank), so a tier-less or low-tier proposal
//     carrying PLATFORM-ADMIN is refused to an IAM-ADMIN. approvalCeiling
//     still applies its tier-slug rules in the handler.
//
// A sponsor session and a participant (non-operator) key are left to the
// handler: they may only vouch, within their sponsorship cap, and the admin
// approve that follows is fenced here. A proposal whose groups cannot be
// read counts as admin-bearing.
func (s *Server) proposalApprovalFence7788(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		action := r.PathValue("action")
		if action == "" {
			action = r.Pattern[strings.LastIndex(r.Pattern, "/")+1:]
		}
		machine := strings.HasPrefix(r.URL.Path, "/api/")
		queues := action == "approve" || action == "sponsor-approve" ||
			(!machine && (action == "retry" || action == "restore"))
		if !queues || s.Onboard == nil {
			next(w, r)
			return
		}
		refuse := func(status int, msg string, actor Actor) {
			s.emitAuditWithInviteToken(r, actor, "onboard_"+action, r.PathValue("id"), "", nil, nil, false, "refused", msg)
			if machine {
				onboardError(w, status, "forbidden", msg)
				return
			}
			s.renderError(w, r, status, msg)
		}
		p, err := s.Onboard.GetProposal(r.Context(), r.PathValue("id"))
		if errors.Is(err, onboardstore.ErrNotFound) {
			next(w, r) // the handler answers not-found in its own shape
			return
		}
		if err != nil {
			refuse(http.StatusServiceUnavailable, "could not load the proposal to check what it grants", actorFromCtx(r.Context()))
			return
		}
		rank := 2
		if groups, ok := s.proposalGrantedGroups7788(p); ok {
			rank = s.adminBearingRank(groups)
		}
		if rank == 0 {
			next(w, r)
			return
		}
		if machine {
			if pr := onboardPrincipalFromCtx(r.Context()); pr.IsOperator && !isServiceOperatorKeyHandle7788(pr.Handle) {
				refuse(http.StatusForbidden, perPersonApprovalRefusal7788, Actor{Email: onboardAPIActor(r)})
				return
			}
			// ADR 1071 as amended: a service key may not vouch for or
			// approve a proposal that would mint the platform tier.
			if rank >= 2 {
				refuse(http.StatusForbidden, operatorKeyPlatformRefusal7788, Actor{Email: onboardAPIActor(r)})
				return
			}
			next(w, r)
			return
		}
		if sponsorFromCtx(r.Context()) != nil {
			next(w, r)
			return
		}
		actor := actorFromCtx(r.Context())
		if rank > s.adminBearingRank(actor.Groups) {
			refuse(http.StatusForbidden, adminTierRefusal, actor)
			return
		}
		// A console decision that creates or re-queues the platform tier
		// (vouch, approve, retry, restore) takes a fresh step-up, as
		// granting the platform tier does (GHSA-7788 review F2).
		if rank >= 2 {
			if err := s.validateCSRF(r); err != nil {
				s.renderError(w, r, 400, err.Error())
				return
			}
			if out := s.requireStepUp(w, r, actor, "decide_platform_tier"); out.Handled {
				s.emitAuditWithInviteToken(r, actor, "onboard_"+action, r.PathValue("id"), "", nil, nil, false, "refused", stepUpChallengeAudit)
				return
			}
		}
		next(w, r)
	}
}

const runnerQueueRefusal7788 = "refused: this proposal would mint an identity holding an admin group above what this credential may create; a per-person operator key may not queue one, and a console admin only up to their own admin tier; use the console at that tier, or a service key"

// installRunnerQueueCheck7788 makes the store run runnerQueueCheck7788 on
// every write that moves a proposal into `approved`, the status the
// onboard-runner mints from. The decide fence above answers the approve
// verbs early; this covers every other way into the queue (restore, retry,
// status writes, create), in both realms, at the state write itself.
func (s *Server) installRunnerQueueCheck7788() {
	if s.Onboard != nil {
		s.Onboard.SetRunnerQueueCheck7788(s.runnerQueueCheck7788)
		s.Onboard.SetRunnerActionCheck7788(s.runnerActionCheck7788)
	}
}

// runnerActionCheck7788 caps the runner-side lifecycle actions (claim,
// result, archive) at the store write. A per-person operator key, or any
// machine principal that is not a service key, may not run them on a
// proposal that mints or minted an admin-bearing identity: claiming hands
// over the manifest, a result records the mint, and archive is the step
// before a restore. Service keys (the runner, ADR 1071) and console sessions
// are unchanged here; console sessions are capped on their own routes.
func (s *Server) runnerActionCheck7788(ctx context.Context, p *onboardstore.Proposal, action string) error {
	pr, machine := ctx.Value(ctxKeyOnboardPrincipal).(onboardPrincipal)
	if !machine || (pr.IsOperator && isServiceOperatorKeyHandle7788(pr.Handle)) {
		return nil
	}
	rank := 2
	if groups, ok := s.proposalGrantedGroups7788(p); ok {
		rank = s.adminBearingRank(groups)
	}
	// The row may be filed at a lower tier than the person holds: rank the
	// person's login as well (GHSA-7788 review F3).
	if rank == 0 && !s.personLoginAdminBearing7788(ctx, p) {
		return nil
	}
	verb := map[string]string{
		onboardstore.ActionClaim7788:    "claim",
		onboardstore.ActionResult7788:   "record a result or status for",
		onboardstore.ActionArchive7788:  "archive",
		onboardstore.ActionOffboard7788: "mark offboarded",
	}[action]
	if verb == "" {
		verb = "act on"
	}
	return &onboardstore.RunnerQueueRefusal{Msg: "refused: a per-person operator key may not " + verb +
		" a proposal for an identity holding an admin group; use the console, or a service key"}
}

// runnerQueueCheck7788 decides whether p may enter the mint queue for the
// request in ctx. Proposals that mint no admin-bearing group always may.
// Otherwise: a machine principal must be a service key (ADR 1071); a
// sponsor session never may; a browser admin may only up to their own admin
// tier. A context carrying no request principal at all (a background sweep)
// is not a request and is not refused.
func (s *Server) runnerQueueCheck7788(ctx context.Context, p *onboardstore.Proposal) error {
	rank := 2
	if groups, ok := s.proposalGrantedGroups7788(p); ok {
		rank = s.adminBearingRank(groups)
	}
	if rank == 0 {
		return nil
	}
	if pr, machine := ctx.Value(ctxKeyOnboardPrincipal).(onboardPrincipal); machine {
		if pr.IsOperator && isServiceOperatorKeyHandle7788(pr.Handle) {
			// ADR 1071 as amended: a service key may not put a
			// platform-tier mint into the queue (approve, restore, retry);
			// it may still record one already minted (register).
			if rank >= 2 && !onboardstore.IsRegisterContext7788(ctx) {
				return &onboardstore.RunnerQueueRefusal{Msg: operatorKeyPlatformRefusal7788}
			}
			return nil
		}
		return &onboardstore.RunnerQueueRefusal{Msg: runnerQueueRefusal7788}
	}
	if sponsorFromCtx(ctx) != nil {
		return &onboardstore.RunnerQueueRefusal{Msg: runnerQueueRefusal7788}
	}
	actor := actorFromCtx(ctx)
	if actor.Email == "" && actor.IdentityID == "" && len(actor.Groups) == 0 {
		return nil
	}
	if rank > s.adminBearingRank(actor.Groups) {
		return &onboardstore.RunnerQueueRefusal{Msg: runnerQueueRefusal7788}
	}
	return nil
}

// runnerResultFence7788 answers the result route's lifecycle refusal with a
// 403 before the handler runs.
func (s *Server) runnerResultFence7788(next http.HandlerFunc) http.HandlerFunc {
	return s.runnerActionFence7788(onboardstore.ActionResult7788, "onboard_proposal_result", next)
}

// runnerActionFence7788 answers a machine route's lifecycle refusal (result,
// mark-onboarded, bookkeeping offboard) with a 403 before the handler runs.
// The store write (SetProposalResult, SetProposalStatus, MarkOffboardedFrom)
// runs the same check again, so the fence only shapes the answer; it is not
// the cap.
func (s *Server) runnerActionFence7788(action, audit string, next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if s.Onboard != nil {
			if p, err := s.Onboard.GetProposal(r.Context(), r.PathValue("id")); err == nil {
				if err := s.runnerActionCheck7788(r.Context(), p, action); err != nil {
					s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, audit, nil, "", nil, nil, false, "refused", err.Error())
					onboardError(w, http.StatusForbidden, "forbidden", err.Error())
					return
				}
			}
		}
		next(w, r)
	}
}

// runnerFollowUpFence7788 answers the follow-up action routes' refusal
// (claim, result) with a 403 before the handler runs, ranked on the
// action's proposal. The store writes (ClaimAction, SetActionResult,
// SetReleaseActionResult, the reissue finish) run the same check again.
func (s *Server) runnerFollowUpFence7788(action, audit string, next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if s.Onboard != nil {
			if a, err := s.Onboard.GetAction(r.Context(), r.PathValue("id")); err == nil && a.ProposalID != "" {
				if p, err := s.Onboard.GetProposal(r.Context(), a.ProposalID); err == nil {
					if err := s.runnerActionCheck7788(r.Context(), p, action); err != nil {
						s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, audit, nil, "", nil, nil, false, "refused", err.Error())
						onboardError(w, http.StatusForbidden, "forbidden", err.Error())
						return
					}
				}
			}
		}
		next(w, r)
	}
}

// personLoginAdminBearing7788 reports whether the person a proposal is for
// holds, by their tenant login's admin-managed groups, an admin-bearing
// group. A login with no metadata groups (unmigrated) counts as one, and so
// does any doubt: no tenant realm, or an identity list that cannot be read.
// A proposal with no email names no login.
func (s *Server) personLoginAdminBearing7788(ctx context.Context, p *onboardstore.Proposal) bool {
	email := strings.TrimSpace(p.Email)
	if email == "" {
		return false
	}
	cli := s.Kratos[tenantIdPKey]
	if cli == nil {
		return true
	}
	ids, err := cli.ListIdentities(ctx)
	if err != nil {
		return true
	}
	for i := range ids {
		if strings.EqualFold(strings.TrimSpace(ids[i].Email()), email) && s.targetAdminBearing7788(&ids[i]) {
			return true
		}
	}
	return false
}

// asRunnerQueueRefusal7788 reports whether err is a runner-queue refusal.
func asRunnerQueueRefusal7788(err error) (*onboardstore.RunnerQueueRefusal, bool) {
	var rq *onboardstore.RunnerQueueRefusal
	return rq, errors.As(err, &rq)
}

// proposalGrantedGroups7788 returns the groups approving p would put on the
// minted login: its tier preset's ory_site.groups and its manifest's
// ory_site.groups. An incomplete draft is also assembled, as the advancing
// arms would assemble it, without writing anything. ok is false when any of
// that cannot be read.
func (s *Server) proposalGrantedGroups7788(p *onboardstore.Proposal) (groups []string, ok bool) {
	hints, herr := loadIntakeHints()
	if tier := strings.TrimSpace(p.Tier); tier != "" {
		if herr != nil {
			return nil, false
		}
		for _, t := range hints.Tiers {
			if t.ID == tier {
				groups = append(groups, tierOryGroups(t)...)
			}
		}
	}
	raw := strings.TrimSpace(p.ManifestJSON)
	if raw == "" {
		return groups, true
	}
	var m map[string]any
	if err := json.Unmarshal([]byte(raw), &m); err != nil {
		return nil, false
	}
	g, ok := manifestOryGroups7788(m)
	if !ok {
		return nil, false
	}
	groups = append(groups, g...)
	if !p.Complete {
		if herr != nil {
			return nil, false
		}
		a, err := s.assembleDraft(p, func() (*intakeHints, error) { return hints, nil })
		if err != nil {
			return nil, false
		}
		if a.outcome == draftCompleted {
			g, ok := manifestOryGroups7788(a.manifest)
			if !ok {
				return nil, false
			}
			groups = append(groups, g...)
		}
	}
	return groups, true
}

// manifestOryGroups7788 reads ory_site.groups from a manifest. Absent is no
// groups; present but not a list of strings is unreadable (ok false).
func manifestOryGroups7788(m map[string]any) ([]string, bool) {
	switch v := getPath(m, "ory_site.groups").(type) {
	case nil:
		return nil, true
	case []string:
		return v, true
	case []any:
		out := make([]string, 0, len(v))
		for _, g := range v {
			gs, ok := g.(string)
			if !ok {
				return nil, false
			}
			out = append(out, strings.TrimSpace(gs))
		}
		return out, true
	default:
		return nil, false
	}
}

const reactivateNoIdentityRefusal7788 = "refused: your sign-in carries no identity id, so a platform-tier login cannot be reactivated from it (step-up needs one); sign in again"

const selfReactivationRefusal7788 = "refused: you cannot reactivate your own platform-tier login; another platform admin must"

// platformInviteStepUp7788 asks an admin session for a fresh step-up before
// a platform-tier onboarding invite, whatever the step-up policy lists
// (round 2 F2): an invite at that tier is the first step to a platform-tier
// login. Sponsor sessions keep their sponsorship cap in the handler.
func (s *Server) platformInviteStepUp7788(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if sponsorFromCtx(r.Context()) != nil || !platformTierInvite7788(r.FormValue("tier")) {
			next(w, r)
			return
		}
		if err := s.validateCSRF(r); err != nil {
			s.renderError(w, r, 400, err.Error())
			return
		}
		actor := actorFromCtx(r.Context())
		if out := s.requireStepUp(w, r, actor, "invite_platform_tier"); out.Handled {
			s.emitAuditWithInviteToken(r, actor, "onboard_mint_invite", "", "", nil,
				map[string]interface{}{"tier": strings.TrimSpace(r.FormValue("tier"))}, false, "refused", stepUpChallengeAudit)
			return
		}
		next(w, r)
	}
}
