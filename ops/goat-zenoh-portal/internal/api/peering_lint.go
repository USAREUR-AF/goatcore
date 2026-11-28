package api

import (
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// Supported lint kinds. Adding a new one means: extend the switch in
// handlePeeringLint, write its semantic-rule function, and surface it in
// the workbench UI's "kind" selector. Keep the API surface narrow on
// purpose — drift between client + server kinds becomes silent 4xx noise.
const (
	lintKindZenohdJSON5 = "zenohd_json5"
)

// peeringLintRequest is the POST /api/peering/lint body. content is the
// user's paste; intended_prefix is optional context the workbench passes
// when the user has typed it in the prefix box — letting the linter call
// out "your ACL rule covers `foo/**` but you said your prefix was `bar`".
type peeringLintRequest struct {
	Kind           string `json:"kind"`
	Content        string `json:"content"`
	IntendedPrefix string `json:"intended_prefix,omitempty"`
}

// Severity ordering: error > warn > info. Errors are things that will
// definitively break the partner's connection (wrong endpoint, missing
// TLS block, ACL rule that won't allow their prefix). Warns are things
// that are usually wrong but technically tolerable (commented-out lines,
// extra unrecognized top-level keys). Infos are advisory.
const (
	sevError = "error"
	sevWarn  = "warn"
	sevInfo  = "info"
)

// peeringLintFinding is one issue. JSONPointer-like path is a `/`-rooted
// dotted-key reference into the parsed config tree; field-name-only is
// fine when the path doesn't disambiguate (e.g. "connect.endpoints").
// DocAnchor links the user back to the canonical guide section.
type peeringLintFinding struct {
	Severity  string `json:"severity"`
	Field     string `json:"field,omitempty"`
	Message   string `json:"message"`
	DocAnchor string `json:"doc_anchor,omitempty"`
}

type peeringLintResponse struct {
	ParseOK  bool                 `json:"parse_ok"`
	Findings []peeringLintFinding `json:"findings"`
	// EchoIntendedPrefix lets the workbench UI confirm the linter saw
	// what it sent — silences a class of "did my prefix get applied?"
	// debugging questions.
	EchoIntendedPrefix string `json:"echo_intended_prefix,omitempty"`
}

// handlePeeringLint is the workbench backend. No auth — same posture as
// /api/fabric/zenohd-config: the content carries no secrets, the rules
// are public, and a partner shouldn't need a vendor row to validate
// their pre-mint config draft.
func (s *Server) handlePeeringLint(w http.ResponseWriter, r *http.Request) {
	var req peeringLintRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_request", "body must be JSON: {kind, content, intended_prefix?}")
		return
	}
	if strings.TrimSpace(req.Content) == "" {
		writeError(w, http.StatusBadRequest, "empty_content",
			"content is required and must be non-empty")
		return
	}

	var findings []peeringLintFinding
	parseOK := true

	switch req.Kind {
	case lintKindZenohdJSON5:
		// Step 1 — pre-parse syntax sniff. Surfaces Python-style literal
		// drift (`True` / `False` / `None`) as dedicated findings and
		// normalizes them in a working copy so the rest of the lint can
		// still run. Without this, a paste from Python collapses into a
		// single "invalid character 'T'" parse error and the user can't
		// see ANY of their semantic mistakes — which was the wall the
		// follow-on closed.
		normalized, pyOccurrences := sniffPythonLiterals(req.Content)
		for _, occ := range pyOccurrences {
			findings = append(findings, peeringLintFinding{
				Severity:  sevWarn,
				Field:     "(syntax line " + itoa(occ.Line) + ")",
				Message:   "looks like a Python literal `" + occ.Token + "`; JSON5 requires the lowercase equivalent `" + pythonToJSONLiteral(occ.Token) + "`. The linter normalized it for further checks, but zenohd will not.",
				DocAnchor: "/peering/guide.html#flavor-b",
			})
		}
		// Step 2 — parse the normalized form and run semantic rules.
		// Graceful continuation: if parse still fails, the parse-error
		// finding is appended AFTER any sniff findings, so the user sees
		// every signal we have rather than the worst one alone.
		_, parsed, err := json5LiteToJSON(normalized)
		if err != nil {
			parseOK = false
			findings = append(findings, peeringLintFinding{
				Severity:  sevError,
				Field:     "(parse)",
				Message:   "JSON5 parse error: " + err.Error(),
				DocAnchor: "/peering/guide.html#flavor-b",
			})
		} else {
			findings = append(findings, lintZenohdJSON5(parsed, req.IntendedPrefix, s.cfg.RouterEndpoint)...)
		}
	default:
		writeError(w, http.StatusBadRequest, "unsupported_kind",
			"unsupported kind; supported: "+lintKindZenohdJSON5)
		return
	}

	writeJSON(w, http.StatusOK, peeringLintResponse{
		ParseOK:            parseOK,
		Findings:           findings,
		EchoIntendedPrefix: req.IntendedPrefix,
	})
}

// lintZenohdJSON5 runs hand-coded semantic checks against the parsed
// config tree. Returns findings in declaration order — the UI surfaces
// them as a flat list, so deterministic ordering keeps the rendering
// stable across debounced re-runs while the user types.
//
// Rules grouped by intent:
//
//  1. mode == "router" (Flavor B precondition; publishers don't use this
//     config shape).
//  2. connect.endpoints contains our router endpoint (string-equal on
//     the configured RouterEndpoint, or substring match on the host part
//     for the case where the user copied a fabric-config-derived endpoint
//     before the operator rotated the hostname).
//  3. transport.link.tls.{enable_mtls, verify_name_on_connect} == true.
//  4. transport.link.tls.{root_ca_certificate, ...key, ...certificate}
//     all set.
//  5. access_control rules (if enabled) reference the intended prefix.
//     If the user passes intended_prefix="my-vendor" but the ACL rule
//     names `their-other-prefix/**`, that's a clear footgun.
//  6. Zenoh version pin advisory: our fabric is pinned to 1.9.0; if the
//     user's container image tag drifts (we can't see it from this
//     paste, so this is always emitted as info).
func lintZenohdJSON5(tree map[string]any, intendedPrefix, expectedRouterEndpoint string) []peeringLintFinding {
	var out []peeringLintFinding

	// Rule 1 — mode.
	mode, _ := tree["mode"].(string)
	switch mode {
	case "router":
		// fine
	case "client", "peer":
		out = append(out, peeringLintFinding{
			Severity:  sevWarn,
			Field:     "mode",
			Message:   "mode is " + dq(mode) + "; the Router Config Generator targets Flavor B (router mode). If you intended Flavor A (single publisher process), skip this config — the operator's bundle already contains a worked first-publisher.py.",
			DocAnchor: "/peering/guide.html",
		})
	default:
		out = append(out, peeringLintFinding{
			Severity:  sevError,
			Field:     "mode",
			Message:   "missing or unrecognized `mode` (expected " + dq("router") + ")",
			DocAnchor: "/peering/guide.html#flavor-b",
		})
	}

	// Rule 2 — connect.endpoints contains our router.
	connectEndpoints := stringSliceAt(tree, "connect", "endpoints")
	if len(connectEndpoints) == 0 {
		out = append(out, peeringLintFinding{
			Severity:  sevError,
			Field:     "connect.endpoints",
			Message:   "missing or empty `connect.endpoints`; the router peer needs to dial the trial fabric explicitly.",
			DocAnchor: "/peering/our-config.html",
		})
	} else if expectedRouterEndpoint != "" {
		matched := false
		for _, ep := range connectEndpoints {
			if ep == expectedRouterEndpoint {
				matched = true
				break
			}
			// Tolerate scheme drift (tls/quic) when host:port matches.
			if hostPortPart(ep) == hostPortPart(expectedRouterEndpoint) && hostPortPart(ep) != "" {
				matched = true
				out = append(out, peeringLintFinding{
					Severity:  sevWarn,
					Field:     "connect.endpoints",
					Message:   "host:port matches our router but scheme differs (" + ep + " vs " + expectedRouterEndpoint + "); the trial fabric only accepts the scheme published at /api/fabric/config.",
					DocAnchor: "/peering/our-config.html",
				})
				break
			}
		}
		if !matched {
			out = append(out, peeringLintFinding{
				Severity:  sevError,
				Field:     "connect.endpoints",
				Message:   "no entry matches the published router endpoint (" + expectedRouterEndpoint + "); fetch the current value from GET /api/fabric/config.",
				DocAnchor: "/peering/our-config.html",
			})
		}
	}

	// Rule 3 + 4 — TLS posture.
	tls := mapAt(tree, "transport", "link", "tls")
	if tls == nil {
		out = append(out, peeringLintFinding{
			Severity:  sevError,
			Field:     "transport.link.tls",
			Message:   "missing `transport.link.tls` block; the trial fabric requires mTLS, so this config will be rejected at the router.",
			DocAnchor: "/peering/our-config.html",
		})
	} else {
		if v, _ := tls["enable_mtls"].(bool); !v {
			out = append(out, peeringLintFinding{
				Severity:  sevError,
				Field:     "transport.link.tls.enable_mtls",
				Message:   "`enable_mtls` must be true; the trial fabric requires mTLS.",
				DocAnchor: "/peering/our-config.html",
			})
		}
		if v, ok := tls["verify_name_on_connect"].(bool); !ok || !v {
			out = append(out, peeringLintFinding{
				Severity:  sevError,
				Field:     "transport.link.tls.verify_name_on_connect",
				Message:   "`verify_name_on_connect` must be true so the router's DNS SAN is pinned (F-149 fix; otherwise IP-spoof attacks against the trial fabric are not detected on the wire).",
				DocAnchor: "/peering/our-config.html",
			})
		}
		for _, k := range []string{"root_ca_certificate", "listen_private_key", "listen_certificate", "connect_private_key", "connect_certificate"} {
			if v, _ := tls[k].(string); strings.TrimSpace(v) == "" {
				out = append(out, peeringLintFinding{
					Severity:  sevError,
					Field:     "transport.link.tls." + k,
					Message:   "`" + k + "` is empty; set it to a container path that maps to your bundle file.",
					DocAnchor: "/peering/zenohd-config.html",
				})
			}
		}
	}

	// Rule 5 — ACL prefix sanity.
	if intendedPrefix != "" {
		// Format check: same regex our registry enforces.
		if err := registry.ValidVendorPrefix(intendedPrefix); err != nil {
			out = append(out, peeringLintFinding{
				Severity:  sevError,
				Field:     "(intended_prefix)",
				Message:   "your intended prefix is invalid: " + err.Error(),
				DocAnchor: "/peering/our-config.html",
			})
		} else {
			// If access_control.enabled is true, scan rules for key_exprs that
			// reference a DIFFERENT prefix's namespace. Empty rules list with
			// enabled=true is itself an error (default-deny will reject all).
			ac := mapAt(tree, "access_control")
			if ac != nil {
				enabled, _ := ac["enabled"].(bool)
				if enabled {
					rules, _ := ac["rules"].([]any)
					if len(rules) == 0 {
						out = append(out, peeringLintFinding{
							Severity:  sevWarn,
							Field:     "access_control.rules",
							Message:   "`access_control` is enabled but `rules` is empty; the default-deny will reject every local publish.",
							DocAnchor: "/peering/guide.html",
						})
					}
					mentionsIntended := false
					for idx, rRaw := range rules {
						r, ok := rRaw.(map[string]any)
						if !ok {
							continue
						}
						keys, _ := r["key_exprs"].([]any)
						for _, kRaw := range keys {
							ks, _ := kRaw.(string)
							if ks == "" {
								continue
							}
							if keyExprMentionsPrefix(ks, intendedPrefix) {
								mentionsIntended = true
								continue
							}
							out = append(out, peeringLintFinding{
								Severity:  sevWarn,
								Field:     "access_control.rules[" + itoa(idx) + "].key_exprs",
								Message:   "key expression " + dq(ks) + " does not start with your intended prefix " + dq(intendedPrefix) + "; double-check this isn't a copy-paste from another vendor's template.",
								DocAnchor: "/peering/guide.html",
							})
						}
					}
					if !mentionsIntended && len(rules) > 0 {
						out = append(out, peeringLintFinding{
							Severity:  sevWarn,
							Field:     "access_control.rules",
							Message:   "no key expression in your ACL rules covers " + dq(intendedPrefix) + "/**; if you intended to publish under that prefix locally, your router will block it before it reaches us.",
							DocAnchor: "/peering/guide.html",
						})
					}
				}
			}
		}
	}

	// Rule 6 — version pin advisory. This config doesn't carry a Zenoh
	// version, but we surface the pin here so the user sees it in
	// context. Always emit at info.
	out = append(out, peeringLintFinding{
		Severity:  sevInfo,
		Field:     "(meta)",
		Message:   "Run this config under Zenoh 1.10.2 (fabric version-of-record). Mismatched versions can wire-format-skew silently. See /api/fabric/config for the current pin.",
		DocAnchor: "/peering/our-config.html",
	})

	return out
}

// --- helpers --------------------------------------------------------------

func mapAt(tree map[string]any, path ...string) map[string]any {
	cur := tree
	for _, p := range path {
		next, ok := cur[p].(map[string]any)
		if !ok {
			return nil
		}
		cur = next
	}
	return cur
}

func stringSliceAt(tree map[string]any, path ...string) []string {
	if len(path) == 0 {
		return nil
	}
	parent := mapAt(tree, path[:len(path)-1]...)
	if parent == nil {
		return nil
	}
	raw, _ := parent[path[len(path)-1]].([]any)
	out := make([]string, 0, len(raw))
	for _, r := range raw {
		if s, ok := r.(string); ok {
			out = append(out, s)
		}
	}
	return out
}

// hostPortPart extracts the host:port tail from a Zenoh locator like
// "tls/zenoh.nova.netbird.nova.example:7447". The scheme is the
// leading slash-segment; everything after the first `/` is the
// host:port. Returns "" if the locator has no `/`.
func hostPortPart(locator string) string {
	idx := strings.Index(locator, "/")
	if idx < 0 {
		return ""
	}
	return locator[idx+1:]
}

// keyExprMentionsPrefix returns true if the key expression starts with
// "<prefix>/" or equals "<prefix>" alone. Tolerates a "**" leading-glob
// (we deliberately do not — that means "everything" which is overbroad)
// and a leading "**/<prefix>/" (also overbroad). Strict-positive check.
func keyExprMentionsPrefix(keyExpr, prefix string) bool {
	if keyExpr == prefix {
		return true
	}
	return strings.HasPrefix(keyExpr, prefix+"/")
}

// itoa is a tiny stand-in for strconv.Itoa so callers don't need the
// import; the only caller is the finding-rendering loop above and the
// values fit a single int comfortably.
func itoa(n int) string {
	if n == 0 {
		return "0"
	}
	neg := false
	if n < 0 {
		neg = true
		n = -n
	}
	var buf [20]byte
	i := len(buf)
	for n > 0 {
		i--
		buf[i] = byte('0' + n%10)
		n /= 10
	}
	if neg {
		i--
		buf[i] = '-'
	}
	return string(buf[i:])
}

// dq wraps a string in double quotes for error messages — the linter's
// output ends up rendered as plain text, so we use literal quotes (not
// `%q` which would escape inner quotes).
func dq(s string) string {
	return "\"" + s + "\""
}
