// ADR 1112 §5 non-widening invariants, mechanized (design doc §13.3): org
// hierarchy must widen org-keyed authority through EXACTLY ONE channel — the
// ratified data.view.org vertical roll-up — and nothing else. These fixtures
// run the real gates against the SAME population twice, flat vs three-level
// chain (nation → service → unit), and assert every governance verdict is
// IDENTICAL while the view roll-up is the one intended change.
//
// A future PR that makes governedOrgs / mayGovernProducer / the co-publish
// governance check consult parent edges fails here, not in a design review.
package api

import (
	"io/fs"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/web"
)

// hierarchyServer builds a Server whose roster carries org `nation-hq` ←
// `service-branch` ← `field-unit` (parent chain when chained=true; all roots
// when false). Each org owns one machine producer; the field unit also has a
// person member so governance seeding paths see a person.
func hierarchyServer(t *testing.T, chained bool) *Server {
	t.Helper()
	dir := t.TempDir()
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	parentService, parentUnit := "null", "null"
	if chained {
		parentService, parentUnit = `"nation-hq"`, `"service-branch"`
	}
	body := `{
      "generated_at": "2026-08-04T00:00:00Z",
      "orgs": [
        {"id": "nation-hq", "kind": "partner", "nation": "DEU", "parent": null, "members": [
          {"handle": "hq-node", "type": "machine", "state": "active", "vendor_tag": "hq-node", "email_hash": ""}
        ]},
        {"id": "service-branch", "kind": "partner", "nation": "DEU", "parent": ` + parentService + `, "members": [
          {"handle": "svc-node", "type": "machine", "state": "active", "vendor_tag": "svc-node", "email_hash": ""}
        ]},
        {"id": "field-unit", "kind": "partner", "nation": "DEU", "parent": ` + parentUnit + `, "members": [
          {"handle": "unit-node", "type": "machine", "state": "active", "vendor_tag": "unit-node", "email_hash": ""},
          {"handle": "unit-person", "type": "person", "state": "active", "vendor_tag": null, "email_hash": "deadbeef"}
        ]}
      ],
      "members": []
    }`
	rosterPath := filepath.Join(dir, "roster.json")
	if err := os.WriteFile(rosterPath, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	staticFS, _ := fs.Sub(web.Static, "static")
	return New(&config.Config{OperatorAPIKey: "op", RosterPath: rosterPath, GovernBootstrapOpen: true}, reg, nil, nil, nil, nil, nil, staticFS)
}

func TestHierarchyNeverWidensGovernance(t *testing.T) {
	flat := hierarchyServer(t, false)
	chained := hierarchyServer(t, true)

	type verdict struct {
		name string
		got  func(s *Server) bool
	}
	principals := map[string]*Principal{
		"hq":   {Vendor: "hq-node"},
		"svc":  {Vendor: "svc-node"},
		"unit": {Vendor: "unit-node"},
	}
	producers := []string{"hq-node", "svc-node", "unit-node"}
	orgs := []string{"nation-hq", "service-branch", "field-unit"}

	var checks []verdict
	for pn, p := range principals {
		p := p
		for _, prod := range producers {
			prod := prod
			checks = append(checks, verdict{
				name: "mayGovernProducer " + pn + "→" + prod,
				got: func(s *Server) bool {
					r := httptest.NewRequest("POST", "/api/access/grants", nil)
					return s.mayGovernProducer(r.WithContext(withPrincipal(p)), prod)
				},
			})
		}
		for _, org := range orgs {
			org := org
			checks = append(checks, verdict{
				name: "governsOrg(co-publish gate) " + pn + "→" + org,
				got: func(s *Server) bool {
					r := httptest.NewRequest("POST", "/api/access/multipub-topics", nil)
					dr, derr := access.LoadDataRoster(s.cfg.RosterPath)
					if derr != nil {
						t.Fatalf("roster: %v", derr)
					}
					return s.governsOrg(r.WithContext(withPrincipal(p)), dr, org)
				},
			})
		}
	}
	widened := 0
	for _, c := range checks {
		f, ch := c.got(flat), c.got(chained)
		if f != ch {
			t.Errorf("VERDICT CHANGED by hierarchy: %s — flat=%v chained=%v", c.name, f, ch)
			widened++
		}
	}
	if widened == 0 {
		t.Logf("all %d governance verdicts identical flat vs chained", len(checks))
	}

	// Cross-org stays refused in BOTH shapes (the absolute floor): a parent-
	// org principal never governs the child org or its producer.
	r := httptest.NewRequest("POST", "/api/access/grants", nil)
	if flatV := flat.mayGovernProducer(r.WithContext(withPrincipal(principals["hq"])), "unit-node"); flatV {
		t.Fatalf("baseline broken: hq governs unit-node even when FLAT")
	}
	if chV := chained.mayGovernProducer(r.WithContext(withPrincipal(principals["hq"])), "unit-node"); chV {
		t.Fatalf("WIDENED: nation-hq principal governs the field unit's producer because a parent edge exists")
	}
}

func TestHierarchyWidensExactlyTheViewRollup(t *testing.T) {
	flatS := hierarchyServer(t, false)
	chainedS := hierarchyServer(t, true)
	drFlat, err := access.LoadDataRoster(flatS.cfg.RosterPath)
	if err != nil {
		t.Fatal(err)
	}
	drChained, err := access.LoadDataRoster(chainedS.cfg.RosterPath)
	if err != nil {
		t.Fatal(err)
	}

	// Flat: a nation-hq org grant covers only its own producer.
	if got := drFlat.ProducersUnderOrg("nation-hq"); len(got) != 1 || got[0] != "hq-node" {
		t.Fatalf("flat roll-up: %v", got)
	}
	// Chained: the SAME grant covers the whole subtree — the one intended
	// widening, ratified in data-access-roles.md §6.1.
	if got := drChained.ProducersUnderOrg("nation-hq"); len(got) != 3 {
		t.Fatalf("chained roll-up: %v (want all three producers)", got)
	}
	// And it is downward-only: the field unit's org grant never reaches up.
	if got := drChained.ProducersUnderOrg("field-unit"); len(got) != 1 || got[0] != "unit-node" {
		t.Fatalf("upward leak: field-unit covers %v", got)
	}
	// Membership derivation stays literal: an anchor's org roles name only
	// its DIRECT owning org (ancestors absent) — the input that keeps
	// governedOrgs un-widened by construction.
	roles := drChained.RolesForAnchor("unit-node").Roles
	for _, role := range roles {
		if role == "data.view.org:service-branch" || role == "data.view.org:nation-hq" {
			t.Fatalf("membership rolled UP the chain: %v", roles)
		}
	}
}
