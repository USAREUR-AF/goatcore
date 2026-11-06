package welcome

import (
	"strings"
	"testing"
)

// zagrosCatalog is the NO-GUIDE configuration: a goatnet that has not deployed the
// onboarding guide page, where the email still renders every section inline.
// That is a real supported configuration (Catalog.Guide blank → old behavior),
// and the long-standing assertions below are its contract. Slim-path tests use
// zagrosCatalogWithGuide.
func zagrosCatalog() Catalog {
	c := DeriveCatalog("zagros",
		"https://netbird.zagros.example",
		"https://get.zagros.netbird.zagros.example",
		"https://portal.zagros.netbird.zagros.example")
	c.Guide = ""
	c.CollabURL = "https://collab.goat.example"
	c.RepoURL = "https://github.com/example-org/goat"
	return c
}

// zagrosCatalogWithGuide is a goatnet WITH the guide deployed — the email slims to
// credentials plus a pointer.
func zagrosCatalogWithGuide() Catalog {
	return DeriveCatalog("zagros",
		"https://netbird.zagros.example",
		"https://get.zagros.netbird.zagros.example",
		"https://portal.zagros.netbird.zagros.example")
}

func TestDeriveCatalog_derivesSurfacesFromPortalHost(t *testing.T) {
	c := zagrosCatalog()
	cases := map[string]string{
		"auth":        "https://auth.zagros.netbird.zagros.example",
		"panoscope":   "https://panoscope.zagros.netbird.zagros.example",
		"sandbox":     "https://sandbox.zagros.netbird.zagros.example",
		"adminConsol": "https://admin.auth.zagros.netbird.zagros.example",
	}
	got := map[string]string{"auth": c.AuthURL, "panoscope": c.Panoscope, "sandbox": c.Sandbox, "adminConsol": c.AdminConsole}
	for k, want := range cases {
		if got[k] != want {
			t.Errorf("%s = %q, want %q", k, got[k], want)
		}
	}
}

func TestDisplayName_uppercasesRealGoatnetKeepsFallback(t *testing.T) {
	cases := map[string]string{
		"zagros":       "ZAGROS",
		"nova":  "NOVA",
		"  zagros  ":   "ZAGROS", // trimmed
		"":           "the data fabric",
		"the-goat-1": "THE-GOAT-1",
	}
	for in, want := range cases {
		if got := DisplayName(in); got != want {
			t.Errorf("DisplayName(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestDeriveCatalog_blankPortalLeavesDerivedBlank(t *testing.T) {
	c := DeriveCatalog("x", "https://netbird.x", "", "")
	if c.Panoscope != "" || c.AuthURL != "" || c.AdminConsole != "" {
		t.Errorf("blank portal should leave derived surfaces blank, got %+v", c)
	}
}

func TestRender_developerPublisher(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Joshua Belke", Handle: "user8x", Tier: "developer",
			Groups: []string{"DEVELOPER", "OBSERVER"}, FabricAccess: true, Publisher: true, AdminConsole: false},
		Artifacts{SetupKey: "KEY-123", RecoveryLink: "https://auth.zagros.netbird.zagros.example/recovery?flow=abc", RecoveryCode: "580180",
			BundleURL: "https://portal.zagros.netbird.zagros.example/api/org-bundle/abc/tok.tar.gz"})

	must := []string{
		"Joshua Belke — ZAGROS Welcome Package",
		"data fabric: Publisher",
		"--setup-key KEY-123",
		"--management-url https://netbird.zagros.example",
		"/recovery?flow=abc",
		"code: 580180",
		"get.zagros.netbird.zagros.example", // CLI step
		"goat tutor",                         // learn-the-fabric pointer
		"panoscope.zagros.netbird.zagros.example",
		"STEP 5 — use the data fabric (you're a Publisher)",
		"come as a bundle",        // the fabric credentials bundle
		"/api/org-bundle/abc/tok", // the mesh download LINK is present
		"goat profile init",       // load-the-cbor instruction
		"goat-cli.cbor",           // the config file in the bundle
		"first-publisher.py",      // runnable proof
		"rest-pub-bridge",         // fallback publish path
		"own namespace",           // scoped write
		"not part of",             // non-admin note
	}
	for _, m := range must {
		if !strings.Contains(out, m) {
			t.Errorf("developer package missing %q\n---\n%s", m, out)
		}
	}
	// The sandbox must NOT be presented as a way to publish to the live fabric,
	// and the cert must NOT be claimed as a Portal self-service item.
	if strings.Contains(out, "in-browser via the Sandbox") {
		t.Error("Step 5 must not present the Sandbox as a publish path — it is disconnected from the live fabric")
	}
	if strings.Contains(out, "shown under your identity in the Portal") {
		t.Error("Step 5 must not claim the cert is a Portal self-service download — it is an OOB bundle")
	}
	if !strings.Contains(out, "NOT the live fabric") {
		t.Error("Step 5 must disclaim that the Sandbox is not the live fabric")
	}
	if strings.Contains(out, "Admin console (your tier grants this)") {
		t.Error("developer should NOT get the admin console surface")
	}
}

func TestRender_collabSection(t *testing.T) {
	base := Recipient{Name: "Pat Liaison", Handle: "pat", Tier: "developer", Groups: []string{"DEVELOPER"}}

	// With a collab account + one-time link, the collaboration section renders.
	out := Render(zagrosCatalog(), base,
		Artifacts{SetupKey: "K", CollabAccount: "pat",
			CollabSetupLink: "https://idm.collab.goat.example/ui/reset?token=ABC"})
	for _, m := range []string{
		"COLLABORATION ENVIRONMENT — set up your account",
		"Your collab username:  pat",
		"https://idm.collab.goat.example/ui/reset",
		"ABC",
		"https://collab.goat.example",
		"Multi-factor auth is required",
	} {
		if !strings.Contains(out, m) {
			t.Errorf("collab section missing %q", m)
		}
	}

	// F-315: the token must NEVER ride in a fetchable URL. A mail-security
	// gateway spends a one-shot credential link in transit, and the reader is
	// then told the token was "reused in another browser" with no way through.
	// Page and code travel
	// separately so neither half is both fetchable and spendable.
	if strings.Contains(out, "reset?token=") {
		t.Error("collab setup must not embed the token in a clickable URL — a mail scanner spends it before the reader")
	}

	// A link the splitter cannot parse is still delivered whole: an
	// undeliverable credential is worse than a scannable one.
	odd := Render(zagrosCatalog(), base,
		Artifacts{SetupKey: "K", CollabAccount: "pat", CollabSetupLink: "ask-your-admin"})
	if !strings.Contains(odd, "ask-your-admin") {
		t.Error("an unsplittable setup link must still be rendered in full")
	}

	// No collab link → section omitted entirely.
	no := Render(zagrosCatalog(), base, Artifacts{SetupKey: "K"})
	if strings.Contains(no, "COLLABORATION ENVIRONMENT") {
		t.Error("collab section must be omitted when no setup link is present")
	}
}

// The re-issue paths — the console's welcome download and the resend — carry
// CollabAccount but never the one-time set-up link, which is not stored. Both
// must still say the person has a collaboration account and where a fresh
// set-up code comes from, and neither may imply a credential it does not hold.
func TestRender_collabSectionWithoutSetupLink(t *testing.T) {
	base := Recipient{Name: "Pat Liaison", Handle: "pat", Tier: "developer", Groups: []string{"DEVELOPER"}}
	art := Artifacts{SetupKey: "K", CollabAccount: "pat"}

	for name, out := range map[string]string{
		"download": Render(zagrosCatalog(), base, art),
		"reissue":  RenderReissue(zagrosCatalog(), base, art),
	} {
		for _, want := range []string{
			"COLLABORATION ENVIRONMENT — you have an account",
			"Your collab username:  pat",
			"https://collab.goat.example",
			"https://admin.auth.zagros.netbird.zagros.example/setup-code",
		} {
			if !strings.Contains(out, want) {
				t.Errorf("%s package: collab block missing %q", name, want)
			}
		}
		// No credential may appear, and nothing may promise one in this package.
		for _, unwanted := range []string{
			"COLLABORATION ENVIRONMENT — set up your account",
			"Paste this code",
			"valid 24 hours",
			"idm.collab.goat.example",
		} {
			if strings.Contains(out, unwanted) {
				t.Errorf("%s package: collab block must not contain %q", name, unwanted)
			}
		}
	}

	// A goatnet with no console surface has no self-service page to name.
	cat := zagrosCatalog()
	cat.AdminConsole = ""
	if out := Render(cat, base, art); !strings.Contains(out, "Ask an admin for a fresh one") {
		t.Errorf("with no console surface the block must send the person to an admin:\n%s", out)
	}

	// Neither an account nor a link: nothing to name, so no block at all.
	if out := Render(zagrosCatalog(), base, Artifacts{SetupKey: "K"}); strings.Contains(out, "COLLABORATION ENVIRONMENT") {
		t.Error("collab block must be omitted when the person has no collab account")
	}
}

func TestSplitCredentialLink(t *testing.T) {
	page, code := splitCredentialLink("https://idm.collab.goat.example/ui/reset?token=AB-CD-EF")
	if page != "https://idm.collab.goat.example/ui/reset" || code != "AB-CD-EF" {
		t.Fatalf("split = (%q, %q)", page, code)
	}
	// Anything without a token query param is left for the caller to render
	// whole — signaled by an empty code, never by a partial result.
	for _, raw := range []string{"", "not a url", "https://idm.example/ui/reset", "/ui/reset?token=X"} {
		if p, c := splitCredentialLink(raw); p != "" || c != "" {
			t.Errorf("splitCredentialLink(%q) = (%q,%q), want empty so the caller renders it unchanged", raw, p, c)
		}
	}
}

// The mesh-login step leads with the CODE, because that is the half that
// survives a scanned inbox; the link stays for readers whose mail is
// not scanned.
func TestRender_loginStepLeadsWithTheCode(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Pat", Handle: "pat", Tier: "developer"},
		Artifacts{SetupKey: "K", RecoveryLink: "https://auth.example/recover?flow=1", RecoveryCode: "123456"})
	iCode := strings.Index(out, "code: 123456")
	iLink := strings.Index(out, "page: https://auth.example/recover?flow=1")
	if iCode < 0 || iLink < 0 {
		t.Fatalf("login step must carry both the code and the page (code=%d link=%d)", iCode, iLink)
	}
	if !strings.Contains(out, "The code still works.") {
		t.Error("the reader must be told the code survives when the link reports it was already used")
	}
}

func TestRender_observerWithFabricAccessGetsReadBundle(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Jeroen", Handle: "jeroen", Tier: "browse",
			Groups: []string{"OBSERVER"}, FabricAccess: true, Publisher: false},
		Artifacts{SetupKey: "K"})

	if !strings.Contains(out, "STEP 5 — use the data fabric (read / subscribe)") {
		t.Error("an observer with fabric access should get a read/subscribe fabric section")
	}
	if !strings.Contains(out, "first-subscriber.py") {
		t.Error("observer fabric section should point at first-subscriber.py")
	}
	// The bundle contents legitimately name both scripts; the observer just must
	// not be told to PUBLISH (that instruction / the Publisher header).
	if strings.Contains(out, "you're a Publisher") || strings.Contains(out, "publishes to\nyour own namespace") || strings.Contains(out, "run first-publisher.py") {
		t.Error("a non-publisher observer must not be told to publish")
	}
	if !strings.Contains(out, "come as a bundle") {
		t.Error("observer still receives their identity as a bundle")
	}
	// No BundleURL supplied → the bundle step falls back to the OOB hand-off.
	if !strings.Contains(out, "hands it to you out of band") {
		t.Error("without a download link, the bundle step should fall back to the OOB instruction")
	}
}

func TestRender_observerNoPublishNoKey(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Obs", Handle: "obs", Tier: "browse",
			Groups: []string{"OBSERVER"}, Publisher: false, AdminConsole: false},
		Artifacts{}) // no setup key, no recovery link

	if strings.Contains(out, "STEP 5") {
		t.Error("a browse participant with no fabric access should not get a fabric section")
	}
	if !strings.Contains(out, "operator will hand you a one-time mesh enrollment key") {
		t.Error("missing setup key should degrade to an instruction, not a blank")
	}
	if !strings.Contains(out, "panoscope.zagros.netbird.zagros.example") {
		t.Error("observer should still get inspection surfaces")
	}
}

func TestRender_adminTierGetsConsole(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Admin", Handle: "adm", Tier: "platform-admin", AdminConsole: true},
		Artifacts{SetupKey: "K"})
	if !strings.Contains(out, "Admin console (your tier grants this)") {
		t.Error("platform-admin should get the admin console surface")
	}
	if strings.Contains(out, "not part of") {
		t.Error("admin tier should not get the 'not part of your tier' note")
	}
}

// --- github repo access + admin/operator section --------------------------

func TestRender_githubSummaryAndRepoSection(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer",
			Groups: []string{"DEVELOPER", "OBSERVER"}, FabricAccess: true, Publisher: true,
			GitHubUsername: "octocat",
			RepoGrants: []RepoGrant{
				{Owner: "example-org", Repo: "example-pod", Permission: "pull", Outcome: "invited"},
				{Owner: "example-org", Repo: "hello-galaxy-access-mgmt", Permission: "pull", Outcome: "added"},
			}},
		Artifacts{SetupKey: "K"})

	must := []string{
		"| github: octocat", // summary line append
		"Repository access", // the section heading
		"example-org/example-pod (pull) — https://github.com/example-org/example-pod",
		"example-org/hello-galaxy-access-mgmt (pull) — https://github.com/example-org/hello-galaxy-access-mgmt",
	}
	for _, m := range must {
		if !strings.Contains(out, m) {
			t.Errorf("developer-with-repos package missing %q\n---\n%s", m, out)
		}
	}
}

func TestRender_repoGrantSourceTag(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer",
			Groups: []string{"DEVELOPER"}, GitHubUsername: "octocat",
			RepoGrants: []RepoGrant{
				{Owner: "example-org", Repo: "example-pod", Permission: "pull", Outcome: "added", Source: "policy"},
				{Owner: "example-org", Repo: "extra-repo", Permission: "push", Outcome: "invited", Source: "audible"},
			}},
		Artifacts{SetupKey: "K"})
	must := []string{
		"example-org/example-pod (pull) (policy) — https://github.com/example-org/example-pod",
		"example-org/extra-repo (push) (audible) — https://github.com/example-org/extra-repo",
	}
	for _, m := range must {
		if !strings.Contains(out, m) {
			t.Errorf("source-tagged repo line missing %q\n---\n%s", m, out)
		}
	}
}

func TestRender_developerNoGithubGetsNudge(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer",
			Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K"})
	if !strings.Contains(out, "You gave no GitHub username") || !strings.Contains(out, "/github") {
		t.Errorf("a code-facing developer with no github username should get the nudge:\n%s", out)
	}
	if strings.Contains(out, "| github:") {
		t.Error("no github username should not append to the summary")
	}
}

func TestRender_browseNoGithubRendersNoRepoSection(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Obs", Handle: "obs", Tier: "browse", Groups: []string{"OBSERVER"}},
		Artifacts{SetupKey: "K"})
	if strings.Contains(out, "Repository access") {
		t.Errorf("browse tier with no github should get no repo-access section:\n%s", out)
	}
}

func TestRender_adminOperatorSectionForEscalatedTiers(t *testing.T) {
	for _, tier := range []string{"iam-admin", "platform-admin"} {
		out := Render(zagrosCatalog(),
			Recipient{Name: "Adm", Handle: "adm-op", Tier: tier, AdminConsole: true, AdminOperator: true},
			Artifacts{SetupKey: "K"})
		must := []string{
			"Operator setup",
			"hand-delivered out of band",   // OOB key delivery discipline
			"GOAT_OPERATOR_HANDLE=adm-op",  // operator env
			"ZAGROS_PORTAL_OPERATOR_API_KEY", // portal admin auth
			"ZAGROS_NB_API_TOKEN",            // mint own NetBird PAT
			"goat admin onboard <handle> --env zagros",
			"goat admin queue ls|approve|reject", // current CLI surface (no stale sponsor-approve verbs)
			"goat admin sponsorship <id> --max-tier <tier>",
			"goat audit ops",
			"admin-operator-guide.md",                          // orientation pointer
			"attributed to it",                                 // attribution note
			"SHARED operator account",                          // host shell is not a per-handle login
			"git clone https://github.com/example-org/goat.git && cd goat", // onboarding-others workflow
			"ops/operators/example-vendor.yml.example",         // manifest scaffolding
			"validate.py",                                      // manifest validation gate
			"netbird.auto_groups",                              // manifest field vocabulary (v3 parity)
		}
		for _, m := range must {
			if !strings.Contains(out, m) {
				t.Errorf("%s operator section missing %q\n---\n%s", tier, m, out)
			}
		}
		// The email must POINT at the OOB bundle, never embed a private key.
		if strings.Contains(out, "-----BEGIN") {
			t.Errorf("%s operator section must never embed key material", tier)
		}
		// Stale command forms flagged by issue #152 must be gone.
		for _, stale := range []string{"sponsor-approve", "goat admin invites ls|revoke"} {
			if strings.Contains(out, stale) {
				t.Errorf("%s operator section still carries stale command %q", tier, stale)
			}
		}
		// The host-shell line must NOT fabricate a per-handle login (adm-op@...).
		if strings.Contains(out, "ssh adm-op@") {
			t.Errorf("%s operator section must not present a per-handle SSH login:\n%s", tier, out)
		}
	}
}

func TestRender_adminOperatorSectionAbsentForNonAdmin(t *testing.T) {
	for _, tier := range []string{"browse", "developer"} {
		out := Render(zagrosCatalog(),
			Recipient{Name: "X", Handle: "x", Tier: tier,
				Groups: []string{"DEVELOPER"}, FabricAccess: tier == "developer"},
			Artifacts{SetupKey: "K"})
		if strings.Contains(out, "Operator setup") {
			t.Errorf("%s tier must NOT get the operator setup section:\n%s", tier, out)
		}
	}
}

func TestRender_adminOperatorHostGeneric(t *testing.T) {
	// No MgmtHost and no shared SSH user → a generic shared-account instruction,
	// never a fabricated per-handle login (the old `ssh <handle>@host` was wrong:
	// the mgmt host is reached via ONE shared operator account, issue #152).
	out := Render(zagrosCatalog(),
		Recipient{Name: "Adm", Handle: "adm-op", Tier: "platform-admin", AdminConsole: true, AdminOperator: true},
		Artifacts{})
	if !strings.Contains(out, "SHARED operator account") {
		t.Errorf("missing mgmt host should render the shared-account instruction:\n%s", out)
	}
	if strings.Contains(out, "ssh adm-op@") {
		t.Errorf("must never fabricate a per-handle SSH login:\n%s", out)
	}
	if !strings.Contains(out, "aren't attributed") {
		t.Errorf("host-shell note must keep the no-attribution caveat:\n%s", out)
	}
}

func TestRender_adminOperatorHostSharedAccountConfigured(t *testing.T) {
	// When the per-goatnet shared account + host are configured, render them
	// concretely — still flagged as shared/not-your-handle, still not attributed.
	out := Render(zagrosCatalog(),
		Recipient{Name: "Adm", Handle: "adm-op", Tier: "platform-admin",
			AdminConsole: true, AdminOperator: true,
			MgmtSSHUser: "goat", MgmtHost: "mgmt.example"},
		Artifacts{})
	if !strings.Contains(out, "ssh goat@mgmt.example") {
		t.Errorf("configured shared account + host should render concretely:\n%s", out)
	}
	if !strings.Contains(out, "Everyone shares the goat account") {
		t.Errorf("even configured, the account must be flagged as shared:\n%s", out)
	}
	if !strings.Contains(out, "aren't attributed to you") {
		t.Errorf("configured host-shell must keep the no-attribution caveat:\n%s", out)
	}
}

// TestRender_keylessButKnownMgmtShowsEnrollTarget covers the keyless-enrollee
// branch: with no setup key but a known mgmt URL, STEP 1 must still surface the
// enrollment command target so the enrollee isn't dead-ended (was: blank).
func TestRender_keylessButKnownMgmtShowsEnrollTarget(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Nokey", Handle: "nokey", Tier: "browse"},
		Artifacts{}) // no setup key
	if !strings.Contains(out, "--management-url https://netbird.zagros.example --setup-key <key>") {
		t.Errorf("keyless enrollee with a known mgmt URL should still see the enroll target:\n%s", out)
	}
}

// TestRender_keylessAndNoMgmtDegradesToInstruction: with neither key nor mgmt URL
// there is no target to show, so it falls back to the plain operator instruction.
func TestRender_keylessAndNoMgmtDegradesToInstruction(t *testing.T) {
	out := Render(DeriveCatalog("x", "", "", ""),
		Recipient{Name: "Nokey", Handle: "nokey", Tier: "browse"},
		Artifacts{})
	if strings.Contains(out, "--management-url") {
		t.Errorf("no mgmt URL means no netbird command should be fabricated:\n%s", out)
	}
	if !strings.Contains(out, "operator will hand you a one-time mesh enrollment key") {
		t.Error("keyless + no mgmt should degrade to the plain instruction")
	}
}

func TestEnrollCommand_rosenpassToggle(t *testing.T) {
	// The rosenpass-off command must mention rosenpass: the flag is stored in the
	// client's local config and PERSISTS across netbird down/up and across
	// re-enrollment with a fresh setup key, and only an explicit
	// --enable-rosenpass=false clears it.
	//
	// So "off" has to mean "turn it off", not "decline to mention it".
	// Otherwise a revoke-and-remint remediation hands people a command that
	// silently leaves the 512 KB key exchange running.
	off := EnrollCommand("https://netbird.zagros.example", "KEY-123", false, "zagros")
	if off != "sudo netbird up --profile zagros --management-url https://netbird.zagros.example --setup-key KEY-123 --enable-rosenpass=false --rosenpass-permissive=false" {
		t.Errorf("rosenpass-off command unexpected: %q", off)
	}
	if !strings.Contains(off, "--enable-rosenpass=false") {
		t.Errorf("rosenpass-off must EXPLICITLY disable, not merely omit: %q", off)
	}
	on := EnrollCommand("https://netbird.zagros.example", "KEY-123", true, "zagros")
	if !strings.Contains(on, "--enable-rosenpass --rosenpass-permissive") {
		t.Errorf("rosenpass-on must append both flags: %q", on)
	}
}

func TestRender_rosenpassFlagsGatedByCatalog(t *testing.T) {
	rcpt := Recipient{Name: "Ada", Handle: "ada", Tier: "developer",
		Groups: []string{"DEVELOPER"}, FabricAccess: true}
	art := Artifacts{SetupKey: "KEY-123"}

	// Default catalog (toggle off): the welcome package must emit the DISABLING
	// form, not silence. See TestEnrollCommand_rosenpassToggle — the flag
	// persists on the client, so omission leaves a previously-enabled peer
	// enabled.
	if out := Render(zagrosCatalog(), rcpt, art); !strings.Contains(out, "--enable-rosenpass=false") {
		t.Errorf("default catalog must explicitly DISABLE rosenpass, not stay silent")
	}
	if out := Render(zagrosCatalog(), rcpt, art); strings.Contains(out, "--enable-rosenpass --rosenpass-permissive") {
		t.Errorf("default catalog must not emit the ENABLING flags")
	}
	// Toggle on: the enrollment command carries both flags.
	cat := zagrosCatalog()
	cat.RosenpassEnabled = true
	out := Render(cat, rcpt, art)
	if !strings.Contains(out, "--enable-rosenpass --rosenpass-permissive") {
		t.Errorf("RosenpassEnabled catalog must append the flags\n---\n%s", out)
	}
}

func TestRenderUpgrade_escalationAcknowledgesExistingIdentity(t *testing.T) {
	out := RenderUpgrade(zagrosCatalog(),
		Recipient{Name: "Morgan", Handle: "morgan", Tier: "platform-admin",
			Groups: []string{"DEVELOPER", "OBSERVER"}, FabricAccess: true, Publisher: true,
			AdminConsole: true, AdminOperator: true},
		Upgrade{PrevGroups: []string{"GRAFANA-READONLY"}, NewGroups: []string{"GRAFANA-READONLY", "PLATFORM-ADMIN"}})

	must := []string{
		"Morgan — ZAGROS access upgrade",
		"not a new",                                 // upgrade vs new-provisioning framing
		"Nothing was re-provisioned",                // explicit acknowledgment
		"+ PLATFORM-ADMIN",                          // the delta
		"(already done)",                            // step 1 reframed
		"(already set)",                             // step 2 reframed
		"sign out and back in",                      // the JWT group-refresh nuance
		"skip if already installed",                 // step 3 reframed
		"admin.auth.zagros.netbird.zagros.example", // newly-granted surface listed
		"Operator setup",                            // escalated tier gets the operator section
		"unchanged by this",                         // fabric bundle keeps working
		"first-publisher.py",                        // complete package retained
	}
	for _, m := range must {
		if !strings.Contains(out, m) {
			t.Errorf("upgrade package missing %q\n---\n%s", m, out)
		}
	}
	// One-time secrets must never appear in an upgrade email. The only
	// permitted --setup-key is the additional-device "<key>" placeholder.
	if strings.Count(out, "--setup-key") != strings.Count(out, "--setup-key <key>") {
		t.Errorf("upgrade package must not carry a real setup key\n---\n%s", out)
	}
	for _, banned := range []string{"code:", "/recovery?", "set your first password", "-----BEGIN"} {
		if strings.Contains(out, banned) {
			t.Errorf("upgrade package must not contain %q\n---\n%s", banned, out)
		}
	}
}

func TestRenderUpgrade_removalOnlyIsChangeNotUpgrade(t *testing.T) {
	out := RenderUpgrade(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", Groups: []string{"DEVELOPER"}},
		Upgrade{PrevGroups: []string{"PLATFORM-ADMIN"}, NewGroups: nil})
	if !strings.Contains(out, "ZAGROS access change") {
		t.Errorf("removal-only should render as access change:\n%s", out)
	}
	if !strings.Contains(out, "- PLATFORM-ADMIN") {
		t.Errorf("removal-only should list the removed group:\n%s", out)
	}
	if strings.Contains(out, "access upgrade") {
		t.Errorf("removal-only must not claim an upgrade:\n%s", out)
	}
	// Non-admin now — no operator section, and the non-admin note stands.
	if strings.Contains(out, "Operator setup") {
		t.Errorf("non-admin change must not include operator setup:\n%s", out)
	}
}

func TestUpgrade_diffIsCaseInsensitiveAndOrderPreserving(t *testing.T) {
	u := Upgrade{
		PrevGroups: []string{"grafana-readonly", "IAM-ADMIN"},
		NewGroups:  []string{"GRAFANA-READONLY", "PLATFORM-ADMIN", "data.view.all"},
	}
	added, removed := u.Added(), u.Removed()
	if len(added) != 2 || added[0] != "PLATFORM-ADMIN" || added[1] != "data.view.all" {
		t.Errorf("added = %v, want [PLATFORM-ADMIN data.view.all]", added)
	}
	if len(removed) != 1 || removed[0] != "IAM-ADMIN" {
		t.Errorf("removed = %v, want [IAM-ADMIN]", removed)
	}
}

func TestRender_adminTiersGetOnboardingGuide(t *testing.T) {
	for _, tier := range []string{"iam-admin", "platform-admin"} {
		out := Render(zagrosCatalog(),
			Recipient{Name: "Adm", Handle: "adm", Tier: tier, AdminConsole: true, AdminOperator: true},
			Artifacts{SetupKey: "K"})
		must := []string{
			"the two doors",
			"Onboarding → Invite",
			"ADMIN APPROVES",         // when the welcome email actually fires
			"not approved yet",       // the "no email yet" decoder
			"bare LOGIN only",        // the new-user-form trap
			"mesh-internal",          // why the recovery link dead-ends
			"Two group vocabularies", // DEVELOPER-in-login-field trap
		}
		for _, m := range must {
			if !strings.Contains(out, m) {
				t.Errorf("%s package missing onboarding-guide %q", tier, m)
			}
		}
	}
	// Upgrade mode inherits the section for a newly-escalated admin.
	up := RenderUpgrade(zagrosCatalog(),
		Recipient{Name: "Esc", Handle: "esc", Tier: "platform-admin", AdminConsole: true, AdminOperator: true},
		Upgrade{PrevGroups: nil, NewGroups: []string{"PLATFORM-ADMIN"}})
	if !strings.Contains(up, "the two doors") {
		t.Error("upgrade package for a new admin must carry the onboarding guide")
	}
}

func TestRender_nonAdminGetsNoOnboardingGuide(t *testing.T) {
	for _, tier := range []string{"browse", "developer"} {
		out := Render(zagrosCatalog(),
			Recipient{Name: "Dev", Handle: "dev", Tier: tier},
			Artifacts{SetupKey: "K"})
		if strings.Contains(out, "the two doors") {
			t.Errorf("%s must not get the admin onboarding guide", tier)
		}
	}
}

func TestRenderUpgrade_emptyDeltaIsCatchUpReference(t *testing.T) {
	out := RenderUpgrade(zagrosCatalog(),
		Recipient{Name: "Morgan", Handle: "morgan", Tier: "platform-admin",
			AdminConsole: true, AdminOperator: true},
		Upgrade{PrevGroups: []string{"PLATFORM-ADMIN"}, NewGroups: []string{"PLATFORM-ADMIN"}})
	must := []string{
		"ZAGROS access reference",    // catch-up title, not upgrade/change
		"catch-up",                 // explicit framing
		"Nothing",                  // nothing changed / re-provisioned
		"(already done)",           // steps still reframed
		"Operator setup",           // full admin package retained
		"the two doors",            // admin guide retained
		"/public/onboarding-guide", // deep-dive pointer, never the retired console page
		"?topic=identity-admin",    // straight to the admin topic
	}
	for _, m := range must {
		if !strings.Contains(out, m) {
			t.Errorf("reference package missing %q\n---\n%s", m, out)
		}
	}
	if strings.Contains(out, "What changed") {
		t.Error("empty delta must not render a what-changed block")
	}
}

// The admin deep-dive pointer must reach the GUIDE, never the console's
// retired /docs/onboarding page. That page was behind onboardGate, so an admin
// could open it and a sponsor-scoped reader mostly could not — and it had
// drifted out of date besides.
func TestRender_adminGuideLinksTheGuide(t *testing.T) {
	adm := Recipient{Name: "Adm", Handle: "adm", Tier: "platform-admin", AdminConsole: true}

	// Goatnet WITH a guide host: the mail already slims to credentials plus a
	// pointer at the unauthenticated guide, so the long admin section does not
	// render at all. What matters is that nothing points at the retired page.
	out := Render(zagrosCatalogWithGuide(), adm, Artifacts{SetupKey: "K"})
	if !strings.Contains(out, "https://guide.zagros.netbird.zagros.example/public/onboarding-guide") {
		t.Errorf("slim package must point at the guide host:\n%s", out)
	}
	if strings.Contains(out, "/docs/onboarding") {
		t.Errorf("nothing may link the retired console page:\n%s", out)
	}

	// Goatnet WITHOUT one: the long admin section renders, and its deep-dive
	// pointer falls back to the console's own copy of the same handler rather
	// than to /docs/onboarding, which no longer holds anything.
	out = Render(zagrosCatalog(), adm, Artifacts{SetupKey: "K"})
	want := "https://admin.auth.zagros.netbird.zagros.example/public/onboarding-guide?topic=identity-admin"
	if !strings.Contains(out, want) {
		t.Errorf("without a guide host the pointer must fall back to %q:\n%s", want, out)
	}
	if strings.Contains(out, "/docs/onboarding") {
		t.Errorf("nothing may link the retired console page:\n%s", out)
	}
}

// --- issue #152: welcome-package clarity/accuracy content -------------------

// TestRender_meshExplainedPlainly: the package must open by saying what "the
// mesh" is, so a non-networking enrollee understands why nothing loads yet.
func TestRender_meshExplainedPlainly(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "New", Handle: "new", Tier: "browse"},
		Artifacts{SetupKey: "K"})
	for _, m := range []string{"What is \"the mesh\"?", "like a VPN", "only works while NetBird is connected"} {
		if !strings.Contains(out, m) {
			t.Errorf("mesh explainer missing %q\n---\n%s", m, out)
		}
	}
}

// TestRender_step1CompleteAndOSAware: Step 1 must carry the install/service/
// verify steps, the Windows-no-sudo note, and — critically — the silent-ignore
// trap (`netbird up` while already connected does nothing) with the `down` fix.
func TestRender_step1CompleteAndOSAware(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "New", Handle: "new", Tier: "browse"},
		Artifacts{SetupKey: "K"})
	for _, m := range []string{
		"brew install netbird",                                       // per-OS install
		"https://docs.netbird.io/get-started/install/windows",        // Windows installer link
		"Windows — the commands have NO \"sudo\":",                   // Windows block heading
		"Command Prompt AS ADMINISTRATOR",                            // Windows shell
		"netbird service install",                                    // background service (macOS/Linux)
		"SILENTLY IGNORED",                                           // the wrong-mesh silent-ignore trap
		"`netbird down` (with sudo on macOS/Linux)",                  // its fix: disconnect first, both OSes
		"netbird status -d",                                          // verification
		"Management: Connected to https://netbird.zagros.example", // which-mesh check, from config
		".netbird.selfhosted",                                        // FQDN sanity check
	} {
		if !strings.Contains(out, m) {
			t.Errorf("step 1 missing %q\n---\n%s", m, out)
		}
	}
}

// TestRender_step5EnrollCommandCarriesADedicatedProfile: PR #1336 review
// (contributore) — a bare `netbird up` reuses whichever NetBird profile is
// currently active and silently overwrites its management URL and setup key,
// and (pre-v0.78 clients) a caller running the fix under sudo resolves that
// "active profile" against root's own state rather than the invoking user's.
// --profile <goatnet> fixes both by construction (see EnrollCommandNamed), so
// step 5's own command — not a separate note — is what every "run step 5
// again" fix (note A, note C, the hosted guide) now inherits for free.
func TestRender_step5EnrollCommandCarriesADedicatedProfile(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "New", Handle: "new", Tier: "browse"},
		Artifacts{SetupKey: "K"})
	if !strings.Contains(out, "sudo netbird up --profile zagros --management-url") {
		t.Errorf("macOS/Linux step 5 must carry --profile zagros\n---\n%s", out)
	}
	if n := strings.Count(out, "netbird up --profile zagros --management-url"); n != 2 {
		t.Errorf("expected exactly one macOS/Linux and one Windows enrollment line with --profile zagros, found %d\n---\n%s", n, out)
	}
}

// TestRender_blankGoatnetProfileFallsBackToAPlaceholder: profileID must never
// splice a blank or multi-word goatnet into --profile unquoted (welcome.go's
// own renderOperatorSetup has the identical `if env == "" { env = "<env>" }`
// guard for the same class of bug).
func TestRender_blankGoatnetProfileFallsBackToAPlaceholder(t *testing.T) {
	out := Render(Catalog{MgmtURL: "https://netbird.example.net"},
		Recipient{Name: "New", Handle: "new", Tier: "browse"},
		Artifacts{SetupKey: "K"})
	if !strings.Contains(out, "--profile <goatnet>") {
		t.Errorf("blank goatnet must fall back to the <goatnet> placeholder, not an empty --profile value\n---\n%s", out)
	}
	if strings.Contains(out, "--profile  ") || strings.Contains(out, "--profile --management-url") {
		t.Errorf("blank goatnet must not render an empty --profile argument\n---\n%s", out)
	}
}

// TestRender_step2RecoveryCodeExpiry: whenever a recovery code is present, the
// 24-hour expiry (ory_recovery_lifespan default) + the "won't load == mesh"
// note must be stated.
func TestRender_step2RecoveryCodeExpiry(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "New", Handle: "new", Tier: "browse"},
		Artifacts{SetupKey: "K", RecoveryLink: "https://auth.example/recovery?flow=q", RecoveryCode: "123456"})
	for _, m := range []string{"expires 24 HOURS", "won't load", "only resolves while NetBird is connected"} {
		if !strings.Contains(out, m) {
			t.Errorf("step 2 missing %q\n---\n%s", m, out)
		}
	}
}

// TestRender_cliStepPathAndNoReplaceNetbird: the CLI step (STEP 4 in the v3
// sequence) must teach the PATH move (the #1 "command not found" cause) and
// that goat does not replace NetBird.
func TestRender_cliStepPathAndNoReplaceNetbird(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", Groups: []string{"DEVELOPER"}},
		Artifacts{SetupKey: "K"})
	for _, m := range []string{"sudo mv goat /usr/local/bin/", "only apply to new sessions", "goat does NOT replace it"} {
		if !strings.Contains(out, m) {
			t.Errorf("CLI step missing %q\n---\n%s", m, out)
		}
	}
}

// TestRender_surfacesPortalLoginIsDifferent: the Portal must be carved out of
// the "single sign-on password" set, and the key note must be role-aware — the
// operator is told to use the OPERATOR key (v3 shows the vp_ line as contrast).
func TestRender_surfacesPortalLoginIsDifferent(t *testing.T) {
	// Non-operator developer → their ACTUAL scoped key inline, and NO
	// operator-key line. (This assertion used to look for the literal
	// "vp_<prefix>_" placeholder, back when the email described the key instead
	// of carrying it and told the reader to go find it in the Portal. A
	// fabric-enabled developer always has a real key, so the placeholder is no
	// longer a state the renderer can produce for one.)
	dev := Render(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", PortalAPIKey: "vp_dev_realkey"}) //nolint:gosec // test fixture; not a real credential
	for _, m := range []string{"does NOT take your Step-2 password", "vp_dev_realkey"} {
		if !strings.Contains(dev, m) {
			t.Errorf("developer Portal note missing %q\n---\n%s", m, dev)
		}
	}
	if strings.Contains(dev, "you use your OPERATOR key") {
		t.Error("a non-operator must not be told to use the operator key")
	}
	// Operator with NO scoped key → the operative instruction is the OPERATOR
	// key, and it is self-service (an admin approves; nothing is installed).
	op := Render(zagrosCatalog(),
		Recipient{Name: "Op", Handle: "op", Tier: "platform-admin", AdminConsole: true, AdminOperator: true},
		Artifacts{SetupKey: "K"})
	if !strings.Contains(op, "you use your OPERATOR key") {
		t.Errorf("operator Portal note should route to the operator key\n---\n%s", op)
	}
	if !strings.Contains(op, "/operator-key") {
		t.Errorf("operator Portal note must name the self-service page\n---\n%s", op)
	}
	if strings.Contains(op, "REQUEST it from an admin") {
		t.Errorf("the operator key is self-service — do not send them to an admin to get one\n---\n%s", op)
	}
}

// TestRender_adminWithScopedKeyGetsItInline: an admin-tier onboardee whose mint
// produced a scoped vp_ key must be GIVEN it. An operator key is a different
// credential, so suppressing the vp_ key leaves a platform admin with nothing to
// paste into the Portal and a 401 while the key sits in the artifacts this very
// email is rendered from.
func TestRender_adminWithScopedKeyGetsItInline(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Op", Handle: "op", Tier: "platform-admin", AdminConsole: true,
			AdminOperator: true, FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", PortalAPIKey: "vp_op_realkey"}) //nolint:gosec // test fixture; not a real credential
	if !strings.Contains(out, "vp_op_realkey") {
		t.Fatalf("admin tier must be given its own scoped key\n---\n%s", out)
	}
	for _, want := range []string{
		"TWO keys",                       // the two are additive, not alternatives
		"/operator-key",                  // where the second one comes from
		"rotation",                       // the rotate-to-refresh trap
		"not for the Portal sign-in box", // what the operator key is NOT for
	} {
		if !strings.Contains(out, want) {
			t.Errorf("admin Portal note missing %q\n---\n%s", want, out)
		}
	}
}

// TestRender_deferredAdminStillDefers: an admin whose credential is deferred has
// no key to print, and must still be sent to the mint-on-collect page rather
// than the inline branch.
func TestRender_deferredAdminStillDefers(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Op", Handle: "op", Tier: "platform-admin", AdminConsole: true,
			AdminOperator: true, FabricAccess: true},
		Artifacts{SetupKey: "K", CredentialDeferred: true})
	if strings.Contains(out, "TWO keys") {
		t.Errorf("a deferred admin has no scoped key to hand over\n---\n%s", out)
	}
}

// TestRender_surfacesBeforeCli asserts the v3 step sequence: surfaces are STEP
// 3 and the CLI is STEP 4 (they were swapped in the earlier package).
func TestRender_surfacesBeforeCli(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", Groups: []string{"DEVELOPER"},
			FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", BundleURL: "https://portal.example/api/org-bundle/x/tok.tar.gz"})
	iSurf := strings.Index(out, "STEP 3 — your surfaces")
	iCli := strings.Index(out, "STEP 4 — install the goat CLI")
	if iSurf < 0 || iCli < 0 {
		t.Fatalf("expected 'STEP 3 — your surfaces' and 'STEP 4 — install the goat CLI'\n---\n%s", out)
	}
	if iSurf > iCli {
		t.Errorf("surfaces (STEP 3) must come before the CLI (STEP 4), matching v3")
	}
	// The Sandbox cross-reference in Step 5 must track the surfaces step number.
	if strings.Contains(out, "the Sandbox (Step 4)") {
		t.Error("Step 5's Sandbox cross-reference must say (Step 3), not (Step 4)")
	}
}

// TestRender_step5DemoIsRunnable: the "prove it works" demo must be actually
// runnable — venv/pip, unpack+cd, subscriber-first ordering, and goat doctor.
func TestRender_step5DemoIsRunnable(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", BundleURL: "https://portal.example/api/org-bundle/x/tok.tar.gz"})
	for _, m := range []string{
		"tar -xzf",                           // unpack
		"pip install eclipse-zenoh protobuf", // python deps (the ModuleNotFoundError fix)
		"ORDER MATTERS",                      // subscriber-first
		"Terminal A (first)   python3 first-subscriber.py",
		"goat doctor",              // reachability check
		"never put it in OneDrive", // private-key hygiene (no cloud sync)
		"cloud-synced location",    // ...for any cloud-synced location
	} {
		if !strings.Contains(out, m) {
			t.Errorf("step 5 demo missing %q\n---\n%s", m, out)
		}
	}
	// The subscriber-first, two-terminal proof is publisher-only. An observer
	// must not be handed the ORDER-MATTERS publish drill.
	obs := Render(zagrosCatalog(),
		Recipient{Name: "Obs", Handle: "obs", Tier: "browse", Groups: []string{"OBSERVER"}, FabricAccess: true, Publisher: false},
		Artifacts{SetupKey: "K"})
	// (The bundle-contents line legitimately names both example scripts; the
	// observer must only be spared the ORDER-MATTERS two-terminal publish drill.)
	if strings.Contains(obs, "ORDER MATTERS") || strings.Contains(obs, "Terminal B (then)") {
		t.Errorf("observer must not receive the publisher proof drill:\n%s", obs)
	}
	// ...but the shared runnability setup (venv/unpack) still applies to them.
	if !strings.Contains(obs, "pip install eclipse-zenoh protobuf") || !strings.Contains(obs, "tar -xzf") {
		t.Errorf("observer should still get the venv/unpack setup:\n%s", obs)
	}
}

// TestRender_noHardcodedTemplateSecrets is the load-bearing guard for issue
// #152: the generator is pure-parameterized, so NONE of an operator's personal
// onboarding values (setup key, recovery flow/code, org id, bundle token,
// handle) may be baked into the code. Render with unrelated inputs and assert
// such literals never appear.
func TestRender_noHardcodedTemplateSecrets(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Someone", Handle: "someone", Tier: "platform-admin",
			Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true,
			AdminConsole: true, AdminOperator: true},
		Artifacts{SetupKey: "TESTKEY", RecoveryLink: "https://auth.example/recovery?flow=zzz",
			RecoveryCode: "000000", BundleURL: "https://portal.example/api/org-bundle/zzz/tok.tar.gz"})
	banned := map[string]string{
		"E3618398-FCF6-46D2-9585-4DC7CB534DEB": "template setup key",
		"573010":                               "template recovery code",
		"60473d6ab090a30e5b9b31ede09a32ce":     "operator org id / namespace",
		"fd68aef97f5c45e5fe64c7333416a1e5":     "template bundle token",
		"39fc853a-56c3-48c9-9783-dbcff93d08e8": "template recovery flow id",
		"user290x":                               "operator handle",
	}
	for lit, what := range banned {
		if strings.Contains(out, lit) {
			t.Errorf("rendered package must never contain the personal %s (%q)", what, lit)
		}
	}
	// The only concrete secrets present must be the ones we passed in.
	for _, want := range []string{"TESTKEY", "flow=zzz", "code: 000000", "/api/org-bundle/zzz/tok"} {
		if !strings.Contains(out, want) {
			t.Errorf("expected the supplied artifact value %q to appear\n---\n%s", want, out)
		}
	}
}

func TestDisplayLabel_taglineParenthetical(t *testing.T) {
	cases := []struct{ goatnet, tagline, want string }{
		{"zagros", "integration gateway", "ZAGROS (integration gateway)"},
		{"zagros", "", "ZAGROS"},
		{"zagros", "  ", "ZAGROS"},
		{"", "integration gateway", "the data fabric"}, // no goatnet → no parenthetical on the fallback
	}
	for _, c := range cases {
		if got := DisplayLabel(c.goatnet, c.tagline); got != c.want {
			t.Errorf("DisplayLabel(%q, %q) = %q, want %q", c.goatnet, c.tagline, got, c.want)
		}
	}
}

func TestRender_purposeBlockPresentWhenConfigured(t *testing.T) {
	cat := zagrosCatalog()
	cat.Tagline = "integration gateway"
	cat.Purpose = "ZAGROS is the integration gateway. It is not zagros-prod."
	out := Render(cat, Recipient{Name: "A", Handle: "a", Tier: "browse"}, Artifacts{})
	for _, m := range []string{
		"About this environment — ZAGROS (integration gateway):",
		"not zagros-prod",
	} {
		if !strings.Contains(out, m) {
			t.Errorf("welcome package missing %q", m)
		}
	}
	// The block leads the package: before the mesh explainer.
	if strings.Index(out, "About this environment") > strings.Index(out, "What is \"the mesh\"?") {
		t.Error("environment block should render before the mesh explainer")
	}
}

func TestRender_noPurposeNoBlock(t *testing.T) {
	out := Render(zagrosCatalog(), Recipient{Name: "A", Handle: "a", Tier: "browse"}, Artifacts{})
	if strings.Contains(out, "About this environment") {
		t.Error("environment block must be absent when no purpose is configured — never fabricated")
	}
}

func TestRender_prominentlyExplainsCanonicalHandle(t *testing.T) {
	r := Recipient{Name: "Casey", Handle: "unit-casey", Tier: "developer"}
	pkg := RenderPackage(zagrosCatalog(), r, Artifacts{})
	for presentation, body := range map[string]string{"text": pkg.Text, "html": unescaped(pkg.HTML)} {
		for _, want := range []string{"Your handle", "unit-casey", "exact identifier", "administrator", "change your account or access"} {
			if !strings.Contains(body, want) {
				t.Errorf("%s welcome missing %q", presentation, want)
			}
		}
	}
}

func TestRender_missingHandleDirectsUserToAdministrator(t *testing.T) {
	out := Render(zagrosCatalog(), Recipient{Name: "Casey", Tier: "browse"}, Artifacts{})
	for _, want := range []string{"Your handle", "No canonical handle is available", "Contact an administrator", "do not substitute your email address"} {
		if !strings.Contains(out, want) {
			t.Errorf("welcome without handle missing %q", want)
		}
	}
}

func TestRenderUpgrade_purposeBlockCarriesToUpgradePath(t *testing.T) {
	cat := zagrosCatalog()
	cat.Purpose = "ZAGROS is the integration gateway."
	out := RenderUpgrade(cat, Recipient{Name: "A", Handle: "a", Tier: "developer"},
		Upgrade{PrevGroups: []string{}, NewGroups: []string{"PLATFORM-ADMIN"}})
	if !strings.Contains(out, "About this environment — ZAGROS:") {
		t.Error("upgrade package missing the environment block")
	}
}

func TestWrapText_wrapsAtWidth(t *testing.T) {
	lines := wrapText("aaa bbb ccc ddd", 7)
	want := []string{"aaa bbb", "ccc ddd"}
	if len(lines) != len(want) {
		t.Fatalf("wrapText lines = %v, want %v", lines, want)
	}
	for i := range want {
		if lines[i] != want[i] {
			t.Errorf("line %d = %q, want %q", i, lines[i], want[i])
		}
	}
}

// The operator section is copy-pasted into a shell by a new admin, so a hole in
// it is worse than a missing section: `export GOAT_OPERATOR_HANDLE=` pasted
// as-is exports an EMPTY handle and every admin action is attributed to nobody.
// (Identities with no onboarding row render with a blank handle.)
func TestOperatorSectionNeverRendersABlankHandle(t *testing.T) {
	got := handleOrPlaceholder("")
	if got == "" {
		t.Fatal("blank handle must render a visible placeholder, never an empty string")
	}
	if got != "<your-handle>" {
		t.Errorf("handleOrPlaceholder(\"\") = %q, want a visible placeholder", got)
	}
	if handleOrPlaceholder("adm-op") != "adm-op" {
		t.Error("a real handle must pass through unchanged")
	}
	if handleOrPlaceholder("  ") == "  " {
		t.Error("whitespace-only handle must be treated as blank, not pasted as-is")
	}
}

// The portal API-key env var is PER-GOATNET. Telling an admin to export
// another goatnet's variable points them at that goatnet's portal.
func TestPortalKeyEnvVarIsPerGoatnet(t *testing.T) {
	for _, tc := range []struct{ env, want string }{
		{"zagros", "ZAGROS_PORTAL_OPERATOR_API_KEY"},
		{"zagros-prod", "ZAGROS_PROD_PORTAL_OPERATOR_API_KEY"},
		{"nova", "NOVA_PORTAL_OPERATOR_API_KEY"},
		{"nova-lan", "NOVA_LAN_PORTAL_OPERATOR_API_KEY"},
	} {
		got := strings.ToUpper(strings.ReplaceAll(tc.env, "-", "_")) + "_PORTAL_OPERATOR_API_KEY"
		if got != tc.want {
			t.Errorf("env %q → %q, want %q (hyphens must become underscores — a hyphen is not legal in a shell var name)", tc.env, got, tc.want)
		}
	}
}

// A reissued package must announce itself as a replacement BEFORE any
// instruction, or a second copy in the inbox reads as a second identity — and
// the enrollee cannot tell which copy to act on.
func TestRenderReissue_bannerPrecedesEverythingAndNamesTheMissingKey(t *testing.T) {
	out := RenderReissue(zagrosCatalog(),
		Recipient{Name: "Austin Webber", Handle: "user364x", Tier: "browse",
			Groups: []string{"OBSERVER"}},
		Artifacts{SetupKey: "KEY-REISSUE"})

	banner := "REPEAT — THIS REPLACES THE WELCOME EMAIL YOU ALREADY RECEIVED"
	if !strings.Contains(out, banner) {
		t.Fatalf("reissue package missing the repeat banner\n---\n%s", out)
	}
	if bi, ni := strings.Index(out, banner), strings.Index(out, "Austin Webber — ZAGROS"); bi > ni {
		t.Errorf("banner must precede the title line (banner at %d, title at %d)", bi, ni)
	}
	for _, m := range []string{
		"not a second account",
		"discard the earlier one",
		"one-time mesh enrollment key", // names what was missing
		"--setup-key KEY-REISSUE",      // and the corrected package carries it
		"do not need redoing",
	} {
		if !strings.Contains(out, m) {
			t.Errorf("reissue package missing %q\n---\n%s", m, out)
		}
	}
}

// The "what was missing" paragraph is specifically about the key. A reissue with
// no key (a non-mesh manifest) must not claim a key is included below.
func TestRenderReissue_withoutKeyOmitsTheKeyExplanation(t *testing.T) {
	out := RenderReissue(zagrosCatalog(),
		Recipient{Name: "No Mesh", Handle: "nomesh", Tier: "browse"}, Artifacts{})
	if !strings.Contains(out, "REPEAT — THIS REPLACES") {
		t.Error("reissue must still carry the banner without a key")
	}
	if strings.Contains(out, "it is included below instead") {
		t.Error("keyless reissue must not claim the key is included")
	}
}

// Render (the fresh path) must stay banner-free — a first package is not a repeat.
func TestRender_freshPackageHasNoRepeatBanner(t *testing.T) {
	out := Render(zagrosCatalog(), Recipient{Name: "First Timer", Handle: "first", Tier: "browse"},
		Artifacts{SetupKey: "KEY-1"})
	if strings.Contains(out, "REPEAT —") {
		t.Error("a first-time welcome package must not carry the repeat banner")
	}
}

// The whole point of the fix: a developer's portal key appears IN the email.
// Anything less sends them digging through a tarball or back to an admin.
func TestRender_portalKeyInlineForDeveloper(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Alex D", Handle: "alexd", Tier: "developer",
			Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", PortalAPIKey: "vp_abc_secret123"})

	if !strings.Contains(out, "vp_abc_secret123") {
		t.Fatalf("portal key missing from the email\n---\n%s", out)
	}
	// The circular instruction must be gone — it is what stranded people.
	if strings.Contains(out, "find it under your identity once you can reach the Portal") {
		t.Error("the circular 'find it in the Portal' instruction must not survive when a key is present")
	}
	// Rotation is what kills the key in the person's bundle; warn them off it.
	if !strings.Contains(out, "makes the key above stop working") {
		t.Error("must warn that rotating invalidates the key just delivered")
	}
}

// No key minted (no fabric leg) → say so plainly. Sending someone to hunt for a
// key that was never minted is the failure mode being removed, not preserved.
func TestRender_noPortalKeySaysSoInsteadOfSendingThemHunting(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Browse Only", Handle: "browse", Tier: "browse", Groups: []string{"OBSERVER"}},
		Artifacts{SetupKey: "K"})

	if strings.Contains(out, "find it under your identity once you can reach the Portal") {
		t.Error("keyless recipient must not be told to find a key that does not exist")
	}
	if !strings.Contains(out, "does not include a portal API key") {
		t.Errorf("keyless recipient should be told plainly\n---\n%s", out)
	}
}

// Operators get BOTH keys named: the vp_ key is what the Portal sign-in box
// takes, the op_ key is for the operator surfaces, and suppressing the first
// hands a platform admin a 401 and nothing to paste. Withholding a credential
// from its owner does not make it safer; it just routes them through an admin.
func TestRender_operatorGetsBothKeysNamed(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Op", Handle: "op", Tier: "platform-admin", Groups: []string{"DEVELOPER"},
			FabricAccess: true, Publisher: true, AdminConsole: true, AdminOperator: true},
		Artifacts{SetupKey: "K", PortalAPIKey: "vp_abc_secret123"})

	if !strings.Contains(out, "vp_abc_secret123") {
		t.Errorf("an operator's own scoped key must be handed over, not withheld\n---\n%s", out)
	}
	if !strings.Contains(out, "OPERATOR key") {
		t.Errorf("the operator key must still be explained as the second credential\n---\n%s", out)
	}
}

// --- slim path: a goatnet WITH the guide deployed ---------------------------

// The whole point of the split: credentials survive, the walkthrough does not.
// If this ever inverts, an enrollee gets a tidy email with nothing to act on.
func TestRender_slimKeepsEveryCredentialAndDropsTheWalkthrough(t *testing.T) {
	out := Render(zagrosCatalogWithGuide(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", Groups: []string{"DEVELOPER"},
			FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "SETUP-KEY-1", PortalAPIKey: "vp_abc_secret",
			RecoveryLink: "https://auth.zagros.netbird.zagros.example/recovery?flow=xyz",
			BundleURL:    "https://portal.zagros.netbird.zagros.example/api/org-bundle/abc/tok.tar.gz"})

	// Credentials — every one must still be here.
	for _, must := range []string{
		"--setup-key SETUP-KEY-1", // mesh join
		"vp_abc_secret",           // portal key
		"recovery?flow=xyz",       // login
		"/api/org-bundle/abc/tok", // one-time bundle link
	} {
		if !strings.Contains(out, must) {
			t.Errorf("slim email dropped a CREDENTIAL %q\n---\n%s", must, out)
		}
	}
	// Walkthrough — must have moved to the guide.
	for _, gone := range []string{
		"goat tutor",         // CLI step
		"first-publisher.py", // fabric walkthrough
		"panoscope.zagros",     // surfaces tour
		"SHA256SUMS",         // windows install detail
	} {
		if strings.Contains(out, gone) {
			t.Errorf("slim email still carries walkthrough content %q — it belongs on the guide", gone)
		}
	}
	// And it must say where that content went, at the reader's tier.
	if !strings.Contains(out, "guide.zagros.netbird.zagros.example/public/onboarding-guide?tier=developer") {
		t.Errorf("slim email missing the tier-correct guide link\n---\n%s", out)
	}
}

// The bundle link is one-time and expires in ~24h. When the fabric walkthrough
// moves out, the link must NOT go with it.
func TestRender_slimStillFetchesTheBundleEarly(t *testing.T) {
	out := Render(zagrosCatalogWithGuide(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", BundleURL: "https://portal.zagros.netbird.zagros.example/api/org-bundle/a/b.tar.gz"})
	if !strings.Contains(out, "the link is one-time") {
		t.Error("slim email must still push the enrollee to fetch the expiring bundle early")
	}
}

// A hand-edited ?tier= must never show someone more than their level. The link
// the email emits is normalized, and the guide degrades unknown tiers to browse.
func TestRender_slimGuideLinkNormalizesTier(t *testing.T) {
	out := Render(zagrosCatalogWithGuide(),
		Recipient{Name: "X", Handle: "x", Tier: "not-a-tier"}, Artifacts{SetupKey: "K"})
	if !strings.Contains(out, "?tier=browse") {
		t.Errorf("an unrecognized tier must resolve to browse in the guide link\n---\n%s", out)
	}
}

// Slimming must not silently shorten the email for a goatnet that has no guide
// to send people to.
func TestRender_noGuideConfiguredKeepsTheFullEmail(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K"})
	if strings.Contains(out, "EVERYTHING ELSE — the guide") {
		t.Error("no guide configured — the email must not point at one")
	}
	if !strings.Contains(out, "goat tutor") {
		t.Error("no guide configured — the CLI walkthrough must still be in the email")
	}
}

// The host-shell paragraph must not claim access the reader does not have.
// It must not say "your SSH key was added when you were onboarded" — false for
// anyone who went through candidate onboarding, because host shell is a
// SEPARATE ceremony (operator-tier-onboard.yml) that installs a pubkey only
// the person can supply.
func TestRender_hostShellDoesNotClaimAccessNotYetGranted(t *testing.T) {
	for _, cat := range []Catalog{zagrosCatalog(), {}} {
		out := Render(cat,
			Recipient{Name: "Adm", Handle: "adm-op", Tier: "platform-admin",
				AdminConsole: true, AdminOperator: true},
			Artifacts{})
		if !strings.Contains(out, "Host shell") {
			continue // catalog without the operator section
		}
		if strings.Contains(out, "key was added when you were onboarded") {
			t.Errorf("host-shell note claims a key was already installed — it is a separate ceremony:\n%s", out)
		}
		if !strings.Contains(out, "SEPARATE ceremony") {
			t.Errorf("host-shell note must say it is a separate ceremony:\n%s", out)
		}
		if !strings.Contains(out, "SSH PUBLIC key") {
			t.Errorf("host-shell note must tell the reader to send their PUBLIC key:\n%s", out)
		}
		// The guarantee the older test protects must survive the rewording.
		if !strings.Contains(out, "aren't attributed") {
			t.Errorf("no-attribution caveat lost:\n%s", out)
		}
	}
}

// Anything that expires must be called out ABOVE the steps, not left as a
// sentence inside one. A reader who skims reaches step 5 well after a 15-minute
// window has closed, and the recovery then costs an admin's time.
func TestRender_timeSensitiveItemsAreHoistedAboveTheSteps(t *testing.T) {
	out := Render(zagrosCatalogWithGuide(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", RecoveryLink: "https://auth.example/recovery?flow=abc",
			BundleURL: "https://portal.example/api/org-bundle/a/b.tar.gz"})

	banner := "TIME-SENSITIVE"
	bi := strings.Index(out, banner)
	if bi < 0 {
		t.Fatalf("no time-sensitive callout\n---\n%s", out)
	}
	if si := strings.Index(out, "STEP 1"); bi > si {
		t.Errorf("the callout must precede the steps (callout %d, STEP 1 %d)", bi, si)
	}
	for _, want := range []string{"works ONCE", "24 hours", "not ready to build", "7 DAYS"} {
		if !strings.Contains(out, want) {
			t.Errorf("time-sensitive block missing %q", want)
		}
	}
}

// It must not cry wolf: a package with nothing expiring gets no banner. Note
// what counts as "nothing" — this test used to pass a SetupKey and assert
// silence, because at a 90-day life the key was the one artifact that could
// genuinely wait. the default is 7 days, so a key IS a
// perishable artifact and the case below had to become key-free to still be
// testing what its name says.
func TestRender_noTimeSensitiveBannerWhenNothingExpires(t *testing.T) {
	out := Render(zagrosCatalogWithGuide(),
		Recipient{Name: "B", Handle: "b", Tier: "browse"}, Artifacts{})
	if strings.Contains(out, "TIME-SENSITIVE") {
		t.Error("no expiring artifact in this package — the banner must not render")
	}
}

// The converse, and the reason the gate changed: a package whose ONLY artifact
// is the mesh key must still print a deadline. Before the 7-day cut this
// rendered no banner at all, so the most perishable thing most people receive
// arrived with no stated window anywhere in the mail.
func TestRender_setupKeyAloneRaisesTheTimeSensitiveBanner(t *testing.T) {
	out := Render(zagrosCatalogWithGuide(),
		Recipient{Name: "B", Handle: "b", Tier: "browse"}, Artifacts{SetupKey: "K"})
	if !strings.Contains(out, "TIME-SENSITIVE") {
		t.Fatalf("a setup-key-only package must carry the banner\n---\n%s", out)
	}
	if !strings.Contains(out, "7 DAYS") {
		t.Error("the banner must state the 7-day mesh-key window")
	}
	// Nothing else in this package expires, so the other windows must stay out
	// of it — the point of the gate is per-artifact honesty, not a blanket warning.
	// "login link and code" is the recovery block's own phrasing — a marker that
	// is unique to it. It does not pin a lifetime, which belongs to Kratos's
	// recovery lifespan and the role default.
	for _, unwanted := range []string{"login link and code", "works ONCE"} {
		if strings.Contains(out, unwanted) {
			t.Errorf("banner claims %q for a package that carries no such artifact", unwanted)
		}
	}
}

// Mint-on-collect: with the credential deferred the email must point at the
// collection page. The failure it guards against is the pre-existing no-key
// branch, which tells the reader their access level does not include a portal
// key — flatly wrong for someone whose key is waiting to be minted, and it
// sends them to an admin who has nothing to give them.
func TestRender_deferredCredentialPointsAtTheCollectionPage(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Alex D", Handle: "alexd", Tier: "developer",
			Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", CredentialDeferred: true})

	if !strings.Contains(out, "/fabric-credential") {
		t.Fatalf("deferred email does not name the collection page\n---\n%s", out)
	}
	if strings.Contains(out, "Your access level does not include a portal API key") {
		t.Error("deferred email tells the reader they get no key — they get one, by collecting it")
	}
	if strings.Contains(out, "vp_") {
		t.Error("deferred email printed something key-shaped; nothing was minted")
	}
	// It must say WHY it is not in the email, or "mint it yourself" reads as
	// bureaucracy rather than the reason the key can be handed over at all.
	if !strings.Contains(out, "never be shown a second time") {
		t.Error("deferred email does not explain why the key waits rather than traveling in email")
	}
}

// A deferred email must not also render the bundle-download section: there is
// no bundle either, and a heading promising credentials with no link under it
// is how people conclude the email is broken.
func TestRender_deferredCredentialOmitsTheBundleSection(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "Alex D", Handle: "alexd", Tier: "developer",
			Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true},
		Artifacts{SetupKey: "K", CredentialDeferred: true})

	if strings.Contains(out, "YOUR FABRIC CREDENTIALS — fetch this first") {
		t.Errorf("deferred email renders the one-time bundle heading with no bundle\n---\n%s", out)
	}
}

// A partner-org member must be routed to the guide's partner view from the
// first email — their tier link stays primary (the partner view is an overlay,
// not a replacement), the partner pointer is additive, and no other affiliation
// kind ever gets it (blank included: never a guess).
func TestRender_partnerOrgGetsThePartnerViewPointer(t *testing.T) {
	cat := DeriveCatalog("zagros",
		"https://netbird.zagros.example",
		"https://get.zagros.netbird.zagros.example",
		"https://portal.zagros.netbird.zagros.example")
	if cat.Guide == "" {
		t.Fatal("fixture needs a derived guide URL")
	}
	base := Recipient{Name: "Ana Ionescu", Handle: "ana", Tier: "developer",
		Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true}

	partner := base
	partner.AffiliationKind = "partner"
	out := Render(cat, partner, Artifacts{SetupKey: "K"})
	for _, want := range []string{
		cat.Guide + "?role=partner",
		"foreign government or military organisation", // reworded from "partner organisation" (ADR 1148: kind=partner is entity type, not alliance standing)
		cat.Guide + "?tier=developer",                 // tier link stays primary
	} {
		if !strings.Contains(out, want) {
			t.Errorf("partner-org email missing %q", want)
		}
	}

	for _, kind := range []string{"", "organic", "vendor", "civil"} {
		r := base
		r.AffiliationKind = kind
		out := Render(cat, r, Artifacts{SetupKey: "K"})
		if strings.Contains(out, "?role=partner") {
			t.Errorf("affiliation kind %q must not get the partner pointer", kind)
		}
	}
}

// The steps are a short list of things to do; every caveat a support case
// earned lives in ONE notes section at the end, after the guide pointer
// (operator direction: "move troubleshooting notes to the bottom to
// lean out the main section"). Pins both halves: the notes come last, and
// Step 1 stays short.
func TestRender_troubleshootingNotesSitAfterTheStepsAndStep1StaysShort(t *testing.T) {
	out := Render(zagrosCatalogWithGuide(),
		Recipient{Name: "Dev", Handle: "dev", Tier: "developer", Groups: []string{"DEVELOPER"}, FabricAccess: true},
		Artifacts{SetupKey: "K", PortalAPIKey: "vp_abc_secret",
			RecoveryLink: "https://auth.example/recovery?flow=q", RecoveryCode: "123456"})

	iNotes := strings.Index(out, "IF SOMETHING DOES NOT WORK")
	iGuide := strings.Index(out, "EVERYTHING ELSE — the guide")
	if iNotes < 0 || iGuide < 0 || iNotes < iGuide {
		t.Fatalf("notes must come after the guide pointer (notes=%d guide=%d)\n---\n%s", iNotes, iGuide, out)
	}
	// The caveats moved, they did not vanish.
	for _, m := range []string{"sign up", "separate machine", "SILENTLY IGNORED", "Oracle Linux", "The code still works.", "Login page won't load?", "Do NOT ask for a rotation"} {
		if !strings.Contains(out[iNotes:], m) {
			t.Errorf("notes section missing %q", m)
		}
		if strings.Contains(out[:iNotes], m) {
			t.Errorf("%q still appears in the steps, above the notes", m)
		}
	}
	// Step 1 is the list of commands and nothing else: bound its length.
	s1 := strings.Index(out, "STEP 1 —")
	s2 := strings.Index(out, "STEP 2 —")
	if s1 < 0 || s2 < 0 || s2 < s1 {
		t.Fatalf("cannot locate STEP 1..STEP 2 (%d, %d)", s1, s2)
	}
	// 40, not 24, (GH #944): Step 1 carries TWO command
	// blocks, macOS/Linux and Windows, because a single sudo-shaped list with a
	// "Windows: drop the sudo" sentence was unfollowable on Windows. Both
	// blocks are commands; the caveats still live in the notes.
	// 52, not 40, (ADR 1175): a third device block, the
	// phone or tablet on the NetBird app, whose four steps carry no commands
	// but one management URL and one key. Its caveats are note E.
	if n := strings.Count(out[s1:s2], "\n"); n > 52 {
		t.Errorf("STEP 1 is %d lines; the caveats belong in the notes section, not the steps\n---\n%s", n, out[s1:s2])
	}
}

// TestRender_step1WindowsBlockHasNoSudo: the Windows block must be pasteable
// as-is into an administrator Command Prompt — no `sudo` anywhere in it, no
// service-install step (the installer does that), and the enrollment line
// present in BOTH shapes. Regression for GH #944: an enrollee had to strip
// sudo from six commands by trial and error.
func TestRender_step1WindowsBlockHasNoSudo(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "New", Handle: "new", Tier: "browse"},
		Artifacts{SetupKey: "K"})
	heading := "Windows — the commands have NO \"sudo\":"
	start := strings.Index(out, heading)
	end := strings.Index(out, "STEP 2 —")
	if start < 0 || end < 0 || end < start {
		t.Fatalf("Windows block / STEP 2 markers not found\n---\n%s", out)
	}
	win := out[start+len(heading) : end]
	if strings.Contains(win, "sudo") {
		t.Errorf("Windows block contains sudo:\n%s", win)
	}
	if strings.Contains(win, "service install") {
		t.Errorf("Windows block must not carry a service-install step:\n%s", win)
	}
	unix := "sudo netbird up --profile zagros --management-url https://netbird.zagros.example --setup-key K"
	if !strings.Contains(out, unix) {
		t.Errorf("macOS/Linux enrollment line missing\n---\n%s", out)
	}
	if !strings.Contains(win, "netbird up --profile zagros --management-url https://netbird.zagros.example --setup-key K") {
		t.Errorf("Windows enrollment line missing\n---\n%s", win)
	}
}

// TestRender_upgradeStep1ShowsBothEnrollShapes: the upgrade ("already done")
// branch of Step 1 renders its additional-device hint in BOTH shapes. The
// first fix for GH #944 reached only the fresh-onboard path; a Windows user
// on an access upgrade still got the sudo-only line.
func TestRender_upgradeStep1ShowsBothEnrollShapes(t *testing.T) {
	out := RenderUpgrade(zagrosCatalog(),
		Recipient{Name: "Up", Handle: "up", Tier: "publisher"},
		Upgrade{PrevGroups: []string{"browse"}, NewGroups: []string{"browse", "publisher"}})
	s1 := strings.Index(out, "(already done)")
	s2 := strings.Index(out, "STEP 2 —")
	if s1 < 0 || s2 < 0 || s2 < s1 {
		t.Fatalf("cannot locate upgrade STEP 1..STEP 2\n---\n%s", out)
	}
	step1 := out[s1:s2]
	for _, m := range []string{
		"sudo netbird up --profile zagros --management-url https://netbird.zagros.example --setup-key <key>",
		"Windows (administrator Command Prompt, no sudo):",
		"\n  netbird up --profile zagros --management-url https://netbird.zagros.example --setup-key <key>",
	} {
		if !strings.Contains(step1, m) {
			t.Errorf("upgrade step 1 missing %q\n---\n%s", m, step1)
		}
	}
}

// TestRender_upgradeStep1HTMLKeepsEachLabelWithItsCommand: the HTML part of
// the upgrade email renders each OS label as its own paragraph directly above
// its own command block. doc.cont folds a label into the preceding paragraph
// and lets the two commands merge into one <pre>, which left a Windows reader
// with two unlabelled commands; the text part never showed it (GH #944).
func TestRender_upgradeStep1HTMLKeepsEachLabelWithItsCommand(t *testing.T) {
	pkg := RenderUpgradePackage(zagrosCatalog(),
		Recipient{Name: "Up", Handle: "up", Tier: "publisher"},
		Upgrade{PrevGroups: []string{"browse"}, NewGroups: []string{"browse", "publisher"}})
	s1 := strings.Index(pkg.HTML, "(already done)")
	s2 := strings.Index(pkg.HTML, "STEP 2 —")
	if s1 < 0 || s2 < 0 || s2 < s1 {
		t.Fatalf("cannot locate upgrade STEP 1..STEP 2 in HTML\n---\n%s", pkg.HTML)
	}
	step1 := pkg.HTML[s1:s2]
	if n := strings.Count(step1, "<pre "); n != 2 {
		t.Errorf("want one <pre> per OS, got %d\n---\n%s", n, step1)
	}
	for _, label := range []string{
		"macOS / Linux (Terminal):</p>",
		"Windows (administrator Command Prompt, no sudo):</p>",
	} {
		i := strings.Index(step1, label)
		if i < 0 {
			t.Errorf("label %q is not its own paragraph\n---\n%s", label, step1)
			continue
		}
		if !strings.HasPrefix(strings.TrimSpace(step1[i+len(label):]), "<pre ") {
			t.Errorf("label %q is not directly followed by its command block\n---\n%s", label, step1)
		}
	}
	w := strings.Index(step1, "no sudo):</p>")
	if w < 0 || !strings.Contains(step1[w:], ">netbird up --profile zagros --management-url") || strings.Contains(step1[w:], ">sudo netbird up") {
		t.Errorf("Windows label must sit above the unprefixed command\n---\n%s", step1)
	}
}

// TestRender_step1WindowsSaysNewWindowAfterInstall: #944 asked for "open a NEW
// terminal as administrator" AFTER installing — a prompt opened before the
// installer ran cannot find netbird. Ordering alone does not say it.
func TestRender_step1WindowsSaysNewWindowAfterInstall(t *testing.T) {
	out := Render(zagrosCatalog(),
		Recipient{Name: "New", Handle: "new", Tier: "browse"},
		Artifacts{SetupKey: "K"})
	for _, m := range []string{
		"AFTER the installer finishes, open a NEW Command Prompt AS ADMINISTRATOR",
		"A window opened before the install cannot find",
	} {
		if !strings.Contains(out, m) {
			t.Errorf("Windows block missing %q\n---\n%s", m, out)
		}
	}
}

func TestWindowsEnrollCommand(t *testing.T) {
	got := WindowsEnrollCommand("https://netbird.zagros.example", "KEY-123", false, "zagros")
	want := "netbird up --profile zagros --management-url https://netbird.zagros.example --setup-key KEY-123 --enable-rosenpass=false --rosenpass-permissive=false"
	if got != want {
		t.Errorf("WindowsEnrollCommand = %q, want %q", got, want)
	}
	if strings.HasPrefix(got, "sudo") {
		t.Errorf("Windows command must not start with sudo: %q", got)
	}
	// Flag parity with the POSIX form: same command minus the sudo prefix.
	if "sudo "+got != EnrollCommand("https://netbird.zagros.example", "KEY-123", false, "zagros") {
		t.Errorf("Windows and POSIX forms diverged beyond the sudo prefix: %q", got)
	}
	named := WindowsEnrollCommandNamed("https://netbird.example.net", "KEY-123", true, "peer-jane-doe-zagros-2026-08-15", "zagros")
	if !strings.Contains(named, `--hostname "peer-jane-doe-zagros-2026-08-15-%COMPUTERNAME%"`) {
		t.Errorf("Windows named form must use %%COMPUTERNAME%%: %q", named)
	}
	if strings.Contains(named, "$(hostname") {
		t.Errorf("POSIX $(hostname -s) leaked into the Windows form: %q", named)
	}
	if !strings.Contains(named, "--enable-rosenpass --rosenpass-permissive") {
		t.Errorf("rosenpass-on must append both flags: %q", named)
	}
}

// TestRender_staleBinaryUpdateSurvivesRootOwnedInstall: STEP 4 installs goat
// into /usr/local/bin, a root-owned folder, so the "goat ca is unrecognized →
// update first" recovery has to work there. A plain `goat update` fails with
// Permission denied, and a bare `sudo goat update` fails too — sudo resets the
// environment, drops GOAT_GET_ENDPOINT, and goat exits "cannot resolve this
// context's download surface". `sudo env VAR=… <full path> update` is the form
// that carries the variable through. Both renderings (new user, returning
// user) must give it, and must keep the unprivileged form for user-owned
// installs and Windows.
func TestRender_staleBinaryUpdateSurvivesRootOwnedInstall(t *testing.T) {
	const get = "https://get.zagros.netbird.zagros.example"
	r := Recipient{Name: "Dev", Handle: "dev", Tier: "developer",
		Groups: []string{"DEVELOPER"}, FabricAccess: true, Publisher: true}
	for name, out := range map[string]string{
		"new user":       Render(zagrosCatalog(), r, Artifacts{SetupKey: "K"}),
		"returning user": RenderUpgrade(zagrosCatalog(), r, Upgrade{PrevGroups: []string{"OBSERVER"}, NewGroups: []string{"DEVELOPER"}}),
	} {
		for _, m := range []string{
			"sudo env GOAT_GET_ENDPOINT=" + get + " /usr/local/bin/goat update",
			"sudo drops it otherwise",
			"GOAT_GET_ENDPOINT=" + get + " goat update", // unprivileged form kept
			`set "GOAT_GET_ENDPOINT=` + get + `" && goat update`,
		} {
			if !strings.Contains(out, m) {
				t.Errorf("%s: update recovery missing %q", name, m)
			}
		}
		if strings.Contains(out, "sudo goat update") {
			t.Errorf("%s: a bare `sudo goat update` drops GOAT_GET_ENDPOINT and cannot resolve the surface", name)
		}
	}
}

// Blank collab and repo URLs render without an address, never a stale default.
func TestRender_blankCollabAndRepoURLs(t *testing.T) {
	cat := zagrosCatalog()
	cat.CollabURL = ""
	cat.RepoURL = ""
	out := Render(cat,
		Recipient{Name: "Adm", Handle: "adm-op", Tier: "platform-admin", AdminConsole: true, AdminOperator: true},
		Artifacts{SetupKey: "K", CollabAccount: "adm", CollabSetupLink: "https://idm.collab.goat.example/ui/reset?token=ABC"})
	for _, want := range []string{
		"base, file sharing, and chat). It has its OWN sign-in,",
		"git clone <platform repository URL from your operator>",
		"admin-operator-guide.md in the platform repository",
		"ZAGROS_NB_API_TOKEN",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q", want)
		}
	}
	if strings.Contains(out, "github.com") || strings.Contains(out, " at https://collab") {
		t.Error("blank URLs must not render an address")
	}
}

func TestRender_adminGuideURLWinsOverRepoURL(t *testing.T) {
	cat := zagrosCatalog()
	cat.RepoURL = "https://github.com/example-org/goatnet-config"
	cat.AdminGuideURL = "https://github.com/example-org/goat/blob/main/docs/operations/admin-operator-guide.md"
	out := Render(cat,
		Recipient{Name: "Adm", Handle: "adm-op", Tier: "platform-admin", AdminConsole: true, AdminOperator: true},
		Artifacts{SetupKey: "K"})
	if !strings.Contains(out, "Full guide: "+cat.AdminGuideURL) {
		t.Error("the guide link must be AdminGuideURL")
	}
	if strings.Contains(out, "goatnet-config/blob/main/docs/operations/admin-operator-guide.md") {
		t.Error("the guide link must not be built from RepoURL when AdminGuideURL is set")
	}
	if !strings.Contains(out, "git clone https://github.com/example-org/goatnet-config.git") {
		t.Error("the clone line still comes from RepoURL")
	}
}
