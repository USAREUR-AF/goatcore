package api

// data_audience_preview.go serves the PRODUCER-facing question, asked before
// encrypting and publishing: "who could decrypt this?"
//
// The wrap audience next door answers it as a list of machine anchors, for
// grant-keyd. That is the wrong shape for a person: it is identifying, it is
// long, and it does not say whether the restriction is set right. This surface
// answers the same question as a count — a total, a breakdown by nationality and
// by organisation, and an honest account of who was refused and why — fast
// enough to be a gut check rather than a review step.
//
// THREE THINGS IT DELIBERATELY DOES.
//
//  1. It shares ONE code path with the real wrap (Server.deriveAudience). A
//     preview computed separately would drift from the thing it previews and
//     then be confidently wrong exactly when they disagreed — worse than
//     nothing, because a producer would already have acted on it.
//
//  2. It distinguishes EXCLUDED from UNDECLARED. Both produce a zero and they
//     mean opposite things: the first is the restriction holding, the second is
//     a registry nobody has filled in. Today every member of every org is
//     undeclared (identity.nationality post-dates every manifest in the store),
//     so an undifferentiated zero would be read as "tightly restricted" when it
//     actually means "unevaluable". That is the wrong conclusion in the unsafe
//     direction, and preventing it is the central design problem here, not an
//     edge case.
//
//  3. It never names anyone. docs/operators/roster.json is public-safe by
//     contract (ops/ci/check-roster-registry.py enforces it) and this surface
//     holds the same line: no handles, no names, no emails, and cells below
//     access.KAnonymityFloor withheld, because "1 holder, CAN, usareur-af" is a
//     name to anyone who knows that org.
//
// It issues no fabric query — only the roster projection, the grant rows and the
// coalition framework file, all of which the portal already holds. That is what
// lets it be called before every publish without client demand amplifying into
// fabric load (ADR 1123).

import (
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/access"
)

// audiencePreviewResponse is the pre-publish answer for one release-set.
type audiencePreviewResponse struct {
	ReleaseSet string `json:"release_set"`

	// Encrypted reports whether this release-set actually carries a grant at
	// all. Not every message is encrypted — the rule governs only payloads a
	// publisher chose to wrap — so a producer with no grants is publishing in
	// clear and the audience question does not apply to it.
	Encrypted bool `json:"encrypted"`

	access.AudiencePreview

	DerivedAt string                  `json:"derived_at"`
	Freshness *access.RosterFreshness `json:"freshness,omitempty"`
}

// handleAudienceAccessPreview serves GET /api/access/audience/preview.
//
// Same auth as the audience read it previews: an operator bearer or the shared
// service token. A producer asking about its own stream is still an operator
// action, and the surface reveals aggregate composition of an organisation —
// small, but not nothing. Never open silently.
func (s *Server) handleAudienceAccessPreview(w http.ResponseWriter, r *http.Request) {
	if !s.lookupAuthorized(r) {
		writeError(w, http.StatusUnauthorized, "auth", "operator bearer or service token required")
		return
	}
	producer := strings.TrimSpace(r.URL.Query().Get("release_set"))
	if producer == "" {
		producer = strings.TrimSpace(r.URL.Query().Get("producer"))
	}
	if producer == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "release_set (or producer) is required")
		return
	}

	if s.cfg.RosterPath == "" {
		writeError(w, http.StatusServiceUnavailable, "unavailable",
			"declared registry (roster path) not configured — the audience cannot be derived")
		return
	}
	dr, err := s.rosters.Load(s.cfg.RosterPath)
	if err != nil {
		log.Printf("data-audience-preview: load roster: %v", err)
		writeError(w, http.StatusInternalServerError, "registry", "could not load the declared registry")
		return
	}

	rows, err := s.reg.ListGrants(r.Context(), producer, "", "", true)
	if err != nil {
		log.Printf("data-audience-preview: list grants for %q: %v", producer, err)
		writeError(w, http.StatusInternalServerError, "registry", "could not load grant rows")
		return
	}

	var fw *access.CoalitionFrameworks
	if s.cfg.CoalitionFrameworksPath != "" {
		if f, ferr := access.LoadCoalitionFrameworks(s.cfg.CoalitionFrameworksPath); ferr == nil {
			fw = f
		} else {
			log.Printf("data-audience-preview: load coalition frameworks: %v", ferr)
		}
	}

	// The same derivation the wrap runs. Its member set is discarded here: the
	// preview reports counts, and the anchors are exactly what must not leave
	// this handler.
	_, outcomes, _ := s.deriveAudience(r, dr, fw, rows, producer)
	preview := access.BuildAudiencePreview(outcomes)

	encrypted := len(rows) > 0
	if !encrypted {
		preview.Notes = append(preview.Notes,
			"This release-set carries no grant, so nothing about it is encrypted and there is "+
				"no audience to preview. The rule governs only payloads a publisher chose to wrap.")
	}
	// The producer always holds its own slot (the DEK hand-off, §3), and it is
	// deliberately not counted as audience: it is the author, not a recipient,
	// and counting it would inflate every small audience by one.
	preview.Notes = append(preview.Notes,
		"The producer's own slot is not counted here — it is the author, not a recipient.")
	preview.Notes = append(preview.Notes,
		"A publisher can widen a specific grant deliberately; this preview shows the rule's "+
			"default, not a ceiling.")

	writeJSON(w, http.StatusOK, audiencePreviewResponse{
		ReleaseSet:      producer,
		Encrypted:       encrypted,
		AudiencePreview: preview,
		DerivedAt:       time.Now().UTC().Format(time.RFC3339),
		Freshness:       dr.Freshness,
	})
}
