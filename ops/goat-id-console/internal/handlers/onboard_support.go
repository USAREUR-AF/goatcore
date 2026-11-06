package handlers

// onboard_support.go holds the onboarding helpers ported from the data-fabric
// portal (goat-zenoh-portal/internal/api/onboard_*.go) into the identity plane
// per docs/design/onboarding-extraction-plan.md, slice 2b.
//
// It carries: the intake-hints reader (tier presets + defaults, sourced from the
// SAME onboard-intake-form.json the ADR 1061 web form uses), the tier-ordering
// helpers (lowestTier / tierWithin, ADR 1071 sponsorship caps), the escalated-tier
// carve-out (ADR 1062 Amendment 1), the manifest assembler (effective inputs →
// lintable operator manifest), and the JSON in/out helpers whose shape matches
// the portal's exactly (the runner + collector are drop-in re-pointed later).

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io/fs"
	"log/slog"
	"net/http"
	"regexp"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/orgstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/web"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// --- JSON in/out (portal-parity shapes) --------------------------------

// onboardJSON writes a JSON body with the given status. Mirror of the portal's
// writeJSON so the machine-token + public onboarding responses are byte-shaped
// identically for the runner + collector.
func onboardJSON(w http.ResponseWriter, status int, body any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	//nolint:errcheck // HTTP response delivery is best-effort after the handler has selected its result.
	_ = json.NewEncoder(w).Encode(body)
}

type onboardAPIError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
	// Detail is operator-facing diagnostic context (e.g. which proposal blocks
	// a duplicate email). It still travels in the response body, but
	// onboard.html renders only Message, so the page never shows it to a
	// candidate; omitempty keeps every existing refusal byte-identical.
	Detail string `json:"detail,omitempty"`
	// Fields carries per-field validation messages for multi-field refusals
	// (422 validation_failed), keyed by intake field name. Omitted on all
	// existing single-field errors.
	Fields map[string]string `json:"fields,omitempty"`
}

// onboardError writes {"error":{"code","message"}} — the exact envelope the
// portal uses and the collector's onboard.html reads (res.body.error.message).
func onboardError(w http.ResponseWriter, status int, code, msg string) {
	onboardErrorDetail(w, status, code, msg, "")
}

// onboardErrorDetail is onboardError plus an operator-facing detail field.
func onboardErrorDetail(w http.ResponseWriter, status int, code, msg, detail string) {
	onboardJSON(w, status, map[string]onboardAPIError{"error": {Code: code, Message: msg, Detail: detail}})
}

// onboardValidationError writes a 422 validation_failed response carrying a
// per-field error map. The invite remains unconsumed; the caller must return
// before invoking ConsumeInvite.
func onboardValidationError(w http.ResponseWriter, fields map[string]string) {
	if len(fields) == 0 {
		// A refusal with nothing to name is a programming error. Writing
		// nothing would let net/http answer 200 to a failed submit.
		slog.Error("onboard submit: validation refusal carries no fields")
		onboardError(w, http.StatusInternalServerError, "submit_error", "could not submit your details")
		return
	}
	onboardJSON(w, http.StatusUnprocessableEntity, map[string]onboardAPIError{
		"error": {
			Code:    "validation_failed",
			Message: "one or more fields need correction before this request can be reviewed",
			Fields:  fields,
		},
	})
}

// --- env slug constraint (selects ops/operators/<env>/) ----------------

var envPattern = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,30}$`)

// nationPattern mirrors the operator-manifest schema's affiliation.nation
// constraint (^[A-Z]{2,8}$): an ISO 3166-1 alpha-3 code (CAN, USA, SVK) or a
// multinational marker (NATO, EU). Applied at candidate-submit so a bad value
// (e.g. a full country name like "Canada") is rejected there with a clear
// message, not stored and only caught at the admin approve-time re-lint.
var nationPattern = regexp.MustCompile(`^[A-Z]{2,8}$`)

// validNationCode reports whether v is an acceptable nation-like code: an ISO
// 3166-1 alpha-3 or a short multinational marker (NATO, EU). The regex admits
// 2–8 letters; the extra length guard keeps out full country names like
// "UKRAINE", so intake and the review editor refuse the same values.
func validNationCode(v string) bool {
	v = strings.TrimSpace(v)
	if v == "" {
		return false
	}
	if !nationPattern.MatchString(v) {
		return false
	}
	return len(v) <= 4
}

// --- escalated-tier carve-out (ADR 1062 Amendment 1) -------------------

// escalatedTierSet is the one hard carve-out: these two backend-admin tiers may
// only be sponsor-offered and are flagged for scrutiny at both review gates.
var escalatedTierSet = map[string]bool{"iam-admin": true, "platform-admin": true}

// isEscalatedTier reports whether a tier id is an escalated admin role.
func isEscalatedTier(tier string) bool {
	return escalatedTierSet[strings.ToLower(strings.TrimSpace(tier))]
}

// affiliationKinds are the selectable affiliation values (grammar taxonomy).
// MUST stay in lock-step with the affiliation.kind enum in the operator-manifest
// schema (operator-manifest.schema.json) and the grammar canon
// (docs/design/identity-grammar.md `aff-kind=<vendor|organic|partner|civil>`).
// `civil` was omitted here after it landed everywhere else (schema, both intake
// forms, help text, grammar SoT), so the served selectable list dropped it — the
// TestAffiliationKindsMatchSchemaEnum parity test now fails closed on this drift.
var affiliationKinds = []string{"vendor", "organic", "partner", "civil"}

// isAffiliationKind reports whether v is one of the four affiliation kinds.
//
// Exists so the INVITE path can refuse a bad value at mint instead of letting
// the lint refuse it at approve, after a candidate has filled in the form and
// when affiliation.kind can no longer be edited (ADR 1137's closed set). See
// F-421. Case-insensitive: the enum is lowercase, and rejecting "Partner"
// would be pedantry rather than safety.
func isAffiliationKind(v string) bool {
	for _, k := range affiliationKinds {
		if strings.EqualFold(strings.TrimSpace(v), k) {
			return true
		}
	}
	return false
}

// coalitionValues are the valid affiliation.alliance_standing values (its
// standing relative to the goatnet's active framework, ADR 1077): member =
// member of the active framework (member release-lane eligible); partner =
// non-member. The console captures an EXPLICIT value only; deriving coalition
// standing from nation + active framework is the registry resolver's job, not
// the intake form's.
var coalitionValues = map[string]bool{"member": true, "partner": true}

// sectorValuesGov are the always-live governmental affiliation.sector values
// (institutional sector — orthogonal to nation and to affiliation.kind).
// intelligence = a national intelligence / security service (the IC), spanning
// military and civilian agencies; maps cleanly to none of the other three.
var sectorValuesGov = map[string]bool{
	"military":         true,
	"law-enforcement":  true,
	"civil-government": true,
	"intelligence":     true,
}

// sectorValuesNongov are the demand-gated non-governmental affiliation.sector
// values (ADR 1077 / ops/identity/grammar.yaml gated_values). Accepted only when
// the goatnet flips ONBOARD_SECTORS_NONGOV_ENABLED; default OFF.
var sectorValuesNongov = map[string]bool{
	"ngo":        true,
	"media":      true,
	"academic":   true,
	"commercial": true,
}

// sectorValuesTarget is the ADR 1151 target set — the UN SNA institutional-sector
// split. It is ALWAYS live and deliberately NOT behind ONBOARD_SECTORS_NONGOV_ENABLED:
// that gate exists to withhold the ADR 1077 non-governmental CLUSTER, and the target
// set is not that cluster, it is the replacement axis. Gating `commercial` here is
// what left a foreign commercial vendor with no correct answer in the first place.
var sectorValuesTarget = map[string]bool{
	"state":         true,
	"commercial":    true,
	"civil-society": true,
}

// govBranchValues is the ADR 1151 affiliation.gov_branch enum. Meaningful only
// when sector is state, which assembleManifest enforces rather than storing a
// branch that contradicts its sector.
var govBranchValues = map[string]bool{
	"military":        true,
	"law-enforcement": true,
	"intelligence":    true,
	"civil":           true,
}

// sectorAllowed reports whether a sector value is acceptable given the goatnet's
// non-gov gate. The ADR 1151 target values and the ADR 1077 governmental values
// are always allowed; the legacy non-gov cluster only when nongovEnabled is set.
func sectorAllowed(s string, nongovEnabled bool) bool {
	if sectorValuesTarget[s] || sectorValuesGov[s] {
		return true
	}
	return nongovEnabled && sectorValuesNongov[s]
}

// unitCodeSchemes is the affiliation.unit_code.scheme enum (schema-mirrored).
var unitCodeSchemes = map[string]bool{"sovereign": true, "nato-logfas": true, "sap": true}

// --- intake hints (tier presets + defaults) ----------------------------

// intakeHints is the parsed slice of onboard-intake-form.json we need.
type intakeHints struct {
	Defaults map[string]any `json:"defaults"`
	Tiers    []intakeTier   `json:"tiers"`
}

type intakeTier struct {
	ID    string         `json:"id"`
	Label string         `json:"label"`
	Rank  int            `json:"rank"` // privilege order for the sponsorship cap (ADR 1071); higher = more capable
	Set   map[string]any `json:"set"`
}

// loadIntakeHints reads + parses the shared onboard-intake-form.json (the single
// source for tier presets + defaults) from the embedded static FS. The intake
// form json is the SAME file the ADR 1061 web form fetches, so there is one
// mapping, not two (no drift).
func loadIntakeHints() (*intakeHints, error) {
	raw, err := fs.ReadFile(web.FS, "static/admin/onboard-intake-form.json")
	if err != nil {
		return nil, fmt.Errorf("read intake hints: %w", err)
	}
	var h intakeHints
	if err := json.Unmarshal(raw, &h); err != nil {
		return nil, fmt.Errorf("parse intake hints: %w", err)
	}
	return &h, nil
}

// selectableTiers returns the tiers an enrollee may self-select — i.e. every
// non-escalated tier. maxTier, when non-empty, additionally caps the list at the
// minting sponsor's sponsorship level (ADR 1071).
func (h *intakeHints) selectableTiers(maxTier string) []intakeTier {
	out := []intakeTier{}
	for _, t := range h.Tiers {
		if isEscalatedTier(t.ID) {
			continue
		}
		if maxTier != "" && !h.tierWithin(t.ID, maxTier) {
			continue
		}
		out = append(out, intakeTier{ID: t.ID, Label: t.Label, Rank: t.Rank})
	}
	return out
}

// actorTier derives the tier an ADMIN themselves holds, from the Ory groups
// they are carrying, using the same tier definitions the intake form offers.
//
// Deliberately inverted from `set.ory_site.groups` rather than written as a
// second groups->tier table: a hardcoded copy is how the guard test in
// goat-cli's admin.rs came to carry its own list of goatnets and pass green
// while the tables it guarded were wrong. One definition, read two ways.
//
// Returns the HIGHEST-ranked tier whose groups the actor holds in full. An actor
// holding no tier's groups gets ok=false, and the caller must treat that as
// "cannot approve anything", never as tier zero — an unknown ceiling that
// defaults to permissive is the shape of the defect this exists to close.
func (h *intakeHints) actorTier(groups []string) (id string, rank int, ok bool) {
	held := make(map[string]bool, len(groups))
	for _, g := range groups {
		held[strings.ToUpper(strings.TrimSpace(g))] = true
	}
	for _, t := range h.Tiers {
		want := tierOryGroups(t)
		if len(want) == 0 {
			continue // a tier granting no Ory group cannot be evidenced from groups
		}
		all := true
		for _, g := range want {
			if !held[strings.ToUpper(g)] {
				all = false
				break
			}
		}
		if all && (!ok || t.Rank > rank) {
			id, rank, ok = t.ID, t.Rank, true
		}
	}
	return id, rank, ok
}

// tierOryGroups pulls `set["ory_site.groups"]` (dotted) or `set.ory_site.groups`
// (nested) out of a tier preset. Both shapes appear in the committed intake
// form, and reading only one silently yields "this tier grants no groups",
// which actorTier would then skip.
func tierOryGroups(t intakeTier) []string {
	var raw any
	if v, ok := t.Set["ory_site.groups"]; ok {
		raw = v
	} else if os, ok := t.Set["ory_site"].(map[string]any); ok {
		raw = os["groups"]
	}
	list, ok := raw.([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(list))
	for _, g := range list {
		if s, ok := g.(string); ok && strings.TrimSpace(s) != "" {
			out = append(out, s)
		}
	}
	return out
}

// tierRank returns the privilege rank of a tier id (higher = more capable),
// or ok=false for an unknown tier.
func (h *intakeHints) tierRank(tier string) (int, bool) {
	t := strings.ToLower(strings.TrimSpace(tier))
	for _, e := range h.Tiers {
		if strings.ToLower(e.ID) == t {
			return e.Rank, true
		}
	}
	return 0, false
}

// lowestTier returns the least-privileged tier id (minimum rank): the fail-closed
// cap applied to an authorized sponsor whose annotation lacks a recorded max tier.
func (h *intakeHints) lowestTier() string {
	id, best := "", 0
	for _, t := range h.Tiers {
		if id == "" || t.Rank < best {
			id, best = strings.ToLower(t.ID), t.Rank
		}
	}
	return id
}

// tierWithin reports whether tier is at or below maxTier in privilege rank
// (ADR 1071). Unknown tier or unknown max fails closed.
func (h *intakeHints) tierWithin(tier, maxTier string) bool {
	tr, ok := h.tierRank(tier)
	if !ok {
		return false
	}
	mr, ok := h.tierRank(maxTier)
	if !ok {
		return false
	}
	return tr <= mr
}

func (h *intakeHints) tierSet(tier string) (map[string]any, bool) {
	t := strings.ToLower(strings.TrimSpace(tier))
	for _, e := range h.Tiers {
		if strings.ToLower(e.ID) == t {
			return e.Set, true
		}
	}
	return nil, false
}

// --- manifest assembly (effective inputs → lintable manifest) ----------

// incompleteManifestError signals the effective inputs can't yet form a complete,
// lintable manifest (a blank-field draft, per minimal-burden). Not a failure —
// the proposal is stored as a draft for admin completion.
type incompleteManifestError struct{ missing []string }

func (e *incompleteManifestError) Error() string {
	return "incomplete manifest: missing " + strings.Join(e.missing, ", ")
}

var handleSlugRe = regexp.MustCompile(`[^a-z0-9-]+`)

// orgIDForIntake is the ORG-derived affiliation id of an intake: the sponsor's
// prebound org first, then the id affiliationIDFromOrg derives from the
// organization the candidate typed (a slug for an ASCII name, a short hash
// for one that cannot slug). Empty when neither was given. Both the manifest
// assembler and the submit-time registry check read this one derivation, so
// the two can never disagree about which id the org registry is asked about.
// There is no handle fallback: intake refuses a submission with no
// organization before the invite is consumed (#257).
func orgIDForIntake(in effectiveInputs) string {
	return firstNonEmpty(strings.TrimSpace(in.AffiliationID), affiliationIDFromOrg(in.Organization))
}

// orgSourceFor reads the same two inputs orgIDForIntake folds and reports WHICH
// of them named the organisation: the sponsor's pinned org, or the candidate's
// typed one. Kept beside that derivation on purpose — a future branch added to
// one and not the other is how the provenance would start lying.
func orgSourceFor(in effectiveInputs) string {
	return orgSourceForPrebound(in.AffiliationID)
}

// orgSourceForPrebound is that same rule over the invite's pinned org alone,
// for callers holding the prebound binding rather than the folded inputs.
func orgSourceForPrebound(preboundOrg string) string {
	if strings.TrimSpace(preboundOrg) != "" {
		return onboardstore.OrgSourceSponsor
	}
	return onboardstore.OrgSourceTyped
}

// slug lowercases + collapses to a handle-safe slug.
func slug(s string) string {
	s = strings.ToLower(strings.TrimSpace(s))
	s = handleSlugRe.ReplaceAllString(s, "-")
	return strings.Trim(s, "-")
}

// affiliationIDMaxLen is the manifest schema's ceiling for affiliation.id
// (operator-manifest.schema.json: ^[a-z0-9][a-z0-9-]{0,62}[a-z0-9]$, so 2 to
// 64 characters). An id derived here must satisfy that shape, or the invite
// is consumed and the proposal dies at the approve-time lint.
const affiliationIDMaxLen = 64

// affiliationIDFromOrg derives a deterministic affiliation id from an
// organisation name. ASCII-friendly names slug to a readable id, cut to the
// schema ceiling. A name whose slug is too short to be an id (no ASCII
// letters or digits, for example a Cyrillic unit name, or a single
// character) falls back to an 8-character SHA-256 hex prefix of the name
// folded by orgstore.NormalizeName, the same fold the registry matcher
// applies, so two spellings the matcher treats as one name hash to one id.
// Empty only when the name carries no letter or digit at all, which the
// intake validator refuses by name.
func affiliationIDFromOrg(org string) string {
	org = strings.TrimSpace(org)
	if org == "" {
		return ""
	}
	s := slug(org)
	if len(s) > affiliationIDMaxLen {
		s = strings.Trim(s[:affiliationIDMaxLen], "-")
	}
	if len(s) >= 2 {
		return s
	}
	folded := orgstore.NormalizeName(org)
	if folded == "" {
		return ""
	}
	sum := sha256.Sum256([]byte(folded))
	return hex.EncodeToString(sum[:])[:8]
}

// deriveHandle makes a best-effort handle from an email local-part.
func deriveHandle(email string) string {
	local, _, ok := strings.Cut(email, "@")
	if !ok {
		return ""
	}
	return slug(local)
}

// effectiveInputs is the resolved (sponsor-locked ⊕ enrollee-filled) set.
type effectiveInputs struct {
	Tier           string
	Affiliation    string // kind: vendor|organic|partner
	AffiliationID  string
	Nation         string // affiliation.nation (optional)
	UnitCode       string // affiliation.unit_code.value — UIC / LOGFAS / SAP-ERP (optional)
	UnitCodeScheme string // affiliation.unit_code.scheme; ""=infer from affiliation kind
	Site           string
	// Goatnet is the goatnet THIS console serves (NOTIFY_GOATNET). It is the
	// fallback for Site, so a manifest never names another goatnet's site in
	// netbird.sites or ory_site.sites.
	Goatnet       string
	Handle        string
	Name          string
	Email         string
	Organization  string
	PrincipalType string // person | system → identity.type (optional)
	// MachineQuota is the machine-enrollment AUTHORIZATION (ADR 1096) — how many
	// OTHER machines this person may enroll as a custodian. Sponsor-set only; 0
	// leaves `machine_quota` unset, which is no grant.
	MachineQuota int
	// UsageLimit is the setup-key CAPACITY — how many of their OWN devices the
	// person may put on the mesh with the key they are issued. 0 leaves
	// `netbird.usage_limit` at the intake-form default (5).
	UsageLimit     int
	Sponsor        string
	GitHubUsername string // identity.github — repo-collaborator grants (optional; normalized to a bare login)
	// Standing is affiliation.alliance_standing (ADR 1148): ALLIANCE standing relative to
	// the active framework — member = an ALLY, partner = EXTERNAL TO THE ALLIANCE.
	// Says nothing about coalition participation: a coalition is an ACTIVATED
	// grouping and a partner may be in none.
	Standing  string // affiliation.alliance_standing (preferred; member|partner, explicit only)
	Coalition string // affiliation.coalition — DEPRECATED alias of Standing, still accepted
	Sector    string // affiliation.sector — institutional sector (optional). ADR 1151 target set (state|commercial|civil-society) and the ADR 1077 governmental cluster are always live; the legacy non-gov cluster stays gated.
	GovBranch string // affiliation.gov_branch — branch of government (ADR 1151). Written only under sector=state.
	// Nationality is the PERSON's own citizenship -> identity.nationality
	// (ADR 1171). Distinct from Nation, which is the ORGANISATION's controlling
	// nation, and never derived from it: the two disagreeing is the normal state
	// for a liaison officer or anyone posted to another country's command.
	Nationality   string
	NongovSectors bool // whether the non-governmental affiliation.sector cluster is accepted (ONBOARD_SECTORS_NONGOV_ENABLED)
	Collab        bool // collab.enabled — include this identity in the cloud collaboration environment (sponsor decision, prebound at invite mint)
	// OperatingSovereign is the nation running THIS goatnet (OPERATING_SOVEREIGN).
	// It is one side of the organic test that derives affiliation.kind for a
	// state-sector org (ADR 1151); blank means the test cannot be answered and a
	// state-sector kind stays unset rather than guessed.
	OperatingSovereign string
}

// assembleManifest builds a valid operator manifest from the effective inputs +
// the intake hints. Returns *incompleteManifestError when a complete manifest
// can't be formed yet (blank tier or handle).
//
// affiliation.kind is NOT in the required set. ADR 1151 deprecated it: the
// schema stopped requiring it, the mint form and the intake page stopped
// asking for it, and it derives from sector + the organic test. Requiring it
// here after that turned every console-minted invite into an incomplete draft
// that neither decision gate would advance.
func assembleManifest(h *intakeHints, in effectiveInputs) (map[string]any, error) {
	var missing []string
	if strings.TrimSpace(in.Tier) == "" {
		missing = append(missing, "tier")
	}
	if strings.TrimSpace(in.Handle) == "" {
		missing = append(missing, "handle")
	}
	if len(missing) > 0 {
		return nil, &incompleteManifestError{missing: missing}
	}
	set, ok := h.tierSet(in.Tier)
	if !ok {
		return nil, fmt.Errorf("unknown tier %q", in.Tier)
	}

	m := map[string]any{}
	// 1. defaults (dotted keys).
	for k, v := range h.Defaults {
		setPath(m, k, v)
	}
	// 2. tier preset expansion (dotted keys); @handle → the real handle.
	for k, v := range set {
		if s, isStr := v.(string); isStr {
			switch s {
			case "@handle":
				v = in.Handle
			case "@email":
				// The candidate flow onboards a person, so a preset-granted
				// sponsorship binds their console login too (ADR 1072).
				v = in.Email
			}
		}
		setPath(m, k, v)
	}
	// netbird.role defaults to 'peer' (the normal member; not the access tier).
	setPath(m, "netbird.role", "peer")

	// 3. identity.
	setPath(m, "identity.handle", in.Handle)
	setPath(m, "identity.email", in.Email)
	setPath(m, "identity.name", firstNonEmpty(in.Name, in.Handle))
	setPath(m, "identity.status", "active")
	setPath(m, "identity.added_by", sponsorAddedBy(in.Sponsor, in.Site))
	setPath(m, "identity.added", time.Now().UTC().Format("2006-01-02"))
	// identity.github (optional) — a bare login for repo-collaborator grants at
	// provision (docs/design/onboarding-github-repo-grants.md). Use the same
	// validator as intake and later edits; an internal caller must not be able to
	// silently persist a malformed login by bypassing the HTTP validation path.
	gh, githubErr := normalizeGitHubLogin(in.GitHubUsername)
	if githubErr != nil {
		return nil, githubErr
	}
	if gh != "" {
		setPath(m, "identity.github", gh)
	}

	// 4. site → netbird.sites + ory_site.sites (required when ory_site.enabled).
	//
	// The fallback is the goatnet this console serves, never a literal site name: a
	// literal would put one goatnet's name into the mesh and Ory site lists of
	// manifests minted on every other goatnet, and a person provisioned from one
	// would land in the wrong NetBird site.
	site := firstNonEmpty(in.Site, in.Goatnet)
	setPath(m, "netbird.sites", []any{site})
	if oe, _ := getPath(m, "ory_site.enabled").(bool); oe {
		setPath(m, "ory_site.sites", []any{site})
	}

	// 5. zenoh trial cleanup: disabled → drop trial; enabled → ensure a
	// trial.vendor prefix (the handle).
	if ze, _ := getPath(m, "zenoh.enabled").(bool); !ze {
		if z, ok := m["zenoh"].(map[string]any); ok {
			delete(z, "trial")
		}
	} else {
		if getPath(m, "zenoh.trial.vendor") == nil {
			setPath(m, "zenoh.trial.vendor", in.Handle)
		}
	}

	// identity.type (person/system) when the enrollee gave one.
	if pt := strings.ToLower(strings.TrimSpace(in.PrincipalType)); pt != "" {
		setPath(m, "identity.type", pt)
	}

	// 6. affiliation (grammar-cutover requires the block; `id` is its required key).
	// The candidate path requires an organisation name when no registered
	// org id is pre-bound; other callers may still assemble from a bare
	// AffiliationID alone. The id is the one derivation the submit-time
	// registry check also reads (orgIDForIntake), and nothing ever reaches
	// for the handle: a person is never an organisation.
	affID := orgIDForIntake(in)
	if affID == "" {
		return nil, &incompleteManifestError{missing: []string{"organization"}}
	}
	setPath(m, "affiliation.id", affID)
	if n := strings.TrimSpace(in.Nation); n != "" {
		setPath(m, "affiliation.nation", n)
	}
	// affiliation.alliance_standing (optional) — coalition standing relative to the
	// goatnet's active framework, captured only as an explicit valid value
	// (member|partner). NOT derived here; deriving standing from nation + the
	// active framework is the registry resolver's job (ADR 1077).
	// ADR 1151 named the field `alliance_standing`. READ both, preferring the new key;
	// WRITE both, so a manifest stays readable by anything still looking for
	// `affiliation.coalition`. That is the ADR 1052 Phase-1 shape -- additive
	// first, flip the emit later, remove last -- and it is why nothing breaks.
	standingIn := in.Standing
	if strings.TrimSpace(standingIn) == "" {
		standingIn = in.Coalition
	}
	if standing := strings.ToLower(strings.TrimSpace(standingIn)); coalitionValues[standing] {
		setPath(m, "affiliation.alliance_standing", standing)
		setPath(m, "affiliation.coalition", standing)
	}
	// affiliation.sector (optional) — institutional sector, captured only as an
	// explicit valid value. The governmental cluster (military|law-enforcement|
	// civil-government|intelligence) is always live; the non-governmental cluster
	// (ngo|media|academic|commercial) is accepted only when this goatnet flips
	// ONBOARD_SECTORS_NONGOV_ENABLED (ADR 1077). A non-gov value submitted while
	// the gate is off is silently dropped, not stamped.
	sector := strings.ToLower(strings.TrimSpace(in.Sector))
	if sectorAllowed(sector, in.NongovSectors) {
		setPath(m, "affiliation.sector", sector)
	} else {
		sector = ""
	}
	// affiliation.kind (deprecated, ADR 1151): an explicit value wins; otherwise
	// it derives from the STAMPED sector and the organic test, and when neither
	// decides it is left out. Downstream readers still key on it (the NetBird
	// ACL matrix, the roster), which is why it is derived wherever it can be
	// rather than simply dropped.
	kind := resolveAffiliationKind(in.Affiliation, sector, in.Nation, in.OperatingSovereign)
	if kind != "" {
		setPath(m, "affiliation.kind", kind)
	}
	// affiliation.gov_branch (optional, ADR 1151) — the branch of government,
	// written ONLY under sector=state. A branch under any other sector is a
	// contradiction, so it is dropped rather than stamped: a commercial company
	// has no branch of government and the manifest must not claim one.
	if b := strings.ToLower(strings.TrimSpace(in.GovBranch)); govBranchValues[b] && sector == "state" {
		setPath(m, "affiliation.gov_branch", b)
	}
	// identity.nationality (optional, ADR 1171) — the PERSON's own citizenship.
	// On `identity`, not inside `affiliation`: the affiliation block describes
	// who the principal BELONGS TO, and this describes the principal. Captured
	// only as an explicit value; absent stays absent and is never inferred from
	// the organisation's nation, which is the inference this exists to end.
	if n := strings.ToUpper(strings.TrimSpace(in.Nationality)); validNationCode(n) {
		setPath(m, "identity.nationality", n)
	}
	if uc := strings.TrimSpace(in.UnitCode); uc != "" {
		// unit_code is a structured object {scheme, value, source}. The enrollee
		// types a bare code, so it's self-asserted. The scheme is the enrollee's
		// pick when valid, else inferred from the affiliation kind.
		scheme := "sovereign"
		if kind == "partner" {
			scheme = "nato-logfas"
		}
		if s := strings.ToLower(strings.TrimSpace(in.UnitCodeScheme)); unitCodeSchemes[s] {
			scheme = s
		}
		setPath(m, "affiliation.unit_code", map[string]any{
			"scheme": scheme,
			"value":  uc,
			"source": "self-asserted",
		})
	}

	// The two machine numbers. They are collected as one field in some paths and
	// as two in others, but they are NOT the same kind of thing and are written
	// from separate inputs on purpose.
	//
	// `machine_quota` is an AUTHORIZATION (ADR 1096) — a custodian holding it may
	// enroll that many OTHER machines, seeded into the portal machine_grants at
	// onboard. Its absence must stay an absence, never a default, and it may only
	// ever come from a sponsor's invite. ADR 1062 §2: capability rides the
	// server-held invite, never the public form.
	//
	// `netbird.usage_limit` is a CAPACITY — how many of their OWN devices the
	// person may put on the mesh with the key they were issued. It carries a
	// default from the `defaults` block of onboard-intake-form.json (applied at
	// step 1 of this assembler), so omitting the number here leaves that default
	// standing rather than falling through to the play's last-resort 1.
	//
	// Why they are separate inputs: writing both from one field, fed by the
	// candidate's own number when the sponsor leaves the invite blank, would let a
	// candidate typing a number into the public form self-grant custodian authority
	// over that many machines, uncapped. Splitting the inputs is what enforces it;
	// a comment saying "sponsor-only" on a field two callers can write does not.
	//
	// Why usage_limit carries a default at all: without one, an intake with no
	// number produces a manifest with no usage_limit, and operator-onboard.yml's
	// `| default(1)` turns that silence into a single-use key. First enrollment
	// consumes it and every later attempt — a second machine, a reinstall, native
	// Windows after a WSL install — fails with "couldn't add peer: setup key is
	// invalid". Nothing displays the limit: identity-card.js only renders the
	// machine row when the value is truthy.
	if in.MachineQuota > 0 {
		setPath(m, "machine_quota", in.MachineQuota)
	}
	if in.UsageLimit > 0 {
		setPath(m, "netbird.usage_limit", in.UsageLimit)
	}

	// 7. organization display name (the legacy `vendor` block, per ADR 1052).
	setPath(m, "vendor.display_name", firstNonEmpty(in.Organization, in.Name, in.Handle))

	// 8. collab: the per-individual cloud-collaboration-environment decision.
	// Always stamped (true|false) so the onboarding record captures the explicit
	// choice, never an ambiguous absence. Loosely coupled + provider-agnostic: this
	// records only the DECISION — delivery is a separate provisioning step (today a
	// Kanidm mint in our own Dex/Kanidm plane; possibly a handoff into an external
	// collaboration env's onboarding flow later). The mesh plane may read this for
	// awareness but never governs it, and nothing enforcing depends on it.
	// See docs/project/collab-onboarding-field.md.
	setPath(m, "collab.enabled", in.Collab)

	return m, nil
}

// setPath sets a dotted path into a nested map, creating intermediate maps.
func setPath(m map[string]any, dotted string, v any) {
	parts := strings.Split(dotted, ".")
	cur := m
	for i, p := range parts {
		if i == len(parts)-1 {
			cur[p] = v
			return
		}
		next, ok := cur[p].(map[string]any)
		if !ok {
			next = map[string]any{}
			cur[p] = next
		}
		cur = next
	}
}

// getPath reads a dotted path from a nested map (nil if absent).
func getPath(m map[string]any, dotted string) any {
	parts := strings.Split(dotted, ".")
	var cur any = m
	for _, p := range parts {
		cm, ok := cur.(map[string]any)
		if !ok {
			return nil
		}
		cur = cm[p]
	}
	return cur
}

// sponsorAddedBy returns an email-formatted identity.added_by from the sponsor.
func sponsorAddedBy(sponsor, site string) string {
	s := strings.TrimSpace(sponsor)
	if strings.Contains(s, "@") && strings.Contains(s, ".") {
		return s
	}
	sp := slug(s)
	if sp == "" {
		sp = "operator"
	}
	// Same class as the site default above: a literal goatnet name here would land
	// in an address minted on any goatnet. `invite` is a neutral last resort — it
	// names no environment, so it cannot be mistaken for one.
	return sp + "@" + firstNonEmpty(slug(site), "invite") + ".invite.local"
}

func firstNonEmpty(vs ...string) string {
	for _, v := range vs {
		if strings.TrimSpace(v) != "" {
			return v
		}
	}
	return ""
}

// manifestHandleEmail pulls identity.handle + identity.email from the tree.
func manifestHandleEmail(tree map[string]any) (string, string) {
	id, ok := tree["identity"].(map[string]any)
	if !ok {
		return "", ""
	}
	handle, _ := id["handle"].(string)
	email, _ := id["email"].(string)
	return handle, email
}
