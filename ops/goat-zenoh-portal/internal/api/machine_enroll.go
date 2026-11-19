package api

// ADR 1096 stage 2 — the machine-enrollment mint leg (docs/design/machine-enrollment.md).
//
// POST /api/machines/enroll turns a custodian's standing enrollment_grant into a
// wire-distinguishable machine: it allocates the machine's OWN slot (ADR 1025),
// mints its cert, and stamps the person→system custody edge (stage 1). The mint
// mirrors onboardSlotSystem; what's new here is the custodian gate + quota +
// custody stamp.
//
// The mint core lives in enrollOne so the single door and the bulk door
// (machine_enroll_bulk.go) are the same mint, not two: one authorization shape,
// one rollback discipline, one audit vocabulary. The mesh-key re-mint for an
// already-enrolled machine is machine_mesh_key.go.
//
// Two deliberate gaps, both deferred, both harmless to attribution:
//   - The node-class/autonomy live in the REGISTRY (born-as record), NOT yet in
//     the cert URI-SAN. The URI-SAN is freeze-critical, CI-fenced identity grammar
//     (docs/design/portal-lexicon.md §"Cert identity layer"); enriching it is a
//     separately-gated follow-on. The wire ACL keys on the CN, so node-class in
//     the SAN would be attribution-only anyway.
//   - The `goat machine enroll` CLI wrapper is stage 2b; this endpoint is the
//     server-side work it calls (the person-flow shape: goat CLI → portal).

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"sort"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// machineSpec is the per-machine half of an enrollment request — what varies
// machine-to-machine inside a bulk act, and the whole late-bound input for a
// single one.
type machineSpec struct {
	MachineHandle string `json:"machine_handle"` // the system's human handle (slot alias)
	NodeClass     string `json:"node_class"`     // grammar node-class (SENSOR-FEED/…/MGMT-HOST/…)
	Autonomy      string `json:"autonomy"`       // grammar autonomy (manual/remote/supervised/autonomous)
	DisplayName   string `json:"display_name,omitempty"`
	// Workloads is a PLATFORM's declared sub-workload composition (function
	// classes: SENSOR/EFFECTOR/PROCESSOR) — ADR 1098. Valid only when node_class
	// is PLATFORM; the SPIRE role registers an SVID per entry (Option B). Empty
	// for a simple system.
	Workloads []string `json:"workloads,omitempty"`

	// ADR 1251 §7.3: a keyless source gets a slot and a custodian and no key,
	// certificate, API key or mesh key. Adapters names the ingress adapters
	// (machine slots with their own leaf) that may publish for it; Receive
	// says how they receive it. Both are required with keyless and refused
	// without it.
	Keyless  bool         `json:"keyless,omitempty"`
	Adapters []string     `json:"adapters,omitempty"`
	Receive  *receiveSpec `json:"receive,omitempty"`
	// Pods requests a named carriage rule per pod slot at enrollment (ADR
	// 1251 §7.5). The rule is active when the caller is also the pod's
	// custodian, otherwise it waits for the pod's custodian to accept it. A
	// refused request does not undo the enrollment; it is reported per pod.
	Pods []string `json:"pods,omitempty"`
}

// receiveSpec is how an ingress adapter receives a keyless source.
type receiveSpec struct {
	Interface string `json:"interface"`
	Port      int64  `json:"port"`
	Sender    string `json:"sender"`
}

// enrollCarriageResult reports one requested pod rule.
type enrollCarriageResult struct {
	PodSlot   string `json:"pod_slot"`
	RuleID    string `json:"rule_id,omitempty"`
	State     string `json:"state,omitempty"`
	ErrorCode string `json:"error_code,omitempty"`
	Error     string `json:"error,omitempty"`
}

type enrollMachineRequest struct {
	machineSpec
	// Custodian names the person whose grant authorizes this. Required when an
	// operator enrolls on someone's behalf (and for infra classes); for a
	// self-service vendor-scoped caller it is ignored (their own identity is used)
	// and rejected if it names anyone else.
	Custodian string `json:"custodian,omitempty"`
	// Exercise scopes the MESH KEY to an exercise's synthetic-site groups
	// (EX-<PREFIX>-<class>) instead of the goatnet's site scope. The name must
	// appear in the portal's exercise map (rendered from the taxonomy's
	// netbird_exercises); the derived groups must exist on the live mgmt server,
	// same fail-closed rule as the site scope. Identity is untouched — the
	// exercise is a key scope, not a cert attribute.
	Exercise string `json:"exercise,omitempty"`
}

type enrollMachineResponse struct {
	SlotID         string   `json:"slot_id"`
	SystemHandle   string   `json:"system_handle"`
	Custodian      string   `json:"custodian"`
	NodeClass      string   `json:"node_class"`
	Autonomy       string   `json:"autonomy"`
	Affiliation    string   `json:"affiliation"`
	Workloads      []string `json:"workloads,omitempty"`
	APIKey         string   `json:"api_key"`
	BundleURL      string   `json:"bundle_url"`
	BundleToken    string   `json:"bundle_token"`
	URISAN         string   `json:"uri_san"`
	LeafCN         string   `json:"leaf_cn"`     // the leaf's CN, "<slot_id>.g<generation>" (GHSA-rr3p)
	LeafStatus     string   `json:"leaf_status"` // live | pending | unknown: whether the router admits leaf_cn yet
	QuotaRemaining int64    `json:"quota_remaining"`
	NoteToShare    string   `json:"note_to_share"`

	// MeshGroups are the FULL NetBird group names this machine's node-class
	// resolves to (ADR 1116 §9). Always populated — the derivation is the point,
	// whether or not the portal is the thing that mints the key.
	MeshGroups []string `json:"mesh_groups"`
	// MeshSetupKey is the one-off key carrying exactly those groups. Empty when
	// mesh-key-mint-enabled is off, in which case MeshNote says so plainly rather
	// than letting an absent key read as "no mesh membership needed".
	MeshSetupKey string `json:"mesh_setup_key,omitempty"`
	MeshNote     string `json:"mesh_note"`
	// MeshScope names the exercise the key was scoped to; empty for the goatnet
	// site scope.
	MeshScope string `json:"mesh_scope,omitempty"`

	// ADR 1251: keyless sources and carriage requested at enrollment.
	Keyless  bool                   `json:"keyless,omitempty"`
	Adapters []string               `json:"adapters,omitempty"`
	Receive  *receiveSpec           `json:"receive,omitempty"`
	Carriage []enrollCarriageResult `json:"carriage,omitempty"`
}

// mintFailure carries an HTTP-shaped refusal out of the mint core, so the
// single door writes it as the response and the bulk door records it against
// its entry — one refusal vocabulary either way.
type mintFailure struct {
	status int
	code   string
	msg    string
}

func (s *Server) writeMintFailure(w http.ResponseWriter, f *mintFailure) {
	writeError(w, f.status, f.code, f.msg)
}

// exercisePrefixFor resolves an exercise name against the configured
// name=EX-PREFIX map. An unknown name is refused with the known names in the
// message — the refusal a caller can act on.
func exercisePrefixFor(raw, name string) (string, *mintFailure) {
	if name == "" {
		return "", nil
	}
	known := make(map[string]string)
	var names []string
	for _, pair := range strings.Split(raw, ",") {
		pair = strings.TrimSpace(pair)
		if pair == "" {
			continue
		}
		k, v, ok := strings.Cut(pair, "=")
		k, v = strings.TrimSpace(k), strings.TrimSpace(v)
		if !ok || k == "" || v == "" {
			continue
		}
		known[strings.ToLower(k)] = v
		names = append(names, k)
	}
	if len(known) == 0 {
		return "", &mintFailure{http.StatusBadRequest, "exercise_unconfigured",
			"this portal has no exercise map (PORTAL_MESH_EXERCISE_PREFIXES is empty) — exercise-scoped minting is not available on this goatnet"}
	}
	p, ok := known[strings.ToLower(name)]
	if !ok {
		sort.Strings(names)
		return "", &mintFailure{http.StatusBadRequest, "unknown_exercise",
			fmt.Sprintf("exercise %q is not in this goatnet's exercise map; known: %s", name, strings.Join(names, ", "))}
	}
	return p, nil
}

// validateMachineSpec runs the closed-grammar checks (cheap, before any
// allocation) and returns the machine's derived mesh group suffixes.
func validateMachineSpec(spec machineSpec) ([]string, *mintFailure) {
	if !registry.ValidNodeClass(spec.NodeClass) {
		return nil, &mintFailure{http.StatusBadRequest, "invalid_node_class",
			fmt.Sprintf("node_class %q is not in the grammar taxonomy (identity-grammar.md §1.4, ADR 1116 gear classes)", spec.NodeClass)}
	}
	// ADR 1116 §9 — the mint is the enforcement point. Two valid-but-unmintable
	// shapes, each refused with the answer rather than a bare rejection: the
	// refusal a custodian cannot act on is how a catch-all gets chosen instead.
	if registry.IsReservedNodeClass(spec.NodeClass) {
		return nil, &mintFailure{http.StatusBadRequest, "reserved_node_class",
			fmt.Sprintf("node_class %q is RESERVED and never enrollable (ADR 1116 §3): it names the sensing element or the interceptor itself, not a mesh peer. "+
				"Class the box by the FUNCTION it performs — SENSOR-FEED / SENSOR-TELEMETRY / SENSOR-ARBITER, or EFFECTOR-CUE / EFFECTOR-TRACK / EFFECTOR-TELEMETRY / EFFECTOR-ARBITER — "+
				"and enroll once per function it performs. A gateway fronting devices that do not speak fabric is a KID.", spec.NodeClass)}
	}
	if registry.IsIntakeOnlyNodeClass(spec.NodeClass) {
		return nil, &mintFailure{http.StatusBadRequest, "intake_only_node_class",
			fmt.Sprintf("node_class %q accepts direction but grants no outbound reach, so it must never be a machine's only class (ADR 1116) — a peer holding it alone could not reach the router to receive the very tasking the class exists for. "+
				"Enroll the machine under the class that carries its outbound half (SENSOR-FEED, EFFECTOR-CUE, KID …); its intake membership is then added deliberately, because that source list is enumerated by hand, permanently.", spec.NodeClass)}
	}
	// PLATFORM sub-workload composition (ADR 1098): valid only on a PLATFORM, each
	// a composable function class. Validate before derivation so an invalid
	// composition receives the stable invalid_workloads refusal, and before any
	// allocation so it consumes no slot or quota.
	if err := registry.ValidWorkloads(spec.NodeClass, spec.Workloads); err != nil {
		return nil, &mintFailure{http.StatusBadRequest, "invalid_workloads", err.Error()}
	}
	// The mesh groups this class resolves to. Computed BEFORE any allocation so a
	// class with no derivation costs nothing, and so the refusal names the class
	// rather than surfacing as a half-enrolled machine. A PLATFORM derives from
	// its declared workloads (ADR 1098) — it is a composition and has no reach of
	// its own.
	meshGroups, err := registry.MeshGroupsForMachine(spec.NodeClass, spec.Workloads)
	if err != nil {
		return nil, &mintFailure{http.StatusBadRequest, "no_mesh_mapping",
			fmt.Sprintf("cannot derive the mesh groups for this machine, so the mint will not guess (ADR 1116 §9): %v. "+
				"PROCESSOR is a legacy whole-system class with no NetBird group — a peer enrolled as one holds nothing and is uncovered; FUSION is the gear class for derived-data workloads.", err)}
	}
	if !registry.ValidAutonomy(spec.Autonomy) {
		return nil, &mintFailure{http.StatusBadRequest, "invalid_autonomy",
			fmt.Sprintf("autonomy %q is not manual/remote/supervised/autonomous", spec.Autonomy)}
	}
	if err := registry.ValidAlias(spec.MachineHandle); err != nil {
		return nil, &mintFailure{http.StatusBadRequest, "invalid_alias", err.Error()}
	}
	return meshGroups, nil
}

// mintMeshKey runs the ADR 1116 §9 mesh leg for a set of derived group
// suffixes: report-only when minting is off, a one-off single-use key when on.
// exercisePrefix "" = the goatnet site scope. Returns (fullGroupNames,
// setupKey, note, failure).
func (s *Server) mintMeshKey(ctx context.Context, keyName, nodeClass, exercisePrefix string, meshGroups []string) ([]string, string, string, *mintFailure) {
	names := make([]string, len(meshGroups))
	for i, g := range meshGroups {
		names[i] = s.nbKeys.GroupNameScoped(exercisePrefix, g)
	}
	switch {
	case !s.cfg.MeshKeyMintEnabled:
		return names, "", "mesh key NOT minted (mesh-key-mint-enabled is off): create a one-off setup key carrying EXACTLY the groups in mesh_groups, and nothing else.", nil
	case !s.nbKeys.Enabled():
		return nil, "", "", &mintFailure{http.StatusPreconditionFailed, "mesh_mint_unconfigured",
			"mesh-key-mint-enabled is on but the portal lacks a netbird mgmt URL, API token or mesh-site-prefix. Refusing to enroll a machine that would get a fabric identity and no mesh membership."}
	default:
		ttl, terr := time.ParseDuration(s.cfg.MeshKeyTTL)
		if terr != nil || ttl <= 0 {
			ttl = 24 * time.Hour
		}
		sk, kerr := s.nbKeys.MintScoped(ctx, keyName, exercisePrefix, meshGroups, ttl)
		if kerr != nil {
			return nil, "", "", &mintFailure{http.StatusBadGateway, "mesh_key_mint",
				fmt.Sprintf("could not mint the mesh setup key for node-class %s: %v.", nodeClass, kerr)}
		}
		note := fmt.Sprintf("mesh key minted one-off, single-use, valid %s, carrying exactly %s — derived from node-class %s, not chosen by hand.",
			ttl, strings.Join(sk.Groups, " + "), nodeClass)
		return sk.Groups, sk.Key, note, nil
	}
}

// enrollOne is the mint core: one machine, against an already-resolved
// custodian + grant, mesh key under the given exercise scope ("" = site).
// meshGroups must come from validateMachineSpec for this spec. Every failure
// path rolls back its own allocations (RevokeSlot returns the quota by
// construction), so a bulk caller can keep going.
func (s *Server) enrollOne(ctx context.Context, actor, custodian string, grant *registry.MachineGrant, spec machineSpec, exerciseName, exercisePrefix string, meshGroups []string) (*enrollMachineResponse, *mintFailure) {
	if mf := s.validateCarriageSpec(ctx, spec); mf != nil {
		return nil, mf
	}
	// Allocate the machine's own slot, then stamp the custody edge — which
	// ATOMICALLY re-checks the quota, closing the concurrent-enroll race any
	// pre-check can't.
	slot, err := s.reg.AllocateSlot(ctx, spec.MachineHandle)
	if errors.Is(err, registry.ErrAliasTaken) {
		return nil, &mintFailure{http.StatusConflict, "alias_taken",
			fmt.Sprintf("machine handle %q already has a live slot; revoke it or pick another", spec.MachineHandle)}
	}
	if errors.Is(err, registry.ErrSlotPoolExhausted) {
		return nil, &mintFailure{http.StatusServiceUnavailable, "pool_exhausted",
			"slot pool exhausted; enrollment paused until a planned router restart refills it (ADR 1025 §6)"}
	}
	if err != nil {
		return nil, &mintFailure{http.StatusInternalServerError, "allocate_slot", err.Error()}
	}
	anchor := slot.SlotID
	// The assignment moved the slot's generation; re-render the router's slot
	// subjects once this enrollment has either completed or rolled back.
	defer s.syncSlotFragment(ctx)

	if err := s.reg.RecordMachineEnrollment(ctx, anchor, custodian, spec.NodeClass, spec.Autonomy, grant.Affiliation, spec.Workloads); err != nil {
		_ = s.reg.RevokeSlot(ctx, anchor)
		if errors.Is(err, registry.ErrQuotaExhausted) {
			return nil, &mintFailure{http.StatusConflict, "quota_exhausted",
				fmt.Sprintf("custodian %q has no remaining machine quota", custodian)}
		}
		if errors.Is(err, registry.ErrAffiliationVariant) {
			// The enrollment inherited the grant's spelling and sent nothing of
			// its own, so this is not the caller's typo: the registry already
			// holds two spellings of the unit (a slot stamped before the guard),
			// and the fork has to be corrected on file before this custodian
			// can enroll again. Loud and nothing written, not a bare 500.
			return nil, &mintFailure{http.StatusConflict, "affiliation_variant", err.Error()}
		}
		return nil, &mintFailure{http.StatusInternalServerError, "record_enrollment", err.Error()}
	}

	if spec.Keyless {
		return s.enrollKeyless(ctx, actor, custodian, grant, spec, anchor)
	}

	// From here the mint mirrors onboardSlotSystem: api_key → vendor row → cert.
	apiKey, err := generateAPIKey(anchor)
	if err != nil {
		_ = s.reg.RevokeSlot(ctx, anchor)
		return nil, &mintFailure{http.StatusInternalServerError, "gen_key", err.Error()}
	}
	hash, digest, err := hashAPIKey(apiKey)
	if err != nil {
		_ = s.reg.RevokeSlot(ctx, anchor)
		return nil, &mintFailure{http.StatusInternalServerError, "hash", err.Error()}
	}

	display := spec.DisplayName
	if display == "" {
		display = spec.MachineHandle
	}
	now := time.Now().Unix()
	v := &registry.Vendor{
		Prefix:       anchor,
		APIKeyHash:   hash,
		APIKeySHA256: digest,
		DisplayName:  display,
		// Pre-baked ACL admits the slot the moment its cert exists — no per-enroll
		// approval gate (the grant already authorized it), so the anchor row is
		// approved on creation, exactly like onboardSlotSystem.
		ApprovedAt: &now,
		Approver:   &actor,
	}
	if err := s.reg.CreateVendor(ctx, v); err != nil {
		_ = s.reg.RevokeSlot(ctx, anchor)
		return nil, &mintFailure{http.StatusInternalServerError, "create", err.Error()}
	}

	// ServerAuth: a machine's leaf doubles as its pod router's LISTEN cert;
	// clientAuth-only leaves fail every local mTLS handshake against that listener.
	// CN = the slot's leaf CN at the generation the assignment gave it
	// (GHSA-rr3p).
	res, err := s.mintSlotLeaf(ctx, anchor, apiKey, slot.Generation, true, "")
	if err != nil {
		_ = s.reg.DeleteVendor(ctx, anchor)
		_ = s.reg.RevokeSlot(ctx, anchor)
		return nil, &mintFailure{http.StatusInternalServerError, "mint", err.Error()}
	}
	if err := s.reg.MarkBundle(ctx, anchor, res.BundleToken, res.TarballPath); err != nil {
		return nil, &mintFailure{http.StatusInternalServerError, "mark_bundle", err.Error()}
	}
	if err := s.reg.StampSlotCertAt(ctx, anchor, res.BundleToken, res.TarballPath, slot.Generation, ""); err != nil {
		return nil, &mintFailure{http.StatusInternalServerError, "stamp_slot", err.Error()}
	}

	// The mesh leg (ADR 1116 §9). The fabric identity above is only half an
	// enrollment; without this the machine still joins on a key whose groups a
	// human picked, which is the gap that let a sensor gateway, an effector
	// ground station and a laptop share one group. When minting is off, the
	// derived groups still travel in the response so the operator assigns
	// exactly them; when it is on, a failure rolls the whole enrollment back
	// rather than leaving a machine with a fabric identity and no mesh.
	meshGroupNames, setupKey, meshNote, mf := s.mintMeshKey(ctx, "machine-"+spec.MachineHandle, spec.NodeClass, exercisePrefix, meshGroups)
	if mf != nil {
		if mf.code == "mesh_key_mint" {
			mf.msg += " The enrollment was rolled back — quota is returned."
		}
		_ = s.reg.DeleteVendor(ctx, anchor)
		_ = s.reg.RevokeSlot(ctx, anchor)
		return nil, mf
	}

	_ = s.reg.LogAuditEvent(ctx, actor, "machine-enroll", spec.MachineHandle,
		fmt.Sprintf(`{"anchor":%q,"custodian":%q,"node_class":%q,"autonomy":%q,"mesh_groups":%q,"mesh_key_minted":%t,"mesh_scope":%q}`,
			anchor, custodian, spec.NodeClass, spec.Autonomy, strings.Join(meshGroupNames, ","), setupKey != "", exerciseName))

	return &enrollMachineResponse{
		SlotID:       anchor,
		SystemHandle: spec.MachineHandle,
		Custodian:    custodian,
		NodeClass:    spec.NodeClass,
		Autonomy:     spec.Autonomy,
		Affiliation:  grant.Affiliation,
		Workloads:    spec.Workloads,
		APIKey:       apiKey,
		BundleURL:    bundleURL(anchor, res.BundleToken),
		BundleToken:  res.BundleToken,
		URISAN:       res.URISAN,
		LeafCN:       registry.LeafCN(anchor, slot.Generation),
		LeafStatus:   s.leafStatus(anchor, slot.Generation),
		NoteToShare: "Deliver the cert tarball to the machine OOB; the api_key is its portal Bearer. Node-class/autonomy are recorded in the registry (born-as); the wire ACL admits it by CN (leaf_cn). " +
			leafStatusNote(s.leafStatus(anchor, slot.Generation)),
		MeshGroups:   meshGroupNames,
		MeshSetupKey: setupKey,
		MeshNote:     meshNote,
		MeshScope:    exerciseName,
	}, nil
}

// enrollMachine mints one machine principal against a custodian's grant. Wrapped
// in AuthMiddleware(required) so PrincipalFromContext is populated; the
// operator-vs-self-service gate (ADR 1096 decision #1) is applied per node-class
// inside.
func (s *Server) enrollMachine(w http.ResponseWriter, r *http.Request) {
	if s.cfg.IdentityModel != config.IdentityModelPerUUID {
		writeError(w, http.StatusPreconditionFailed, "not_slot_pool",
			"machine enrollment requires the per-uuid slot-pool identity model (ADR 1025)")
		return
	}
	var req enrollMachineRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_json", err.Error())
		return
	}

	exercisePrefix, mf := exercisePrefixFor(s.cfg.MeshExercisePrefixes, req.Exercise)
	if mf != nil {
		s.writeMintFailure(w, mf)
		return
	}
	meshGroups, mf := validateMachineSpec(req.machineSpec)
	if mf != nil {
		s.writeMintFailure(w, mf)
		return
	}

	// Resolve the custodian + apply the decision-#1 gate.
	p := PrincipalFromContext(r.Context())
	if p == nil {
		writeError(w, http.StatusUnauthorized, "auth", "missing principal")
		return
	}
	custodian, mf := s.resolveEnrollCustodian(r.Context(), p, req.NodeClass, req.Custodian)
	if mf != nil {
		s.writeMintFailure(w, mf)
		return
	}

	grant, mf := s.grantForEnroll(r.Context(), custodian)
	if mf != nil {
		s.writeMintFailure(w, mf)
		return
	}

	resp, mf := s.enrollOne(r.Context(), actorOf(r), custodian, grant, req.machineSpec, req.Exercise, exercisePrefix, meshGroups)
	if mf != nil {
		s.writeMintFailure(w, mf)
		return
	}
	s.requestEnrollCarriage(r, resp, req.Pods)
	resp.QuotaRemaining = -1
	if q, qerr := s.reg.MachineQuotaFor(r.Context(), custodian); qerr == nil {
		resp.QuotaRemaining = q.Remaining
	}
	writeJSON(w, http.StatusCreated, resp)
}

// grantForEnroll reads the custodian's standing pre-authorization (quota +
// inherited affiliation) and applies the shared refusals.
func (s *Server) grantForEnroll(ctx context.Context, custodian string) (*registry.MachineGrant, *mintFailure) {
	grant, err := s.reg.MachineGrantFor(ctx, custodian)
	if errors.Is(err, registry.ErrNotFound) {
		return nil, &mintFailure{http.StatusForbidden, "no_enrollment_grant",
			fmt.Sprintf("custodian %q holds no machine enrollment grant", custodian)}
	}
	if err != nil {
		return nil, &mintFailure{http.StatusInternalServerError, "grant_lookup", err.Error()}
	}
	if grant.Affiliation == "" {
		return nil, &mintFailure{http.StatusConflict, "grant_missing_affiliation",
			"the custodian's grant carries no affiliation; re-seed the grant before enrolling"}
	}
	if q, qerr := s.reg.MachineQuotaFor(ctx, custodian); qerr == nil && q.Remaining <= 0 {
		return nil, &mintFailure{http.StatusConflict, "quota_exhausted",
			fmt.Sprintf("custodian %q has no remaining machine quota (%d/%d used)", custodian, q.Used, q.Quota)}
	}
	return grant, nil
}

// resolveEnrollCustodian applies ADR 1096 decision #1 and returns the custodian
// handle this enrollment draws down.
//
//   - infra node-class → operator-only, custodian named explicitly (substrate).
//   - function node-class → self-service: an operator may enroll on behalf (naming
//     the custodian); a vendor-scoped caller enrolls only against its OWN identity.
func (s *Server) resolveEnrollCustodian(ctx context.Context, p *Principal, nodeClass, requested string) (string, *mintFailure) {
	if registry.IsInfraNodeClass(nodeClass) {
		if !p.Operator {
			// ADR 1113 — one narrow delegation off the operator-only rule: a
			// custodian holding the network-admin entitlement may enroll the
			// delegable infra classes (NETWORK, EDGE-SERVER — ADR 1216) for their OWN site. Everything
			// else stays operator-gated.
			if !registry.IsDelegableInfraNodeClass(nodeClass) {
				return "", &mintFailure{http.StatusForbidden, "infra_requires_operator",
					"infrastructure node-classes (MGMT-HOST/ZENOH/SNITCH/SERVICES/BREAKGLASS/NETWORK-JUMP) are operator-gated (ADR 1096 decision #1)"}
			}
			custodian := p.Vendor
			if custodian == "" {
				return "", &mintFailure{http.StatusForbidden, "infra_requires_operator",
					"caller has no custodian identity to enroll against"}
			}
			// A network admin enrolls only against their own identity — naming
			// someone else would be enrolling infrastructure on another person's
			// quota, which is the operator's prerogative, not theirs.
			if requested != "" && !strings.EqualFold(requested, custodian) {
				return "", &mintFailure{http.StatusForbidden, "custodian_mismatch",
					"a delegated network admin may enroll only against their own custodian identity"}
			}
			grant, gerr := s.reg.MachineGrantFor(ctx, custodian)
			if gerr != nil || grant == nil || !grant.NetworkAdmin {
				return "", &mintFailure{http.StatusForbidden, "network_admin_required",
					"node-class " + nodeClass + " is delegable, but this custodian holds no network-admin entitlement (ADR 1113). An operator grants it with: goat machine grant <custodian> --network-admin"}
			}
			if grant.Site == "" {
				return "", &mintFailure{http.StatusForbidden, "network_admin_unscoped",
					"network-admin entitlement is site-scoped and this grant carries no site; refusing fail-closed"}
			}
			return custodian, nil
		}
		if requested == "" {
			return "", &mintFailure{http.StatusBadRequest, "custodian_required",
				"operator must name the custodian for an infrastructure enrollment"}
		}
		return requested, nil
	}
	// function class — self-service
	if p.Operator {
		if requested == "" {
			return "", &mintFailure{http.StatusBadRequest, "custodian_required",
				"operator must name the custodian to enroll a machine on their behalf"}
		}
		return requested, nil
	}
	if p.Vendor == "" {
		return "", &mintFailure{http.StatusForbidden, "no_custodian", "caller has no custodian identity"}
	}
	if requested != "" && requested != p.Vendor {
		return "", &mintFailure{http.StatusForbidden, "custodian_mismatch",
			"a non-operator may only enroll machines against their own custodian identity"}
	}
	return p.Vendor, nil
}
