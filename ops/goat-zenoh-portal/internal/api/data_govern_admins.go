package api

// data_govern_admins.go is the U9 surface: nominate / remove an org's
// data-governance admins (ADR 1102 decision 1), and — via governedOrgs — the
// SINGLE place org governance authority (data.govern.org:<O>) is decided.
//
// Governance authority is NOT emitted by the pure roster resolver; it is
// computed here from the nomination store plus the per-goatnet bootstrap-open
// default (config.GovernBootstrapOpen), so the resolver surface
// (goat access show / GET /api/access/data-roles) and the grant/nominate gates
// all read one definition:
//
//   - a NOMINATED admin of O always governs O;
//   - under the bootstrap-open default (the kick-start posture), a MEMBER of an
//     org with NO nominations governs it (the goatnet starts open; orgs narrow
//     themselves) — on BOTH planes (a person via membership, a producer cert via
//     its owning org);
//   - an org WITH nominations is governed ONLY by its nominees, always;
//   - under the canonical fail-closed default, membership alone confers nothing
//     — a new org is seeded with its first member so it is not born locked out.
//
// Nomination is at-most-own-scope (ADR 1071/1072 pattern): only a caller who
// currently governs O may nominate or remove. Removing the last active admin is
// refused — narrowing is monotonic; an org never silently falls back open
// (recovery for a locked-out org is operator proxy stewardship, ADR 1102).

import (
	"errors"
	"fmt"
	"net/http"
	"sort"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// governedOrgs is the SINGLE source of org data-governance authority (ADR 1102).
// Given the orgs a principal is a MEMBER of (memberOrgs, from its data.view.org
// sentences) and — for a person — its identity UUID (personIdentity; "" for a
// machine, which is never nominated), it returns the set of orgs the principal
// may govern, and honesty notes explaining any org membership that did NOT
// confer governance.
//
// The rule, in order:
//   - a NOMINATED admin of O always governs O (both bootstrap-open and closed);
//   - additionally, when the goatnet's bootstrap-open default is set, a MEMBER
//     of an org O that has NO active nominations governs O (the kick-start
//     posture — the goatnet starts open, orgs narrow themselves);
//   - an org WITH active nominations is governed ONLY by its nominees, always
//     (narrowing is unconditional and independent of the open default);
//   - when the default is closed, a member of a not-yet-nominated org does NOT
//     govern it (fail-closed; the canonical model seeds a new org's first
//     member so it is not born locked out).
//
// Fail-closed: a store error propagates so the caller denies rather than
// serving unverified authority.
func (s *Server) governedOrgs(r *http.Request, personIdentity string, memberOrgs []string) (map[string]bool, []string, error) {
	narrowed, err := s.reg.GovernNarrowedOrgs(r.Context())
	if err != nil {
		return nil, nil, err
	}
	var adminOf map[string]bool
	if personIdentity != "" {
		if adminOf, err = s.reg.GovernAdminOrgs(r.Context(), personIdentity); err != nil {
			return nil, nil, err
		}
	}
	governs := map[string]bool{}
	for o := range adminOf {
		governs[o] = true // a nominated admin always governs its org
	}
	var notes []string
	for _, o := range memberOrgs {
		if governs[o] {
			continue
		}
		switch {
		case narrowed[o]:
			notes = append(notes, "org "+o+" has nominated governance admins — governance is narrowed to them (ADR 1102); this principal is not nominated")
		case s.cfg.GovernBootstrapOpen:
			governs[o] = true // kick-start posture: member of a not-yet-nominated org governs it
		default:
			notes = append(notes, "org "+o+" has no nominated governance admins and the goatnet is fail-closed (ADR 1102); membership alone does not confer governance")
		}
	}
	return governs, notes, nil
}

// governRolesFor returns the data.govern.org:<O> sentences a principal holds,
// plus honesty notes — the resolver-surface projection of governedOrgs (so
// `goat access show` and the data-roles endpoint speak the same governance the
// grant gate enforces).
func (s *Server) governRolesFor(r *http.Request, personIdentity string, memberOrgs []string) ([]string, []string, error) {
	governs, notes, err := s.governedOrgs(r, personIdentity, memberOrgs)
	if err != nil {
		return nil, nil, err
	}
	roles := make([]string, 0, len(governs))
	for o := range governs {
		roles = append(roles, "data.govern.org:"+o)
	}
	sort.Strings(roles)
	return roles, notes, nil
}

// mayGovernOrg reports whether the caller may administer org O's governance
// (nominate/remove admins): an operator always may; a producer-cert caller may
// when its narrowed derived role set carries data.govern.org:<O>. Same
// fail-closed posture as mayGovernProducer.
func (s *Server) mayGovernOrg(r *http.Request, org string) bool {
	p := PrincipalFromContext(r.Context())
	if p == nil {
		return false
	}
	if p.Operator {
		return true
	}
	if p.Vendor == "" || s.cfg.RosterPath == "" {
		return false
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		return false
	}
	// A machine caller: its member orgs are the anchor's owning orgs; it is
	// never a nominated admin (personIdentity ""), so it governs O only via the
	// bootstrap-open member path on a not-yet-nominated org.
	governs, _, err := s.governedOrgs(r, "", access.OrgsFromRoles(dr.RolesForAnchor(p.Vendor).Roles))
	if err != nil {
		return false
	}
	return governs[org]
}

// governAdminsResponse lists an org's nomination rows plus which mode the org
// is in — "bootstrap" (no active nominations; every member governs) or
// "nominated" (narrowed to the listed admins).
type governAdminsResponse struct {
	Org    string                 `json:"org,omitempty"`
	Mode   string                 `json:"mode,omitempty"`
	Admins []registry.GovernAdmin `json:"admins"`
}

// handleListGovernAdmins serves GET /api/access/govern-admins[?org=<O>][&all=1]:
// nomination rows (active by default; all=1 includes removed history). Same
// service-consumer auth as the other access reads.
func (s *Server) handleListGovernAdmins(w http.ResponseWriter, r *http.Request) {
	if !s.lookupAuthorized(r) {
		writeError(w, http.StatusUnauthorized, "auth", "operator bearer or service token required")
		return
	}
	org := strings.TrimSpace(r.URL.Query().Get("org"))
	onlyActive := r.URL.Query().Get("all") == ""
	rows, err := s.reg.ListGovernAdmins(r.Context(), org, onlyActive)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", "could not list governance admins")
		return
	}
	resp := governAdminsResponse{Org: org, Admins: rows}
	if org != "" {
		active, err := s.reg.ListGovernAdmins(r.Context(), org, true)
		if err != nil {
			writeError(w, http.StatusInternalServerError, "db", "could not list governance admins")
			return
		}
		switch {
		case len(active) > 0:
			resp.Mode = "nominated (narrowed: only the active admins listed govern — ADR 1102)"
		case s.cfg.GovernBootstrapOpen:
			resp.Mode = "bootstrap-open (no admins nominated; any member of the org may govern it until it names admins — ADR 1102 kick-start default)"
		default:
			resp.Mode = "fail-closed (no admins nominated; membership alone does not confer governance — ADR 1102 canonical default; seed the first member to establish governance)"
		}
	}
	writeJSON(w, http.StatusOK, resp)
}

type nominateGovernAdminBody struct {
	Org           string `json:"org"`
	IdentityID    string `json:"identity_id"` // nominee identity UUID; or use Email
	Email         string `json:"email"`       // login-email alias, resolved to the UUID
	Justification string `json:"justification"`
}

// handleNominateGovernAdmin serves POST /api/access/govern-admins: nominate a
// person as org O's governance admin. Gate: the caller must currently govern O
// (at-most-own-scope). The nominee must resolve to a live identity AND be an
// active registry member of O — an org self-administers its own admins, it
// cannot appoint outsiders.
func (s *Server) handleNominateGovernAdmin(w http.ResponseWriter, r *http.Request) {
	var body nominateGovernAdminBody
	if err := readJSON(w, r, &body); err != nil {
		writeDecodeError(w, err)
		return
	}
	body.Org = strings.TrimSpace(body.Org)
	if body.Org == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "org is required")
		return
	}
	if !s.mayGovernOrg(r, body.Org) {
		writeError(w, http.StatusForbidden, "not_governor",
			"only a current governance admin of the org (or an operator) may nominate")
		return
	}
	if s.humans == nil {
		writeError(w, http.StatusServiceUnavailable, "unavailable",
			"identity layer (kratos) not configured — nominees cannot be resolved")
		return
	}
	var ref identityRef
	var rerr error
	switch {
	case strings.TrimSpace(body.IdentityID) != "":
		ref, rerr = s.humans.byID(r.Context(), strings.TrimSpace(body.IdentityID))
	case strings.TrimSpace(body.Email) != "":
		ref, rerr = s.humans.byEmail(r.Context(), strings.TrimSpace(body.Email))
	default:
		writeError(w, http.StatusBadRequest, "bad_request", "identity_id or email is required")
		return
	}
	if errors.Is(rerr, errHumanNotFound) {
		writeError(w, http.StatusNotFound, "unknown_principal", "no identity matched the nominee selector")
		return
	}
	if rerr != nil {
		writeError(w, http.StatusInternalServerError, "identity", "could not resolve the nominee")
		return
	}
	// Membership check: the nominee must be an active member of the org (the
	// roster's data.view.org:<O> sentence is the membership marker).
	if s.cfg.RosterPath == "" {
		writeError(w, http.StatusServiceUnavailable, "unavailable",
			"declared registry (roster path) not configured — membership cannot be verified")
		return
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "registry", "could not load the declared registry")
		return
	}
	member := false
	for _, o := range access.OrgsFromRoles(dr.RolesForPerson(access.EmailHash(ref.Email)).Roles) {
		if o == body.Org {
			member = true
			break
		}
	}
	if !member {
		writeError(w, http.StatusUnprocessableEntity, "not_a_member",
			"the nominee is not an active registry member of the org — an org administers its own members only")
		return
	}
	g := &registry.GovernAdmin{
		Org:           body.Org,
		IdentityID:    ref.ID,
		NominatedBy:   actorOf(r),
		Justification: body.Justification,
	}
	if err := s.reg.NominateGovernAdmin(r.Context(), g); err != nil {
		if errors.Is(err, registry.ErrGovernAdminExists) {
			writeError(w, http.StatusConflict, "already_admin", err.Error())
			return
		}
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.govern.nominate", g.Org,
		fmt.Sprintf(`{"id":%q,"identity":%q}`, g.ID, g.IdentityID))
	writeJSON(w, http.StatusCreated, g)
}

// handleRemoveGovernAdmin serves POST /api/access/govern-admins/{id}/remove.
// Same at-most-own-scope gate; refuses removing the org's last active admin
// (monotonic narrowing — recovery is operator proxy stewardship, never a
// silent fall-open).
func (s *Server) handleRemoveGovernAdmin(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	g, err := s.reg.GetGovernAdmin(r.Context(), id)
	if errors.Is(err, registry.ErrNotFound) {
		writeError(w, http.StatusNotFound, "not_found", "nomination not found")
		return
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", "could not load the nomination")
		return
	}
	if !s.mayGovernOrg(r, g.Org) {
		writeError(w, http.StatusForbidden, "not_governor",
			"only a current governance admin of the org (or an operator) may remove")
		return
	}
	if err := s.reg.RemoveGovernAdmin(r.Context(), id, actorOf(r)); err != nil {
		if errors.Is(err, registry.ErrLastGovernAdmin) {
			writeError(w, http.StatusConflict, "last_admin", err.Error())
			return
		}
		writeError(w, http.StatusInternalServerError, "remove", "could not remove the nomination")
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.govern.remove", g.Org,
		fmt.Sprintf(`{"id":%q,"identity":%q}`, g.ID, g.IdentityID))
	writeJSON(w, http.StatusOK, map[string]any{"id": id, "status": registry.GovernAdminRemoved})
}
