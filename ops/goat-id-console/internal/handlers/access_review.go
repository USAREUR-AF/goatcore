package handlers

import (
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// access_review.go — the read-only access review (ADR 1272 section 5): for
// this goatnet, the accounts that deserve a second look. It reports and never
// removes; the Offboard button on the onboarding page is the only cut.
//
// Five findings, each a fact the console can read:
//
//	idle                   an active login with no live session issued in the
//	                       last N days (Kratos lists live sessions only, so a
//	                       person whose session expired reads as idle)
//	sponsor-left           the account's sponsor has a departure on record
//	inactive-no-departure  the identity service marks the login inactive and
//	                       no departure is recorded for the person
//	departed-still-active  a finished cut is on record and the login is active
//	admin                  the account holds an admin group

const (
	findingIdle           = "idle"
	findingSponsorLeft    = "sponsor-left"
	findingInactiveNoDep  = "inactive-no-departure"
	findingDepartedActive = "departed-still-active"
	findingAdmin          = "admin"

	accessReviewDefaultIdleDays = 90
	accessReviewMaxIdleDays     = 3650
)

// accessReviewRow is one account with at least one finding.
type accessReviewRow struct {
	Handle      string   `json:"handle,omitempty"`
	Email       string   `json:"email"`
	State       string   `json:"state"`
	Groups      []string `json:"groups,omitempty"`
	LastSession string   `json:"last_session,omitempty"` // YYYY-MM-DD; "" when no live session
	Sponsor     string   `json:"sponsor,omitempty"`
	Findings    []string `json:"findings"`
}

// accessReviewVars is the page and JSON payload.
type accessReviewVars struct {
	IdleDays int               `json:"idle_days"`
	Accounts int               `json:"accounts"`
	Counts   map[string]int    `json:"counts"`
	Rows     []accessReviewRow `json:"rows"`
	// SessionsUnread is set when the session list could not be read: the idle
	// finding is then left out rather than reported for everyone.
	SessionsUnread string `json:"sessions_unread,omitempty"`
	Err            string `json:"error,omitempty"`
}

// adminGroup reports whether a group name is an admin group: the two
// platform-admin names, or any group named <PLANE>-ADMIN.
func adminGroup(group string) bool {
	return group == platformAdminGroup || group == legacyAdminGroup || strings.HasSuffix(group, "-ADMIN")
}

// buildAccessReview derives the review from what the identity service and the
// console's own store hold. lastByID nil means the session list was unread.
func buildAccessReview(now time.Time, idleDays int, ids []kratos.Identity,
	lastByID map[string]time.Time, proposals []onboardstore.Proposal) *accessReviewVars {
	out := &accessReviewVars{IdleDays: idleDays, Accounts: len(ids), Counts: map[string]int{}}
	// The newest onboarding row per login email, and who has a departure.
	latest := map[string]onboardstore.Proposal{}
	for _, p := range proposals {
		key := strings.ToLower(strings.TrimSpace(p.Email))
		if prev, ok := latest[key]; !ok || p.CreatedAt > prev.CreatedAt {
			latest[key] = p
		}
	}
	departure := map[string]string{} // lower email or handle -> proposal status
	for _, p := range latest {
		for _, status := range onboardstore.DepartureStatuses {
			if p.Status == status {
				departure[strings.ToLower(strings.TrimSpace(p.Email))] = p.Status
				if h := strings.ToLower(strings.TrimSpace(p.Handle)); h != "" {
					departure[h] = p.Status
				}
			}
		}
	}
	cutoff := now.Add(-time.Duration(idleDays) * 24 * time.Hour)
	for _, id := range ids {
		email := id.Email()
		key := strings.ToLower(strings.TrimSpace(email))
		row := accessReviewRow{Email: email, State: id.State, Groups: id.Groups()}
		p, known := latest[key]
		if known {
			row.Handle, row.Sponsor = p.Handle, p.Sponsor
		}
		active := strings.EqualFold(id.State, "active")
		last, seen := lastByID[id.ID]
		if seen && !last.IsZero() {
			row.LastSession = last.UTC().Format(time.DateOnly)
		}
		if active && lastByID != nil && (!seen || last.Before(cutoff)) && id.CreatedAt.Before(cutoff) {
			row.Findings = append(row.Findings, findingIdle)
		}
		if sponsor := strings.ToLower(strings.TrimSpace(row.Sponsor)); active && sponsor != "" {
			if _, left := departure[sponsor]; left {
				row.Findings = append(row.Findings, findingSponsorLeft)
			}
		}
		status, departed := departure[key]
		if !active && !departed {
			row.Findings = append(row.Findings, findingInactiveNoDep)
		}
		if active && departed && status == onboardstore.ProposalOffboarded {
			row.Findings = append(row.Findings, findingDepartedActive)
		}
		for _, group := range row.Groups {
			if active && adminGroup(group) {
				row.Findings = append(row.Findings, findingAdmin)
				break
			}
		}
		if len(row.Findings) == 0 {
			continue
		}
		for _, finding := range row.Findings {
			out.Counts[finding]++
		}
		out.Rows = append(out.Rows, row)
	}
	sort.SliceStable(out.Rows, func(i, j int) bool {
		if len(out.Rows[i].Findings) != len(out.Rows[j].Findings) {
			return len(out.Rows[i].Findings) > len(out.Rows[j].Findings)
		}
		return out.Rows[i].Email < out.Rows[j].Email
	})
	return out
}

// handleAccessReview renders GET /access-review[?idle_days=N][&format=json].
// It reads; it changes nothing.
func (s *Server) handleAccessReview(w http.ResponseWriter, r *http.Request) {
	idleDays := accessReviewDefaultIdleDays
	if raw := strings.TrimSpace(r.URL.Query().Get("idle_days")); raw != "" {
		n, err := strconv.Atoi(raw)
		if err != nil || n < 1 || n > accessReviewMaxIdleDays {
			s.renderError(w, r, http.StatusBadRequest, "idle_days is a number of days from 1 to 3650")
			return
		}
		idleDays = n
	}
	idp := idpFromCtx(r.Context())
	cli := s.Kratos[idp.Key]
	ids, err := cli.ListIdentities(r.Context())
	if err != nil {
		s.renderError(w, r, http.StatusBadGateway, "the identity service could not be read, so there is no review: "+err.Error())
		return
	}
	lastByID, lastErr := cli.LastSessionByIdentity(r.Context())
	if lastErr != nil {
		lastByID = nil
	}
	var proposals []onboardstore.Proposal
	if s.Onboard != nil {
		if proposals, err = s.Onboard.ListProposals(r.Context(), ""); err != nil {
			s.renderError(w, r, http.StatusInternalServerError, "the onboarding store could not be read, so there is no review: "+err.Error())
			return
		}
	}
	review := buildAccessReview(time.Now(), idleDays, ids, lastByID, proposals)
	if lastErr != nil {
		review.SessionsUnread = "the session list could not be read, so idle accounts are not reported: " + lastErr.Error()
	}
	if r.URL.Query().Get("format") == "json" {
		onboardJSON(w, http.StatusOK, review)
		return
	}
	s.render(w, r, "access_review", &pageVars{Title: "Access review — " + idp.Label, AccessReview: review})
}
