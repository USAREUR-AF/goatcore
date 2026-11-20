package handlers

import (
	"context"
	"fmt"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/selfcheck"
)

// personForms is every name the console can establish for one person, for
// matching their operator keys at offboard: the identity's addresses, the
// email and handle the caller names, and the email and handle of the newest
// onboarding record under any of those, in ANY status. Offboarded records
// count: they are exactly the ones an offboard leaves behind, and the active
// lookups exclude them.
func (s *Server) personForms(ctx context.Context, ident *kratos.Identity, email, handle string) ([]string, error) {
	forms := append(identityAddresses(ident), email, handle)
	if s.Onboard == nil {
		return trimForms(forms), nil
	}
	seed := trimForms(forms)
	for _, f := range seed {
		p, ok, err := s.Onboard.LatestProposalFor(ctx, f)
		if err != nil {
			return nil, fmt.Errorf("look up the onboarding record for %s: %w", f, err)
		}
		if ok {
			forms = append(forms, p.Email, p.Handle)
		}
	}
	return trimForms(forms), nil
}

func trimForms(in []string) []string {
	out := make([]string, 0, len(in))
	for _, v := range in {
		if v = strings.TrimSpace(v); v != "" {
			out = append(out, v)
		}
	}
	return out
}

// revokeOperatorKeysAtOffboard revokes every requested or active operator key
// that belongs to the person being offboarded, in the same operation as the
// offboard itself. An operator key authorizes the machine realm on its own (no
// session, no step-up), so a person whose identity is switched off but whose
// key still works keeps a door that can switch the identity back on.
//
// A key belongs to the person when its handle or email matches any of their
// forms (personForms) under selfcheck's fold. Key rows record no Kratos
// identity id, so the onboarding record is the join. For an active key the
// on-host file the portal reads is removed first, then the row is revoked, the
// same order the queue's revoke uses: a file that outlives its row is a live
// credential nobody can see. A requested row that was never issued has no
// file and only its record is revoked. Each revocation is audited.
//
// This is the console side only. Keys pinned in the console's or the portal's
// environment (ONBOARD_OPERATOR_API_KEYS and the portal's own list) are not
// rows here and are not touched (GHSA-2w7v-pcf5-h8mw).
func (s *Server) revokeOperatorKeysAtOffboard(r *http.Request, actor Actor, via string, ident *kratos.Identity, email, handle string) ([]string, error) {
	if s.Onboard == nil {
		return nil, nil
	}
	ctx := r.Context()
	forms, err := s.personForms(ctx, ident, email, handle)
	if err != nil {
		return nil, err
	}
	if len(forms) == 0 {
		return nil, nil
	}
	rows, err := s.Onboard.ListOperatorKeys(ctx, "")
	if err != nil {
		return nil, fmt.Errorf("list operator keys: %w", err)
	}
	by := firstNonEmpty(strings.TrimSpace(actor.Email), "offboard")
	var revoked []string
	for _, k := range rows {
		if k.Status != onboardstore.OperatorKeyRequested && k.Status != onboardstore.OperatorKeyActive {
			continue
		}
		owner := trimForms([]string{k.Handle, k.Email})
		match := selfcheck.AnySame(owner, forms)
		for _, o := range owner {
			for _, f := range forms {
				if strings.EqualFold(o, f) {
					match = true
				}
			}
		}
		if !match {
			continue
		}
		before := map[string]interface{}{"id": k.ID, "handle": k.Handle, "status": k.Status}
		// Only an ISSUED key (approved, so it carries a fingerprint) has a
		// file of ours to remove. A requested-but-never-issued row is
		// revoked in the record only: its handle may also name a key this
		// console never issued, and that file is not ours to touch.
		if k.Status == onboardstore.OperatorKeyActive && strings.TrimSpace(k.Fingerprint) != "" {
			if rerr := s.operatorKeyDir().Remove(k.Handle); rerr != nil {
				s.emitAudit(r, actor, "operator_key_revoked_at_offboard", nil, "", before,
					map[string]interface{}{"via": via}, false, "error", rerr.Error())
				return revoked, fmt.Errorf("operator key %s for %s is STILL LIVE: %w", k.ID, k.Handle, rerr)
			}
		}
		row, rerr := s.Onboard.RevokeOperatorKey(ctx, k.ID, by, "revoked at offboard ("+via+")")
		if rerr != nil {
			s.emitAudit(r, actor, "operator_key_revoked_at_offboard", nil, "", before,
				map[string]interface{}{"via": via}, false, "error", rerr.Error())
			return revoked, fmt.Errorf("revoke operator key %s for %s: %w", k.ID, k.Handle, rerr)
		}
		s.emitAudit(r, actor, "operator_key_revoked_at_offboard", nil, "", before,
			map[string]interface{}{"id": row.ID, "handle": row.Handle, "status": row.Status,
				"fingerprint": row.Fingerprint, "via": via}, false, "ok", "")
		revoked = append(revoked, k.ID)
	}
	return revoked, nil
}

// revokeKeysForProposal revokes the operator keys of the person proposal p is
// about, for an offboard recorded through the decide flow. The actor is the
// session actor when there is one, otherwise the decided-by value (an
// operator key's handle on the machine door).
func (s *Server) revokeKeysForProposal(r *http.Request, decidedBy string, p *onboardstore.Proposal, via string) *decideError {
	actor := actorFromCtx(r.Context())
	if strings.TrimSpace(actor.Email) == "" {
		actor = Actor{Email: decidedBy}
	}
	if _, err := s.revokeOperatorKeysAtOffboard(r, actor, via, nil, p.Email, p.Handle); err != nil {
		return &decideError{status: http.StatusInternalServerError,
			msg: "recorded as offboarded, but the person's operator keys could not all be revoked: " + err.Error()}
	}
	return nil
}

// heldResponse buffers a handler's response so the caller can act on its
// outcome before anything reaches the client.
type heldResponse struct {
	header http.Header
	status int
	body   []byte
}

func (h *heldResponse) Header() http.Header { return h.header }
func (h *heldResponse) WriteHeader(c int) {
	if h.status == 0 {
		h.status = c
	}
}
func (h *heldResponse) Write(b []byte) (int, error) {
	if h.status == 0 {
		h.status = http.StatusOK
	}
	h.body = append(h.body, b...)
	return len(b), nil
}

func (h *heldResponse) flush(w http.ResponseWriter) {
	for k, v := range h.header {
		w.Header()[k] = v
	}
	if h.status == 0 {
		h.status = http.StatusOK
	}
	w.WriteHeader(h.status)
	_, _ = w.Write(h.body) //nolint:errcheck // behavior kept as reviewed for disclosure; lint cleanup goatcore#1028
}

// offboardCutRevokingKeys runs the queued cut (handleOffboardCut) and, when
// the cut was actually queued (step-up passed and the row moved), revokes the
// person's operator keys before the admin sees the result. The runner's cut
// does not touch operator keys, so without this the key would outlive the
// offboard. A revoke failure replaces the "queued" redirect with an error that
// names the key still live; the queue entry stands.
func (s *Server) offboardCutRevokingKeys(w http.ResponseWriter, r *http.Request, id string, actor Actor) {
	held := &heldResponse{header: http.Header{}}
	s.handleOffboardCut(held, r, id, actor)
	if held.status == http.StatusSeeOther &&
		strings.Contains(held.header.Get("Location"), "flash="+flashOffboardCutQueued) {
		if p, err := s.Onboard.GetProposal(r.Context(), id); err != nil {
			s.renderError(w, r, http.StatusInternalServerError,
				"the cut is queued, but the person's operator keys could not be checked: "+err.Error())
			return
		} else if _, kerr := s.revokeOperatorKeysAtOffboard(r, actor, "offboard_cut", nil, p.Email, p.Handle); kerr != nil {
			s.renderError(w, r, http.StatusInternalServerError,
				"the cut is queued, but the person's operator keys could not all be revoked: "+kerr.Error())
			return
		}
	}
	held.flush(w)
}
