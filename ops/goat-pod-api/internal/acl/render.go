// Package acl renders the carriage part of the pod router's access control into the router
// configuration first-boot rendered from the pod template.
//
// The template (zagros-moon-pod host/zenoh-router.json5.tmpl) carries three marker lines, one
// in each of the access_control rules, subjects and policies arrays. Each marker line is
// replaced with the generated entries; everything else in the file is kept byte for byte.
package acl

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
)

// Markers, each alone on its line in the rendered base configuration.
const (
	MarkerRules    = "// @carriage-rules"
	MarkerSubjects = "// @carriage-subjects"
	MarkerPolicies = "// @carriage-policies"
)

// Subject ids the template defines and the carriage policies refer to.
const (
	FabricSubject = "fabric-peer"
	PodSubject    = "pod-peer"
)

var (
	writeMsgs    = []string{"put", "delete", "declare_queryable", "reply", "liveliness_token"}
	queryMsgs    = []string{"query"}
	interestMsgs = []string{"declare_subscriber", "query", "declare_liveliness_subscriber", "liveliness_query"}
	deliverMsgs  = []string{"put", "delete", "declare_queryable", "reply", "liveliness_token"}
)

// Options shape the rendering.
type Options struct {
	// PodSlot is this pod's slot; used for the health key.
	PodSlot string
	// HealthKey, when set, lets the pod's own identity publish its carriage health there and
	// forward it upstream.
	HealthKey string
	// Reserved CNs are never admitted as carried principals (the pod's own and the fabric
	// router's), so a rule cannot widen what those identities may do.
	Reserved []string
}

type rule struct {
	ID       string   `json:"id"`
	Messages []string `json:"messages"`
	Flows    []string `json:"flows"`
	Perm     string   `json:"permission"`
	Keys     []string `json:"key_exprs"`
}

type subject struct {
	ID        string   `json:"id"`
	CNs       []string `json:"cert_common_names"`
	Protocols []string `json:"link_protocols"`
}

type policy struct {
	Rules    []string `json:"rules"`
	Subjects []string `json:"subjects"`
}

// Fragments are the generated entries for each marker.
type Fragments struct {
	Rules    []rule
	Subjects []subject
	Policies []policy
}

// Build generates the carriage entries for sources (the effective set) and read.
func Build(sources []generation.Source, read []string, opt Options) (Fragments, error) {
	var f Fragments
	reserved := map[string]bool{}
	for _, cn := range opt.Reserved {
		reserved[cn] = true
	}

	// One subject per distinct principal CN.
	cnSet := map[string]bool{}
	for _, s := range sources {
		if err := generation.ValidateSource(s); err != nil {
			return f, err
		}
		for _, p := range s.Principals {
			if reserved[p] {
				return f, fmt.Errorf("source %s names %q, which is the pod's or the fabric router's own identity", s.Slot, p)
			}
			cnSet[p] = true
		}
	}
	cns := make([]string, 0, len(cnSet))
	for cn := range cnSet {
		cns = append(cns, cn)
	}
	sort.Strings(cns)
	subjectOf := map[string]string{}
	for i, cn := range cns {
		id := fmt.Sprintf("carriage-cn-%d", i+1)
		subjectOf[cn] = id
		f.Subjects = append(f.Subjects, subject{ID: id, CNs: []string{cn}, Protocols: []string{"tls", "quic"}})
	}

	var upstream []string
	var carriedSubjects []string
	seenSubject := map[string]bool{}
	for _, s := range sources {
		var subs []string
		for _, p := range s.Principals {
			subs = append(subs, subjectOf[p])
			if !seenSubject[subjectOf[p]] {
				seenSubject[subjectOf[p]] = true
				carriedSubjects = append(carriedSubjects, subjectOf[p])
			}
		}
		sort.Strings(subs)
		base := "carriage-src-" + s.Slot
		f.Rules = append(f.Rules,
			// The source writes its own slot as it enters the pod...
			rule{ID: base + "-in", Messages: writeMsgs, Flows: []string{"ingress"}, Perm: "allow", Keys: s.Write},
			// ...and the pod forwards it to the fabric router.
			rule{ID: base + "-up", Messages: writeMsgs, Flows: []string{"egress"}, Perm: "allow", Keys: s.Write},
			// Queries for what the source serves arrive from the fabric and go to the source.
			rule{ID: base + "-query-in", Messages: queryMsgs, Flows: []string{"ingress"}, Perm: "allow", Keys: s.Write},
			rule{ID: base + "-query-out", Messages: queryMsgs, Flows: []string{"egress"}, Perm: "allow", Keys: s.Write},
		)
		f.Policies = append(f.Policies,
			policy{Rules: []string{base + "-in", base + "-query-out"}, Subjects: subs},
		)
		upstream = append(upstream, base+"-up", base+"-query-in")
	}
	if len(upstream) > 0 {
		f.Policies = append(f.Policies, policy{Rules: upstream, Subjects: []string{FabricSubject}})
	}

	if len(read) > 0 && len(carriedSubjects) > 0 {
		for _, r := range read {
			if r == "**" || !generation.ValidKeyExpr(r) {
				return f, fmt.Errorf("read expression %q refused", r)
			}
		}
		sort.Strings(carriedSubjects)
		f.Rules = append(f.Rules,
			rule{ID: "carriage-read-interest-in", Messages: interestMsgs, Flows: []string{"ingress"}, Perm: "allow", Keys: read},
			rule{ID: "carriage-read-interest-up", Messages: interestMsgs, Flows: []string{"egress"}, Perm: "allow", Keys: read},
			rule{ID: "carriage-read-data-in", Messages: deliverMsgs, Flows: []string{"ingress"}, Perm: "allow", Keys: read},
			rule{ID: "carriage-read-data-out", Messages: deliverMsgs, Flows: []string{"egress"}, Perm: "allow", Keys: read},
		)
		f.Policies = append(f.Policies,
			policy{Rules: []string{"carriage-read-interest-in", "carriage-read-data-out"}, Subjects: carriedSubjects},
			policy{Rules: []string{"carriage-read-interest-up", "carriage-read-data-in"}, Subjects: []string{FabricSubject}},
		)
	}

	if opt.HealthKey != "" {
		if !generation.ValidKeyExpr(opt.HealthKey) {
			return f, fmt.Errorf("health key %q refused", opt.HealthKey)
		}
		f.Rules = append(f.Rules,
			rule{ID: "carriage-health-in", Messages: []string{"put"}, Flows: []string{"ingress"}, Perm: "allow", Keys: []string{opt.HealthKey}},
			rule{ID: "carriage-health-up", Messages: []string{"put"}, Flows: []string{"egress"}, Perm: "allow", Keys: []string{opt.HealthKey}},
		)
		f.Policies = append(f.Policies,
			policy{Rules: []string{"carriage-health-in"}, Subjects: []string{PodSubject}},
			policy{Rules: []string{"carriage-health-up"}, Subjects: []string{FabricSubject}},
		)
	}
	return f, nil
}

// Render replaces the three markers in base with the generated entries.
func Render(base []byte, sources []generation.Source, read []string, opt Options) ([]byte, error) {
	f, err := Build(sources, read, opt)
	if err != nil {
		return nil, err
	}
	rules, err := entries(f.Rules)
	if err != nil {
		return nil, err
	}
	subjects, err := entries(f.Subjects)
	if err != nil {
		return nil, err
	}
	policies, err := entries(f.Policies)
	if err != nil {
		return nil, err
	}
	return replaceMarkers(base, map[string][]string{
		MarkerRules: rules, MarkerSubjects: subjects, MarkerPolicies: policies,
	})
}

func entries[T any](items []T) ([]string, error) {
	out := make([]string, 0, len(items))
	for _, it := range items {
		raw, err := json.Marshal(it)
		if err != nil {
			return nil, err
		}
		out = append(out, string(raw)+",")
	}
	return out, nil
}

func replaceMarkers(base []byte, with map[string][]string) ([]byte, error) {
	lines := bytes.Split(base, []byte("\n"))
	found := map[string]int{}
	var out [][]byte
	for _, line := range lines {
		trimmed := strings.TrimSpace(string(line))
		repl, ok := with[trimmed]
		if !ok {
			out = append(out, line)
			continue
		}
		found[trimmed]++
		indent := line[:len(line)-len(bytes.TrimLeft(line, " \t"))]
		// The marker stays in the output, so a reader can see where the carriage entries begin.
		out = append(out, line)
		for _, r := range repl {
			out = append(out, append(append([]byte(nil), indent...), r...))
		}
	}
	var missing []string
	for m := range with {
		switch found[m] {
		case 1:
		case 0:
			missing = append(missing, m)
		default:
			return nil, fmt.Errorf("marker %q appears %d times in the base configuration", m, found[m])
		}
	}
	if len(missing) > 0 {
		sort.Strings(missing)
		return nil, errors.New("the base router configuration lacks the carriage markers " +
			strings.Join(missing, ", ") + "; render it from a pod template that carries them")
	}
	return bytes.Join(out, []byte("\n")), nil
}
