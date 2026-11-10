// org-backfill-dryrun — a zero-mutation legacy-duplicate AWARENESS report.
//
// RE-SCOPED (ADR 1112 Amendment 1): the bulk registry import this
// tool once proposed was DROPPED — the registry accretes incrementally at
// onboarding. The report's evidence classes remain useful as roster/manifest
// HYGIENE findings (same-email pairs are real legacy duplicate identities);
// nothing here feeds a registry apply.
//
// Reads the committed roster projection (docs/operators/roster.json) and the
// current org catalog (ops/identity/orgs/<env>.yaml), and emits a reviewable
// report proposing:
//
//  1. IMPORT — roster orgs absent from the registry, as org rows carrying
//     kind/nation from the roster and provenance `open-source` (the roster is
//     manifest-derived, not operator-confirmed per row);
//  2. REVIEW-PAIRS — org id pairs the matcher scores into the high band
//     against each other. NEVER auto-merged: every pair is a human decision
//     (similar spelling is not the same org — the allen pair is exactly why),
//     resolved later as `goat admin org merge` (with the governance-union
//     acknowledgment) or dismissed as `--despite` negative pairs;
//  3. CHAIN — parent attachments already decided by canon (the design
//     doc's worked example: existing flat orgs attached under a
//     to-be-created root — flat is the explicit interim, attaching
//     parents re-keys nothing).
//
// This tool MUTATES NOTHING: no network, no DB. Apply is a separate operator
// act composed only of increments 1–3's audited, individually-reversible
// operations — there is deliberately no bulk-undo to trust (§13.4).
//
//	Run: cd ops/goat-id-console && go run ./tools/org-backfill-dryrun \
//	       --roster ../../docs/operators/roster.json \
//	       --catalog ../../ops/identity/orgs/<goatnet>.yaml > report.md
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"sort"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/nation"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/orgstore"
)

type rosterOrg struct {
	ID      string  `json:"id"`
	Kind    *string `json:"kind"`
	Nation  *string `json:"nation"`
	Parent  *string `json:"parent"`
	Members []struct {
		Handle    string `json:"handle"`
		State     string `json:"state"`
		EmailHash string `json:"email_hash"`
	} `json:"members"`
}

func str(p *string) string {
	if p == nil {
		return ""
	}
	return *p
}

func main() {
	rosterPath := flag.String("roster", "docs/operators/roster.json", "committed roster projection")
	catalogPath := flag.String("catalog", "", "committed org catalog (yaml), e.g. ops/identity/orgs/<goatnet>.yaml (required)")
	flag.Parse()
	if *catalogPath == "" {
		fmt.Fprintln(os.Stderr, "--catalog is required (e.g. ops/identity/orgs/<goatnet>.yaml)")
		os.Exit(2)
	}

	raw, err := os.ReadFile(*rosterPath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "read roster: %v\n", err)
		os.Exit(1)
	}
	var roster struct {
		Orgs []rosterOrg `json:"orgs"`
	}
	if err := json.Unmarshal(raw, &roster); err != nil {
		fmt.Fprintf(os.Stderr, "parse roster: %v\n", err)
		os.Exit(1)
	}

	// The catalog is generated YAML with a known flat shape; parse the id
	// lines rather than pulling a YAML dependency into the module for a
	// report generator. (Format: sync-org-catalog.py, keys sorted, one
	// `- id: <slug>` per org.)
	inCatalog := map[string]bool{}
	if cat, err := os.ReadFile(*catalogPath); err == nil {
		for _, line := range strings.Split(string(cat), "\n") {
			line = strings.TrimSpace(line)
			if rest, ok := strings.CutPrefix(line, "- id: "); ok {
				inCatalog[strings.TrimSpace(rest)] = true
			}
		}
	}

	var imports []rosterOrg
	valid := make([]rosterOrg, 0, len(roster.Orgs))
	skipped := 0
	for _, o := range roster.Orgs {
		if !orgstore.ValidID(o.ID) {
			skipped++ // the "(internal / no vendor)" bucket etc. oss-allow: generic bucket label
			continue
		}
		valid = append(valid, o)
		if !inCatalog[o.ID] {
			imports = append(imports, o)
		}
	}
	sort.Slice(imports, func(i, j int) bool { return imports[i].ID < imports[j].ID })

	// Self-join through the real matcher: rank every valid org's id against
	// the rest with orgstore.RankCandidates, the ranking Suggest uses, over an
	// in-process catalog (no database). High-band pairs are REVIEW items —
	// never merges.
	catalog := make([]orgstore.OrgView, 0, len(valid))
	seeded := map[string]bool{}
	for _, o := range valid {
		if seeded[o.ID] {
			fmt.Fprintf(os.Stderr, "seed %s: duplicate org id in roster\n", o.ID)
			os.Exit(1)
		}
		seeded[o.ID] = true
		catalog = append(catalog, orgstore.OrgView{Org: orgstore.Org{
			ID: o.ID, Nation: nation.Normalize(str(o.Nation)), Provenance: "open-source", Status: "active",
		}})
	}
	type pair struct {
		A, B     string
		Score    float64
		Evidence string // same-email | same-handle-diff-email | none
		Detail   string
	}
	// Member evidence: the roster's email_hash is ground truth for "the
	// same person stands behind both orgs" — the strongest duplicate signal
	// we hold. A same-name-different-email pair is NOT that: it may be a
	// deliberate second identity (the -2 family includes both kinds), so it
	// is labeled distinctly and never presented as a confirmed duplicate.
	emailsOf := map[string]map[string]bool{}
	handlesOf := map[string]map[string]bool{}
	normHandle := func(h string) string {
		h = strings.ToLower(strings.TrimSpace(h))
		for i := len(h) - 1; i > 0; i-- {
			if h[i] == '-' {
				suffix := h[i+1:]
				digits := len(suffix) > 0
				for _, r := range suffix {
					if r < '0' || r > '9' {
						digits = false
						break
					}
				}
				if digits {
					return h[:i]
				}
				break
			}
			if h[i] < '0' || h[i] > '9' {
				break
			}
		}
		return h
	}
	for _, o := range valid {
		emailsOf[o.ID] = map[string]bool{}
		handlesOf[o.ID] = map[string]bool{}
		for _, m := range o.Members {
			if m.EmailHash != "" {
				emailsOf[o.ID][m.EmailHash] = true
			}
			if m.Handle != "" {
				handlesOf[o.ID][normHandle(m.Handle)] = true
			}
		}
	}
	evidenceFor := func(a, b string) (string, string) {
		var sharedEmails, sharedHandles int
		for e := range emailsOf[a] {
			if emailsOf[b][e] {
				sharedEmails++
			}
		}
		for h := range handlesOf[a] {
			if handlesOf[b][h] {
				sharedHandles++
			}
		}
		switch {
		case sharedEmails > 0:
			return "same-email", fmt.Sprintf("%d shared member email(s) — same person both sides", sharedEmails)
		case sharedHandles > 0:
			return "same-handle-diff-email", fmt.Sprintf("%d matching handle(s), DIFFERENT emails — possibly a deliberate second identity", sharedHandles)
		default:
			return "none", "no shared members — name similarity only"
		}
	}

	seen := map[string]bool{}
	var pairs []pair
	for _, o := range valid {
		cs := orgstore.RankCandidates(catalog, nil, o.ID, "", 6)
		for _, c := range cs {
			if c.ID == o.ID || c.Band != orgstore.BandHigh {
				continue
			}
			a, b := o.ID, c.ID
			if a > b {
				a, b = b, a
			}
			if seen[a+"|"+b] {
				continue
			}
			seen[a+"|"+b] = true
			ev, detail := evidenceFor(a, b)
			pairs = append(pairs, pair{A: a, B: b, Score: c.Score, Evidence: ev, Detail: detail})
		}
	}
	rank := map[string]int{"same-email": 0, "same-handle-diff-email": 1, "none": 2}
	sort.Slice(pairs, func(i, j int) bool {
		if rank[pairs[i].Evidence] != rank[pairs[j].Evidence] {
			return rank[pairs[i].Evidence] < rank[pairs[j].Evidence]
		}
		return pairs[i].Score > pairs[j].Score
	})

	// ---- the report -----------------------------------------------------
	fmt.Println("# Org-registry backfill — DRY RUN (zero mutations; §13.4)")
	fmt.Println()
	fmt.Printf("Roster orgs: %d valid (%d non-slug buckets skipped). Already in the registry catalog: %d.\n\n",
		len(valid), skipped, len(valid)-len(imports))
	fmt.Println("Apply is a separate operator act built ONLY from increments 1–3's audited,")
	fmt.Println("individually-reversible operations. There is deliberately no bulk-undo.")
	fmt.Println()

	fmt.Printf("## 1. IMPORT — %d org rows to create (provenance `open-source`)\n\n", len(imports))
	fmt.Println("| id | kind | nation | active members |")
	fmt.Println("|---|---|---|---|")
	for _, o := range imports {
		active := 0
		for _, m := range o.Members {
			s := strings.ToLower(m.State)
			if s == "" || s == "active" {
				active++
			}
		}
		fmt.Printf("| %s | %s | %s | %d |\n", o.ID, str(o.Kind), str(o.Nation), active)
	}
	fmt.Println()

	counts := map[string]int{}
	for _, p := range pairs {
		counts[p.Evidence]++
	}
	fmt.Printf("## 2. REVIEW-PAIRS — %d high-band pairs (HUMAN DECISION EACH; never auto-merged)\n\n", len(pairs))
	fmt.Println("Similar spelling is NOT the same org. Evidence classes (roster ground truth):")
	fmt.Printf("- **same-email (%d)** — a shared member email hash: the same person stands behind both orgs. The strongest duplicate signal; still human-confirmed.\n", counts["same-email"])
	fmt.Printf("- **same-handle-diff-email (%d)** — matching person name, DIFFERENT emails: possibly a deliberate second identity, NOT a confirmed duplicate.\n", counts["same-handle-diff-email"])
	fmt.Printf("- **none (%d)** — name similarity only; most are family-prefix siblings, likely distinct.\n\n", counts["none"])
	fmt.Println("For each pair, decide: `goat admin org merge <dup> --into <canonical> --acknowledge-governance-union`,")
	fmt.Println("or dismiss as distinct (a `--despite` negative pair on the next register).")
	fmt.Println()
	fmt.Println("| evidence | score | a | b | detail |")
	fmt.Println("|---|---|---|---|---|")
	for _, p := range pairs {
		fmt.Printf("| %s | %.2f | %s | %s | %s |\n", p.Evidence, p.Score, p.A, p.B, p.Detail)
	}
	fmt.Println()

	fmt.Println("## 3. CHAIN — canon-decided parent attachments")
	fmt.Println()
	fmt.Println("Per the design doc's worked example (§9): register the root org")
	fmt.Println("(partner, its nation, operator-verified) and attach the existing flat")
	fmt.Println("orgs under it — zero credential/grant churn by construction:")
	fmt.Println()
	fmt.Println("    goat admin org register <root-org> --kind partner --nation <ISO3> \\")
	fmt.Println("      --display-name \"<Root Org Name>\"")
	fmt.Println("    goat admin org set-parent <flat-org> --parent <root-org>")
}
