package api

// Carriage drift check (ADR 1251 phase 4): compare the carry-<anchor> subjects
// the router has with the carriers the registry's rules give, and name every
// difference. Two places are compared: the live config the portal splices
// (what the router loads at its next restart) and the fragment the router
// host's reload watcher recorded as loaded (what the router runs now).

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// Drift kinds.
const (
	DriftMissing    = "missing"    // the rules make CN a carrier of the slot; the router does not admit it
	DriftExtra      = "extra"      // the router admits CN as a carrier; no rule does
	DriftUndeclared = "undeclared" // the router declares no carry-<anchor> subject for the slot
	DriftInert      = "inert"      // the slot's carriage policy binds the inert -none subject: a role run is due
	DriftDisabled   = "disabled"   // the router template binds no carry- subject at all (zenoh_carriage_core_enabled off)
)

// DriftItem is one difference.
type DriftItem struct {
	Where string `json:"where"` // config | loaded
	Kind  string `json:"kind"`
	Slot  string `json:"slot,omitempty"`
	CN    string `json:"cn,omitempty"`
}

// String is the line the drift command prints.
func (d DriftItem) String() string {
	switch d.Kind {
	case DriftMissing:
		return fmt.Sprintf("%s: %s should carry %s, the router does not admit it", d.Where, d.CN, d.Slot)
	case DriftExtra:
		return fmt.Sprintf("%s: %s carries %s with no rule", d.Where, d.CN, d.Slot)
	case DriftUndeclared:
		return fmt.Sprintf("%s: no carry- subject for %s (fragment rendered before carriage)", d.Where, d.Slot)
	case DriftInert:
		return fmt.Sprintf("%s: carriage policy of %s binds the inert -none subject; run the router role once", d.Where, d.Slot)
	case DriftDisabled:
		return fmt.Sprintf("%s: the router template binds no carry- subject (zenoh_carriage_core_enabled is off) but rules name carriers", d.Where)
	}
	return d.Where + ": " + d.Kind
}

var (
	carrySubjectRe = regexp.MustCompile(`id: "carry-([0-9a-f]{32})",\n\s*cert_common_names: \[([^\]]*)\]`)
	carryLeafRe    = regexp.MustCompile(`"([0-9a-f]{32}(?:\.g[1-9][0-9]*)?)"`)
	carryPolicyRe  = regexp.MustCompile(`subjects: \["carry-([0-9a-f]{32})(-none)?"\]`)
)

// slotRegion is the text between the slot-fragment markers, or "" without them.
func slotRegion(text string) string {
	b := strings.Index(text, acl.SlotFragmentBegin)
	if b < 0 {
		return ""
	}
	start := strings.IndexByte(text[b:], '\n')
	if start < 0 {
		return ""
	}
	start += b + 1
	e := strings.Index(text[start:], acl.SlotFragmentEnd)
	if e < 0 {
		return ""
	}
	return text[start : start+e]
}

// compareCarry compares one rendered text with the expected carriers.
// fullConfig is true for the live config (policies are in it), false for the
// loaded record (the fragment alone).
func compareCarry(where, text string, fullConfig bool, anchors []string, want map[string][]string) []DriftItem {
	var out []DriftItem
	frag := text
	if fullConfig {
		frag = slotRegion(text)
	}
	have := map[string]map[string]bool{}
	for _, m := range carrySubjectRe.FindAllStringSubmatch(frag, -1) {
		set := map[string]bool{}
		for _, cn := range carryLeafRe.FindAllStringSubmatch(m[2], -1) {
			set[cn[1]] = true
		}
		have[m[1]] = set
	}
	expectAny := false
	for _, a := range anchors {
		if len(want[a]) > 0 {
			expectAny = true
		}
	}
	if fullConfig {
		policies := map[string]bool{} // anchor -> bound to -none
		for _, m := range carryPolicyRe.FindAllStringSubmatch(text, -1) {
			policies[m[1]] = m[2] != ""
		}
		if len(policies) == 0 {
			if expectAny {
				out = append(out, DriftItem{Where: where, Kind: DriftDisabled})
			}
		} else {
			for _, a := range anchors {
				if policies[a] && len(want[a]) > 0 {
					out = append(out, DriftItem{Where: where, Kind: DriftInert, Slot: a})
				}
			}
		}
	}
	for _, a := range anchors {
		got, declared := have[a]
		if !declared {
			if len(want[a]) > 0 || len(have) > 0 {
				out = append(out, DriftItem{Where: where, Kind: DriftUndeclared, Slot: a})
			}
			for _, cn := range want[a] {
				out = append(out, DriftItem{Where: where, Kind: DriftMissing, Slot: a, CN: cn})
			}
			continue
		}
		exp := map[string]bool{}
		for _, cn := range want[a] {
			exp[cn] = true
			if !got[cn] {
				out = append(out, DriftItem{Where: where, Kind: DriftMissing, Slot: a, CN: cn})
			}
		}
		extra := make([]string, 0)
		for cn := range got {
			if !exp[cn] {
				extra = append(extra, cn)
			}
		}
		sort.Strings(extra)
		for _, cn := range extra {
			out = append(out, DriftItem{Where: where, Kind: DriftExtra, Slot: a, CN: cn})
		}
	}
	return out
}

// CarriageDriftReport is the drift check's answer.
type CarriageDriftReport struct {
	InSync      bool        `json:"in_sync"`
	Compared    []string    `json:"compared"`
	Differences []DriftItem `json:"differences"`
}

// CarriageDrift compares the router config at configPath (and the loaded
// record beside it, when the reload watcher wrote one) with the carriers the
// registry's rules give now.
func CarriageDrift(ctx context.Context, reg *registry.Registry, cfg *config.Config, configPath string) (*CarriageDriftReport, error) {
	if configPath == "" {
		return nil, errors.New("no router config path (--acl-config / PORTAL_ACL_CONFIG)")
	}
	text, err := os.ReadFile(configPath)
	if err != nil {
		return nil, fmt.Errorf("read router config: %w", err)
	}
	bindings, err := reg.SlotBindings(ctx)
	if err != nil {
		return nil, err
	}
	anchors := make([]string, 0, len(bindings))
	for _, b := range bindings {
		anchors = append(anchors, b.Anchor)
	}
	sn, err := loadCarriageSnapshot(ctx, reg, cfg, &fragmentRosters)
	if err != nil {
		return nil, err
	}
	want := sn.carriers()
	rep := &CarriageDriftReport{Compared: []string{"config"}, Differences: []DriftItem{}}
	rep.Differences = append(rep.Differences, compareCarry("config", string(text), true, anchors, want)...)
	if loaded, err := os.ReadFile(filepath.Join(filepath.Dir(configPath), acl.SlotFragmentLoadedFile)); err == nil {
		rep.Compared = append(rep.Compared, "loaded")
		rep.Differences = append(rep.Differences, compareCarry("loaded", string(loaded), false, anchors, want)...)
	}
	rep.InSync = len(rep.Differences) == 0
	return rep, nil
}

// handleCarriageDrift serves GET /api/carriage/drift (operator).
func (s *Server) handleCarriageDrift(w http.ResponseWriter, r *http.Request) {
	if s.cfg.ACLConfigPath == "" {
		writeError(w, http.StatusServiceUnavailable, "config", "this portal has no router config path (PORTAL_ACL_CONFIG)")
		return
	}
	rep, err := CarriageDrift(r.Context(), s.reg, s.cfg, s.cfg.ACLConfigPath)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	writeJSON(w, http.StatusOK, rep)
}
