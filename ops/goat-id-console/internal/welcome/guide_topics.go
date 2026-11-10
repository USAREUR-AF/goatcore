package welcome

// The deep dives. The index carries a role's path in a few sentences per step;
// everything that would bloat it lives here, one click away.
//
// Every topic is reachable by URL regardless of role. The guide is
// unauthenticated and credential-free by construction, so a role's topic list is
// about relevance, not restriction — and a platform admin reading the browse
// path, or a developer reading the network one, is a good outcome, not a leak.

// Topic is one deep dive.
type Topic struct {
	Key        string
	Title      string
	Standfirst string
	Sections   []Section

	// --- the badges -----------------------------------------------------
	// A wall of equally-weighted cards tells a newcomer nothing about where to
	// start, so every card advertises who it is for, how much it matters, and
	// what it will cost them. All four are deliberately coarse: the point is to
	// let somebody TRIAGE, and false precision would invite argument about
	// whether a number is exactly right.

	// Audience is the short "who needs this" line, in plain words.
	Audience string
	// Essential marks the things a reader cannot do their job without. Anything
	// not essential is genuinely optional — if everything is essential, nothing
	// is, and the badge stops carrying information.
	Essential bool
	// ReadMin is minutes to read and understand.
	ReadMin int
	// DoMin is minutes to actually PROVE you can do the thing — install it, run
	// it, watch it work. Zero means there is nothing to prove, only to know.
	// It is separate from ReadMin because "I read it" and "I can do it" are
	// different states, and estimating only the first is how people end up
	// surprised by an afternoon.
	DoMin int
}

// Section is a heading plus paragraphs, optional fixed-width block, optional
// deadline, optional warning.
type Section struct {
	// ID is a stable handle for sections the renderer SWAPS per goatnet (the
	// credential-delivery shape). Only varying sections carry one.
	ID      string
	Heading string
	// For says WHO this section applies to. A topic can legitimately mix
	// audiences — the traps page is worth everyone's time, but half of it
	// describes commands only an admin can run. Leaving that unmarked makes a
	// reader work out whether a warning is even about them, and most will
	// either assume it is not, or worry that it is.
	// Blank means everyone.
	For string

	// The parts below are TYPED rather than prose, because an earlier version
	// wrote "THE SITUATION:" and "WHAT TO DO INSTEAD:" into the paragraphs
	// themselves. That is a text substitute for layout: it adds words to label a
	// dimension the page should be SHOWING. Typed parts let each get its own
	// visual treatment, and let the labels shrink to a quiet marker or vanish.

	// Scenario is the concrete situation that produces this — what the reader
	// was doing when they met it. A warning without one leaves them unable to
	// tell whether it is ever about them.
	Scenario string
	// Paras explain what is actually happening.
	Paras []string
	// Links are surfaces this section points at, resolved from the goatnet's
	// Catalog at render time. Topics are static data, so they name a surface by
	// KEY and the renderer supplies the URL — which is what makes this a
	// mesh-hosted guide rather than a description of one. A key with no
	// configured URL renders as plain text, never as a dead link.
	Links []Link
	// Instead is the corrected action. Rendered as its own affordance, because
	// it is the one line a reader in a hurry needs.
	Instead string
	Pre     string // fixed-width block, rendered verbatim
	// WinPre is the Windows shape of Pre, rendered as its OWN block under an
	// OS label (Pre gets a "macOS / Linux" label when WinPre is set). One
	// block with "# macOS" / "# Windows" comment lines was tried first: `#`
	// is not a comment in cmd.exe, so a Windows reader pasting it as-is got
	// "'#' is not recognized" plus the sudo errors (GH #944).
	WinPre string
	Clock  *Deadline // time-sensitive, rendered as its own element
	Warn   string    // a gotcha inline with its context
}

// Link names a surface in the goatnet Catalog. Key is one of: portal,
// panoscope, flashlight, stethoscope, microscope, grafana, map, sandbox, docs,
// isitup, chat, get, relayhealth, adminconsole, identity, auth.
type Link struct {
	Label string
	Key   string
	// URL is an outside page the guide points at — an upstream vendor's own
	// troubleshooting doc, for a problem that is theirs to describe. Set one
	// of Key or URL, never both. A Key link is this goatnet's surface and is
	// resolved at render time; a URL link is the same on every goatnet.
	URL string
}

// The fabric topic's opening section describes HOW THE BUNDLE REACHES YOU,
// which is the same per-goatnet fact the credential path step turns on (see
// guide_model.go). It has to swap with it: the step and the deep dive it links
// to were giving opposite instructions on both goatnets — on the goatnet that
// mints on collect the step said "mint it yourself" and this section said
// "wait for the emailed link", and on the goatnet that emails it, the reverse.
var bundleSectionEmailed = Section{
	ID: "bundle", Heading: "Your bundle", Paras: []string{
		"A certificate and private key, the fabric CA root, and a profile for the CLI. It arrives as a one-time, mesh-only download link in your welcome email — fetch it early, before you are ready to build."},
	Clock: &Deadline{Window: "about 24 hours, and it works only ONCE",
		What: "the bundle download link in your welcome email", Recover: "ask an admin to re-send your welcome package"},
}

var bundleSectionMintOnCollect = Section{
	ID: "bundle", Heading: "Your bundle", Paras: []string{
		"A certificate and private key, the fabric CA root, and a profile for the CLI. Nothing is sent to you: you mint the bundle yourself, once, on the console's fabric-credential page, together with your portal API key. Do it early — before you are ready to build."},
	// Limit, not Window: the page does not expire, it just refuses a second
	// attempt. See the Deadline doc comment.
	Clock: &Deadline{Limit: "does not expire, but works only ONCE",
		What: "the bundle + portal key on the fabric-credential page", Recover: "ask an admin to re-issue (rotate) your credential — that invalidates the previous key and bundle, so only if they are truly lost"},
}

// BundleSectionFor returns the bundle section matching this goatnet's flow.
func BundleSectionFor(mintOnCollect bool) Section {
	if mintOnCollect {
		return bundleSectionMintOnCollect
	}
	return bundleSectionEmailed
}

// Topics is the closed set, keyed for URL lookup.
var Topics = map[string]Topic{

	"credentials": {
		Key: "credentials", Title: "The four credentials, and where each lives",
		Standfirst: "Most confusion about access is really confusion about which of these four is meant. They are not interchangeable and they arrive by different routes.",
		Audience:   "Everyone, helpers most of all", Essential: true, ReadMin: 4, DoMin: 0,
		Sections: []Section{
			{Heading: "Your handle is an identifier, not a credential", Paras: []string{
				"Your handle is the canonical name GOAT uses for your account. Find it on the identity console's Your identity page and give that exact value to an administrator when requesting an account or access change.",
				"A handle is not your email address, display name, organisation identifier, password, setup key, API key, certificate, or private key. Never guess it from any of those values."},
				Links: []Link{{Label: "Open Your identity", Key: "identity"}}},
			{Heading: "Mesh setup key", Paras: []string{
				"Joins your machine to the private network. Arrives in your welcome email. Nothing else works until this does — every internal address stays unreachable until the mesh is up.",
				"It is the whole credential. There is no account to create and no password to pick for the mesh — if a page is asking you to register, you are on the wrong one."},
				Warn: "A NetBird account is not one of the four. The client you install is also sold as a hosted service at netbird.io, and signing up there puts your machine on that public network instead of this one — while `netbird status` still reports \"Connected\". Download the client from netbird.io if you like; never register with it."},
			{Heading: "Login (password or one-time link)", Paras: []string{
				"Single sign-on for the web surfaces that accept your login. Your welcome email contains a one-time recovery link or code.",
				"Open the recovery page from STEP 2 of your welcome email. Enter the code if one is supplied. Recovery opens account settings; it does not set your password.",
				"Find the Password section. Enter a new password. Select that section's save button. Wait for confirmation that the password was saved before you continue to STEP 3.",
				"If the password is rejected, follow the validation message. Enter an acceptable password and save again. For an expired link or code, ask your admin for a fresh one.",
				"An access-update email does not require a new password. Your existing password keeps working."},
				Clock: &Deadline{Window: "24 hours — the login link and code come from one request and share the deadline",
					What: "the one-time login link", Recover: "ask an admin for a fresh one — your account is not locked"}},
			{Heading: "Portal API key (vp_…)", Paras: []string{
				"The Portal, and only the Portal. It does NOT take your password — you paste this key into its sign-in box. You mint it yourself, once, on the console's fabric-credential page, together with your certificate: it is shown a single time and cannot be re-read afterwards, by you or by an admin. Older packages carried it in the welcome email and in the goat-cli.cbor inside the bundle; a key collected on that page is in neither. Roles with no fabric identity have no portal key at all, which is a level, not a fault.",
				"This is the key nearly everyone means when they say they cannot get into the Portal. The operator key (op_…) is a different credential with a different job — it administers the whole goatnet, only admins hold one, and it is not what signs you in here."},
				Warn: "Do not ask for a rotation to get a \"fresh\" key. Rotating re-issues your certificate and makes the key you already have stop working."},
			{Heading: "Fabric certificate and private key", Paras: []string{
				"Your identity on the data fabric. This is the one credential that NEVER travels by email — it comes only in the bundle, over the mesh. The private key is not recoverable: losing it means re-issuing your identity."},
				Clock: &Deadline{Window: "about 24 hours, and it works only ONCE",
					What: "the bundle download link", Recover: "ask an admin to re-send your welcome package; nothing you already hold is invalidated"}},
		},
	},

	"cli": {
		Key: "cli", Title: "Install the goat CLI",
		Standfirst: "The one-line install, the per-OS traps, and how to learn the fabric before touching anything live.",
		Audience:   "Anyone who will run commands", Essential: true, ReadMin: 5, DoMin: 15,
		Sections: []Section{
			{Heading: "Install", Paras: []string{
				"Open the get page for your goatnet while on the mesh — it shows the exact one-line command for your operating system."},
				Links: []Link{{Label: "Open the get page", Key: "get"}, {Label: "CLI docs", Key: "docs"}}},
			{Heading: "macOS", Paras: []string{
				"The page includes a one-time re-sign step immediately after install. Run it. Without it the binary can hang on launch, which looks like a network problem and is not."}},
			{Heading: "Windows", Paras: []string{
				"The page offers three ways in — a Command Prompt line (curl.exe ships with Windows 10+), a PowerShell line, and a direct .exe. They are the same binary; run one of them, not several. Either shell works; the mesh-join step used Command Prompt. Verify against the published checksums (`certutil -hashfile goat.exe SHA256` in Command Prompt); if yours differs, delete it and re-download on a stable connection.",
				"After adding the binary's folder to your PATH, close the Command Prompt (or PowerShell) window and open a NEW one. PATH changes only apply to new windows, and this is the single most common \"it didn't install\" report."}},
			{Heading: "Learn it hands-on", Paras: []string{
				"`goat tutor` is an interactive, resumable walkthrough against a built-in sandbox. It needs no profile and no bundle, so it works before you have fabric credentials.",
				"`goat tutor --list` shows the lessons without starting."}},
			{Heading: "The lenses: top, who, flow", Paras: []string{
				"Three commands answer the questions everyone ends up asking. `goat top` is the live dashboard of the fabric — who is producing, how much, into what. `goat who` is the roster — who is on this goatnet, searchable, with what each identity actually is across the layers. `goat flow` answers the narrowest and most common question: is data flowing on this topic, and when did it last?",
				"`goat flow` labels every answer with WHICH observer said it, and says \"no observer could answer\" rather than \"no data\" when it cannot know — those are different facts, and the CLI refuses to round one into the other.",
				"If your installed CLI lacks one of these, it predates them — re-run the install from the get page to update."},
				Links: []Link{{Label: "Open the get page", Key: "get"}}},
			{Heading: "The CLI does not replace the mesh client", Paras: []string{
				"Keep managing the mesh with the NetBird client. On some systems the CLI can wrap its tunnel; on Windows it cannot yet."}},
		},
	},

	// Machine enrolment is CUSTODIAN SELF-SERVICE for the function classes
	// (ADR 1096 decision #1), so the person who performs it is usually a
	// developer-tier custodian, not an admin. The guide previously mentioned
	// custodianship only in the partner path and one line of partner-data,
	// which left the people who actually run `goat machine enroll` with the
	// walkthrough repo and no practice to go with it.
	//
	// The standfirst also promised the OTHER half — "for the admins who grant
	// the quota, what you are authorizing when you do" — and no section
	// delivered it. That gap is not cosmetic: the custodian's instruction is
	// "ask for the quota", and if the admin they ask has never read what
	// granting it means, the request becomes a ticket, the gear sits on the
	// bench, and the ungoverned path (a joining key with hand-picked groups)
	// is the one that gets taken. So the granting side is a marked section
	// here rather than a separate admin-only page.
	"machines": {
		Key: "machines", Title: "Bringing equipment onto the fabric",
		Standfirst: "Enrolling a machine is one command, but the class you choose is a decision the whole mesh then enforces. This is what that choice means, and how to undo it — and, for the admins who grant the quota, what you are authorizing when you do.",
		Audience:   "Anyone who owns equipment", Essential: true, ReadMin: 8, DoMin: 20,
		Sections: []Section{
			{Heading: "It is your own act, against a standing allowance", Paras: []string{
				"Equipment you own enrolls through YOUR identity as its custodian, against a quota an operator grants you once. There is no review queue: the quota is the pre-authorization, and it was granted deliberately.",
				"A quota of zero is the starting state, not an error. Ask for the quota before you try to enroll, and the refusal you would otherwise hit will not happen."}},
			{Heading: "Granting the allowance — what you are authorizing", For: "Admins who grant the quota",
				Paras: []string{
					"You are not approving a machine and you are not choosing its class. You are saying how many machines this person may bring on, and that each one may mint its own fabric identity, derive its own network groups, and be recorded as theirs. What the machine may reach is derived from its class at enrollment and enforced from then on — that is not a judgement call you make per machine, which is why there is no review queue for you to sit in.",
					"Grant it at onboarding, not on request. A custodian whose allowance is still zero when the gear is on the bench has an alternative that nobody governs, and it takes about a minute. Setting the allowance when you bring the person on removes the pressure that produces it.",
					"An allowance is a ceiling, not a budget. Retiring a machine returns its unit, so a custodian who cleans up after themselves does not come back to you for more. Raising one is cheap and reversible: lower it again and the machines already enrolled are untouched — only the next enrollment is refused."},
				Warn: "Never resolve an urgent enrollment by minting a joining key and choosing its groups yourself. There is a supported way to do it while the portal is down — the same single-use key carrying exactly the groups the class derives, and the machine stamped with its custodian afterwards — and the audit will keep reporting that peer until the stamp lands, which is the behavior we want and not a fault."},
			{Heading: "The class is a FUNCTION, not a kind of box", Paras: []string{
				"Pick the class by what the machine DOES, not what it is. A box doing two jobs is enrolled twice, once per job — that is exactly what stops a catch-all category forming, which is what happened before these classes existed.",
				"The class then derives the machine's network groups automatically and mints it a single-use joining key carrying exactly those and nothing else. Nobody types the groups. That is the point: on one afternoon before this existed, an effector ground station, a sensor gateway and a person's laptop were put into one group in a single sitting — not carelessness, just nothing in the path having an opinion."}},
			{Heading: "Refusals are telling you the answer", Paras: []string{
				"Some classes are deliberately not enrollable, and the refusal names what to use instead. Read it rather than working around it: a refusal you cannot act on is how the wrong category gets chosen.",
				"If the refusal says the site's groups do not exist, that needs BOTH halves — the groups created AND a policy that uses them. Creating the groups alone leaves the network wide open while everything looks correctly labeled."}},
			{Heading: "Name the peer after the machine", Paras: []string{
				"When the machine joins, its network name must match the handle you enrolled it under. That pair is what ties the machine on the wire to the record of who brought it on.",
				"Leave it on a factory hostname and two things break quietly: the audit reports it as unaccounted-for even though its paperwork is fine, and the retire command cannot find it to cut it off."},
				Warn: "If you rename anything, rename the peer to match the handle — never add an exception to the audit instead."},
			{Heading: "Retiring equipment is part of owning it", Paras: []string{
				"When a machine is done, retire it. That cuts it off the network first — the peer and any unused joining key — and only then releases its identity, so a half-finished retirement cannot leave a machine still connected behind a record that says otherwise.",
				"Your allowance comes back automatically when you do, so retiring is also how you make room for the next one. Re-enrolling the same name later creates a NEW identity; the old one is not recoverable, which is why retiring is worth doing deliberately rather than by habit."}},
			{Heading: "Offline and hand-carried cases", Paras: []string{
				"The joining key is single-use with a limited window, which is a window to JOIN and not a measure of how long the machine is trusted. If a box was still in transit when the window closed, ask for a fresh key rather than a fresh enrolment — a lapsed window is not a second machine and does not cost you another unit of quota.",
				"Equipment that will never see the network at enrolment time can be issued an offline package instead, produced from the same record so its groups are still derived rather than typed."}},
		},
	},

	"partner-data": {
		Key: "partner-data", Title: "Data across organization lines",
		Standfirst: "Your namespace is yours alone. What crosses between organizations moves through release lanes that somebody deliberately opened — and it is worth knowing plainly what is enforced today and what is not.",
		Audience:   "Partner organizations", Essential: true, ReadMin: 8, DoMin: 0,
		Sections: []Section{
			{Heading: "Your namespace is yours alone", Paras: []string{
				"Everything you publish lives under your own write anchor, and nobody else can publish under it — not another organization, and not an admin. That single-writer rule is absolute on this fabric; there is no grant that adds a second writer to your root.",
				"The same protection means writes outside your namespace are refused. The refusal is silent by design, so a publish that seems to vanish is usually a write outside your anchor, not a bug in your code."}},
			{Heading: "Release lanes are how data crosses", Paras: []string{
				"A release lane is a shared namespace opened for a named set of organizations. A lane whose set holds one partner is a bilateral release — only they see it. A lane whose set is broad releases to everyone in it. The set's membership IS the audience; there is nothing to configure on your side.",
				"Reading a lane comes with membership. If a lane you expect is not visible, that is a membership or configuration question for the goatnet's admins — asking costs a message, and debugging an absence you cannot see into costs an afternoon."}},
			{Heading: "Publishing into a lane is a grant, not a default", Paras: []string{
				"Being able to READ a lane never implies you may WRITE into it. A producer is granted into a lane by name, by the goatnet's admins. Ask your sponsor or an admin to set it up before you build against it."},
				Warn: "The fabric refuses ungranted writes silently. If your publishes into a lane seem to disappear, check the grant exists before debugging your publisher."},
			{Heading: "One room until it is carved", Paras: []string{
				"Be clear-eyed about the boundary: read separation between organizations on a shared goatnet is applied per-organization by the goatnet's operators — it is not something being a partner switches on. Where it has not been applied, enrolled machines can read broadly across the fabric.",
				"So the honest rule is: treat the goatnet you are on as one room. If something must not be visible to the other organizations in that room, raise it with the operators BEFORE publishing it, and they will either carve the partition or route it another way. Nothing about publishing is undoable-by-support afterwards."}},
			{Heading: "Sharing inside your own organization", Paras: []string{
				"Your organization's own sharing decisions belong to its nominated admins, not to the platform. Shared multi-publisher spaces inside an organization exist as an opt-in; your org's admin creates one and grants publishers onto it. Across organizations, it is always lanes — there is deliberately no shortcut."}},
			{Heading: "Machines and gear", Paras: []string{
				"Your organization's machines enroll through your own custodian identity, against a quota an operator grants. A quota of zero is the starting state, not an error — ask for the quota before enrolling.",
				"Gear that also talks to your own platform — a path off this fabric — is enrolled under a deliberately narrow class with management-plane reach only. Those restrictions describe the device's exposure, not distrust of you; widening one is a decision the operators make, never a side effect of enrollment."}},
			{Heading: "Encrypted audiences", Paras: []string{
				"Some streams are additionally encrypted to a granted audience. If you hold a fabric certificate and are in the audience, decryption happens in your own process — the platform holds no keys. Staff without certificates read such streams through the mediated web surfaces instead, which is a supported path rather than a workaround."}},
		},
	},

	"surfaces": {
		Key: "surfaces", Title: "The surfaces, and which credential each wants",
		Standfirst: "All of them are mesh-only. All of them take your password — except one.",
		Audience:   "Everyone", Essential: true, ReadMin: 3, DoMin: 5,
		Sections: []Section{
			{Heading: "Portal", Paras: []string{"Your identity, certificates, API keys and topics. Signs in with your portal API key, NOT your password."},
				Links: []Link{{Label: "Open the Portal", Key: "portal"}}},
			{Heading: "Panoscope", Paras: []string{"The live common operating picture of the fabric — start here to see what is actually flowing."},
				Links: []Link{{Label: "Open Panoscope", Key: "panoscope"}}},
			{Heading: "Flashlight", Paras: []string{"A minimal inspector, built to stay usable on a bad link."},
				Links: []Link{{Label: "Open Flashlight", Key: "flashlight"}}},
			{Heading: "Stethoscope", Paras: []string{"Deep inspection of a single target."},
				Links: []Link{{Label: "Open Stethoscope", Key: "stethoscope"}}},
			{Heading: "Sandbox", Paras: []string{"An isolated playground for learning the API."},
				Links: []Link{{Label: "Open the Sandbox", Key: "sandbox"}},
				Warn:  "The Sandbox is not the live fabric. Nothing you publish there reaches it, and nothing on the fabric appears there."},
			{Heading: "Microscope", For: "Identity and platform admins", Paras: []string{
				"Everything about ONE entity — a person or a machine: what it may do, what it has been doing, and what it was refused. Panoscope answers \"what is the fabric carrying\"; Microscope answers \"what is this one actor up to\".",
				"It is fenced to admin levels, and every look is itself recorded — inspecting somebody is an auditable act, not a casual one."},
				Links: []Link{{Label: "Open Microscope", Key: "microscope"}}},
			{Heading: "Metrics (Grafana)", For: "Admins", Paras: []string{
				"Dashboards over the hosts and services under the fabric. The door is strict on purpose: without a metrics role on your account the sign-in handshake refuses you outright — that is the mapping working, not an outage."},
				Links: []Link{{Label: "Open Grafana", Key: "grafana"}}},
			{Heading: "Geographic map", For: "Goatnets carrying a live geographic feed", Paras: []string{
				"Live positions and tracks from the fabric's sensor feeds, on a map. What each viewer sees is scoped to what they may see — two people can open the same view and get different tracks, by design.",
				"Only some goatnets carry a feed worth mapping, so this surface exists where the feed does."},
				Links: []Link{{Label: "Open the map", Key: "map"}}},
			{Heading: "Service status", Paras: []string{"What is up right now. Check here before reporting an outage — it answers \"is it me or the system\" faster than anyone can."},
				Links: []Link{{Label: "Open service status", Key: "isitup"}}},
			{Heading: "Relay-health dashboard", For: "Admins and on-call operators", Paras: []string{
				"Answers the question everyone asks during an outage: is it me, or is it the network? It probes the relay fleet from outside and reports what each relay is doing.",
				"Two separate things decide what you get, and confusing them causes most of the questions about this page. First a DOOR: viewing is restricted to admins and operators who have been added to the viewer list, because relay addresses and placement are worth withholding. Then, once inside, GRADING: platform and identity admins get the full document including per-relay detail, everyone else a reduced one that still answers whether the fleet is healthy. Reduced is what anything unrecognized falls back to, so the failure direction is toward less.",
				"If you are not an admin, this page is not your surface and you are not missing anything you need — service status answers \"is the thing I use up?\" for you. If your work genuinely needs the fleet view, ask an admin to add you rather than assuming you were overlooked."},
				Links: []Link{{Label: "Open service status", Key: "isitup"}, {Label: "Open the relay-health dashboard", Key: "relayhealth"}}},
			{Heading: "Identity admin console", For: "Identity and platform admins", Paras: []string{
				"The front door for bringing people on and deciding what they reach: invites, approvals, group changes, the SSH-key queue, and self-service operator keys. If you administer people, this is where that work lives — the surfaces above only show you the consequences."},
				Links: []Link{{Label: "Open the admin console", Key: "adminconsole"}}},
			{Heading: "Other admin surfaces", Paras: []string{"Dashboard, management, storage and compliance consoles are gated to admin levels. If your level does not include them they will refuse you, and that is expected rather than a fault."}},
		},
	},

	"fabric": {
		Key: "fabric", Title: "Publishing to the data fabric",
		Standfirst: "Your bundle, your namespace, and why writes outside it are refused.",
		Audience:   "Developers and publishers", Essential: true, ReadMin: 6, DoMin: 30,
		Sections: []Section{
			bundleSectionEmailed, // swapped per goatnet by BuildGuidePage
			{Heading: "Trust the CA, then load the profile", Paras: []string{
				"goat ships trusting no goatnet, so `profile init` refuses until you add the root that signed your bundle. The `-ca.pem` sits beside the `.cbor` it verifies.",
				"`goat ca add` prints a fingerprint — check it against one your operator gives you before you rely on it. Once per goatnet; `goat ca ls` shows what you trust."},
				Pre:  "goat ca add <id>-goat-cli-ca.pem\ngoat profile init <id>-goat-cli.cbor",
				Warn: "If `goat ca` errors as an unrecognized command, your binary predates this feature (before goat-cli-v0.5.0) and will reject a perfectly valid bundle as `E_BUNDLE_INVALID — wrong CA`. Update the binary first — see Anti-patterns and traps — then retry `profile init` on the SAME file."},
			{Heading: "Protect the private key", Paras: []string{
				"Keep it readable only by you. It never travels by email and it is not recoverable — a lost key means your identity is re-issued."}},
			{Heading: "Publish under your own namespace", Paras: []string{
				"Writes outside your namespace are refused by design, not by misconfiguration. Your write anchor is shown in the Portal under your identity.",
				"Start from the runnable example in your bundle rather than from scratch — it already has the endpoint, certificate paths and key expression right."}},
		},
	},

	"identity-admin": {
		Key: "identity-admin", Title: "Bringing people on",
		Standfirst: "Onboarding runs on rails for a reason. Going around them produces identities that look right and behave differently.",
		Audience:   "Identity and platform admins", Essential: true, ReadMin: 11, DoMin: 20,
		Sections: []Section{
			{Heading: "Three stages, on purpose", Paras: []string{
				"An invite, a sponsor's vouch, and an admin's approval. No single person both invites and admits. The split is the control — it is not ceremony."}},
			{Heading: "Use the console, never hand-made identities", Paras: []string{
				"A person created directly in the identity provider gets a login and nothing else: no mesh membership, no fabric identity, no record of who brought them on. It looks like an onboarded person right up until they try to work."},
				Warn: "If you find yourself creating identities by hand to move faster, that is a defect report about the rails, not a shortcut."},
			{Heading: "Sponsorship has a ceiling", Paras: []string{
				"You cannot sponsor above your own level. The gate is fail-closed: it refuses rather than quietly granting less than asked."}},
			{Heading: "\"They never got the email\" is almost always \"nobody approved it\"",
				Scenario: "Someone tells you their invitee has heard nothing.",
				Paras: []string{
					"The welcome package is sent seconds after an ADMIN APPROVES and provisioning finishes — not when the invite goes out, and not when the candidate submits. So the first place to look is the onboarding queue, not the mail logs.",
					"Candidates are not left waiting blind in the meantime: a short status mail goes out at each earlier stage, so silence specifically means the proposal is still sitting at a gate."},
				Links: []Link{{Label: "Open the onboarding queue", Key: "onboardingqueue"}}},
			{Heading: "What the record says about who did what",
				Paras: []string{
					"Both human gates are recorded permanently: the voucher as \"sponsorship approved by\", the approving admin as \"onboarding approved by\". An admin is allowed to act at the sponsor gate too, which is why \"sponsorship approved by\" and \"sponsored by\" can name different people on a perfectly normal record.",
					"Records marked retroactive belong to people onboarded before this system existed. Their unrecorded provenance fields say so in as many words rather than showing a blank, so an empty-looking field is a real gap worth asking about rather than just an old record."}},
			{Heading: "When someone cannot reach something", Paras: []string{
				"Work out which of the four credentials is actually involved before minting anything. Most \"I can't get in\" reports are a mesh problem or a level problem, not a missing key."}},
			{Heading: "Changing access beats issuing keys", Paras: []string{
				"If a person needs a surface their level does not include, change the level — that mints what the level implies. Hand-issuing a credential leaves the registry saying one thing and reality another."}},
			{Heading: "Two group vocabularies, and they are not interchangeable",
				Paras: []string{
					"Authorization lives in two separate planes, each with its own group names. Mesh groups answer \"what can this DEVICE reach over the network\" and are attached to a peer; they are written as role stems in tiers and manifests — DEVELOPER, OBSERVER — but the live groups carry the goatnet's prefix, so on a goatnet called GALAXY they are GALAXY-DEVELOPER and GALAXY-OBSERVER. The bare stem is vocabulary; the prefixed form is what exists.",
					"Sign-on groups answer \"what may this PERSON do once a web service signs them in\", ride in their token, and are written exactly as-is with no prefix: PLATFORM-ADMIN, IAM-ADMIN, GRAFANA-READONLY, COMPLIANCE, plus data-access grants like data.view.all or data.view.org:<org>.",
					"Where the same concept genuinely exists in both planes it keeps the same name in both — today that is PLATFORM-ADMIN alone. That is an alignment rule, not permission to mix the planes."},
				Warn:  "A name from one plane pasted into the other grants nothing. The console refuses the attempt, and that refusal is protecting you: a wrong-plane group would save silently and then gate nothing at all.",
				Links: []Link{{Label: "Open the permissions matrix", Key: "permissions"}}},
			{Heading: "Changing someone's role later — the save IS the notification",
				Paras: []string{
					"Edit their sign-on groups on their user page and click Save groups. That save is the trigger: the person is automatically sent an upgrade-framed mail stating that nothing was re-provisioned, showing exactly what changed, and carrying the full reference for their new role. One save, one mail; a save that changes nothing sends nothing. There is no separate send step to remember.",
					"The same page has an Email role reference action for (re)sending anyone the complete current reference for their role — the catch-up for people whose access predates these mails.",
					"Mesh roles and fabric credentials are NOT changed here. Those follow from the onboarding level, and moving them is a re-onboard or an operator action."}},
			{Heading: "Approving SSH keys is a decision, not a formality", For: "Identity and platform admins",
				Links: []Link{{Label: "Open the SSH-key approval queue", Key: "sshkeyqueue"}},
				Paras: []string{
					"People submit their own public keys; nothing is installed until someone approves. That gap is the entire control. The console is reachable with an ordinary web session, so approval is what stands between a stolen session and a shell on a production host.",
					"What you are deciding is whether THIS person should hold shell at the scope the key would carry — not whether the key parses. Approve slowly for anything you would not grant in person.",
					"Approving does not install. An approved key is a decision; a separate operator-run installer places it on the hosts that person's role may reach and only then does the record say installed. So a person you approved still has no access until somebody runs that, which is worth saying to them rather than leaving them to discover it."},
				Warn: "Revoke through the queue rather than editing authorized_keys on a host by hand. The queue's revoke removes the key the same way the offboarding cut does; a hand edit leaves the record and the host disagreeing, and the record is what the next install run reconciles against."},
		},
	},

	"operator": {
		Key: "operator", Title: "Running the goatnet",
		Standfirst: "Host access, the operator key, and the deploy rules that keep two environments from silently diverging.",
		Audience:   "Platform admins", Essential: true, ReadMin: 8, DoMin: 45,
		Sections: []Section{
			{Heading: "Your SSH key is not installed for you", Paras: []string{
				"The onboarding ceremony adds your public key to the shared account only if it HAS your public key — and only you can supply that. When it is absent the step is skipped silently.",
				"Submit your PUBLIC key on the console page below, then wait. Submitting and being granted a shell are separate events on purpose: the console is reachable with an ordinary web session, so if uploading a key also authorized it, a stolen session would become a root-capable shell on a production host.",
				"There are three states and it is worth knowing which one you are in. SUBMITTED means recorded. APPROVED means an admin decided yes — and still no access, because installing is a separate run against the hosts your role may reach. INSTALLED is the one that means you can log in. If the page says approved and SSH refuses you, nothing is broken; the install has not run yet.",
				"Paste the contents of a .pub file. If you are ever unsure which half you are holding, that is the right moment to stop — the page will reject a private key, but the habit of checking is what keeps you from pasting one somewhere that does not."},
				Links: []Link{{Label: "Submit your SSH public key", Key: "sshkey"}},
				Warn:  "If the console refuses you at the door, that is the console gate rather than anything about your key — on some goatnets it admits admins only. Ask an admin."},
			{Heading: "The operator key is a SECOND key, and you request it yourself", Paras: []string{
				"It is a per-person key, prefixed op_, that makes your Portal admin actions attributable to YOU rather than to a shared credential. It is ADDITIONAL to the scoped vp_ key in your welcome email — that one signs you into the Portal and is not replaced by this one. Holding an admin tier does not mean your vp_ key is inert; assuming it did left a platform admin with nothing to paste (2026-08-11).",
				"Request it on the console's operator-key page. An admin approves, and you collect it there once. Approval IS the grant: the Portal reloads its key directory, so there is no install step and no restart to wait for — if you were told to wait for one, that was the old process."},
				Links: []Link{{Label: "Request your operator key", Key: "operatorkey"}}},
			{Heading: "Deploy by the ledger", Paras: []string{
				"Image pins are governed centrally and every deploy records a rollback. Bump the governed pin, never a role default."},
				Warn: "A role-default bump is invisible to anyone reading a goatnet's own config — that is how two environments end up running different builds with nothing to show it."},
			{Heading: "Paired components deploy together", Paras: []string{
				"Some changes span two services. Deploying one half can be worse than deploying neither: a surface that renders a credential nobody supplies will confidently tell people they do not have one."}},
		},
	},

	"network": {
		Key: "network", Title: "Bringing transport gear on",
		Standfirst: "What a network admin is, and why the gear you enroll is the most restricted thing on the mesh. Those two are different subjects, and mixing them up is the usual source of confusion.",
		Audience:   "Network admins", Essential: true, ReadMin: 7, DoMin: 30,
		Sections: []Section{
			{Heading: "First: you and the gear are not the same subject", Paras: []string{
				"This page describes two things that are easy to run together. ONE is you — a person with an admin tier and a set of sites. The OTHER is the NETWORK class, which is what a router or radio becomes when you enroll it.",
				"The class is deliberately the most restricted thing on the mesh. None of those restrictions apply to YOU. Your own reach comes from your tier, exactly like any other admin. If you read a rule below and it sounds like it limits what you personally can see or do, you are reading a rule about a switch."}},
			{Heading: "What a network admin actually is", Paras: []string{
				"It is not a standalone role. You hold the IDENTITY-ADMIN tier, plus a group naming which site's gear you may enroll (ADR 1113). Two dimensions the identity grammar already has — role and site — rather than a new role invented for the purpose.",
				"The practical consequence, and the part most people miss: everything an identity admin does, you do. Invites, sponsorship within your ceiling, working out which credential somebody is actually missing. Enrolling transport gear is what the site group ADDS; it is not what replaces the rest."}},
			{Heading: "Your sites are a separate question from your tier", Paras: []string{
				"Which sites you cover comes from the site group or groups you hold. That is commonly one site, but it can be several, and it can be fleet-wide. Being a network admin somewhere does not make you one everywhere, and covering more than one site needs no different role.",
				"Whatever your scope, it never changes the CLASS. More sites means more places you may enroll transport gear. It never means enrolling something that is not transport gear."}},
			{Heading: "What belongs in the NETWORK class", Paras: []string{
				"Routers, switches and radios carrying the IP underlay the mesh runs over. Not servers, and not anything that publishes data."}},
			{Heading: "Why the class is so narrow — about the DEVICE", Paras: []string{
				"A bearer device is the most physically exposed peer on the mesh: it often sits in a rack or on a mast somebody else can walk up to. So the class is asymmetric on purpose.",
				"Admins and infrastructure can reach a network device on any port. The device itself may only initiate toward management-plane services — web, DNS, time, logging, authentication and traps. It gets no fabric ports, no lateral reach to other peers, and no peer-to-peer connections.",
				"Read that list as the device's leash, not yours. A switch cannot wander the mesh; you are not similarly constrained."}},
			{Heading: "Diagnosing is not limited to your own sites", Paras: []string{
				"Your tier opens the relay-health dashboard, which means you can look at how transport is behaving across the goatnet — not only at gear you happen to have enrolled. When somebody reports an outage anywhere, you are one of the few people equipped to say whether the network is genuinely at fault."},
				Links: []Link{{Label: "Open the relay-health dashboard", Key: "relayhealth"}, {Label: "Open service status", Key: "isitup"}}},
			{Heading: "Widening a device is a decision, never a shortcut", Paras: []string{
				"Granting a bearer device fabric access is separate and deliberate. If a device appears to need it, raise it rather than adding it during enrolment — that would turn the most exposed peer on the mesh into a publisher."},
				Warn: "A refusal during enrolment is usually the class doing its job, not a misconfiguration. Check what you are enrolling before you reach for a wider permission."},
		},
	},

	"network-health": {
		Key: "network-health", Title: "Is it me, or is it the network?",
		Standfirst: "The diagnostic order for transport problems, and what the relay-health dashboard can and cannot tell you.",
		Audience:   "Admins on call", Essential: false, ReadMin: 5, DoMin: 10,
		Sections: []Section{
			{Heading: "Why relays need their own dashboard",
				Links: []Link{{Label: "Open the relay-health dashboard", Key: "relayhealth"}},
				Paras: []string{
					"Relays are NOT mesh peers. Nothing on the mesh can see them the way it sees a normal node, so a prober reaches their public addresses directly and reports what it finds. That is why relay health is a separate surface rather than another panel on the fabric picture."}},
			{Heading: "Read it as a fleet, not a number", Paras: []string{
				"It reports what each relay is actually doing. Treat a single unhappy relay as a routing question, not an outage — peers fall back, and the fabric is designed to keep working while one path is unavailable."},
				Warn: "Be suspicious of any summary that reports a tidy percentage. Availability across a relay fleet is not one number, and a surface that invents one is telling you less than it appears to."},
			{Heading: "Order of diagnosis", Paras: []string{
				"Start local and widen. Is this device on the mesh at all; is it on the RIGHT mesh; does a second device on the same connection fail the same way; is the service itself reported down; and only then, is a relay path degraded.",
				"Most reports that arrive as \"the network is down\" resolve in the first two steps."}},
			{Heading: "Why the link leaves the mesh",
				Links: []Link{{Label: "Open the relay-health dashboard", Key: "relayhealth"}},
				Paras: []string{
					"The dashboard this guide links to is hosted OFF the mesh, which looks inconsistent next to every other surface here and is the single most useful thing about it. A network-health page reachable only over the mesh is unavailable exactly when the mesh is the thing that broke. This one is still there.",
					"It also watches from outside, which is the vantage that matters: it reaches relay public addresses the way a peer trying to connect would, rather than asking the fleet how it feels."}},
			{Heading: "Who sees what, and why it differs",
				Links: []Link{{Label: "Open the relay-health dashboard", Key: "relayhealth"}, {Label: "Open service status", Key: "isitup"}},
				Paras: []string{
					"Two layers, decided separately. The DOOR is who is admitted at all: on the off-mesh copy that is an explicit list of viewer emails, and being an admin elsewhere does not put you on it. The GRADING is what an admitted viewer receives: platform and identity admins the full document with per-relay detail, everyone else a reduced one carrying every status, latency and capacity number but not the addresses and coordinates.",
					"Withheld means ABSENT, never zeroed — a zero would read as a measurement rather than as something you were not shown.",
					"The off-mesh copy grades everyone down to reduced, admins included, because the public sign-in carries no group information. So the same person can legitimately see more on a mesh copy than on the public one; that asymmetry is chosen, on the principle that an admin seeing less there beats a non-admin seeing more.",
					"When triaging for somebody else, ask what THEY can see rather than assuming it matches your view. And distinguish the two failure shapes: a redirect or refusal is the door, an incomplete-looking page is the grading, and neither is an outage."}},
		},
	},

	"gotchas": {
		Key: "gotchas", Title: "Anti-patterns and traps",
		Standfirst: "Things that look like the fix and are not. Each has already cost somebody a day. Entries are tagged with who they apply to — several are admin-only commands, so treat those as background if you are not an admin.",
		Audience:   "Everyone, admins most of all", Essential: true, ReadMin: 6, DoMin: 0,
		Sections: []Section{
			{Heading: "Rotating a certificate to give somebody a key",
				For:      "Identity and platform admins",
				Scenario: "Somebody says the Portal will not let them in. You know rotation issues a fresh key, so you run it for them, paste the new key into a message, and they are working within a minute.",
				Paras: []string{
					"Rotation re-issues their certificate in place and returns a NEW API key. The previous one stops working — including the copy inside their own bundle, and any copy already pasted into a browser on another machine.",
					"So the next person who follows the documented route and looks in their bundle finds a key that fails. You fixed one person and broke the path for the next, which is why these arrive in clusters."},
				Instead: "Re-send their welcome package — it delivers the key they should already have and invalidates nothing. Rotate only when you mean to rotate: a suspected compromise, a scheduled refresh, an offboarding."},

			{Heading: "Linking a namespace to a person by recognizing the name",
				For:      "Identity and platform admins",
				Scenario: "You meet a fabric slot with no recorded owner — while answering \"whose namespace is this?\", while revoking someone, or while working the backfill. It is called something like `jane-doe`, you know exactly who that is, and filling it in looks like tidying up.",
				Paras: []string{
					"The recorded owner is what revocation acts on. Get it wrong and you have handed one person authority over another person's data — and it will look correct to everyone who reads it afterwards.",
					"A slot with NO owner falls back to matching on the name anyway, which is merely the older behavior. A slot with the WRONG owner is a new failure that nothing will catch."},
				Instead: "Link it only when a source of truth already says who owns it. If nothing does, leave it and ask. Many unowned slots are not people at all — machines, services, demos and org slots correctly have no owner."},

			{Heading: "Reading a missing welcome email as a delivery problem",
				For:      "Identity and platform admins",
				Scenario: "Somebody was onboarded and says no email arrived, so you start looking at mail delivery.",
				Paras: []string{
					"The system refuses to send a welcome package that would arrive incomplete — for example somebody going onto the mesh with no enrollment key yet — and records that it withheld it."},
				Instead: "Check for a withheld send before chasing the mail path. The record says which it was."},

			{Heading: "Creating a login by hand to move faster",
				For:      "Identity and platform admins",
				Scenario: "Onboarding runs through three stages and somebody needs access now, so you create the identity directly in the identity provider. It appears immediately and looks like every other account.",
				Paras: []string{
					"Everything except the login is missing: no mesh membership, no fabric identity, no record of who brought them on. It fails the moment they first try to work, and nothing explains why they are different."},
				Instead: "Use the console. If the rails are too slow for a real situation, report that as a defect in the rails rather than routing around them."},

			{Heading: "Treating \"cannot tell\" as \"no\"",
				For:      "Everyone",
				Scenario: "A surface reports that it cannot attribute something — which device belongs to whom, whether a peer is really offline — and it is tempting to read that as a plain no.",
				Paras: []string{
					"Several surfaces deliberately distinguish \"known absent\" from \"cannot know\", because rounding the second into the first invents a fact."},
				Instead: "Treat it as a question to answer, not an answer."},
			{Heading: "Fixing onboarding by editing onboard.db by hand",
				For:      "Identity and platform admins",
				Scenario: "An onboarding row is wedged — somebody left the system mid-mint or a revoke failed — and poking the console's SQLite file looks faster than learning the rails.",
				Paras: []string{
					"The console's onboarding store (onboard.db) is the source of truth for sponsor and admin decisions. A hand-written UPDATE against it bypasses every guard the rails rely on and leaves no audit of who changed what.",
					"The duplicate-email guard and the admin queue both assume the proposal state machine is consistent. Editing rows directly can strand a person in a state that refuses new invites and never appears in the UI, and it is almost impossible to reconstruct later what was done."},
				Instead: "Use the console's offboard action on the proposal instead. It marks the row offboarded with the operator recorded in audit, frees the duplicate-email guard for a fresh invite, and keeps the sponsor/approver history intact."},

			{Heading: "Assuming `E_BUNDLE_INVALID` means a bad bundle",
				For:      "Everyone",
				Scenario: "`goat profile init <id>-goat-cli.cbor` fails with `error: bundle signature or expiry verification failed ... wrong CA, tampered payload, or canonical-form mismatch`. The natural read is a corrupt file, so you re-download it or ask for a fresh one — and it fails the same way again.",
				Paras: []string{
					"Every goat-cli release through 0.4.44 compiles in a small, closed list of CA keys. Anything older than 0.4.44 recognizes exactly one goatnet; 0.4.44 widened that to four, and plenty of goatnets are still not among them. A completely valid, uncorrupted bundle from a goatnet outside that binary's list fails this exact check, every time. Re-downloading or re-minting it changes nothing — the bundle was never the problem.",
					"`goat ca` not existing as a subcommand at all is the tell: the binary predates the fix, which compiles in no CA at all and trusts only what you explicitly add."},
				Instead: "Update first: `GOAT_GET_ENDPOINT=https://get.<site>.<zone> goat update` — it reads that variable before it ever needs a working profile, so it updates even on a machine where `profile init` has never succeeded. If goat is in `/usr/local/bin` (a root-owned folder), run it under sudo and carry the variable through with env, because sudo drops it otherwise: `sudo env GOAT_GET_ENDPOINT=https://get.<site>.<zone> /usr/local/bin/goat update`. Confirm `goat ca ls` is now a recognized command, then retry `profile init` on the SAME file."},
		},
	},

	"windows": {
		Key: "windows", Title: "Windows: snags and fixes",
		Standfirst: "The mesh client and the goat CLI both run on Windows, and most Windows reports come down to a dozen repeat snags. Each entry opens with what you see, then the fix. Where the problem is NetBird's own and NetBird documents it well, the entry says which page and links it rather than restating it. Commands run in an administrator Command Prompt or an administrator PowerShell; where the two differ, both forms are given.",
		Audience:   "Anyone on Windows", Essential: false, ReadMin: 6, DoMin: 10,
		Sections: []Section{
			{Heading: "First, the commands that answer most questions",
				Paras: []string{
					"Run these as administrator: press the Windows key, type cmd (or powershell), right-click the result, choose Run as administrator. `netbird status -d` names the management server you are on, says whether each peer is P2P or Relayed, and lists the nameservers it installed. `sc query netbird` says whether the background service is running at all. The debug bundle is what to send an admin: one zip under C:\\Windows\\Temp with status, routes, config and logs, addresses anonymised."},
				Pre:   "netbird status -d\nsc query netbird\nnet stop netbird && net start netbird\nnetbird debug bundle --anonymize --system-info",
				Links: []Link{{Label: "NetBird: reading status and collecting a bundle", URL: "https://docs.netbird.io/help/troubleshooting-client"}}},

			{Heading: "The tray app asks \"Cloud or Self-hosted\" and opens a browser login",
				Scenario: "You installed the client from the download page, a NetBird icon appeared in the system tray, and a welcome dialog offers NetBird Cloud or Self-hosted and then wants to sign you in through a browser.",
				Paras: []string{
					"That dialog is the desktop app's sign-up flow for NetBird's public service. Choosing Cloud and signing in puts this machine on NetBird's public network, not on this goatnet — and the tray then says Connected, which reads as success. Your goatnet never uses that login: enrolment is one command with the management URL and the one-time key from your welcome package."},
				Instead: "Close the dialog without signing in. Run the enrolment line from your welcome package — it names --profile <goatnet>, so it always lands in a profile dedicated to this goatnet rather than reusing (and silently overwriting) whichever one is currently active. Then confirm the Management line in `netbird status -d` names YOUR goatnet's server.",
				Pre:     "netbird up --profile <goatnet> --management-url <your management URL> --setup-key <your key>\nnetbird status -d",
				Warn:    "\"Connected\" in the tray or in `netbird status` is true for ANY network. Only the Management line in `netbird status -d` says which one."},

			{Heading: "netbird up says \"Already connected\" and nothing changes",
				Scenario: "You ran the enrolment line, it printed Already connected, and `netbird status -d` still names a different management server or no peers.",
				Paras: []string{
					"A BARE `netbird up` against a new server while a session is already up is silently ignored. Windows shows this most often because the desktop app connected first, in the background, before you opened a terminal.",
					"The enrolment line from your welcome package is not bare, though — it names --profile <goatnet>, which forces the daemon to drop the current session before reconnecting instead of silently ignoring the request. Re-running it is always safe: it switches to the existing profile rather than creating a second one."},
				Instead: "Re-run the enrolment line from your welcome package, then check the Management line.",
				Pre:     "netbird up --profile <goatnet> --management-url <your management URL> --setup-key <your key>\nnetbird status -d"},

			{Heading: "'netbird' is not recognized as an internal or external command",
				Paras: []string{
					"A window that was open when you installed keeps its old PATH. Close it and open a new administrator window; until then run the client by its full path."},
				Pre: "\"C:\\Program Files\\NetBird\\netbird.exe\" status -d"},

			{Heading: "failed to connect to daemon, or the service starts and stops again",
				Scenario: "Any `netbird` command fails with connection refused or context deadline exceeded; or the service will not stay up and the log says failed creating tunnel interface wt0.",
				Paras: []string{
					"Refused means the service is not running. Timed out means it is running but wedged — a known Windows state after long sleep or a network change, which a service restart does not always clear; a reboot does. The tunnel-interface failure is a stale wintun driver entry after a Windows update, cleared by deleting the entry so the client re-registers it.",
					"NetBird's Windows page covers the service, the driver, the log location and the debug switches; it is the reference for anything beyond the three commands here."},
				Instead: "Check the service, restart it, and if the CLI still times out, reboot. For the wt0 failure run the three driver lines as administrator.",
				Pre:     "sc query netbird\nnet stop netbird && net start netbird\n\nrem stale wintun driver only:\nsc.exe delete wintun\nnet stop netbird\nnet start netbird",
				Links:   []Link{{Label: "NetBird: the Windows client", URL: "https://docs.netbird.io/help/troubleshooting-client/windows"}}},

			{Heading: "Connected, but nothing loads by name — the IP works",
				Scenario: "`netbird status -d` shows peers Connected and an address such as 100.64.x.x answers, but the login page and the surfaces will not resolve.",
				Paras: []string{
					"On Windows the client resolves mesh names through a Name Resolution Policy Table rule. A Group Policy that also manages name resolution (typical on a company-managed laptop), or a stale policy container left by an older rule, silences that rule while `netbird status -d` still lists the nameserver as available. The two PowerShell cmdlets below tell them apart: a rule that is written but not effective is being overridden.",
					"The automatic name fallback other platforms carry is not built for Windows, so there is no second resolver here — until the rule is effective, reach the surfaces by mesh IP. NetBird's DNS page has the registry path, the Group Policy remedy and the after-disconnect reset."},
				Instead: "Flush the cache and retry. Written but not effective on a company laptop is one for its IT (clear the stale policy container or `gpupdate /force`); on your own machine delete the empty container and reconnect.",
				Pre:     "ipconfig /flushdns\n\n# PowerShell:\nGet-DnsClientNrptRule\nGet-DnsClientNrptPolicy -Effective",
				Links:   []Link{{Label: "NetBird: DNS troubleshooting", URL: "https://docs.netbird.io/manage/dns/troubleshooting"}},
				Warn:    "Do not \"fix\" this with `netbird up --disable-dns` or by editing the adapter's DNS servers by hand. Disabling DNS turns every mesh name off, and a hand edit is overwritten on the next reconnect."},

			{Heading: "After sleep or a lid-close nothing internal loads",
				Paras: []string{
					"The client sometimes keeps a dead session across sleep, and sometimes leaves its resolver (an address ending .255.254) on your real adapter. Down then up reconnects with the stored enrolment — no key needed. If names still fail and `ipconfig /all` shows that address on your Wi-Fi or Ethernet adapter after `netbird down`, the DNS page's after-disconnect reset clears it."},
				Pre:   "netbird down\nnetbird up",
				Links: []Link{{Label: "NetBird: DNS troubleshooting", URL: "https://docs.netbird.io/manage/dns/troubleshooting"}}},

			{Heading: "Everything works but slowly, and status says Relayed for every peer",
				Paras: []string{
					"Relayed is a working connection through the relay fleet, not a fault. On Windows the usual reason direct connections fail is Windows Defender Firewall dropping the incoming WireGuard handshake on the physical adapter. One inbound program rule for the client fixes it; never turn the firewall off. NetBird's ports page lists what the client needs and how to verify the rule."},
				Pre:   "netsh advfirewall firewall add rule name=\"NetBird P2P\" dir=in action=allow protocol=UDP program=\"C:\\Program Files\\NetBird\\netbird.exe\"\nnetbird down && netbird up\nnetbird status -d",
				Links: []Link{{Label: "NetBird: ports and firewalls", URL: "https://docs.netbird.io/about-netbird/ports-and-firewalls"}}},

			{Heading: "Another VPN or Tailscale is installed",
				Paras: []string{
					"Tailscale and the mesh both use the 100.64.0.0/10 range, so two clients claim the same routes; corporate VPN and security agents rewrite routes and proxy settings while running. Disconnect the other client while you work on the mesh and tell your admin which product it is. Running both at once is not a supported state on Windows."}},

			{Heading: "Windows protected your PC — when running goat.exe",
				Scenario: "You downloaded goat-windows-amd64.exe by clicking the link on the get page, ran it, and SmartScreen refused.",
				Paras: []string{
					"A browser download carries a Mark of the Web, and the goat binary is not Authenticode-signed, so SmartScreen has no reputation to check and blocks it. A file fetched with the curl or iwr line on the get page carries no mark."},
				Instead: "Fetch it with the command line from the get page, or unblock the file you have: right-click goat.exe, Properties, tick Unblock, OK. \"More info, Run anyway\" on the dialog does the same for that one file.",
				Links:   []Link{{Label: "Open the get page", Key: "get"}},
				Pre:     "rem Command Prompt\ncurl -fsSL https://<your get host>/goat-windows-amd64.exe -o %USERPROFILE%\\goat.exe\n%USERPROFILE%\\goat.exe --version\n\n# PowerShell\niwr https://<your get host>/goat-windows-amd64.exe -OutFile $env:USERPROFILE\\goat.exe\n& $env:USERPROFILE\\goat.exe --version"},

			{Heading: "'goat' is not recognized",
				Paras: []string{
					"goat.exe is wherever you saved it, usually your user folder, and that folder is not on PATH. Add it to your user PATH once (Settings, search \"environment variables\", edit Path), then close every terminal window and open a new one. Until then, run it by path."},
				Pre: "rem Command Prompt\n%USERPROFILE%\\goat.exe doctor\n\n# PowerShell\n& $env:USERPROFILE\\goat.exe doctor"},

			{Heading: "goat profile init: bundle signature or expiry verification failed — wrong CA",
				Scenario: "`goat ca add` is refused as an unrecognized command, or `goat profile init` rejects a bundle you just downloaded.",
				Paras: []string{
					"A goat binary older than 0.5.0 has no `goat ca` step and trusts only a short built-in list of goatnets, so it rejects a perfectly valid bundle from anywhere else with that message. The bundle is fine; the binary is stale."},
				Instead: "Update the binary first, then retry `profile init` on the SAME .cbor. The get host is the get page's address.",
				Pre:     "rem Command Prompt\nset \"GOAT_GET_ENDPOINT=https://<your get host>\" && goat update\n\n# PowerShell\n$env:GOAT_GET_ENDPOINT=\"https://<your get host>\"; goat update\n\nrem then, in either shell:\ngoat ca ls\ngoat ca add <id>-goat-cli-ca.pem\ngoat profile init <id>-goat-cli.cbor",
				Links:   []Link{{Label: "Open the get page", Key: "get"}}},

			{Heading: "goat client ... says it is not available on this platform",
				Paras: []string{
					"The goat client wrapper that manages the mesh tunnel on macOS and Linux is not built for Windows. On Windows the NetBird client stays in charge of the tunnel: keep using `netbird up`, `netbird down` and `netbird status -d`. Nothing else in goat is affected."}},

			{Heading: "Installed the client inside WSL instead of Windows",
				Scenario: "You ran the Linux install line inside Ubuntu on WSL. WSL says Connected, Windows browsers and goat.exe see nothing, and your key shows a device you did not mean to enroll.",
				Paras: []string{
					"WSL is a separate machine as far as the mesh is concerned: it spent one of your key's device slots, put itself on the mesh, and left Windows off it."},
				Instead: "Inside WSL run `netbird down` and uninstall it there. Install the WINDOWS client from the download page and enroll it from an administrator window. Tell your admin the WSL device name so it can be removed from your key's device list.",
				Warn:    "Mesh names inside WSL are resolved by WSL's own resolver, not the Windows policy rule, so a name may not resolve from a WSL shell even with Windows enrolled. Use the mesh IP from WSL, or run the tools on Windows."},

			{Heading: "Reinstalling does not reset anything",
				Paras: []string{
					"Uninstall leaves the client's state in place — `C:\\ProgramData\\Netbird` holds the config and the peer's private key, and `%AppData%\\netbird` the desktop app's — so a reinstall picks up the old server and peer name. The EXE and MSI installers also do not know about each other; reinstall with the same type you used first. NetBird's install page has the installer detail; the two folders are what to delete for a clean start, and a fresh enrolment needs a fresh key from your admin if the old one has been spent."},
				Pre:   "net stop netbird\n\nrem Command Prompt\nrmdir /s /q \"C:\\ProgramData\\Netbird\"\nrmdir /s /q \"%AppData%\\netbird\"\n\n# PowerShell\nRemove-Item -Recurse -Force \"C:\\ProgramData\\Netbird\", \"$env:APPDATA\\netbird\"",
				Links: []Link{{Label: "NetBird: Windows installation", URL: "https://docs.netbird.io/get-started/install/windows"}}},
		},
	},

	"repos": {
		Key: "repos", Title: "Worked examples",
		Standfirst: "Small, runnable references that show the system actually working. Each one encodes constraints that are easy to get subtly wrong, and each is a working starting point you can read, run and take apart.",
		Audience:   "Anyone building on the fabric", Essential: false, ReadMin: 2, DoMin: 0,
	},

	// Hosted utilities are deliberately described as a CLASS and never named:
	// a utility is declared per goatnet as an external service (ADR 1178) and
	// leaves with its declaration, so the only place its name appears is the
	// status page's "Hosted utilities" section. Naming one here would be a
	// footprint a teardown has to visit.
	"utilities": {
		Key: "utilities", Title: "Hosted utilities: useful, not goat",
		Standfirst: "A goatnet sometimes hosts things for the people on it that are not part of goat and not about the goatnet. Here is how to recognize one and what to expect.",
		Audience:   "Everyone", Essential: false, ReadMin: 2, DoMin: 0,
		Sections: []Section{
			{Heading: "Where they are listed", Paras: []string{
				"The service status page has a section called Hosted utilities. Whatever this goatnet is hosting for you right now is a row there, with its address and a line saying what it is: a shared whiteboard for a workshop week, a pad, a poll, an API marketplace or a registry for an exercise. This guide does not list them, because they come and go; the row is the documentation, and where a utility has its own guide the row links to it."},
				Links: []Link{{Label: "Open service status", Key: "isitup"}}},
			{Heading: "What to expect from one", Paras: []string{
				"Reachable only over the mesh, by the name on the status page, with the goatnet's normal certificate. If your mesh client shows Connected and the name does not load, the fault is on your device or the mesh, not the utility — the troubleshooting topic applies.",
				"No separate sign-in unless the row says so. Where the row says mesh-only, being on the mesh is the whole gate: anyone on this goatnet can open it, and it records nothing about you beyond what its own page says.",
				"Not a goatnet service. Nothing else here depends on it; if it is down, nothing else is. A utility that offers an API of its own documents that API itself — the goatnet gives it a name and a door, not a contract."}},
			{Heading: "They may be temporary", Paras: []string{
				"A utility stood up for an event carries a retirement date. When it passes, the utility is withdrawn and its row goes. Anything you want to keep from one — a drawing, a page — export it before the event ends; the utility's own page says how."}},
			{Heading: "Hosting one", For: "Platform admins", Paras: []string{
				"One declaration file per goatnet and one play, and nothing else in core learns the name: the operator topic points at the external-service path. The workload itself lives with its owner, in its own repository."},
				Links: []Link{{Label: "Open service status", Key: "isitup"}}},
		},
	},

	"firewall": {
		Key: "firewall", Title: "Joining from behind a corporate firewall",
		Standfirst: "A device inside a corporate network or cloud VPC can only join if its egress firewall lets it reach this goatnet's management server and relays. This page is what to hand the network team: the ports, the addresses, and the three firewall settings that decide whether it works.",
		Audience:   "Behind a corporate firewall", ReadMin: 5, DoMin: 15,
		Sections: []Section{
			{Heading: "What to ask for",
				Scenario: "Your security team will not open outbound traffic without a list of destinations — typically an F5, Palo Alto or cloud egress firewall in front of a corporate VPC.",
				Paras: []string{
					"Ask for OUTBOUND rules only, from the device's network to the addresses further down this page. Nothing on this goatnet ever connects inward to your network, so no inbound rule is needed.",
					"An IP allowlist is the right request. A client X.509 certificate is not a substitute: nothing at this goatnet's network edge checks one, so it gives your firewall nothing to match on. A device joins with a one-time setup key and a WireGuard key pair, and the person signs in on a page that is only reachable inside the encrypted tunnel. The servers present ordinary publicly trusted TLS certificates, so no private certificate authority has to be trusted either.",
					"The allowlist only lets traffic leave your network. It grants nothing on the goatnet — access there is by identity, not by source address.",
				}},
			{Heading: "Ports and protocols",
				Paras: []string{
					"TCP 443 to the management server — REQUIRED. Enrollment, then a long-lived gRPC (HTTP/2) connection held open while the device is online. Signaling rides the same connection.",
					"TCP on the relay port (33073 on the goatnets that exist today) to every relay — REQUIRED. Relay traffic over WebSocket with TLS, used whenever two devices cannot reach each other directly.",
					"UDP on the same relay port to every relay — strongly recommended. The same relay traffic over QUIC; faster than the TCP relay.",
					"UDP 3478 to the STUN hosts — recommended. Lets a device learn its public address so it can try a direct connection.",
					"TCP 443 and TCP on the relay port are the working minimum. With only those, every connection goes through a relay: everything works, at lower throughput. Direct device-to-device connections additionally need outbound UDP to other participants' public addresses, which change and cannot be listed; without it traffic falls back to the relays automatically.",
				}},
			{ID: "egress-addresses"},
			{Heading: "Firewall settings that decide whether it works",
				Paras: []string{
					"Exempt these addresses from TLS inspection (SSL break-and-inspect). The management connection is a long-lived HTTP/2 stream and the relay is WebSocket on a non-standard port; an inspecting proxy commonly breaks both. QUIC and STUN are UDP and cannot be inspected, so they are either passed or dropped.",
					"Do not apply a short idle timeout to TCP 443 connections to the management server. The connection is held open and quiet for long periods by design; a firewall that reaps it makes the device drop and reconnect repeatedly.",
					"Let the device resolve the hostnames in public DNS. The client dials the management server and relays by name, not by address.",
				},
				Warn: "Addresses can change when a relay server is rebuilt. Re-check the list on this page, or with dig, before re-using an old firewall request."},
			{Heading: "Installing the client",
				Paras: []string{
					"Installation needs outbound TCP 443 to the NetBird download hosts, pkgs.netbird.io and netbird.io, at install time only. They are served from a CDN whose addresses change, so allow them by hostname (FQDN), or install from a machine outside the restricted network.",
				}},
			{Heading: "Check it before you enroll",
				Paras: []string{
					"From a host inside the restricted network, confirm names resolve and each relay answers on TCP 443 and the relay port. Then enroll, and read the status: Management and Signal should say Connected, and each peer should show P2P (direct) or Relayed — both are working states. Relayed everywhere means UDP is blocked, which works but is slower.",
				},
				Pre: "dig +short <management hostname>\nnc -zv <relay address> 443\nnc -zv <relay address> 33073\n# after enrolling:\nnetbird status -d"},
			{Heading: "If it still will not connect", For: "Sponsors and admins",
				Paras: []string{
					"This guide is only reachable on the mesh, so a person stuck behind a firewall cannot open it. Send them this page as a PDF (print it from the browser) or the operations document it is written from, before their setup key's clock starts.",
					"If management connects but no peer ever leaves Connecting, the relay port is blocked or inspected. If nothing connects at all, TCP 443 or DNS is the problem. The relay-health dashboard tells you whether the relays themselves are up.",
				},
				Links: []Link{{Label: "Open the relay-health dashboard", Key: "relayhealth"}}},
		},
	},

	"troubleshooting": {
		Key: "troubleshooting", Title: "When something will not load",
		Standfirst: "Almost every \"it's broken\" is the mesh. Work down this list before reporting a fault.",
		Audience:   "Everyone", Essential: true, ReadMin: 4, DoMin: 5,
		Sections: []Section{
			{Heading: "The login link from your welcome email does not open",
				Paras: []string{
					"The login link, the identity console and /fabric-credential exist only on the mesh: their names have no public address. Open them on a device where NetBird is connected. If the only device you enrolled is a server, the link will not open on the workstation where you read the email — enroll that workstation too, with the same setup key (it enrolls more than one device), then open the link there.",
					"Check on the device you are browsing from: `netbird status -d` must say Management: Connected."},
				Pre: "netbird status -d"},
			{Heading: "Connected, but 0 peers — the login link does not open",
				Paras: []string{
					"`netbird status -d` says Management: Connected, but the peer count is 0 connected, every peer stays at Connecting, and the relays report `context deadline exceeded`. The device has registered (that uses ordinary HTTPS on TCP 443), but your site network blocks the ports devices use to reach each other, so nothing on the mesh — the login link, this guide, the name server — is reachable from it.",
					"This is the site network, not your account, and every device on the same network behaves the same way. Hand your network team the firewall page in this guide (or the public network-requirements page linked from your invite): it lists the relay host names and the TCP/UDP relay port and UDP 3478 to allow outbound. To confirm in five minutes, connect the device to a phone hotspot instead and check the peer count again."},
				Pre: "netbird status -d\nnc -zv <relay host> <relay port>"},
			{Heading: "1. Is the mesh connected?", Paras: []string{"Run `netbird status -d`. If it is disconnected, nothing internal resolves — including this page.",
				"On Windows, the same check plus the dozen snags that follow it are on the Windows page in this guide: the tray app's Cloud login, a wedged service, stale drivers after an update, names that stop resolving after sleep, and the goat CLI's own Windows quirks."},
				Pre: "netbird status -d"},
			{Heading: "2. Are you on the RIGHT mesh?", Paras: []string{
				"The status output names the management server. If it names a different one you are on another network. Re-run the enrolment line from your welcome package — it names --profile <goatnet>, so unlike a bare `netbird up` it forces the daemon to drop the current session first instead of silently ignoring the request, and it always lands in a profile dedicated to this goatnet rather than reusing (and overwriting) whichever one is currently active.",
				"--profile needs NetBird 0.52 or later, and 0.73.0 refuses a profile that does not exist yet. If the command answers `unknown flag: --profile` or `profile \"<goatnet>\" not found`, update the NetBird client (`netbird version` shows which one you have), then run it again."},
				Pre:    "sudo netbird up --profile <goatnet> --management-url <your management URL> --setup-key <your key>",
				WinPre: "netbird up --profile <goatnet> --management-url <your management URL> --setup-key <your key>"},
			{Heading: "3. Is the service actually down?", Paras: []string{"Check the service status page before assuming it is you."}},
			{Heading: "4. Is it you or the network?", Paras: []string{"Try the same address from a second device on the same connection. Both failing points at the network; one failing points at that device."}},
			{Heading: "5. Does a surface refuse you?", Paras: []string{
				"Sign out and back in so a fresh session carries your current access. If it still refuses, your level may not include it — the fix is a level change, not a key."}},
		},
	},
}
