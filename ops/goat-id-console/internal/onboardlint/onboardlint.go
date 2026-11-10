// Package onboardlint validates operator-onboarding manifests against the
// canonical JSON Schema + the ADR 1043 identity-grammar gate, in pure Go.
//
// Why Go and not a shell-out to ops/operators/validate.py: the portal runtime
// is a Chainguard distroless single-binary image (ADR 0700) with no Python or
// shell. Baking a Python runtime in to run a linter would multiply the attack
// surface the distroless image exists to remove. Instead the *bulk* of
// validation — the JSON Schema — is the single source of truth, consumed here
// UNCHANGED via an embedded build-synced copy (ops/ci/check-onboard-schema-sync.sh
// asserts byte-equality with ops/operators/schema/operator-manifest.schema.json).
// Only the two grammar-regression rules + the display classify_shape are ported
// from validate.py; a CI parity test (onboardlint_parity_test.go) asserts this
// linter and `validate.py --json` return identical pass/fail verdicts over a
// fixture corpus, so the small duplicated surface is provably equal, not
// hopefully equal. See docs/design/operator-onboard-intake.md.
package onboardlint

import (
	"bytes"
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"time"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"golang.org/x/text/language"
	"golang.org/x/text/message"
	"gopkg.in/yaml.v3"
)

//go:embed operator-manifest.schema.json
var schemaJSON []byte

// enPrinter renders santhosh-tekuri ErrorKind messages in English. The exact
// wording is library-specific; only the verdict (any findings ⇒ not OK) is
// held equal to validate.py by the parity test.
var enPrinter = message.NewPrinter(language.English)

// defaultGrammarCutover mirrors validate.py's GRAMMAR_CUTOVER default
// (ADR 1043 Phase C). Manifests whose identity.added is ON OR AFTER this
// date must declare an affiliation block and avoid deprecated groups.
// Overridable via GOAT_GRAMMAR_CUTOVER (same env var validate.py reads), so
// the parity test can drive both linters to the same cutover.
const defaultGrammarCutover = "2026-06-16"

// deprecatedGroups are auto_groups tokens the grammar retired (ADR 1043),
// mapped to their replacement guidance. Mirrors validate.py::_DEPRECATED_GROUPS
// verbatim — OPERATOR is deliberately absent (the grammar redefined it as the
// tactical-actor role, indistinguishable from the legacy sense at the group
// level, so we cannot fail-closed on it).
var deprecatedGroups = map[string]string{
	"VENDOR":  "DEVELOPER (the role-sense of vendor; affiliation 'vendor' rides the affiliation block, not a group)",
	"OFFSITE": "a real command echelon (COMPONENT/COMMAND/…); OFFSITE is retired",
}

// Finding is one issue with a manifest. Field is a dotted instance location
// (e.g. "netbird.auto_groups.0") so the webform can highlight the offending
// input inline; "identity-grammar" / "<root>" appear for cross-field rules.
type Finding struct {
	Severity string `json:"severity"`
	Field    string `json:"field,omitempty"`
	Message  string `json:"message"`
}

// Result is the linter verdict for one manifest. Shape/Detail are the
// display classification (only meaningful when OK); they are NOT surfaced
// raw to users (the form relexes them to tier names — see the design doc).
type Result struct {
	OK       bool      `json:"ok"`
	Findings []Finding `json:"findings"`
	Shape    string    `json:"shape,omitempty"`
	Detail   string    `json:"detail,omitempty"`
}

// compiled is the package-level compiled schema (compiled once).
var compiled *jsonschema.Schema

func init() {
	doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(schemaJSON))
	if err != nil {
		panic("onboardlint: embedded schema is not valid JSON: " + err.Error())
	}
	c := jsonschema.NewCompiler()
	if err := c.AddResource("operator-manifest.schema.json", doc); err != nil {
		panic("onboardlint: AddResource failed: " + err.Error())
	}
	sch, err := c.Compile("operator-manifest.schema.json")
	if err != nil {
		panic("onboardlint: schema compile failed: " + err.Error())
	}
	compiled = sch
}

// SchemaJSON returns the embedded canonical manifest schema bytes. The intake
// form fetches this (via GET /api/onboard/schema) to source field enums,
// required-ness, and patterns directly from the SoT — so the form's dropdowns
// can never drift from what the linter enforces.
func SchemaJSON() []byte { return schemaJSON }

// GrammarCutover is the active cutover date, exported so the form can show
// "affiliation required" affordances consistently with the gate.
func GrammarCutover() string { return grammarCutover() }

// grammarCutover returns the active cutover date (env override or default).
func grammarCutover() string {
	if v := os.Getenv("GOAT_GRAMMAR_CUTOVER"); v != "" {
		return v
	}
	return defaultGrammarCutover
}

// Lint validates one manifest's raw bytes (YAML or JSON) and returns a Result.
// format may be "yaml", "json", or "" (auto-detect: leading '{' ⇒ JSON).
//
// Parity note vs validate.py: this validates CONTENT, not a file, so the
// filename-stem↔identity.handle cross-check is intentionally skipped (there is
// no filename in the webform path). The parity corpus uses files whose stem
// already matches the handle, so that check never fires there either.
func Lint(content []byte, format string) Result {
	raw, err := parse(content, format)
	if err != nil {
		return Result{OK: false, Findings: []Finding{{Severity: "error", Field: "(parse)", Message: err.Error()}}}
	}
	if raw == nil {
		return Result{OK: false, Findings: []Finding{{Severity: "error", Field: "(parse)", Message: "empty manifest"}}}
	}

	// Normalize through JSON so the schema validator sees canonical JSON types
	// (float64 numbers, dates already coerced to ISO strings by parse()).
	buf, err := json.Marshal(raw)
	if err != nil {
		return Result{OK: false, Findings: []Finding{{Severity: "error", Field: "(parse)", Message: "cannot encode manifest: " + err.Error()}}}
	}
	inst, err := jsonschema.UnmarshalJSON(bytes.NewReader(buf))
	if err != nil {
		return Result{OK: false, Findings: []Finding{{Severity: "error", Field: "(parse)", Message: "cannot re-decode manifest: " + err.Error()}}}
	}

	var findings []Finding
	if verr := compiled.Validate(inst); verr != nil {
		var ve *jsonschema.ValidationError
		if errors.As(verr, &ve) {
			findings = append(findings, schemaFindings(ve)...)
		} else {
			findings = append(findings, Finding{Severity: "error", Field: "<root>", Message: verr.Error()})
		}
	}

	// Grammar-regression gate (ADR 1043) only runs on a schema-valid manifest,
	// mirroring validate.py (it reads identity.added / netbird.auto_groups).
	data, _ := raw.(map[string]any)
	if len(findings) == 0 && data != nil {
		for _, msg := range grammarRegressionErrors(data) {
			findings = append(findings, Finding{Severity: "error", Field: "identity-grammar", Message: msg})
		}
	}

	if len(findings) > 0 {
		// Stable order so output (and the parity test) is deterministic.
		sort.SliceStable(findings, func(i, j int) bool {
			if findings[i].Field != findings[j].Field {
				return findings[i].Field < findings[j].Field
			}
			return findings[i].Message < findings[j].Message
		})
		return Result{OK: false, Findings: findings}
	}

	shape, detail := classifyShape(data)
	return Result{OK: true, Findings: []Finding{}, Shape: shape, Detail: detail}
}

// parse decodes YAML or JSON into a generic tree and coerces any YAML
// timestamp (e.g. identity.added:) back to an ISO date string, so
// the schema's `format: date` string fields validate — mirroring validate.py's
// _coerce step (PyYAML auto-parses dates to date objects there too).
func parse(content []byte, format string) (any, error) {
	trimmed := bytes.TrimSpace(content)
	useJSON := format == "json" || (format == "" && len(trimmed) > 0 && trimmed[0] == '{')
	var raw any
	if useJSON {
		if err := json.Unmarshal(content, &raw); err != nil {
			return nil, fmt.Errorf("JSON parse error: %w", err)
		}
	} else {
		if err := yaml.Unmarshal(content, &raw); err != nil {
			return nil, fmt.Errorf("YAML parse error: %w", err)
		}
	}
	return coerceDates(raw), nil
}

// coerceDates walks the tree converting time.Time (a YAML !!timestamp) to an
// ISO date string; a zero time-of-day renders as YYYY-MM-DD (the manifest
// case), otherwise RFC3339.
func coerceDates(node any) any {
	switch v := node.(type) {
	case map[string]any:
		for k, val := range v {
			v[k] = coerceDates(val)
		}
		return v
	case []any:
		for i, val := range v {
			v[i] = coerceDates(val)
		}
		return v
	case time.Time:
		if v.Hour() == 0 && v.Minute() == 0 && v.Second() == 0 && v.Nanosecond() == 0 {
			return v.Format("2006-01-02")
		}
		return v.Format(time.RFC3339)
	default:
		return node
	}
}

// schemaFindings walks the validation-error tree to its leaves and emits one
// Finding per leaf (the actionable, field-addressed errors). The verdict
// (any findings ⇒ not OK) is what the parity test holds equal to validate.py;
// the message text is library-specific and intentionally not compared.
func schemaFindings(ve *jsonschema.ValidationError) []Finding {
	var out []Finding
	var walk func(e *jsonschema.ValidationError)
	walk = func(e *jsonschema.ValidationError) {
		if len(e.Causes) == 0 {
			out = append(out, Finding{
				Severity: "error",
				Field:    locField(e.InstanceLocation),
				Message:  e.ErrorKind.LocalizedString(enPrinter),
			})
			return
		}
		for _, c := range e.Causes {
			walk(c)
		}
	}
	walk(ve)
	if len(out) == 0 {
		out = append(out, Finding{Severity: "error", Field: locField(ve.InstanceLocation), Message: "does not satisfy schema"})
	}
	return out
}

// locField renders a JSON-pointer instance location as a dotted field path
// ("netbird.auto_groups.0"); empty location ⇒ "<root>" (matching validate.py).
func locField(loc []string) string {
	if len(loc) == 0 {
		return "<root>"
	}
	return strings.Join(loc, ".")
}

// grammarRegressionErrors ports validate.py::_grammar_regression_errors verbatim.
func grammarRegressionErrors(data map[string]any) []string {
	added := ""
	if id, ok := data["identity"].(map[string]any); ok {
		if a, ok := id["added"].(string); ok {
			added = a
		}
	}
	cutover := grammarCutover()
	// Grandfather anything before the cutover (lexical compare is correct for
	// zero-padded ISO dates); blank added is treated as legacy (schema already
	// requires it, so a blank fails earlier).
	if added == "" || added < cutover {
		return nil
	}

	var errs []string
	if _, ok := data["affiliation"]; !ok {
		errs = append(errs, fmt.Sprintf(
			"identity-grammar: manifest added %s (>= cutover %s) must declare an 'affiliation' block (id; kind is derived, ADR 1151) per ADR 1043 — capability is never named after affiliation. Legacy manifests are grandfathered.",
			added, cutover))
	}
	// Walk the fixed deprecated table, not the manifest's own values: the
	// quoted name in the message is then always one of our constants, never
	// text from the submitted document.
	groups := autoGroups(data)
	var dep []string
	for g := range deprecatedGroups {
		if groups[g] {
			dep = append(dep, g)
		}
	}
	sort.Strings(dep)
	for _, g := range dep {
		errs = append(errs, fmt.Sprintf(
			"identity-grammar: deprecated netbird.auto_groups entry '%s' on a manifest added %s — use %s (ADR 1043).",
			g, added, deprecatedGroups[g]))
	}
	return errs
}

// classifyShape ports validate.py::classify_shape verbatim. Returns the
// canonical (shape, note); display-only, never surfaced raw to users.
func classifyShape(data map[string]any) (string, string) {
	if data == nil {
		return "unclassifiable", ""
	}
	groups := autoGroups(data)
	// ADR 1052: the org-neutral `org:` block is the going-forward authoring noun
	// (the legacy `vendor:` block is still accepted, additive back-compat). Prefer
	// `org`, fall back to `vendor`, so an `org:`-authored manifest classifies
	// identically. Same dual-read as gen-roster.py + the operator-onboard play.
	orgBlock, hasVendorBlock := data["org"].(map[string]any)
	if !hasVendorBlock {
		orgBlock, hasVendorBlock = data["vendor"].(map[string]any)
	}
	zenohEnabled := false
	if z, ok := data["zenoh"].(map[string]any); ok {
		zenohEnabled, _ = z["enabled"].(bool)
	}

	isVendor := groups["VENDOR"] || groups["DEVELOPER"]
	isOperator := groups["ADMIN"] || groups["OPERATOR"] || groups["MGMT-HOST"]
	isBrowse := groups["OBSERVER"] && !isVendor && !isOperator

	vendorDisplay := func() string {
		if orgBlock != nil {
			if n, ok := orgBlock["display_name"].(string); ok {
				return n
			}
		}
		return ""
	}

	switch {
	case isVendor && hasVendorBlock && zenohEnabled:
		zv := "?"
		if z, ok := data["zenoh"].(map[string]any); ok {
			if t, ok := z["trial"].(map[string]any); ok {
				// prefer zenoh.trial.org, fall back to zenoh.trial.vendor
				if v, ok := t["org"].(string); ok {
					zv = v
				} else if v, ok := t["vendor"].(string); ok {
					zv = v
				}
			}
		}
		return "vendor-primary", fmt.Sprintf("%s (cert=%s)", vendorDisplay(), zv)
	case isVendor && hasVendorBlock && !zenohEnabled:
		return "vendor-sibling", vendorDisplay()
	case isOperator && !hasVendorBlock:
		ks := sortedKeys(groups)
		return "operator", "groups=" + strings.Join(ks, ",")
	case isBrowse && !zenohEnabled:
		org := "—"
		if d := vendorDisplay(); d != "" {
			org = d
		}
		return "browse", fmt.Sprintf("OBSERVER-only (%s)", org)
	default:
		return "unclassifiable", fmt.Sprintf("auto_groups=%v vendor_block=%v zenoh=%v", sortedKeys(groups), hasVendorBlock, zenohEnabled)
	}
}

// autoGroups returns netbird.auto_groups as a set.
func autoGroups(data map[string]any) map[string]bool {
	out := map[string]bool{}
	nb, ok := data["netbird"].(map[string]any)
	if !ok {
		return out
	}
	ag, ok := nb["auto_groups"].([]any)
	if !ok {
		return out
	}
	for _, g := range ag {
		if s, ok := g.(string); ok {
			out[s] = true
		}
	}
	return out
}

func sortedKeys(set map[string]bool) []string {
	out := make([]string, 0, len(set))
	for k := range set {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}
