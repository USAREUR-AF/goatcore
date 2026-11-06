// Package permissions loads ops/permissions/group-permissions.yml — the
// canonical source of truth for what each group grants on each service.
// The console renders this verbatim on /permissions; the same file is
// (eventually) consumed by the L7 sidecar templates.
package permissions

import (
	"fmt"
	"os"

	"gopkg.in/yaml.v3"
)

type Grant struct {
	Service string `yaml:"service"`
	Access  string `yaml:"access"`
}

type Group struct {
	Name string `yaml:"name"`
	// Description is ONE plain sentence, rendered in the grid beside the name
	// and on the new-user form. Details is the optional longer explainer,
	// rendered behind a click. Neither may carry repo paths, ADR numbers or
	// finding IDs — those go in YAML comments; a reader of the console cannot
	// follow them (TestGroupTextIsForReaders).
	Description string  `yaml:"description"`
	Details     string  `yaml:"details"`
	Grants      []Grant `yaml:"grants"`
}

// Door is the closed set of front-door postures a service may declare.
// It is what tells a reader how to read a blank cell in the group->service
// grid: only GroupGated makes a blank cell a denial.
const (
	DoorGroupGated    = "group-gated"   // oauth2-proxy fence admitting only granted groups
	DoorAuthenticated = "authenticated" // oauth2-proxy fence admitting any authenticated identity
	DoorInApp         = "in-app"        // no proxy; the app authenticates and authorizes itself
	DoorMeshOnly      = "mesh-only"     // no identity anywhere; Traefik mesh IPAllowList only
)

type Service struct {
	Name        string `yaml:"name"`
	Door        string `yaml:"door"`
	Description string `yaml:"description"`
	Surface     string `yaml:"surface"`
	Fence       string `yaml:"fence"`
	AuditPoint  string `yaml:"audit_point"`
}

// GridIsAuthoritative reports whether a blank cell in this service's column
// means the group is DENIED. Only a group-gated door makes that true. For
// every other door the grid records who reads WHAT, or nothing at all — a
// mesh-only surface admits any mesh caller regardless of what the grid says.
//
// The console marks the non-authoritative columns because the unmarked grid
// misled an operator : airdefenders showed a play grant to
// PLATFORM-ADMIN and an em-dash everywhere else, which reads as "nobody else
// may play", while the surface in fact had no identity gate at all.
func (s Service) GridIsAuthoritative() bool { return s.Door == DoorGroupGated }

// OpenCell is what the grid renders in a cell the matrix grants nothing
// for. On a group-gated column that is an em-dash: the group is denied. On
// every other door the honest cell says what the door admits, because the
// legend above the grid was not enough — a reader who scrolls past it sees
// a green pill in one row and dashes in the rest and reads a denial.
//
// The labels name the thing itself. "any identity" vs "everyone" did not:
// the operator asked what the difference was. "any login" — you must be
// someone; "no login" — you need not be anyone; "app's own gate" — no proxy,
// the application asks you to log in and decides for itself.
func (s Service) OpenCell() (label, class string) {
	switch s.Door {
	case DoorGroupGated:
		return "", ""
	case DoorAuthenticated:
		return "any login", "open"
	case DoorInApp:
		return "app's own gate", "open"
	case DoorMeshOnly:
		return "no login", "everyone"
	}
	return "unverified", "open"
}

// OpenCellLabel and OpenCellClass exist because html/template cannot
// destructure a two-value return.
func (s Service) OpenCellLabel() string { l, _ := s.OpenCell(); return l }
func (s Service) OpenCellClass() string { _, c := s.OpenCell(); return c }

// DoorNote is the short human gloss rendered beside a non-authoritative
// column, and in the service-fence table.
func (s Service) DoorNote() string {
	switch s.Door {
	case DoorGroupGated:
		return "oauth2-proxy fence admitting only the groups granted below — a blank cell is a denial."
	case DoorAuthenticated:
		return "oauth2-proxy fence admitting ANY authenticated identity; the app differentiates behind it. A blank cell is not a denial."
	case DoorInApp:
		return "No proxy fence — the application authenticates and authorizes itself. A blank cell describes the app's tiers, not the door."
	case DoorMeshOnly:
		return "NO identity gate. Traefik mesh IPAllowList only, so any caller that reaches the mesh is admitted. A blank cell is not a denial — nothing here is group-gated."
	case "":
		return "No door declared. Treat the column as unverified."
	}
	return "Unknown door value " + s.Door + " — treat the column as unverified."
}

type Matrix struct {
	SchemaVersion  int       `yaml:"schema_version"`
	Services       []Service `yaml:"services"`
	Groups         []Group   `yaml:"groups"`
	AccessLevels   []string  `yaml:"access_levels"`
	StepUpRequired []string  `yaml:"step_up_required"`
}

func Load(path string) (*Matrix, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	m := &Matrix{}
	if err := yaml.Unmarshal(data, m); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	if m.SchemaVersion != 1 {
		return nil, fmt.Errorf("%s: unsupported schema_version %d (want 1)", path, m.SchemaVersion)
	}
	return m, nil
}

// AccessByServiceByGroup returns map[groupName][serviceName] -> accessLevel
// for fast lookup when rendering the matrix table.
func (m *Matrix) AccessByServiceByGroup() map[string]map[string]string {
	out := make(map[string]map[string]string, len(m.Groups))
	for _, g := range m.Groups {
		row := make(map[string]string, len(g.Grants))
		for _, gr := range g.Grants {
			row[gr.Service] = gr.Access
		}
		out[g.Name] = row
	}
	return out
}

// AccessFor returns the access level the given group holds on the given
// service, or "" when the matrix grants nothing. A nil matrix grants
// nothing (fail-closed).
func (m *Matrix) AccessFor(group, service string) string {
	if m == nil {
		return ""
	}
	for _, g := range m.Groups {
		if g.Name != group {
			continue
		}
		for _, gr := range g.Grants {
			if gr.Service == service {
				return gr.Access
			}
		}
	}
	return ""
}

// AnyGroupGrants reports whether any of the caller's groups holds one of
// the wanted access levels on the given service, per the matrix. This is
// the ADR 1073 gate primitive: services consult the matrix instead of
// hardcoding group literals, so granting a new group (e.g. IAM-ADMIN)
// is a YAML edit, not a code change. Fail-closed on nil matrix, empty
// groups, or "none" grants.
func (m *Matrix) AnyGroupGrants(groups []string, service string, wanted ...string) bool {
	for _, g := range groups {
		access := m.AccessFor(g, service)
		if access == "" || access == "none" {
			continue
		}
		for _, w := range wanted {
			if access == w {
				return true
			}
		}
	}
	return false
}

// IsStepUpAction returns true if the given action requires AAL2 step-up
// per the step_up_required list. Action keys are matched exact-string,
// e.g. "grant_ADMINS" / "delete_identity".
func (m *Matrix) IsStepUpAction(action string) bool {
	for _, a := range m.StepUpRequired {
		if a == action {
			return true
		}
	}
	return false
}
