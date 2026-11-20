package handlers

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/opkeys"
)

// Operator-key browser realm — the human half of "self-service request,
// admin-gated approval" (docs/design/operator-action-audit.md).
//
//	GET/POST /operator-key             — any signed-in person, their OWN key
//	GET      /operator-key/collect/{id} — the single delivery of the secret
//	GET      /onboarding/operator-keys — ADMINS ONLY, the approval queue
//	POST     /operator-keys/{id}/{action}
//
// The split is the same one SSH keys use, for the same reason: the console is
// reachable with an ordinary web session, so requesting must never authorize.
// What differs is the tail. An SSH key still needs an installer run against
// hosts, so approved is genuinely not access. An operator key is live as soon
// as the file lands and the portal reloads, so approval IS the grant — and the
// wording must not borrow the SSH page's "not yet" or people will sit waiting
// for a step that does not exist.
//
// Because approval IS the grant, two rules bind the queue (#210): it sits
// behind adminGate (not the sponsor-admitting onboardGate the rest of
// /onboarding takes), and the store refuses any approve/reject whose decider
// is the row's own requester — two humans per credential, on both doors.

// operatorKeyEligible reports whether this person may hold an operator key at
// all.
//
// An operator key is not a bigger version of the vp_ key every onboarded person
// already has: it authorizes the Portal's whole admin API AND this console's
// machine API for every org, slot and person on the goatnet. So the people who
// may hold one are exactly the people who may already act as admins here, and
// the predicate is the matrix (ADR 1073) rather than a group literal — the same
// question adminGate asks, asked about the requester.
//
// /operator-key shows a request form only to these people. Checking only the
// decider is not enough: an admin could otherwise approve a developer's request
// whose reason describes the vp_ key's job, such as logging in to the portal.
func (s *Server) operatorKeyEligible(actor Actor) bool {
	if s.Cfg.DevBypassAuth {
		return true
	}
	return s.Permissions.AnyGroupGrants(actor.Groups, "id-admin-console", "admin")
}

// operatorKeyIneligibleMsg names the credential the reader actually wants. The
// commonest reason for asking is Portal sign-in, which the person's own vp_ key
// already does — so a refusal that only says "no" sends them to an admin to ask
// again for the same wrong thing.
const operatorKeyIneligibleMsg = "An operator key is an admin credential for the whole goatnet, and your account holds no admin group, " +
	"so this page cannot request one for you. If you are trying to sign in to the Portal, the key you want is your own " +
	"scoped portal API key (vp_…), which you collect once from the fabric-credential page — not an operator key. " +
	"If you need admin rights, ask an admin to change your access level first."

// operatorKeyNoReasonMsg — the reason is what the approving admin decides on,
// so it is required.
const operatorKeyNoReasonMsg = "Say what you need the key for. The admin deciding this reads that line, and a request with nothing in it " +
	"cannot be told apart from one that should be refused."

// handleOperatorKeySelf renders the requester's own view.
func (s *Server) handleOperatorKeySelf(w http.ResponseWriter, r *http.Request) {
	s.renderOperatorKeySelf(w, r, nil)
}

// handleOperatorKeyRequest records a request for the SIGNED-IN person.
//
// The handle comes from the session, never the form — the same rule the SSH
// page enforces. A form-supplied handle would let anyone with a console session
// park a request against somebody else's name and have an admin approve a key
// that then attributes THEIR portal actions to that person, which inverts the
// entire point of an attributable key.
func (s *Server) handleOperatorKeyRequest(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, http.StatusForbidden, err.Error())
		return
	}
	actor := actorFromCtx(r.Context())
	// Eligibility BEFORE the handle lookup: a person who may not hold a key at
	// all should read why, not "you have no onboarding record" — which sounds
	// like a missing step and sends them to an admin to have it finished.
	if !s.operatorKeyEligible(actor) {
		s.emitAudit(r, actor, "operator_key_request", nil, "", nil,
			map[string]interface{}{"env": s.Cfg.NotifyGoatnet, "via": "console-self"},
			false, "refused", "requester holds no console-admin group")
		s.renderError(w, r, http.StatusForbidden, operatorKeyIneligibleMsg)
		return
	}
	handle, herr := s.sshKeyHandleFor(r, actor.Email)
	if herr != nil {
		s.renderError(w, r, http.StatusForbidden, herr.Error())
		return
	}
	reason := strings.TrimSpace(r.FormValue("reason"))
	if reason == "" {
		s.renderOperatorKeySelf(w, r, &flash{Level: "error", Message: operatorKeyNoReasonMsg})
		return
	}
	if herr := opkeys.CheckHandle(handle); herr != nil { // GHSA-2w7v
		s.renderOperatorKeySelf(w, r, &flash{Level: "error", Message: "your handle cannot hold an operator key: " + herr.Error()})
		return
	}
	r = r.WithContext(onboardstore.WithOperatorKeyRequester(r.Context(), actor.Email)) // GHSA-2w7v
	row, err := s.Onboard.RequestOperatorKey(r.Context(), &onboardstore.OperatorKey{ //nolint:contextcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
		Handle: handle,
		Email:  actor.Email,
		Env:    s.Cfg.NotifyGoatnet,
		Reason: reason,
	})
	if errors.Is(err, onboardstore.ErrOperatorKeyReservedHandle) {
		s.emitAudit(r, actor, "operator_key_request", nil, "", nil,
			map[string]interface{}{"handle": handle, "env": s.Cfg.NotifyGoatnet, "via": "console-self"},
			false, "refused", "reserved service handle")
		s.renderOperatorKeySelf(w, r, &flash{Level: "error",
			Message: "Your handle (" + handle + ") is a reserved service label, so it cannot hold an operator key. Ask an admin to give you your own handle."})
		return
	}
	if err != nil {
		s.renderOperatorKeySelf(w, r, &flash{Level: "error", Message: "could not record the request"})
		return
	}
	s.emitAudit(r, actor, "operator_key_request", nil, "", nil,
		map[string]interface{}{"handle": row.Handle, "id": row.ID, "env": row.Env, "via": "console-self"},
		false, "ok", "")
	s.renderOperatorKeySelf(w, r, &flash{Level: "success",
		Message: "Requested. It grants nothing until an admin approves it; once they do, the key appears here once for you to copy."})
}

// handleOperatorKeyCollect hands the secret over exactly once.
//
// Rendered as its own page rather than inline in a list so the person sees the
// value in a context that says plainly it will not be shown again. The store
// clears it on read, so a refresh genuinely cannot recover it — which is why
// the page says so before they navigate away rather than after.
func (s *Server) handleOperatorKeyCollect(w http.ResponseWriter, r *http.Request) {
	actor := actorFromCtx(r.Context())
	handle, herr := s.sshKeyHandleFor(r, actor.Email)
	if herr != nil {
		s.renderError(w, r, http.StatusForbidden, herr.Error())
		return
	}
	id := r.PathValue("id")
	// Bound to the signed-in identity as it is shown, in the collect's own
	// transaction and only on the collector's own row (GHSA-mxf4-8rv7-774w).
	holder, berr := s.collectHolder(r.Context(), handle, actor.IdentityID)
	if berr != nil {
		s.emitAudit(r, actor, "operator_key_collect_refused", nil, "", nil,
			map[string]interface{}{"id": id, "handle": handle, "via": "console-self"}, false, "refused", berr.Error())
		s.renderOperatorKeySelf(w, r, &flash{Level: "error", Message: "this key cannot be issued to you: " + berr.Error()})
		return
	}
	secret, err := s.Onboard.CollectOperatorKeyAs(r.Context(), id, handle, holder)
	if err == nil {
		forgetOperatorOwner(handle)
	}
	switch {
	case errors.Is(err, onboardstore.ErrOperatorKeyHolderMismatch):
		s.renderOperatorKeySelf(w, r, &flash{Level: "error", Message: "this key cannot be issued to you: " + err.Error()})
		return
	case errors.Is(err, onboardstore.ErrNotFound):
		// Same response whether the row is absent or belongs to somebody else:
		// distinguishing them would turn this into an oracle for whose keys
		// exist.
		s.renderOperatorKeySelf(w, r, &flash{Level: "error", Message: "no such key of yours"})
		return
	case errors.Is(err, onboardstore.ErrOperatorKeyAlreadyDelivered):
		s.renderOperatorKeySelf(w, r, &flash{Level: "error",
			Message: "That key was already shown once and cannot be shown again. If you lost it, revoke it and request a new one — a key nobody holds is worse than no key."})
		return
	case err != nil:
		s.renderOperatorKeySelf(w, r, &flash{Level: "error", Message: "could not collect that key"})
		return
	}
	s.emitAudit(r, actor, "operator_key_collect", nil, "", nil,
		map[string]interface{}{"id": id, "handle": handle, "via": "console-self"}, false, "ok", "")
	s.render(w, r, "operator_keys", &pageVars{
		Title:             "Your operator key",
		OperatorKeySelf:   true,
		OperatorKeyHandle: handle,
		OperatorKeySecret: secret,
	})
}

// handleOperatorKeysQueue is the admin approval queue.
func (s *Server) handleOperatorKeysQueue(w http.ResponseWriter, r *http.Request) {
	s.renderOperatorKeyQueue(w, r, nil)
}

// operatorKeyDeciderIDs is every identity form THIS session is known by: the
// oauth2-proxy email, the current addresses of the identity the session's
// subject names (so an email changed since sign-in still counts), and the
// handle each address resolves to (best-effort — many admins predate the
// onboarding store and resolve to none, which is why the store REFUSES rather
// than waves through a pair it cannot compare). It feeds the store's
// requester ≠ decider rule AND the queue's own-row rendering, so what the
// page disables and what the store permits are the same fact (#210).
//
// An identity that cannot be read back is an error; the decision path
// refuses on it and the queue page falls back to the session email and its
// handle.
func (s *Server) operatorKeyDeciderIDs(r *http.Request, actor Actor) ([]string, error) {
	forms, err := s.actorForms(r.Context(), actor)
	if err != nil {
		ids := []string{actor.Email}
		if handle, herr := s.sshKeyHandleFor(r, actor.Email); herr == nil {
			ids = append(ids, handle)
		}
		return ids, err
	}
	return forms, nil
}

// operatorKeySelfDecisionMsg is the refusal wording, with the single-admin
// break-glass pointer IN the refusal rather than only in the design doc — the
// moment of refusal is exactly where a sole admin needs the detour (#210
// review addendum 2).
const operatorKeySelfDecisionMsg = "This is your own request — a different admin must decide it. " +
	"Sole admin on this goatnet? Use the break-glass path: an API decision under a different " +
	"operator principal (e.g. the onboard-runner's key, via `goat admin operator-key`) — " +
	"see docs/design/operator-action-audit.md."

// operatorKeyUnverifiedMsg is the OTHER refusal: not "this is yours" but "no
// one can tell whether this is yours". It happens when the session resolves to
// no handle (an admin with no onboarding record) and the request records no
// email — two identities in different dimensions, whose non-match proves
// nothing. Naming the cause matters, because the fix is an ordinary one:
// decide it from the machine door, where the principal is always a handle.
const operatorKeyUnverifiedMsg = "Refused: this session cannot be shown to be a different person from " +
	"the requester. Your login resolves to no handle and this request records no email, so there is " +
	"nothing to compare. Decide it with `goat admin operator-key` (the machine door always resolves a " +
	"handle), or finish the requester's onboarding record so the request carries an email."

// handleOperatorKeyDecision is approve / reject / revoke.
//
// Approve and revoke both touch the on-host directory the portal reads. If that
// write fails the decision is ROLLED BACK rather than left recorded, because a
// row saying active with no file is a person holding a credential the fabric
// refuses — and a row saying revoked with a file still present is worse: access
// the record says was taken away.
func (s *Server) handleOperatorKeyDecision(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, http.StatusForbidden, err.Error())
		return
	}
	id := r.PathValue("id")
	action := r.PathValue("action")
	actor := actorFromCtx(r.Context())
	dir := s.operatorKeyDir()
	deciderIDs, derr := s.operatorKeyDeciderIDs(r, actor)
	if derr != nil && (action == "approve" || action == "reject") {
		s.renderOperatorKeyQueueStatus(w, r,
			&flash{Level: "error", Message: "Refused: " + derr.Error()}, http.StatusServiceUnavailable)
		return
	}

	var row *onboardstore.OperatorKey
	var err error
	var revokeNote string
	switch action {
	case "approve":
		row, err = s.Onboard.ApproveOperatorKey(r.Context(), id, actor.Email, deciderIDs)
		if err == nil {
			if werr := dir.Write(row.Handle, row.Secret); werr != nil {
				// Undo, then report. Leaving it approved would tell the person
				// they have a key the portal will never accept.
				if _, rerr := s.Onboard.RevokeOperatorKey(r.Context(), id, actor.Email,
					"rolled back: could not write the key file"); rerr != nil {
					s.renderOperatorKeyQueue(w, r, &flash{Level: "error",
						Message: "the key file could not be written and the approval could not be rolled back: " + rerr.Error()})
					return
				}
				s.renderOperatorKeyQueue(w, r, &flash{Level: "error",
					Message: "approved nothing — the key file could not be written, so the request was rolled back: " + werr.Error()})
				return
			}
		}
	case "reject":
		row, err = s.Onboard.RejectOperatorKey(r.Context(), id, actor.Email, r.FormValue("note"), deciderIDs)
	case "revoke":
		// Tombstone the handle and remove the file FIRST (GHSA-2w7v: the tombstone
		// also refuses an env-pinned key for the handle). If that fails, the record must keep
		// saying active, because the key still works and a record that claims
		// otherwise is how a live credential goes unnoticed.
		if cur, gerr := s.Onboard.OperatorKey(r.Context(), id); gerr == nil {
			// Tombstone only a key the console issued (GHSA-2w7v review).
			var rerr error
			if revokeNote, rerr = dir.RevokeIssued(cur.Handle, cur.Issued()); rerr != nil {
				s.renderOperatorKeyQueue(w, r, &flash{Level: "error",
					Message: "the key is STILL LIVE — its tombstone could not be written or its file removed: " + rerr.Error()})
				return
			}
		}
		row, err = s.Onboard.RevokeOperatorKey(r.Context(), id, actor.Email, r.FormValue("note"))
	default:
		s.renderError(w, r, http.StatusBadRequest, "unknown action")
		return
	}

	switch {
	case errors.Is(err, onboardstore.ErrOperatorKeySelfDecision):
		// An attempt is signal: audit it even though nothing changed (#210).
		s.emitAudit(r, actor, "operator_key_self_decision_refused", nil, "", nil,
			map[string]interface{}{"id": id, "action": action, "via": "console"},
			false, "refused", "requester == decider")
		s.renderOperatorKeyQueueStatus(w, r,
			&flash{Level: "error", Message: operatorKeySelfDecisionMsg}, http.StatusForbidden)
	case errors.Is(err, onboardstore.ErrOperatorKeyUnverifiedDecider):
		// Also a refusal, also 403 — but a DIFFERENT reason, and one an
		// operator can clear. Audited under its own name so the two never
		// blur in the record: one is somebody trying, the other is the
		// console unable to tell.
		s.emitAudit(r, actor, "operator_key_unverified_decider_refused", nil, "", nil,
			map[string]interface{}{"id": id, "action": action, "via": "console", "detail": err.Error()},
			false, "refused", "decider not comparable to requester")
		s.renderOperatorKeyQueueStatus(w, r,
			&flash{Level: "error", Message: operatorKeyUnverifiedMsg}, http.StatusForbidden)
	case errors.Is(err, onboardstore.ErrOperatorKeyReservedHandle):
		s.renderOperatorKeyQueue(w, r, &flash{Level: "error",
			Message: "cannot approve: the request is filed under a reserved service label; reject it and have the person request under their own handle"})
	case errors.Is(err, onboardstore.ErrNotFound):
		s.renderOperatorKeyQueue(w, r, &flash{Level: "error", Message: "that request no longer exists"})
	case errors.Is(err, onboardstore.ErrOperatorKeyWrongState):
		cur := "in another state"
		if k, gerr := s.Onboard.OperatorKey(r.Context(), id); gerr == nil {
			cur = "currently " + k.Status
		}
		s.renderOperatorKeyQueue(w, r, &flash{Level: "error", Message: "cannot " + action + " a request that is " + cur})
	case err != nil:
		s.renderOperatorKeyQueue(w, r, &flash{Level: "error", Message: "could not " + action + " the request"})
	default:
		s.emitAudit(r, actor, "operator_key_"+action, nil, "", nil,
			map[string]interface{}{"id": id, "handle": row.Handle, "fingerprint": row.Fingerprint,
				"env": row.Env, "via": "console"}, false, "ok", "")
		// Tell the requester. Nothing else does: an approver sees their own
		// decision land on this queue, so when two admins approve each other
		// BOTH end up waiting on a signal that never comes. Best-effort and after
		// the decision is recorded — see operatorkeys_notify.go.
		switch action {
		case "approve":
			s.notifyOperatorKeyApproved(r, row)
		case "reject":
			s.notifyOperatorKeyRejected(r, row, r.FormValue("note"))
		}
		msg := action + "ed " + row.Handle
		switch action {
		case "approve":
			// Unlike an SSH key, this IS the grant. Say the delay is the
			// portal's reload rather than another step somebody must take.
			msg = "Approved " + row.Handle + " (" + row.Fingerprint +
				"). Live once the portal re-reads its key directory — seconds, not another step. " +
				row.Handle + " collects the key from their own page."
		case "revoke":
			msg = "Revoked " + row.Handle + ". The file is gone; the portal stops accepting it on its next reload."
			if revokeNote != "" {
				s.renderOperatorKeyQueue(w, r, &flash{Level: "error", Message: "Revoked " + row.Handle + ", but " + revokeNote})
				return
			}
		}
		s.renderOperatorKeyQueue(w, r, &flash{Level: "success", Message: msg})
	}
}

// humanAge renders how long ago a unix timestamp was, coarsely. Coarse on
// purpose: the reader is deciding "chase this" vs "revoke this", and hours
// versus days is the whole of that judgement.
func humanAge(unix int64) string {
	if unix == 0 {
		return "unknown"
	}
	d := time.Since(time.Unix(unix, 0))
	switch {
	case d < time.Hour:
		return "under an hour"
	case d < 24*time.Hour:
		return fmt.Sprintf("%dh", int(d.Hours()))
	default:
		return fmt.Sprintf("%dd", int(d.Hours()/24))
	}
}

// operatorKeyDir is the on-host directory the portal reads. Unset means the
// console records requests but grants nothing — visible in the queue rather
// than silent, because an approval that goes nowhere must not look successful.
func (s *Server) operatorKeyDir() opkeys.Dir {
	return opkeys.Dir{Path: s.Cfg.OperatorKeyDir}
}

func (s *Server) renderOperatorKeySelf(w http.ResponseWriter, r *http.Request, f *flash) {
	actor := actorFromCtx(r.Context())
	if strings.TrimSpace(actor.Email) == "" {
		s.renderError(w, r, http.StatusUnauthorized, "no signed-in identity on this request")
		return
	}
	eligible := s.operatorKeyEligible(actor)
	if !eligible {
		// Render, do not refuse. This page is where a person arrives asking
		// "where is my key" — the useful answer names the credential they do
		// have, and a 403 with no page says only that they were turned away.
		s.render(w, r, "operator_keys", &pageVars{
			Title:                "Your operator key",
			OperatorKeySelf:      true,
			OperatorKeyEligible:  false,
			OperatorKeyIneligMsg: operatorKeyIneligibleMsg,
			Flashes:              flashes(f),
		})
		return
	}
	handle, err := s.sshKeyHandleFor(r, actor.Email)
	if err != nil {
		s.renderError(w, r, http.StatusForbidden, err.Error())
		return
	}
	mine, lerr := s.Onboard.LiveOperatorKeyFor(r.Context(), handle, s.Cfg.NotifyGoatnet)
	if lerr != nil {
		s.renderError(w, r, http.StatusInternalServerError, "could not read your operator key")
		return
	}
	vars := &pageVars{
		Title:                "Your operator key",
		OperatorKeySelf:      true,
		OperatorKeyHandle:    handle,
		OperatorKeyEligible:  eligible,
		OperatorKeyIneligMsg: operatorKeyIneligibleMsg,
		Flashes:              flashes(f),
	}
	if mine != nil {
		vars.OperatorKeys = []*onboardstore.OperatorKey{mine}
	}
	s.render(w, r, "operator_keys", vars)
}

// operatorKeyRow decorates a queue row with WHY this viewer may not decide it,
// when they may not. Such rows stay VISIBLE — a vanished row reads as a bug to
// the person who filed it — but render plain text where approve/reject would
// be. Both flags come from Store.OperatorKeyDecisionCheck, the same call the
// decision handler makes, so the page cannot offer a button the store will
// refuse (#210).
type operatorKeyRow struct {
	*onboardstore.OperatorKey
	// Own — this viewer IS the requester.
	Own bool
	// Unverifiable — viewer and requester are not comparable, so a two-person
	// decision cannot be proven either way and the store refuses it.
	Unverifiable bool
	// LegacyRequest — filed before the filer was recorded (GHSA-2w7v); it
	// cannot be decided, only withdrawn and requested again.
	LegacyRequest bool
	// Tier — the requester's console-admin groups, as the queue's own answer to
	// "should this person hold an admin credential at all". Empty means
	// the login carries no admin group, which on a pending row is the single
	// most decision-relevant fact on it and used to be invisible here.
	Tier string
}

// operatorKeyTier renders a requester's admin standing for the queue. It reads
// the login groups live rather than a stored copy: a row filed before a
// promotion (or after a demotion) must show what the person is NOW, since that
// is what the approving admin is deciding about.
func (s *Server) operatorKeyTier(ctx context.Context, email string) string {
	email = strings.TrimSpace(email)
	if email == "" {
		return ""
	}
	cli := s.Kratos[tenantIdPKey]
	if cli == nil {
		return ""
	}
	ids, err := cli.ListIdentities(ctx)
	if err != nil {
		return ""
	}
	for i := range ids {
		if !strings.EqualFold(strings.TrimSpace(ids[i].Email()), email) {
			continue
		}
		var admin []string
		for _, g := range ids[i].Groups() {
			if s.Permissions.AnyGroupGrants([]string{g}, "id-admin-console", "admin") {
				admin = append(admin, g)
			}
		}
		return strings.Join(admin, ", ")
	}
	return ""
}

func (s *Server) renderOperatorKeyQueue(w http.ResponseWriter, r *http.Request, f *flash) {
	s.renderOperatorKeyQueueStatus(w, r, f, 0)
}

// renderOperatorKeyQueueStatus renders the queue with a non-200 status when
// code is set — a refused self-decision answers 403 while still landing the
// admin back on the queue with the reason, not on a dead-end error page.
func (s *Server) renderOperatorKeyQueueStatus(w http.ResponseWriter, r *http.Request, f *flash, code int) {
	status := strings.TrimSpace(r.URL.Query().Get("status"))
	if status == "" {
		status = onboardstore.OperatorKeyRequested
	}
	if status == "all" {
		status = ""
	}
	ks, err := s.Onboard.ListOperatorKeys(r.Context(), status)
	if err != nil {
		s.renderError(w, r, http.StatusInternalServerError, "could not list operator keys")
		return
	}
	viewerIDs, _ := s.operatorKeyDeciderIDs(r, actorFromCtx(r.Context())) //nolint:errcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
	rows := make([]operatorKeyRow, 0, len(ks))
	for _, k := range ks {
		row := operatorKeyRow{OperatorKey: k}
		// Only pending rows carry decide controls, so only they need the
		// check — which also keeps the owner lookup off the "all" view.
		if k.Status == onboardstore.OperatorKeyRequested {
			switch err := s.Onboard.OperatorKeyDecisionCheck(r.Context(), k, viewerIDs); {
			case errors.Is(err, onboardstore.ErrOperatorKeySelfDecision):
				row.Own = true
			case errors.Is(err, onboardstore.ErrOperatorKeyLegacyRequest):
				row.Unverifiable, row.LegacyRequest = true, true
			case err != nil:
				row.Unverifiable = true
			}
			row.Tier = s.operatorKeyTier(r.Context(), k.Email)
		}
		rows = append(rows, row)
	}
	vars := &pageVars{
		Title:              "Operator keys",
		OperatorKeyRows:    rows,
		OperatorKeyStatus:  status,
		OperatorKeyDirPath: s.Cfg.OperatorKeyDir,
		Flashes:            flashes(f),
	}
	// Drift is worth showing an admin unprompted: a grant no row explains is
	// somebody with access nobody decided to give them.
	if dir := s.operatorKeyDir(); dir.Managed() {
		if res, derr := dir.Reconcile(r.Context(), s.Onboard, "", false); derr == nil {
			vars.OperatorKeyStale = res.Stale
			vars.OperatorKeyUnmanaged = res.Unmanaged
			vars.OperatorKeyMissing = res.Missing
		}
	}
	// Live-but-never-collected: the person has no credential in their hands
	// and no reason to think one is waiting, so such a key can sit unnoticed.
	// It is surfaced here for that reason.
	// One hour of grace so a just-approved row does not flash up as a problem.
	if ks, uerr := s.Onboard.UncollectedOperatorKeys(r.Context(), time.Now().Add(-time.Hour).Unix()); uerr == nil {
		for _, k := range ks {
			vars.OperatorKeyUncollected = append(vars.OperatorKeyUncollected, struct {
				Handle string
				Age    string
			}{Handle: k.Handle, Age: humanAge(k.DecidedAt)})
		}
	}
	if code != 0 && code != http.StatusOK {
		// render never calls WriteHeader, so setting the headers it would set
		// and committing the status here keeps the page intact under a 403.
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Referrer-Policy", "no-referrer")
		w.WriteHeader(code)
	}
	s.render(w, r, "operator_keys", vars)
}
