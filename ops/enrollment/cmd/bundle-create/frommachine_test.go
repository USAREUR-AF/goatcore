package main

// Tests for the machine-mesh-alignment D6a legs: --from-machine (acl_groups
// derived from the portal registry + provenance stamped into the bundle) and
// the gear-class refusal on the hand-typed --acl-groups path.

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
)

// fakePortal serves GET /api/machines/{anchor} the way the portal does,
// asserting the operator key rides as the Bearer.
func fakePortal(t *testing.T, wantKey string, machines map[string]portalMachine) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/machines/{anchor}", func(w http.ResponseWriter, r *http.Request) {
		if got := r.Header.Get("Authorization"); got != "Bearer "+wantKey {
			w.WriteHeader(http.StatusUnauthorized)
			_, _ = w.Write([]byte(`{"error":{"code":"auth"}}`))
			return
		}
		m, ok := machines[r.PathValue("anchor")]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			_, _ = w.Write([]byte(`{"error":{"code":"not_a_machine"}}`))
			return
		}
		_ = json.NewEncoder(w).Encode(m)
	})
	ts := httptest.NewTLSServer(mux)
	t.Cleanup(ts.Close)
	return ts
}

func TestRun_FromMachine_DerivedGroupsAndProvenanceStamp(t *testing.T) {
	caKey, ageID, caPub := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")

	portal := fakePortal(t, "op-key-test", map[string]portalMachine{
		"slot-kid-7": {
			SlotID: "slot-kid-7", SystemHandle: "gateway-one-gw-1", NodeClass: "KID",
			MeshGroups: []string{"GALAXY-PROD-KID"}, MeshDerivable: true,
		},
	})

	args := runArgs{
		Site:              "galaxy-prod",
		DeviceID:          "gateway-one-gw-1",
		WGCP0Address:      "10.99.0.20/24",
		RelaysPath:        relaysPath,
		FromMachine:       "slot-kid-7",
		PortalURL:         portal.URL,
		PortalOperatorKey: "op-key-test",
		portalHTTPClient:  portal.Client(),
		CAID:              "dev-goat-ca-test",
		CAKeyPath:         caKey,
		AgeIdentityPath:   ageID,
		OutPath:           outPath,
		TTLDays:           defaultTTLDays,
		ActivationDays:    defaultActivationDays,
		Now:               fixedNow(),
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}

	wire, err := os.ReadFile(outPath)
	if err != nil {
		t.Fatalf("read out: %v", err)
	}
	b, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if err := b.Verify(caPub); err != nil {
		t.Fatalf("verify: %v", err)
	}
	if len(b.ACLGroups) != 1 || b.ACLGroups[0] != "GALAXY-PROD-KID" {
		t.Fatalf("ACLGroups = %v, want the DERIVED [GALAXY-PROD-KID]", b.ACLGroups)
	}
	if b.EnrollmentAnchor != "slot-kid-7" {
		t.Fatalf("EnrollmentAnchor = %q, want slot-kid-7", b.EnrollmentAnchor)
	}
	if b.NodeClass != "KID" {
		t.Fatalf("NodeClass = %q, want KID", b.NodeClass)
	}
}

func TestRun_FromMachine_RefusesUnderivableClass(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)

	portal := fakePortal(t, "op-key-test", map[string]portalMachine{
		"slot-legacy": {
			SlotID: "slot-legacy", NodeClass: "SENSOR", MeshDerivable: false,
			MeshNote: `this machine's born-as node-class "SENSOR" has no mesh derivation: reserved. Re-stamp it with a gear class (PUT /api/slots/{anchor}/custody) and re-mint.`,
		},
	})

	err := run(runArgs{
		Site: "galaxy-prod", DeviceID: "legacy-1", WGCP0Address: "10.99.0.21/24",
		RelaysPath: relaysPath, FromMachine: "slot-legacy",
		PortalURL: portal.URL, PortalOperatorKey: "op-key-test", portalHTTPClient: portal.Client(),
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: filepath.Join(t.TempDir(), "b.cbor"),
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
	})
	if err == nil {
		t.Fatal("expected refusal for mesh_derivable=false")
	}
	// The refusal must carry the portal's re-stamp fix, not a bare no.
	if !strings.Contains(err.Error(), "Re-stamp") {
		t.Fatalf("refusal should print the re-stamp fix, got: %v", err)
	}
}

func TestValidateArgs_FromMachineContract(t *testing.T) {
	base := runArgs{
		Site: "s", DeviceID: "d", WGCP0Address: "10.99.0.9/24", RelaysPath: "/tmp/r",
		CAID: "c", CAKeyPath: "/tmp/k", AgeIdentityPath: "/tmp/a", OutPath: "/tmp/o",
		TTLDays: 90, ActivationDays: 14,
	}

	both := base
	both.FromMachine, both.ACLGroupsRaw = "slot-1", "G"
	if err := validateArgs(&both); err == nil || !strings.Contains(err.Error(), "mutually exclusive") {
		t.Fatalf("both sources: err = %v, want mutual-exclusion refusal", err)
	}

	neither := base
	if err := validateArgs(&neither); err == nil || !strings.Contains(err.Error(), "one of --acl-groups or --from-machine") {
		t.Fatalf("neither source: err = %v", err)
	}

	noURL := base
	noURL.FromMachine = "slot-1"
	if err := validateArgs(&noURL); err == nil || !strings.Contains(err.Error(), "--portal-url") {
		t.Fatalf("missing portal url: err = %v", err)
	}

	httpURL := base
	httpURL.FromMachine, httpURL.PortalURL, httpURL.PortalOperatorKey = "slot-1", "http://portal", "k"
	if err := validateArgs(&httpURL); err == nil || !strings.Contains(err.Error(), "https") {
		t.Fatalf("http portal url: err = %v", err)
	}

	noKey := base
	noKey.FromMachine, noKey.PortalURL = "slot-1", "https://portal"
	if err := validateArgs(&noKey); err == nil || !strings.Contains(err.Error(), "GOAT_PORTAL_OPERATOR_KEY") {
		t.Fatalf("missing operator key: err = %v, want a message naming the env var", err)
	}
}

// fixtureTaxonomy writes a minimal taxonomy with the real file's shape:
// site prefixes, exercise prefixes, and the gear-class template.
func fixtureTaxonomy(t *testing.T) string {
	t.Helper()
	content := `netbird_acl_taxonomy:
  galaxy:
    site_prefix: GALAXY
    groups:
      - suffix: INFRA
        description: "substrate"
  galaxy-prod:
    site_prefix: GALAXY-PROD
    groups:
      - suffix: INFRA
        description: "substrate"
netbird_exercise_gear_template:
  groups:
    - suffix: SENSOR-FEED
      description: "gear"
    - suffix: KID
      description: "gear"
netbird_exercises:
  site-one:
    site_prefix: SITE_ONE
    host_site: galaxy-prod
    gear: true
`
	path := filepath.Join(t.TempDir(), "taxonomy.yml")
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write taxonomy: %v", err)
	}
	return path
}

func TestRefuseHandTypedGearGroups(t *testing.T) {
	tax := fixtureTaxonomy(t)
	refused := [][]string{
		{"GALAXY-PROD-KID"},                    // longest-prefix strip (GALAXY-PROD, not GALAXY)
		{"SITE_ONE-KID"},                       // exercise/site-rung prefix
		{"KID"},                                // bare gear suffix
		{"GALAXY-INFRA", "GALAXY-SENSOR-FEED"}, // one gear group poisons the list
	}
	for _, groups := range refused {
		err := refuseHandTypedGearGroups(tax, groups)
		if err == nil {
			t.Fatalf("groups %v: expected gear-class refusal", groups)
		}
		if !strings.Contains(err.Error(), gearGroupRefusal) {
			t.Fatalf("groups %v: refusal message = %v, want the --from-machine pointer", groups, err)
		}
	}
	allowed := [][]string{
		{"LAB", "LAB-MGMT"},   // unknown prefix, not gear — the existing legitimate use
		{"GALAXY-INFRA"},      // site infra group
		{"GALAXY-PROD-INFRA"}, // longest-prefix strip leaves INFRA
		{"OPERATORS", "G"},    // pre-existing test vocabulary
	}
	for _, groups := range allowed {
		if err := refuseHandTypedGearGroups(tax, groups); err != nil {
			t.Fatalf("groups %v: unexpected refusal: %v", groups, err)
		}
	}
}

func TestRefuseHandTypedGearGroups_DefaultPathWalksUpToRepo(t *testing.T) {
	// The default --taxonomy resolves repo-relative by walking up from the
	// working directory: from a package directory deep in a tree it must find
	// that tree's ops/ansible/netbird_acl_taxonomy.yml, whose gear vocabulary
	// refuses SENSOR-FEED. The tree here is a fixture, so the test reads no
	// goatnet's taxonomy.
	content, err := os.ReadFile(fixtureTaxonomy(t))
	if err != nil {
		t.Fatalf("read fixture taxonomy: %v", err)
	}
	root := t.TempDir()
	taxonomy := filepath.Join(root, filepath.FromSlash(defaultTaxonomyRelPath))
	if err := os.MkdirAll(filepath.Dir(taxonomy), 0o750); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	if err := os.WriteFile(taxonomy, content, 0o644); err != nil {
		t.Fatalf("write taxonomy: %v", err)
	}
	pkg := filepath.Join(root, "ops", "enrollment", "cmd", "bundle-create")
	if err := os.MkdirAll(pkg, 0o750); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	t.Chdir(pkg)
	err = refuseHandTypedGearGroups(defaultTaxonomyRelPath, []string{"GALAXY-PROD-SENSOR-FEED"})
	if err == nil || !strings.Contains(err.Error(), gearGroupRefusal) {
		t.Fatalf("walked-up taxonomy gear refusal: err = %v", err)
	}
	if err := refuseHandTypedGearGroups(defaultTaxonomyRelPath, []string{"LAB", "GALAXY-INFRA"}); err != nil {
		t.Fatalf("walked-up taxonomy infra groups should pass: %v", err)
	}
}

func TestRun_ACLGroups_GearClassRefusedEndToEnd(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	err := run(runArgs{
		Site: "galaxy-prod", DeviceID: "sneaky-1", WGCP0Address: "10.99.0.22/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "GALAXY-PROD-KID",
		TaxonomyPath: fixtureTaxonomy(t),
		CAID:         "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: filepath.Join(t.TempDir(), "b.cbor"),
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
	})
	if err == nil || !strings.Contains(err.Error(), gearGroupRefusal) {
		t.Fatalf("hand-typed gear group must refuse the whole run: err = %v", err)
	}
}
