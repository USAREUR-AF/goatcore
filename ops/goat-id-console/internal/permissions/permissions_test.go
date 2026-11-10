package permissions

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLoadCanonicalYAML(t *testing.T) {
	// The canonical YAML lives at ops/permissions/group-permissions.yml,
	// three levels up from this test file (internal/permissions/).
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	yamlPath := filepath.Join(wd, "..", "..", "..", "permissions", "group-permissions.yml")
	if _, err := os.Stat(yamlPath); err != nil {
		t.Skipf("canonical YAML not at %s: %v", yamlPath, err)
	}
	m, err := Load(yamlPath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if m.SchemaVersion != 1 {
		t.Errorf("SchemaVersion=%d, want 1", m.SchemaVersion)
	}
	if len(m.Services) == 0 {
		t.Error("no services in canonical YAML")
	}
	if len(m.Groups) == 0 {
		t.Error("no groups in canonical YAML")
	}

	// Spot-check known-canon entries so a future edit of the YAML can't
	// silently drop these without a CI signal.
	// ADMINS + VENDOR were contracted out of the SoT (Ory-plane
	// rename completion); PLATFORM-ADMIN is the canonical platform-admin group.
	gotPlatformAdmin := false
	for _, g := range m.Groups {
		if g.Name == "ADMINS" {
			t.Error("canonical YAML still carries the contracted ADMINS group")
		}
		if g.Name == "VENDOR" {
			t.Error("canonical YAML still carries the contracted VENDOR group")
		}
		if g.Name == "PLATFORM-ADMIN" {
			gotPlatformAdmin = true
		}
	}
	if !gotPlatformAdmin {
		t.Error("canonical YAML missing PLATFORM-ADMIN group")
	}

	if !m.IsStepUpAction("grant_PLATFORM-ADMIN") {
		t.Error("grant_PLATFORM-ADMIN should be a step-up action")
	}
	if m.IsStepUpAction("read_anything") {
		t.Error("read_anything should NOT be a step-up action")
	}
}

func TestAccessByServiceByGroup(t *testing.T) {
	m := &Matrix{
		SchemaVersion: 1,
		Groups: []Group{
			{Name: "ADMINS", Grants: []Grant{
				{Service: "dashboard", Access: "admin"},
				{Service: "grafana", Access: "read"},
			}},
		},
	}
	got := m.AccessByServiceByGroup()
	if got["ADMINS"]["dashboard"] != "admin" {
		t.Errorf("dashboard access = %q, want admin", got["ADMINS"]["dashboard"])
	}
	if got["ADMINS"]["grafana"] != "read" {
		t.Errorf("grafana access = %q, want read", got["ADMINS"]["grafana"])
	}
	if got["ADMINS"]["unknown"] != "" {
		t.Errorf("unknown service should map to empty, got %q", got["ADMINS"]["unknown"])
	}
}

func TestAnyGroupGrants(t *testing.T) {
	m := &Matrix{
		SchemaVersion: 1,
		Groups: []Group{
			{Name: "ADMINS", Grants: []Grant{{Service: "id-admin-console", Access: "admin"}}},
			{Name: "IAM-ADMIN", Grants: []Grant{{Service: "id-admin-console", Access: "admin"}}},
			{Name: "AUDITOR", Grants: []Grant{{Service: "id-admin-console", Access: "read"}}},
			{Name: "BLOCKED", Grants: []Grant{{Service: "id-admin-console", Access: "none"}}},
		},
	}
	cases := []struct {
		name   string
		groups []string
		wanted []string
		want   bool
	}{
		{"admins pass", []string{"ADMINS"}, []string{"admin"}, true},
		{"iam-admin passes without a code change", []string{"IAM-ADMIN"}, []string{"admin"}, true},
		{"read tier does not pass admin", []string{"AUDITOR"}, []string{"admin"}, false},
		{"read tier passes read-or-admin", []string{"AUDITOR"}, []string{"admin", "read"}, true},
		{"explicit none never passes", []string{"BLOCKED"}, []string{"admin", "read", "none"}, false},
		{"unknown group fails closed", []string{"WHOEVER"}, []string{"admin"}, false},
		{"empty groups fail closed", nil, []string{"admin"}, false},
		{"mixed groups admit on any match", []string{"WHOEVER", "ADMINS"}, []string{"admin"}, true},
	}
	for _, c := range cases {
		if got := m.AnyGroupGrants(c.groups, "id-admin-console", c.wanted...); got != c.want {
			t.Errorf("%s: AnyGroupGrants(%v, %v)=%v, want %v", c.name, c.groups, c.wanted, got, c.want)
		}
	}
}

func TestAnyGroupGrantsNilMatrixFailsClosed(t *testing.T) {
	var m *Matrix
	if m.AnyGroupGrants([]string{"ADMINS"}, "id-admin-console", "admin") {
		t.Error("nil matrix must grant nothing")
	}
	if m.AccessFor("ADMINS", "id-admin-console") != "" {
		t.Error("nil matrix AccessFor must be empty")
	}
}

func TestCanonicalMatrixGrantsConsoleAdminToIAMAdmin(t *testing.T) {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	yamlPath := filepath.Join(wd, "..", "..", "..", "permissions", "group-permissions.yml")
	if _, err := os.Stat(yamlPath); err != nil {
		t.Skipf("canonical YAML not at %s: %v", yamlPath, err)
	}
	m, err := Load(yamlPath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	// The gate this repo actually runs: both admin groups must satisfy it.
	for _, g := range []string{"PLATFORM-ADMIN", "IAM-ADMIN"} {
		if !m.AnyGroupGrants([]string{g}, "id-admin-console", "admin") {
			t.Errorf("canonical matrix does not grant %s admin on id-admin-console", g)
		}
	}
}

// TestEveryServiceDeclaresAValidDoor is the Go half of the ratchet that
// ops/ci/check-group-permissions-parity.sh enforces in CI. A service with no
// door renders a column whose blank cells cannot be read: only a group-gated
// door makes a blank cell a denial. airdefenders, brief and dataflows sat with
// no door and a `fence:` naming an oauth2-proxy that does not exist until
// and the console rendered them as denied to every group but
// PLATFORM-ADMIN while nothing gated them at all.
func TestEveryServiceDeclaresAValidDoor(t *testing.T) {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	yamlPath := filepath.Join(wd, "..", "..", "..", "permissions", "group-permissions.yml")
	if _, err := os.Stat(yamlPath); err != nil {
		t.Skipf("canonical YAML not at %s: %v", yamlPath, err)
	}
	m, err := Load(yamlPath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	valid := map[string]bool{
		DoorGroupGated: true, DoorAuthenticated: true,
		DoorInApp: true, DoorMeshOnly: true,
	}
	for _, s := range m.Services {
		if s.Door == "" {
			t.Errorf("%s: no door declared", s.Name)
			continue
		}
		if !valid[s.Door] {
			t.Errorf("%s: door %q is not one of group-gated/authenticated/in-app/mesh-only", s.Name, s.Door)
			continue
		}
		if s.DoorNote() == "" {
			t.Errorf("%s: door %q renders an empty note", s.Name, s.Door)
		}
		if got, want := s.GridIsAuthoritative(), s.Door == DoorGroupGated; got != want {
			t.Errorf("%s: GridIsAuthoritative()=%v for door %q, want %v", s.Name, got, s.Door, want)
		}
	}
}

// TestMeshOnlyDoorsAreNamed pins the three surfaces that have no identity gate
// at all. If one of these gains a real fence, change the door here in the same
// change — the point of the list is that "ungated" is a decision on the record.
func TestMeshOnlyDoorsAreNamed(t *testing.T) {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	yamlPath := filepath.Join(wd, "..", "..", "..", "permissions", "group-permissions.yml")
	if _, err := os.Stat(yamlPath); err != nil {
		t.Skipf("canonical YAML not at %s: %v", yamlPath, err)
	}
	m, err := Load(yamlPath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	// Empty : airdefenders, brief and dataflows sit behind
	// authenticated doors (oauth2-proxy-surface). Turning a door off again on
	// every goatnet means putting its name back here in the same change.
	// help (ADR 1176): a dormant door. The row says mesh-only because
	// help_oauth2_proxy_enabled is false on every goatnet and the parity
	// check refuses an authenticated door that runs nowhere; the go-live
	// change flips the flag, sets the door to authenticated and removes the
	// name from here.
	want := map[string]bool{"help": true}
	got := map[string]bool{}
	for _, s := range m.Services {
		if s.Door == DoorMeshOnly {
			got[s.Name] = true
		}
	}
	for n := range want {
		if !got[n] {
			t.Errorf("%s: expected door mesh-only, got %q", n, doorOf(m, n))
		}
	}
	for n := range got {
		if !want[n] {
			t.Errorf("%s newly declares door mesh-only (no identity gate) — intended?", n)
		}
	}
}

// TestMeshOnlyDoorsHoldNoGrants: a grant on a door that admits everyone is
// enforced by nothing and renders as an access list. The `play` grant to
// PLATFORM-ADMIN on airdefenders is how the grid read "nobody else may play".
func TestMeshOnlyDoorsHoldNoGrants(t *testing.T) {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	yamlPath := filepath.Join(wd, "..", "..", "..", "permissions", "group-permissions.yml")
	if _, err := os.Stat(yamlPath); err != nil {
		t.Skipf("canonical YAML not at %s: %v", yamlPath, err)
	}
	m, err := Load(yamlPath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	for _, g := range m.Groups {
		for _, gr := range g.Grants {
			if doorOf(m, gr.Service) == DoorMeshOnly {
				t.Errorf("%s grants %q on %s, whose door is mesh-only — nothing enforces it", g.Name, gr.Access, gr.Service)
			}
		}
	}
}

func TestOpenCellFollowsTheDoor(t *testing.T) {
	cases := map[string]string{
		DoorGroupGated: "", DoorAuthenticated: "any login",
		DoorInApp: "app's own gate", DoorMeshOnly: "no login", "": "unverified",
	}
	for door, want := range cases {
		if got := (Service{Door: door}).OpenCellLabel(); got != want {
			t.Errorf("door %q: OpenCellLabel()=%q, want %q", door, got, want)
		}
	}
}

// TestGroupTextIsForReaders: the grid used to render each group's full
// internal description — rename windows, ADR numbers, repo paths, a
// "provision/rotate/revoke/edit" aside — as a wall of text in the first
// column. A console reader cannot follow any of that. One short sentence in
// the row; the rest behind a click; nothing that only makes sense in the repo.
func TestGroupTextIsForReaders(t *testing.T) {
	wd, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	yamlPath := filepath.Join(wd, "..", "..", "..", "permissions", "group-permissions.yml")
	if _, err := os.Stat(yamlPath); err != nil {
		t.Skipf("canonical YAML not at %s: %v", yamlPath, err)
	}
	m, err := Load(yamlPath)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	internal := []string{"docs/", "ADR ", "§", "F-", "ops/", ".md", "ratified", "RATIFIED", "dual-accept"}
	for _, g := range m.Groups {
		if g.Description == "" {
			t.Errorf("%s: no description", g.Name)
		}
		if n := len(g.Description); n > 120 {
			t.Errorf("%s: description is %d chars; one sentence, under 120 — move the rest to details:", g.Name, n)
		}
		for _, s := range internal {
			for field, text := range map[string]string{"description": g.Description, "details": g.Details} {
				if strings.Contains(text, s) {
					t.Errorf("%s: %s contains %q — internal reference; keep it in a YAML comment", g.Name, field, s)
				}
			}
		}
	}
}

func doorOf(m *Matrix, name string) string {
	for _, s := range m.Services {
		if s.Name == name {
			return s.Door
		}
	}
	return "<absent>"
}
