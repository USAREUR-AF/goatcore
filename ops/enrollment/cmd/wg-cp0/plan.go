package main

// plan.go — minimal dry-run printer for the Block 22R.2 metadata
// subcommands (update-endpoints / update-ttl / add-relay) owned by
// Session C of the parallel cluster.
//
// Session A owns the canonical diff_formatter.go for the wider
// reconcile / apply pipeline; this file is deliberately scoped to
// the additive-metadata operations that don't touch live wire state
// at the wg-set / wg-syncconf layer. When A's formatter lands, this
// file's printPlan() helper can be refactored to delegate to A's
// surface; the data shape (PlanLine) is intentionally simple to make
// that retrofit mechanical.

import (
	"fmt"
	"io"
	"strings"
)

// PlanOp tags each line of the dry-run preview with the shape of
// change it represents. The tag drives the leading glyph in the
// printed output, matching docs/design/wg-cp0-declarative-substrate.md
// §3.1's operator-scannable shape: + ADD / ~ UPDATE / - REMOVE /
// = KEEP.
type PlanOp int

const (
	PlanAdd PlanOp = iota
	PlanUpdate
	PlanRemove
	PlanKeep
	// PlanReuseKeypair is a distinguished tag for metadata-only
	// operations that re-mint the bundle without rotating the
	// keypair. Surfaces prominently in the preview so the operator
	// can confirm no rotation is happening — this is the §3.1
	// "REUSING KEYPAIR" affordance.
	PlanReuseKeypair
)

// PlanLine is one row in the dry-run preview. Subject is the entity
// being changed (peer name, relay name); Detail is the human-readable
// what-changed string. Both render verbatim — no further markup.
type PlanLine struct {
	Op      PlanOp
	Subject string
	Detail  string
}

// Plan is the structured preview rendered for both --dry-run (the
// default) and --apply paths. Empty Plan is valid (operator-scannable
// "nothing to do" output).
type Plan struct {
	Command     string     // command-line shape, e.g. "wg-cp0 update-endpoints site-a --add-relay relay-b"
	StateTarget string     // what file the state plan would mutate ("ops/enrollment/wg-cp0-bundle-allowlist.json")
	State       []PlanLine // what would change in the JSON-of-record
	Apply       []PlanLine // what would change in live wire state (bundle re-mint, relay-side wg syncconf, etc.)
	Notes       []string   // free-form lines surfaced under the plan (e.g. "REUSING KEYPAIR for site-a")
}

// glyph returns the leading character for a plan op, matching the
// snitch-tier engineering style for scannable diff output.
func glyph(op PlanOp) string {
	switch op {
	case PlanAdd:
		return "+ ADD"
	case PlanUpdate:
		return "~ UPDATE"
	case PlanRemove:
		return "- REMOVE"
	case PlanKeep:
		return "= KEEP"
	case PlanReuseKeypair:
		return "* REUSING KEYPAIR"
	default:
		return "?"
	}
}

// printPlan renders the dry-run preview to out. The shape matches the
// §3.1 example block; downstream tests assert on the presence of the
// leading glyphs + the subject names.
func printPlan(out io.Writer, p Plan) {
	fmt.Fprintf(out, "PLAN — %s\n\n", p.Command)

	if p.StateTarget != "" && len(p.State) > 0 {
		fmt.Fprintf(out, "State plan (would mutate %s):\n", p.StateTarget)
		for _, l := range p.State {
			fmt.Fprintf(out, "  %s %s%s\n", glyph(l.Op), l.Subject, indentDetail(l.Detail))
		}
		fmt.Fprintln(out)
	}

	if len(p.Apply) > 0 {
		fmt.Fprintln(out, "Apply plan (would mutate live wire state):")
		for _, l := range p.Apply {
			fmt.Fprintf(out, "  %s %s%s\n", glyph(l.Op), l.Subject, indentDetail(l.Detail))
		}
		fmt.Fprintln(out)
	}

	for _, n := range p.Notes {
		fmt.Fprintln(out, n)
	}
	if len(p.Notes) > 0 {
		fmt.Fprintln(out)
	}
}

// indentDetail formats the trailing detail clause so multiline details
// (e.g. a list of relay endpoints) render with consistent left-margin
// indentation under the subject line.
func indentDetail(detail string) string {
	if detail == "" {
		return ""
	}
	lines := strings.Split(detail, "\n")
	if len(lines) == 1 {
		return ": " + lines[0]
	}
	var b strings.Builder
	b.WriteString(":")
	for _, l := range lines {
		b.WriteString("\n      ")
		b.WriteString(l)
	}
	return b.String()
}

// printApplyHint adds the closing "Re-run with --apply to execute"
// line on dry-run paths. The empty-plan case skips the hint so a
// no-op operation looks like a no-op.
func printApplyHint(out io.Writer, hasWork bool) {
	if !hasWork {
		fmt.Fprintln(out, "No changes required; live state matches declared intent.")
		return
	}
	fmt.Fprintln(out, "Re-run with --apply to execute.")
}

// printApplyCompleted is the end-of-apply success message. Matches the
// shape downstream A-authored audit emission expects to log.
func printApplyCompleted(out io.Writer, what string) {
	fmt.Fprintf(out, "Applied: %s\n", what)
}
