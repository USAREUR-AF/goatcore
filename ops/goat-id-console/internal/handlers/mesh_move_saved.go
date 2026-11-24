package handlers

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"slices"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

const meshMoveReviewRetention = 7 * 24 * time.Hour
const meshMoveReviewAuditAction = "mesh_move_declaration_review"

type meshMoveSavedResult struct {
	From        string               `json:"from_group"`
	To          string               `json:"to_group"`
	Declaration *meshMoveDeclaration `json:"declaration,omitempty"`
	Error       string               `json:"error,omitempty"`
}

func (r meshMoveSavedResult) hasReviewedDeclaration(peerID string) bool {
	if !r.Declaration.bindsPeer(peerID) || r.Declaration.Effective == nil {
		return false
	}
	return meshMoveGroupRefusal(r.From, r.To) == "" &&
		meshMoveGroupRefusal(r.Declaration.Effective.Add...) == "" &&
		meshMoveGroupRefusal(r.Declaration.Effective.Remove...) == ""
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

func writeSavedMeshMoveReview(w http.ResponseWriter, row onboardstore.MeshMoveReview, withholdReason string) {
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
	if !result.hasReviewedDeclaration(row.PeerID) {
		http.Error(w, "The saved declaration lacks a current warrant review. Preview the move again.", http.StatusConflict)
		return
	}
	state := "declaration_ready"
	if withholdReason != "" {
		state = "saved_review"
		result.Declaration = nil
	}
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(struct {
		meshMoveSavedResult
		ConfirmationState string `json:"confirmation_state"`
		OperationID       string `json:"operation_id"`
		ExpiresAt         string `json:"expires_at"`
		Notice            string `json:"notice,omitempty"`
	}{result, state, row.ID, row.ExpiresAt.UTC().Format(time.RFC3339), withholdReason}); err != nil {
		slog.Warn("could not write saved mesh move response", "operation_id", row.ID, "error", err)
	}
}

func (s *Server) getSavedMeshMoveReview(ctx context.Context, id string) (onboardstore.MeshMoveReview, error) {
	if s.Onboard == nil {
		return onboardstore.MeshMoveReview{}, errors.New("move review store unavailable")
	}
	return s.Onboard.GetMeshMoveReview(ctx, id, time.Now())
}

func (s *Server) saveMeshMoveReview(w http.ResponseWriter, r *http.Request, binding meshMoveBinding, id string, status int, result meshMoveSavedResult) {
	payload, err := json.Marshal(result)
	if err != nil {
		http.Error(w, "The console could not encode the move result. Try confirming again.", http.StatusServiceUnavailable)
		return
	}
	now := time.Now().UTC()
	identity, _ := r.Context().Value(browserTargetKey{}).(*kratos.Identity)
	details := map[string]interface{}{
		"operation_id": id, "site": binding.Site, "peer_id": binding.Peer,
		"from_group": binding.From, "to_group": binding.To,
	}
	outcome, message := "ok", ""
	if status == http.StatusOK {
		details["state"] = "declaration_ready"
	} else {
		outcome, message = "refused", "Declaration preparation failed."
		if status >= http.StatusInternalServerError {
			outcome = "error"
		}
		details["state"] = "preparation_failed"
		details["http_status"] = status
	}
	event := newAuditEvent(r, actorFromCtx(r.Context()), meshMoveReviewAuditAction, identity, binding.Realm, nil, details, false, outcome, message)
	event.TS = now
	row, created, err := s.Onboard.SaveMeshMoveReview(r.Context(), onboardstore.MeshMoveReview{
		ID: id, ActorID: binding.ActorID, Realm: binding.Realm, Identity: binding.Identity,
		Site: binding.Site, PeerID: binding.Peer, Status: status, Result: string(payload),
		SavedAt: now, ExpiresAt: now.Add(meshMoveReviewRetention),
	}, event)
	if err != nil {
		slog.WarnContext(r.Context(), "could not save mesh move review and audit event", "operation_id", id, "error", err)
		http.Error(w, "The console could not save the move result. Try confirming again.", http.StatusServiceUnavailable)
		return
	}
	if created {
		// Mirror the committed record to existing console audit sinks.
		s.Audit.Emit(r.Context(), event)
	}
	writeSavedMeshMoveReview(w, row, result.Error)
}

func (s *Server) recentAuditEvents(ctx context.Context, limit int) ([]audit.Event, bool) {
	events := s.Audit.Recent(limit)
	if s.Onboard == nil {
		return events, false
	}
	moveEvents, err := s.Onboard.RecentMeshMoveAudit(ctx, limit)
	if err != nil {
		slog.WarnContext(ctx, "could not load durable mesh move audit records", "error", err)
		return events, true
	}
	operations := make(map[string]bool, len(moveEvents))
	for _, event := range moveEvents {
		if id, ok := event.After["operation_id"].(string); ok {
			operations[id] = true
		}
	}
	for _, event := range events {
		id, _ := event.After["operation_id"].(string)
		if event.Action != meshMoveReviewAuditAction || !operations[id] {
			moveEvents = append(moveEvents, event)
		}
	}
	slices.SortStableFunc(moveEvents, func(a, b audit.Event) int { return b.TS.Compare(a.TS) })
	return moveEvents[:min(len(moveEvents), limit)], false
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
		if row.Status == http.StatusOK && !view.Result.hasReviewedDeclaration(row.PeerID) {
			continue
		}
		view.JSON = row.Result
		user.UnavailableMoveReviews[row.PeerID] = view
	}
	for _, peer := range user.Access.Peers {
		review, ok := user.UnavailableMoveReviews[peer.ID]
		if !ok || meshMovePeerRefusal(peer.Groups) != "" {
			continue
		}
		if review.Result.Declaration != nil && (user.Authority.Mode != authorityEnforce ||
			!user.Access.MeshAvailable || user.Access.KnownHandlesError != "" ||
			!slices.Contains(user.MoveGroups, review.Result.From) ||
			!slices.Contains(user.MoveGroups, review.Result.To) ||
			(!slices.Contains(peer.Groups, review.Result.From) && !slices.Contains(peer.Groups, review.Result.To))) {
			continue
		}
		user.MoveReviews[peer.ID] = review
		delete(user.UnavailableMoveReviews, peer.ID)
	}
}
