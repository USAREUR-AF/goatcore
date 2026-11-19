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
	// StateSettled: a local rule the portal has acknowledged; from the next central generation on,
	// the portal decides it (ratified rules arrive in the generation, lapsed ones do not).
	StateSettled = "settled"
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
	Kind       string     `json:"kind"`
	Origin     string     `json:"origin"` // central | local
	State      string     `json:"state"`
	Until      *time.Time `json:"until"`
	Principals []string   `json:"principals"`
	Write      []string   `json:"write"`
	Reason     string     `json:"reason,omitempty"`
	// Pending is true for a local change the portal has not acknowledged.
	Pending bool `json:"pending"`
	// LocalUntil is set when a local extend or reactivate moved a central rule's end date.
	LocalUntil *time.Time `json:"local_until,omitempty"`
	// EndedAt is when the rule stopped being carried (its end date, or when it left the
	// generation); reactivation is allowed within the window after it.
	EndedAt *time.Time `json:"ended_at,omitempty"`
}

// Revocation is an active local revoke.
type Revocation struct {
	Source string    `json:"source"`
	Until  time.Time `json:"until"`
	Reason string    `json:"reason"`
	Seq    int64     `json:"seq"`
}

// Inputs feed Compute.
type Inputs struct {
	Gen     *generation.Generation
	Archive map[string]Archived
	Ledger  []ledger.Entry
	Ack     ledger.Ack
	Now     time.Time
}

// Result is the computed rule set.
type Result struct {
	// Effective is what the router renders: one entry per carried slot.
	Effective []generation.Source
	// Rules lists every rule the pod knows of, carried or not.
	Rules []Rule
	// Revocations lists local revokes still in force.
	Revocations []Revocation
}

type localRule struct {
	rule    Rule
	lastSeq int64
	revoked bool
}

type override struct {
	until time.Time
	seq   int64
}

// Compute folds the inputs. It is pure: the same inputs give the same result.
func Compute(in Inputs) Result {
	now := in.Now
	settled := func(seq int64) bool {
		return seq <= in.Ack.LastSeq && in.Gen != nil && in.Gen.Generation > in.Ack.Generation
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

	for _, e := range in.Ledger {
		switch e.Action {
		case ledger.ActionAllow:
			until := e.Until
			if _, ok := locals[e.RuleID]; !ok {
				localOrder = append(localOrder, e.RuleID)
			}
			locals[e.RuleID] = &localRule{rule: Rule{
				ID: e.RuleID, Source: e.Source, Kind: "local", Origin: "local", Until: &until,
				Principals: e.Principals, Write: e.Write, Reason: e.Reason,
			}, lastSeq: e.Seq}
			delete(revokes, e.Source)
		case ledger.ActionExtend, ledger.ActionReactivate:
			if lr, ok := locals[e.RuleID]; ok {
				until := e.Until
				lr.rule.Until = &until
				lr.lastSeq = e.Seq
				lr.rule.Reason = e.Reason
				if e.Action == ledger.ActionReactivate {
					lr.revoked = false
					delete(revokes, lr.rule.Source)
				}
			} else {
				overrides[e.RuleID] = override{until: e.Until, seq: e.Seq}
				if e.Action == ledger.ActionReactivate {
					delete(revokes, e.Source)
				}
			}
		case ledger.ActionRevoke:
			revokes[e.Source] = Revocation{Source: e.Source, Until: e.Until, Reason: e.Reason, Seq: e.Seq}
			for _, lr := range locals {
				if lr.rule.Source == e.Source {
					lr.revoked = true
				}
			}
		}
	}
	revokedNow := func(slot string) bool {
		r, ok := revokes[slot]
		return ok && r.Until.After(now)
	}

	var res Result
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
		if hasOv && !settled(ov.seq) {
			u := ov.until
			r.LocalUntil = &u
			r.Pending = ov.seq > in.Ack.LastSeq
			until = &u
		} else {
			hasOv = false
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
		r.Pending = lr.lastSeq > in.Ack.LastSeq
		switch {
		case settled(lr.lastSeq):
			r.State = StateSettled
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
		if r.Until.After(now) {
			res.Revocations = append(res.Revocations, r)
		}
	}
	sort.Slice(res.Revocations, func(i, j int) bool { return res.Revocations[i].Source < res.Revocations[j].Source })
	return res
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
