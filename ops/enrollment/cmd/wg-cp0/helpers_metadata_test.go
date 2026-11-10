package main

// helpers_metadata_test.go — coverage for the pure-function helpers
// in plan.go and bundle_create_invoke.go owned by Session C of Block
// 22R.2. Per-subcommand behavior tests live in their respective
// cmd_*_test.go files; this file covers the shared primitives that
// multiple subcommands rely on.

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

// ---- plan.go ----

func TestGlyph_CoversAllPlanOps(t *testing.T) {
	cases := []struct {
		op   PlanOp
		want string
	}{
		{PlanAdd, "+ ADD"},
		{PlanUpdate, "~ UPDATE"},
		{PlanRemove, "- REMOVE"},
		{PlanKeep, "= KEEP"},
		{PlanReuseKeypair, "* REUSING KEYPAIR"},
		{PlanOp(99), "?"},
	}
	for _, c := range cases {
		if got := glyph(c.op); got != c.want {
			t.Errorf("glyph(%v) = %q, want %q", c.op, got, c.want)
		}
	}
}

func TestIndentDetail_Empty(t *testing.T) {
	if got := indentDetail(""); got != "" {
		t.Errorf("indentDetail empty = %q, want empty", got)
	}
}

func TestIndentDetail_SingleLine(t *testing.T) {
	if got := indentDetail("hello"); got != ": hello" {
		t.Errorf("indentDetail single-line = %q", got)
	}
}

func TestIndentDetail_Multiline(t *testing.T) {
	got := indentDetail("a\nb\nc")
	if !strings.HasPrefix(got, ":") || !strings.Contains(got, "\n      a") {
		t.Errorf("indentDetail multi = %q", got)
	}
}

func TestPrintPlan_RendersAllSections(t *testing.T) {
	var buf bytes.Buffer
	printPlan(&buf, Plan{
		Command:     "wg-cp0 add-relay relay-rld",
		StateTarget: "ops/enrollment/wg-cp0-bundle-allowlist.json",
		State: []PlanLine{
			{Op: PlanAdd, Subject: "relays[]: relay-rld", Detail: "endpoint=x.example.com:51821"},
		},
		Apply: []PlanLine{
			{Op: PlanReuseKeypair, Subject: "lab-a", Detail: "re-mint via bundle-create"},
		},
		Notes: []string{"REUSING KEYPAIR for 1 peer."},
	})
	out := buf.String()
	for _, w := range []string{"PLAN —", "State plan", "Apply plan", "+ ADD", "* REUSING KEYPAIR", "REUSING KEYPAIR for 1 peer."} {
		if !strings.Contains(out, w) {
			t.Errorf("printPlan missing %q\nfull:\n%s", w, out)
		}
	}
}

func TestPrintPlan_NoSections(t *testing.T) {
	var buf bytes.Buffer
	printPlan(&buf, Plan{Command: "wg-cp0 add-relay relay-rld"})
	out := buf.String()
	if !strings.HasPrefix(out, "PLAN — wg-cp0 add-relay relay-rld") {
		t.Errorf("header missing: %q", out)
	}
	if strings.Contains(out, "State plan") || strings.Contains(out, "Apply plan") {
		t.Errorf("empty plan should not render section headers: %q", out)
	}
}

func TestPrintApplyHint_NoWork(t *testing.T) {
	var buf bytes.Buffer
	printApplyHint(&buf, false)
	if !strings.Contains(buf.String(), "No changes required") {
		t.Errorf("no-work hint missing: %q", buf.String())
	}
}

func TestPrintApplyHint_HasWork(t *testing.T) {
	var buf bytes.Buffer
	printApplyHint(&buf, true)
	if !strings.Contains(buf.String(), "--apply") {
		t.Errorf("has-work hint missing --apply: %q", buf.String())
	}
}

func TestPrintApplyCompleted(t *testing.T) {
	var buf bytes.Buffer
	printApplyCompleted(&buf, "add-relay relay-rld (added)")
	if !strings.Contains(buf.String(), "Applied: add-relay relay-rld (added)") {
		t.Errorf("applied line missing: %q", buf.String())
	}
}

// ---- bundle_create_invoke.go ----

func TestResolveBundleCreateBin_EnvOverride(t *testing.T) {
	tmp := filepath.Join(t.TempDir(), "fake-bundle-create")
	if err := os.WriteFile(tmp, []byte("#!/bin/sh\nexit 0\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("WG_CP0_BUNDLE_CREATE_BIN", tmp)
	got, err := resolveBundleCreateBin()
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if got != tmp {
		t.Errorf("got %q, want %q", got, tmp)
	}
}

func TestResolveBundleCreateBin_EnvMissing(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "does-not-exist")
	t.Setenv("WG_CP0_BUNDLE_CREATE_BIN", missing)
	if _, err := resolveBundleCreateBin(); err == nil {
		t.Errorf("expected error for missing binary")
	}
}

func TestCASigningEnv_DevMode(t *testing.T) {
	t.Setenv("GOAT_CA_MODE", "dev")
	t.Setenv("GOAT_CA_ID", "test-ca")
	t.Setenv("GOAT_CA_KEY", "/tmp/key")
	t.Setenv("GOAT_AGE_IDENTITY", "/tmp/id")
	t.Setenv("GOAT_TLS_TRUST_ANCHOR", "")
	args, err := caSigningEnv()
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	joined := strings.Join(args, " ")
	for _, w := range []string{"--mode dev", "--ca-id test-ca", "--ca-key /tmp/key", "--age-identity /tmp/id"} {
		if !strings.Contains(joined, w) {
			t.Errorf("missing %q in %q", w, joined)
		}
	}
}

func TestCASigningEnv_ProdMode(t *testing.T) {
	t.Setenv("GOAT_CA_MODE", "prod")
	t.Setenv("GOAT_CA_ID", "prod-ca")
	t.Setenv("GOAT_CA_KEY", "")
	t.Setenv("GOAT_AGE_IDENTITY", "")
	t.Setenv("GOAT_TLS_TRUST_ANCHOR", "")
	args, err := caSigningEnv()
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	joined := strings.Join(args, " ")
	if !strings.Contains(joined, "--mode prod") || !strings.Contains(joined, "--ca-id prod-ca") {
		t.Errorf("prod flags missing: %q", joined)
	}
	if strings.Contains(joined, "--ca-key") || strings.Contains(joined, "--age-identity") {
		t.Errorf("prod mode should not include dev-only flags: %q", joined)
	}
}

func TestCASigningEnv_DefaultsToDev(t *testing.T) {
	t.Setenv("GOAT_CA_MODE", "")
	t.Setenv("GOAT_CA_ID", "test-ca")
	t.Setenv("GOAT_CA_KEY", "/tmp/key")
	t.Setenv("GOAT_AGE_IDENTITY", "/tmp/id")
	t.Setenv("GOAT_TLS_TRUST_ANCHOR", "")
	args, err := caSigningEnv()
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if !strings.Contains(strings.Join(args, " "), "--mode dev") {
		t.Errorf("default mode should be dev: %v", args)
	}
}

func TestCASigningEnv_MissingDevFlags(t *testing.T) {
	t.Setenv("GOAT_CA_MODE", "dev")
	t.Setenv("GOAT_CA_ID", "test-ca")
	t.Setenv("GOAT_CA_KEY", "")
	t.Setenv("GOAT_AGE_IDENTITY", "")
	if _, err := caSigningEnv(); err == nil {
		t.Errorf("expected error for missing dev flags")
	}
}

func TestCASigningEnv_MissingCAID(t *testing.T) {
	t.Setenv("GOAT_CA_MODE", "dev")
	t.Setenv("GOAT_CA_ID", "")
	t.Setenv("GOAT_CA_KEY", "/tmp/key")
	t.Setenv("GOAT_AGE_IDENTITY", "/tmp/id")
	if _, err := caSigningEnv(); err == nil {
		t.Errorf("expected error for missing GOAT_CA_ID")
	}
}

func TestCASigningEnv_UnknownMode(t *testing.T) {
	t.Setenv("GOAT_CA_MODE", "frobnicate")
	t.Setenv("GOAT_CA_ID", "test-ca")
	if _, err := caSigningEnv(); err == nil {
		t.Errorf("expected error for unknown mode")
	}
}

func TestCASigningEnv_TLSTrustAnchorSplicedWhenSet(t *testing.T) {
	t.Setenv("GOAT_CA_MODE", "prod")
	t.Setenv("GOAT_CA_ID", "prod-ca")
	t.Setenv("GOAT_TLS_TRUST_ANCHOR", "/etc/ca/root.pem")
	args, err := caSigningEnv()
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if !strings.Contains(strings.Join(args, " "), "--tls-trust-anchor /etc/ca/root.pem") {
		t.Errorf("trust anchor not spliced: %v", args)
	}
}

func TestWriteRelaysFile_RoundTrip(t *testing.T) {
	relays := []peerset.Relay{
		{
			Name:          "relay-rla",
			Endpoint:      "1.2.3.4:51821",
			MeshAddress:   "198.18.0.2",
			CurrentPubkey: base64.StdEncoding.EncodeToString(make([]byte, 32)),
		},
	}
	path, cleanup, err := writeRelaysFile(relays)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	defer cleanup()

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var out []map[string]string
	if err := json.Unmarshal(data, &out); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(out) != 1 {
		t.Fatalf("len=%d, want 1", len(out))
	}
	if out[0]["name"] != "relay-rla" {
		t.Errorf("name=%q", out[0]["name"])
	}
	if out[0]["endpoint"] != "1.2.3.4:51821" {
		t.Errorf("endpoint=%q", out[0]["endpoint"])
	}
}

func TestWriteRelaysFile_RefusesIncompleteRelays(t *testing.T) {
	relays := []peerset.Relay{
		{Name: "broken", Endpoint: "", MeshAddress: "", CurrentPubkey: ""},
	}
	_, _, err := writeRelaysFile(relays)
	if err == nil {
		t.Errorf("expected error for incomplete relay")
	}
	if !strings.Contains(err.Error(), "missing endpoint/pubkey/mesh-address") {
		t.Errorf("error message: %v", err)
	}
}

func TestNormalizeRelayPubkey_Base64Passthrough(t *testing.T) {
	pub := base64.StdEncoding.EncodeToString(make([]byte, 32))
	got, err := normalizeRelayPubkey(pub)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if got != pub {
		t.Errorf("got %q, want %q", got, pub)
	}
}

func TestNormalizeRelayPubkey_Raw32Bytes(t *testing.T) {
	raw := strings.Repeat("a", 32)
	got, err := normalizeRelayPubkey(raw)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if got != base64.StdEncoding.EncodeToString([]byte(raw)) {
		t.Errorf("got %q", got)
	}
}

func TestNormalizeRelayPubkey_RejectsGarbage(t *testing.T) {
	_, err := normalizeRelayPubkey("not-a-pubkey")
	if err == nil {
		t.Errorf("expected error for malformed pubkey")
	}
}

func TestInferACLGroups_SiteOnly(t *testing.T) {
	got := inferACLGroups(peerset.Peer{Site: "lab"})
	if len(got) != 1 || got[0] != "LAB" {
		t.Errorf("got %v", got)
	}
}

func TestInferACLGroups_NoSite(t *testing.T) {
	got := inferACLGroups(peerset.Peer{})
	if got != nil {
		t.Errorf("expected nil, got %v", got)
	}
}

func TestUpperKebab(t *testing.T) {
	cases := []struct{ in, want string }{
		{"lab", "LAB"},
		{"prov_fra", "PROV-FRA"},
		{"already-up", "ALREADY-UP"},
		{"", ""},
	}
	for _, c := range cases {
		if got := upperKebab(c.in); got != c.want {
			t.Errorf("upperKebab(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestBuildBundleCreateArgs_HappyPath(t *testing.T) {
	p := peerset.Peer{
		Name:         "lab-a",
		WGCP0Address: "198.18.0.1",
		Site:         "lab",
	}
	relays := []peerset.Relay{{Name: "relay-rla"}}
	args, err := buildBundleCreateArgs(invokeOpts{
		Peer:    p,
		Relays:  relays,
		OutPath: "/tmp/out.cbor",
	}, "/tmp/relays.json", "/tmp/allow.json", []string{"--mode", "dev"})
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	joined := strings.Join(args, " ")
	for _, w := range []string{
		"--site lab",
		"--device-id lab-a",
		"--wg-cp0-address 198.18.0.1/24",
		"--wg-cp0-relays /tmp/relays.json",
		"--acl-groups LAB",
		"--out /tmp/out.cbor",
		"--ttl-days 90",
		"--reuse-keypair-from lab-a",
		"--update-allowlist /tmp/allow.json",
		"--mode dev",
	} {
		if !strings.Contains(joined, w) {
			t.Errorf("buildBundleCreateArgs missing %q in %q", w, joined)
		}
	}
}

func TestBuildBundleCreateArgs_OverridesACLGroups(t *testing.T) {
	args, err := buildBundleCreateArgs(invokeOpts{
		Peer:      peerset.Peer{Name: "p", WGCP0Address: "198.18.0.1", Site: "lab"},
		Relays:    []peerset.Relay{{Name: "r"}},
		ACLGroups: []string{"CUSTOM-GROUP", "ANOTHER"},
		OutPath:   "/tmp/out.cbor",
	}, "/tmp/relays.json", "", nil)
	if err != nil {
		t.Fatalf("err: %v", err)
	}
	if !strings.Contains(strings.Join(args, " "), "--acl-groups CUSTOM-GROUP,ANOTHER") {
		t.Errorf("ACL override not applied: %v", args)
	}
}

func TestBuildBundleCreateArgs_RejectsMissingFields(t *testing.T) {
	bad := []invokeOpts{
		{},                                                                                                  // empty
		{Peer: peerset.Peer{Name: "p"}},                                                                     // missing address
		{Peer: peerset.Peer{Name: "p", WGCP0Address: "198.18.0.1"}},                                         // missing site
		{Peer: peerset.Peer{Name: "p", WGCP0Address: "198.18.0.1", Site: "lab"}},                         // missing OutPath
		{Peer: peerset.Peer{Name: "p", WGCP0Address: "198.18.0.1", Site: "lab"}, OutPath: "/tmp/out"},     // missing relays
	}
	for i, b := range bad {
		if _, err := buildBundleCreateArgs(b, "/tmp/r.json", "", nil); err == nil {
			t.Errorf("case %d: expected error", i)
		}
	}
}

func TestBuildBundleCreateArgs_NoACLGroupsWithoutSite(t *testing.T) {
	_, err := buildBundleCreateArgs(invokeOpts{
		Peer:    peerset.Peer{Name: "p", WGCP0Address: "198.18.0.1"},
		Relays:  []peerset.Relay{{Name: "r"}},
		OutPath: "/tmp/out.cbor",
	}, "/tmp/relays.json", "", nil)
	if err == nil {
		t.Errorf("expected error when site is absent and no ACL override")
	}
}

func TestBuildBundleCreateArgs_DefaultTTL(t *testing.T) {
	args, _ := buildBundleCreateArgs(invokeOpts{
		Peer:    peerset.Peer{Name: "p", WGCP0Address: "198.18.0.1", Site: "lab"},
		Relays:  []peerset.Relay{{Name: "r"}},
		OutPath: "/tmp/out.cbor",
	}, "/tmp/r.json", "", nil)
	if !strings.Contains(strings.Join(args, " "), "--ttl-days 90") {
		t.Errorf("default TTL not applied: %v", args)
	}
}

func TestCanonicalCIDR(t *testing.T) {
	cases := []struct{ in, want string }{
		{"198.18.0.1", "198.18.0.1/24"},
		{"198.18.0.1/24", "198.18.0.1/24"},
		{"198.18.0.1/32", "198.18.0.1/32"},
		{"", ""},
	}
	for _, c := range cases {
		if got := canonicalCIDR(c.in); got != c.want {
			t.Errorf("canonicalCIDR(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestJoinCSV(t *testing.T) {
	cases := []struct {
		in   []string
		want string
	}{
		{nil, ""},
		{[]string{"a"}, "a"},
		{[]string{"a", "b", "c"}, "a,b,c"},
	}
	for _, c := range cases {
		if got := joinCSV(c.in); got != c.want {
			t.Errorf("joinCSV(%v) = %q, want %q", c.in, got, c.want)
		}
	}
}

func TestComputeDefaultOutPath_NamesByDate(t *testing.T) {
	when := time.Date(2026, 5, 14, 12, 0, 0, 0, time.UTC)
	p := peerset.Peer{Name: "lab-a", Site: "lab"}
	got := computeDefaultOutPath(p, "/tmp/bundles", when)
	want := "/tmp/bundles/lab-lab-a-2026-05-14.bundle.cbor"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestComputeDefaultOutPath_DefaultsOutDir(t *testing.T) {
	when := time.Date(2026, 5, 14, 12, 0, 0, 0, time.UTC)
	got := computeDefaultOutPath(peerset.Peer{Name: "p", Site: "s"}, "", when)
	if !strings.Contains(got, "s-p-2026-05-14.bundle.cbor") {
		t.Errorf("got %q", got)
	}
	if strings.HasPrefix(got, "/") {
		t.Errorf("default out-dir should be relative; got %q", got)
	}
}

func TestComputeDefaultOutPath_NoSite(t *testing.T) {
	when := time.Date(2026, 5, 14, 12, 0, 0, 0, time.UTC)
	got := computeDefaultOutPath(peerset.Peer{Name: "p"}, "", when)
	if !strings.Contains(got, "unsited-p-") {
		t.Errorf("got %q", got)
	}
}
