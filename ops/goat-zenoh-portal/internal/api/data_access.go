package api

// data_access.go serves the ADR 1075 discover->request->approve loop and the
// preemptive-grant path (ADR 1075 §12). A viewer requests access to a producer's
// namespace; the producer custodian (the vendor principal whose prefix IS the
// producer — the data-authority, data-access-roles.md §8) or an operator
// approves, which MINTS a grant row; and a custodian may also grant preemptively
// with no pending request. Both directions mint the identical row the data-roles
// resolver evaluates. Modeled on the ADR 1071/1072 sponsor-door workflow.

import (
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// loadDirectoryOrgs reads the org rows (id/kind/nation) from the declared roster
// projection for the grant-target picker. A missing/malformed file is an error
// (fail-closed — an empty directory must not silently read as "no orgs exist").
func loadDirectoryOrgs(path string) ([]directoryOrg, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var rf struct {
		Orgs []struct {
			ID     string `json:"id"`
			Kind   string `json:"kind"`
			Nation string `json:"nation"`
		} `json:"orgs"`
	}
	if err := json.Unmarshal(raw, &rf); err != nil {
		return nil, err
	}
	out := make([]directoryOrg, 0, len(rf.Orgs))
	for _, o := range rf.Orgs {
		if o.ID == "" {
			continue
		}
		out = append(out, directoryOrg{ID: o.ID, Kind: o.Kind, Nation: o.Nation})
	}
	return out, nil
}

// isProducerCustodianOrOperator reports whether the caller may approve/grant/
// revoke for a producer: an operator always may; a vendor principal may only for
// its OWN producer namespace (the data-authority owns its own grants). The
// principal is already on the context (AuthMiddleware ran).
func isProducerCustodianOrOperator(r *http.Request, producer string) bool {
	p := PrincipalFromContext(r.Context())
	if p == nil {
		return false
	}
	if p.Operator {
		return true
	}
	return p.Vendor != "" && p.Vendor == producer
}

// mayGovernProducer generalizes the custody gate to the ORG data-governance role
// (ADR 1102): a caller may grant/revoke for a producer if it is the producer's
// own custodian or an operator (isProducerCustodianOrOperator), OR it holds
// `data.govern.org:<O>` for an org O that owns the target producer.
//
// The org authority is now read from the FIRST-CLASS derived role
// (`RolesForAnchor(...).Roles`, the same `data.govern.org:<O>` sentence the
// resolver emits and `GET /api/access/data-roles` enumerates), not re-derived
// inline — so the gate and the access-language surface share one definition of
// "who governs org O" (WS3, data-access-roles.yaml). A browser-only org admin
// governs through the console's operator/service-token path (its person is
// recorded via X-Goat-Operator-Actor); a partner-org admin authenticating
// directly with only an Ory credential is a separate auth-surface item
// (sponsor-scoped console, ADR 1072), not this gate. Fail-closed: a caller with
// no producer identity, or a roster that can't be loaded, gets no org authority.
func (s *Server) mayGovernProducer(r *http.Request, producer string) bool {
	if isProducerCustodianOrOperator(r, producer) {
		return true
	}
	p := PrincipalFromContext(r.Context())
	if p == nil || p.Vendor == "" || s.cfg.RosterPath == "" {
		return false
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		return false // fail-closed: can't confirm org membership → no org authority
	}
	// The orgs this caller governs — the ONE source (ADR 1102): a nominated
	// admin governs its org; else, under the bootstrap-open default, a member of
	// a not-yet-nominated org governs it; an org with nominations is governed
	// only by its nominees. A machine caller is never nominated, so its authority
	// rides the bootstrap-open member path (plus own-producer custody, the gate
	// above). Fail-closed on any store error.
	governs, _, err := s.governedOrgs(r, "", access.OrgsFromRoles(dr.RolesForAnchor(p.Vendor).Roles))
	if err != nil {
		return false
	}
	if len(governs) == 0 {
		return false
	}
	for _, o := range dr.OrgsOfProducer(producer) {
		if governs[o] {
			return true // governs an org that owns the producer
		}
	}
	return false
}

// requesterOf resolves the requesting principal's identity + display label for a
// request row (attribution, not a join key). An operator/delegated actor is
// named by handle (actorOf); a vendor principal by its prefix.
func requesterOf(r *http.Request) (id, label string) {
	p := PrincipalFromContext(r.Context())
	if p != nil && p.Vendor != "" {
		return p.Vendor, p.Vendor
	}
	a := actorOf(r)
	return a, a
}

// producerExists checks the live registry for a producer under this key, so a
// request/grant can't name a producer that does not exist. Fail-closed on a DB
// error.
func (s *Server) producerExists(r *http.Request, producer string) (bool, error) {
	if _, err := s.reg.GetVendor(r.Context(), producer); err != nil {
		if errors.Is(err, registry.ErrNotFound) {
			return false, nil
		}
		return false, err
	}
	return true, nil
}

type createRequestBody struct {
	Producer      string `json:"producer"`
	GranteeKind   string `json:"grantee_kind"` // "org" | "principal"; default principal
	GranteeID     string `json:"grantee_id"`   // org id, or the requester's identity UUID
	Justification string `json:"justification"`
}

// handleCreateAccessRequest lets any authenticated viewer ask for access to a
// producer's restricted namespace (ADR 1075 §"The loop"). The grantee scope is
// the caller's choice — "for my org X" (the coalition default) or "for me"
// (an identity UUID). Mints nothing; a custodian decides.
func (s *Server) handleCreateAccessRequest(w http.ResponseWriter, r *http.Request) {
	var body createRequestBody
	if err := readJSON(w, r, &body); err != nil {
		writeDecodeError(w, err)
		return
	}
	body.Producer = strings.TrimSpace(body.Producer)
	if body.Producer == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "producer is required")
		return
	}
	ok, err := s.producerExists(r, body.Producer)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	if !ok {
		writeError(w, http.StatusNotFound, "not_found", "no producer registered under this key")
		return
	}
	reqID, reqLabel := requesterOf(r)
	req := &registry.DataAccessRequest{
		RequesterID:    reqID,
		RequesterLabel: reqLabel,
		Producer:       body.Producer,
		GranteeKind:    body.GranteeKind,
		GranteeID:      body.GranteeID,
		Justification:  body.Justification,
	}
	if err := s.reg.CreateRequest(r.Context(), req); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.access.request", body.Producer,
		fmt.Sprintf(`{"id":%q,"grantee_kind":%q,"grantee_id":%q}`, req.ID, req.GranteeKind, req.GranteeID))
	writeJSON(w, http.StatusCreated, req)
}

// handleListAccessRequests lists requests. An operator sees all (filtered by
// ?producer=/?status=); a vendor principal sees only its OWN producer's inbox
// (the producer filter is forced to its prefix, ignoring a spoofed query).
func (s *Server) handleListAccessRequests(w http.ResponseWriter, r *http.Request) {
	p := PrincipalFromContext(r.Context())
	producer := strings.TrimSpace(r.URL.Query().Get("producer"))
	status := strings.TrimSpace(r.URL.Query().Get("status"))
	if p != nil && !p.Operator {
		producer = p.Vendor // a custodian only ever sees its own inbox
	}
	rows, err := s.reg.ListRequests(r.Context(), producer, status)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"requests": rows})
}

type decideRequestBody struct {
	Justification string `json:"justification"` // custodian's recorded why (defaults to the request's)
	EffectiveTo   *int64 `json:"effective_to"`  // optional grant expiry (unix) — the comrel-window pattern
	Note          string `json:"note"`
}

// handleApproveAccessRequest approves a pending request: the producer custodian
// or an operator mints a grant row from the request's grantee + producer, then
// marks the request approved (a single-shot guarded UPDATE — no double-mint).
func (s *Server) handleApproveAccessRequest(w http.ResponseWriter, r *http.Request) {
	req, ok := s.loadPendingRequest(w, r)
	if !ok {
		return
	}
	if !s.mayGovernProducer(r, req.Producer) {
		writeError(w, http.StatusForbidden, "not_custodian",
			"only the producer's custodian or an operator may approve — the producer owns its own grants")
		return
	}
	var body decideRequestBody
	_ = readJSON(w, r, &body)
	justification := strings.TrimSpace(body.Justification)
	if justification == "" {
		justification = req.Justification
	}
	grant := &registry.Grant{
		GranteeKind:   req.GranteeKind,
		GranteeID:     req.GranteeID,
		Producer:      req.Producer,
		GrantedBy:     actorOf(r),
		Justification: justification,
		EffectiveTo:   body.EffectiveTo,
	}
	if err := s.reg.CreateGrant(r.Context(), grant); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	if err := s.reg.DecideRequest(r.Context(), req.ID, registry.RequestApproved, actorOf(r), grant.ID, body.Note); err != nil {
		// The grant was minted but the request wasn't pending anymore — revoke the
		// just-minted grant so we never leave a stray grant behind a lost race.
		_ = s.reg.RevokeGrant(r.Context(), grant.ID, actorOf(r))
		if errors.Is(err, registry.ErrRequestNotPending) {
			writeError(w, http.StatusConflict, "not_pending", "request was already decided")
			return
		}
		writeError(w, http.StatusInternalServerError, "decide", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.access.approve", req.Producer,
		fmt.Sprintf(`{"request":%q,"grant":%q,"grantee_kind":%q,"grantee_id":%q}`, req.ID, grant.ID, grant.GranteeKind, grant.GranteeID))
	writeJSON(w, http.StatusOK, map[string]any{"request": req.ID, "status": registry.RequestApproved, "grant": grant})
}

// handleDeclineAccessRequest declines a pending request (custodian/operator). No
// grant is minted.
func (s *Server) handleDeclineAccessRequest(w http.ResponseWriter, r *http.Request) {
	req, ok := s.loadPendingRequest(w, r)
	if !ok {
		return
	}
	if !s.mayGovernProducer(r, req.Producer) {
		writeError(w, http.StatusForbidden, "not_custodian", "only the producer's custodian or an operator may decline")
		return
	}
	var body decideRequestBody
	_ = readJSON(w, r, &body)
	if err := s.reg.DecideRequest(r.Context(), req.ID, registry.RequestDeclined, actorOf(r), "", body.Note); err != nil {
		if errors.Is(err, registry.ErrRequestNotPending) {
			writeError(w, http.StatusConflict, "not_pending", "request was already decided")
			return
		}
		writeError(w, http.StatusInternalServerError, "decide", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.access.decline", req.Producer,
		fmt.Sprintf(`{"request":%q,"note":%q}`, req.ID, body.Note))
	writeJSON(w, http.StatusOK, map[string]any{"request": req.ID, "status": registry.RequestDeclined})
}

// loadPendingRequest loads the {id} request and confirms it is still pending,
// writing the error itself. Returns ok=false when the caller should stop.
func (s *Server) loadPendingRequest(w http.ResponseWriter, r *http.Request) (*registry.DataAccessRequest, bool) {
	id := r.PathValue("id")
	req, err := s.reg.GetRequest(r.Context(), id)
	if errors.Is(err, registry.ErrNotFound) {
		writeError(w, http.StatusNotFound, "not_found", "request not found")
		return nil, false
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return nil, false
	}
	if req.Status != registry.RequestPending {
		writeError(w, http.StatusConflict, "not_pending", fmt.Sprintf("request is %q, not pending", req.Status))
		return nil, false
	}
	return req, true
}

type createGrantBody struct {
	Producer      string `json:"producer"`
	GranteeKind   string `json:"grantee_kind"` // "org" | "principal"
	GranteeID     string `json:"grantee_id"`
	Justification string `json:"justification"`
	EffectiveTo   *int64 `json:"effective_to"` // optional expiry (unix)
}

// handleCreateGrant is the PREEMPTIVE grant path (ADR 1075 §12): a producer
// custodian (or operator) grants access before anyone asks — the grant verb has
// no pending-request precondition. Same row as the request-approved path.
func (s *Server) handleCreateGrant(w http.ResponseWriter, r *http.Request) {
	var body createGrantBody
	if err := readJSON(w, r, &body); err != nil {
		writeDecodeError(w, err)
		return
	}
	body.Producer = strings.TrimSpace(body.Producer)
	if body.Producer == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "producer is required")
		return
	}
	if !s.mayGovernProducer(r, body.Producer) {
		writeError(w, http.StatusForbidden, "not_custodian",
			"only the producer's custodian or an operator may grant access to it")
		return
	}
	ok, err := s.producerExists(r, body.Producer)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	if !ok {
		writeError(w, http.StatusNotFound, "not_found", "no producer registered under this key")
		return
	}
	// An org grant has to name an org the roster declares. Any other string is
	// accepted by the store and then wraps to nobody, with only a note on the
	// audience read to say so. The usual slip is a member's anchor where its org
	// id belongs, so the refusal names the org that declares that anchor.
	if body.GranteeKind == registry.GranteeOrg && s.cfg.RosterPath != "" {
		dr, err := s.rosters.Load(s.cfg.RosterPath)
		if err != nil {
			log.Printf("data-access: load roster: %v", err)
			writeError(w, http.StatusInternalServerError, "registry", "could not load the declared registry")
			return
		}
		org := strings.TrimSpace(body.GranteeID)
		if !dr.KnownOrg(org) {
			msg := fmt.Sprintf("no org %q in the roster", org)
			if orgs := dr.OrgsOfProducer(org); len(orgs) > 0 {
				msg += "; that is a member anchor, declared under org " + strings.Join(orgs, ", ")
			}
			writeError(w, http.StatusBadRequest, "unknown_org", msg)
			return
		}
	}
	grant := &registry.Grant{
		GranteeKind:   body.GranteeKind,
		GranteeID:     body.GranteeID,
		Producer:      body.Producer,
		GrantedBy:     actorOf(r),
		Justification: body.Justification,
		EffectiveTo:   body.EffectiveTo,
	}
	if err := s.reg.CreateGrant(r.Context(), grant); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.access.grant", body.Producer,
		fmt.Sprintf(`{"grant":%q,"grantee_kind":%q,"grantee_id":%q,"preemptive":true}`, grant.ID, grant.GranteeKind, grant.GranteeID))
	writeJSON(w, http.StatusCreated, grant)
}

// governsOrg reports whether the caller governs `org` — an operator (platform),
// or a `data.govern.org:<org>` holder under the ADR 1102 single-sourced
// definition. It delegates to `governedOrgs` (the one definition of "who governs
// org O", shared with mayGovernProducer + the resolver surface) rather than
// re-deriving, so co-publish and governance never disagree. Fail-closed: no
// principal, no roster, or a store error → false. `dr` is the already-loaded
// roster (callers load it once).
func (s *Server) governsOrg(r *http.Request, dr *access.DataRoster, org string) bool {
	if strings.TrimSpace(org) == "" {
		return false
	}
	if p := PrincipalFromContext(r.Context()); p != nil && p.Operator {
		return true // the platform governs every org
	}
	p := PrincipalFromContext(r.Context())
	if p == nil || p.Vendor == "" {
		return false
	}
	memberOrgs := access.OrgsFromRoles(dr.RolesForAnchor(p.Vendor).Roles)
	governs, _, err := s.governedOrgs(r, "", memberOrgs)
	if err != nil {
		return false // fail-closed: can't confirm governance
	}
	return governs[org]
}

type createMultipubTopicBody struct {
	Org           string `json:"org"`           // owning org id (the naming org)
	DisplayName   string `json:"display_name"`  // human alias (what a discoverer sees)
	Justification string `json:"justification"` // the recorded why (creation is deliberate)
	Persist       *bool  `json:"persist"`       // render a store (history); default true
}

// handleCreateMultipubTopic is the DELIBERATE act of standing up a
// multi-publisher topic (ADR 1105). Only a govern-admin of the
// naming org (or operator) may create one — the PUBSUB-ORG-ADMIN capability.
// It mints the org's stable pool handle (once) and an opaque topic id,
// returning the multipub/<pool>/<topic> key-base. Single-owner roots are never
// involved — this is a separate, marked, discoverable namespace.
func (s *Server) handleCreateMultipubTopic(w http.ResponseWriter, r *http.Request) {
	var body createMultipubTopicBody
	if err := readJSON(w, r, &body); err != nil {
		writeDecodeError(w, err)
		return
	}
	body.Org = strings.TrimSpace(body.Org)
	body.DisplayName = strings.TrimSpace(body.DisplayName)
	if body.Org == "" || body.DisplayName == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "org and display_name are required")
		return
	}
	if s.cfg.RosterPath == "" {
		writeError(w, http.StatusServiceUnavailable, "no_roster", "roster unavailable — cannot confirm governance authority")
		return
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "roster", err.Error())
		return
	}
	if !s.governsOrg(r, dr, body.Org) {
		writeError(w, http.StatusForbidden, "not_governor",
			"only a data-governance admin of the org (data.govern.org), or an operator, may create a multi-publisher topic for it")
		return
	}
	persist := true
	if body.Persist != nil {
		persist = *body.Persist
	}
	st, err := s.reg.CreateMultipubTopic(r.Context(), body.Org, body.DisplayName, body.Justification, actorOf(r), persist)
	if err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.multipub-topic.create", st.KeyBase(),
		fmt.Sprintf(`{"topic_id":%q,"org":%q,"display_name":%q}`, st.TopicID, st.Org, st.DisplayName))
	writeJSON(w, http.StatusCreated, multipubTopicView(st))
}

// handleListMultipubTopics is the discovery read: every multi-publisher topic
// (optionally ?org=), each carrying its owning org, key-base, and — via the
// grant store — its declared publisher set, so the MULTI-PUBLISHER marker is
// discernible on discovery. Any authenticated principal may list (visibility
// is advisory, ADR 1028); the marker is the point.
func (s *Server) handleListMultipubTopics(w http.ResponseWriter, r *http.Request) {
	topics, err := s.reg.ListMultipubTopics(r.Context(), strings.TrimSpace(r.URL.Query().Get("org")))
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	now := time.Now().Unix()
	out := make([]map[string]any, 0, len(topics))
	for _, st := range topics {
		v := multipubTopicView(&st)
		// Attach the declared publisher set (in-effect write grants naming this
		// topic's key-base) — the discoverable "who writes here".
		grants, gerr := s.reg.ListGrants(r.Context(), st.KeyBase(), registry.GranteePrincipal, "", true)
		if gerr == nil {
			var pubs []string
			for _, g := range grants {
				if g.IsWrite() && g.InEffectAt(now) {
					pubs = append(pubs, g.GranteeID)
				}
			}
			v["publishers"] = pubs
		}
		out = append(out, v)
	}
	writeJSON(w, http.StatusOK, map[string]any{"multipub_topics": out})
}

// multipubTopicView is the discovery projection of a multi-publisher topic —
// always carrying the MULTI-PUBLISHER class marker and the owning org (handle
// → name is the surface's alias resolve).
func multipubTopicView(st *registry.MultipubTopic) map[string]any {
	return map[string]any{
		"topic_id":     st.TopicID,
		"org":          st.Org,
		"pool_handle":  st.PoolHandle,
		"key_base":     st.KeyBase(),
		"display_name": st.DisplayName,
		"persist":      st.Persist,
		"status":       st.Status,
		"class":        "multi-publisher", // the discernible marker (ADR 1105)
		"created_at":   st.CreatedAt,
	}
}

type createCoPublishBody struct {
	Writer        string `json:"writer"`         // the co-publisher's anchor (grantee X)
	MultipubTopicID string `json:"multipub_topic_id"` // the target — a REGISTERED multi-publisher topic id
	Justification string `json:"justification"`
	EffectiveTo   *int64 `json:"effective_to"` // optional expiry (unix)
}

// handleCreateCoPublishGrant adds a publisher to a multi-publisher topic (ADR
// 1105, PUBSUB-ORG-ADMIN). The target MUST be a registered multi-publisher
// topic — the guardrail: an admin cannot open a single-owner root.
// At-most-own-org: the multi-publisher topic's org and the writer X must both
// be an org the caller governs. Justification required; mints a WRITE grant
// whose producer is the topic's key-base. Takes effect on the next router
// render + restart.
func (s *Server) handleCreateCoPublishGrant(w http.ResponseWriter, r *http.Request) {
	var body createCoPublishBody
	if err := readJSON(w, r, &body); err != nil {
		writeDecodeError(w, err)
		return
	}
	body.Writer = strings.TrimSpace(body.Writer)
	body.MultipubTopicID = strings.TrimSpace(body.MultipubTopicID)
	body.Justification = strings.TrimSpace(body.Justification)
	if body.Writer == "" || body.MultipubTopicID == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "writer (co-publisher anchor) and multipub_topic_id are required")
		return
	}
	if body.Justification == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "justification is required for a co-publish grant (the recorded why)")
		return
	}
	// The GUARDRAIL: resolve the target to a REGISTERED multi-publisher topic.
	// Anything that is not a multi-publisher topic (a single-owner anchor root,
	// an unknown id) cannot be a co-publish target.
	st, err := s.reg.GetMultipubTopic(r.Context(), body.MultipubTopicID)
	if errors.Is(err, registry.ErrNotFound) {
		writeError(w, http.StatusNotFound, "not_found", "no registered multi-publisher topic with that id — co-publish can only target a multi-publisher topic, never a single-owner root")
		return
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	if st.Status != registry.MultipubTopicActive {
		writeError(w, http.StatusConflict, "retired", "multi-publisher topic is retired")
		return
	}
	if s.cfg.RosterPath == "" {
		writeError(w, http.StatusServiceUnavailable, "no_roster", "roster unavailable — cannot confirm governance authority")
		return
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "roster", err.Error())
		return
	}
	// At-most-own-org: the caller must govern the topic's org, and the writer X
	// must belong to that same org (a publisher can only join its OWN org's
	// multi-publisher topic).
	if !s.governsOrg(r, dr, st.Org) {
		writeError(w, http.StatusForbidden, "not_governor",
			"you must hold data.govern.org for the multi-publisher topic's org to add a publisher to it")
		return
	}
	writerInOrg := false
	for _, o := range dr.OrgsOfProducer(body.Writer) {
		if o == st.Org {
			writerInOrg = true
			break
		}
	}
	if !writerInOrg {
		writeError(w, http.StatusForbidden, "cross_org",
			"the writer must belong to the multi-publisher topic's own org — cross-org co-publish is refused")
		return
	}
	if exists, err := s.producerExists(r, body.Writer); err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	} else if !exists {
		writeError(w, http.StatusNotFound, "not_found", "no producer registered under key "+body.Writer)
		return
	}
	grant := &registry.Grant{
		GranteeKind:   registry.GranteePrincipal,
		GranteeID:     body.Writer,
		Producer:      st.KeyBase(),
		Action:        registry.GrantActionWrite,
		GrantedBy:     actorOf(r),
		Justification: body.Justification,
		EffectiveTo:   body.EffectiveTo,
	}
	if err := s.reg.CreateGrant(r.Context(), grant); err != nil {
		writeError(w, http.StatusBadRequest, "validate", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.copublish.grant", st.KeyBase(),
		fmt.Sprintf(`{"grant":%q,"writer":%q,"multipub_topic":%q,"org":%q}`, grant.ID, body.Writer, st.TopicID, st.Org))
	writeJSON(w, http.StatusCreated, grant)
}

// handleListGrants lists grant rows. Operator sees all (filtered by
// ?producer=/?grantee_kind=/?grantee_id=); a vendor principal sees only its own
// producer's grants.
func (s *Server) handleListGrants(w http.ResponseWriter, r *http.Request) {
	p := PrincipalFromContext(r.Context())
	producer := strings.TrimSpace(r.URL.Query().Get("producer"))
	if p != nil && !p.Operator {
		producer = p.Vendor
	}
	rows, err := s.reg.ListGrants(r.Context(), producer,
		strings.TrimSpace(r.URL.Query().Get("grantee_kind")),
		strings.TrimSpace(r.URL.Query().Get("grantee_id")), false)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"grants": rows})
}

// handleCoPublishGrantsExport materializes the in-effect co-publish (write)
// grants into the shape the ACL render consumes — the `zenoh_copublish_grants`
// group_var, a map of writer-anchor → [ target key-expr base, … ] (ADR 1105).
// This is the portal→group_var seam: the registry stays authoritative, the
// operator captures this snapshot into group_vars/<env>/data-access.yml at
// deploy time, re-renders, and restarts the router.
// Operator-gated: it enumerates the fabric's cross-anchor write topology.
func (s *Server) handleCoPublishGrantsExport(w http.ResponseWriter, r *http.Request) {
	grants, err := s.reg.ActiveWriteGrants(r.Context())
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	now := time.Now().Unix()
	out := map[string][]string{}
	seen := map[string]bool{}
	for _, g := range grants {
		if !g.InEffectAt(now) {
			continue
		}
		base := g.CoPublishKeyBase()
		if base == "" {
			continue
		}
		key := g.GranteeID + "\x00" + base
		if seen[key] {
			continue // de-dupe overlapping grants to the same base
		}
		seen[key] = true
		out[g.GranteeID] = append(out[g.GranteeID], base)
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"zenoh_copublish_grants": out,
		"note":                   "materialized in-effect co-publish (write) grants (ADR 1105); capture into group_vars/<env>/data-access.yml, set zenoh_copublish_enabled: true, re-render + restart the router",
	})
}

// handleRevokeGrant end-dates a grant (custodian of the grant's producer, or
// operator). Deprecate-never-delete — the row stays for the audit history.
func (s *Server) handleRevokeGrant(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	g, err := s.reg.GetGrant(r.Context(), id)
	if errors.Is(err, registry.ErrNotFound) {
		writeError(w, http.StatusNotFound, "not_found", "grant not found")
		return
	}
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	if !s.mayGovernProducer(r, g.Producer) {
		writeError(w, http.StatusForbidden, "not_custodian", "only the producer's custodian or an operator may revoke its grants")
		return
	}
	if err := s.reg.RevokeGrant(r.Context(), id, actorOf(r)); err != nil {
		writeError(w, http.StatusInternalServerError, "revoke", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "data.access.revoke", g.Producer,
		fmt.Sprintf(`{"grant":%q}`, id))
	writeJSON(w, http.StatusOK, map[string]any{"grant": id, "status": registry.GrantRevoked})
}

// directoryOrg / directoryPerson are the grant-target picker rows (ADR 1075 §12
// preemptive grants: the publisher browses the roster the other way). Orgs come
// from the declared roster; people from live Kratos (their identity UUID is the
// principal-grant key). Both are display + a machine id the caller passes back
// as grantee_id.
type directoryOrg struct {
	ID     string `json:"id"`
	Kind   string `json:"kind,omitempty"`
	Nation string `json:"nation,omitempty"`
}
type directoryPerson struct {
	ID    string `json:"id"` // identity UUID — the principal-grant key
	Name  string `json:"name,omitempty"`
	Email string `json:"email,omitempty"`
}

// handleAccessDirectory serves the grant-target directory (ADR 1075 §12). Any
// authenticated viewer may read it — it is the same roster the COP serves, the
// other way round. Orgs derive from the roster; people from Kratos when wired
// (empty + a note otherwise, never fabricated). ?q= narrows case-insensitively.
func (s *Server) handleAccessDirectory(w http.ResponseWriter, r *http.Request) {
	q := strings.ToLower(strings.TrimSpace(r.URL.Query().Get("q")))
	orgs := []directoryOrg{}
	notes := []string{}
	if s.cfg.RosterPath != "" {
		roster, err := loadDirectoryOrgs(s.cfg.RosterPath)
		if err != nil {
			writeError(w, http.StatusInternalServerError, "roster", "could not load the declared roster: "+err.Error())
			return
		}
		for _, o := range roster {
			if q == "" || strings.Contains(strings.ToLower(o.ID), q) {
				orgs = append(orgs, o)
			}
		}
	} else {
		notes = append(notes, "roster not configured — org directory is empty")
	}
	people := []directoryPerson{}
	if s.kratos != nil {
		ids, err := s.kratos.ListIdentities(r.Context())
		if err != nil {
			writeError(w, http.StatusInternalServerError, "kratos", "could not list identities: "+err.Error())
			return
		}
		for _, idn := range ids {
			name, email := idn.Name(), idn.Email()
			if q == "" || strings.Contains(strings.ToLower(name), q) || strings.Contains(strings.ToLower(email), q) {
				people = append(people, directoryPerson{ID: idn.ID, Name: name, Email: email})
			}
		}
	} else {
		notes = append(notes, "identity layer (kratos) not configured — person directory is empty; grant to an org, or supply an identity UUID directly")
	}
	writeJSON(w, http.StatusOK, map[string]any{"orgs": orgs, "people": people, "notes": notes})
}
