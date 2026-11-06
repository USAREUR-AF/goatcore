package certmint_test

import (
	"archive/tar"
	"compress/gzip"
	"context"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
)

func TestMint_FakeMode(t *testing.T) {
	dir := t.TempDir()
	root := filepath.Join(dir, "fake-root.pem")
	if err := os.WriteFile(root, []byte("FAKE ROOT"), 0o644); err != nil {
		t.Fatalf("seed root: %v", err)
	}
	onb := filepath.Join(dir, "onboarding.md")
	_ = os.WriteFile(onb, []byte("# fake onboarding"), 0o644)

	// Seed an onboarding doc that includes the placeholders the renderer
	// is supposed to substitute. Verifies the F-DOG-5 fix (verbatim doc
	// was shipped before; now placeholders get filled at tarball-build).
	onbBody := "# fake onboarding\n\nendpoint=<router-endpoint>\nvendor=<vendor>\n"
	_ = os.WriteFile(onb, []byte(onbBody), 0o644)

	m := &certmint.Minter{
		Fake:             true,
		CARoot:           root,
		OnboardingPacket: onb,
		VendorBundleDir:  filepath.Join(dir, "bundles"),
		RouterEndpoint:   "tls/zenoh.goat.example:7447",
		PortalURL:        "https://portal.goat.example",
		TrialSunset:      "open-ended (trial-fabric)",
		FeedbackChannel:  "Matrix #goat-trial-feedback",
	}
	res, err := m.Mint(context.Background(), "community", "vp_community_testkey", certmint.MintOptions{})
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	if res.URISAN != "urn:goat:example:org=community" {
		t.Errorf("URI-SAN %q is not the built-in default prefix + community", res.URISAN)
	}
	if _, err := os.Stat(res.TarballPath); err != nil {
		t.Fatalf("tarball not created: %v", err)
	}
	// Verify tarball contents.
	f, err := os.Open(res.TarballPath)
	if err != nil {
		t.Fatalf("open tarball: %v", err)
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		t.Fatalf("gzip: %v", err)
	}
	tr := tar.NewReader(gz)
	expected := map[string]bool{
		"community-cert.pem":        false,
		"community-key.pem":         false,
		"community-goat-cli.cbor":   false, // fake-mode .cbor placeholder rides the tarball
		"ca-root.pem":               false,
		"zenoh-trial-onboarding.md": false,
		"NOTES.md":                  false,
		"first-publisher.py":        false,
		"first-subscriber.py":       false,
	}
	// Per-file content checks — keyed by file name, returns the substrings
	// the body must contain. Closes F-DOG-5/10/11/14: vendor must be able
	// to read what their endpoint is and what the mTLS posture is without
	// asking the operator.
	contentChecks := map[string][]string{
		"zenoh-trial-onboarding.md": {
			"endpoint=tls/zenoh.goat.example:7447", // F-DOG-5: router-endpoint placeholder substituted
			"vendor=community",                     // F-DOG-5: vendor placeholder substituted
		},
		"first-publisher.py": {
			"VENDOR = \"community\"",                   // baked-in prefix
			"ROUTER = \"tls/zenoh.goat.example:7447\"", // baked-in endpoint
			"\"enable_mtls\": True",                    // F-DOG-11: client-cert send enabled
			"\"verify_name_on_connect\": True",         // F-DOG-10 resolved: DNS-SAN cert + hostname endpoint = hostname verification stays on
			"insert_json5(\"transport/link/tls\"",      // F-DOG-11: whole-block insert (not per-sub-key)
		},
		"first-subscriber.py": {
			"CERT_SLOT_ID = \"community\"",             // baked-in prefix
			"ROUTER = \"tls/zenoh.goat.example:7447\"", // baked-in endpoint
			"declare_subscriber",                       // subscriber, not publisher
			"TOPIC_SLOT_ID + \"/**\"",                  // default key expression is the topic slot's subtree
			"TOPIC_SLOT_ID = CERT_SLOT_ID",             // by default the subscriber reads the cert's own slot
			"\"enable_mtls\": True",                    // same mTLS posture as publisher
			"\"verify_name_on_connect\": True",
			"insert_json5(\"transport/link/tls\"",
		},
		"NOTES.md": {
			"tls/zenoh.goat.example:7447",        // router endpoint surfaced
			"https://portal.goat.example",        // portal URL surfaced
			"pip install eclipse-zenoh protobuf", // worked-install path
		},
	}
	bodies := map[string]string{}
	for {
		h, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatalf("tar next: %v", err)
		}
		if _, want := expected[h.Name]; want {
			expected[h.Name] = true
		}
		if _, check := contentChecks[h.Name]; check {
			body, err := io.ReadAll(tr)
			if err != nil {
				t.Fatalf("read %s: %v", h.Name, err)
			}
			bodies[h.Name] = string(body)
		}
	}
	for name, present := range expected {
		if !present {
			t.Errorf("tarball missing %q", name)
		}
	}
	for name, mustContain := range contentChecks {
		body, ok := bodies[name]
		if !ok {
			continue // already reported by the expected[] check above
		}
		for _, needle := range mustContain {
			if !strings.Contains(body, needle) {
				t.Errorf("%s does not contain %q\n--- body ---\n%s", name, needle, body)
			}
		}
	}
}

// TestMint_PerGoatnetOverride_Galaxy: when a goatnet specifies its own per-goatnet
// identity strings (URIPrefix, DNSSuffix, CNSuffix) and onboarding filenames
// (TarballStem, PacketName, RootCertName), the minted cert + tarball carry ZERO
// legacy wording. This is the freeze-critical guarantee — the cert URI-SAN
// grammar external orgs pin must not carry the built-in default prefix / "vendor" / "trial".
func TestMint_PerGoatnetOverride_Galaxy(t *testing.T) {
	dir := t.TempDir()
	root := filepath.Join(dir, "fake-root.pem")
	_ = os.WriteFile(root, []byte("FAKE ROOT"), 0o644)
	onb := filepath.Join(dir, "onboarding.md")
	_ = os.WriteFile(onb, []byte("# galaxy onboarding\nvendor=<vendor>\n"), 0o644)

	m := &certmint.Minter{
		Fake:             true,
		CARoot:           root,
		OnboardingPacket: onb,
		VendorBundleDir:  filepath.Join(dir, "bundles"),
		// Per-goatnet identity + filenames.
		URIPrefix:    "urn:goat:galaxy:org=",
		DNSSuffix:    "galaxy.peer",
		CNSuffix:     "-peer",
		TarballStem:  "galaxy-onboarding",
		PacketName:   "galaxy-fabric-onboarding.md",
		RootCertName: "galaxy-ca-root.pem",
	}
	res, err := m.Mint(context.Background(), "example-mil", "vp_x", certmint.MintOptions{})
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	if res.URISAN != "urn:goat:galaxy:org=example-mil" {
		t.Errorf("URI-SAN = %q; want urn:goat:galaxy:org=example-mil (no default prefix, no vendor=)", res.URISAN)
	}
	if strings.HasPrefix(res.URISAN, "urn:goat:example:") || strings.Contains(res.URISAN, "vendor=") {
		t.Errorf("URI-SAN %q still carries the built-in default prefix", res.URISAN)
	}
	stem := filepath.Base(res.TarballPath)
	if !strings.HasPrefix(stem, "galaxy-onboarding-") {
		t.Errorf("tarball %q does not use the galaxy-onboarding stem", stem)
	}
	if strings.HasPrefix(stem, "goat-onboarding-") {
		t.Errorf("tarball filename %q still uses the built-in default stem", stem)
	}
	// The onboarding packet + CA-root file inside the tarball must use the
	// per-goatnet filenames, with no legacy filenames present.
	f, _ := os.Open(res.TarballPath)
	defer f.Close()
	gz, _ := gzip.NewReader(f)
	tr := tar.NewReader(gz)
	want := map[string]bool{"galaxy-fabric-onboarding.md": false, "galaxy-ca-root.pem": false}
	for {
		h, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatalf("tar next: %v", err)
		}
		if _, ok := want[h.Name]; ok {
			want[h.Name] = true
		}
		if h.Name == "ca-root.pem" || h.Name == "zenoh-trial-onboarding.md" {
			t.Errorf("tarball still contains built-in default file %q", h.Name)
		}
	}
	for name, present := range want {
		if !present {
			t.Errorf("tarball missing GALAXY file %q", name)
		}
	}
}

func TestMint_RealModeMissingPrereqs(t *testing.T) {
	m := &certmint.Minter{
		Fake:            false,
		VendorBundleDir: t.TempDir(),
		// Deliberately omit CABundleDir + AgeIdentityPath
	}
	// Should fail with the actionable Track-B-style message rather than
	// crash. We can't actually run cert-sign in unit tests; this asserts
	// the precondition guard fires.
	_, err := m.Mint(context.Background(), "community", "vp_community_testkey", certmint.MintOptions{})
	if err == nil {
		t.Fatal("expected error in real mode without pre-reqs")
	}
	// We don't bind to the exact error string; just confirm the failure
	// is informative.
	if !strings.Contains(err.Error(), "cert-sign") && !strings.Contains(err.Error(), "ca-bundle") {
		t.Errorf("expected cert-sign/ca-bundle related error, got: %v", err)
	}
}

// readTarball returns {name->body} and the set of member names in a .tar.gz.
func readTarball(t *testing.T, path string) (map[string]string, map[string]bool) {
	t.Helper()
	f, err := os.Open(path)
	if err != nil {
		t.Fatalf("open tarball: %v", err)
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		t.Fatalf("gzip: %v", err)
	}
	tr := tar.NewReader(gz)
	bodies := map[string]string{}
	names := map[string]bool{}
	for {
		h, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatalf("tar next: %v", err)
		}
		names[h.Name] = true
		b, _ := io.ReadAll(tr)
		bodies[h.Name] = string(b)
	}
	return bodies, names
}

// TestMint_PerGoatnetRootCert: the sample scripts must not hardcode the legacy
// CA filename while the tarball ships the CA under RootCertName, or
// first-publisher.py / first-subscriber.py point at a file not in the tarball
// and fail on connect. The script's root_ca_certificate goes through
// applyLexicon's RootCertName replacer; this test pins it.
func TestMint_PerGoatnetRootCert(t *testing.T) {
	dir := t.TempDir()
	root := filepath.Join(dir, "fake-root.pem")
	if err := os.WriteFile(root, []byte("FAKE ROOT"), 0o644); err != nil {
		t.Fatalf("seed root: %v", err)
	}
	onb := filepath.Join(dir, "onboarding.md")
	_ = os.WriteFile(onb, []byte("# fake onboarding\n"), 0o644)

	m := &certmint.Minter{
		Fake:             true,
		CARoot:           root,
		OnboardingPacket: onb,
		VendorBundleDir:  filepath.Join(dir, "bundles"),
		RouterEndpoint:   "tls/zenoh.galaxy.goat.example:7447",
		PortalURL:        "https://portal.galaxy.goat.example",
		RootCertName:     "galaxy-ca-root.pem", // per-goatnet
		TarballStem:      "galaxy-onboarding",
	}
	res, err := m.Mint(context.Background(), "example-org-blake", "vp_blake_testkey", certmint.MintOptions{})
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	bodies, names := readTarball(t, res.TarballPath)

	// The CA file ships under the per-goatnet name, not the legacy literal.
	if !names["galaxy-ca-root.pem"] {
		t.Errorf("tarball missing galaxy-ca-root.pem; members=%v", names)
	}
	if names["ca-root.pem"] {
		t.Errorf("tarball still ships the default ca-root.pem under a per-goatnet RootCertName")
	}
	// The scripts must reference the file that's actually in the tarball.
	for _, script := range []string{"first-publisher.py", "first-subscriber.py"} {
		b, ok := bodies[script]
		if !ok {
			t.Fatalf("tarball missing %s", script)
		}
		if !strings.Contains(b, `"galaxy-ca-root.pem"`) {
			t.Errorf("%s does not reference the shipped CA filename galaxy-ca-root.pem", script)
		}
		if strings.Contains(b, "<ca-root-cert-name>") || strings.Contains(b, `"ca-root.pem"`) {
			t.Errorf("%s does not reference the per-goatnet CA filename — would ZError on connect", script)
		}
	}
}

// TestMint_OnboardingDoc_GoatnetNoLeakage renders the REAL onboarding template
// (docs/operations/zenoh-trial-onboarding.md) through a full mint and asserts
// the packet the org receives carries none of:
//   - a doubled router port (`tls/host:7447:7447`, `http://host:7447:8000`) —
//     <router-endpoint>/<router-host> are the substitutions that avoid it.
//   - hardcoded legacy hostnames + a legacy URI-SAN scheme in the body.
//   - the legacy CA-root filename.
//
// A stub body cannot catch template drift, so this uses the real template.
func TestMint_OnboardingDoc_GoatnetNoLeakage(t *testing.T) {
	// The real template lives at repo docs/operations/; the test package is
	// four levels down under ops/goat-zenoh-portal/internal/certmint.
	tmpl := filepath.Join("..", "..", "..", "..", "docs", "operations", "zenoh-trial-onboarding.md")
	if _, err := os.Stat(tmpl); err != nil {
		t.Skipf("real onboarding template not found (%v); skipping cross-file guard", err)
	}

	dir := t.TempDir()
	root := filepath.Join(dir, "fake-root.pem")
	if err := os.WriteFile(root, []byte("FAKE ROOT"), 0o644); err != nil {
		t.Fatalf("seed root: %v", err)
	}

	m := &certmint.Minter{
		Fake:             true,
		CARoot:           root,
		OnboardingPacket: tmpl,
		VendorBundleDir:  filepath.Join(dir, "bundles"),
		// Per-goatnet identity + connection details.
		URIPrefix:      "urn:goat:galaxy:org=",
		DNSSuffix:      "galaxy.peer",
		TarballStem:    "galaxy-onboarding",
		PacketName:     "galaxy-fabric-onboarding.md",
		RootCertName:   "galaxy-ca-root.pem",
		RouterEndpoint: "tls/zenoh.galaxy.goat.example:7447",
		PortalURL:      "https://portal.galaxy.goat.example",
	}
	res, err := m.Mint(context.Background(), "9876f7eb3db3e8cca39ca5d0fc1f92c5", "vp_x", certmint.MintOptions{})
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	bodies, _ := readTarball(t, res.TarballPath)
	body, ok := bodies["galaxy-fabric-onboarding.md"]
	if !ok {
		t.Fatalf("tarball missing galaxy-fabric-onboarding.md")
	}

	// Must NOT contain any of this drift.
	for _, bad := range []string{
		":7447:7447",            // doubled TLS port
		":7447:8000",            // doubled REST port
		"nova.example",   // legacy zone
		"urn:goat-nova",           // legacy URI-SAN scheme
		"nova-ca-root.pem", // legacy CA filename
		"chronyc add server",    // router advertised as an NTP server; no goatnet serves time (chrony `port 0`)
		"Time-sync server",      // the connection-details row that made that promise
	} {
		if strings.Contains(body, bad) {
			t.Errorf("rendered GALAXY onboarding packet still contains %q", bad)
		}
	}

	// Must contain — the correctly-substituted per-goatnet values.
	for _, want := range []string{
		"tls/zenoh.galaxy.goat.example:7447",    // <router-endpoint>
		"http://zenoh.galaxy.goat.example:8000", // <router-host> (no doubled port)
		"urn:goat:galaxy:org=",                  // <uri-san-prefix>
		"https://portal.galaxy.goat.example",    // <portal-url>
		"galaxy-ca-root.pem",                    // per-goatnet CA filename
	} {
		if !strings.Contains(body, want) {
			t.Errorf("rendered GALAXY onboarding packet is missing %q", want)
		}
	}
}

// The U7 publish-my-key persist: every mint drops the PUBLIC leaf at
// <RecipientKeysDir>/<prefix>.pem, a re-mint (rotation, ADR 1044) overwrites it
// with the new leaf, and the private key never lands in the store.
func TestMint_PersistsRecipientCert(t *testing.T) {
	dir := t.TempDir()
	root := filepath.Join(dir, "fake-root.pem")
	_ = os.WriteFile(root, []byte("FAKE ROOT"), 0o644)
	onb := filepath.Join(dir, "onboarding.md")
	_ = os.WriteFile(onb, []byte("# fake onboarding"), 0o644)
	keysDir := filepath.Join(dir, "recipient-keys")

	m := &certmint.Minter{
		Fake:             true,
		CARoot:           root,
		OnboardingPacket: onb,
		VendorBundleDir:  filepath.Join(dir, "bundles"),
		RecipientKeysDir: keysDir,
		RouterEndpoint:   "tls/zenoh.example:7447",
		PortalURL:        "https://portal.example",
	}
	res, err := m.Mint(context.Background(), "acme", "vp_key", certmint.MintOptions{})
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	stored, err := os.ReadFile(filepath.Join(keysDir, "acme.pem"))
	if err != nil {
		t.Fatalf("recipient cert not persisted: %v", err)
	}
	minted, err := os.ReadFile(res.CertPath)
	if err == nil {
		// The stage dir may already be gone by the time we look; when the minted
		// cert is still readable it must equal the persisted copy byte-for-byte.
		if string(stored) != string(minted) {
			t.Fatalf("persisted cert differs from minted leaf")
		}
	}
	if strings.Contains(string(stored), "PRIVATE KEY") {
		t.Fatalf("recipient store must never carry private key material")
	}

	// Re-mint (the rotation path) overwrites with the fresh leaf.
	if _, err := m.Mint(context.Background(), "acme", "vp_key2", certmint.MintOptions{}); err != nil {
		t.Fatalf("re-mint: %v", err)
	}
	if _, err := os.Stat(filepath.Join(keysDir, "acme.pem")); err != nil {
		t.Fatalf("recipient cert missing after re-mint: %v", err)
	}

	// A path-shaped prefix must be refused before touching the store.
	if _, err := m.Mint(context.Background(), "../escape", "vp_key3", certmint.MintOptions{}); err == nil {
		t.Fatalf("path-shaped prefix must fail the mint")
	}

	// Unset dir = the pre-U7 behavior, no persist and no error.
	m2 := *m
	m2.RecipientKeysDir = ""
	if _, err := m2.Mint(context.Background(), "acme2", "vp_key4", certmint.MintOptions{}); err != nil {
		t.Fatalf("mint without recipient store: %v", err)
	}
	if _, err := os.Stat(filepath.Join(keysDir, "acme2.pem")); err == nil {
		t.Fatalf("mint without RecipientKeysDir must not persist")
	}
}

// TestMint_EKUArgv pins the EKU set handed to cert-sign: clientAuth alone for
// person/org mints, clientAuth+serverAuth when MintOptions.ServerAuth is set
// (machine mints: the leaf doubles as the pod router's listen cert, and a
// clientAuth-only listener fails every local mTLS handshake).
func TestMint_EKUArgv(t *testing.T) {
	stubDir := t.TempDir()
	argsFile := filepath.Join(stubDir, "args.txt")
	stub := filepath.Join(stubDir, "cert-sign")
	script := "#!/bin/sh\n" +
		"echo \"$@\" > " + argsFile + "\n" +
		"while [ $# -gt 0 ]; do\n" +
		"  case $1 in\n" +
		"    --out-cert) echo stub-cert > \"$2\"; shift 2;;\n" +
		"    --out-key)  echo stub-key  > \"$2\"; shift 2;;\n" +
		"    *) shift;;\n" +
		"  esac\n" +
		"done\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}

	mint := func(opts certmint.MintOptions) string {
		t.Helper()
		m := &certmint.Minter{
			BinaryPath:      stub,
			Fake:            false,
			CABundleDir:     stubDir,
			AgeIdentityPath: filepath.Join(stubDir, "age.key"),
			VendorBundleDir: t.TempDir(),
		}
		if _, err := m.Mint(context.Background(), "acme", "vp_acme_testkey", opts); err != nil {
			t.Fatalf("mint: %v", err)
		}
		argv, err := os.ReadFile(argsFile)
		if err != nil {
			t.Fatalf("stub argv not recorded: %v", err)
		}
		return string(argv)
	}

	if argv := mint(certmint.MintOptions{}); !strings.Contains(argv, "--eku clientAuth ") {
		t.Errorf("person/org mint: want --eku clientAuth alone, argv: %s", argv)
	}
	if argv := mint(certmint.MintOptions{ServerAuth: true}); !strings.Contains(argv, "--eku clientAuth,serverAuth") {
		t.Errorf("machine mint: want --eku clientAuth,serverAuth, argv: %s", argv)
	}
}

// TestMint_OrgArgv pins the Subject O handed to cert-sign: --org <value> as one
// argv element when Minter.Org is set (spaces survive), and no --org at all when
// it is empty or blank, so cert-sign's built-in default applies and a cert-sign
// that predates the flag keeps working.
func TestMint_OrgArgv(t *testing.T) {
	stubDir := t.TempDir()
	argsFile := filepath.Join(stubDir, "args.txt")
	stub := filepath.Join(stubDir, "cert-sign")
	// One argv element per line, so a value with a space is visible as one.
	script := "#!/bin/sh\n" +
		": > " + argsFile + "\n" +
		"for a in \"$@\"; do printf '%s\\n' \"$a\" >> " + argsFile + "; done\n" +
		"while [ $# -gt 0 ]; do\n" +
		"  case $1 in\n" +
		"    --out-cert) echo stub-cert > \"$2\"; shift 2;;\n" +
		"    --out-key)  echo stub-key  > \"$2\"; shift 2;;\n" +
		"    *) shift;;\n" +
		"  esac\n" +
		"done\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}

	mint := func(org string) []string {
		t.Helper()
		m := &certmint.Minter{
			BinaryPath:      stub,
			CABundleDir:     stubDir,
			AgeIdentityPath: filepath.Join(stubDir, "age.key"),
			VendorBundleDir: t.TempDir(),
			Org:             org,
		}
		if _, err := m.Mint(context.Background(), "acme", "vp_acme_testkey", certmint.MintOptions{}); err != nil {
			t.Fatalf("mint: %v", err)
		}
		argv, err := os.ReadFile(argsFile)
		if err != nil {
			t.Fatalf("stub argv not recorded: %v", err)
		}
		return strings.Split(strings.TrimRight(string(argv), "\n"), "\n")
	}
	orgArg := func(argv []string) (string, bool) {
		for i, a := range argv {
			if a == "--org" {
				if i+1 < len(argv) {
					return argv[i+1], true
				}
				return "", true
			}
		}
		return "", false
	}

	if v, ok := orgArg(mint("Example Org")); !ok || v != "Example Org" {
		t.Errorf("Org set: want --org \"Example Org\", got present=%v value=%q", ok, v)
	}
	if v, ok := orgArg(mint("  Example Org  ")); !ok || v != "Example Org" {
		t.Errorf("Org padded: want --org trimmed to \"Example Org\", got present=%v value=%q", ok, v)
	}
	for _, empty := range []string{"", "   "} {
		if _, ok := orgArg(mint(empty)); ok {
			t.Errorf("Org %q: want no --org in the argv", empty)
		}
	}
}

// A cert-sign that predates --org fails with Go's flag error; the mint error
// names the fix instead of passing the bare usage text through.
func TestMint_OrgRejectedByOldCertSign(t *testing.T) {
	stubDir := t.TempDir()
	stub := filepath.Join(stubDir, "cert-sign")
	script := "#!/bin/sh\n" +
		"for a in \"$@\"; do [ \"$a\" = --org ] && { echo 'flag provided but not defined: -org' >&2; exit 2; }; done\n" +
		"exit 0\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	m := &certmint.Minter{
		BinaryPath:      stub,
		CABundleDir:     stubDir,
		AgeIdentityPath: filepath.Join(stubDir, "age.key"),
		VendorBundleDir: t.TempDir(),
		Org:             "Example Org",
	}
	_, err := m.Mint(context.Background(), "acme", "vp_acme_testkey", certmint.MintOptions{})
	if err == nil {
		t.Fatal("want an error from a cert-sign that rejects --org")
	}
	if !strings.Contains(err.Error(), "predates the flag") {
		t.Errorf("error does not name the fix: %v", err)
	}
}

// A .cbor that nobody can verify is not a profile bundle, it is a file.
// goat-cli compiles in no CA root: roots are operator-supplied, so one binary
// works everywhere. That puts the burden here: whoever hands out a .cbor must
// hand out the public key that verifies it, or the recipient is stuck at step
// one.
//
// Asserts the pairing (both files, sibling names), that NOTES.md teaches the
// two-command install rather than the old one-command one, and that leaving the
// pubkey unconfigured degrades to omitting the file rather than failing a mint.
func TestMint_ShipsTheCAThatVerifiesTheCbor(t *testing.T) {
	dir := t.TempDir()
	root := filepath.Join(dir, "fake-root.pem")
	if err := os.WriteFile(root, []byte("FAKE ROOT"), 0o644); err != nil {
		t.Fatalf("seed root: %v", err)
	}
	caPub := filepath.Join(dir, "dev-galaxy-prod-ca-ecdsa-2026-08-03.pem")
	if err := os.WriteFile(caPub, []byte("FAKE CA PUBKEY"), 0o644); err != nil {
		t.Fatalf("seed ca pubkey: %v", err)
	}

	// Stand in for bundle-create: writes whatever --out names, so Mint takes
	// the configured path without a real CA or age identity.
	bc := filepath.Join(dir, "bundle-create")
	script := "#!/bin/sh\nwhile [ $# -gt 0 ]; do if [ \"$1\" = \"--out\" ]; then echo cbor > \"$2\"; fi; shift; done\n"
	if err := os.WriteFile(bc, []byte(script), 0o755); err != nil {
		t.Fatalf("seed bundle-create: %v", err)
	}

	newMinter := func(bundleCreate, caPubkey string) *certmint.Minter {
		return &certmint.Minter{
			Fake:             true,
			CARoot:           root,
			BundleCreatePath: bundleCreate,
			BundleCAPubkey:   caPubkey,
			CABundleDir:      dir,
			AgeIdentityPath:  filepath.Join(dir, "age.key"),
			VendorBundleDir:  filepath.Join(dir, "bundles"),
			RouterEndpoint:   "tls/zenoh.galaxy-prod.example:7447",
			PortalURL:        "https://portal.galaxy-prod.example",
			RootCertName:     "galaxy-prod-ca-root.pem",
			TarballStem:      "galaxy-prod-onboarding",
		}
	}

	res, err := newMinter(bc, caPub).Mint(
		context.Background(), "lab-dev", "vp_ltu_testkey", certmint.MintOptions{})
	if err != nil {
		t.Fatalf("mint: %v", err)
	}
	bodies, names := readTarball(t, res.TarballPath)

	if !names["lab-dev-goat-cli.cbor"] {
		t.Fatalf("no .cbor in the tarball, so this test proves nothing; members=%v", names)
	}
	// Sibling names: the two files are useless apart, and a directory listing
	// should make the pairing obvious.
	if !names["lab-dev-goat-cli-ca.pem"] {
		t.Errorf("tarball ships a .cbor with no CA to verify it; members=%v", names)
	}
	if got := bodies["lab-dev-goat-cli-ca.pem"]; got != "FAKE CA PUBKEY" {
		t.Errorf("CA file body = %q, want the configured pubkey", got)
	}

	notes := bodies["NOTES.md"]
	if !strings.Contains(notes, "goat ca add lab-dev-goat-cli-ca.pem") {
		t.Errorf("NOTES.md does not tell the reader to add the root before init:\n%s", notes)
	}
	// The trust step is worthless if the reader is not told to check it against
	// something other than the file they were just handed.
	if !strings.Contains(notes, "fingerprint") {
		t.Errorf("NOTES.md ships a root without telling anyone to verify it:\n%s", notes)
	}
	if i, j := strings.Index(notes, "goat ca add"), strings.Index(notes, "goat profile init"); i < 0 || j < 0 || i > j {
		t.Errorf("NOTES.md orders the steps wrong — `ca add` must precede `profile init`")
	}

	// A goatnet that distributes roots some other way leaves BundleCAPubkey
	// unset. That must degrade to "no CA file in the tarball", not to a failed
	// mint — an unconfigured convenience must never block issuing credentials.
	res2, err := newMinter(bc, "").Mint(
		context.Background(), "lab-dev2", "vp_ltu2_testkey", certmint.MintOptions{})
	if err != nil {
		t.Fatalf("mint with no BundleCAPubkey must still succeed: %v", err)
	}
	_, names2 := readTarball(t, res2.TarballPath)
	if names2["lab-dev2-goat-cli-ca.pem"] {
		t.Errorf("shipped a CA pubkey that was never configured; members=%v", names2)
	}
	if !names2["lab-dev2-goat-cli.cbor"] {
		t.Errorf("the .cbor must still ship; members=%v", names2)
	}
}
