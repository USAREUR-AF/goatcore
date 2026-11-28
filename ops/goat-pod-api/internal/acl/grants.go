package acl

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
)

// Grants lists what the carriage entries of a router configuration allow, as a set of
// "<subject> <flow> <message> <key>" strings. A subject is a certificate CN, or the id of a
// subject the template defines. Entry ids do not count: they shift when a source is added.
func Grants(conf []byte) (map[string]bool, error) {
	rules, cns := map[string]rule{}, map[string][]string{}
	var policies []policy
	for _, line := range bytes.Split(conf, []byte("\n")) {
		text := strings.TrimSuffix(strings.TrimSpace(string(line)), ",")
		if !strings.HasPrefix(text, "{") || !strings.Contains(text, "carriage-") {
			continue
		}
		var e struct {
			ID       string   `json:"id"`
			Messages []string `json:"messages"`
			Flows    []string `json:"flows"`
			Keys     []string `json:"key_exprs"`
			CNs      []string `json:"cert_common_names"`
			Rules    []string `json:"rules"`
			Subjects []string `json:"subjects"`
		}
		if err := json.Unmarshal([]byte(text), &e); err != nil {
			return nil, fmt.Errorf("carriage entry: %w", err)
		}
		switch {
		case len(e.Keys) > 0:
			rules[e.ID] = rule{Messages: e.Messages, Flows: e.Flows, Keys: e.Keys}
		case len(e.CNs) > 0:
			cns[e.ID] = e.CNs
		default:
			policies = append(policies, policy{Rules: e.Rules, Subjects: e.Subjects})
		}
	}
	out := map[string]bool{}
	for _, p := range policies {
		for _, id := range p.Rules {
			r, ok := rules[id]
			if !ok {
				return nil, fmt.Errorf("a carriage policy names rule %q, which is not there", id)
			}
			for _, sid := range p.Subjects {
				names, ok := cns[sid]
				if !ok {
					names = []string{"id:" + sid}
				}
				for _, n := range names {
					for _, f := range r.Flows {
						for _, m := range r.Messages {
							for _, k := range r.Keys {
								out[n+" "+f+" "+m+" "+k] = true
							}
						}
					}
				}
			}
		}
	}
	return out, nil
}

// Writers lists, per slot, what certificates may write as it enters the router under conf.
func Writers(conf []byte) (map[string]map[string]bool, error) {
	grants, err := Grants(conf)
	if err != nil {
		return nil, err
	}
	out := map[string]map[string]bool{}
	for g := range grants {
		f := strings.SplitN(g, " ", 4)
		if strings.HasPrefix(f[0], "id:") || f[1] != "ingress" || f[2] != "put" {
			continue
		}
		slot, _, _ := strings.Cut(strings.TrimPrefix(f[3], "volatile/"), "/")
		if out[slot] == nil {
			out[slot] = map[string]bool{}
		}
		out[slot][g] = true
	}
	return out, nil
}

// Removes reports whether moving from live to next takes away any carriage grant. A live
// configuration that cannot be read counts as a removal.
func Removes(live, next []byte) bool {
	have, err := Grants(live)
	if err != nil {
		return true
	}
	want, err := Grants(next)
	if err != nil {
		return true
	}
	for g := range have {
		if !want[g] {
			return true
		}
	}
	return false
}

// Carries reports whether conf holds any carriage grant, or cannot be read.
func Carries(conf []byte) bool {
	g, err := Grants(conf)
	return err != nil || len(g) > 0
}
