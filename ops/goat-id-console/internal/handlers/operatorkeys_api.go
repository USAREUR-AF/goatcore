package handlers

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/opkeys"
)

// Machine half of the operator-key flow — the same engine the browser pages
// use, so `goat admin operator-key` and the console cannot drift into two
// different sets of rules. Every rule that matters (a request grants nothing,
// approval writes the file, revoke removes the file before the record changes,
// the secret is delivered once) lives in the store and the writer, not here.
//
// Two doors into a REQUEST, matching the console's own "for yourself vs for
// somebody else" split (docs/design/operator-action-audit.md, "Requesting one,
// both ways"): an operator principal may name any handle, typically at the
// point of onboarding somebody. A non-operator principal — a participant's own
// vp_ key, resolved and sponsorship-authorized by onboardAPIGate the same way
// every other machine-realm route resolves one — may request ONLY for their
// own resolved handle. Before this, the CLI's only door required already
// holding an operator key to ask for a first one at all (#1373); this is the
// CLI-side counterpart of the console's session-authenticated self-service
// /operator-key page, not a new authorization concept.
//
// The approval gate is what makes both doors safe: recording a request grants
// nothing (RequestOperatorKey), and a human — never the requester — still has
// to decide it (#210, OperatorKeyDecisionCheck). Widening who may ASK changes
// nothing about who may GRANT.
//
// COLLECT is narrower on purpose: an operator's API key proves nothing about
// which human is typing, so it stays bound to the resolved principal's own
// handle for every caller, operator or not (see apiCollectOperatorKey).

type operatorKeyRequestBody struct {
	Handle string `json:"handle"`
	Email  string `json:"email"`
	Env    string `json:"env"`
	Reason string `json:"reason"`
}

// apiRequestOperatorKey records a request. An operator principal may name any
// handle (acting for somebody else, typically at onboarding). A non-operator
// principal may request only for themselves: the resolved principal's own
// handle is used regardless of what the body says, and a body handle that
// names somebody else is refused rather than silently overridden — silently
// substituting the caller's own handle would let a UI bug or a copy-pasted
// request body look like it worked while quietly filing under the wrong name.
func (s *Server) apiRequestOperatorKey(w http.ResponseWriter, r *http.Request) {
	p := onboardPrincipalFromCtx(r.Context())
	var body operatorKeyRequestBody
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_json", "could not parse the request body")
		return
	}
	bodyHandle := strings.TrimSpace(body.Handle)

	handle := bodyHandle
	email := body.Email
	if !p.IsOperator {
		// Self-request door (mirrors the console's session-based one). A
		// participant principal's Handle is its org anchor — on a slot-pool
		// goatnet a 32-hex UUID, not a person — so the ROSTER handle is bound
		// through the login the portal recorded on the identity and the same
		// resolver the browser page uses (operatorKeySelfHandle). Filing under
		// the anchor would name the key file and every attributed action after
		// a hex string, and ADR 1182's revoke-by-handle would never find it.
		//
		// The door also starts under the holder rule the browser door does not
		// enforce yet (platform-admin-lifecycle.md § Operator key, ADR 1182):
		// only a platform admin holds an operator key, and a request says what
		// admin task needs it.
		if p.MaxTier != operatorKeyHolderTier {
			tier := firstNonEmpty(p.MaxTier, "not recorded")
			s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_self_request_refused", nil, "", nil,
				map[string]interface{}{"via": "api", "self": true, "tier": tier},
				false, "refused", "level holds no operator key")
			onboardError(w, http.StatusForbidden, "not_platform_admin",
				fmt.Sprintf(operatorKeyNotPlatformAdminMessage, tier))
			return
		}
		if strings.TrimSpace(body.Reason) == "" {
			onboardError(w, http.StatusBadRequest, "missing_reason", operatorKeyMissingReasonMessage)
			return
		}
		caller, err := s.operatorKeySelfHandle(r.Context(), p)
		if err != nil {
			s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_self_request_refused", nil, "", nil,
				map[string]interface{}{"via": "api", "self": true, "detail": err.Error()},
				false, "refused", "principal not bound to a person")
			onboardError(w, http.StatusForbidden, "unbound_principal", operatorKeyUnboundMessage)
			return
		}
		if bodyHandle != "" && !strings.EqualFold(bodyHandle, caller) {
			// Naming somebody else is the inversion the browser form guards
			// against by never reading a form handle; here the attempt is
			// signal, so it is audited.
			s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_self_request_refused", nil, "", nil,
				map[string]interface{}{"via": "api", "self": true, "handle": bodyHandle, "caller": caller},
				false, "refused", "handle is not the caller's")
			onboardError(w, http.StatusForbidden, "self_only",
				"a non-operator key may only request an operator key for itself ("+caller+") — omit \"handle\" or set it to your own")
			return
		}
		handle = caller
		if strings.TrimSpace(email) == "" {
			email = p.SponsorEmail
		}
	}
	if strings.TrimSpace(handle) == "" {
		onboardError(w, http.StatusBadRequest, "missing_handle", "handle is required")
		return
	}
	// GHSA-2w7v: a handle a revocation tombstone cannot name is refused up
	// front, so every key the console issues can be fully revoked.
	if herr := opkeys.CheckHandle(handle); herr != nil {
		onboardError(w, http.StatusBadRequest, "bad_handle", herr.Error())
		return
	}
	// The filing principal goes on the row; approve then needs a DIFFERENT
	// principal, so one operator key cannot request, approve and revoke a key
	// for an arbitrary handle on its own (GHSA-2w7v).
	r = r.WithContext(onboardstore.WithOperatorKeyRequester(r.Context(), onboardAPIActor(r)))
	env := strings.TrimSpace(body.Env)
	if env == "" {
		env = s.Cfg.NotifyGoatnet
	}
	if p.IsOperator {
		h, e, rerr := s.operatorKeyRosterPerson(r.Context(), handle, email) //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		if rerr != nil {
			s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_request_refused", nil, "", nil,
				map[string]interface{}{"via": "api", "handle": handle, "detail": rerr.Error()},
				false, "refused", "handle names no roster person")
			code := "not_on_roster"
			if errors.Is(rerr, errReservedOperatorHandle) {
				code = "reserved_handle"
			}
			onboardError(w, http.StatusBadRequest, code, rerr.Error())
			return
		}
		handle, email = h, e
	}
	row, err := s.Onboard.RequestOperatorKey(r.Context(), &onboardstore.OperatorKey{ //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		Handle: handle, Email: email, Env: env, Reason: body.Reason,
	})
	if errors.Is(err, onboardstore.ErrOperatorKeyReservedHandle) {
		s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_request_refused", nil, "", nil,
			map[string]interface{}{"via": "api", "handle": handle},
			false, "refused", "reserved service handle")
		onboardError(w, http.StatusBadRequest, "reserved_handle", onboardstore.ErrOperatorKeyReservedHandle.Error())
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "request_error", "could not record the request")
		return
	}
	s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_request", nil, "", nil,
		map[string]interface{}{"id": row.ID, "handle": row.Handle, "env": row.Env, "via": "api",
			"self": !p.IsOperator},
		false, "ok", "")
	onboardJSON(w, http.StatusOK, row)
}

// apiListOperatorKeys lists by status ("" / "all" = every state).
func (s *Server) apiListOperatorKeys(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	status := strings.TrimSpace(r.URL.Query().Get("status"))
	if status == "" {
		status = onboardstore.OperatorKeyRequested
	}
	if status == "all" {
		status = ""
	}
	ks, err := s.Onboard.ListOperatorKeys(r.Context(), status)
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "list_error", "could not list operator keys")
		return
	}
	if ks == nil {
		ks = []*onboardstore.OperatorKey{}
	}
	// The Secret field is json:"-", so a listing can never carry key material
	// however wide the caller sets its filter.
	onboardJSON(w, http.StatusOK, ks)
}

// apiCollectOperatorKey delivers the secret once, to its owner. Open to any
// resolved principal, operator or not — the self-match check below is what
// actually protects this, not who the caller is, and a self-requester (see
// apiRequestOperatorKey) needs this exact door to ever receive the key their
// own request produced.
func (s *Server) apiCollectOperatorKey(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	// H2: collection is bound to the CALLING principal, not to a free query
	// parameter. The store only checks that the row's handle matches the handle
	// it is handed — so without this, any principal could name another
	// person's handle and walk off with their not-yet-delivered op_ key,
	// defeating the F-207 per-person attribution and the two-person rule. The
	// caller's handle is the resolved per-handle principal, or for a person on
	// their own key the roster handle that key binds to (operatorKeySelfHandle,
	// same binding as the request); the query handle, if present, must equal
	// it. Fail closed.
	caller, err := s.operatorKeySelfHandle(r.Context(), onboardPrincipalFromCtx(r.Context()))
	if err != nil {
		onboardError(w, http.StatusForbidden, "unbound_principal", operatorKeyUnboundMessage)
		return
	}
	caller = strings.TrimSpace(caller)
	handle := strings.TrimSpace(r.URL.Query().Get("handle"))
	if handle == "" {
		handle = caller
	}
	if caller == "" || !strings.EqualFold(handle, caller) {
		onboardError(w, http.StatusForbidden, "not_your_key",
			"collection is bound to the calling principal — you may only collect your own operator key")
		return
	}
	// The key is bound to its holder's identity as it is handed over, in
	// the collect's own transaction and only on the caller's own row
	// (GHSA-mxf4-8rv7-774w); one whose holder cannot be resolved is not
	// issued.
	holder, berr := s.collectHolder(r.Context(), caller, "")
	if berr != nil {
		s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_collect_refused", nil, "", nil,
			map[string]interface{}{"id": id, "handle": handle, "via": "api"}, false, "refused", berr.Error())
		onboardError(w, http.StatusForbidden, "holder_unbound", "this key cannot be bound to exactly one identity, so it is not issued: "+berr.Error())
		return
	}
	secret, err := s.Onboard.CollectOperatorKeyAs(r.Context(), id, caller, holder)
	if err == nil {
		forgetOperatorOwner(caller)
	}
	switch {
	case errors.Is(err, onboardstore.ErrOperatorKeyHolderMismatch):
		onboardError(w, http.StatusForbidden, "holder_mismatch", err.Error())
		return
	case errors.Is(err, onboardstore.ErrNotFound):
		onboardError(w, http.StatusNotFound, "not_found", "no such key for that handle")
		return
	case errors.Is(err, onboardstore.ErrOperatorKeyAlreadyDelivered):
		onboardError(w, http.StatusConflict, "already_delivered",
			"that key was already delivered once; revoke it and request a new one")
		return
	case err != nil:
		onboardError(w, http.StatusInternalServerError, "collect_error", "could not collect that key")
		return
	}
	s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_collect", nil, "", nil,
		map[string]interface{}{"id": id, "handle": handle, "via": "api"}, false, "ok", "")
	onboardJSON(w, http.StatusOK, map[string]string{"handle": handle, "key": secret})
}

// operatorKeyDecision routes approve / reject / revoke through one guarded path
// so the API and the console cannot diverge in what a decision DOES — in
// particular, that approve writes the file before reporting success and revoke
// removes it before the record says revoked.
func (s *Server) operatorKeyDecision(w http.ResponseWriter, r *http.Request, action string) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	id := r.PathValue("id")
	// Attribution + the requester ≠ decider rule both key on the resolved
	// per-handle operator principal — the zero browser Actor carries
	// nothing on this realm.
	decider := onboardAPIActor(r)
	actor := Actor{Email: decider}
	dir := s.operatorKeyDir()
	note := strings.TrimSpace(r.URL.Query().Get("note"))

	var row *onboardstore.OperatorKey
	var err error
	var revokeNote string
	switch action {
	case "approve":
		row, err = s.Onboard.ApproveOperatorKey(r.Context(), id, decider, []string{decider})
		if err == nil {
			if werr := dir.Write(row.Handle, row.Secret); werr != nil {
				if _, rerr := s.Onboard.RevokeOperatorKey(r.Context(), id, decider,
					"rolled back: could not write the key file"); rerr != nil {
					onboardError(w, http.StatusInternalServerError, "rollback_error",
						"the key file could not be written and the approval could not be rolled back: "+rerr.Error())
					return
				}
				onboardError(w, http.StatusInternalServerError, "write_error",
					"rolled back — the key file could not be written: "+werr.Error())
				return
			}
		}
	case "reject":
		row, err = s.Onboard.RejectOperatorKey(r.Context(), id, decider, note, []string{decider})
	case "revoke":
		if cur, gerr := s.Onboard.OperatorKey(r.Context(), id); gerr == nil {
			// Tombstone only a key the console issued (GHSA-2w7v review): a
			// withdrawn request must not disable an env-tier key.
			var rerr error
			if revokeNote, rerr = dir.RevokeIssued(cur.Handle, cur.Issued()); rerr != nil {
				// The record must keep saying active: the key still works, and
				// a record claiming otherwise is how a live credential is lost
				// track of.
				onboardError(w, http.StatusInternalServerError, "remove_error",
					"the key is STILL LIVE — its tombstone could not be written or its file removed: "+rerr.Error())
				return
			}
		}
		row, err = s.Onboard.RevokeOperatorKey(r.Context(), id, actor.Email, note)
	default:
		onboardError(w, http.StatusBadRequest, "bad_action", "unknown action")
		return
	}

	switch {
	case errors.Is(err, onboardstore.ErrOperatorKeySelfDecision):
		// The refused attempt is itself signal (#210).
		s.emitAudit(r, actor, "operator_key_self_decision_refused", nil, "", nil,
			map[string]interface{}{"id": id, "action": action, "via": "api"},
			false, "refused", "requester == decider")
		onboardError(w, http.StatusForbidden, "self_decision",
			"this is your own request — a different operator principal must decide it (requester ≠ decider)")
	case errors.Is(err, onboardstore.ErrOperatorKeyUnverifiedDecider):
		// A 403 with its own code: the caller did nothing wrong, the console
		// simply could not establish that two people are involved. A 500 here
		// would read as a fault and invite a retry that cannot succeed.
		s.emitAudit(r, actor, "operator_key_unverified_decider_refused", nil, "", nil,
			map[string]interface{}{"id": id, "action": action, "via": "api", "detail": err.Error()},
			false, "refused", "decider not comparable to requester")
		onboardError(w, http.StatusForbidden, "unverified_decider", err.Error())
	case errors.Is(err, onboardstore.ErrOperatorKeyReservedHandle):
		onboardError(w, http.StatusConflict, "reserved_handle", err.Error())
	case errors.Is(err, onboardstore.ErrNotFound):
		onboardError(w, http.StatusNotFound, "not_found", "no such operator key")
	case errors.Is(err, onboardstore.ErrOperatorKeyWrongState):
		onboardError(w, http.StatusConflict, "wrong_state", err.Error())
	case err != nil:
		onboardError(w, http.StatusInternalServerError, action+"_error", "could not "+action+" the request")
	default:
		s.emitAudit(r, actor, "operator_key_"+action, nil, "", nil,
			map[string]interface{}{"id": id, "handle": row.Handle, "fingerprint": row.Fingerprint,
				"env": row.Env, "via": "api"}, false, "ok", "")
		// Same notification as the console door — a decision made from the
		// machine realm leaves the same person waiting, and more so: nobody
		// is even looking at a queue page. See operatorkeys_notify.go.
		switch action {
		case "approve":
			s.notifyOperatorKeyApproved(r, row)
		case "reject":
			s.notifyOperatorKeyRejected(r, row, note)
		}
		if revokeNote != "" {
			// Said plainly in the response, not only in a log.
			onboardJSON(w, http.StatusOK, map[string]interface{}{
				"id": row.ID, "handle": row.Handle, "status": row.Status,
				"fingerprint": row.Fingerprint, "env": row.Env, "warning": revokeNote})
			return
		}
		onboardJSON(w, http.StatusOK, row)
	}
}

func (s *Server) apiApproveOperatorKey(w http.ResponseWriter, r *http.Request) {
	s.operatorKeyDecision(w, r, "approve")
}
func (s *Server) apiRejectOperatorKey(w http.ResponseWriter, r *http.Request) {
	s.operatorKeyDecision(w, r, "reject")
}
func (s *Server) apiRevokeOperatorKey(w http.ResponseWriter, r *http.Request) {
	s.operatorKeyDecision(w, r, "revoke")
}

// apiReconcileOperatorKeys reports — and optionally repairs — disagreement
// between the record and the directory the portal actually reads.
//
// Dry by default. A sweep that silently deleted grants the first time somebody
// ran it would be the wrong default for a tool whose whole job is telling you
// what is true.
func (s *Server) apiReconcileOperatorKeys(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	apply := r.URL.Query().Get("apply") == "true"
	env := strings.TrimSpace(r.URL.Query().Get("env"))
	res, err := s.operatorKeyDir().Reconcile(r.Context(), s.Onboard, env, apply)
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "reconcile_error", err.Error())
		return
	}
	nonNil := func(v []string) []string {
		if v == nil {
			return []string{}
		}
		return v
	}
	if apply && (len(res.Stale) > 0 || len(res.Untombstoned) > 0) {
		s.emitAudit(r, Actor{Email: onboardAPIActor(r)}, "operator_key_reconcile", nil, "", nil,
			map[string]interface{}{"removed": res.Stale, "tombstoned": res.Untombstoned, "env": env, "via": "api"}, false, "ok", "")
	}
	onboardJSON(w, http.StatusOK, map[string]interface{}{
		"dir":     s.Cfg.OperatorKeyDir,
		"applied": apply,
		// Three cases, three different actions. Collapsing them is what made
		// --apply offer to delete the onboard-runner's key.
		"revoked_but_still_granted": nonNil(res.Stale),
		"granted_but_not_ours":      nonNil(res.Unmanaged),
		"recorded_but_not_granted":  nonNil(res.Missing),
		// GHSA-2w7v: revoked handles with no tombstone; an env-pinned key for
		// them is still accepted until --apply writes the tombstone.
		"revoked_without_tombstone": nonNil(res.Untombstoned),
	})
}
