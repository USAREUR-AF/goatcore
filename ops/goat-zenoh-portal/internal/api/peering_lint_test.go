package api_test

import (
	"bytes"
	"encoding/json"
	"net/http"
	"strings"
	"testing"
)

// goldenZenohdJSON5 mirrors what GET /api/fabric/zenohd-config returns —
// a clean Flavor-B config the workbench will load as its starter template.
// Using a string literal (not a fetch) so the test isolates the lint
// rules from the generator template.
const goldenZenohdJSON5 = `// header comment
{
  mode: "router",
  connect: { endpoints: ["tls/zenoh.goat.example:7447"] },
  listen: { endpoints: ["tcp/127.0.0.1:7447"] },
  transport: {
    link: {
      tls: {
        enable_mtls: true,
        verify_name_on_connect: true,
        root_ca_certificate: "/cert/root-cert.pem",
        listen_private_key: "/cert/key.pem",
        listen_certificate: "/cert/cert.pem",
        connect_private_key: "/cert/key.pem",
        connect_certificate: "/cert/cert.pem",
      },
    },
  },
  access_control: { enabled: false },
}
`

func TestPeeringLint_GoldenIsClean(t *testing.T) {
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", goldenZenohdJSON5, "my-vendor")
	if !body.ParseOK {
		t.Fatalf("golden should parse OK; findings=%+v", body.Findings)
	}
	for _, f := range body.Findings {
		if f.Severity == "error" {
			t.Errorf("golden produced an error finding: %+v", f)
		}
	}
}

func TestPeeringLint_MissingMTLS(t *testing.T) {
	src := strings.Replace(goldenZenohdJSON5, "enable_mtls: true", "enable_mtls: false", 1)
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if !findingsContain(body.Findings, "error", "enable_mtls") {
		t.Errorf("expected error about enable_mtls; got %+v", body.Findings)
	}
}

func TestPeeringLint_MissingTLSBlockEntirely(t *testing.T) {
	src := `{ mode: "router", connect: { endpoints: ["tls/zenoh.goat.example:7447"] } }`
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if !findingsContain(body.Findings, "error", "transport.link.tls") {
		t.Errorf("expected error about missing tls block; got %+v", body.Findings)
	}
}

func TestPeeringLint_WrongMode(t *testing.T) {
	src := strings.Replace(goldenZenohdJSON5, `mode: "router"`, `mode: "client"`, 1)
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if !findingsContain(body.Findings, "warn", "mode") {
		t.Errorf("expected warn about mode; got %+v", body.Findings)
	}
}

func TestPeeringLint_MissingConnectEndpoints(t *testing.T) {
	src := strings.Replace(goldenZenohdJSON5,
		`connect: { endpoints: ["tls/zenoh.goat.example:7447"] },`,
		``, 1)
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if !findingsContain(body.Findings, "error", "connect.endpoints") {
		t.Errorf("expected error about connect.endpoints; got %+v", body.Findings)
	}
}

func TestPeeringLint_ACLPrefixMismatch(t *testing.T) {
	src := `{
  mode: "router",
  connect: { endpoints: ["tls/zenoh.goat.example:7447"] },
  transport: { link: { tls: {
    enable_mtls: true, verify_name_on_connect: true,
    root_ca_certificate: "/cert/root-cert.pem",
    listen_private_key: "/cert/key.pem", listen_certificate: "/cert/cert.pem",
    connect_private_key: "/cert/key.pem", connect_certificate: "/cert/cert.pem",
  } } },
  access_control: {
    enabled: true,
    rules: [ { key_exprs: ["someone-else/**"] } ],
  },
}`
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "my-vendor")
	// Both findings should surface: the per-rule mismatch + the "no rule
	// covers your prefix" overall finding.
	if !findingsContain(body.Findings, "warn", "someone-else") {
		t.Errorf("expected warn about someone-else mismatch; got %+v", body.Findings)
	}
	if !findingsContain(body.Findings, "warn", "my-vendor") {
		t.Errorf("expected warn about no rule covering my-vendor; got %+v", body.Findings)
	}
}

func TestPeeringLint_BadJSON5(t *testing.T) {
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", "{ this is not valid", "")
	if body.ParseOK {
		t.Errorf("expected parse_ok=false")
	}
	if !findingsContain(body.Findings, "error", "parse") {
		t.Errorf("expected parse error finding; got %+v", body.Findings)
	}
}

func TestPeeringLint_EmptyContent(t *testing.T) {
	ts := newTestServer(t)
	resp, err := http.Post(ts.URL+"/api/peering/lint", "application/json",
		strings.NewReader(`{"kind":"zenohd_json5","content":""}`))
	if err != nil {
		t.Fatalf("POST: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusBadRequest {
		t.Errorf("empty content: want 400, got %d", resp.StatusCode)
	}
}

func TestPeeringLint_UnknownKind(t *testing.T) {
	ts := newTestServer(t)
	resp, err := http.Post(ts.URL+"/api/peering/lint", "application/json",
		strings.NewReader(`{"kind":"some_other_thing","content":"x"}`))
	if err != nil {
		t.Fatalf("POST: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusBadRequest {
		t.Errorf("unknown kind: want 400, got %d", resp.StatusCode)
	}
}

func TestPeeringLint_EchoesIntendedPrefix(t *testing.T) {
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", goldenZenohdJSON5, "my-vendor")
	if body.EchoIntendedPrefix != "my-vendor" {
		t.Errorf("echo intended prefix: got %q want my-vendor", body.EchoIntendedPrefix)
	}
}

// --- Python-literal sniff + graceful continuation -------------------------

// TestPeeringLint_PythonTrueParsesAndSemanticsRun is the headline case: an
// otherwise-clean config with capital-T `True` lands a warn for the
// Python-ism + the normal info advisory, AND the semantic stage runs
// (parse_ok is true after normalization). Before this change the lint
// surface collapsed into a single generic JSON5 parse error.
func TestPeeringLint_PythonTrueParsesAndSemanticsRun(t *testing.T) {
	src := strings.ReplaceAll(goldenZenohdJSON5, "enable_mtls: true", "enable_mtls: True")
	src = strings.ReplaceAll(src, "verify_name_on_connect: true", "verify_name_on_connect: True")
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "my-vendor")
	if !body.ParseOK {
		t.Fatalf("expected parse_ok=true after Python-literal normalization; findings=%+v", body.Findings)
	}
	if !findingsContain(body.Findings, "warn", "Python literal `True`") {
		t.Errorf("expected warn about Python True; got %+v", body.Findings)
	}
	// Semantic checks must still run — enable_mtls error must NOT fire
	// (normalization produced lowercase true), and the version-pin info
	// advisory must be present.
	for _, f := range body.Findings {
		if f.Severity == "error" && strings.Contains(f.Field, "enable_mtls") {
			t.Errorf("enable_mtls error should not fire after normalization: %+v", f)
		}
	}
	if !findingsContain(body.Findings, "info", "Zenoh 1.9.0") {
		t.Errorf("expected version-pin info finding; got %+v", body.Findings)
	}
}

// TestPeeringLint_PythonFalseAndNone covers `False` and `None`
// normalization. None → null is structurally invalid in our schema
// (every field we care about is bool / string / array / object), but
// the sniff still flags it. Useful as a forward-compatibility signal.
func TestPeeringLint_PythonFalseAndNone(t *testing.T) {
	src := strings.Replace(goldenZenohdJSON5, "access_control: { enabled: false }", "access_control: { enabled: False }", 1)
	src = strings.Replace(src, "verify_name_on_connect: true", "verify_name_on_connect: None", 1)
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "my-vendor")
	if !findingsContain(body.Findings, "warn", "Python literal `False`") {
		t.Errorf("expected warn about Python False; got %+v", body.Findings)
	}
	if !findingsContain(body.Findings, "warn", "Python literal `None`") {
		t.Errorf("expected warn about Python None; got %+v", body.Findings)
	}
	// None → null on a bool field — semantic stage should flag the
	// verify_name_on_connect violation (null isn't bool true).
	if !findingsContain(body.Findings, "error", "verify_name_on_connect") {
		t.Errorf("expected verify_name_on_connect error; got %+v", body.Findings)
	}
}

// TestPeeringLint_PythonInsideStringNotFlagged: literal `True` inside a
// double-quoted string is not a Python-ism — it's data. Sniff must
// leave it alone.
func TestPeeringLint_PythonInsideStringNotFlagged(t *testing.T) {
	src := `{
  mode: "router",
  connect: { endpoints: ["tls/zenoh.goat.example:7447"] },
  // Comment about True / False / None - should NOT trigger
  some_string_field: "this string contains True and False and None",
  transport: { link: { tls: {
    enable_mtls: true, verify_name_on_connect: true,
    root_ca_certificate: "/cert/root-cert.pem",
    listen_private_key: "/cert/key.pem", listen_certificate: "/cert/cert.pem",
    connect_private_key: "/cert/key.pem", connect_certificate: "/cert/cert.pem",
  } } },
}`
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	for _, f := range body.Findings {
		if strings.Contains(f.Message, "Python literal") {
			t.Errorf("Python-literal warning fired for content inside a string/comment: %+v", f)
		}
	}
}

// TestPeeringLint_GracefulContinuationOnPythonPlusSyntaxError: a paste
// with BOTH a Python `True` AND a real JSON5 syntax error (missing
// closing brace) should surface BOTH findings — the sniff warn for the
// Python-ism AND the parse-error finding from the still-broken text.
func TestPeeringLint_GracefulContinuationOnPythonPlusSyntaxError(t *testing.T) {
	// Missing closing brace AND a Python True. The sniff fires on True
	// regardless of whether the rest is parseable.
	src := `{ mode: "router", enable_mtls: True, this is bad`
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if body.ParseOK {
		t.Errorf("expected parse_ok=false on broken syntax; findings=%+v", body.Findings)
	}
	if !findingsContain(body.Findings, "warn", "Python literal `True`") {
		t.Errorf("expected sniff to fire even with parse error; got %+v", body.Findings)
	}
	if !findingsContain(body.Findings, "error", "parse") {
		t.Errorf("expected parse error finding alongside sniff; got %+v", body.Findings)
	}
}

// TestPeeringLint_LineNumberInSniffFinding sanity-checks the (line N)
// field. A True on line 5 should be reported at line 5.
func TestPeeringLint_LineNumberInSniffFinding(t *testing.T) {
	src := "\n\n\n\nenable_mtls: True\n"
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if !findingsContain(body.Findings, "warn", "line 5") {
		t.Errorf("expected (syntax line 5) field; got %+v", body.Findings)
	}
}

// --- lintTreeOnly tests for the JSON5 preprocessor ---

func TestJSON5Lite_HandlesSingleQuotes(t *testing.T) {
	// Direct test through the lint surface — a single-quoted value should
	// be tolerated and reach the semantic stage as a normal string.
	src := `{ mode: 'router', connect: { endpoints: ['tls/zenoh.goat.example:7447'] }, transport: { link: { tls: { enable_mtls: true, verify_name_on_connect: true, root_ca_certificate: '/cert/root-cert.pem', listen_private_key: '/cert/key.pem', listen_certificate: '/cert/cert.pem', connect_private_key: '/cert/key.pem', connect_certificate: '/cert/cert.pem' } } } }`
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if !body.ParseOK {
		t.Errorf("single-quoted JSON5 should parse; findings=%+v", body.Findings)
	}
}

func TestJSON5Lite_HandlesTrailingCommas(t *testing.T) {
	src := `{ mode: "router", connect: { endpoints: ["tls/zenoh.goat.example:7447",], }, transport: { link: { tls: { enable_mtls: true, verify_name_on_connect: true, root_ca_certificate: "/cert/root-cert.pem", listen_private_key: "/cert/key.pem", listen_certificate: "/cert/cert.pem", connect_private_key: "/cert/key.pem", connect_certificate: "/cert/cert.pem", }, }, }, }`
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if !body.ParseOK {
		t.Errorf("trailing-comma JSON5 should parse; findings=%+v", body.Findings)
	}
}

func TestJSON5Lite_HandlesBlockComments(t *testing.T) {
	src := `/* leading block */
{ mode: "router" /* inline */, connect: { endpoints: ["tls/zenoh.goat.example:7447"] }, transport: { link: { tls: { enable_mtls: true, verify_name_on_connect: true, root_ca_certificate: "/cert/root-cert.pem", listen_private_key: "/cert/key.pem", listen_certificate: "/cert/cert.pem", connect_private_key: "/cert/key.pem", connect_certificate: "/cert/cert.pem" } } } }`
	ts := newTestServer(t)
	body := mustLint(t, ts.URL, "zenohd_json5", src, "")
	if !body.ParseOK {
		t.Errorf("block-comment JSON5 should parse; findings=%+v", body.Findings)
	}
}

// --- helpers --------------------------------------------------------------

type lintRespBody struct {
	ParseOK            bool `json:"parse_ok"`
	Findings           []struct {
		Severity  string `json:"severity"`
		Field     string `json:"field"`
		Message   string `json:"message"`
		DocAnchor string `json:"doc_anchor"`
	} `json:"findings"`
	EchoIntendedPrefix string `json:"echo_intended_prefix"`
}

func mustLint(t *testing.T, baseURL, kind, content, intended string) lintRespBody {
	t.Helper()
	payload, _ := json.Marshal(map[string]string{
		"kind":            kind,
		"content":         content,
		"intended_prefix": intended,
	})
	resp, err := http.Post(baseURL+"/api/peering/lint", "application/json", bytes.NewReader(payload))
	if err != nil {
		t.Fatalf("POST lint: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("lint status: got %d want 200", resp.StatusCode)
	}
	var body lintRespBody
	if err := json.NewDecoder(resp.Body).Decode(&body); err != nil {
		t.Fatalf("decode: %v", err)
	}
	return body
}

func findingsContain(findings []struct {
	Severity  string `json:"severity"`
	Field     string `json:"field"`
	Message   string `json:"message"`
	DocAnchor string `json:"doc_anchor"`
}, severity, needle string) bool {
	for _, f := range findings {
		if f.Severity != severity {
			continue
		}
		if strings.Contains(f.Field, needle) || strings.Contains(f.Message, needle) {
			return true
		}
	}
	return false
}
