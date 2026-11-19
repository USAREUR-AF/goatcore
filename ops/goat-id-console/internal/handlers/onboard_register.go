package handlers

import (
	"encoding/json"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// registerBody is what the CLI mint path and the roster backfill both POST.
type registerBody struct {
	Handle       string `json:"handle"`
	Email        string `json:"email"`
	Env          string `json:"env"`
	Sponsor      string `json:"sponsor"`
	MintedBy     string `json:"minted_by"`
	ManifestJSON string `json:"manifest_json"`
	Note         string `json:"note"`
	Tier         string `json:"tier"`
}

// apiRegisterMintedProposal records an onboarding performed outside the review
// queue, so the console store covers everybody rather than only the people who
// came through it.
//
// ONE ENDPOINT, TWO CALLERS, on purpose: `goat admin onboard` calls it as the
// last step of a mint, and ops/identity/backfill-console-rows.py calls it for
// the identities already onboarded that way before this existed. A second
// entry point for the backfill would be a third path writing the register, and
// a third path is how the gap opened in the first place.
//
// IT DOES NOT MINT ANYTHING. This is a bookkeeping write about a credential
// that already exists; nothing here touches NetBird, Ory or the fabric. That is
// why it can be safely idempotent — a repeated call records the same fact.
func (s *Server) apiRegisterMintedProposal(w http.ResponseWriter, r *http.Request) {
	if !requireOperatorPrincipal(w, r) {
		return
	}
	var req registerBody
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, 1<<20)).Decode(&req); err != nil {
		onboardError(w, http.StatusBadRequest, "bad_body", "could not parse the request body")
		return
	}
	req.Handle = strings.TrimSpace(req.Handle)
	req.Env = strings.TrimSpace(req.Env)
	if req.Handle == "" || req.Env == "" {
		onboardError(w, http.StatusBadRequest, "missing_fields",
			"handle and env are both required — a row without them cannot be joined to anything")
		return
	}

	actor := onboardAPIActor(r)
	sponsor := strings.TrimSpace(req.Sponsor)
	if sponsor == "" {
		sponsor = actor
	}
	mintedBy := strings.TrimSpace(req.MintedBy)
	if mintedBy == "" {
		mintedBy = actor
	}

	p := &onboardstore.Proposal{
		Sponsor:      sponsor,
		Env:          req.Env,
		Handle:       req.Handle,
		Email:        strings.TrimSpace(req.Email),
		Status:       onboardstore.ProposalOnboarded,
		ManifestJSON: strings.TrimSpace(req.ManifestJSON),
		Note:         strings.TrimSpace(req.Note),
		Tier:         strings.TrimSpace(req.Tier),
		DecidedBy:    &mintedBy,
	}

	created, existing, err := s.Onboard.RegisterMintedProposal(r.Context(), p)
	if rq, refused := asRunnerQueueRefusal7788(err); refused {
		s.emitAudit(r, Actor{Email: actor}, "onboard_register_minted", nil, "", nil,
			map[string]interface{}{"handle": p.Handle, "env": p.Env, "tier": p.Tier}, false, "refused", rq.Msg)
		onboardError(w, http.StatusForbidden, "forbidden", rq.Msg)
		return
	}
	if err != nil {
		onboardError(w, http.StatusInternalServerError, "register_error",
			"could not register the minted proposal")
		return
	}
	if !created {
		// 200, not 409: the caller asked for this person to be recorded and they
		// ARE recorded. A conflict code would make a correct re-run look like a
		// failure and push callers toward retry loops or, worse, toward forcing a
		// duplicate. The body says plainly that nothing was written.
		onboardJSON(w, http.StatusOK, map[string]any{
			"id": existing.ID, "handle": existing.Handle, "status": existing.Status,
			"created": false,
			"detail":  "an entry for this email already exists; nothing was written",
		})
		return
	}

	s.emitAudit(r, Actor{Email: actor}, "onboard_register_minted", nil, "",
		nil,
		map[string]interface{}{"status": onboardstore.ProposalOnboarded,
			"proposal_id": p.ID, "handle": p.Handle, "env": p.Env, "minted_by": mintedBy},
		false, "ok", "")
	onboardJSON(w, http.StatusCreated, map[string]any{
		"id": p.ID, "handle": p.Handle, "status": p.Status, "created": true,
	})
}
