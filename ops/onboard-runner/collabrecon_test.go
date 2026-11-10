package main

import (
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"gopkg.in/yaml.v3"
)

// writeFile is a tiny fixture helper: create parents, write content.
func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("mkdir %s: %v", filepath.Dir(path), err)
	}
	if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

const manifestActiveRecorded = `identity:
  handle: ada-lovelace
  email: person-5@example.org
  name: Ada Lovelace
  status: active
collab:
  enabled: true
  provider: goat-collab
  activated: true
  account: ada-lovelace
zenoh:
  enabled: false
`

const manifestActiveUnrecorded = `identity:
  handle: grace-hopper
  email: person-47@example.org
  name: Grace Hopper
  status: active
zenoh:
  enabled: false
`

const manifestRevoked = `identity:
  handle: user8x-example
  email: person-60@example.org
  name: Joshua Example
  status: revoked
collab:
  enabled: true
  provider: goat-collab
zenoh:
  enabled: false
`

const manifestDeclaredNoAccount = `identity:
  handle: alan-turing
  email: person-11@example.org
  name: Alan Turing
  status: active
collab:
  enabled: true
zenoh:
  enabled: false
`

const manifestStaleRecord = `identity:
  handle: karen-sparck
  email: person-62@example.org
  name: Karen Sparck
  status: active
collab:
  enabled: true
  provider: goat-collab
  account: karen-sparck
zenoh:
  enabled: false
`

// fixtureRepo lays out a repo root with per-env manifests + a roster.
func fixtureRepo(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	env := filepath.Join(root, "ops", "operators", "zagros")
	writeFile(t, filepath.Join(env, "ada-lovelace.yml"), manifestActiveRecorded)
	writeFile(t, filepath.Join(env, "grace-hopper.yml"), manifestActiveUnrecorded)
	writeFile(t, filepath.Join(env, "user8x-example.yml"), manifestRevoked)
	writeFile(t, filepath.Join(env, "alan-turing.yml"), manifestDeclaredNoAccount)
	writeFile(t, filepath.Join(env, "karen-sparck.yml"), manifestStaleRecord)
	// Files the walker must ignore: a template, a generated out-file, a test
	// fixture tree. A stray one of these would fabricate identities.
	writeFile(t, filepath.Join(root, "ops", "operators", "example.yml.example"), manifestActiveRecorded)
	writeFile(t, filepath.Join(root, "ops", "operators", "out", "ada-lovelace-collab.md"), "not a manifest")
	writeFile(t, filepath.Join(root, "ops", "operators", "test-revoke", "notional.yml"), manifestActiveRecorded)
	writeFile(t, filepath.Join(root, "docs", "operators", "roster.json"), `{"members":[
	  {"handle":"ada-lovelace","type":"person","status":"active","email_hash":"`+emailHash("person-5@example.org")+`"},
	  {"handle":"user167x","type":"person","status":"active","email_hash":"`+emailHash("person-58@example.org")+`"}
	]}`)
	return root
}

func TestLoadManifestRecordsSkipsNonManifests(t *testing.T) {
	root := fixtureRepo(t)
	recs, warns, err := loadManifestRecords(root)
	if err != nil {
		t.Fatalf("loadManifestRecords: %v", err)
	}
	if len(warns) != 0 {
		t.Fatalf("unexpected warnings: %v", warns)
	}
	if len(recs) != 5 {
		got := make([]string, 0, len(recs))
		for _, r := range recs {
			got = append(got, r.Handle)
		}
		t.Fatalf("want 5 manifest records, got %d: %v", len(recs), got)
	}
	byHandle := map[string]identityRecord{}
	for _, r := range recs {
		byHandle[r.Handle] = r
	}
	ada := byHandle["ada-lovelace"]
	if ada.Env != "zagros" {
		t.Errorf("env: want zagros, got %q", ada.Env)
	}
	if ada.Manifest != "ops/operators/zagros/ada-lovelace.yml" {
		t.Errorf("manifest path: got %q", ada.Manifest)
	}
	if ada.Collab.Account != "ada-lovelace" || !ada.Collab.Enabled {
		t.Errorf("collab block not read: %+v", ada.Collab)
	}
	// The email hash must match gen-roster.py's (lowercased, trimmed).
	if ada.EmailHash != emailHash("person-5@example.org") {
		t.Errorf("email hash is not case-normalized: %q", ada.EmailHash)
	}
	if byHandle["user8x-example"].Status != "revoked" {
		t.Errorf("status not read: %q", byHandle["user8x-example"].Status)
	}
}

func TestLoadManifestRecordsWarnsOnUnparseable(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "ops", "operators", "zagros", "good.yml"), manifestActiveRecorded)
	writeFile(t, filepath.Join(root, "ops", "operators", "zagros", "bad.yml"), "identity: [this is: not valid")
	recs, warns, err := loadManifestRecords(root)
	if err != nil {
		t.Fatalf("loadManifestRecords: %v", err)
	}
	if len(recs) != 1 {
		t.Errorf("one bad manifest must not blind the inventory: got %d records", len(recs))
	}
	if len(warns) != 1 || !strings.Contains(warns[0], "bad.yml") {
		t.Errorf("want a warning naming bad.yml, got %v", warns)
	}
}

// reconcileFixture runs the delta over the fixture repo with a fixed account set.
func reconcileFixture(t *testing.T, accts []CollabAccount) reconcileReport {
	t.Helper()
	root := fixtureRepo(t)
	manifests, warns, err := loadManifestRecords(root)
	if err != nil {
		t.Fatalf("manifests: %v", err)
	}
	roster, err := loadRosterRecords(root)
	if err != nil {
		t.Fatalf("roster: %v", err)
	}
	return reconcileCollab(accts, manifests, roster, "goat-collab", "https://idm.example", "2026-08-02T00:00:00Z", warns)
}

func TestReconcileVerdicts(t *testing.T) {
	rep := reconcileFixture(t, []CollabAccount{
		{Handle: "ada-lovelace", Email: "person-5@example.org"},    // recorded + active
		{Handle: "grace-hopper", Email: "person-47@example.org"},   // active, unrecorded
		{Handle: "user8x-example", Email: "person-60@example.org"}, // revoked, live account
		{Handle: "nobody-here", Email: "person-72@example.org"},    // no identity at all
		{Handle: "jvanhay", Email: "person-58@example.org"},        // roster-only, matched by email
	})
	got := map[string]string{}
	for _, r := range rep.Rows {
		got[r.Handle] = r.Verdict
	}
	want := map[string]string{
		"ada-lovelace":   verdictOK,
		"grace-hopper":   verdictUnrecorded,
		"user8x-example": verdictRevokedLiveAccount,
		"nobody-here":    verdictOrphanAccount,
		"jvanhay":        verdictUnrecorded,
		"alan-turing":    verdictDeclaredNoAccount,
		"karen-sparck":   verdictStaleRecord,
	}
	for h, w := range want {
		if got[h] != w {
			t.Errorf("%s: want verdict %q, got %q", h, w, got[h])
		}
	}
	if rep.Counts[verdictRevokedLiveAccount] != 1 {
		t.Errorf("revoked-with-live-account count: %v", rep.Counts)
	}
	// The loudest verdict must sort first — the report is read top-down.
	if rep.Rows[0].Verdict != verdictRevokedLiveAccount {
		t.Errorf("want the action candidate first, got %q", rep.Rows[0].Verdict)
	}
	// An account matched by email to a different handle must say so, and must be
	// attributed to the account name the operator would act on.
	for _, r := range rep.Rows {
		if r.Handle == "jvanhay" && !strings.Contains(r.Note, "user167x") {
			t.Errorf("email-hash match not explained: %q", r.Note)
		}
	}
}

func TestReconcileEmitsNoEmailAddresses(t *testing.T) {
	rep := reconcileFixture(t, []CollabAccount{{Handle: "grace-hopper", Email: "person-47@example.org"}})
	var sb strings.Builder
	writeReport(&sb, rep)
	if strings.Contains(sb.String(), "@") {
		t.Errorf("report leaked an email address:\n%s", sb.String())
	}
}

func TestBuildBackfillSkipsAlreadyRecordedAndIdentityless(t *testing.T) {
	rep := reconcileFixture(t, []CollabAccount{
		{Handle: "ada-lovelace", Email: "person-5@example.org"},
		{Handle: "grace-hopper", Email: "person-47@example.org"},
		{Handle: "user8x-example", Email: "person-60@example.org"},
		{Handle: "nobody-here", Email: "person-72@example.org"},
	})
	doc := buildBackfill(rep, "2026-08-02T00:00:00Z")
	handles := map[string]backfillPatch{}
	for _, p := range doc.Patches {
		handles[p.Handle] = p
	}
	if _, ok := handles["ada-lovelace"]; ok {
		t.Error("already-recorded account must not be patched")
	}
	if _, ok := handles["nobody-here"]; ok {
		t.Error("an account with no manifest cannot be patched")
	}
	g, ok := handles["grace-hopper"]
	if !ok {
		t.Fatal("unrecorded account missing from the backfill")
	}
	if g.Collab.Account != "grace-hopper" || !g.Collab.Enabled || !g.Collab.Activated {
		t.Errorf("patch body: %+v", g.Collab)
	}
	if g.Manifest != "ops/operators/zagros/grace-hopper.yml" || g.Env != "zagros" {
		t.Errorf("patch target: %+v", g)
	}
	// A revoked person's live account IS recorded — that record is what makes the
	// sweep possible; the disable stays a separate operator act.
	if _, ok := handles["user8x-example"]; !ok {
		t.Error("a revoked person's live account must still be recorded")
	}
	// activated_at must not be invented: the provider list carries no date.
	b, err := yaml.Marshal(doc)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if strings.Contains(string(b), "activated_at") {
		t.Errorf("backfill invented an activation date:\n%s", b)
	}
}

func TestApplyBackfillWritesAndIsIdempotent(t *testing.T) {
	root := fixtureRepo(t)
	doc := backfillDoc{Version: 1, ObservedAt: "2026-08-02T00:00:00Z", Provider: "goat-collab",
		Patches: []backfillPatch{{
			Handle: "grace-hopper", Env: "zagros", Manifest: "ops/operators/zagros/grace-hopper.yml",
			Collab: backfillPart{Enabled: true, Provider: "goat-collab", Account: "grace-hopper", Activated: true},
		}}}

	// Dry run writes nothing.
	var sb strings.Builder
	changed, _, errs := applyBackfill(root, doc, false, &sb)
	if len(errs) != 0 || changed != 1 {
		t.Fatalf("dry run: changed=%d errs=%v", changed, errs)
	}
	path := filepath.Join(root, "ops", "operators", "zagros", "grace-hopper.yml")
	before, _ := os.ReadFile(path)
	if strings.Contains(string(before), "collab") {
		t.Fatal("dry run mutated the manifest")
	}

	changed, _, errs = applyBackfill(root, doc, true, &sb)
	if len(errs) != 0 || changed != 1 {
		t.Fatalf("apply: changed=%d errs=%v", changed, errs)
	}
	var got manifestFile
	b, _ := os.ReadFile(path)
	if err := yaml.Unmarshal(b, &got); err != nil {
		t.Fatalf("re-parse: %v", err)
	}
	if got.Collab.Account != "grace-hopper" || !got.Collab.Enabled || !got.Collab.Activated {
		t.Errorf("collab block not written: %+v", got.Collab)
	}
	if got.Identity.Email != "person-47@example.org" || got.Identity.Status != "active" {
		t.Errorf("patch clobbered the identity block: %+v", got.Identity)
	}
	if fi, err := os.Stat(path); err != nil || fi.Mode().Perm() != 0o600 {
		t.Errorf("manifest mode changed: %v %v", fi.Mode().Perm(), err)
	}

	// Second apply is a no-op — a re-run after a partial failure converges.
	changed, unchanged, errs := applyBackfill(root, doc, true, &sb)
	if len(errs) != 0 || changed != 0 || unchanged != 1 {
		t.Fatalf("re-apply not idempotent: changed=%d unchanged=%d errs=%v", changed, unchanged, errs)
	}
}

func TestApplyBackfillPreservesUnrelatedKeysAndComments(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(root, "ops", "operators", "zagros", "ada-lovelace.yml")
	writeFile(t, path, `# hand-authored manifest
identity:
  handle: ada-lovelace
  email: person-6@example.org
  status: active
netbird:
  enabled: true
  auto_groups:
    - OPERATOR
collab:
  enabled: true
`)
	doc := backfillDoc{Version: 1, Patches: []backfillPatch{{
		Handle: "ada-lovelace", Manifest: "ops/operators/zagros/ada-lovelace.yml",
		Collab: backfillPart{Enabled: true, Provider: "goat-collab", Account: "ada-lovelace", Activated: true},
	}}}
	var sb strings.Builder
	if _, _, errs := applyBackfill(root, doc, true, &sb); len(errs) != 0 {
		t.Fatalf("apply: %v", errs)
	}
	out, _ := os.ReadFile(path)
	for _, want := range []string{"# hand-authored manifest", "OPERATOR", "account: ada-lovelace", "provider: goat-collab"} {
		if !strings.Contains(string(out), want) {
			t.Errorf("missing %q after patch:\n%s", want, out)
		}
	}
}

func TestApplyBackfillRefusesPathsOutsideOperators(t *testing.T) {
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "ops", "operators", "zagros", "keep.yml"), manifestActiveUnrecorded)
	writeFile(t, filepath.Join(root, "secrets.yml"), "token: hunter2\n")
	doc := backfillDoc{Version: 1, Patches: []backfillPatch{{
		Handle: "escape", Manifest: "ops/operators/../../secrets.yml",
		Collab: backfillPart{Enabled: true, Account: "escape"},
	}}}
	var sb strings.Builder
	changed, _, errs := applyBackfill(root, doc, true, &sb)
	if changed != 0 || len(errs) != 1 || !strings.Contains(errs[0], "outside ops/operators") {
		t.Fatalf("traversal not refused: changed=%d errs=%v", changed, errs)
	}
	b, _ := os.ReadFile(filepath.Join(root, "secrets.yml"))
	if string(b) != "token: hunter2\n" {
		t.Fatalf("file outside the manifests tree was rewritten: %s", b)
	}
}

func TestReadBackfillRejectsUnknownVersion(t *testing.T) {
	root := t.TempDir()
	p := filepath.Join(root, "bf.yaml")
	writeFile(t, p, "version: 9\npatches: []\n")
	if _, err := readBackfill(p); err == nil || !strings.Contains(err.Error(), "version") {
		t.Fatalf("want a version error, got %v", err)
	}
}

// End-to-end through the operator entry point, against the mock IdP — the
// reconcile is wired to the same provider/credentials as the provisioning leg,
// and never needs a live service to be exercised.
func TestRunCollabOpReconcileEmitsBackfill(t *testing.T) {
	st := &fakeState{personStatus: 200, personBody: `[
	  {"attrs":{"name":["grace-hopper"],"mail":["person-47@example.org"],"displayname":["Grace Hopper"]}},
	  {"attrs":{"name":["user8x-example"],"mail":["person-60@example.org"],"displayname":["Joshua Example"]}}
	]`, token: "T"}
	srv := fakeKanidmSrv(t, st)
	t.Setenv("COLLAB_PROVISION_ENABLED", "1")
	t.Setenv("COLLAB_KANIDM_API_TOKEN", "SVC-TOKEN")
	t.Setenv("COLLAB_KANIDM_URL", srv.URL)

	root := fixtureRepo(t)
	out := filepath.Join(t.TempDir(), "backfill.yaml")
	log := slog.New(slog.NewTextHandler(io.Discard, nil))
	code := runCollabOp(collabOpts{
		op: "reconcile", repoRoot: root, emitBackfill: out, timeout: 5 * time.Second,
	}, log)
	if code != 0 {
		t.Fatalf("reconcile exit = %d, want 0 (a report that finds things is not a failure)", code)
	}
	doc, err := readBackfill(out)
	if err != nil {
		t.Fatalf("read emitted backfill: %v", err)
	}
	got := map[string]bool{}
	for _, p := range doc.Patches {
		got[p.Handle] = true
	}
	if !got["grace-hopper"] || !got["user8x-example"] {
		t.Fatalf("emitted patches = %v, want both live accounts recorded", got)
	}
	if doc.Provider != "goat-collab" || doc.Source != srv.URL {
		t.Errorf("document does not name its source: provider=%q source=%q", doc.Provider, doc.Source)
	}
	// Applying it must be enough to make the next reconcile quiet for that person.
	var sb strings.Builder
	if changed, _, errs := applyBackfill(root, doc, true, &sb); len(errs) != 0 || changed != 2 {
		t.Fatalf("apply: changed=%d errs=%v", changed, errs)
	}
	manifests, _, err := loadManifestRecords(root)
	if err != nil {
		t.Fatalf("re-read manifests: %v", err)
	}
	for _, m := range manifests {
		if m.Handle == "grace-hopper" && m.Collab.Account != "grace-hopper" {
			t.Errorf("backfill did not stick: %+v", m.Collab)
		}
	}
}

// A wrong --repo-root must not read as "every account is an orphan".
func TestRunCollabOpReconcileRefusesEmptyIdentitySide(t *testing.T) {
	st := &fakeState{personStatus: 200, personBody: `[{"attrs":{"name":["someone"]}}]`, token: "T"}
	srv := fakeKanidmSrv(t, st)
	t.Setenv("COLLAB_PROVISION_ENABLED", "1")
	t.Setenv("COLLAB_KANIDM_API_TOKEN", "SVC-TOKEN")
	t.Setenv("COLLAB_KANIDM_URL", srv.URL)

	log := slog.New(slog.NewTextHandler(io.Discard, nil))
	code := runCollabOp(collabOpts{op: "reconcile", repoRoot: t.TempDir(), timeout: 5 * time.Second}, log)
	if code != 2 {
		t.Fatalf("exit = %d, want 2 (usage error) for an empty identity side", code)
	}
}

func TestBackfillRoundTrip(t *testing.T) {
	rep := reconcileFixture(t, []CollabAccount{{Handle: "grace-hopper", Email: "person-47@example.org"}})
	doc := buildBackfill(rep, "2026-08-02T00:00:00Z")
	p := filepath.Join(t.TempDir(), "backfill.yaml")
	if err := writeBackfill(p, doc); err != nil {
		t.Fatalf("write: %v", err)
	}
	back, err := readBackfill(p)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	if len(back.Patches) != len(doc.Patches) || back.Patches[0].Handle != "grace-hopper" {
		t.Fatalf("round trip lost patches: %+v", back)
	}
	if back.Note == "" {
		t.Error("the emitted document must explain how to apply itself")
	}
}
