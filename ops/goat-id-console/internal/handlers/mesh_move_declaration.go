package handlers

import (
	"io/fs"
	"net/http"
	"path"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

type meshMoveDeclaration struct {
	Fragment         string                   `json:"fragment"`
	ReplacesExisting bool                     `json:"replaces_existing"`
	CreatesFragment  bool                     `json:"creates_fragment"`
	YAML             string                   `json:"yaml"`
	Effective        *meshMoveEffectiveReview `json:"effective"`
}

type meshMoveEffectiveReview struct {
	Add          []string `json:"add"`
	Remove       []string `json:"remove"`
	ExpiryBefore *string  `json:"expiry_before"`
	ExpiryAfter  *string  `json:"expiry_after"`
}

type meshMoveDeclarationRequest struct {
	dir   string
	owner string
}

// A configured planner can be older than the console. Verify the actual YAML
// before handing it off, including when displaying a previously saved review.
func (d *meshMoveDeclaration) bindsPeer(id string) bool {
	if d == nil || len(d.YAML) > 16384 || id == "" {
		return false
	}
	type entry struct {
		PeerID yaml.Node `yaml:"peer_id"`
	}
	var entries []entry
	if d.CreatesFragment {
		var fragment struct {
			Entries []entry `yaml:"entries"`
		}
		if err := yaml.Unmarshal([]byte(d.YAML), &fragment); err != nil {
			return false
		}
		entries = fragment.Entries
	} else if err := yaml.Unmarshal([]byte(d.YAML), &entries); err != nil {
		return false
	}
	return len(entries) == 1 && entries[0].PeerID.Tag == "!!str" && entries[0].PeerID.Value == id
}

func (s *Server) prepareMeshMoveDeclaration(r *http.Request, binding meshMoveBinding) (meshMovePlan, *meshMovePreviewFailure) {
	scope := strings.TrimSpace(s.Cfg.MembershipScopePath)
	owner := sessionOperatorActor(actorFromCtx(r.Context()))
	if owner == "" {
		return meshMovePlan{}, &meshMovePreviewFailure{"missing_actor", "Your session could not be attributed. Sign in again before preparing a declaration.", http.StatusForbidden}
	}
	if !filepath.IsAbs(scope) || filepath.Base(scope) != "_scope.yaml" {
		return meshMovePlan{}, &meshMovePreviewFailure{"declaration_scope", "The scope’s declaration is unavailable. Ask a network administrator to check the console’s declaration configuration.", http.StatusServiceUnavailable}
	}
	plan, failure := s.planMeshMove(r.Context(), binding.Site, binding.Peer, binding.From, binding.To, &meshMoveDeclarationRequest{
		dir: filepath.Dir(scope), owner: owner,
	})
	if failure != nil {
		return meshMovePlan{}, failure
	}
	if len(plan.Movers) == 0 {
		return plan, nil
	}
	proposal := plan.Declaration
	if proposal == nil || (proposal.ReplacesExisting && proposal.CreatesFragment) ||
		!fs.ValidPath(proposal.Fragment) || path.Ext(proposal.Fragment) != ".yaml" ||
		strings.HasPrefix(path.Base(proposal.Fragment), "_") {
		return meshMovePlan{}, &meshMovePreviewFailure{"invalid_declaration", "The console could not verify where to put the proposed declaration. Ask a network administrator to check the scope declaration.", http.StatusBadGateway}
	}
	if !proposal.bindsPeer(binding.Peer) {
		return meshMovePlan{}, &meshMovePreviewFailure{"unbound_declaration", "The proposal does not bind the selected device. Ask a network administrator to check the move planner version.", http.StatusBadGateway}
	}
	if proposal.Effective == nil || proposal.Effective.Add == nil || proposal.Effective.Remove == nil {
		return meshMovePlan{}, &meshMovePreviewFailure{"missing_effective_plan", "The planner did not return the effective warrant changes. Ask a network administrator to check the move planner version.", http.StatusBadGateway}
	}
	for _, groups := range [][]string{proposal.Effective.Add, proposal.Effective.Remove} {
		if refusal := meshMoveGroupRefusal(groups...); refusal != "" {
			return meshMovePlan{}, &meshMovePreviewFailure{"managed_lane", refusal, http.StatusUnprocessableEntity}
		}
	}
	proposal.Fragment = "ops/warrant/membership/" + binding.Site + "/" + proposal.Fragment
	return plan, nil
}
