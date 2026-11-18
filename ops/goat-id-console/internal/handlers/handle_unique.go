package handlers

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/selfcheck"
)

// A handle names one person. The operator-key owner join, the offboard
// revoke and the runner's revoke-by-handle all read it that way, so two
// live onboarding records under one handle for two people make every one of
// them ambiguous (GHSA-mxf4-8rv7-774w).

// handleHeldByOther returns a non-terminal onboarding record (any status but
// rejected, failed and offboarded) that already holds handle for a different
// address, or nil. A record for the same address is left to the doors' own
// duplicate-email rules.
func (s *Server) handleHeldByOther(ctx context.Context, handle, email string) (*onboardstore.Proposal, error) {
	handle, email = strings.TrimSpace(handle), strings.TrimSpace(email)
	if s.Onboard == nil || handle == "" {
		return nil, nil
	}
	ps, err := s.Onboard.ProposalsUnderHandle(ctx, handle)
	if err != nil {
		return nil, err
	}
	for i := range ps {
		p := &ps[i]
		if !onboardstore.ProposalNonTerminal(p.Status) {
			continue
		}
		if email != "" && (strings.EqualFold(strings.TrimSpace(p.Email), email) || selfcheck.Same(p.Email, email)) {
			continue
		}
		return p, nil
	}
	return nil, nil
}

// handleTakenMessage is the refusal both intake doors give.
func handleTakenMessage(handle string, p *onboardstore.Proposal) string {
	return "identity.handle " + handle + " is already held by onboarding record " + p.ID +
		" (status " + p.Status + ") for a different address; a handle names one person — choose another, or settle that record first"
}

// withoutSubmittedHandle drops a "handle" key from a candidate's submission.
// The public form renders no handle input; the handle is derived from the
// email (deriveHandle), so a handle in the JSON is something the form never
// asked for, and honoring it let an invitee file under somebody else's
// handle. A submission that is not a JSON object is returned unchanged for
// the decoder to refuse.
func withoutSubmittedHandle(raw json.RawMessage) json.RawMessage {
	if len(raw) == 0 {
		return raw
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(raw, &m); err != nil {
		return raw
	}
	if _, ok := m["handle"]; !ok {
		return raw
	}
	delete(m, "handle")
	out, err := json.Marshal(m)
	if err != nil {
		return raw
	}
	return out
}
