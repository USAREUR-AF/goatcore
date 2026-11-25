package handlers

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"net/url"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/meshstate"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

type meshMovePlan struct {
	From   string `json:"from_group"`
	To     string `json:"to_group"`
	Movers []struct {
		ID          string   `json:"peer_id"`
		Name        string   `json:"name"`
		OtherGroups []string `json:"other_groups"`
	} `json:"movers"`
	AlreadyAtDestination int                  `json:"already_at_destination"`
	Authority            string               `json:"authority"`
	ConfirmationToken    string               `json:"confirmation_token,omitempty"`
	ConfirmationState    string               `json:"confirmation_state,omitempty"`
	Declaration          *meshMoveDeclaration `json:"declaration,omitempty"`
}

const meshMoveQuarantineRefusal = "Use the security quarantine hold and release procedure for this device."

func isMeshQuarantineGroup(group string) bool {
	scope, found := strings.CutSuffix(strings.ToUpper(strings.TrimSpace(group)), "-QUARANTINED")
	return found && scope != ""
}

func hasMeshQuarantineGroup(groups []string) bool {
	return slices.ContainsFunc(groups, isMeshQuarantineGroup)
}

const meshMoveLifecycleRefusal = "Use the dormancy hold and reactivation procedure for this device."
const meshMoveP2PRefusal = "Use the P2P graduation and revocation procedure."

func isMeshProbationGroup(group string) bool {
	scope, role, found := strings.Cut(strings.ToUpper(strings.TrimSpace(group)), "-PROBATION-")
	return found && scope != "" && role != ""
}

func isMeshP2PGroup(group string) bool {
	scope, found := strings.CutSuffix(strings.ToUpper(strings.TrimSpace(group)), "-P2P")
	return found && scope != ""
}

const meshMovePrivilegedRefusal = "Use the dedicated platform-admin, break-glass, or external-service workflow for this group."

func isMeshPrivilegedGroup(group string) bool {
	group = strings.ToUpper(strings.TrimSpace(group))
	for _, suffix := range []string{"-PLATFORM-ADMIN", "-BREAKGLASS", "-EXTERNAL-INFRA"} {
		if scope, found := strings.CutSuffix(group, suffix); found && scope != "" {
			return true
		}
	}
	return false
}

func meshMoveGroupRefusal(groups ...string) string {
	switch {
	case hasMeshQuarantineGroup(groups):
		return meshMoveQuarantineRefusal
	case slices.ContainsFunc(groups, isMeshProbationGroup):
		return meshMoveLifecycleRefusal
	case slices.ContainsFunc(groups, isMeshP2PGroup):
		return meshMoveP2PRefusal
	case slices.ContainsFunc(groups, isMeshPrivilegedGroup):
		return meshMovePrivilegedRefusal
	default:
		return ""
	}
}

func meshMovePeerRefusal(groups []string) string {
	switch {
	case hasMeshQuarantineGroup(groups):
		return meshMoveQuarantineRefusal
	case slices.ContainsFunc(groups, isMeshProbationGroup):
		return meshMoveLifecycleRefusal
	default:
		return ""
	}
}

func (s *Server) movePlannerReadURL() string {
	raw := s.Cfg.MovePlannerReadURL
	endpoint, err := url.Parse(raw)
	if err != nil || endpoint.User != nil || endpoint.RawQuery != "" ||
		endpoint.ForceQuery || endpoint.Fragment != "" {
		return ""
	}
	host := strings.ToLower(endpoint.Hostname())
	if endpoint.Scheme == "https" && strings.HasPrefix(host, "mgmt-read.") &&
		(endpoint.Path == "" || endpoint.Path == "/") {
		return raw
	}
	if s.Cfg.DevBypassAuth && endpoint.Scheme == "http" &&
		(host == "127.0.0.1" || host == "::1") {
		return raw
	}
	return ""
}

func (s *Server) movePlannerAvailable() bool {
	return filepath.IsAbs(s.Cfg.MovePlannerBin) && filepath.IsAbs(s.Cfg.MovePlannerProfile) &&
		s.movePlannerReadURL() != "" && strings.TrimSpace(s.Cfg.EnrollWatchToken) != ""
}

func (s *Server) handleMeshMovePreview(w http.ResponseWriter, r *http.Request) {
	s.handleMeshMoveReview(w, r, false)
}

func (s *Server) handleMeshMoveConfirm(w http.ResponseWriter, r *http.Request) {
	s.handleMeshMoveReview(w, r, true)
}

func (s *Server) handleMeshMoveReview(w http.ResponseWriter, r *http.Request, confirm bool) {
	w.Header().Set("Cache-Control", "no-store")
	if !containsString(actorFromCtx(r.Context()).Groups, platformAdminGroup) {
		http.Error(w, "You don’t have permission to review a mesh move.", http.StatusForbidden)
		return
	}
	peerID, from, to := r.PostForm.Get("peer"), r.PostForm.Get("from"), r.PostForm.Get("to")
	if peerID == "" || strings.TrimSpace(from) == "" || strings.TrimSpace(to) == "" ||
		strings.EqualFold(from, to) || strings.EqualFold(from, "All") || strings.EqualFold(to, "All") {
		http.Error(w, "Select a device and two different mesh groups.", http.StatusBadRequest)
		return
	}
	if refusal := meshMoveGroupRefusal(from, to); refusal != "" {
		http.Error(w, refusal, http.StatusUnprocessableEntity)
		return
	}
	identity, _ := r.Context().Value(browserTargetKey{}).(*kratos.Identity)
	binding := meshMoveBinding{
		ActorID: actorFromCtx(r.Context()).IdentityID, ActorEmail: actorFromCtx(r.Context()).Email,
		Realm: idpFromCtx(r.Context()).Key, Identity: r.PathValue("id"),
		Site: strings.TrimSpace(s.Cfg.NotifyGoatnet), Peer: peerID, From: from, To: to,
	}
	var reviewed meshMoveConfirmation
	var operationID string
	var saved *onboardstore.MeshMoveReview
	if confirm {
		var ok bool
		proof := r.PostForm.Get("confirmation")
		reviewed, ok = s.readMeshMoveConfirmation(proof, binding, time.Now())
		if !ok {
			http.Error(w, "This preview is missing, invalid, or expired. Preview the move again before confirming.", http.StatusConflict)
			return
		}
		if reviewed.Authority == authorityEnforce && binding.ActorID != "" {
			operationID = meshMoveOperationID(proof)
			row, err := s.getSavedMeshMoveReview(r.Context(), operationID)
			switch {
			case err == nil:
				if row.Status != http.StatusOK {
					writeSavedMeshMoveReview(w, row, "")
					return
				}
				saved = &row
			case !errors.Is(err, onboardstore.ErrNotFound):
				slog.WarnContext(r.Context(), "could not read mesh move review", "operation_id", operationID, "error", err)
				http.Error(w, "The console could not read the saved move result. Try again.", http.StatusServiceUnavailable)
				return
			}
		}
	}
	fail := func(message string, status int) {
		if saved != nil {
			writeSavedMeshMoveReview(w, *saved, message)
			return
		}
		if operationID == "" {
			http.Error(w, message, status)
			return
		}
		s.saveMeshMoveReview(w, r, binding, operationID, status, meshMoveSavedResult{From: from, To: to, Error: message})
	}
	failPlanner := func(failure *meshMovePreviewFailure) {
		fail(meshMoveFailureMessage(r, failure), failure.status)
	}
	if !s.movePlannerAvailable() {
		fail("Move preview is unavailable. Contact an administrator.", http.StatusServiceUnavailable)
		return
	}
	access, groups, lookupErr := s.resolveAccess(r.Context(), identity, true)
	if access.HandleError != "" {
		fail("Could not verify the device and available groups. Refresh the page and retry.", http.StatusServiceUnavailable)
		return
	}
	if lookupErr != nil {
		failure := moveNetworkUnavailableFailure()
		var statusErr *meshstate.HTTPStatusError
		if errors.As(lookupErr, &statusErr) && (statusErr.Status == http.StatusUnauthorized || statusErr.Status == http.StatusForbidden) {
			failure = moveNetworkCredentialsFailure()
		}
		failPlanner(failure)
		return
	}
	if !access.MeshAvailable || access.KnownHandlesError != "" {
		fail("Could not verify the device and available groups. Refresh the page and retry.", http.StatusServiceUnavailable)
		return
	}
	peerIndex := slices.IndexFunc(access.Peers, func(peer meshstate.PeerView) bool { return peer.ID == peerID })
	if peerIndex < 0 {
		fail("The device is unavailable for this user. Refresh the page and retry.", http.StatusNotFound)
		return
	}
	peerGroups := access.Peers[peerIndex].Groups
	if refusal := meshMovePeerRefusal(peerGroups); refusal != "" {
		fail(refusal, http.StatusUnprocessableEntity)
		return
	}
	if len(groups) == 0 {
		fail("Could not verify the device and available groups. Refresh the page and retry.", http.StatusServiceUnavailable)
		return
	}
	if !slices.Contains(groups, from) || !slices.Contains(groups, to) {
		fail("A selected group is no longer available. Refresh the page and retry.", http.StatusConflict)
		return
	}
	inSource, inDestination := slices.Contains(peerGroups, from), slices.Contains(peerGroups, to)
	if !inSource && !inDestination {
		fail("The device is no longer in the source group. Refresh the page and retry.", http.StatusConflict)
		return
	}

	authority := s.membershipAuthority().Mode
	if saved != nil {
		if authority != authorityEnforce {
			fail("Warrant membership authority could not be verified. Preview the move again.", http.StatusConflict)
			return
		}
		writeSavedMeshMoveReview(w, *saved, "")
		return
	}
	var plan meshMovePlan
	var failure *meshMovePreviewFailure
	if authority == authorityEnforce {
		plan, failure = s.prepareMeshMoveDeclaration(r, binding)
	} else {
		plan, failure = s.planMeshMove(r.Context(), access.Site, peerID, from, to, nil)
	}
	if failure != nil {
		failPlanner(failure)
		return
	}
	if len(plan.Movers) == 0 && plan.AlreadyAtDestination == 0 {
		fail("The device’s group membership changed. Refresh the page and retry.", http.StatusConflict)
		return
	}
	plan.Authority = authority
	planHash, err := meshMovePlanHash(plan)
	if err != nil {
		fail("The console could not verify the move plan. Refresh the page and retry.", http.StatusBadGateway)
		return
	}
	status := http.StatusOK
	switch {
	case confirm && reviewed.PlanHash != planHash:
		status = http.StatusConflict
		plan.ConfirmationToken = s.issueMeshMoveConfirmation(binding, plan, time.Now())
		plan.ConfirmationState = "changed"
	case confirm && plan.Authority == authorityEnforce && plan.Declaration != nil:
		plan.ConfirmationState = "declaration_ready"
		if operationID != "" {
			s.saveMeshMoveReview(w, r, binding, operationID, http.StatusOK, meshMoveSavedResult{From: from, To: to, Declaration: plan.Declaration})
			return
		}
	case confirm:
		plan.ConfirmationState = "confirmed"
	default:
		plan.ConfirmationToken = s.issueMeshMoveConfirmation(binding, plan, time.Now())
	}
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(plan); err != nil {
		slog.WarnContext(r.Context(), "could not write mesh move preview response", "error", err)
	}
}

type meshMovePreviewFailure struct {
	reason  string
	message string
	status  int
}

func meshMoveFailureMessage(r *http.Request, failure *meshMovePreviewFailure) string {
	message := failure.message
	if failure.status >= http.StatusInternalServerError {
		id := requestID(r)
		slog.WarnContext(r.Context(), "mesh move preview failed", "request_id", id, "reason", failure.reason)
		if failure.reason == "network_credentials" {
			message += " Reference: " + id + "."
		} else {
			message += " Request ID: " + id + "."
		}
	}
	return message
}

func moveNetworkCredentialsFailure() *meshMovePreviewFailure {
	return &meshMovePreviewFailure{"network_credentials", "The console couldn’t access NetBird to validate this move. Ask a network administrator to check the console’s NetBird credentials.", http.StatusServiceUnavailable}
}

func moveNetworkUnavailableFailure() *meshMovePreviewFailure {
	return &meshMovePreviewFailure{"network_unavailable", "The network service could not be reached or returned an error. Try again. If the problem continues, contact an administrator.", http.StatusBadGateway}
}

// The planner writes human-readable diagnostics. Recognize its known error
// prefixes and emit fixed messages. Upstream text may include private data.
func classifyMeshMovePlannerError(stderr string) *meshMovePreviewFailure {
	for line := range strings.SplitSeq(stderr, "\n") {
		line = strings.TrimSpace(line)
		line = strings.TrimSpace(strings.TrimPrefix(line, "error:"))
		switch {
		case strings.HasPrefix(line, "DECLARATION PREPARATION REFUSED: warrant cannot apply the selected peer's entry: grant/absent needs a baseline"):
			return &meshMovePreviewFailure{"missing_baseline", "Warrant could not resolve a membership baseline for this device. Ask a network administrator to check its declared baseline or enrollment key.", http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "DECLARATION PREPARATION REFUSED: warrant cannot apply the selected peer's entry: refused (coverage gate)"):
			return &meshMovePreviewFailure{"effective_coverage", "The effective warrant change includes a group that is missing or has no enabled policy. Ask a network administrator to check the baseline and group rules.", http.StatusUnprocessableEntity}
		case line == "DECLARATION PREPARATION REFUSED: the proposed entry has expired":
			return &meshMovePreviewFailure{"expired_entry", "The existing declaration has expired. Review its expiry separately in the configuration repository before preparing this move.", http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "DECLARATION PREPARATION REFUSED: security quarantine"):
			return &meshMovePreviewFailure{"quarantine", meshMoveQuarantineRefusal, http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "DECLARATION PREPARATION REFUSED: dormancy"):
			return &meshMovePreviewFailure{"probation", meshMoveLifecycleRefusal, http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "DECLARATION PREPARATION REFUSED: P2P"):
			return &meshMovePreviewFailure{"p2p", meshMoveP2PRefusal, http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "DECLARATION PREPARATION REFUSED: privileged membership"):
			return &meshMovePreviewFailure{"privileged_group", meshMovePrivilegedRefusal, http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "DECLARATION PREPARATION REFUSED:"):
			return &meshMovePreviewFailure{"declaration_unavailable", "The console could not prepare a declaration for this move. Ask a network administrator to verify the scope’s declaration and group rules.", http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "COVERAGE GATE:"):
			return &meshMovePreviewFailure{"destination_policy", "No enabled network policy covers this destination group. Choose another group or ask a network administrator to review the policy.", http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "REFUSED security-quarantine transition:"):
			return &meshMovePreviewFailure{"quarantine", "This device cannot use this quarantine workflow. Ask a security administrator to review the move.", http.StatusUnprocessableEntity}
		case strings.HasPrefix(line, "no group named "), line == "group/peer not found":
			return &meshMovePreviewFailure{"stale_selection", "A selected device or group is no longer available. Refresh the page and retry.", http.StatusConflict}
		case strings.HasPrefix(line, "--from and --to resolve to the same group"):
			return &meshMovePreviewFailure{"same_group", "Select two different mesh groups.", http.StatusBadRequest}
		case strings.HasPrefix(line, "NetBird mgmt API returned HTTP 401"),
			strings.HasPrefix(line, "NetBird mgmt API returned HTTP 403"),
			strings.HasPrefix(line, "NetBird mgmt API denied this request"),
			strings.HasPrefix(line, "help: NetBird mgmt API denied this request"),
			line == "code: E_NOT_ADMIN":
			return moveNetworkCredentialsFailure()
		case strings.HasPrefix(line, "NetBird mgmt API unreachable:"),
			strings.HasPrefix(line, "NetBird mgmt API returned HTTP "):
			return moveNetworkUnavailableFailure()
		case strings.HasPrefix(line, "NetBird mgmt client not configured"), line == "code: E_PROFILE_MISSING":
			return &meshMovePreviewFailure{"network_access", "The console could not verify network access for the move. Ask a network administrator to check the console’s NetBird access.", http.StatusServiceUnavailable}
		}
	}
	return &meshMovePreviewFailure{"unclassified_planner_error", "Move preview could not be completed. Try again. If the problem continues, contact an administrator with the request ID.", http.StatusBadGateway}
}

func (s *Server) planMeshMove(ctx context.Context, site, peerID, from, to string, declaration *meshMoveDeclarationRequest) (meshMovePlan, *meshMovePreviewFailure) {
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()

	profile := s.Cfg.MovePlannerProfile
	token := strings.TrimSpace(s.Cfg.EnrollWatchToken)
	// Equals-form arguments keep group names and peer IDs in value positions.
	// The invocation performs the CLI's default dry run and uses the console's
	// current scope and read credential.
	args := []string{"--profile-dir=" + profile, "--context=" + site,
		"admin", "members", "move", "--env=" + site, "--planes=mesh", "--json", "--peer-id=" + peerID}
	if declaration != nil {
		args = append(args, "--prepare-declaration="+declaration.dir, "--owner="+declaration.owner)
	}
	args = append(args, "--from="+from, "--to="+to)
	cmd := exec.CommandContext(ctx, s.Cfg.MovePlannerBin, args...) //nolint:gosec // G204: configured executable, fixed read-only verb, validated peer and group values passed as arguments
	cmd.Dir = profile
	cmd.Env = []string{
		"GOAT_NB_API_TOKEN=" + token,
		"GOAT_NB_MGMT_URL=" + s.movePlannerReadURL(),
		"NO_COLOR=1",
	}
	var plan meshMovePlan
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	output, err := cmd.Output()
	if ctx.Err() != nil {
		return plan, &meshMovePreviewFailure{"planner_timeout", "Move preview timed out. Try again.", http.StatusGatewayTimeout}
	}
	if err != nil {
		var exitErr *exec.ExitError
		if !errors.As(err, &exitErr) {
			return plan, &meshMovePreviewFailure{"planner_start", "Could not start the move planner. Contact an administrator.", http.StatusServiceUnavailable}
		}
		return plan, classifyMeshMovePlannerError(stderr.String())
	}
	if json.Unmarshal(output, &plan) != nil || plan.From != from || plan.To != to || plan.Movers == nil ||
		plan.AlreadyAtDestination < 0 || len(plan.Movers)+plan.AlreadyAtDestination > 1 ||
		(len(plan.Movers) == 1 && plan.Movers[0].ID != peerID) {
		return meshMovePlan{}, &meshMovePreviewFailure{"invalid_plan", "The console could not verify the move plan. Refresh the page and retry.", http.StatusBadGateway}
	}
	if declaration == nil {
		plan.Declaration = nil
	}
	if len(plan.Movers) == 1 {
		slices.Sort(plan.Movers[0].OtherGroups)
	}
	return plan, nil
}
