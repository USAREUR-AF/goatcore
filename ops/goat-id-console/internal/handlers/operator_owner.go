package handlers

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/selfcheck"
)

// The machine realm authorizes on an operator key alone: no session, no
// step-up, no second person. A per-person key is therefore only as current as
// the console's knowledge that its holder is still an admin. Revoking keys at
// offboard covers the doors the console runs, but not an offboard done
// elsewhere, a key whose row the offboard join cannot reach, or a demotion
// that lands between two checks. So every machine request made with a
// per-person operator key is tied to its holder's identity as it is now, and
// refused unless that identity is active and still holds a group that would
// let it hold an operator key (operatorKeyEligible).

// isServiceOperatorPrincipal applies the one service-key rule
// (isServiceKeyHandle) with this console's configured list.
func (s *Server) isServiceOperatorPrincipal(handle string) bool {
	var configured []string
	if s.Cfg != nil {
		configured = s.Cfg.ServiceOperatorHandles
	}
	return isServiceKeyHandle(handle, configured)
}

// operatorOwners caches handle → identity id, so the identity list is not read
// on every machine request. The identity itself (state, groups) is read through
// liveActor and its own cache.
type operatorOwnerEntry struct {
	id string
	at time.Time
}

var operatorOwners = struct {
	mu sync.Mutex
	m  map[string]operatorOwnerEntry
}{m: map[string]operatorOwnerEntry{}}

var errOperatorOwnerUnknown = errors.New("this operator key cannot be tied to a person's identity")

// errReservedOperatorHandle refuses an operator-key request filed under a
// service handle (reported as reserved_handle, the code the store's own
// reserved-handle refusal uses).
var errReservedOperatorHandle = errors.New("reserved service handle")

// errOperatorOwnerAmbiguous is a handle whose evidence names more than one
// person. It fails closed: a key that could be two people's is nobody's.
var errOperatorOwnerAmbiguous = errors.New("this operator key's handle names more than one person")

// operatorOwnerEvidence is what the console knows about the person behind an
// operator handle, strongest first:
//
//  1. holderIDs: the Kratos identity ids recorded on the handle's key rows
//     when they were collected. The active rows' ids are used when any active
//     row has one, otherwise any row's. These are bound to an identity, not
//     to a string, and nothing filed later can change them.
//  2. rowEmails: for rows from before that binding, the emails on the
//     handle's live rows (requested or active).
//  3. rosterEmails: only when the rows carry neither, the emails of EVERY
//     settled onboarding record under the handle (past the admin gate, any
//     later status), plus the handle itself when it is an address. A pending
//     record (proposed, sponsor-approved) never counts: anyone holding an
//     invite can file one.
type operatorOwnerEvidence struct {
	holderIDs    []string
	rowEmails    []string
	rosterEmails []string
}

func (e operatorOwnerEvidence) empty() bool {
	return len(e.holderIDs) == 0 && len(e.rowEmails) == 0 && len(e.rosterEmails) == 0
}

func uniqueFolded(in []string) []string {
	var out []string
	seen := map[string]bool{}
	for _, v := range in {
		v = strings.TrimSpace(v)
		k := strings.ToLower(v)
		if v == "" || seen[k] {
			continue
		}
		seen[k] = true
		out = append(out, v)
	}
	return out
}

func (s *Server) operatorOwnerEvidenceFor(ctx context.Context, handle string) (operatorOwnerEvidence, error) {
	var ev operatorOwnerEvidence
	if s.Onboard == nil {
		if selfcheck.IsEmail(handle) {
			ev.rosterEmails = []string{handle}
		}
		return ev, nil
	}
	rows, err := s.Onboard.OperatorKeyHoldersForHandle(ctx, handle)
	if err != nil {
		return ev, err
	}
	var activeIDs, anyIDs []string
	for _, k := range rows {
		if id := strings.TrimSpace(k.HolderIdentityID); id != "" {
			anyIDs = append(anyIDs, id)
			if k.Status == onboardstore.OperatorKeyActive {
				activeIDs = append(activeIDs, id)
			}
		}
		if k.Status == onboardstore.OperatorKeyRequested || k.Status == onboardstore.OperatorKeyActive {
			ev.rowEmails = append(ev.rowEmails, k.Email)
		}
	}
	if len(activeIDs) > 0 {
		ev.holderIDs = uniqueFolded(activeIDs)
		return ev, nil
	}
	if len(anyIDs) > 0 {
		ev.holderIDs = uniqueFolded(anyIDs)
		return ev, nil
	}
	ev.rowEmails = uniqueFolded(ev.rowEmails)
	if len(ev.rowEmails) > 0 {
		return ev, nil
	}
	ps, err := s.Onboard.ProposalsUnderHandle(ctx, handle)
	if err != nil {
		return ev, err
	}
	var emails []string
	if selfcheck.IsEmail(handle) {
		emails = append(emails, handle)
	}
	for _, p := range ps {
		if onboardstore.ProposalPending(p.Status) || p.Status == onboardstore.ProposalRejected {
			continue
		}
		emails = append(emails, p.Email)
	}
	ev.rosterEmails = uniqueFolded(emails)
	return ev, nil
}

// operatorOwnerEmails is every address the console can tie to an operator
// handle (the string evidence, without the stored holder ids).
func (s *Server) operatorOwnerEmails(ctx context.Context, handle string) ([]string, error) { //nolint:unused // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
	ev, err := s.operatorOwnerEvidenceFor(ctx, handle)
	if err != nil {
		return nil, err
	}
	return append(append([]string(nil), ev.rowEmails...), ev.rosterEmails...), nil
}

// operatorOwnerID resolves the identity id of a per-person operator key's
// holder (operatorOwnerEvidence). A stored holder id is the answer when there
// is exactly one; two are ambiguous. Otherwise the string evidence must match
// exactly one identity. None is unknown; every failure is closed.
func (s *Server) operatorOwnerID(ctx context.Context, cli *kratos.Client, handle string) (string, error) {
	now := time.Now()
	operatorOwners.mu.Lock()
	e, ok := operatorOwners.m[handle]
	operatorOwners.mu.Unlock()
	if ok && now.Sub(e.at) <= liveActorTTL {
		return e.id, nil
	}
	ev, err := s.operatorOwnerEvidenceFor(ctx, handle)
	if err != nil {
		return "", err
	}
	found := ""
	switch {
	case len(ev.holderIDs) > 1:
		return "", errOperatorOwnerAmbiguous
	case len(ev.holderIDs) == 1:
		found = ev.holderIDs[0]
	default:
		emails := ev.rowEmails
		if len(emails) == 0 {
			emails = ev.rosterEmails
		}
		if len(emails) == 0 {
			return "", errOperatorOwnerUnknown
		}
		ids, err := cli.ListIdentities(ctx)
		if err != nil {
			return "", fmt.Errorf("list identities: %w", err)
		}
		for i := range ids {
			if !selfcheck.AnySame(identityAddresses(&ids[i]), emails) {
				continue
			}
			if found != "" && found != ids[i].ID {
				return "", errOperatorOwnerAmbiguous
			}
			found = ids[i].ID
		}
		if found == "" {
			return "", errOperatorOwnerUnknown
		}
	}
	operatorOwners.mu.Lock()
	if len(operatorOwners.m) > 4096 {
		operatorOwners.m = map[string]operatorOwnerEntry{}
	}
	operatorOwners.m[handle] = operatorOwnerEntry{id: found, at: now}
	operatorOwners.mu.Unlock()
	return found, nil
}

// operatorOwnerGate refuses a machine request made with a per-person operator
// key whose holder is no longer an active admin. It returns true when it has
// written the refusal. Service keys, participant (portal) principals, a
// console with LiveIdentityCheck off and DevBypassAuth are not checked.
func (s *Server) operatorOwnerGate(w http.ResponseWriter, r *http.Request, pr onboardPrincipal) bool {
	if !pr.IsOperator || s.Cfg == nil || s.Cfg.DevBypassAuth || !s.Cfg.LiveIdentityCheck {
		return false
	}
	handle := strings.TrimSpace(pr.Handle)
	if s.isServiceOperatorPrincipal(handle) {
		return false
	}
	refuse := func(status int, reason string) bool {
		s.emitAudit(r, Actor{Email: handle}, "operator_key_owner_refused", nil, "", nil,
			map[string]interface{}{"handle": handle, "via": "api"}, false, "refused", reason)
		code := "forbidden"
		if status == http.StatusServiceUnavailable {
			code = "not_configured"
		}
		onboardError(w, status, code, reason)
		return true
	}
	cli := s.Kratos[s.Cfg.StepUpSSOIdP]
	if cli == nil {
		return refuse(http.StatusServiceUnavailable, fmt.Sprintf("sign-in realm %q is unavailable, so this operator key's holder cannot be checked", s.Cfg.StepUpSSOIdP))
	}
	id, err := s.operatorOwnerID(r.Context(), cli, handle)
	if errors.Is(err, errOperatorOwnerUnknown) {
		return refuse(http.StatusForbidden, "forbidden: "+errOperatorOwnerUnknown.Error()+" (no onboarding record or key row names an identity for "+handle+")")
	}
	if errors.Is(err, errOperatorOwnerAmbiguous) {
		return refuse(http.StatusForbidden, "forbidden: "+errOperatorOwnerAmbiguous.Error()+" ("+handle+"); an admin must settle which onboarding record holds it")
	}
	if err != nil {
		return refuse(http.StatusServiceUnavailable, "this operator key's holder could not be checked: "+err.Error())
	}
	live, status, err := s.liveActor(r.Context(), Actor{Email: handle, IdentityID: id})
	if err != nil {
		if status == 0 {
			status = http.StatusForbidden
		}
		return refuse(status, "operator key refused: "+err.Error())
	}
	if !s.operatorKeyEligible(live) {
		return refuse(http.StatusForbidden, "forbidden: this operator key's holder no longer holds an admin group")
	}
	return false
}

// forgetIdentity drops an identity from the live-identity caches, so a state
// or group change the console itself just made reaches that person's open
// session and keys on their next request rather than within liveActorTTL.
func forgetIdentity(id string) {
	id = strings.TrimSpace(id)
	if id == "" {
		return
	}
	liveActors.mu.Lock()
	delete(liveActors.m, id)
	liveActors.mu.Unlock()
	operatorOwners.mu.Lock()
	for h, e := range operatorOwners.m {
		if e.id == id {
			delete(operatorOwners.m, h)
		}
	}
	operatorOwners.mu.Unlock()
}

// afterIdentityDeleted revokes a deleted identity's operator keys and drops it
// from the caches. target is the identity as read before the delete, so the
// join to its keys still works.
func (s *Server) afterIdentityDeleted(r *http.Request, actor Actor, target *kratos.Identity) error {
	if target != nil {
		forgetIdentity(target.ID)
	}
	_, err := s.revokeOperatorKeysAtOffboard(r, actor, "delete_identity", target, "", "")
	return err
}

// afterGroupChange re-reads the identity a group change just landed on and,
// when it no longer holds any group that lets it hold an operator key,
// revokes its operator keys in the same operation. It also drops the identity
// from the caches so the new groups take effect at once.
func (s *Server) afterGroupChange(r *http.Request, actor Actor, idpKey, id string) error {
	forgetIdentity(id)
	cli := s.Kratos[idpKey]
	if cli == nil {
		return fmt.Errorf("realm %q unavailable", idpKey)
	}
	ident, err := cli.GetIdentity(r.Context(), id)
	if err != nil {
		return fmt.Errorf("re-read the identity: %w", err)
	}
	if s.operatorKeyEligible(Actor{Groups: ident.Groups()}) {
		return nil
	}
	_, err = s.revokeOperatorKeysAtOffboard(r, actor, "group_change", ident, "", "")
	return err
}

// operatorKeyRosterPerson binds an operator-realm key request to a person on
// the onboarding roster: the handle (or an address) must name a live
// onboarding record, the row is filed under that record's handle, and its
// email is the record's. A key that matches no person could not be found by
// the offboard join or tied to an identity by the machine-realm gate, so it is
// never created. A body email that is not the record's is refused rather than
// recorded.
func (s *Server) operatorKeyRosterPerson(ctx context.Context, handle, email string) (string, string, error) {
	handle, email = strings.TrimSpace(handle), strings.TrimSpace(email)
	if s.isServiceOperatorPrincipal(handle) || isServiceSponsorLabel(strings.ToLower(handle)) {
		return "", "", fmt.Errorf("%w: %q is a service handle; an operator key is filed for a person", errReservedOperatorHandle, handle)
	}
	var recHandle, recEmail string
	if selfcheck.IsEmail(handle) {
		p, ok, err := s.Onboard.ActiveProposalForEmail(ctx, handle)
		if err != nil {
			return "", "", fmt.Errorf("could not look up the onboarding record: %w", err)
		}
		if !ok || onboardstore.ProposalPending(p.Status) {
			return "", "", fmt.Errorf("%q names no person on the onboarding roster; an operator key is filed under a roster handle so it can be tied to its holder", handle)
		}
		handle = p.Handle
	}
	// The roster person is the one every settled record under the handle
	// names. A pending record (proposed, sponsor-approved) is not the
	// roster: anyone holding an invite can file one. Two settled records
	// naming different people make the handle ambiguous, and no key is
	// filed under it until an admin settles it.
	ps, err := s.Onboard.ProposalsUnderHandle(ctx, handle)
	if err != nil {
		return "", "", fmt.Errorf("could not look up the onboarding record: %w", err)
	}
	for _, p := range ps {
		if !onboardstore.ProposalNonTerminal(p.Status) || onboardstore.ProposalPending(p.Status) {
			continue
		}
		pe := strings.TrimSpace(p.Email)
		if pe == "" {
			continue
		}
		if recEmail != "" && !selfcheck.Same(recEmail, pe) && !strings.EqualFold(recEmail, pe) {
			return "", "", fmt.Errorf("%q is held by more than one onboarding record (%s and %s); an admin must settle which one before a key is filed under it", handle, recEmail, pe)
		}
		if recEmail == "" {
			recHandle, recEmail = p.Handle, pe
		}
	}
	if strings.TrimSpace(recHandle) == "" || strings.TrimSpace(recEmail) == "" {
		return "", "", fmt.Errorf("%q names no person on the onboarding roster; an operator key is filed under a roster handle so it can be tied to its holder", handle)
	}
	if email != "" && !selfcheck.Same(email, recEmail) && !strings.EqualFold(email, recEmail) {
		return "", "", fmt.Errorf("email %q is not the onboarding record's address for %q", email, recHandle)
	}
	return recHandle, recEmail, nil
}

// reactivatorUnprovenOther reports whether a machine principal asking to
// reactivate an identity is a per-person key the console cannot tie to any
// person. The self-reactivation refusal compares the key's person with the
// identity; a key that names nobody makes that compare vacuous, so it is
// refused whether or not the live machine-realm gate is on. A store failure
// also refuses.
func (s *Server) reactivatorUnprovenOther(r *http.Request, pr onboardPrincipal) bool {
	if !pr.IsOperator || s.isServiceOperatorPrincipal(pr.Handle) {
		return false
	}
	ev, err := s.operatorOwnerEvidenceFor(r.Context(), strings.TrimSpace(pr.Handle))
	return err != nil || ev.empty()
}

// collectHolder is the Kratos identity a key collected by this caller is to
// be bound to: the signed-in session's identity on the browser door, or on the
// machine door the one identity the caller's own handle resolves to
// (operatorOwnerID; ambiguous or unknown refuses, so the key is not issued).
// It writes nothing: the store binds it inside the collect transaction, and
// only once the row is shown to be the caller's (CollectOperatorKeyAs), so a
// collect of somebody else's row id can neither bind nor lock that row.
// Without the live identity check (no sign-in realm to resolve against) it
// returns "" and nothing is bound; the gate is off as well.
func (s *Server) collectHolder(ctx context.Context, handle, sessionIdentityID string) (string, error) {
	if s.Onboard == nil || s.Cfg == nil || !s.Cfg.LiveIdentityCheck || s.Cfg.DevBypassAuth {
		return "", nil
	}
	if id := strings.TrimSpace(sessionIdentityID); id != "" {
		return id, nil
	}
	cli := s.Kratos[s.Cfg.StepUpSSOIdP]
	if cli == nil {
		return "", fmt.Errorf("sign-in realm %q is unavailable, so the key's holder cannot be bound", s.Cfg.StepUpSSOIdP)
	}
	return s.operatorOwnerID(ctx, cli, strings.TrimSpace(handle))
}

// forgetOperatorOwner drops a handle's cached owner after its key's holder
// was bound.
func forgetOperatorOwner(handle string) {
	operatorOwners.mu.Lock()
	delete(operatorOwners.m, strings.TrimSpace(handle))
	operatorOwners.mu.Unlock()
}
