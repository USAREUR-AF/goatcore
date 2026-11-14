package api

// data_roles.go serves the registry data-roles resolver (ADR 1074 build item 1,
// docs/design/data-access-roles.md §6). It is the interchange described in §5:
// two entry points (a human principal, a machine anchor), one registry hub, one
// derived role set. The heavy lifting is the PURE resolver in internal/access
// (dataroles.go); this handler does only the live resolution that touches I/O —
// the Kratos identity ↔ email map for humans, the vendors registry for anchor
// existence — and the fail-closed service-token gate.
//
// Auth mirrors the ADR 1072 sponsorship lookup exactly: an operator bearer, or
// the console↔portal shared service token (s.lookupAuthorized). Internal
// service consumers only; never open silently.

import (
	"context"
	"errors"
	"net/http"
	"sort"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/kratosadmin"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// identityRef is the resolved {UUID, login email} of an Ory identity. The
// resolver keys the principal on the UUID; the email is used only to compute
// the roster join hash, never stored or joined on (ADR 1051/1069).
type identityRef struct {
	ID    string
	Email string
}

// errHumanNotFound signals that no Ory identity matched the id/email — the
// resolver fails closed (404) rather than derive roles for a phantom principal.
var errHumanNotFound = errors.New("data-roles: no identity matched")

// humanResolver maps an Ory identity by its stable UUID or (as a login-email
// alias) to {UUID, email}, and enumerates all identities for the governance
// cutover's email-hash→UUID bridge. Kratos-backed in production; faked in tests.
type humanResolver interface {
	byID(ctx context.Context, id string) (identityRef, error)
	byEmail(ctx context.Context, email string) (identityRef, error)
	listAll(ctx context.Context) ([]identityRef, error)
}

// kratosHumanResolver adapts the read-only Kratos admin client to humanResolver.
type kratosHumanResolver struct{ c *kratosadmin.Client }

func (k kratosHumanResolver) byID(ctx context.Context, id string) (identityRef, error) {
	idn, err := k.c.GetIdentity(ctx, id)
	if errors.Is(err, kratosadmin.ErrNotFound) {
		return identityRef{}, errHumanNotFound
	}
	if err != nil {
		return identityRef{}, err
	}
	return identityRef{ID: idn.ID, Email: idn.Email()}, nil
}

// byEmail resolves a login email to its identity by scanning the identity list
// (case-insensitive) — Kratos has no admin filter we rely on here, and the
// trial fabric is small (the same scan the COP already does). The email is the
// alias; the returned UUID is the principal key.
func (k kratosHumanResolver) byEmail(ctx context.Context, email string) (identityRef, error) {
	want := strings.ToLower(strings.TrimSpace(email))
	if want == "" {
		return identityRef{}, errHumanNotFound
	}
	ids, err := k.c.ListIdentities(ctx)
	if err != nil {
		return identityRef{}, err
	}
	for _, idn := range ids {
		if strings.ToLower(strings.TrimSpace(idn.Email())) == want {
			return identityRef{ID: idn.ID, Email: idn.Email()}, nil
		}
	}
	return identityRef{}, errHumanNotFound
}

// listAll returns every Ory identity as {UUID, email}. The governance cutover
// hashes each email to bridge the roster's email-hashes to identity UUIDs.
func (k kratosHumanResolver) listAll(ctx context.Context) ([]identityRef, error) {
	ids, err := k.c.ListIdentities(ctx)
	if err != nil {
		return nil, err
	}
	out := make([]identityRef, 0, len(ids))
	for _, idn := range ids {
		out = append(out, identityRef{ID: idn.ID, Email: idn.Email()})
	}
	return out, nil
}

// dataRolesResponse is the resolver's JSON. It carries ONLY registry-derived
// roles — no platform groups, no claim-carried extras, no notional default
// (those ride the Ory claim / per-goatnet flag; design doc §5).
type dataRolesResponse struct {
	Principal dataRolePrincipal       `json:"principal"`
	Roles     []string                `json:"roles"`
	Producers []string                `json:"producers"`
	DerivedAt string                  `json:"derived_at"`
	Freshness *access.RosterFreshness `json:"freshness,omitempty"`
	Notes     []string                `json:"notes,omitempty"`
}

// dataRolePrincipal is the resolved principal: the stable key (identity UUID for
// a person, producer/write-anchor key for a machine) and its kind.
type dataRolePrincipal struct {
	ID   string `json:"id"`
	Kind string `json:"kind"` // person | machine
}

// handleDataRoles serves GET /api/access/data-roles?identity=<uuid>|email=<addr>|anchor=<producer-key>.
// Exactly one selector. `identity` and `anchor` key on the stable principal id;
// `email` is a login-email alias resolved to the identity UUID first. Fail-closed
// on every error and on an unknown principal.
func (s *Server) handleDataRoles(w http.ResponseWriter, r *http.Request) {
	if !s.lookupAuthorized(r) {
		writeError(w, http.StatusUnauthorized, "auth", "operator bearer or console service token required")
		return
	}
	identity := strings.TrimSpace(r.URL.Query().Get("identity"))
	email := strings.TrimSpace(r.URL.Query().Get("email"))
	anchor := strings.TrimSpace(r.URL.Query().Get("anchor"))
	set := 0
	for _, v := range []string{identity, email, anchor} {
		if v != "" {
			set++
		}
	}
	if set != 1 {
		writeError(w, http.StatusBadRequest, "bad_request",
			"provide exactly one of identity=<uuid>, email=<addr>, anchor=<producer-key>")
		return
	}

	// The declared registry projection is the roll-up source; without it the
	// resolver cannot derive org scope, so fail closed rather than under-report.
	if s.cfg.RosterPath == "" {
		writeError(w, http.StatusServiceUnavailable, "unavailable",
			"declared registry (roster path) not configured — the data-roles resolver cannot derive roles")
		return
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "registry", "could not load the declared registry: "+err.Error())
		return
	}

	var principal dataRolePrincipal
	var result access.DataRoleResult
	// principalKey is the id in-effect grant rows are matched against: the
	// identity UUID for a person, the write anchor for a machine.
	var principalKey string

	switch {
	case anchor != "":
		// A machine principal: known if the roster declares the producer OR the
		// live vendors registry holds it (a producer minted since the last roster
		// regen still resolves — with a note that org roll-up needs the roster).
		known := dr.KnownProducer(anchor)
		if !known {
			if _, verr := s.reg.GetVendor(r.Context(), anchor); verr == nil {
				known = true
			} else if !errors.Is(verr, registry.ErrNotFound) {
				writeError(w, http.StatusInternalServerError, "registry", "could not check producer: "+verr.Error())
				return
			}
		}
		if !known {
			writeError(w, http.StatusNotFound, "unknown_principal", "no producer is registered under this anchor")
			return
		}
		principal = dataRolePrincipal{ID: anchor, Kind: "machine"}
		result = dr.RolesForAnchor(anchor)
		principalKey = anchor

	default: // a human principal (identity= or email=)
		if s.humans == nil {
			writeError(w, http.StatusServiceUnavailable, "unavailable",
				"identity layer (kratos) not configured — human principals cannot be resolved")
			return
		}
		var ref identityRef
		var rerr error
		if identity != "" {
			ref, rerr = s.humans.byID(r.Context(), identity)
		} else {
			// Principal-key discipline: resolve the email alias to the identity
			// UUID first; from here the principal is keyed on the UUID.
			ref, rerr = s.humans.byEmail(r.Context(), email)
		}
		if errors.Is(rerr, errHumanNotFound) {
			writeError(w, http.StatusNotFound, "unknown_principal", "no identity matched the given selector")
			return
		}
		if rerr != nil {
			writeError(w, http.StatusInternalServerError, "identity", "could not resolve identity: "+rerr.Error())
			return
		}
		principal = dataRolePrincipal{ID: ref.ID, Kind: "person"}
		principalKey = ref.ID
		// The roster join is the tag-independent email-hash, computed
		// from the resolved identity's own email — never the raw query email.
		result = dr.RolesForPerson(access.EmailHash(ref.Email))
	}

	// ADR 1075 grant-row evaluation (the extension point the #1515 resolver
	// left): fold in-effect grant rows into the derived set. Read the active
	// grants live from the registry (no parallel store — SoT no_third_shim),
	// keep only those in effect right now (the store owns the window + revocation
	// logic), and merge — a principal grant matching this principal, or an org
	// grant for an org the principal derived membership in, yields
	// data.view.producer:<producer>. A read error fails closed rather than
	// silently under-reporting a grantee's access.
	activeGrants, gerr := s.reg.ActiveGrants(r.Context())
	if gerr != nil {
		writeError(w, http.StatusInternalServerError, "registry", "could not load grant rows: "+gerr.Error())
		return
	}
	now := time.Now().Unix()
	var rows []access.GrantRow
	for _, g := range activeGrants {
		if g.InEffectAt(now) {
			rows = append(rows, access.GrantRow{GranteeKind: g.GranteeKind, GranteeID: g.GranteeID, Producer: g.Producer, Action: g.Action})
		}
	}
	if len(rows) > 0 {
		result = access.MergeGrants(result, principalKey, rows)
	}

	// Governance authority (ADR 1102) is added here from the ONE source
	// (governedOrgs), NOT emitted by the pure resolver — so the data-roles
	// surface (`goat access show`) speaks exactly the governance the grant gate
	// enforces. A person is matched to its nominations by identity UUID; a
	// machine is never nominated and rides the bootstrap-open member path.
	// memberOrgs are the org memberships the resolver already derived
	// (data.view.org sentences). Fail-closed on a store error.
	personIdentity := ""
	if principal.Kind == "person" {
		personIdentity = principal.ID
	}
	governRoles, governNotes, gverr := s.governRolesFor(r, personIdentity, access.OrgsFromRoles(result.Roles))
	if gverr != nil {
		writeError(w, http.StatusInternalServerError, "registry", "could not evaluate governance authority")
		return
	}
	result.Roles = append(result.Roles, governRoles...)
	sort.Strings(result.Roles)
	result.Notes = append(result.Notes, governNotes...)

	resp := dataRolesResponse{
		Principal: principal,
		Roles:     result.Roles,
		Producers: result.Producers,
		DerivedAt: time.Now().UTC().Format(time.RFC3339),
		Freshness: dr.Freshness,
		Notes:     result.Notes,
	}
	if resp.Roles == nil {
		resp.Roles = []string{}
	}
	if resp.Producers == nil {
		resp.Producers = []string{}
	}
	writeJSON(w, http.StatusOK, resp)
}
