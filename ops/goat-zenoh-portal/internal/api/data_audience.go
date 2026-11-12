package api

// data_audience.go serves the machine-plane AUDIENCE for a release-set — the
// member anchors the grant-keyed-encryption key service (grant-keyd) wraps the
// DEK to. It is the server half of Piece B of the user-readiness plan
// (docs/project/grant-encryption-user-readiness-plan.md) and rests on the
// authority + audience model in docs/design/grant-keyed-encryption.md §3a.
//
// A release-set is a producer's stream identity (rs ≡ producer K). The audience
// is DERIVED, never a stored list (ADR 1074 virtual-role discipline): it is the
// projection of K's in-effect grant rows through the roster's org→producer
// expansion —
//   - an ORG grantee expands to the producer anchors under it (ProducersUnderOrg,
//     the vertical rollup);
//   - a PRINCIPAL grantee that names a machine anchor contributes it directly;
//   - a PRINCIPAL grantee with no machine anchor is a browser-only reader — it
//     reads through the human-plane surface and holds no wrapping key, so it is
//     NOT a machine-plane wrap target (noted, not included).
//   - the producer itself is always in its own audience, so it can unwrap its own
//     slot to obtain the DEK (the producer-as-member DEK-handoff decision, §3).
//
// Each returned member is a WRITE-ANCHOR — the fabric's machine-identity axis
// (the router ACL keys on the CN/anchor; identity-access-and-onboarding-reference
// §2.3/§2.8). The key service resolves anchor→recipient key via its own key
// source (keyd SvidRecipientKeys / cert dir).
//
// Auth mirrors the data-roles resolver: an operator bearer or the shared service
// token (lookupAuthorized) — grant-keyd is an internal service consumer. Never
// open silently.

import (
	"errors"
	"log"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// audienceResponse is the derived machine-plane audience for one release-set.
type audienceResponse struct {
	ReleaseSet string                  `json:"release_set"`
	Members    []string                `json:"members"`
	DerivedAt  string                  `json:"derived_at"`
	Freshness  *access.RosterFreshness `json:"freshness,omitempty"`
	Notes      []string                `json:"notes,omitempty"`
}

func (s *Server) handleAudienceAccess(w http.ResponseWriter, r *http.Request) {
	if !s.lookupAuthorized(r) {
		writeError(w, http.StatusUnauthorized, "auth", "operator bearer or service token required")
		return
	}
	// Accept either ?release_set= or ?producer= (rs ≡ producer for the MVP).
	producer := strings.TrimSpace(r.URL.Query().Get("release_set"))
	if producer == "" {
		producer = strings.TrimSpace(r.URL.Query().Get("producer"))
	}
	if producer == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "release_set (or producer) is required")
		return
	}

	// The roster projection is the org→member-anchor source; without it the
	// audience cannot be derived, so fail closed rather than wrap to a partial
	// (silently under-including) audience.
	if s.cfg.RosterPath == "" {
		writeError(w, http.StatusServiceUnavailable, "unavailable",
			"declared registry (roster path) not configured — the audience cannot be derived")
		return
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		// Do not echo err (it carries the roster filesystem path) — log
		// server-side, return a generic message.
		log.Printf("data-audience: load roster: %v", err)
		writeError(w, http.StatusInternalServerError, "registry", "could not load the declared registry")
		return
	}

	// In-effect grants for this producer, read live from the store (no parallel
	// store — SoT no_third_shim). A read error fails closed.
	rows, err := s.reg.ListGrants(r.Context(), producer, "", "", true)
	if err != nil {
		log.Printf("data-audience: list grants for %q: %v", producer, err)
		writeError(w, http.StatusInternalServerError, "registry", "could not load grant rows")
		return
	}
	// The coalition frameworks expand a multinational org's owning body (NATO)
	// into its member nations, with effective dating. Absent config is not an
	// error: only a `multinational` org needs it, and one is refused explicitly
	// below rather than silently wrapped to everybody.
	var fw *access.CoalitionFrameworks
	if s.cfg.CoalitionFrameworksPath != "" {
		if f, ferr := access.LoadCoalitionFrameworks(s.cfg.CoalitionFrameworksPath); ferr == nil {
			fw = f
		} else {
			log.Printf("data-audience: load coalition frameworks: %v", ferr)
		}
	}

	members, _, notes := s.deriveAudience(r, dr, fw, rows, producer)

	out := make([]string, 0, len(members))
	for a := range members {
		out = append(out, a)
	}
	sort.Strings(out)

	writeJSON(w, http.StatusOK, audienceResponse{
		ReleaseSet: producer,
		Members:    out,
		DerivedAt:  time.Now().UTC().Format(time.RFC3339),
		Freshness:  dr.Freshness,
		Notes:      notes,
	})
}

// deriveAudience is the ONE derivation both the wrap audience and the
// pre-publish preview run. It returns the anchors that earn a wrap slot, the
// per-anchor outcomes the preview tallies, and the human notes.
//
// Extracted rather than copied deliberately. The preview's whole value is that
// it predicts what the wrap will do; a second implementation would drift from
// this one and then be confidently wrong exactly when the two disagreed —
// worse than showing nothing, because a producer would have acted on it.
//
// It reads only what the portal already holds: the roster projection loaded for
// this request, the grant rows from the registry, and the coalition framework
// file. It issues NO fabric query, which is what lets the preview be called
// before every publish without client demand amplifying into fabric load
// (ADR 1123, surface-overload-protection.md — panoscope is the only surface
// permitted to query on request).
func (s *Server) deriveAudience(
	r *http.Request,
	dr *access.DataRoster,
	fw *access.CoalitionFrameworks,
	rows []registry.Grant,
	producer string,
) (map[string]bool, []access.PreviewInput, []string) {
	now := time.Now().Unix()
	asOf := time.Unix(now, 0).UTC()

	members := map[string]bool{}
	var outcomes []access.PreviewInput
	var notes []string
	// Producer-as-member: the producer is always in its own audience so it can
	// unwrap its own slot for the DEK (§3 DEK-handoff decision).
	members[producer] = true

	for _, g := range rows {
		if !g.InEffectAt(now) {
			continue
		}
		switch g.GranteeKind {
		case registry.GranteeOrg:
			anchors := dr.ProducersUnderOrg(g.GranteeID)
			// THE NATIONALITY INTERSECTION (ADR 1171, operator-ratified
			// ). Org membership alone no longer earns a wrap slot:
			// the holder's OWN nationality must also be one of the nations that
			// own the org. Fail-closed both ways — an org that declares no
			// nation, and a holder who declares none, wrap to nobody.
			//
			// This is where a Canadian at the US-owned usareur-af stops
			// receiving USAREUR-AF material while the American beside them keeps
			// it. `relationship` is not consulted; nationality does the work.
			var admitted, refused int
			for _, anchor := range anchors {
				ok, refusal, why := dr.ClassifyOrgMaterial(anchor, g.GranteeID, fw, asOf)
				held, _ := dr.ProducerNationality(anchor)
				outcomes = append(outcomes, access.PreviewInput{
					Anchor:      anchor,
					Org:         g.GranteeID,
					Admitted:    ok,
					Refusal:     refusal,
					Nationality: held,
				})
				if ok {
					members[anchor] = true
					admitted++
					continue
				}
				refused++
				// Name the reason per anchor. A silent shrink is the failure
				// mode this fabric already has too much of: a producer
				// must be able to see that the audience is smaller than the org
				// and why, before publishing.
				notes = append(notes, "org grantee "+g.GranteeID+": "+anchor+
					" is a member but earns no wrap slot — "+why)
			}
			// A grantee-less org (all members browser-only, or empty) expands to
			// nothing — say so, rather than silently returning just {producer}.
			if len(anchors) == 0 {
				notes = append(notes, "org grantee "+g.GranteeID+
					" expands to no machine anchor — its members are browser-only readers (human-plane) or it has no active producer members; nothing to wrap")
			} else if admitted == 0 {
				notes = append(notes, "org grantee "+g.GranteeID+": all "+
					strconv.Itoa(refused)+" member anchor(s) were refused by the nationality intersection — nothing is wrapped to this org")
			}
		case registry.GranteePrincipal:
			switch {
			case dr.KnownProducer(g.GranteeID):
				// A machine-principal grant: the grantee IS a machine anchor.
				members[g.GranteeID] = true
			default:
				// A person-principal grant: the grantee is an identity UUID.
				// Resolve UUID → email → email-hash → the person's machine
				// anchor(s), so a grant to a person reaches their machines (WS4).
				anchors := s.personMachineAnchors(r, dr, g.GranteeID)
				if len(anchors) > 0 {
					for _, a := range anchors {
						members[a] = true
					}
				} else {
					notes = append(notes, "principal grantee "+g.GranteeID+
						" has no machine anchor in the roster — a browser-only reader (human-plane), not a machine-plane wrap target")
				}
			}
		}
	}

	return members, outcomes, notes
}

// releaseSetsResponse enumerates the release-sets (producers) with at least one
// active grant — the sweep list grant-keyd derives in grant-row mode (WS4).
type releaseSetsResponse struct {
	ReleaseSets []string `json:"release_sets"`
	DerivedAt   string   `json:"derived_at"`
}

// handleAccessReleaseSets serves GET /api/access/release-sets: the distinct
// producers carrying an active grant. grant-keyd polls this to learn WHICH
// release-sets to sweep, so a `goat access grant` alone activates encryption for
// that producer without an operator editing the daemon config (the readiness
// DoD). Same auth as the audience read (operator bearer or service token).
func (s *Server) handleAccessReleaseSets(w http.ResponseWriter, r *http.Request) {
	if !s.lookupAuthorized(r) {
		writeError(w, http.StatusUnauthorized, "auth", "operator bearer or service token required")
		return
	}
	producers, err := s.reg.ProducersWithViewGrants(r.Context())
	if err != nil {
		log.Printf("data-audience: producers with view grants: %v", err)
		writeError(w, http.StatusInternalServerError, "registry", "could not enumerate release-sets")
		return
	}
	if producers == nil {
		producers = []string{} // grant-keyd parses a list; null fails its sweep
	}
	writeJSON(w, http.StatusOK, releaseSetsResponse{
		ReleaseSets: producers,
		DerivedAt:   time.Now().UTC().Format(time.RFC3339),
	})
}

// personMachineAnchors resolves a person-principal grant's identity UUID to the
// machine anchor(s) that person carries in the roster, so a grant to a person
// reaches their machines (WS4). UUID → email (Kratos) → email-hash (the roster
// join key, F-234) → MachineAnchorsForPerson. Returns nil when the identity
// layer is unavailable, the UUID does not resolve, or the person holds no
// machine anchor (a browser-only reader) — the caller then emits the browser-only
// note. Fail-closed: any resolution error yields no anchors, never a guess.
func (s *Server) personMachineAnchors(r *http.Request, dr *access.DataRoster, identityUUID string) []string {
	if s.humans == nil {
		return nil
	}
	ref, err := s.humans.byID(r.Context(), identityUUID)
	if err != nil {
		if !errors.Is(err, errHumanNotFound) {
			log.Printf("data-audience: resolve principal %q: %v", identityUUID, err)
		}
		return nil
	}
	return dr.MachineAnchorsForPerson(access.EmailHash(ref.Email))
}
