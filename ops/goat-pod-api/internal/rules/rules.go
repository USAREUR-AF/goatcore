// Package rules computes a pod's effective rule set from the central generation it holds, the
// central rules it has seen before (for reactivation), and its local ledger.
package rules

import (
	"sort"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/ledger"
)

// Rule states.
const (
	StateActive  = "active"  // carried now
	StateExpired = "expired" // past its end date
	StateRevoked = "revoked" // cut off by a local revoke
	StateArchive = "archived"
)

// Ratification values for local changes.
const (
	RatPending  = "pending"
	RatRatified = "ratified"
	RatLapsed   = "lapsed"
)

// Archived is a central rule this pod held in an earlier generation.
type Archived struct {
	Source   generation.Source `json:"source"`
	LastSeen time.Time         `json:"last_seen"`
}

// Rule is one rule as `goat pod rules ls` shows it.
type Rule struct {
	ID         string     `json:"id"`
	Source     string     `json:"source"`
	SourceSlot string     `json:"source_slot"`
	Kind       string     `json:"kind"`
	Origin     string     `json:"origin"` // central | local
	State      string     `json:"state"`
	Until      *time.Time `json:"until"`
	Principals []string   `json:"principals"`
	Write      []string   `json:"write"`
	Reason     string     `json:"reason,omitempty"`
	// Authority made the local change (local rules and locally changed central rules).
	Authority *generation.Authority `json:"authority,omitempty"`
	// Ratification is the portal's decision on the local change behind this rule: pending,
	// ratified or lapsed; null for a central rule no local change touched.
	Ratification *string `json:"ratification"`
	// LocalUntil is set when a local extend or reactivate moved a central rule's end date.
	LocalUntil *time.Time `json:"local_until,omitempty"`
	// EndedAt is when the rule stopped being carried; reactivation is allowed within the window
	// after it.
	EndedAt *time.Time `json:"ended_at,omitempty"`
}

// Revocation is a local revoke in force.
type Revocation struct {
	Source string `json:"source"`
	Reason string `json:"reason"`
	Seq    int64  `json:"seq"`
}

// Inputs feed Compute.
type Inputs struct {
	Gen     *generation.Generation
	Archive map[string]Archived
	Ledger  []ledger.Record
	Now     time.Time
}

// Result is the computed rule set.
type Result struct {
	// Effective is what the router renders: one entry per carried slot.
	Effective []generation.Source
	// Rules lists every rule the pod knows of, carried or not.
	Rules []Rule
	// Revocations lists local revokes in force.
	Revocations []Revocation
	// Decisions maps a ledger seq to the portal's decision, from the held generation.
	Decisions map[int64]string
}

type localRule struct {
	rule    Rule
	revoked bool
	decided string
}

type override struct {
	source string
	until  time.Time
	seq    int64
	auth   generation.Authority
}

func ptr(s string) *string { return &s }

// Compute folds the inputs. It is pure: the same inputs give the same result.
func Compute(in Inputs) Result {
	now := in.Now
	res := Result{Decisions: map[int64]string{}}
	byRule := map[string]string{}
	if in.Gen != nil {
		for _, d := range in.Gen.LocalDecisions {
			res.Decisions[d.Seq] = d.State
			if d.LocalRuleID != "" {
				byRule[d.LocalRuleID] = d.State
			}
		}
	}

	central := map[string]generation.Source{}
	inGen := map[string]bool{}
	if in.Gen != nil {
		for _, s := range in.Gen.Sources {
			central[s.RuleID] = s
			inGen[s.RuleID] = true
		}
	}
	for id, a := range in.Archive {
		if !inGen[id] {
			central[id] = a.Source
		}
	}

	locals := map[string]*localRule{}
	var localOrder []string
	overrides := map[string]override{}
	revokes := map[string]Revocation{}

	for _, r := range in.Ledger {
		e := r.Entry
		if res.Decisions[e.Seq] != "" {
			// The portal has decided this change: a ratified one now arrives centrally, a lapsed
			// one no longer applies. Either way it stops acting locally.
			if e.Action == ledger.ActionAllow {
				if _, ok := locals[e.RuleID]; !ok {
					localOrder = append(localOrder, e.RuleID)
				}
				until := e.UntilTime()
				locals[e.RuleID] = &localRule{rule: Rule{
					ID: e.RuleID, Source: e.SourceSlot, Kind: "local", Origin: "local", Until: &until,
					Principals: r.Local.Principals, Write: r.Local.Write, Reason: e.Reason,
					Authority: &generation.Authority{Identity: e.Authority.Identity, CertCN: e.Authority.CertCN},
				}, decided: res.Decisions[e.Seq]}
			}
			if e.Action == ledger.ActionRevoke {
				// The decision frees central rules only. What was granted locally before the
				// revoke stays cut off until a later local allow or reactivate.
				for _, lr := range locals {
					if lr.rule.Source == e.SourceSlot {
						lr.revoked = true
					}
				}
				for id, ov := range overrides {
					if ov.source == e.SourceSlot {
						delete(overrides, id)
					}
				}
			}
			continue
		}
		switch e.Action {
		case ledger.ActionAllow:
			until := e.UntilTime()
			if _, ok := locals[e.RuleID]; !ok {
				localOrder = append(localOrder, e.RuleID)
			}
			locals[e.RuleID] = &localRule{rule: Rule{
				ID: e.RuleID, Source: e.SourceSlot, Kind: "local", Origin: "local", Until: &until,
				Principals: r.Local.Principals, Write: r.Local.Write, Reason: e.Reason,
				Authority: &generation.Authority{Identity: e.Authority.Identity, CertCN: e.Authority.CertCN},
			}, decided: byRule[e.RuleID]}
			delete(revokes, e.SourceSlot)
		case ledger.ActionExtend, ledger.ActionReactivate:
			until := e.UntilTime()
			if lr, ok := locals[e.RuleID]; ok {
				lr.rule.Until = &until
				lr.rule.Reason = e.Reason
				if e.Action == ledger.ActionReactivate {
					lr.revoked = false
					delete(revokes, lr.rule.Source)
				}
			} else {
				overrides[e.RuleID] = override{source: e.SourceSlot, until: until, seq: e.Seq, auth: e.Authority}
				if e.Action == ledger.ActionReactivate {
					delete(revokes, e.SourceSlot)
				}
			}
		case ledger.ActionRevoke:
			// A local revoke holds until the portal decides it or a later local allow or
			// reactivate lifts it.
			revokes[e.SourceSlot] = Revocation{Source: e.SourceSlot, Reason: e.Reason, Seq: e.Seq}
			for _, lr := range locals {
				if lr.rule.Source == e.SourceSlot {
					lr.revoked = true
				}
			}
		}
	}
	revokedNow := func(slot string) bool {
		_, ok := revokes[slot]
		return ok
	}

	carried := map[string]*generation.Source{}
	carry := func(s generation.Source, until *time.Time) {
		c, ok := carried[s.Slot]
		if !ok {
			cp := s
			cp.Principals = append([]string(nil), s.Principals...)
			cp.Write = append([]string(nil), s.Write...)
			cp.Until = until
			carried[s.Slot] = &cp
			return
		}
		c.Principals = union(c.Principals, s.Principals)
		c.Write = union(c.Write, s.Write)
		switch {
		case c.Until == nil:
		case until == nil:
			c.Until = nil
		case until.After(*c.Until):
			c.Until = until
		}
	}

	ids := make([]string, 0, len(central))
	for id := range central {
		ids = append(ids, id)
	}
	sort.Strings(ids)
	for _, id := range ids {
		s := central[id]
		r := Rule{ID: id, Source: s.Slot, Kind: s.Kind, Origin: "central", Until: s.Until,
			Principals: s.Principals, Write: s.Write}
		until := s.Until
		ov, hasOv := overrides[id]
		if hasOv {
			u := ov.until
			r.LocalUntil = &u
			r.Ratification = ptr(RatPending)
			r.Authority = &generation.Authority{Identity: ov.auth.Identity, CertCN: ov.auth.CertCN}
			until = &u
		}
		switch {
		case !inGen[id] && !hasOv:
			r.State = StateArchive
			if a, ok := in.Archive[id]; ok {
				ended := a.LastSeen
				if s.Until != nil && s.Until.Before(ended) {
					ended = *s.Until
				}
				r.EndedAt = &ended
			}
		case until != nil && !until.After(now):
			r.State = StateExpired
			ended := *until
			r.EndedAt = &ended
		case revokedNow(s.Slot):
			r.State = StateRevoked
		default:
			r.State = StateActive
			carry(s, until)
		}
		res.Rules = append(res.Rules, r)
	}

	for _, id := range localOrder {
		lr := locals[id]
		r := lr.rule
		r.SourceSlot = r.Source
		switch lr.decided {
		case RatRatified, RatLapsed:
			r.Ratification = ptr(lr.decided)
			r.State = StateArchive
			ended := now
			if r.Until != nil && r.Until.Before(now) {
				ended = *r.Until
			}
			r.EndedAt = &ended
			res.Rules = append(res.Rules, r)
			continue
		}
		r.Ratification = ptr(RatPending)
		switch {
		case lr.revoked || revokedNow(r.Source):
			r.State = StateRevoked
		case !r.Until.After(now):
			r.State = StateExpired
			ended := *r.Until
			r.EndedAt = &ended
		default:
			r.State = StateActive
			carry(generation.Source{Slot: r.Source, RuleID: r.ID, Kind: "local", Principals: r.Principals,
				Write: r.Write}, r.Until)
		}
		res.Rules = append(res.Rules, r)
	}
	for i := range res.Rules {
		res.Rules[i].SourceSlot = res.Rules[i].Source
	}

	slots := make([]string, 0, len(carried))
	for slot := range carried {
		slots = append(slots, slot)
	}
	sort.Strings(slots)
	for _, slot := range slots {
		c := carried[slot]
		sort.Strings(c.Principals)
		sort.Strings(c.Write)
		res.Effective = append(res.Effective, *c)
	}
	for _, r := range revokes {
		res.Revocations = append(res.Revocations, r)
	}
	sort.Slice(res.Revocations, func(i, j int) bool { return res.Revocations[i].Source < res.Revocations[j].Source })
	return res
}

// NextExpiry returns the earliest end date after now of a rule that is carried now.
func (r Result) NextExpiry(now time.Time) (time.Time, bool) {
	var next time.Time
	for _, rule := range r.Rules {
		until := rule.Until
		if rule.LocalUntil != nil {
			until = rule.LocalUntil
		}
		if rule.State != StateActive || until == nil || !until.After(now) {
			continue
		}
		if next.IsZero() || until.Before(next) {
			next = *until
		}
	}
	return next, !next.IsZero()
}

// Find returns the rule with id, if known.
func (r Result) Find(id string) (Rule, bool) {
	for _, rule := range r.Rules {
		if rule.ID == id {
			return rule, true
		}
	}
	return Rule{}, false
}

// KnownSource returns the principals and write set last associated with slot in any rule.
func (r Result) KnownSource(slot string) (principals, write []string, ok bool) {
	for _, rule := range r.Rules {
		if rule.Source == slot && len(rule.Principals) > 0 {
			principals = union(principals, rule.Principals)
			write = union(write, rule.Write)
			ok = true
		}
	}
	sort.Strings(principals)
	sort.Strings(write)
	return principals, write, ok
}

// NextArchive returns the archive after adopting next in place of prev: every central rule
// of prev missing from next is archived as of now; entries older than keep are dropped.
func NextArchive(old map[string]Archived, prev, next *generation.Generation, now time.Time, keep time.Duration) map[string]Archived {
	out := map[string]Archived{}
	for id, a := range old {
		if now.Sub(a.LastSeen) <= keep {
			out[id] = a
		}
	}
	if prev != nil {
		for _, s := range prev.Sources {
			out[s.RuleID] = Archived{Source: s, LastSeen: now}
		}
	}
	if next != nil {
		for _, s := range next.Sources {
			delete(out, s.RuleID)
		}
	}
	return out
}

func union(a, b []string) []string {
	seen := map[string]bool{}
	var out []string
	for _, s := range append(append([]string(nil), a...), b...) {
		if !seen[s] {
			seen[s] = true
			out = append(out, s)
		}
	}
	return out
}
