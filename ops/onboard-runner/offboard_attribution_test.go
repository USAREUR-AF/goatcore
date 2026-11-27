package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const approverID = "11111111-2222-4333-8444-555555555555"

// fakeGoat writes a goat stand-in that logs each invocation (the operator
// handle it ran as, then its argv) one line per call. It exits exitRevoke for
// `admin onboard ... --revoke` and exitAudit for `audit ...`.
func fakeGoat(t *testing.T, exitRevoke, exitAudit int) (bin, log string) {
	t.Helper()
	dir := t.TempDir()
	bin = filepath.Join(dir, "goat")
	log = filepath.Join(dir, "calls.log")
	script := "#!/bin/sh\necho \"as=$GOAT_OPERATOR_HANDLE $*\" >> \"" + log + "\"\n" +
		"case \"$*\" in audit*) [ " + string(rune('0'+exitAudit)) + " = 0 ] || echo 'error: emit an audit record: router unreachable' >&2; exit " + string(rune('0'+exitAudit)) + ";; " +
		"*--revoke*) exit " + string(rune('0'+exitRevoke)) + ";; esac\nexit 0\n"
	if err := os.WriteFile(bin, []byte(script), 0o755); err != nil { // #nosec G306 -- test fixture executable
		t.Fatal(err)
	}
	return bin, log
}

func readCalls(t *testing.T, log string) []string {
	t.Helper()
	b, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	return strings.Split(strings.TrimSpace(string(b)), "\n")
}

// offboardWith runs one queued offboard through processOffboard against the
// fake portal with the given fake goat, and returns the runner and portal.
func offboardWith(t *testing.T, bin string, p proposal) (*runner, *fakePortal) {
	t.Helper()
	f := newFakePortal()
	p.Status = "offboard-requested"
	f.add(p)
	r, _ := testRunner(t, f)
	r.goatBin = bin
	r.processOffboard(context.Background(), *f.proposals[p.ID])
	return r, f
}

func fallbackLines(t *testing.T, r *runner) []operatorActionRecord {
	t.Helper()
	b, err := os.ReadFile(filepath.Join(r.repoRoot, "out", "audit", "operator-actions.log"))
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		t.Fatal(err)
	}
	var out []operatorActionRecord
	for _, l := range strings.Split(strings.TrimSpace(string(b)), "\n") {
		var rec operatorActionRecord
		if err := json.Unmarshal([]byte(l), &rec); err != nil {
			t.Fatalf("fallback line %q: %v", l, err)
		}
		out = append(out, rec)
	}
	return out
}

// A console-requested offboard is approved by one person and executed by the
// runner. The operator-action trail names both: admin.offboard records under
// the approver's session identity bracket the revoke the runner runs as
// itself, and the result written back says the record was made.
func TestOffboardRecordsApproverAndExecutor(t *testing.T) {
	for _, tc := range []struct {
		name, status, terminal string
		exit                   int
	}{{"ok", "offboarded", "--outcome ok", 0}, {"failed", "offboard-failed", "--outcome failed", 1}} {
		t.Run(tc.name, func(t *testing.T) {
			bin, log := fakeGoat(t, tc.exit, 0)
			r, f := offboardWith(t, bin, proposal{ID: "off-1", Env: "zagros", Handle: "jane",
				OffboardRequestedBy: "admin@example.org", OffboardRequestedByIdentity: approverID})
			calls := readCalls(t, log)
			if len(calls) != 3 || !strings.Contains(calls[1], "as=onboard-runner-test admin onboard jane --revoke --native") {
				t.Fatalf("goat calls = %q, want started audit, the revoke as the runner, terminal audit", calls)
			}
			for i, want := range []string{"--outcome started", tc.terminal} {
				for _, part := range []string{"as=kratos-" + approverID + " audit emit --actor kratos-" + approverID,
					"--action admin.offboard", "--target jane", "--env zagros", want} {
					if !strings.Contains(calls[i*2], part) {
						t.Errorf("audit call %q lacks %q", calls[i*2], part)
					}
				}
			}
			if f.results["off-1"] != tc.status || f.resultAudit["off-1"] != "recorded" {
				t.Fatalf("writeback = %q audit %q, want %s / recorded", f.results["off-1"], f.resultAudit["off-1"], tc.status)
			}
			if got := fallbackLines(t, r); len(got) != 0 {
				t.Fatalf("fallback log has %d records, want none when the fabric took them", len(got))
			}
		})
	}
}

// The GHSA-3m4r round-2 review case: the audit fabric refuses both emits.
// The cut still runs and lands; both records go to the fallback log; and the
// result written back says the operator-action record was not written, so
// the console does not show an unqualified success.
func TestOffboardAuditFabricDownIsFallbackAndVisible(t *testing.T) {
	bin, log := fakeGoat(t, 0, 1)
	r, f := offboardWith(t, bin, proposal{ID: "off-1", Env: "zagros", Handle: "jane",
		OffboardRequestedBy: "admin@example.org", OffboardRequestedByIdentity: approverID})
	calls := readCalls(t, log)
	if len(calls) != 3 || !strings.Contains(calls[1], "--revoke") {
		t.Fatalf("calls = %q, want the revoke to run between the two audit attempts", calls)
	}
	if f.results["off-1"] != "offboarded" || f.resultErrs["off-1"] != "" {
		t.Fatalf("writeback = %q err %q, want a clean offboarded (the cut landed)", f.results["off-1"], f.resultErrs["off-1"])
	}
	a := f.resultAudit["off-1"]
	if !strings.HasPrefix(a, "failed ") || !strings.Contains(a, "router unreachable") || !strings.Contains(a, "operator-actions.log") {
		t.Fatalf("operator audit written back = %q, want failed naming the emit error and the fallback log", a)
	}
	recs := fallbackLines(t, r)
	if len(recs) != 2 || recs[0].Outcome != "started" || recs[1].Outcome != "ok" {
		t.Fatalf("fallback records = %+v, want started then ok", recs)
	}
	for _, rec := range recs {
		if rec.Actor != "kratos-"+approverID || rec.Action != "admin.offboard" || rec.Target != "jane" || len(rec.ArgsDigest) != 64 || rec.TsUnixMs == 0 {
			t.Errorf("fallback record %+v", rec)
		}
	}
	if fi, err := os.Stat(filepath.Join(r.repoRoot, "out", "audit", "operator-actions.log")); err != nil || fi.Mode().Perm() != 0o600 {
		t.Errorf("fallback log mode = %v (%v), want 0600", fi.Mode().Perm(), err)
	}
}

// With neither the fabric nor the fallback writable, the cut still lands and
// the writeback says the record is lost.
func TestOffboardAuditLostEverywhereIsStillVisible(t *testing.T) {
	bin, _ := fakeGoat(t, 0, 1)
	f := newFakePortal()
	f.add(proposal{ID: "off-1", Env: "zagros", Handle: "jane", Status: "offboard-requested",
		OffboardRequestedBy: "a@b", OffboardRequestedByIdentity: approverID})
	r, _ := testRunner(t, f)
	r.goatBin = bin
	blocker := filepath.Join(t.TempDir(), "not-a-dir")
	if err := os.WriteFile(blocker, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	r.auditFallbackPath = filepath.Join(blocker, "operator-actions.log")
	r.processOffboard(context.Background(), *f.proposals["off-1"])
	if f.results["off-1"] != "offboarded" || !strings.Contains(f.resultAudit["off-1"], "not in the fallback log") {
		t.Fatalf("writeback = %q audit %q", f.results["off-1"], f.resultAudit["off-1"])
	}
}

// With no approver identity on the row, the admin.offboard record is still
// written (never skipped), under the runner, saying the approver identity is
// missing, and the writeback reports the trail incomplete. A malformed
// identity is treated as none; it never reaches the actor field.
func TestOffboardWithoutApproverIdentityStillRecordsAndRevokes(t *testing.T) {
	for _, id := range []string{"", "admin-alex", "kratos-" + approverID, "../x"} {
		t.Run(id, func(t *testing.T) {
			bin, log := fakeGoat(t, 0, 0)
			_, f := offboardWith(t, bin, proposal{ID: "off-1", Env: "zagros", Handle: "jane",
				OffboardRequestedBy: "a@b", OffboardRequestedByIdentity: id})
			calls := readCalls(t, log)
			if len(calls) != 3 || !strings.Contains(calls[1], "--revoke") {
				t.Fatalf("calls = %q, want started audit, revoke, terminal audit", calls)
			}
			for _, c := range []string{calls[0], calls[2]} {
				for _, part := range []string{"audit emit --actor onboard-runner-test ", "--action admin.offboard",
					"--error approver identity not recorded (requested_by=a@b)"} {
					if !strings.Contains(c, part) {
						t.Errorf("audit call %q lacks %q", c, part)
					}
				}
			}
			if f.results["off-1"] != "offboarded" || !strings.HasPrefix(f.resultAudit["off-1"], "failed ") ||
				!strings.Contains(f.resultAudit["off-1"], "approver identity not recorded") {
				t.Fatalf("writeback = %q audit %q", f.results["off-1"], f.resultAudit["off-1"])
			}
		})
	}
}

// The departure the console recorded travels into the admin.offboard record
// as reason and day; anything outside the three classes or the date shape is
// left out, never copied.
func TestDepartureDigest(t *testing.T) {
	for _, tc := range []struct{ class, day, want string }{
		{"left-service", "2031-02-03", "reason=left-service effective=2031-02-03"},
		{"new-position", "", "reason=new-position"},
		{"other", "03/02/2031", "reason=other"},
		{"", "2031-02-03", ""},
		{"left-service --outcome ok", "2031-02-03", ""},
	} {
		got := departureDigest(proposal{OffboardReasonClass: tc.class, OffboardEffective: tc.day})
		if got != tc.want {
			t.Errorf("departureDigest(%q, %q) = %q, want %q", tc.class, tc.day, got, tc.want)
		}
	}
}
