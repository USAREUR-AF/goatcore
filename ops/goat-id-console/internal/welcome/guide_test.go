package welcome

import (
	"html"
	"regexp"
	"strings"
	"testing"
)

// unescaped renders HTML entities back to text so assertions can be written in
// prose. Without it a stray apostrophe or quote in content turns a content test
// into a punctuation test.
func unescaped(s string) string { return html.UnescapeString(s) }

func guideCat() Catalog {
	c := DeriveCatalog("zagros", "https://netbird.zagros.example",
		"https://get.zagros.netbird.zagros.example",
		"https://portal.zagros.netbird.zagros.example")
	c.Tagline = "integration gateway"
	return c
}

func renderGuide(t *testing.T, role, topic, base string) string {
	t.Helper()
	out, err := RenderGuidePage(BuildGuidePage(guideCat(), role, topic, base))
	if err != nil {
		t.Fatalf("render %s/%s: %v", role, topic, err)
	}
	return unescaped(out)
}

// The page is unauthenticated, so the invariant that makes it safe is that it
// renders no credential VALUES. Matches on the SHAPE of each secret — naming a
// credential is necessary, carrying one is the defect.
func TestGuide_carriesNoCredentialValues(t *testing.T) {
	shapes := map[string]*regexp.Regexp{
		"portal api key": regexp.MustCompile(`vp_[A-Za-z0-9]{12,}`),
		"setup key":      regexp.MustCompile(`[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}`),
		"bundle token":   regexp.MustCompile(`/api/org-bundle/[A-Za-z0-9]+/[A-Za-z0-9]{8,}`),
		"recovery link":  regexp.MustCompile(`recovery\?flow=[A-Za-z0-9-]{8,}`),
	}
	for _, r := range Roles {
		pages := []string{renderGuide(t, r.Key, "", "")}
		for _, tk := range r.Topics {
			pages = append(pages, renderGuide(t, r.Key, tk, ""))
		}
		for _, out := range pages {
			for name, re := range shapes {
				if m := re.FindString(out); m != "" {
					t.Errorf("%s leaked a %s (%q)", r.Key, name, m)
				}
			}
		}
	}
}

func TestGuide_explainsWhereHandleLivesAndWhatItIsNot(t *testing.T) {
	out := renderGuide(t, "developer", "credentials", "")
	for _, want := range []string{
		"Your handle is an identifier, not a credential",
		"Your identity page",
		"not your email address, display name, organisation identifier, password",
		"Never guess it",
		"https://admin.auth.zagros.netbird.zagros.example/identity",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("credentials guide missing %q", want)
		}
	}
	for _, role := range []string{"browse", "developer"} {
		idx := renderGuide(t, role, "", "")
		if !strings.Contains(idx, Topics["credentials"].Title) {
			t.Errorf("%s guide index does not expose the credentials topic", role)
		}
	}
}

// The whole point of the redesign: each role gets its OWN process. iam-admin and
// platform-admin rendering identically is the bug this replaced.
func TestGuide_everyRoleHasADistinctPath(t *testing.T) {
	seen := map[string]string{}
	for _, r := range Roles {
		out := renderGuide(t, r.Key, "", "")
		if !strings.Contains(out, r.OneLine) {
			t.Errorf("%s index missing its own one-liner", r.Key)
		}
		// Resolve the per-goatnet steps against the SAME catalog this rendered
		// with — the credential step's title differs by flow, so comparing to
		// the raw package data would assert a title this goatnet never shows.
		for _, step := range withCredentialFlow(r, guideCat().MintOnCollect).Path {
			if !strings.Contains(out, step.Title) {
				t.Errorf("%s index missing step %q", r.Key, step.Title)
			}
		}
		if prev, dup := seen[out]; dup {
			t.Errorf("%s renders IDENTICALLY to %s — roles must differ", r.Key, prev)
		}
		seen[out] = r.Key
	}
	if len(Roles) != 6 {
		t.Errorf("expected 6 documented roles including network-admin and partner, got %d", len(Roles))
	}
}

// Partner is affiliation, not a tier — the view must say so itself, and its
// data-sharing page must be honest about what is enforced today: lanes are
// granted, the read partition is applied per-organization by operators, and
// an uncarved goatnet is one room.
func TestGuide_partnerViewSeparatesAffiliationFromTier(t *testing.T) {
	idx := renderGuide(t, "partner", "", "")
	for _, want := range []string{
		"Partner is who you are, not a permission level",
		"Read the path for your level too",
	} {
		if !strings.Contains(idx, want) {
			t.Errorf("partner index missing %q", want)
		}
	}
	deep := renderGuide(t, "partner", "partner-data", "")
	for _, want := range []string{
		"Your namespace is yours alone",
		"release lane",
		"Publishing into a lane is a grant, not a default",
		"it is not something being a partner switches on",
		"treat the goatnet you are on as one room",
		"BEFORE publishing",
	} {
		if !strings.Contains(deep, want) {
			t.Errorf("partner-data topic missing %q", want)
		}
	}
	// The honesty statement must never be softened into an automatic guarantee.
	for _, never := range []string{
		"your data is only visible to",
		"automatically partitioned",
	} {
		if strings.Contains(deep, never) {
			t.Errorf("partner-data topic must not promise automatic separation: found %q", never)
		}
	}
}

// The map surface exists only where a goatnet carries a geographic feed, so its
// link is CONFIGURED, never derived — a derived Catalog must leave it blank and
// the surfaces page must then render the honest "not configured" affordance.
func TestGuide_mapSurfaceIsConfiguredNotDerived(t *testing.T) {
	if guideCat().Map != "" {
		t.Fatal("DeriveCatalog must not invent a map URL — the map is configured per goatnet")
	}
	surf := renderGuide(t, "browse", "surfaces", "")
	if !strings.Contains(surf, "not configured on this goatnet") {
		t.Error("an unconfigured map must render the not-configured affordance, not vanish")
	}
	cat := guideCat()
	cat.Map = "https://map.zagros-prod.examplebird.zagros-prod.example"
	out, err := RenderGuidePage(BuildGuidePage(cat, "browse", "surfaces", ""))
	if err != nil {
		t.Fatalf("render with map configured: %v", err)
	}
	if !strings.Contains(unescaped(out), cat.Map) {
		t.Error("a configured map URL must render as a live link")
	}
}

// The admin lenses added to the surfaces page must derive from the portal base
// like their siblings, and the CLI page must carry the three lenses.
func TestGuide_adminLensesAndCliLensesAreSurfaced(t *testing.T) {
	c := guideCat()
	if c.Microscope != "https://microscope.zagros.netbird.zagros.example" {
		t.Errorf("microscope should derive from the portal base, got %q", c.Microscope)
	}
	if c.Grafana != "https://grafana.zagros.netbird.zagros.example" {
		t.Errorf("grafana should derive from the portal base, got %q", c.Grafana)
	}
	surf := renderGuide(t, "iam-admin", "surfaces", "")
	for _, want := range []string{"Microscope", "every look is itself recorded", "Grafana", "Identity admin console"} {
		if !strings.Contains(surf, want) {
			t.Errorf("surfaces page missing %q", want)
		}
	}
	cli := renderGuide(t, "developer", "cli", "")
	for _, want := range []string{"goat top", "goat who", "goat flow", "no observer could answer"} {
		if !strings.Contains(cli, want) {
			t.Errorf("cli page missing lens content %q", want)
		}
	}
}

// Network admin is its own view, not an overlay, and carries its own rules.
func TestGuide_networkAdminIsItsOwnViewWithItsOwnRules(t *testing.T) {
	out := renderGuide(t, "network-admin", "network", "")
	// The class restrictions must still be documented — just clearly as the
	// DEVICE's rules rather than the reader's.
	for _, want := range []string{"no lateral reach", "no fabric ports"} {
		if !strings.Contains(out, want) {
			t.Errorf("network topic missing %q", want)
		}
	}
	// Scope is a SEPARATE dimension from the role — a network admin may hold one
	// site, several, or the whole fleet. Asserting "site-scoped" as though it
	// were part of the role would be wrong the first time somebody spans two.
	for _, want := range []string{"can be several", "fleet-wide", "never changes the CLASS"} {
		if !strings.Contains(out, want) {
			t.Errorf("network topic should treat scope as its own question: missing %q", want)
		}
	}
	if strings.Contains(out, "That is its whole extent") {
		t.Error("scope must not be described as inherently single-site")
	}
	// The page must separate the PERSON from the DEVICE CLASS. An earlier version
	// listed the class's restrictions ("no lateral reach, never peer-to-peer") on
	// a page addressed to "you", so it read as limits on the admin. For someone
	// new to all of this that is actively misleading.
	for _, want := range []string{
		"you and the gear are not the same subject",
		"you are reading a rule about a switch",
		"the device's leash, not yours",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("network topic must separate the person from the device class: missing %q", want)
		}
	}
	// And it must say what the role is actually built from, since that is the
	// part readers miss: it is the identity-admin tier plus a site group.
	for _, want := range []string{"IDENTITY-ADMIN tier", "everything an identity admin does, you do"} {
		if !strings.Contains(out, want) {
			t.Errorf("network topic must state the role is a superset of identity admin: missing %q", want)
		}
	}
	// It must not be reachable only via another role's tier spelling.
	if RoleByKey("network_admin").Key != "network-admin" {
		t.Error("network_admin spelling should resolve to the network-admin view")
	}
}

// Time-sensitive things must render as their own marked element, never as one
// more sentence in a paragraph a reader skims past.
func TestGuide_deadlinesRenderAsTheirOwnElement(t *testing.T) {
	out := renderGuide(t, "developer", "", "")
	if !strings.Contains(out, `class="clock"`) {
		t.Fatal("developer path must mark its time-sensitive steps")
	}
	if !strings.Contains(out, "Time-sensitive") {
		t.Error("the marked element must be labeled")
	}
	if !strings.Contains(out, "works only ONCE") {
		t.Error("the bundle window must state that it is single-use")
	}
	// A deadline without a recovery route reads as a locked door.
	if !strings.Contains(out, "If it has already lapsed") {
		t.Error("every deadline must say what to do when it has passed")
	}
}

// Repo links are composed from a configurable base because repositories get
// renamed and move orgs. With no base, names still render, with an ask note.
func TestGuide_repoCatalogueSurvivesRenamesAndOrgMoves(t *testing.T) {
	withBase := renderGuide(t, "developer", "repos", "https://github.com/example-org")
	if !strings.Contains(withBase, "https://github.com/example-org/hello-galaxy-zenoh-patterns") {
		t.Error("a configured base should compose real links")
	}
	noBase := renderGuide(t, "developer", "repos", "")
	if strings.Contains(noBase, "https://github.com/") {
		t.Error("with no base configured the guide must not invent a host")
	}
	if !strings.Contains(noBase, "hello-galaxy-zenoh-patterns") {
		t.Error("the slug must still render so a reader can ask for it by name")
	}
	for _, want := range []string{"ask an admin", "renamed or moved between organisations"} { //nolint:misspell // keep "organisations" as spelled in the original copy
		if !strings.Contains(noBase, want) {
			t.Errorf("repos topic missing access/rename guidance %q", want)
		}
	}
}

// The series is community-wide, so a role's view ORDERS the catalog — it
// must never truncate it. The old per-role filter silently dropped whatever it
// did not pick, and a reader on the identity-admin view concluded the fabric
// usage patterns and the pod standup repos did not exist.
func TestGuide_reposPageShowsWholeCatalogueForEveryRole(t *testing.T) {
	for _, role := range Roles {
		out := renderGuide(t, role.Key, "repos", "")
		for _, r := range Catalogue { //nolint:misspell // keep "Catalogue" as the exported name used in docs
			if !strings.Contains(out, r.Slug) {
				t.Errorf("%s repos page is missing %s", role.Key, r.Slug)
			}
		}
		// The role's own picks come first; the remainder sits under its own
		// heading rather than being interleaved as if equally targeted.
		if len(OtherReposFor(RoleByKey(role.Key))) > 0 &&
			!strings.Contains(out, "The rest of the series") {
			t.Errorf("%s repos page does not separate the rest of the series", role.Key)
		}
	}
}

// The gotchas topic is the one that keeps costing people days.
func TestGuide_gotchasCarryTheRotateAndOwnershipTraps(t *testing.T) {
	out := renderGuide(t, "platform-admin", "gotchas", "")
	for _, want := range []string{
		"re-issues their certificate in place", // rotate
		"copy inside their own bundle",         // why it hurts
		"what revocation acts on",              // the linkage trap
		"withheld send before chasing the mail path",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("gotchas missing %q", want)
		}
	}
}

// An unknown role must degrade to the least-privileged view, never error.
func TestGuide_unknownRoleDegradesToBrowse(t *testing.T) {
	for _, in := range []string{"", "wizard", "  PLATFORM_ADMIN "} {
		got := RoleByKey(in)
		if in == "  PLATFORM_ADMIN " {
			if got.Key != "platform-admin" {
				t.Errorf("%q should normalize to platform-admin, got %s", in, got.Key)
			}
			continue
		}
		if got.Key != "browse" {
			t.Errorf("%q should degrade to browse, got %s", in, got.Key)
		}
	}
}

// Relay health has a restricted door AND a graded document behind it. Naming
// only one of the two advertises either a door that will not open or a page
// that will not show what was promised.
func TestGuide_relayHealthIsIntroducedDifferentially(t *testing.T) {
	// The roles who diagnose transport get it as part of their own path.
	for _, role := range []string{"platform-admin", "network-admin"} {
		idx := renderGuide(t, role, "", "")
		if !strings.Contains(idx, "network-health") {
			t.Errorf("%s index should link the network-health topic", role)
		}
		deep := renderGuide(t, role, "network-health", "")
		for _, want := range []string{"Relays are NOT mesh peers", "single unhappy relay"} {
			if !strings.Contains(deep, want) {
				t.Errorf("%s network-health missing %q", role, want)
			}
		}
	}
	// Everyone else is pointed at the equivalent question they CAN answer,
	// and told plainly why the dashboard will refuse them.
	// Relay health has TWO independent layers and describing only one misleads.
	// The DOOR (a viewer list) decides admission; the GRADING decides what an
	// admitted viewer receives. Saying only "it is gated" sends people away from
	// detail they could have had; saying only "you get a reduced view" promises
	// non-admins a page that will not let them in.
	surf := renderGuide(t, "browse", "surfaces", "")
	for _, want := range []string{"DOOR", "GRADING", "reduced", "service status answers"} {
		if !strings.Contains(surf, want) {
			t.Errorf("surfaces must describe both relay-health layers: missing %q", want)
		}
	}
	// A non-admin reading this must be pointed at the surface that does serve
	// them, not merely told what they cannot have.
	if !strings.Contains(surf, "ask an admin to add you") {
		t.Error("a non-admin needs the recourse, not just the refusal")
	}
	// And it must not appear in a browse reader's own path as a thing to do.
	idx := renderGuide(t, "browse", "", "")
	if strings.Contains(idx, "network-health") {
		t.Error("browse path should not route someone to an admin-only diagnostic")
	}
}

// A wall of equally-weighted cards tells a newcomer nothing about where to
// start. Every branch point — topic cards, the topic page itself, and the repo
// catalog one level down — carries the same triage badges.
func TestGuide_branchPointsCarryTriageBadges(t *testing.T) {
	idx := renderGuide(t, "platform-admin", "", "")
	for _, want := range []string{"Essential", "min read", "min to do it"} {
		if !strings.Contains(idx, want) {
			t.Errorf("topic cards missing badge %q", want)
		}
	}
	// The same shape one level down, where it branches again.
	repos := renderGuide(t, "platform-admin", "repos", "")
	for _, want := range []string{"min read", "min to run it"} {
		if !strings.Contains(repos, want) {
			t.Errorf("repo catalog missing badge %q", want)
		}
	}
	// And on the topic page a reader lands on.
	deep := renderGuide(t, "platform-admin", "operator", "")
	if !strings.Contains(deep, "min read") {
		t.Error("a topic page should carry its own badges")
	}
}

// "Essential" only means something if some things are not. If everything is
// essential the badge stops carrying information.
func TestGuide_essentialIsNotEverything(t *testing.T) {
	var ess, opt int
	for _, tp := range Topics {
		if tp.Essential {
			ess++
		} else {
			opt++
		}
	}
	if opt == 0 {
		t.Error("every topic is marked essential — the badge then means nothing")
	}
	if ess == 0 {
		t.Error("no topic is marked essential — a reader has no starting point")
	}
	// A topic claiming time-to-do must actually have something to do.
	for k, tp := range Topics {
		if tp.DoMin > 0 && len(tp.Sections) == 0 {
			t.Errorf("%s claims %d minutes of doing but has no content", k, tp.DoMin)
		}
	}
}

// The role picker must announce itself. "Browse" reads as a verb next to nouns,
// so an unlabelled row of links does not obviously say "pick your role".
func TestGuide_rolePickerIsLabelled(t *testing.T) {
	out := renderGuide(t, "developer", "", "")
	if !strings.Contains(out, "Choose the role that matches your work") {
		t.Error("the role selector needs a label saying what it is")
	}
	if !strings.Contains(out, "Every view is readable by anyone") {
		t.Error("the picker should say that other views are open, not restricted")
	}
}

// A curriculum that does not say what it costs gets started at the wrong moment
// and abandoned halfway. The path states a total AND times each step, so someone
// can take one step in a gap instead of clearing an afternoon.
func TestGuide_pathStatesItsTotalAndPerStepCost(t *testing.T) {
	for _, r := range Roles {
		if r.TotalMins() == 0 {
			t.Errorf("%s path has no time estimate at all", r.Key)
		}
		out := renderGuide(t, r.Key, "", "")
		if !strings.Contains(out, "steps,") || !strings.Contains(out, "end to end") {
			t.Errorf("%s index does not state its total", r.Key)
		}
		if !strings.Contains(out, HumanMins(r.TotalMins())) {
			t.Errorf("%s total not rendered as %q", r.Key, HumanMins(r.TotalMins()))
		}
		for _, st := range r.Path {
			if st.Mins == 0 {
				t.Errorf("%s step %q carries no estimate", r.Key, st.Title)
			}
		}
	}
}

// Deep dives are NOT folded into the path total — each advertises its own cost,
// and rolling optional reading into the headline would overstate the ask.
func TestGuide_totalCountsOnlyThePath(t *testing.T) {
	r := RoleByKey("platform-admin")
	sum := 0
	for _, s := range r.Path {
		sum += s.Mins
	}
	if r.TotalMins() != sum {
		t.Errorf("total %d should equal the sum of path steps %d", r.TotalMins(), sum)
	}
}

// The catalog badges a CONCERN, not an importance ranking: these are
// best-practice references for different problems, and which matters depends on
// what you are building today.
func TestGuide_catalogBadgesConcernNotImportance(t *testing.T) {
	out := renderGuide(t, "platform-admin", "repos", "")
	// Sampled categories, not the whole set — "Getting started" used to be here
	// and went with the archived on-ramp repo it belonged to.
	for _, want := range []string{"Data fabric", "Onboarding", "Standing up a pod", "Timekeeping"} {
		if !strings.Contains(out, want) {
			t.Errorf("catalog missing concern badge %q", want)
		}
	}
	for _, unwanted := range []string{"Nice to have", "Essential</span>"} {
		if strings.Contains(out, unwanted) {
			t.Errorf("catalog must not rank references by importance (found %q)", unwanted)
		}
	}
	for _, r := range Catalogue { //nolint:misspell // keep "Catalogue" as the exported name used in docs
		if r.Category == "" {
			t.Errorf("%s has no concern category", r.Slug)
		}
	}
}

// A page shown to everyone that contains admin-only commands must say so per
// section. Without it a reader has to work out whether a warning is even about
// them, and most will either assume it is not or worry that it is.
func TestGuide_mixedAudienceSectionsSayWhoTheyApplyTo(t *testing.T) {
	tp := Topics["gotchas"]
	for _, sec := range tp.Sections {
		if sec.For == "" {
			t.Errorf("gotchas section %q does not say who it applies to", sec.Heading)
		}
	}
	out := renderGuide(t, "browse", "gotchas", "")
	// Audience rides on the heading as a chip, not a sentence beneath it.
	if !strings.Contains(out, `class="for"`) {
		t.Error("the rendered page must show per-section applicability as a chip")
	}
	if !strings.Contains(out, "treat those as background if you are not an admin") {
		t.Error("non-admins need to know the admin entries are not instructions for them")
	}
	// The scenario and the fix must be distinct surfaces a reader can find
	// without reading the prose between them.
	for _, want := range []string{`class="scenario"`, `class="instead"`, "When this happens", "Do this"} {
		if !strings.Contains(out, want) {
			t.Errorf("traps must render as distinct parts: missing %q", want)
		}
	}
}

// Each trap must open with the SITUATION that produces it. A warning with no
// scenario leaves the reader unable to tell whether they are ever in it.
func TestGuide_everyTrapOpensWithItsSituation(t *testing.T) {
	for _, sec := range Topics["gotchas"].Sections {
		if len(sec.Paras) == 0 {
			t.Errorf("gotchas section %q has no body", sec.Heading)
			continue
		}
		if strings.TrimSpace(sec.Scenario) == "" {
			t.Errorf("gotchas section %q has no Scenario — a warning with no situation leaves "+
				"the reader unable to tell whether it is ever about them", sec.Heading)
		}
		if strings.TrimSpace(sec.Instead) == "" {
			t.Errorf("gotchas section %q has no Instead — naming a trap without the correct "+
				"action is half a page", sec.Heading)
		}
		// The dimensions must be STRUCTURE, not shouted prose inside a paragraph.
		for _, para := range sec.Paras {
			for _, banned := range []string{"THE SITUATION:", "WHAT TO DO INSTEAD:", "WHY NOT:", "CHECK THIS FIRST:"} {
				if strings.Contains(para, banned) {
					t.Errorf("gotchas section %q labels %q in prose — that dimension has its own "+
						"field and its own visual treatment", sec.Heading, banned)
				}
			}
		}
	}
}

func TestHumanMins(t *testing.T) {
	for in, want := range map[int]string{0: "", 17: "about 17 minutes", 60: "about an hour",
		120: "about 2 hours", 100: "about 1 h 40 min"} {
		if got := HumanMins(in); got != want {
			t.Errorf("HumanMins(%d) = %q, want %q", in, got, want)
		}
	}
}

// Every role's path must STAND ALONE. Telling a reader to go read another
// role's path is homework, not onboarding: it breaks the sequence, drops them
// into content written for somebody else, and leaves them to work out which
// parts apply. Where roles overlap, the overlapping work is restated in the
// reader's own terms at the point in THEIR order where it matters.
func TestGuide_noPathDefersToAnotherRolesPath(t *testing.T) {
	for _, r := range Roles {
		for _, st := range r.Path {
			body := strings.ToLower(st.Title + " " + st.Body)
			for _, other := range Roles {
				if other.Key == r.Key {
					continue
				}
				// "read the <other> path too" and similar hand-offs.
				if strings.Contains(body, "read the "+other.Key+" path") ||
					strings.Contains(body, strings.ToLower(other.Title)+" path too") {
					t.Errorf("%s step %q defers to the %s path — restate the work in this role's own terms",
						r.Key, st.Title, other.Key)
				}
			}
			if strings.Contains(body, "is a superset") {
				t.Errorf("%s step %q describes itself as a superset instead of documenting the work",
					r.Key, st.Title)
			}
		}
	}
}

// A path that stands alone must actually cover its role's own work. Platform
// admin administers identities and runs hosts, so both must appear in ITS path.
func TestGuide_platformAdminPathCoversItsOwnWork(t *testing.T) {
	out := renderGuide(t, "platform-admin", "", "")
	for _, want := range []string{
		"Get your own access working",      // it starts where the reader starts
		"Send your SSH public key",         // host access
		"Bring people on",                  // identity work, in their terms
		"Learn the credential map",         // what they hand out
		"Fix access by changing the level", // the habit that matters
		"Deploy by the ledger",             // running the goatnet
	} {
		if !strings.Contains(out, want) {
			t.Errorf("platform-admin path missing its own work: %q", want)
		}
	}
}

// Badges must be able to WRAP inside their card. An earlier version set
// white-space:nowrap on every badge, so a long audience label ran out of its
// grid cell and got clipped mid-word. Only the short numeric badges may refuse
// to wrap, and flex/grid children need min-width:0 or a long child forces
// overflow regardless of wrapping.
func TestGuide_badgesCanWrapInsideTheirCard(t *testing.T) {
	out := renderGuide(t, "platform-admin", "", "")
	if strings.Contains(out, ".b{") && !strings.Contains(out, "white-space:normal") {
		t.Error("badges must wrap by default, or a long label overflows its card")
	}
	if !strings.Contains(out, "overflow-wrap:anywhere") {
		t.Error("badges need overflow-wrap so an unbroken label cannot escape the card")
	}
	if !strings.Contains(out, "min-width:0") {
		t.Error("flex/grid children need min-width:0 or a long child forces overflow")
	}
	// The numeric badges are short and read badly when broken.
	if !strings.Contains(out, ".b.time{font-variant-numeric:tabular-nums;white-space:nowrap}") {
		t.Error("time badges should keep nowrap")
	}
}

// A badge is meant to be scanned. Long prose in one defeats the purpose and was
// what pushed labels out of their cards in the first place.
func TestGuide_audienceLabelsStayShortEnoughToScan(t *testing.T) {
	const max = 34
	for key, tp := range Topics {
		if len(tp.Audience) > max {
			t.Errorf("%s audience label is %d chars (max %d): %q — shorten it or move the nuance into the standfirst",
				key, len(tp.Audience), max, tp.Audience)
		}
	}
}

// The guide is MESH-HOSTED. Its reason to exist is that it holds working links
// to the surfaces it describes. Topics as static data can silently lose every URL
// and still name Panoscope, the get page and the relay dashboard.
//
// So: if a section names a surface, it must carry a resolvable link to it.
func TestGuide_sectionsThatNameASurfaceLinkIt(t *testing.T) {
	// surface phrase -> the catalog key that must be linked
	must := map[string]string{
		"get page":               "get",
		"relay-health dashboard": "relayhealth",
		"service-status page":    "isitup",
	}
	for key, tp := range Topics {
		for _, sec := range tp.Sections {
			body := strings.ToLower(sec.Heading + " " + sec.Scenario + " " + strings.Join(sec.Paras, " "))
			for phrase, wantKey := range must {
				if !strings.Contains(body, phrase) {
					continue
				}
				found := false
				for _, l := range sec.Links {
					if l.Key == wantKey {
						found = true
					}
				}
				if !found {
					t.Errorf("%s/%q names %q but carries no %q link — the guide exists to be clickable",
						key, sec.Heading, phrase, wantKey)
				}
			}
		}
	}
}

// Every link key must resolve against a fully-configured catalog. A typo in a
// key would silently render as "not configured", which reads like a deployment
// gap rather than the bug it is.
func TestGuide_everyLinkKeyIsRealAndResolves(t *testing.T) {
	c := guideCat()
	c.RelayHealth = "https://relay-health.zagros.netbird.zagros.example"
	c.Map = "https://map.zagros-prod.examplebird.zagros-prod.example"
	for key, tp := range Topics {
		for _, sec := range tp.Sections {
			for _, l := range sec.Links {
				if l.URL != "" {
					if l.Key != "" || !strings.HasPrefix(l.URL, "https://") {
						t.Errorf("%s/%q link %q: an outside link carries an https URL and no key", key, sec.Heading, l.Label)
					}
					continue
				}
				if c.ResolveLink(l.Key) == "" {
					t.Errorf("%s/%q link %q uses key %q which resolves to nothing — typo, or a "+
						"catalog field that does not exist", key, sec.Heading, l.Label, l.Key)
				}
			}
		}
	}
}

// The surfaces topic is the page a reader opens to GET somewhere. It must link,
// not merely list.
func TestGuide_surfacesTopicActuallyLinks(t *testing.T) {
	out := renderGuide(t, "browse", "surfaces", "")
	for _, want := range []string{
		"portal.zagros.netbird.zagros.example",
		"panoscope.zagros.netbird.zagros.example",
		"isitup.zagros.netbird.zagros.example",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("surfaces topic does not link %q — it lists surfaces without a way to reach them", want)
		}
	}
}

// The SSH-key pages are DERIVED from the admin-console base rather than
// configured, so a goatnet cannot have the console without them. This locks
// three things a restructure has already broken once: that the person-facing
// topic links the submit page, that the admin topic links the queue, and that
// both resolve to the console rather than to a bare path.
func TestGuide_sshKeyPagesAreLinkedAndDerived(t *testing.T) {
	cat := guideCat()

	submit := cat.ResolveLink("sshkey")
	queue := cat.ResolveLink("sshkeyqueue")
	if !strings.HasPrefix(submit, cat.AdminConsole) || !strings.HasSuffix(submit, "/ssh-key") {
		t.Errorf("submit page must hang off the console base, got %q (base %q)", submit, cat.AdminConsole)
	}
	if !strings.HasPrefix(queue, cat.AdminConsole) || !strings.HasSuffix(queue, "/onboarding/ssh-keys") {
		t.Errorf("queue must hang off the console base, got %q (base %q)", queue, cat.AdminConsole)
	}

	// An empty console base must yield NO link rather than a bare path — the
	// whole point of resolving centrally is that unconfigured renders as words.
	if got := (Catalog{}).ResolveLink("sshkey"); got != "" {
		t.Errorf("no console ⇒ no ssh-key link, got %q", got)
	}

	op := renderGuide(t, "platform-admin", "operator", "")
	if !strings.Contains(op, submit) {
		t.Error("the page that tells a platform admin to submit a key must link the page that takes it")
	}
	// Submitted / approved / installed are three different states and the gap
	// between the last two is where people conclude the system is broken.
	for _, want := range []string{"APPROVED", "INSTALLED", "install has not run yet"} {
		if !strings.Contains(op, want) {
			t.Errorf("operator topic must distinguish approval from installation: missing %q", want)
		}
	}

	adm := renderGuide(t, "iam-admin", "identity-admin", "")
	if !strings.Contains(adm, queue) {
		t.Error("the admin who approves keys needs the queue linked, not described")
	}
	if !strings.Contains(adm, "Approving does not install") {
		t.Error("an approver who thinks approval grants access will tell people they have shell they do not have")
	}
}

// HOW A CREDENTIAL REACHES A PERSON IS PER-GOATNET CONFIG, AND THE GUIDE MUST
// FOLLOW IT. A goatnet that mints on collect must not promise an emailed bundle,
// and a goatnet that mints at provisioning and prints the key IN the email must
// not tell readers their credentials "do not travel by email".
func TestGuide_credentialFlowFollowsTheGoatnet(t *testing.T) {
	render := func(mintOnCollect bool, topic string) string {
		c := guideCat()
		c.MintOnCollect = mintOnCollect
		out, err := RenderGuidePage(BuildGuidePage(c, "developer", topic, ""))
		if err != nil {
			t.Fatalf("render mintOnCollect=%v topic=%q: %v", mintOnCollect, topic, err)
		}
		return unescaped(out)
	}

	// Mint-on-collect goatnet: the page sends them to the console page, and
	// must never promise an emailed bundle link.
	for _, topic := range []string{"", "fabric"} {
		got := render(true, topic)
		if !strings.Contains(got, "fabric-credential page") {
			t.Errorf("mint-on-collect topic=%q should name the fabric-credential page", topic)
		}
		if strings.Contains(got, "bundle download link in your welcome email") {
			t.Errorf("mint-on-collect topic=%q promises an emailed bundle link that does not exist there", topic)
		}
	}

	// Emailed goatnet: the credential is already minted and in their mail. The
	// page must not send them to a mint page they cannot use, and must not
	// claim the key avoids email — it is printed in it.
	for _, topic := range []string{"", "fabric"} {
		got := render(false, topic)
		if !strings.Contains(got, "welcome email") {
			t.Errorf("emailed-flow topic=%q should point at the welcome email", topic)
		}
		if strings.Contains(got, "do not travel by email") {
			t.Errorf("emailed-flow topic=%q claims credentials avoid email, but the key is printed in it", topic)
		}
		if strings.Contains(got, "you mint them once on the console") {
			t.Errorf("emailed-flow topic=%q sends the reader to a mint page that is not their flow", topic)
		}
	}

	// The step and the deep dive it links to must agree — they disagreed on
	// BOTH goatnets before this, in opposite directions.
	for _, mint := range []bool{true, false} {
		step, deep := render(mint, ""), render(mint, "fabric")
		stepMint := strings.Contains(step, "fabric-credential page")
		deepMint := strings.Contains(deep, "fabric-credential page")
		if stepMint != deepMint {
			t.Errorf("mintOnCollect=%v: path step and fabric deep dive describe different flows", mint)
		}
	}

	// Swapping is per-request: it must never mutate the shared package data.
	_ = render(true, "fabric")
	if Topics["fabric"].Sections[0].ID != "bundle" ||
		!strings.Contains(strings.Join(Topics["fabric"].Sections[0].Paras, " "), "welcome email") {
		t.Error("rendering a mint-on-collect page mutated the shared Topics data")
	}
	if RoleByKey("developer").Path[1].ID != "credential" {
		t.Error("rendering mutated the shared Roles data")
	}
}

// The console's /docs/onboarding page was retired into this topic (its content
// had drifted and its gate refused most of the people sent to it). Three of its
// sections had no equivalent anywhere in the guide, so they moved here rather
// than being dropped. This test is the anchor that keeps them.
func TestGuide_identityAdminCarriesTheRetiredConsolePageContent(t *testing.T) {
	out := renderGuide(t, "iam-admin", "identity-admin", "")

	for _, want := range []string{
		// "no email yet" almost always means "not approved yet"
		"ADMIN APPROVES",
		"onboarding queue",
		// the two group vocabularies, and that they do not mix
		"GALAXY-DEVELOPER",
		"PLATFORM-ADMIN",
		"grants nothing",
		// changing a role later — the save IS the notification
		"Save groups",
		"Email role reference",
		// provenance fields an admin actually sees on a record
		"sponsorship approved by",
		"retroactive",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("identity-admin topic lost %q — it has no other home since the console page was retired", want)
		}
	}
}

// The mesh plane is site-prefixed and the sign-on plane is not. Stating it the
// other way round sends an admin to type a name that gates nothing, which is
// precisely the failure the section warns about.
func TestGuide_groupVocabulariesDoNotPrefixTheSignOnPlane(t *testing.T) {
	out := renderGuide(t, "iam-admin", "identity-admin", "")

	if strings.Contains(out, "ZAGROS-PLATFORM-ADMIN") || strings.Contains(out, "ZAGROS-IAM-ADMIN") {
		t.Error("sign-on groups rendered with a site prefix; they are canonical and unprefixed")
	}
}

// Deadline renders as "<What> lasts <Window>." so Window MUST be a duration.
// A clause there produced, verbatim on the live page:
//
//	the portal key + bundle on the fabric-credential page lasts
//	the mint page works only ONCE.
//
// Nothing failed — it is two valid strings in a valid template. This walks
// every Clock in the model and rejects a Window that cannot follow "lasts".
func TestGuide_everyClockWindowReadsAsADuration(t *testing.T) {
	check := func(where string, d *Deadline) {
		t.Helper()
		if d == nil {
			return
		}
		if d.Window == "" && d.Limit == "" {
			t.Errorf("%s: Clock sets neither Window nor Limit — renders a bare sentence", where)
			return
		}
		if d.Window == "" {
			return // Limit form; it reads as a predicate, not a span
		}
		// A duration opens with a number, or with a hedge before one. Anything
		// starting with an article or a noun phrase is a clause, not a span.
		for _, bad := range []string{"the ", "a ", "it ", "this ", "you "} {
			if strings.HasPrefix(strings.ToLower(d.Window), bad) {
				t.Errorf("%s: Window %q cannot follow \"lasts\" — it is a clause, not a duration. Use Limit.",
					where, d.Window)
			}
		}
		if d.Recover == "" {
			t.Errorf("%s: Clock has no Recover — a bound with no way out reads as a locked door", where)
		}
	}

	for _, r := range Roles {
		for _, s := range r.Path {
			check("role "+r.Key+" step "+s.Title, s.Clock)
		}
	}
	// Topic sections carry Clocks too. The first version of this test walked
	// only the role paths and passed while a second copy of the same broken
	// Window sat in bundleSectionMintOnCollect — found by rendering every view
	// and reading it, not by the test.
	for key, tp := range Topics {
		for _, s := range tp.Sections {
			check("topic "+key+" section "+s.Heading, s.Clock)
		}
	}
	check("credentialStepMintOnCollect", credentialStepMintOnCollect.Clock)
	check("credentialStepEmailed", credentialStepEmailed.Clock)
	check("bundleSectionMintOnCollect", bundleSectionMintOnCollect.Clock)
	check("bundleSectionEmailed", bundleSectionEmailed.Clock)
}

// The rendered sentence itself, on every view a reader can actually reach, in
// BOTH credential shapes — the broken copy lived in the mint-on-collect branch,
// so a test that rendered only the default shape saw nothing wrong.
func TestGuide_clockSentencesAreGrammatical(t *testing.T) {
	roles := []string{"browse", "developer", "iam-admin", "platform-admin", "network-admin", "partner"}
	topics := []string{"credentials", "cli", "partner-data", "surfaces", "fabric", "identity-admin",
		"operator", "network", "network-health", "gotchas", "repos", "troubleshooting"}

	for _, mint := range []bool{true, false} {
		cat := guideCat()
		cat.MintOnCollect = mint
		render := func(role, topic string) string {
			t.Helper()
			out, err := RenderGuidePage(BuildGuidePage(cat, role, topic, ""))
			if err != nil {
				t.Fatalf("mintOnCollect=%v %s/%s: %v", mint, role, topic, err)
			}
			return unescaped(out)
		}
		views := map[string]string{}
		for _, r := range roles {
			views["role="+r] = render(r, "")
		}
		for _, tp := range topics {
			views["topic="+tp] = render("platform-admin", tp)
		}
		for name, out := range views {
			for _, bad := range []string{"lasts the ", "lasts a ", "lasts it ", "lasts you "} {
				if strings.Contains(out, bad) {
					t.Errorf("mintOnCollect=%v %s rendered %q — a Clock Window carries a clause, not a duration",
						mint, name, bad)
				}
			}
		}
	}
}

// Anything describing HOW the credential arrives must swap with the goatnet's
// flow. The swap existed but was wired to one step; three other roles described
// fetching an emailed bundle in their own words and never swapped, so on a
// mint-on-collect goatnet they told the reader to fetch a link that is never
// sent and to beat an expiry that does not exist.
//
// This asserts the OUTCOME on the rendered page rather than the mechanism: on
// mint-on-collect, no view may promise an emailed bundle.
func TestGuide_noViewPromisesAnEmailedBundleWhenTheGoatnetMintsOnCollect(t *testing.T) {
	cat := guideCat()
	cat.MintOnCollect = true

	emailedBundleClaims := []string{
		"bundle download link in your welcome email",
		"fetch your bundle before its link expires",
		"your certificate and key arrive as a one-time bundle",
		"the bundle link about 24 hours",
	}

	for _, role := range []string{"browse", "developer", "iam-admin", "platform-admin", "network-admin", "partner"} {
		out, err := RenderGuidePage(BuildGuidePage(cat, role, "", ""))
		if err != nil {
			t.Fatalf("render %s: %v", role, err)
		}
		out = unescaped(out)
		for _, claim := range emailedBundleClaims {
			if strings.Contains(out, claim) {
				t.Errorf("role %s promises %q on a mint-on-collect goatnet — the step does not swap", role, claim)
			}
		}
	}
}

// And the mirror: on a goatnet that DOES email the bundle, no view may tell the
// reader to go and mint one. Getting this direction wrong is the same defect.
func TestGuide_noViewSendsYouToMintWhenTheGoatnetEmailsTheBundle(t *testing.T) {
	cat := guideCat()
	cat.MintOnCollect = false

	for _, role := range []string{"browse", "developer", "iam-admin", "platform-admin", "network-admin", "partner"} {
		out, err := RenderGuidePage(BuildGuidePage(cat, role, "", ""))
		if err != nil {
			t.Fatalf("render %s: %v", role, err)
		}
		out = unescaped(out)
		for _, claim := range []string{"mint your fabric credential on the console", "you mint your own certificate and key"} {
			if strings.Contains(out, claim) {
				t.Errorf("role %s tells the reader to mint on a goatnet that emails the bundle: %q", role, claim)
			}
		}
	}
}

// Every registered variant pair must actually differ, and keep its own ID —
// a pair that is accidentally identical is a swap that silently does nothing.
func TestGuide_stepVariantsDifferAndKeepTheirID(t *testing.T) {
	for id, v := range stepVariants {
		if v[0].Body == v[1].Body {
			t.Errorf("stepVariants[%q]: both variants have the same Body — the swap does nothing", id)
		}
		for i, s := range v {
			if s.ID != id {
				t.Errorf("stepVariants[%q][%d] has ID %q — a swapped-in step that loses its ID cannot swap again", id, i, s.ID)
			}
		}
	}
}

// The custodian is told to ASK for a machine allowance. The person they ask
// must have been told what granting it means, in their own path — otherwise
// the request lands as a ticket, the gear waits, and the ungoverned path (a
// joining key with hand-picked groups) is the one that gets taken. Both admin
// tiers grant it, so both carry the step; and the deep dive must mark the
// granting side as an admin section, because its Audience badge says
// "Anyone who owns equipment" and an unmarked admin instruction inside that
// reads as something every custodian is supposed to do.
func TestGuide_theQuotaGranterIsToldWhatTheyAuthorise(t *testing.T) {
	for _, key := range []string{"iam-admin", "platform-admin"} {
		r := RoleByKey(key)
		found := false
		for _, st := range r.Path {
			if st.Topic == "machines" {
				found = true
			}
		}
		if !found {
			t.Errorf("%s path has no machines step — this tier grants the allowance that "+
				"machine enrolment spends, so it cannot learn that only by clicking a topic list", key)
		}
	}
	var granting *Section
	for i, sec := range Topics["machines"].Sections {
		if strings.Contains(strings.ToLower(sec.Heading), "granting") {
			granting = &Topics["machines"].Sections[i]
		}
	}
	if granting == nil {
		t.Fatal("the machines topic promises the granting side in its standfirst and has no section for it")
	}
	if granting.For == "" {
		t.Error("the granting section does not say it is for admins — inside a topic badged " +
			"'Anyone who owns equipment' that reads as instructions for every custodian")
	}
	if strings.TrimSpace(granting.Warn) == "" {
		t.Error("the granting section must name the wrong move an admin under time pressure " +
			"actually makes, not only describe the right one")
	}
	// And it must actually reach the page with its audience chip — a For field
	// that renders as nothing is the same as not having marked it.
	out := renderGuide(t, "platform-admin", "machines", "")
	if !strings.Contains(out, granting.Heading) {
		t.Errorf("the machines topic does not render %q", granting.Heading)
	}
	if !strings.Contains(out, `class="for"`) {
		t.Error("the granting section's audience chip does not render")
	}
}

// TestGuide_troubleshootingNetbirdBlocksArePerOS: the "right mesh" fix must be
// two pasteable blocks, one per OS. A single block with "# macOS" / "# Windows"
// comment lines fails on Windows — `#` is not a comment in cmd.exe — which is
// the paste-as-is failure the welcome email was rewritten to avoid (GH #944).
func TestGuide_troubleshootingNetbirdBlocksArePerOS(t *testing.T) {
	var sec *Section
	for i := range Topics["troubleshooting"].Sections {
		if strings.Contains(Topics["troubleshooting"].Sections[i].Heading, "RIGHT mesh") {
			sec = &Topics["troubleshooting"].Sections[i]
		}
	}
	if sec == nil {
		t.Fatal("troubleshooting topic has no 'RIGHT mesh' section")
	}
	// PR #1336 review (contributore): the block must name --profile <goatnet> so
	// it lands in a profile dedicated to this goatnet rather than reusing (and
	// silently overwriting) whichever one is currently active. A separate
	// `netbird profile add/select` pair ahead of a bare down/up does NOT fix
	// this: root's own active-profile pointer is what a bare `sudo netbird up`
	// reads (pre-v0.78 clients), not the one just selected as the unprivileged
	// user. --profile on the up itself switches and connects as one identity
	// (root before v0.78, the sudo invoking user from v0.78), so it is the
	// only place this can be fixed.
	if !strings.Contains(sec.Pre, "sudo netbird up --profile <goatnet> --management-url") {
		t.Errorf("macOS/Linux block must run the up itself with --profile <goatnet>: %q", sec.Pre)
	}
	if strings.Contains(sec.WinPre, "sudo") || strings.Contains(sec.WinPre, "#") {
		t.Errorf("Windows block must be pasteable into cmd.exe as-is (no sudo, no # lines): %q", sec.WinPre)
	}
	if !strings.Contains(sec.WinPre, "netbird up --profile <goatnet> --management-url") {
		t.Errorf("Windows block must run the up itself with --profile <goatnet>: %q", sec.WinPre)
	}
	out := renderGuide(t, "browse", "troubleshooting", "")
	if strings.Count(out, "<pre><code>netbird up --profile") != 1 || strings.Count(out, "<pre><code>sudo netbird up --profile") != 1 {
		t.Errorf("expected one <pre> per OS running --profile up, one with sudo and one without\n---\n%s", out)
	}
	for _, label := range []string{"macOS / Linux</span>", "Windows (administrator Command Prompt)</span>"} {
		if !strings.Contains(out, label) {
			t.Errorf("each block needs its OS label: missing %q", label)
		}
	}
}

// The Windows page's two NetBird traps ("Cloud or Self-hosted" dialog, and
// "Already connected" and nothing changes) both fix themselves by re-running
// the enrolment line. PR #1336 review: that line must name --profile
// <goatnet> — a bare `netbird down` + `netbird up` silently overwrites
// whichever profile is active, and doesn't even fix "Already connected"
// (still silently ignored with no --profile to force the drop).
func TestGuide_windowsNetbirdTrapsUseTheProfileEnrolmentLine(t *testing.T) {
	var found int
	for _, sec := range Topics["windows"].Sections {
		if !strings.Contains(sec.Heading, "Cloud or Self-hosted") && !strings.Contains(sec.Heading, "Already connected") {
			continue
		}
		found++
		if !strings.Contains(sec.Pre, "netbird up --profile <goatnet> --management-url") {
			t.Errorf("section %q must run the up itself with --profile <goatnet>: %q", sec.Heading, sec.Pre)
		}
		if strings.Contains(sec.Pre, "netbird profile add") || strings.Contains(sec.Pre, "netbird down") {
			t.Errorf("section %q should not need a separate profile-creation or down step: %q", sec.Heading, sec.Pre)
		}
	}
	if found != 2 {
		t.Fatalf("expected both NetBird traps on the windows page, found %d", found)
	}
}

// The stale-binary trap tells people to update first. On macOS/Linux the
// binary is in /usr/local/bin (root-owned), where a plain update is denied and
// a bare `sudo goat update` loses GOAT_GET_ENDPOINT — the fix has to carry it
// through with `sudo env`.
func TestGuide_staleBinaryTrapCoversRootOwnedInstall(t *testing.T) {
	var found bool
	for _, sec := range Topics["gotchas"].Sections {
		if !strings.Contains(sec.Heading, "E_BUNDLE_INVALID") {
			continue
		}
		found = true
		if !strings.Contains(sec.Instead, "sudo env GOAT_GET_ENDPOINT=") || !strings.Contains(sec.Instead, "/usr/local/bin/goat update") {
			t.Errorf("%q: Instead must give the sudo-safe update for a root-owned install:\n%s", sec.Heading, sec.Instead)
		}
		if strings.Contains(sec.Instead, "sudo goat update") {
			t.Errorf("%q: a bare `sudo goat update` drops GOAT_GET_ENDPOINT", sec.Heading)
		}
	}
	if !found {
		t.Fatal("no gotchas section mentions E_BUNDLE_INVALID; test needs updating")
	}
}
