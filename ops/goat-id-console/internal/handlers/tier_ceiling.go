package handlers

// Admin tier ceiling (GHSA-vr6w). One rule, enforced wherever a browser
// admin can produce or reach a login at a tier: an actor may not grant,
// revoke, recover, reset the second factor of, or create an identity at a
// tier above their own, and may not add groups to their own identity.
//
// The ceiling covers both the consumers of an identity's groups (recovery
// link, 2FA reset) and their producers (group change, create, invite). A
// ceiling on the consumers alone is hollow: the actor would lower the
// target's tier, take the login, and raise the tier again.
//
// Self is decided by the identity id the session carries
// (X-Forwarded-User, the OIDC sub), never by email: an email is not unique
// across realms and is not a stable identity. An actor whose session carries
// no identity id is never treated as the target, and is treated as possibly
// the target wherever that refuses more.

import (
	"encoding/json"
	"net/http"
	"os"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// perPersonOperatorKeyVr6w reports whether a machine request is made with a
// per-person operator key (anything that is not a service key). A
// per-person key never grants or revokes the platform tier and never acts on
// an identity holding it, whoever it is filed for: the onboarding rows that
// would name its owner can be written by its holder, so no rank is derived
// from them. Those actions need the console with step-up, or a service key
// (ADR 1071). Only a console admin is eligible for a per-person key
// (operatorKeyEligible), so its holder is at least IAM-ADMIN and this is the
// tier ceiling for it. GHSA-7788 widens the rule to every admin-bearing
// group (IAM-ADMIN included); in this patch alone IAM-ADMIN changes by a
// per-person key keep their existing behaviour.
func perPersonOperatorKeyVr6w(r *http.Request) bool {
	pr := onboardPrincipalFromCtx(r.Context())
	return !pr.IsOperator || !isServiceOperatorKeyHandleVr6w(pr.Handle)
}

const perPersonTierRefusal = "refused: a per-person operator key may not change the platform-admin tier or act on an identity holding it; use the console with step-up, or a service key"

// platformTier is adminTierRank's top rank (PLATFORM-ADMIN or ADMINS).
const platformTier = 2

// adminTierRank orders identities by the admin access their groups give:
// platform-admin (either name) above IAM-ADMIN above everyone else.
func adminTierRank(groups []string) int {
	switch {
	case containsString(groups, "PLATFORM-ADMIN") || containsString(groups, "ADMINS"):
		return 2
	case containsString(groups, "IAM-ADMIN"):
		return 1
	}
	return 0
}

// targetTierRankVr6w is the admin tier of a target identity, read only from
// its admin-managed metadata_public.groups. An identity whose metadata
// carries no groups list (not yet migrated off self-editable traits, F-572
// / ADR 1188) ranks unknown, which counts as the platform tier: only a
// PLATFORM-ADMIN acts on it until an admin sets its groups. Traits are never
// read: a value the person can edit must not raise or lower their
// protection.
func targetTierRankVr6w(target *kratos.Identity) int {
	if target == nil {
		return platformTier
	}
	groups, ok := metadataGroupsVr6w(target)
	if !ok {
		return platformTier
	}
	return adminTierRank(groups)
}

// metadataGroupsVr6w returns metadata_public.groups; ok is false when the
// key is absent or is not a list of strings.
func metadataGroupsVr6w(target *kratos.Identity) ([]string, bool) {
	raw, present := target.MetadataPublic["groups"]
	if !present {
		return nil, false
	}
	arr, ok := raw.([]interface{})
	if !ok {
		return nil, false
	}
	out := make([]string, 0, len(arr))
	for _, g := range arr {
		s, ok := g.(string)
		if !ok {
			return nil, false
		}
		out = append(out, s)
	}
	return out, true
}

// actorIsIdentity reports whether the session's own identity id is id. It
// is false when the session carries no identity id.
func actorIsIdentity(actor Actor, id string) bool {
	return actor.IdentityID != "" && strings.EqualFold(actor.IdentityID, strings.TrimSpace(id))
}

// targetOutranksActor reports whether acting on target would let actor reach
// an account above their own tier: a recovery link or a second-factor reset
// on such an account is an account takeover. An actor's own identity, by
// identity id, never outranks them.
func targetOutranksActor(actor Actor, target *kratos.Identity) bool {
	if target == nil || actorIsIdentity(actor, target.ID) {
		return false
	}
	return targetTierRankVr6w(target) > adminTierRank(actor.Groups)
}

// groupsOutrankActor reports whether an identity holding groups would sit
// above actor's tier: creating it, or inviting someone into it, is refused.
func groupsOutrankActor(actor Actor, groups []string) bool {
	return adminTierRank(groups) > adminTierRank(actor.Groups)
}

// groupChangeCeiling returns the refusal for a browser group change on
// identity id (currently holding before) to newGroups, or "" when the
// change is within the actor's tier.
func groupChangeCeiling(actor Actor, id string, before []string, beforeRank int, newGroups []string) string {
	self := actorIsIdentity(actor, id)
	if !self && beforeRank > adminTierRank(actor.Groups) {
		return tierCeilingRefusal
	}
	if adminTierRank(newGroups) > adminTierRank(actor.Groups) {
		return tierGrantRefusal
	}
	// Nobody adds a group to their own identity. Without an identity id the
	// session might be the target, so it may not add an admin tier either.
	if self && addsGroup(before, newGroups) {
		return selfGrantRefusal
	}
	if actor.IdentityID == "" && adminTierRank(newGroups) > beforeRank {
		return selfGrantRefusal
	}
	return ""
}

// addsGroup reports whether after holds a group that before does not.
func addsGroup(before, after []string) bool {
	for _, g := range after {
		if !containsString(before, g) {
			return true
		}
	}
	return false
}

const (
	tierCeilingRefusal = "refused: this identity holds a higher admin tier than yours; an admin at its tier must do this"
	tierGrantRefusal   = "refused: these groups are above your own admin tier; an admin at that tier must grant them"
	selfGrantRefusal   = "refused: you cannot add groups to your own identity; another admin must make this change"
)

// isServiceOperatorKeyHandleVr6w reports whether an operator key's handle names a service rather
// than a person: "operator", "onboard-api", "id-console", the
// onboarding runner (onboard-runner, onboard-runner-<site>), or a handle
// listed in ONBOARD_SERVICE_OPERATOR_HANDLES (comma-separated). A service
// key is uncapped (ADR 1071). The exact rule of GHSA-mxf4's
// isServiceOperatorKeyHandle (trimmed, exact match, no case folding; an
// empty handle is not a service), copied
// under its own name so this patch applies alone and stacks.
func isServiceOperatorKeyHandleVr6w(handle string) bool {
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

// proposalAdminRankVr6w is the admin tier approving p would put on the
// minted identity: through its tier's ory_site.groups, the ory_site.groups
// of its manifest (the runner writes manifest.ory_site.groups to the new
// identity, operator-onboard.yml), or, for an incomplete draft, the manifest
// the advancing arms would assemble from it. Anything that cannot be read
// counts as the platform tier (fail closed).
func (s *Server) proposalAdminRankVr6w(h *intakeHints, p *onboardstore.Proposal) int {
	var groups []string
	if tier := strings.TrimSpace(p.Tier); tier != "" {
		if h == nil {
			return platformTier
		}
		for _, tr := range h.Tiers {
			if tr.ID == tier {
				groups = append(groups, tierOryGroups(tr)...)
			}
		}
	}
	if raw := strings.TrimSpace(p.ManifestJSON); raw != "" {
		var m map[string]any
		if err := json.Unmarshal([]byte(raw), &m); err != nil {
			return platformTier
		}
		g, ok := manifestOryGroupsVr6w(m)
		if !ok {
			return platformTier
		}
		groups = append(groups, g...)
		if !p.Complete {
			if h == nil {
				return platformTier
			}
			a, err := s.assembleDraft(p, func() (*intakeHints, error) { return h, nil })
			if err != nil {
				return platformTier
			}
			if a.outcome == draftCompleted {
				g, ok := manifestOryGroupsVr6w(a.manifest)
				if !ok {
					return platformTier
				}
				groups = append(groups, g...)
			}
		}
	}
	return adminTierRank(groups)
}

// manifestOryGroupsVr6w reads ory_site.groups from a manifest: absent is no
// groups; present but not a list of strings is unreadable (ok false).
func manifestOryGroupsVr6w(m map[string]any) ([]string, bool) {
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
	}
	return nil, false
}

// personTierRankVr6w is the admin tier of the person proposal p is for: the
// higher of what p grants and what their tenant identity (matched by email)
// holds. ids, when non-nil, is a tenant identity list already read; a lookup
// error counts as the platform tier (fail closed).
func (s *Server) personTierRankVr6w(r *http.Request, p *onboardstore.Proposal, ids []kratos.Identity) int {
	hints, _ := loadIntakeHints() //nolint:errcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
	rank := s.proposalAdminRankVr6w(hints, p)
	if rank >= platformTier {
		return rank
	}
	if ids == nil {
		cli := s.Kratos[tenantIdPKey]
		if cli == nil {
			return rank
		}
		var err error
		if ids, err = cli.ListIdentities(r.Context()); err != nil {
			return platformTier
		}
	}
	for i := range ids {
		if strings.EqualFold(strings.TrimSpace(ids[i].Email()), strings.TrimSpace(p.Email)) {
			if g := targetTierRankVr6w(&ids[i]); g > rank {
				rank = g
			}
		}
	}
	return rank
}

// personAboveTierVr6w reports whether the person proposal p is for sits
// above the browser actor's tier, by what p grants or by their tenant
// identity's groups (a lookup failure counts as above). Above tier, the
// welcome package carries no credential, the mesh keys are not reissued and
// the access cut is not queued.
func (s *Server) personAboveTierVr6w(r *http.Request, p *onboardstore.Proposal) bool {
	actor := actorFromCtx(r.Context())
	if actor.Email == "" {
		return false // machine writeback: mailed to the enrollee's own address
	}
	return s.personTierRankVr6w(r, p, nil) > adminTierRank(actor.Groups)
}

const perPersonApprovalRefusal = "refused: this proposal grants an admin tier, which a per-person operator key may not approve; approve it in the console, or with a service key"

// redactAboveTierVr6w blanks the stored mesh enrolment keys and bundle path
// of every row whose person is at the platform tier when a per-person
// operator key reads the queue: those are that person's credentials, and a
// per-person key does not act on the platform tier. A service key (the
// runner) and other principals see the rows unchanged.
func (s *Server) redactAboveTierVr6w(r *http.Request, rows []onboardstore.Proposal) {
	pr := onboardPrincipalFromCtx(r.Context())
	if !pr.IsOperator || !perPersonOperatorKeyVr6w(r) {
		return
	}
	var ids []kratos.Identity
	listed := false
	for i := range rows {
		p := &rows[i]
		if p.SetupKey == "" && p.HandsetKey == "" && p.BundlePath == "" {
			continue
		}
		if !listed {
			listed = true
			if cli := s.Kratos[tenantIdPKey]; cli != nil {
				var err error
				if ids, err = cli.ListIdentities(r.Context()); err != nil {
					ids = nil //nolint:ineffassign // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
					// Fail closed: without the identity list every row with
					// credentials is redacted.
					for j := range rows[i:] {
						rows[i+j].SetupKey, rows[i+j].HandsetKey, rows[i+j].BundlePath = "", "", ""
					}
					return
				}
			}
			if ids == nil {
				ids = []kratos.Identity{}
			}
		}
		if s.personTierRankVr6w(r, p, ids) >= platformTier {
			p.SetupKey, p.HandsetKey, p.BundlePath = "", "", ""
		}
	}
}

const proposalTierRefusalVr6w = "refused: approving this proposal grants an admin tier above yours (by its tier or by its manifest's groups); an admin may approve up to their own tier, not above it"

// grantCeilingVr6w is the admin tier ceiling on every decision that puts a
// proposal into the runner's mint queue (approve, restore, retry), ranked by
// what the mint would write (proposalAdminRankVr6w), not by the tier slug
// alone. A sponsor session is not decided here (it only vouches, within its
// cap). Otherwise only a service operator key (ADR 1071) is uncapped
// (uncapped true); a principal with no groups (a per-person key, or none)
// may not queue any admin tier; a browser admin may not queue one above
// their own tier.
func (s *Server) grantCeilingVr6w(r *http.Request, p *onboardstore.Proposal, hints *intakeHints, entersQueue bool) (derr *decideError, uncapped bool) {
	if sponsorFromCtx(r.Context()) != nil {
		return nil, false
	}
	grant := s.proposalAdminRankVr6w(hints, p)
	actor := actorFromCtx(r.Context())
	if len(actor.Groups) == 0 {
		if pr := onboardPrincipalFromCtx(r.Context()); pr.IsOperator && isServiceOperatorKeyHandleVr6w(pr.Handle) {
			// ADR 1071 as amended: a service key may not put a
			// platform-tier mint into the queue (approve, vouch, restore,
			// retry). The runner keeps claim, result and archive.
			if entersQueue && grant >= platformTier {
				return &decideError{status: http.StatusForbidden, msg: operatorKeyPlatformRefusalVr6w}, false
			}
			return nil, true
		}
		if grant > 0 {
			return &decideError{status: http.StatusForbidden, msg: perPersonApprovalRefusal}, false
		}
		return nil, false
	}
	if grant > adminTierRank(actor.Groups) {
		return &decideError{status: http.StatusForbidden, msg: proposalTierRefusalVr6w}, false
	}
	return nil, false
}

// inviteTierCeilingVr6w returns the refusal for an operator key minting an
// invite pre-bound to tier, or "". The browser invite form refuses groups
// above the actor's own tier (groupsOutrankActor). A per-person key's holder
// is not resolved from any record (those can be written by the holder), so
// its ceiling is the lowest tier eligible for a per-person key, IAM-ADMIN:
// an invite whose tier grants the platform tier is refused. A tier the table
// cannot place counts as the platform tier (fail closed); no tier is a
// candidate's choice, capped again at approval. Participants keep the
// sponsorship cap (ADR 1071) and service keys are uncapped.
func inviteTierCeilingVr6w(r *http.Request, tier string) string {
	tier = strings.TrimSpace(tier)
	pr := onboardPrincipalFromCtx(r.Context())
	if tier == "" || !pr.IsOperator {
		return ""
	}
	if inviteTierRankVr6w(tier) >= platformTier {
		if !perPersonOperatorKeyVr6w(r) {
			// ADR 1071 as amended: service keys too.
			return operatorKeyPlatformRefusalVr6w
		}
		return perPersonInviteRefusalVr6w
	}
	return ""
}

// operatorKeyPlatformRefusalVr6w is the ADR 1071 amendment's refusal: the
// platform tier is invited, vouched for, approved, re-queued, reactivated
// and granted only from a browser PLATFORM-ADMIN session with step-up. No
// operator key does any of these. The runner (a service key) still claims a
// platform-tier proposal, posts its result and archives it, and a service
// key still deactivates and removes the platform tier.
const operatorKeyPlatformRefusalVr6w = "refused: the platform-admin tier is invited, approved, re-queued, reactivated and granted only in the console by a PLATFORM-ADMIN with step-up; no operator key (service or per-person) may do this"

// inviteTierRankVr6w is the admin tier an invite pre-bound to tier would
// grant: adminTierRank of the tier preset's ory_site.groups. A tier the
// table cannot place, or an unreadable table, counts as the platform tier.
func inviteTierRankVr6w(tier string) int {
	h, err := loadIntakeHints()
	if err != nil {
		return platformTier
	}
	for _, t := range h.Tiers {
		if t.ID == strings.TrimSpace(tier) {
			return adminTierRank(tierOryGroups(t))
		}
	}
	return platformTier
}

const perPersonInviteRefusalVr6w = "refused: a per-person operator key may not mint an invite at the platform-admin tier (the console's invite form refuses it above your own tier); mint it in the console as a platform admin, or with a service key"

// queueCeilingVr6w applies grantCeilingVr6w to a lifecycle action on p;
// entersQueue is true for the actions that put the row back into the mint
// queue (restore, retry).
func (s *Server) queueCeilingVr6w(r *http.Request, p *onboardstore.Proposal, entersQueue bool) *decideError {
	hints, _ := loadIntakeHints() //nolint:errcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
	derr, _ := s.grantCeilingVr6w(r, p, hints, entersQueue)
	if derr != nil && derr.msg == perPersonApprovalRefusal {
		derr.msg = perPersonLifecycleRefusalVr6w
	}
	return derr
}

const perPersonLifecycleRefusalVr6w = "refused: this proposal grants an admin tier, and a per-person operator key may not restore, retry, claim, report on or archive it; use the console at that tier, or a service key"

// metadataRankBelowPlatformVr6w reports whether target's admin-managed
// groups do not already hold the platform tier: true for an unmigrated
// identity, whose first metadata groups would be a grant.
func metadataRankBelowPlatformVr6w(target *kratos.Identity) bool {
	groups, ok := metadataGroupsVr6w(target)
	return !ok || adminTierRank(groups) < platformTier
}

// platformTierDecisionStepUpVr6w asks an admin session for a fresh step-up
// before a decision that creates or re-queues the platform tier (vouch,
// approve, retry, restore of a proposal whose grant reaches it), as granting
// the platform tier does. It reports whether it answered the request (a
// step-up challenge). Other actions, sponsor sessions and lower tiers pass.
func (s *Server) platformTierDecisionStepUpVr6w(w http.ResponseWriter, r *http.Request, actor Actor, id, action string) bool {
	switch action {
	case "approve", "sponsor-approve", "retry", "restore":
	default:
		return false
	}
	if sponsorFromCtx(r.Context()) != nil || s.Onboard == nil {
		return false
	}
	p, err := s.Onboard.GetProposal(r.Context(), id)
	if err != nil {
		return false // the decision itself answers not-found and store errors
	}
	hints, _ := loadIntakeHints() //nolint:errcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
	if s.proposalAdminRankVr6w(hints, p) < platformTier {
		return false
	}
	if out := s.requireStepUp(w, r, actor, "decide_platform_tier"); out.Handled {
		s.emitAuditWithInviteToken(r, actor, "onboard_"+action, id, "", nil, nil, false, "refused", stepUpChallengeAudit)
		return true
	}
	return false
}

const reactivateNoIdentityRefusalVr6w = "refused: your sign-in carries no identity id, so a platform-tier login cannot be reactivated from it (step-up needs one); sign in again"

const selfReactivationRefusalVr6w = "refused: you cannot reactivate your own platform-tier login; another platform admin must"
