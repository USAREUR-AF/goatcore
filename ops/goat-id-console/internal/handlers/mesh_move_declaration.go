package handlers

import (
	"io/fs"
	"net/http"
	"path"
	"path/filepath"
	"strings"
	"time"
)

type meshMoveDeclaration struct {
	Fragment         string `json:"fragment"`
	ReplacesExisting bool   `json:"replaces_existing"`
	CreatesFragment  bool   `json:"creates_fragment"`
	YAML             string `json:"yaml"`
}

type meshMoveDeclarationRequest struct {
	dir   string
	owner string
}

func (s *Server) prepareMeshMoveDeclaration(r *http.Request, binding meshMoveBinding, reviewed meshMovePlan) (meshMovePlan, *meshMovePreviewFailure) {
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
	plan.Authority = s.membershipAuthority().Mode
	hash, err := meshMovePlanHash(plan)
	reviewedHash, reviewedErr := meshMovePlanHash(reviewed)
	if err != nil || reviewedErr != nil {
		return meshMovePlan{}, &meshMovePreviewFailure{"invalid_plan", "The console could not verify the move plan. Refresh the page and retry.", http.StatusBadGateway}
	}
	if hash != reviewedHash {
		plan.Declaration = nil
		plan.ConfirmationState = "changed"
		plan.ConfirmationToken = s.issueMeshMoveConfirmation(binding, plan, time.Now())
		return plan, nil
	}
	proposal := plan.Declaration
	if proposal == nil || (proposal.ReplacesExisting && proposal.CreatesFragment) || strings.TrimSpace(proposal.YAML) == "" || len(proposal.YAML) > 16384 ||
		!fs.ValidPath(proposal.Fragment) || path.Ext(proposal.Fragment) != ".yaml" ||
		strings.HasPrefix(path.Base(proposal.Fragment), "_") {
		return meshMovePlan{}, &meshMovePreviewFailure{"invalid_declaration", "The console could not verify where to put the proposed declaration. Ask a network administrator to check the scope declaration.", http.StatusBadGateway}
	}
	proposal.Fragment = "ops/warrant/membership/" + binding.Site + "/" + proposal.Fragment
	plan.ConfirmationState = "declaration_ready"
	return plan, nil
}
