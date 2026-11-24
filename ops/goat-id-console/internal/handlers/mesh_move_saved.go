package handlers

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

const meshMoveReviewRetention = 7 * 24 * time.Hour

type meshMoveSavedResult struct {
	From        string               `json:"from_group"`
	To          string               `json:"to_group"`
	Declaration *meshMoveDeclaration `json:"declaration,omitempty"`
	Error       string               `json:"error,omitempty"`
}

type meshMoveReviewView struct {
	ID        string
	PeerID    string
	Result    meshMoveSavedResult
	ExpiresAt string
	JSON      string
}

func meshMoveOperationID(proof string) string {
	digest := sha256.Sum256([]byte(proof))
	return "move-" + hex.EncodeToString(digest[:16])
}

func writeSavedMeshMoveReview(w http.ResponseWriter, row onboardstore.MeshMoveReview) {
	w.Header().Set("X-Move-Operation-ID", row.ID)
	w.Header().Set("X-Move-Expires-At", row.ExpiresAt.UTC().Format(time.RFC3339))
	var result meshMoveSavedResult
	if err := json.Unmarshal([]byte(row.Result), &result); err != nil {
		http.Error(w, "The saved move result could not be read.", http.StatusServiceUnavailable)
		return
	}
	if row.Status != http.StatusOK {
		http.Error(w, result.Error, row.Status)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(struct {
		meshMoveSavedResult
		ConfirmationState string `json:"confirmation_state"`
		OperationID       string `json:"operation_id"`
		ExpiresAt         string `json:"expires_at"`
	}{result, "declaration_ready", row.ID, row.ExpiresAt.UTC().Format(time.RFC3339)}); err != nil {
		slog.Warn("could not write saved mesh move response", "operation_id", row.ID, "error", err)
	}
}

func (s *Server) replayMeshMoveReview(w http.ResponseWriter, r *http.Request, id string) bool {
	if s.Onboard != nil {
		row, err := s.Onboard.GetMeshMoveReview(r.Context(), id, time.Now())
		if err == nil {
			writeSavedMeshMoveReview(w, row)
			return true
		}
		if errors.Is(err, onboardstore.ErrNotFound) {
			return false
		}
		slog.WarnContext(r.Context(), "could not read mesh move review", "operation_id", id, "error", err)
	}
	http.Error(w, "The console could not read the saved move result. Try again.", http.StatusServiceUnavailable)
	return true
}

func (s *Server) saveMeshMoveReview(w http.ResponseWriter, r *http.Request, binding meshMoveBinding, id string, status int, result meshMoveSavedResult) {
	payload, err := json.Marshal(result)
	if err != nil {
		http.Error(w, "The console could not encode the move result. Try confirming again.", http.StatusServiceUnavailable)
		return
	}
	now := time.Now().UTC()
	row, created, err := s.Onboard.SaveMeshMoveReview(r.Context(), onboardstore.MeshMoveReview{
		ID: id, ActorID: binding.ActorID, Realm: binding.Realm, Identity: binding.Identity,
		Site: binding.Site, PeerID: binding.Peer, Status: status, Result: string(payload),
		SavedAt: now, ExpiresAt: now.Add(meshMoveReviewRetention),
	})
	if err != nil {
		slog.WarnContext(r.Context(), "could not save mesh move review", "operation_id", id, "error", err)
		http.Error(w, "The console could not save the move result. Try confirming again.", http.StatusServiceUnavailable)
		return
	}
	if created {
		identity, _ := r.Context().Value(browserTargetKey{}).(*kratos.Identity)
		details := map[string]interface{}{
			"operation_id": row.ID, "site": row.Site, "peer_id": row.PeerID,
			"from_group": binding.From, "to_group": binding.To,
		}
		outcome, message := "ok", ""
		if row.Status == http.StatusOK {
			details["state"] = "declaration_ready"
		} else {
			outcome, message = "refused", "Declaration preparation failed."
			if row.Status >= http.StatusInternalServerError {
				outcome = "error"
			}
			details["state"] = "preparation_failed"
			details["http_status"] = row.Status
		}
		s.emitAudit(r, actorFromCtx(r.Context()), "mesh_move_declaration_review", identity, row.Realm, nil, details, false, outcome, message)
	}
	writeSavedMeshMoveReview(w, row)
}

func (s *Server) loadMeshMoveReviews(r *http.Request, user *userDetail) {
	actor := actorFromCtx(r.Context())
	if !user.CanMovePeer || actor.IdentityID == "" || s.Onboard == nil {
		return
	}
	rows, err := s.Onboard.ListMeshMoveReviews(r.Context(), actor.IdentityID, idpFromCtx(r.Context()).Key,
		r.PathValue("id"), strings.TrimSpace(s.Cfg.NotifyGoatnet), time.Now())
	if err != nil {
		slog.WarnContext(r.Context(), "could not load mesh move reviews", "error", err)
		user.MoveReviewsUnavailable = true
		return
	}
	user.MoveReviews = make(map[string]meshMoveReviewView, len(rows))
	user.UnavailableMoveReviews = make(map[string]meshMoveReviewView, len(rows))
	for _, row := range rows {
		view := meshMoveReviewView{ID: row.ID, PeerID: row.PeerID, ExpiresAt: row.ExpiresAt.UTC().Format(time.RFC3339)}
		if err := json.Unmarshal([]byte(row.Result), &view.Result); err != nil {
			user.MoveReviewsUnavailable = true
			continue
		}
		view.JSON = row.Result
		user.UnavailableMoveReviews[row.PeerID] = view
	}
	for _, peer := range user.Access.Peers {
		if review, ok := user.UnavailableMoveReviews[peer.ID]; ok {
			user.MoveReviews[peer.ID] = review
			delete(user.UnavailableMoveReviews, peer.ID)
		}
	}
}
