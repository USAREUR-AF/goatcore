package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

func TestBrowserOffboard_RendersButtonForEligibleStates(t *testing.T) {
	s := newOnboardTestServer(t)
	h := s.Routes()
	ctx := context.Background()

	cases := []struct {
		state   string
		renders bool
		prep    func(t *testing.T, s *Server, h http.Handler, email string) string
	}{
		{onboardstore.ProposalOnboarded, true, func(t *testing.T, s *Server, h http.Handler, email string) string {
			return onboardedProposalInState(t, s, h, email, onboardstore.ProposalOnboarded)
		}},
		{onboardstore.ProposalEnrolled, true, func(t *testing.T, s *Server, h http.Handler, email string) string {
			return onboardedProposalInState(t, s, h, email, onboardstore.ProposalEnrolled)
		}},
		{onboardstore.ProposalArchived, true, func(t *testing.T, s *Server, h http.Handler, email string) string {
			return onboardedProposalInState(t, s, h, email, onboardstore.ProposalArchived)
		}},
		{onboardstore.ProposalProposed, false, func(t *testing.T, s *Server, h http.Handler, email string) string {
			tok := mintTokenViaStore(t, s)
			return submit(t, h, tok, map[string]any{"name": "Pat Start", "email": email, "organization": "Offboard Org"})
		}},
		{onboardstore.ProposalSponsorApproved, false, func(t *testing.T, s *Server, h http.Handler, email string) string {
			tok := mintTokenViaStore(t, s)
			pid := submit(t, h, tok, map[string]any{"name": "Pat Start", "email": email, "organization": "Offboard Org"})
			if err := s.Onboard.DecideProposal(context.Background(), pid, onboardstore.ProposalSponsorApproved, "sponsor@x", ""); err != nil {
				t.Fatalf("decide: %v", err)
			}
			return pid
		}},
	}

	for _, tc := range cases {
		t.Run(tc.state, func(t *testing.T) {
			email := "pat-" + tc.state + "@partner.mil"
			pid := tc.prep(t, s, h, email)

			req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/onboarding?status="+tc.state+"&id="+pid, nil)
			req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
			req.Header.Set("X-Forwarded-Email", "admin@example.org")
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, req)

			if rec.Code != http.StatusOK {
				t.Fatalf("want 200, got %d: %s", rec.Code, rec.Body.String())
			}

			body := rec.Body.String()
			action := "/onboarding/" + pid + "/offboard"

			if tc.renders {
				if !strings.Contains(body, action) {
					t.Fatalf("want offboard button for %s (action=%q), but it was absent", tc.state, action)
				}
			} else {
				if strings.Contains(body, action) {
					t.Fatalf("offboard button rendered for %s (action=%q), but it must not be present", tc.state, action)
				}
			}
		})
	}
}

// The offboard cards name a full pastable revoke command. A row without a
// recorded goatnet must render it WITHOUT the --env flag: `--env ` with a
// blank value makes a pasted shell command swallow `--revoke` as the env
// name. Env is optional at proposal creation, so blank-env rows exist.
func TestBrowserOffboard_ConfirmCommandOmitsBlankEnv(t *testing.T) {
	cases := []struct {
		name string
		env  string
	}{
		{"env recorded", "zagros"},
		{"env blank", ""},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			ctx := context.Background()

			p := &onboardstore.Proposal{
				Sponsor:      "dev@bypass.local",
				Env:          tc.env,
				Handle:       "pat-envcase",
				Email:        "person-94@example.org",
				Status:       onboardstore.ProposalOnboarded,
				ManifestJSON: "{}",
			}
			if err := s.Onboard.CreateProposal(ctx, p); err != nil {
				t.Fatalf("create proposal: %v", err)
			}

			req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/onboarding?status=onboarded&id="+p.ID, nil)
			req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
			req.Header.Set("X-Forwarded-Email", "admin@example.org")
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, req)

			if rec.Code != http.StatusOK {
				t.Fatalf("want 200, got %d: %s", rec.Code, rec.Body.String())
			}

			body := rec.Body.String()
			if tc.env != "" {
				want := "--env " + tc.env + " --revoke"
				if !strings.Contains(body, want) {
					t.Fatalf("confirm command must carry %q when the row records a goatnet", want)
				}
			} else if strings.Contains(body, "--env") {
				t.Fatalf("confirm command must omit --env for a blank-env row (a pasted `--env  --revoke` misparses)")
			}
		})
	}
}

// offboardConfirmText returns the confirm() call on the Offboard form for pid:
// the last thing an operator reads before the record changes state.
func offboardConfirmText(t *testing.T, body, pid string) string {
	t.Helper()
	_, form, found := strings.Cut(body, `action="/onboarding/`+pid+`/offboard"`)
	if !found {
		t.Fatalf("no offboard form for %s", pid)
	}
	_, call, found := strings.Cut(form, "confirm(")
	if !found {
		t.Fatalf("offboard form for %s has no confirm()", pid)
	}
	text, _, found := strings.Cut(call, "return false")
	if !found {
		t.Fatalf("offboard confirm for %s has no refusal branch", pid)
	}
	return text
}

// Offboarding an ARCHIVED row is the one irreversible transition of the three:
// RestoreProposal is guarded on status = 'archived', so an offboarded record has
// no way back. The card's prose said so and the confirm dialog did not (#577).
// The reversible states must not carry the warning, or it stops meaning anything.
func TestBrowserOffboard_ArchivedConfirmNamesTheOneWayDoor(t *testing.T) {
	const oneWay = "cannot be restored"
	cases := []struct {
		state      string
		wantOneWay bool
	}{
		{onboardstore.ProposalOnboarded, false},
		{onboardstore.ProposalEnrolled, false},
		{onboardstore.ProposalArchived, true},
	}

	for _, tc := range cases {
		t.Run(tc.state, func(t *testing.T) {
			s := newOnboardTestServer(t)
			h := s.Routes()
			ctx := context.Background()

			p := &onboardstore.Proposal{
				Sponsor:      "dev@bypass.local",
				Env:          "zagros",
				Handle:       "pat-oneway",
				Email:        "person-98@example.org",
				Status:       tc.state,
				ManifestJSON: "{}",
			}
			if err := s.Onboard.CreateProposal(ctx, p); err != nil {
				t.Fatalf("create proposal: %v", err)
			}

			req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/onboarding?status="+tc.state+"&id="+p.ID, nil)
			req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
			req.Header.Set("X-Forwarded-Email", "admin@example.org")
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, req)
			if rec.Code != http.StatusOK {
				t.Fatalf("want 200, got %d: %s", rec.Code, rec.Body.String())
			}

			confirm := offboardConfirmText(t, rec.Body.String(), p.ID)
			if got := strings.Contains(confirm, oneWay); got != tc.wantOneWay {
				t.Fatalf("%s confirm names the one-way door = %v, want %v:\n%s", tc.state, got, tc.wantOneWay, confirm)
			}
		})
	}
}

// offboardFormMarkup returns the whole <form class="ob-offboard">…</form>
// element for pid, opening tag to closing tag inclusive.
func offboardFormMarkup(t *testing.T, body, pid string) string {
	t.Helper()
	action := `action="/onboarding/` + pid + `/offboard"`
	at := strings.Index(body, action)
	if at < 0 {
		t.Fatalf("no offboard form for %s", pid)
	}
	start := strings.LastIndex(body[:at], "<form ")
	if start < 0 {
		t.Fatalf("offboard action for %s is not inside a <form>", pid)
	}
	end := strings.Index(body[at:], "</form>")
	if end < 0 {
		t.Fatalf("offboard form for %s is not closed", pid)
	}
	return body[start : at+end+len("</form>")]
}

// renderOffboardCard creates a proposal in state and returns the rendered
// onboarding page for it.
func renderOffboardCard(t *testing.T, state, handle, env string) (*Server, string, string) {
	t.Helper()
	s := newOnboardTestServer(t)
	h := s.Routes()
	ctx := context.Background()

	p := &onboardstore.Proposal{
		Sponsor:      "dev@bypass.local",
		Env:          env,
		Handle:       handle,
		Email:        handle + "@partner.mil",
		Status:       state,
		ManifestJSON: "{}",
	}
	if err := s.Onboard.CreateProposal(ctx, p); err != nil {
		t.Fatalf("create proposal: %v", err)
	}

	req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/onboarding?status="+state+"&id="+p.ID, nil)
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	req.Header.Set("X-Forwarded-Email", "admin@example.org")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("want 200, got %d: %s", rec.Code, rec.Body.String())
	}
	return s, p.ID, rec.Body.String()
}

// The three Offboard forms render from one shared block. Pin every byte of
// each form against the markup the three hand-written forms produced before
// they were merged, so the shared block cannot drift any of them: the only
// intended differences are the archived card's required note (label, the
// required attribute, placeholder). Indentation inside the form is part of
// the contract; the closing tag sits two columns left of its children.
func TestBrowserOffboard_FormsRenderByteForByte(t *testing.T) {
	const (
		handle       = "pat-bytes"
		env          = "zagros"
		reversible   = "Mark this onboarding record offboarded? "
		oneWay       = "Offboard this archived record for good? This is a one-way door: an offboarded record cannot be restored. To re-open this onboarding instead, cancel and restore it first. "
		confirmTail  = "This changes no access. It records that access was already cut elsewhere (goat admin onboard " + handle + " --env " + env + " --revoke), and releases the email so a fresh invite is accepted."
		onsubmitTail = "')) return false; this.querySelector('button[type=submit]').disabled=true; return true;\">\n"
	)
	optionalNote := "          <label for=\"offboard-note-%[1]s\">Note (optional)</label>\n" +
		"          <input id=\"offboard-note-%[1]s\" type=\"text\" name=\"note\" placeholder=\"why access was cut\">\n"
	requiredNote := "          <label for=\"offboard-note-%[1]s\">Note (required)</label>\n" +
		"          <input id=\"offboard-note-%[1]s\" type=\"text\" name=\"note\" placeholder=\"why this record is being closed for good\" required>\n"

	cases := []struct {
		state  string
		lead   string
		note   string
		button string
	}{
		{onboardstore.ProposalOnboarded, reversible, optionalNote, "Mark offboarded"},
		{onboardstore.ProposalEnrolled, reversible, optionalNote, "Mark offboarded"},
		{onboardstore.ProposalArchived, oneWay, requiredNote, "Mark offboarded"},
	}

	for _, tc := range cases {
		t.Run(tc.state, func(t *testing.T) {
			s, pid, body := renderOffboardCard(t, tc.state, handle, env)

			want := "<form class=\"ob-offboard\" method=\"post\" action=\"/onboarding/" + pid + "/offboard\" onsubmit=\"if (!confirm('" + tc.lead + confirmTail + onsubmitTail +
				"          <input type=\"hidden\" name=\"csrf\" value=\"" + s.csrfNonce + "\">\n" +
				strings.ReplaceAll(tc.note, "%[1]s", pid) +
				"          <button class=\"bad\" type=\"submit\" aria-label=\"Mark " + handle + " offboarded\">" + tc.button + "</button>\n" +
				"        </form>"

			if got := offboardFormMarkup(t, body, pid); got != want {
				t.Fatalf("%s offboard form drifted.\n--- got ---\n%s\n--- want ---\n%s", tc.state, got, want)
			}
		})
	}
}

// The note is required on the archived card only: that offboard is one-way
// and the gate refuses it blank, so the form must say so and the browser must
// not submit an empty field. The reversible cards keep the optional note.
func TestBrowserOffboard_ArchivedNoteIsRequired(t *testing.T) {
	cases := []struct {
		state        string
		wantRequired bool
	}{
		{onboardstore.ProposalOnboarded, false},
		{onboardstore.ProposalEnrolled, false},
		{onboardstore.ProposalArchived, true},
	}

	for _, tc := range cases {
		t.Run(tc.state, func(t *testing.T) {
			_, pid, body := renderOffboardCard(t, tc.state, "pat-required", "zagros")
			form := offboardFormMarkup(t, body, pid)

			_, input, found := strings.Cut(form, `name="note"`)
			if !found {
				t.Fatalf("%s form has no note input:\n%s", tc.state, form)
			}
			input, _, _ = strings.Cut(input, ">")
			if got := strings.Contains(input, " required"); got != tc.wantRequired {
				t.Fatalf("%s note input required = %v, want %v:\n%s", tc.state, got, tc.wantRequired, form)
			}
			if got := strings.Contains(form, "Note (required)"); got != tc.wantRequired {
				t.Fatalf("%s form labels the note required = %v, want %v:\n%s", tc.state, got, tc.wantRequired, form)
			}
			if got := strings.Contains(form, "Note (optional)"); got == tc.wantRequired {
				t.Fatalf("%s form labels the note optional = %v, want %v:\n%s", tc.state, got, !tc.wantRequired, form)
			}
		})
	}
}
