package api

import (
	"context"
	"crypto/rand"
	"embed"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io/fs"
	"log/slog"
	"net/http"
	"strings"
	"sync/atomic"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/activity"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/kratosadmin"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/netbirdkeys"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/netbirdpoll"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/notify"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/schemastore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/zenohproxy"
)

// Server owns the HTTP surface. Constructed once at startup; handlers
// pull dependencies off the struct.
type Server struct {
	cfg       *config.Config
	reg       *registry.Registry
	zenoh     *zenohproxy.Proxy
	minter    *certmint.Minter
	aclRender *acl.Renderer
	schemas   *schemastore.Store  // nil-able; routes 503 when unset
	kratos    *kratosadmin.Client // nil-able; admin/vendor-activity hides the identities column when nil
	// humans resolves an Ory identity by UUID or (as an alias) login email for
	// the data-roles resolver; nil when kratos is unset (the human paths then
	// fail closed). Kratos-backed in production; overridable in tests.
	humans     humanResolver
	nb         *netbirdpoll.Client // shared, cache-backed mgmt client; the background poller warms its cache
	nbKeys     *netbirdkeys.Client // ADR 1116 §9 machine setup-key mint; disabled unless mesh-key-mint-enabled + prefix + creds
	notifier   *notify.Notifier    // candidate stage-notification channel (increment 3); disabled → every call no-ops
	// activity answers "how many declared topics has this org published
	// under" for the swimlane. Never nil — an unconfigured goatnet gets
	// activity.Disabled, which answers "unavailable" rather than zero, so the
	// surface cannot present a fabricated count as a real one. See F-537 and
	// internal/activity.
	activity activity.Source
	staticFS   fs.FS
	startedAt  time.Time
	reqCounter atomic.Uint64

	// legacyVendorHits counts requests that arrive on a deprecated /api/vendors*
	// route (ADR 1052 Phase 1, vendor→org machine rename). Surfaced on
	// /api/health so Phase-4 removal of the vendor surface can be gated on the
	// counter reading zero over a window — the telemetry gate, not a calendar.
	legacyVendorHits atomic.Uint64

	// Per-actor throughput-envelope declare step (ADR 1045). The per-class
	// rough-default baselines + the class a new actor's default is seeded
	// from, resolved once from config at construction.
	envBaselines    registry.BaselineSet
	envDefaultClass string

	// rosters memoises the parsed data roster (cfg.RosterPath) for the
	// data-access routes; it re-parses when the file changes on disk, so
	// roster edits still apply on the next request (access.DataRosterCache).
	rosters access.DataRosterCache
}

// New builds a Server with the supplied dependencies. staticFS holds the
// embedded frontend (typically web/static/ rooted). schemas may be nil
// (Block 85 schema upload endpoints will 503 until it's wired). kratos may
// be nil (admin/vendor-activity hides the per-vendor identity-roster
// column and notes the gap inline).
func New(cfg *config.Config, reg *registry.Registry, zenoh *zenohproxy.Proxy, minter *certmint.Minter, aclR *acl.Renderer, schemas *schemastore.Store, kratos *kratosadmin.Client, staticFS fs.FS) *Server {
	// Resolve the throughput-envelope baselines once (ADR 1045). A malformed
	// PORTAL_ENVELOPE_BASELINES override falls back to the built-in rough
	// placeholders rather than mis-seeding every actor — envelopes are
	// advisory and never gate registration.
	baselines, err := registry.ParseBaselines(cfg.EnvelopeBaselinesJSON)
	if err != nil {
		slog.Warn("invalid PORTAL_ENVELOPE_BASELINES; using built-in defaults", "err", err)
		baselines = registry.DefaultBaselines()
	}
	defaultClass := cfg.EnvelopeDefaultClass
	if strings.TrimSpace(defaultClass) == "" {
		defaultClass = registry.DefaultBaselineClass
	}
	// The data-roles resolver's human paths need Kratos to map identity UUID ↔
	// login email; wire the adapter only when Kratos is configured (nil leaves
	// those paths failing closed with an honest note).
	var humans humanResolver
	if kratos != nil {
		humans = kratosHumanResolver{c: kratos}
	}
	// Activity source. Unset URL is the ordinary case on a goatnet
	// with no Prometheus, and it is not an error: Disabled answers
	// "unavailable" for every org, which the swimlane renders as unknown. A
	// MALFORMED url is different — that is a config mistake someone should
	// see, so it is logged loudly and then also degrades to Disabled rather
	// than refusing to start the portal over a display field.
	var act activity.Source = activity.Disabled{}
	if u := strings.TrimSpace(cfg.PrometheusURL); u != "" {
		if p, err := activity.NewPrometheus(u, 3*time.Second); err != nil {
			slog.Error("invalid PORTAL_PROMETHEUS_URL; swimlane activity counts will read as unavailable",
				"url", u, "err", err)
		} else {
			// 30 s memo: the swimlane is refreshed by several operators at
			// once and the number is an activity reading, not a fact of
			// record, so one upstream query a half-minute is ample.
			act = activity.NewCached(p, 30*time.Second)
			slog.Info("swimlane activity counts sourced from prometheus", "url", u)
		}
	}
	return &Server{
		activity:        act,
		cfg:             cfg,
		reg:             reg,
		zenoh:           zenoh,
		minter:          minter,
		aclRender:       aclR,
		schemas:         schemas,
		kratos:          kratos,
		humans:          humans,
		nb:              netbirdpoll.New(cfg.NetbirdMgmtURL, cfg.NetbirdAPIToken),
		nbKeys:          netbirdkeys.New(cfg.NetbirdMgmtURL, cfg.NetbirdAPIToken, cfg.MeshSitePrefix),
		notifier:        notify.New(cfg.NotifyConsoleURL, cfg.NotifyToken),
		staticFS:        staticFS,
		startedAt:       time.Now(),
		envBaselines:    baselines,
		envDefaultClass: defaultClass,
	}
}

// NetbirdClient is the shared, cache-backed netbird-mgmt client. main.go runs
// the background poller on this same instance so its periodic fetches keep the
// access-COP peer cache warm (see netbirdpoll.PeersForAccess) — the synchronous
// access path then never serializes a slow mgmt call onto every request.
func (s *Server) NetbirdClient() *netbirdpoll.Client { return s.nb }

// Routes returns the http.ServeMux wired with every endpoint. Use
// http.Server-side timeouts at the caller per Block 50A.1.
func (s *Server) Routes() http.Handler {
	// Recovery + access-log wrapper, outside gzip so the access log records the
	// real status. gzipMiddleware covers the generated API payloads; the static
	// handler carries its own precomputed encoding and is skipped by the
	// already-has-Content-Encoding check. conditionalMiddleware sits inside
	// gzip so its ETag covers the identity body (cache.go).
	return s.middleware(gzipMiddleware(conditionalMiddleware(s.mux())))
}

// mux is the bare route table. Split from Routes so the OpenAPI contract test
// can resolve every documented method and path against it.
func (s *Server) mux() *http.ServeMux {
	mux := http.NewServeMux()

	// Liveness / readiness.
	mux.HandleFunc("GET /api/health", revalidated(s.handleHealth))

	// Block 94 cross-surface deep-link contract — sibling-surface
	// base URLs the static catalog JS reads on page load to render
	// `diagnose ↗` + `tail ↗` buttons. Public; no secrets cross
	// this surface (the URLs are mesh-DNS hostnames already visible
	// in DNS). See docs/design/zenoh-surface-deep-links.md.
	mux.HandleFunc("GET /api/siblings", s.handleSiblings)

	// Display lexicon (docs/design/portal-lexicon.md). Public, no auth — it is
	// branding text the frontend JS reads on page load to label runtime-
	// generated UI (the static HTML is already substituted server-side).
	mux.HandleFunc("GET /api/branding", s.handleBranding)

	// Fabric self-description (Peering tab; PR-2). Public, no auth — the
	// whole point is that prospective partners can read this to discover
	// how to connect, what's reserved, and what's already taken. Same JSON
	// drives the static /peering/our-config.html renderer.
	mux.HandleFunc("GET /api/fabric/config", s.handleFabricConfig)
	// Public root-CA bundle. Partners need this BEFORE they're a vendor
	// (to trust-anchor our router cert pre-mint). Served from disk; the
	// path is in portal config.
	mux.HandleFunc("GET /peering/root-ca.pem", s.handleRootCABundle)
	// Partner-side zenohd.json5 generator (PR-4). Public, no auth — the
	// generated config carries no secrets; cert paths are placeholders
	// the partner edits to match their bind-mounts.
	mux.HandleFunc("GET /api/fabric/zenohd-config", s.handleFabricZenohdConfig)
	// Interactive config workbench (Peering tab). POST a paste, get back
	// structured findings. Public; same posture as zenohd-config (no
	// secrets cross this surface — the user's content is pre-mint draft).
	mux.HandleFunc("POST /api/peering/lint", s.handlePeeringLint)
	// Live-troubleshooting surfaces (Peering tab; design at
	// docs/design/portal-live-troubleshooting.md).
	//
	// Surface 1 — log-paste analyzer. Vendor pastes raw publisher stderr,
	// backend pattern-matches against a curated signature catalog.
	mux.HandleFunc("POST /api/peering/diagnose-log", s.handleDiagnoseLog)
	// Surface 2 — connectivity probe result interpreter. Vendor runs the
	// probe script locally (rendered at GET /peering/probe.sh) and pastes
	// the JSON output back; backend returns per-layer findings.
	mux.HandleFunc("POST /api/peering/probe-result", s.handleProbeResult)
	mux.HandleFunc("GET /peering/probe.sh", s.handleProbeScript)

	// Vendors. Every /api/vendors* route is registered together with an
	// /api/orgs* alias via dualRoute (ADR 1052 Phase 1 — vendor→org machine
	// rename, back-compat-first). Both paths hit the same handler and keep the
	// {vendor} path-param name (the param/identifier rename is Phase 2); the
	// legacy /api/vendors* leg is counted for the Phase-4 removal gate.
	s.dualRoute(mux, "GET", "/api/vendors", revalidated(s.AuthMiddleware(true, s.listVendors)))
	// Slot alias map for the display layer (ADR 1039) — slot_id↔human handle.
	mux.HandleFunc("GET /api/aliases", s.AuthMiddleware(true, s.handleListAliases))
	// Per-actor throughput envelopes (ADR 1045): the anchor→expected-band map
	// `goat top` fetches + caches (read like the alias registry); editable by
	// the owning party (or operator) via the scoped PUT below.
	mux.HandleFunc("GET /api/envelopes", revalidated(s.AuthMiddleware(true, s.handleListEnvelopes)))
	// ADR 1096 stage 2 — delegated machine enrollment. AuthMiddleware(required)
	// only: the operator-vs-self-service gate is per node-class inside the handler
	// (infra classes operator-gated; function classes custodian self-service).
	mux.HandleFunc("POST /api/machines/enroll", noStore(s.AuthMiddleware(true, s.enrollMachine)))
	// D7 bulk door: N machines, one custodian, one act — same enrollOne core,
	// N single-use keys, never one reusable key handed around.
	mux.HandleFunc("POST /api/machines/enroll-bulk", noStore(s.AuthMiddleware(true, s.enrollMachinesBulk)))
	// D1 re-mint: fresh one-off mesh key for an already-enrolled machine whose
	// join window lapsed (mint-then-hand-carry). Quota-neutral.
	mux.HandleFunc("POST /api/machines/{anchor}/mesh-key", noStore(s.AuthMiddleware(true, s.machineMeshKey)))
	// ADR 1096 stage 5 — the custody surface (read): a custodian's fleet, or the
	// operator's whole-fleet view. Own-scope for vendor callers (same gate as enroll).
	mux.HandleFunc("GET /api/machines", s.AuthMiddleware(true, s.listMachines))
	// D6a single-machine read with the mesh derivation attached: the
	// machineListItem plus its DERIVED full group names (+ mesh_derivable).
	// This is the read `bundle-create --from-machine` consumes so a gear
	// bundle's acl_groups come from the born-as class, never a hand-typed
	// list. Gate mirrors the mesh-key custody model.
	mux.HandleFunc("GET /api/machines/{anchor}", s.AuthMiddleware(true, s.getMachine))
	// The exit. Mesh-first (ADR 1031): cut the peer + any unconsumed setup key
	// on the mgmt server, THEN free the slot and delete the anchor row. Without
	// it a machine could be created and never retired. Gate mirrors the mesh-key
	// custody model.
	mux.HandleFunc("DELETE /api/machines/{anchor}", s.AuthMiddleware(true, s.machineRevoke))
	// Operator-only quota management: set (create or adjust) a custodian's machine
	// enrollment quota, so quota is manageable after onboarding (not just seeded at
	// mint). Operator gate is inside the handler. Backs `goat machine grant` + the
	// onboarding/permissions console.
	mux.HandleFunc("POST /api/machines/grant", s.AuthMiddleware(true, s.grantMachineQuota))
	mux.HandleFunc("POST /api/machines/affiliation/repair", s.AuthMiddleware(true, s.repairMachineAffiliation))
	// The read half of the line above. ADR 1131: the Herd gear console gates
	// interactive bearer-gear sessions on this entitlement, and until now the
	// only way to read it was to be the portal.
	mux.HandleFunc("GET /api/machines/grant/{custodian}", s.AuthMiddleware(true, s.getMachineGrant))
	s.dualRoute(mux, "POST", "/api/vendors", noStore(s.requireOperator(s.createVendor)))
	s.dualRoute(mux, "PATCH", "/api/vendors/{vendor}", s.requireOperator(s.updateVendor))
	s.dualRoute(mux, "DELETE", "/api/vendors/{vendor}", s.requireOperator(s.deleteVendor))
	// Re-issue a peer's cert in place — same anchor, fresh leaf (rotation, the
	// sibling of revocation; ADR 1031). No slot allocation, no ACL render.
	s.dualRoute(mux, "POST", "/api/vendors/{vendor}/rotate", noStore(s.requireOperator(s.rotateVendor)))
	// Key-only reissue (goat#1193): the lighter sibling of rotate — swaps only
	// the portal api_key, cert untouched. Org-only, no /api/vendors twin: new
	// machine surfaces are additive on the org noun (ADR 1052).
	mux.HandleFunc("POST /api/orgs/{vendor}/rekey", s.requireOperator(s.rekeyVendor))
	// Identity-keyed sibling (goat#320): the console names the PERSON, the
	// portal resolves their slot — the collect trust boundary applied to
	// rotation. Registered on /api/orgs only: new door, no legacy alias.
	mux.HandleFunc("POST /api/orgs/rotate", noStore(s.requireOperator(s.rotateByIdentity)))
	// Sponsorship annotation (ADR 1071): the case-by-case grant/revoke lever
	// for identities that didn't get sponsorship by tier default at onboarding.
	// Org-only, no /api/vendors twin: new machine surfaces are additive on the
	// org noun per ADR 1052 (the vendor-surface ratchet enforces this).
	mux.HandleFunc("POST /api/orgs/{vendor}/sponsorship", s.requireOperator(s.setVendorSponsorship))
	// Mint-on-collect (docs/design/fabric-credential-collection.md): mint a
	// person's fabric identity when they come and collect it, keyed on WHO they
	// are rather than which anchor they name. Operator-gated — goat-id-console
	// authenticates the human through Ory and calls this with its operator
	// bearer, the same delegation X-Goat-Operator-Actor already rests on.
	// Org-only surface, no /api/vendors twin (ADR 1052); the literal segment wins
	// over the {vendor} wildcard routes around it.
	mux.HandleFunc("POST /api/orgs/collect", noStore(s.requireOperator(s.collectFabricCredential)))
	// Sponsorship-by-email resolve (ADR 1072): the id-admin-console asks this
	// before granting a non-ADMINS session sponsor mode. Self-gated inside the
	// handler (operator bearer OR the console↔portal service secret) — the
	// exact literal segment wins over the {vendor} wildcard above.
	mux.HandleFunc("GET /api/orgs/sponsorship", noStore(s.handleSponsorshipLookup))
	// Producer data labels (ADR 1075): read is any authenticated viewer (labels
	// are catalog metadata, ADR 1028); set is the producer itself (vendor-scoped)
	// or an operator — the data-authority owns its own labels. Org-only surface
	// (additive on the org noun, ADR 1052). The deeper {vendor}/labels path never
	// collides with the literal /api/orgs/sponsorship above.
	mux.HandleFunc("GET /api/orgs/{vendor}/labels", s.AuthMiddleware(true, s.handleGetLabels))
	mux.HandleFunc("PUT /api/orgs/{vendor}/labels", s.scopedHandler(s.handleSetLabels))
	// Read-only auth-delegation: resolves the presented key → principal + ADR
	// 1071 sponsorship. goat-id-console (which now owns the onboarding engine)
	// calls this to resolve a participant's portal key at invite-mint — the key
	// store + vendors registry stay the SoT here. Any authenticated principal.
	mux.HandleFunc("GET /api/whoami", noStore(s.AuthMiddleware(true, s.handleWhoami)))

	// Durable audit read (operator-gated) — the id-console audit view merges
	// this with its own ring so non-onboarding portal actions (org mint,
	// sponsorship) stay visible (ADR 0860). Onboarding audit is now
	// console-native + file-durable (AUDIT_FILE_PATH), so this no longer
	// carries onboarding rows.
	mux.HandleFunc("GET /api/audit/events", s.requireOperator(s.handleAuditEvents))
	// Edit an actor's throughput envelope (ADR 1045). Scoped: the owning
	// vendor (the registering party, who knows its workload best) OR the
	// operator. Never gates anything — declared intent, not authorization.
	s.dualRoute(mux, "PUT", "/api/vendors/{vendor}/envelope", s.scopedHandler(s.putEnvelope))

	// Topics — vendor-scoped paths.
	s.dualRoute(mux, "GET", "/api/vendors/{vendor}/topics", revalidated(s.AuthMiddleware(true, s.listTopics)))
	s.dualRoute(mux, "POST", "/api/vendors/{vendor}/topics", s.scopedHandler(s.createTopic))
	s.dualRoute(mux, "GET", "/api/vendors/{vendor}/topics/{id}", revalidated(s.AuthMiddleware(true, s.getTopic)))
	s.dualRoute(mux, "PATCH", "/api/vendors/{vendor}/topics/{id}", s.scopedHandler(s.updateTopic))
	s.dualRoute(mux, "DELETE", "/api/vendors/{vendor}/topics/{id}", s.scopedHandler(s.deleteOrRetireTopic))

	// Bulk import/export per vendor.
	s.dualRoute(mux, "GET", "/api/vendors/{vendor}/topics/export", revalidated(s.AuthMiddleware(true, s.exportTopics)))
	s.dualRoute(mux, "POST", "/api/vendors/{vendor}/topics/import", s.scopedHandler(s.importTopics))

	// Schema bodies per topic (Block 85; design memo:
	// docs/design/portal-schema-publishing.md).
	s.dualRoute(mux, "POST", "/api/vendors/{vendor}/topics/{id}/schemas", s.scopedHandler(s.uploadSchema))
	s.dualRoute(mux, "GET", "/api/vendors/{vendor}/topics/{id}/schemas", revalidated(s.AuthMiddleware(true, s.listSchemas)))

	// Data-Access COP read API (ADR 1026 / goat-cli Phase G.4). Three-tier
	// privilege (data-access-cop.md §"Security"): the forward who/why
	// queries are self-or-operator scoped (a vendor may diagnose its OWN
	// access without admin — resolves the catch-22), enforced in-handler;
	// the cross-identity reverse topic query is operator-only.
	// Per-person presence slice for the id-console's offboard-verification
	// view (goat#319). Console-secret-or-operator (lookupAuthorized, in the
	// handler): ONE person's WhoResult, never the fleet roster — the roster
	// stays operator-only below.
	mux.HandleFunc("GET /api/lookup/person-presence", noStore(s.handlePersonPresence))
	mux.HandleFunc("GET /api/access/who", s.AuthMiddleware(true, s.handleAccessWho))
	mux.HandleFunc("GET /api/access/why", s.AuthMiddleware(true, s.handleAccessWhy))
	mux.HandleFunc("GET /api/access/topic", s.requireOperator(s.handleAccessTopic))
	mux.HandleFunc("GET /api/access/roster", noStore(s.requireOperator(s.handleAccessRoster)))

	// Registry data-roles resolver (ADR 1074 build item 1). Internal
	// service-to-service lookup — the same operator-bearer-or-console-token
	// gate as the sponsorship lookup (handled inside the handler, fail-closed).
	mux.HandleFunc("GET /api/access/data-roles", noStore(s.handleDataRoles))
	// Machine-plane audience for a release-set (rs ≡ producer): the member anchors
	// grant-keyd wraps the DEK to, derived from in-effect grant rows (Piece B,
	// grant-keyed-encryption §3a). Auth in-handler (lookupAuthorized).
	mux.HandleFunc("GET /api/access/audience", noStore(s.handleAudienceAccess))
	// The producer-facing "who could decrypt this?" gut check, asked before
	// publishing. Same derivation as /audience above, reported as counts rather
	// than anchors so it names nobody.
	mux.HandleFunc("GET /api/access/audience/preview", s.handleAudienceAccessPreview)
	mux.HandleFunc("GET /api/access/release-sets", s.handleAccessReleaseSets)
	// Governance-admin nominations (ADR 1102 decision 1, U9): list is a
	// service-consumer read; nominate/remove are gated in-handler on the
	// at-most-own-scope mayGovernOrg (a current governor of O, or an operator).
	mux.HandleFunc("GET /api/access/govern-admins", s.handleListGovernAdmins)
	mux.HandleFunc("POST /api/access/govern-admins", s.AuthMiddleware(true, s.handleNominateGovernAdmin))
	mux.HandleFunc("POST /api/access/govern-admins/{id}/remove", s.AuthMiddleware(true, s.handleRemoveGovernAdmin))
	// By-anchor PUBLIC recipient-cert store (U7 publish-my-key distribution):
	// grant-keyd fetches each audience member's current leaf here and validates
	// it against the portal-CA root before wrapping. Auth in-handler.
	mux.HandleFunc("GET /api/access/recipient-cert", noStore(s.handleRecipientCert))
	// Restrictive-share drift telemetry (ADR 1075 §6): the non-community label
	// share per goatnet — the ratchet-up-bias counter. A plain honest ratio,
	// operator-gated like the other governance counters.
	mux.HandleFunc("GET /api/access/label-drift", s.requireOperator(s.handleLabelDrift))

	// Topic-budget grant store + projection (ADR 1075 fold-in; cardinality
	// allocation layer 2b/3) and ADR 1104 slot lifecycle classes. All
	// operator-only; the fabric-crawler reads /api/budget-projection as its
	// budget source (live/stale/unavailable semantics on its side).
	mux.HandleFunc("GET /api/budget-projection", s.requireOperator(s.handleBudgetProjection))
	mux.HandleFunc("GET /api/budget-grants", s.requireOperator(s.handleListBudgetGrants))
	mux.HandleFunc("PUT /api/budget-grants/{owner}", s.requireOperator(s.handlePutBudgetGrant))
	mux.HandleFunc("DELETE /api/budget-grants/{owner}", s.requireOperator(s.handleDeleteBudgetGrant))
	mux.HandleFunc("GET /api/slots/lifecycle", s.requireOperator(s.handleListSlotLifecycles))
	mux.HandleFunc("PUT /api/slots/{anchor}/lifecycle", s.requireOperator(s.handlePutSlotLifecycle))
	// Record (or correct) a slot's person linkage after the fact. The onboard POST
	// stamps it inline when the caller has the UUID in hand, but the native
	// `goat admin onboard` runs its Ory leg AFTER the slot mint, and existing
	// slots predate the column entirely — both need a supported write path so a
	// backfill is an audited API call, not a hand-edited SQLite row.
	mux.HandleFunc("PUT /api/slots/{anchor}/identity", s.requireOperator(s.handlePutSlotIdentity))
	// Retro-stamp the ADR 1096 custody edge on a slot that already exists.
	// The enroll endpoint above allocates + mints; this only records a
	// relationship that already exists in the world (see slot_custody.go).
	mux.HandleFunc("PUT /api/slots/{anchor}/custody", s.requireOperator(s.handlePutSlotCustody))

	// Discover -> request -> approve loop + preemptive grants (ADR 1075 §"The
	// loop" / §12). Any authenticated viewer requests + reads the grant-target
	// directory; the producer custodian (its own vendor principal) or an operator
	// approves/declines/grants/revokes — the data-authority owns its own grants,
	// enforced in-handler. Approval mints a grant row the data-roles resolver
	// evaluates; the preemptive grant path has no pending-request precondition.
	mux.HandleFunc("POST /api/access/requests", s.AuthMiddleware(true, s.handleCreateAccessRequest))
	mux.HandleFunc("GET /api/access/requests", s.AuthMiddleware(true, s.handleListAccessRequests))
	mux.HandleFunc("POST /api/access/requests/{id}/approve", s.AuthMiddleware(true, s.handleApproveAccessRequest))
	mux.HandleFunc("POST /api/access/requests/{id}/decline", s.AuthMiddleware(true, s.handleDeclineAccessRequest))
	mux.HandleFunc("POST /api/access/grants", s.AuthMiddleware(true, s.handleCreateGrant))
	mux.HandleFunc("GET /api/access/grants", s.AuthMiddleware(true, s.handleListGrants))
	mux.HandleFunc("POST /api/access/grants/{id}/revoke", s.AuthMiddleware(true, s.handleRevokeGrant))
	// ADR 1105 — co-publish WRITE grants (PUBSUB-ORG-ADMIN). Minting is a
	// distinct, more-scrutinized path (at-most-own-org, justification-required);
	// revocation reuses the grants/{id}/revoke path above (a co-publish grant IS
	// a grant row). The materialized dict is the deploy render input for
	// zenoh_copublish_grants (operator-gated — it enumerates the fabric's
	// cross-anchor write topology). ADR 1105 multi-publisher topics — the
	// deliberately-created, marked, discoverable multi-publisher class. Create =
	// a govern-admin act; list = discovery (any authenticated principal,
	// visibility advisory).
	mux.HandleFunc("POST /api/access/multipub-topics", s.AuthMiddleware(true, s.handleCreateMultipubTopic))
	mux.HandleFunc("GET /api/access/multipub-topics", s.AuthMiddleware(true, s.handleListMultipubTopics))
	mux.HandleFunc("POST /api/access/co-publish", s.AuthMiddleware(true, s.handleCreateCoPublishGrant))
	mux.HandleFunc("GET /api/access/co-publish-grants", s.requireOperator(s.handleCoPublishGrantsExport))
	mux.HandleFunc("GET /api/access/directory", s.AuthMiddleware(true, s.handleAccessDirectory))
	// Peer→person resolver (peer-identity-resolution.md, Option B) — operator-only
	// (exposes person PII: email/name/ory id). GET single + POST batch (one graph
	// assembly for a whole peer list — a card-baker uses the batch).
	mux.HandleFunc("GET /api/access/peer", noStore(s.requireOperator(s.handleAccessPeer)))
	mux.HandleFunc("POST /api/access/peers", noStore(s.requireOperator(s.handleAccessPeersBatch)))
	// Anomaly probation list (ADR 1068) — hold/list/decide, operator-gated.
	mux.HandleFunc("GET /api/access/probation", s.requireOperator(s.handleProbationList))
	mux.HandleFunc("POST /api/access/probation", s.requireOperator(s.handleProbationHold))
	mux.HandleFunc("POST /api/access/probation/{id}/decide", s.requireOperator(s.handleProbationDecide))
	// Audit-tail read (goat-cli G.6 / ADR 1038): mint short-TTL presigned S3
	// GET URLs for the audit objects covering a time window, so the operator
	// reads the Object-Lock audit-tail without MinIO credentials. Operator-
	// gated — the audit-tail is a fleet-wide history.
	mux.HandleFunc("GET /api/audit/presign", noStore(s.requireOperator(s.handleAuditPresign)))
	// Structured "Swagger for protobuf" view of a topic's uploaded
	// descriptor — flat JSON tree of messages + enums consumed by the
	// portal's /schema.html viewer. Registered BEFORE the catch-all
	// fetch route below so Go 1.22+ pattern-precedence picks the more
	// specific path.
	mux.HandleFunc("GET /api/schemas/render/{key...}", s.AuthMiddleware(true, s.renderSchemaByKey))
	// Catch-all consumer-facing fetch (any authenticated principal):
	// GET /api/schemas/<vendor>/<system>/.../<topic-version>[?version=v2]
	mux.HandleFunc("GET /api/schemas/{key...}", s.AuthMiddleware(true, s.fetchSchemaByKey))

	// Per-topic, per-language vendor-provided code examples (Block 85 PR-D).
	// Operator-or-matching-vendor scoped, same as topic mutations.
	s.dualRoute(mux, "GET", "/api/vendors/{vendor}/topics/{id}/examples", s.AuthMiddleware(true, s.listCodeExamples))
	s.dualRoute(mux, "PUT", "/api/vendors/{vendor}/topics/{id}/examples/{lang}", s.scopedHandler(s.putCodeExample))
	s.dualRoute(mux, "GET", "/api/vendors/{vendor}/topics/{id}/examples/{lang}", s.AuthMiddleware(true, s.getCodeExample))
	s.dualRoute(mux, "DELETE", "/api/vendors/{vendor}/topics/{id}/examples/{lang}", s.scopedHandler(s.deleteCodeExample))

	// Inspector (read-through to Zenoh REST + SSE).
	// The catalog — one response carrying every org, its topics and each
	// topic's schema versions. Replaces a browser-side fan-out of
	// 1 + one-per-org + one-per-topic requests per load. Any authenticated
	// principal, slot keys included — topic visibility is advisory, not access
	// (ADR 1028).
	mux.HandleFunc("GET /api/catalog", revalidated(s.AuthMiddleware(true, s.handleCatalog)))
	mux.HandleFunc("GET /api/topics/latest", revalidated(s.AuthMiddleware(true, s.handleLatest)))
	mux.HandleFunc("GET /api/topics/recent", revalidated(s.AuthMiddleware(true, s.handleRecent)))
	mux.HandleFunc("GET /api/topics/stream", s.AuthMiddleware(true, s.handleStream))
	// Block 85 PR-C — server-side protobuf decode using the uploaded
	// FileDescriptorSet. Browser dispatches here for protobuf:<MsgName>
	// topics instead of trying to decode binary descriptors in JS.
	mux.HandleFunc("GET /api/topics/decoded", s.AuthMiddleware(true, s.handleDecoded))

	// Vendor-progress swimlane (Block 88; design at
	// docs/design/portal-vendor-progress.md). Operator-only. Backed by
	// vendor_progress rows the netbird poller + MinIO audit tailer
	// keep updated.
	mux.HandleFunc("GET /api/progress", s.requireOperator(s.listProgress))

	// State-of-record snapshot (Block 86; design at
	// docs/design/pubsub-state-as-code.md). Operator-only. The body is
	// a tar+gzip of the canonical pubsub-state/ tree the reconciler reads
	// back. The /meta variant returns just the summary headers as JSON for
	// the auto-commit hook to short-circuit on unchanged content.
	mux.HandleFunc("GET /api/admin/snapshot", noStore(s.requireOperator(s.handleAdminSnapshot)))
	mux.HandleFunc("GET /api/admin/snapshot/meta", noStore(s.requireOperator(s.handleAdminSnapshotMeta)))

	// Per-vendor activity roster: registry-level aggregates joined with the
	// Ory identities associated with each vendor via traits.vendor.short_tag.
	// Operator-only. Design: docs/design/portal-vendor-activity-admin.md.
	mux.HandleFunc("GET /api/admin/vendor-activity", noStore(s.requireOperator(s.handleAdminVendorActivity)))
	mux.HandleFunc("GET /api/admin/vendor-activity/{vendor}/events", noStore(s.requireOperator(s.handleAdminVendorAuditEvents)))

	// trashgoat — operator-controlled chaos publisher (Block 92, PR #2).
	// Spec persistence + lifecycle + control-topic emit only; the runner
	// is a sibling Rust binary that lands in PR #3. Design: docs/design/trashgoat.md.
	mux.HandleFunc("GET /api/admin/trashgoat/specs", s.requireOperator(s.listTrashgoatSpecs))
	mux.HandleFunc("POST /api/admin/trashgoat/specs", s.requireOperator(s.createTrashgoatSpec))
	mux.HandleFunc("GET /api/admin/trashgoat/specs/{id}", s.requireOperator(s.getTrashgoatSpec))
	mux.HandleFunc("PATCH /api/admin/trashgoat/specs/{id}", s.requireOperator(s.patchTrashgoatSpec))
	mux.HandleFunc("DELETE /api/admin/trashgoat/specs/{id}", s.requireOperator(s.deleteTrashgoatSpec))
	mux.HandleFunc("POST /api/admin/trashgoat/specs/{id}/activate", s.requireOperator(s.activateTrashgoatSpec))
	mux.HandleFunc("POST /api/admin/trashgoat/specs/{id}/pause", s.requireOperator(s.pauseTrashgoatSpec))
	mux.HandleFunc("POST /api/admin/trashgoat/specs/{id}/retire", s.requireOperator(s.retireTrashgoatSpec))
	mux.HandleFunc("POST /api/admin/trashgoat/specs/{id}/cleanse", s.requireOperator(s.cleanseTrashgoatSpec))

	// One-time tarball download for newly-minted orgs (token gate, no
	// bearer auth — token is bound to a single download). ADR 1052: this route
	// was registered with a plain HandleFunc and so was never given an
	// /api/orgs* twin by dualRoute, nor counted toward legacyVendorHits (a
	// Phase-1 coverage gap surfaced by the Phase-4 audit — F-233). It is now
	// dual-served like every other surface: the org path /api/org-bundle is the
	// primary (bundleURL mints it), the legacy /api/vendor-bundle path stays for
	// back-compat and is counted so the Phase-4 gate sees it. Both keep the
	// {vendor} path-param name (param rename is the deferred cosmetic step).
	mux.HandleFunc("GET /api/org-bundle/{vendor}/{token}", noStore(s.handleVendorBundle))
	mux.HandleFunc("GET /api/vendor-bundle/{vendor}/{token}", noStore(s.countLegacyVendor(s.handleVendorBundle)))

	// Static frontend, served from a table precomputed at construction:
	// display-lexicon substitution applied to HTML (docs/design/portal-lexicon.md),
	// plus a strong ETag and a gzip encoding per asset.
	mux.Handle("GET /", s.staticHandler())

	return mux
}

// middleware wraps every request with a request-id, access log, and a
// panic recover so one bad handler doesn't kill the listener. Under /api it
// also stamps the contract version, refuses a request for another major
// version, and turns the standard library's plain-text 404 and 405 into the
// problem shape (statusWriter).
func (s *Server) middleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		reqID := s.nextRequestID()
		w.Header().Set("X-Request-Id", reqID)
		w.Header().Set("X-Content-Type-Options", "nosniff")
		api := strings.HasPrefix(r.URL.Path, "/api/")
		if api {
			w.Header().Set("Api-Version", APIVersion)
			// The backstop above every route's own cap; w is the server's
			// writer, so an oversize body also closes the connection.
			r.Body = http.MaxBytesReader(w, r.Body, maxRequestBody)
		}
		defer func() {
			if rec := recover(); rec != nil {
				slog.Error("handler panic",
					"request_id", reqID,
					"path", r.URL.Path,
					"method", r.Method,
					"panic", rec,
				)
				writeError(w, http.StatusInternalServerError, "panic", "internal server error")
			}
		}()
		ww := &statusWriter{ResponseWriter: w, status: http.StatusOK, principal: "anon", api: api}
		if api && !apiVersionAccepted(r.Header.Get("Api-Version")) {
			// The supplied value is not echoed; the constant names what is served.
			writeError(ww, http.StatusBadRequest, "unsupported_api_version",
				"this portal serves Api-Version "+APIVersion)
		} else {
			next.ServeHTTP(ww, r)
		}
		// principal is stashed on ww by the auth layer (the context the
		// auth middleware augments rides on the request handed *down* the
		// chain, so the outer middleware's r never sees it — ww is the
		// single object shared by pointer, so it does). F-207.
		//
		// A refusal is logged at Info so the instance a caller quotes from a
		// problem body meets a line in a production log; a success stays at
		// Debug.
		level := slog.LevelDebug
		if ww.status >= http.StatusBadRequest {
			level = slog.LevelInfo
		}
		slog.Log(r.Context(), level, "access",
			"request_id", reqID,
			"method", r.Method,
			"path", r.URL.Path,
			"status", ww.status,
			"principal", ww.principal,
			"latency_ms", time.Since(start).Milliseconds(),
		)
	})
}

// scopedHandler is the operator-or-matching-vendor wrapper for routes
// whose vendor identity is in the URL path.
func (s *Server) scopedHandler(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		vendor := r.PathValue("vendor")
		s.requireVendorScope(vendor, next)(w, r)
	}
}

// dualRoute registers h under both the legacy /api/vendors* path and its
// /api/orgs* alias (ADR 1052 Phase 1 — vendor→org machine rename,
// back-compat-first). Both paths share the same already-wrapped handler and
// keep the {vendor} path-param name, so handlers are untouched (the param +
// identifier rename is Phase 2). The legacy /api/vendors* leg is wrapped with a
// hit counter so Phase-4 removal can be gated on observed zero use, never a
// calendar. See docs/design/vendor-to-org-machine-rename.md.
func (s *Server) dualRoute(mux *http.ServeMux, method, vendorsPath string, h http.HandlerFunc) {
	orgsPath := strings.Replace(vendorsPath, "/api/vendors", "/api/orgs", 1)
	mux.HandleFunc(method+" "+vendorsPath, s.countLegacyVendor(h))
	mux.HandleFunc(method+" "+orgsPath, h)
}

// countLegacyVendor increments the deprecated-/api/vendors*-surface counter
// before delegating (ADR 1052 Phase 1). Surfaced on /api/health.
func (s *Server) countLegacyVendor(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		s.legacyVendorHits.Add(1)
		next(w, r)
	}
}

// nextRequestID is 96 random bits under a fixed prefix. It goes to every
// caller in X-Request-Id and in a problem body's instance, so it carries no
// boot time and no request count; the counter stays internal.
func (s *Server) nextRequestID() string {
	s.reqCounter.Add(1)
	var b [12]byte
	if _, err := rand.Read(b[:]); err != nil {
		// crypto/rand does not fail on a working host; a request still gets
		// a unique id rather than none.
		return fmt.Sprintf("rq-%d-%d", s.startedAt.UnixNano(), s.reqCounter.Load())
	}
	return "rq-" + base64.RawURLEncoding.EncodeToString(b[:])
}

type statusWriter struct {
	http.ResponseWriter
	status int
	// api marks a request under /api, where a plain-text 404 or 405 from the
	// standard library is rewritten as a problem body.
	api bool
	// replaced is set once that rewrite has happened; the stdlib body that
	// follows is dropped.
	replaced bool
	// principal is the access-log label the auth layer stamps once it
	// resolves a Principal ("operator:<actor>" / "vendor:<prefix>"); it
	// stays "anon" for unauthenticated requests. F-207. ww is shared by
	// pointer down the handler chain, so this is how the principal reaches
	// the outer access-log middleware even though the augmented request
	// context only flows downward.
	principal string
}

func (sw *statusWriter) WriteHeader(code int) {
	sw.status = code
	if sw.api && stdlibErrorPage(sw.Header(), code) {
		sw.replaced = true
		h := sw.Header()
		h.Del("Content-Length")
		// The gzip layer sits inside this writer; a coding it chose for the
		// stdlib body must not label the problem body written here instead.
		h.Del("Content-Encoding")
		writeProblem(sw.ResponseWriter, code, stdlibProblem(sw, code))
		return
	}
	sw.ResponseWriter.WriteHeader(code)
}

// stdlibErrorPage reports whether a response is the standard library's own
// 404 or 405 page (http.Error: text/plain, or the mux's 405 with no type set)
// rather than a body a handler chose.
func stdlibErrorPage(h http.Header, code int) bool {
	if code != http.StatusNotFound && code != http.StatusMethodNotAllowed {
		return false
	}
	ct := h.Get("Content-Type")
	return ct == "" || strings.HasPrefix(ct, "text/plain")
}

func (sw *statusWriter) Write(b []byte) (int, error) {
	if sw.replaced {
		return len(b), nil
	}
	return sw.ResponseWriter.Write(b)
}

// Unwrap lets http.ResponseController reach the writer beneath.
func (sw *statusWriter) Unwrap() http.ResponseWriter { return sw.ResponseWriter }

// Flush forwards to the underlying writer.
//
// Without this, statusWriter would silently disable server-push streaming for
// the whole API. Embedding the http.ResponseWriter INTERFACE promotes only that
// interface's three methods — Flush is not among them — so `w.(http.Flusher)`
// on a statusWriter would be false however capable the real writer underneath.
//
// /api/topics/stream depends on it: StreamSSE takes the flusher assertion at
// its top and returns "response writer does not support flushing" when it
// fails, which handleStream deliberately swallows, so the endpoint would answer
// 200 with an empty body and no event would reach a client.
func (sw *statusWriter) Flush() {
	if f, ok := sw.ResponseWriter.(http.Flusher); ok {
		f.Flush()
	}
}

// setPrincipalLabel stamps the access-log principal onto the statusWriter
// when w is one (it always is under middleware()). Called by the auth layer
// after a Principal is resolved. F-207.
func setPrincipalLabel(w http.ResponseWriter, p *Principal) {
	sw, ok := w.(*statusWriter)
	if !ok || p == nil {
		return
	}
	switch {
	case p.Operator:
		actor := p.Actor
		if actor == "" {
			actor = defaultOperatorHandle
		}
		sw.principal = "operator:" + actor
	case p.Vendor != "":
		sw.principal = "vendor:" + p.Vendor
	}
}

// --- helpers ----------------------------------------------------------

func writeJSON(w http.ResponseWriter, status int, body any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(body)
}

// apiError is the legacy error envelope, kept inside Problem for the clients
// that read error.code and error.message.
type apiError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

// maxJSONBody caps every JSON request body. A body past it decodes to a
// *http.MaxBytesError, which writeDecodeError answers with 413.
const maxJSONBody = 1 << 20

// maxRequestBody is the backstop middleware() puts on every /api request
// body, above the largest cap a route sets for itself (the schema upload at
// schemastore.MaxFileSize plus multipart framing). A route that forgets its
// own cap can no longer read an unbounded body.
const maxRequestBody = 2 << 20

func readJSON(w http.ResponseWriter, r *http.Request, into any) error {
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxJSONBody))
	dec.DisallowUnknownFields()
	return dec.Decode(into)
}

// decodeJSON is readJSON for the routes that accept unknown fields: the same
// cap, the same *http.MaxBytesError past it.
func decodeJSON(w http.ResponseWriter, r *http.Request, into any) error {
	return json.NewDecoder(http.MaxBytesReader(w, r.Body, maxJSONBody)).Decode(into)
}

// _ keeps embed import-stable even if a future maintainer moves the
// static-FS construction. Static FS is supplied by main; the API package
// shouldn't import its own embed bundle.
var _ = embed.FS{}

// ensure context import lives (Routes path values use it transitively).
var _ context.Context
var _ strings.Builder
