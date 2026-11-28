package api

// Phase 4 of ADR 1251: what the core router admits from carriage rules
// (docs/design/pod-carriage-and-provenance.md §2 Rendering, §7.3).
//
// One snapshot of the registry answers two questions with the same logic:
// which sources a pod carries (its signed generation, carriage.go) and which
// certificates the core lets write a source's slot on that source's behalf
// (the carry-<anchor> subjects in the router's slot fragment). A pod's leaf CN
// is a carrier of every source its generation lists; an ingress adapter's leaf
// CN is a carrier of every keyless source that names it. The router binds
// carry-<anchor> to the slot's own write rule slot-w-<anchor>, so a carrier
// may write exactly what the source itself may write, and nothing else.

import (
	"context"
	"sort"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/carriage"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// carriageSnapshot is the registry state rendering needs, read once.
type carriageSnapshot struct {
	pods    []registry.CarriagePod
	rules   []registry.CarriageRule // active rules, every pod
	slots   []registry.CarriageSlot
	bySlot  map[string]registry.CarriageSlot
	keyless map[string]registry.KeylessSource
	optOuts map[string]bool
	orgsOf  carriageOrgs
}

// carriageOrgResolverFor is the data-roster org membership of a slot (its own
// orgs and its custodian's). No roster: no org membership.
func carriageOrgResolverFor(cfg *config.Config, rosters *access.DataRosterCache) (carriageOrgs, error) {
	if cfg == nil || cfg.RosterPath == "" {
		return func(string, *string) []string { return nil }, nil
	}
	dr, err := rosters.Load(cfg.RosterPath)
	if err != nil {
		return nil, err
	}
	return func(slotID string, enrolledBy *string) []string {
		set := map[string]bool{}
		for _, o := range dr.OrgsOfProducer(slotID) {
			set[o] = true
		}
		if enrolledBy != nil && *enrolledBy != "" {
			for _, o := range dr.OrgsOfProducer(*enrolledBy) {
				set[o] = true
			}
		}
		out := make([]string, 0, len(set))
		for o := range set {
			out = append(out, o)
		}
		sort.Strings(out)
		return out
	}, nil
}

func loadCarriageSnapshot(ctx context.Context, reg *registry.Registry, cfg *config.Config, rosters *access.DataRosterCache) (*carriageSnapshot, error) {
	orgsOf, err := carriageOrgResolverFor(cfg, rosters)
	if err != nil {
		return nil, err
	}
	sn := &carriageSnapshot{orgsOf: orgsOf}
	if sn.pods, err = reg.CarriagePods(ctx); err != nil {
		return nil, err
	}
	if sn.rules, err = reg.ListCarriageRules(ctx, registry.CarriageRuleFilter{State: registry.CarriageActive}); err != nil {
		return nil, err
	}
	if sn.slots, err = reg.LiveCarriageSlots(ctx); err != nil {
		return nil, err
	}
	if sn.keyless, err = reg.KeylessSources(ctx); err != nil {
		return nil, err
	}
	if sn.optOuts, err = reg.CarriageOptOuts(ctx); err != nil {
		return nil, err
	}
	sn.bySlot = make(map[string]registry.CarriageSlot, len(sn.slots))
	for _, sl := range sn.slots {
		sn.bySlot[sl.SlotID] = sl
	}
	return sn, nil
}

// leafCN is a live slot's current leaf CN, or "" when it has no leaf.
func (sn *carriageSnapshot) leafCN(slot string) string {
	sl, ok := sn.bySlot[slot]
	if !ok || !sl.HasLeaf {
		return ""
	}
	return registry.LeafCN(sl.SlotID, sl.Generation)
}

// principals are the CNs a pod admits as writers of slot: the source's own
// leaf, or for a keyless source its live adapters' leaves.
func (sn *carriageSnapshot) principals(slot string) []string {
	if k, ok := sn.keyless[slot]; ok {
		var out []string
		for _, a := range k.AdapterSlots() {
			if cn := sn.leafCN(a); cn != "" {
				out = append(out, cn)
			}
		}
		sort.Strings(out)
		return out
	}
	if cn := sn.leafCN(slot); cn != "" {
		return []string{cn}
	}
	return nil
}

// podSources is the sources list of pod's generation: one entry per carried
// source, under the org rule if one covers it, otherwise the rule that ends
// last; exclusions take a source off the org rule; a source no certificate
// could write for is left out.
func (sn *carriageSnapshot) podSources(pod *registry.CarriagePod) []carriage.Source {
	var rules []*registry.CarriageRule
	excluded := map[string]bool{}
	for i := range sn.rules {
		c := &sn.rules[i]
		if c.PodSlot != nil && *c.PodSlot != pod.PodSlot || c.PodOrg != nil && *c.PodOrg != pod.Org {
			continue
		}
		if c.Kind == registry.CarriageKindExclusion {
			excluded[deref(c.SourceSlot)] = true
			continue
		}
		rules = append(rules, c)
	}
	better := func(a, b *registry.CarriageRule) bool {
		switch {
		case a.Until == nil && b.Until != nil:
			return true
		case a.Until != nil && b.Until == nil:
			return false
		case a.Until != nil && b.Until != nil && *a.Until != *b.Until:
			return *a.Until > *b.Until
		}
		return a.ID < b.ID
	}
	chosen := map[string]*registry.CarriageRule{}
	for _, c := range rules {
		for _, sl := range sn.slots {
			if sl.SlotID == pod.PodSlot {
				continue // the pod's own slot is the pod template's, not a carried source
			}
			if !ruleCovers(c, sl.SlotID, sn.orgsOf(sl.SlotID, sl.EnrolledBy), pod, sn.optOuts[sl.SlotID] || excluded[sl.SlotID]) {
				continue
			}
			if cur, ok := chosen[sl.SlotID]; !ok || better(c, cur) {
				chosen[sl.SlotID] = c
			}
		}
	}
	slots := make([]string, 0, len(chosen))
	for slot := range chosen {
		slots = append(slots, slot)
	}
	sort.Strings(slots)
	out := []carriage.Source{}
	for _, slot := range slots {
		p := sn.principals(slot)
		if len(p) == 0 {
			continue
		}
		c := chosen[slot]
		var until *string
		if c.Until != nil {
			u := carriage.FormatTime(time.Unix(*c.Until, 0))
			until = &u
		}
		out = append(out, carriage.Source{Slot: slot, RuleID: c.ID, Kind: c.Kind, Until: until, Principals: p, Write: carriage.DefaultWrite(slot)})
	}
	return out
}

// carriers maps each source slot to the CNs, other than its own, that the
// core admits as writers of it: every registered pod (with a live leaf) whose
// generation lists the source, and every live adapter of a keyless source.
// Each list is sorted and free of repeats.
func (sn *carriageSnapshot) carriers() map[string][]string {
	set := map[string]map[string]bool{}
	add := func(src, cn string) {
		if set[src] == nil {
			set[src] = map[string]bool{}
		}
		set[src][cn] = true
	}
	for i := range sn.pods {
		pod := &sn.pods[i]
		podCN := sn.leafCN(pod.PodSlot)
		if podCN == "" {
			continue // a pod with no live leaf carries nothing at the core
		}
		for _, s := range sn.podSources(pod) {
			add(s.Slot, podCN)
		}
	}
	for slot, k := range sn.keyless {
		if _, live := sn.bySlot[slot]; !live {
			continue
		}
		for _, a := range k.AdapterSlots() {
			if cn := sn.leafCN(a); cn != "" {
				add(slot, cn)
			}
		}
	}
	out := make(map[string][]string, len(set))
	for src, cns := range set {
		l := make([]string, 0, len(cns))
		for cn := range cns {
			l = append(l, cn)
		}
		sort.Strings(l)
		out[src] = l
	}
	return out
}

// fragmentRosters is the roster cache the slot-fragment render uses (it runs
// outside any request, from the boot and periodic renders as well).
var fragmentRosters access.DataRosterCache
