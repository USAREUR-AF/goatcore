package handlers

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// The access review (ADR 1272 section 5) reports five findings and removes
// nothing. These pin each finding to the fact it is derived from.

func reviewIdent(id, email, state string, created time.Time, groups ...string) kratos.Identity {
	gs := make([]interface{}, len(groups))
	for i, g := range groups {
		gs[i] = g
	}
	return kratos.Identity{ID: id, State: state, CreatedAt: created,
		Traits:         map[string]interface{}{"email": email},
		MetadataPublic: map[string]interface{}{"groups": gs}}
}

func TestBuildAccessReview_EachFinding(t *testing.T) {
	now := time.Date(2031, 6, 1, 0, 0, 0, 0, time.UTC)
	old, recent := now.AddDate(-1, 0, 0), now.AddDate(0, 0, -5)
	ids := []kratos.Identity{
		reviewIdent("id-fresh", "fresh@example.test", "active", old),
		reviewIdent("id-idle", "idle@example.test", "active", old),
		reviewIdent("id-new", "new@example.test", "active", recent),
		reviewIdent("id-stale", "stale@example.test", "active", old),
		reviewIdent("id-orphan", "orphan@example.test", "active", old),
		reviewIdent("id-off", "off@example.test", "inactive", old),
		reviewIdent("id-gone", "gone@example.test", "inactive", old),
		reviewIdent("id-back", "back@example.test", "active", old),
		reviewIdent("id-admin", "admin@example.test", "active", old, "IAM-ADMIN", "DEVELOPER"),
		reviewIdent("id-exadmin", "exadmin@example.test", "inactive", old, "PLATFORM-ADMIN"),
	}
	last := map[string]time.Time{
		"id-fresh": recent, "id-stale": now.AddDate(0, 0, -200), "id-orphan": recent,
		"id-back": recent, "id-admin": recent,
	}
	proposals := []onboardstore.Proposal{
		{ID: "p1", Email: "orphan@example.test", Handle: "orphan", Sponsor: "GONE@example.test", Status: onboardstore.ProposalOnboarded, CreatedAt: 10},
		{ID: "p2", Email: "gone@example.test", Handle: "gone", Status: onboardstore.ProposalOffboarded, CreatedAt: 10},
		{ID: "p3", Email: "back@example.test", Handle: "back", Status: onboardstore.ProposalOffboarded, CreatedAt: 10},
		{ID: "p4", Email: "fresh@example.test", Handle: "fresh", Sponsor: "someone@example.test", Status: onboardstore.ProposalOnboarded, CreatedAt: 10},
		// An older, finished row for a person onboarded again is not a departure.
		{ID: "p5", Email: "fresh@example.test", Handle: "fresh", Status: onboardstore.ProposalOffboarded, CreatedAt: 1},
	}
	review := buildAccessReview(now, 90, ids, last, proposals)
	got := map[string][]string{}
	for _, row := range review.Rows {
		got[row.Email] = row.Findings
	}
	want := map[string][]string{
		"idle@example.test":    {findingIdle},           // no live session at all
		"stale@example.test":   {findingIdle},           // live session older than 90 days
		"orphan@example.test":  {findingSponsorLeft},    // sponsor matched case-insensitively
		"off@example.test":     {findingInactiveNoDep},  // inactive, nothing recorded
		"back@example.test":    {findingDepartedActive}, // cut recorded, login active
		"admin@example.test":   {findingAdmin},
		"exadmin@example.test": {findingInactiveNoDep}, // an inactive login's groups are not an admin finding
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("findings:\n got %v\nwant %v", got, want)
	}
	if review.Accounts != len(ids) || review.Counts[findingIdle] != 2 || review.Counts[findingAdmin] != 1 {
		t.Errorf("accounts=%d counts=%v", review.Accounts, review.Counts)
	}
}

// With the session list unread, nobody is reported idle: an outage must not
// read as every account being abandoned.
func TestBuildAccessReview_UnreadSessionsReportNoIdle(t *testing.T) {
	now := time.Date(2031, 6, 1, 0, 0, 0, 0, time.UTC)
	ids := []kratos.Identity{reviewIdent("id-a", "a@example.test", "active", now.AddDate(-1, 0, 0))}
	if review := buildAccessReview(now, 90, ids, nil, nil); len(review.Rows) != 0 {
		t.Errorf("rows = %+v, want none", review.Rows)
	}
}

func TestAdminGroup(t *testing.T) {
	for group, want := range map[string]bool{"PLATFORM-ADMIN": true, "ADMINS": true, "IAM-ADMIN": true,
		"NETBIRD-ADMIN": true, "DEVELOPER": false, "OBS-READONLY": false, "ADMIN-ASSISTANT": false} {
		if adminGroup(group) != want {
			t.Errorf("adminGroup(%q) = %v, want %v", group, !want, want)
		}
	}
}

func TestAccessReview_PageAndJSONChangeNothing(t *testing.T) {
	f := newOpKeyFixture(t, onboardstore.ProposalOnboarded)
	h := f.s.Routes()
	get := func(path string) *httptest.ResponseRecorder {
		req := httptest.NewRequestWithContext(t.Context(), http.MethodGet, path, nil)
		req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
		req.Header.Set("X-Forwarded-Email", "other-admin@example.test")
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		return rec
	}
	page := get("/access-review?idp=tenant")
	if page.Code != http.StatusOK {
		t.Fatalf("GET /access-review = %d: %s", page.Code, page.Body.String())
	}
	for _, want := range []string{`data-access-review-accounts="2"`, `data-review="alice@example.test"`,
		`data-review="bob@example.test"`, "admin"} {
		if !strings.Contains(page.Body.String(), want) {
			t.Errorf("page lacks %q", want)
		}
	}
	raw := get("/access-review?idp=tenant&format=json&idle_days=30")
	var review accessReviewVars
	if err := json.Unmarshal(raw.Body.Bytes(), &review); err != nil {
		t.Fatalf("json: %v: %s", err, raw.Body.String())
	}
	if review.IdleDays != 30 || review.Accounts != 2 || review.Counts[findingAdmin] != 2 {
		t.Errorf("review = %+v", review)
	}
	if bad := get("/access-review?idp=tenant&idle_days=0"); bad.Code != http.StatusBadRequest {
		t.Errorf("idle_days=0: %d, want 400", bad.Code)
	}
	if groups, state := f.read(); state != "active" || len(groups) != 1 || groups[0] != "IAM-ADMIN" {
		t.Errorf("after the review: state %q groups %v; it must change nothing", state, groups)
	}
}
