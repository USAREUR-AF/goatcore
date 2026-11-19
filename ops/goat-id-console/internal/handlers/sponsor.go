package handlers

// Sponsor mode (ADR 1072): the /onboarding surface for non-ADMINS sessions.
// An ADMINS session keeps the full admin console (adminGate path, unchanged).
// A non-admin session is admitted ONLY when the portal resolves their Ory
// login email to a sponsorship annotation (vendors.sponsor_email — the portal
// stays the sponsorship SoT, read via the operator-or-service-token-gated
// GET /api/orgs/sponsorship?email=). Admitted sponsors see and vouch ONLY
// their own invitees, capped at their annotated tier; the scoping runs here
// against the console's own onboardstore (the console owns the onboarding
// engine post-extraction), keyed on the sponsor's {prefix, alias, email} —
// the three forms a recorded sponsor value takes across mint paths.

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"time"
)

// sponsorSession is the resolved sponsor identity for a non-admin session.
type sponsorSession struct {
	Email   string // the Ory login the portal binding matched
	Prefix  string // registry anchor of the sponsor's identity
	Alias   string // live slot alias ("" when none)
	MaxTier string // highest tier this sponsor may offer/vouch
}

// keys returns the normalized sponsor-match set for scoping.
func (sp *sponsorSession) keys() map[string]struct{} {
	return sponsorKeySet(sp.Prefix, sp.Alias, sp.Email)
}

// sponsorKeySet normalizes the identity forms a recorded invite/proposal
// `sponsor` value can carry (anchor prefix from machine-key mints, slot alias
// in display-derived paths, login email from console human mints).
func sponsorKeySet(parts ...string) map[string]struct{} {
	keys := map[string]struct{}{}
	for _, p := range parts {
		if v := strings.ToLower(strings.TrimSpace(p)); v != "" {
			keys[v] = struct{}{}
		}
	}
	return keys
}

func sponsorAllows(keys map[string]struct{}, sponsor string) bool {
	_, ok := keys[strings.ToLower(strings.TrimSpace(sponsor))]
	return ok
}

const ctxKeySponsor ctxKey = 100 // distinct from the ctxKeyActor/ctxKeyIdP iota block

// sponsorFromCtx returns the sponsor session, or nil for admin sessions.
func sponsorFromCtx(ctx context.Context) *sponsorSession {
	if v, ok := ctx.Value(ctxKeySponsor).(*sponsorSession); ok {
		return v
	}
	return nil
}

// sponsorCacheTTL bounds how long a resolved (or refused) sponsorship
// binding is trusted before the portal is asked again — a revoke takes at
// most this long to bite on an open console session.
const sponsorCacheTTL = 60 * time.Second

type sponsorCacheEntry struct {
	sp      *sponsorSession // nil = resolved as not-a-sponsor
	expires time.Time
}

var (
	sponsorCacheMu sync.Mutex
	sponsorCache   = map[string]sponsorCacheEntry{}
)

// sponsorLookupResponse mirrors the portal's sponsorship-by-email payload
// (the whoami response shape).
type sponsorLookupResponse struct {
	Org               string `json:"vendor"`
	Alias             string `json:"alias"`
	SponsorAuthorized bool   `json:"sponsor_authorized"`
	SponsorMaxTier    string `json:"sponsor_max_tier"`
}

// resolveSponsorByEmail asks the portal (through a short cache) whether email
// is a bound sponsor. (nil, nil) means "definitively not a sponsor"; an error
// means the portal couldn't answer (callers fail closed, nothing is cached).
func (s *Server) resolveSponsorByEmail(ctx context.Context, email string) (*sponsorSession, error) {
	email = strings.ToLower(strings.TrimSpace(email))
	base := strings.TrimRight(strings.TrimSpace(s.Cfg.PortalInternalURL), "/")
	token := strings.TrimSpace(s.Cfg.PortalLookupToken)
	if email == "" || base == "" || token == "" {
		// Not configured for sponsor mode — resolve as not-a-sponsor
		// (ADMINS-only behavior, exactly the pre-ADR-1072 posture).
		return nil, nil
	}
	sponsorCacheMu.Lock()
	if e, ok := sponsorCache[email]; ok && time.Now().Before(e.expires) {
		sponsorCacheMu.Unlock()
		return e.sp, nil
	}
	sponsorCacheMu.Unlock()

	req, err := http.NewRequestWithContext(ctx, http.MethodGet,
		base+"/api/orgs/sponsorship?email="+url.QueryEscape(email), nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+token)
	resp, err := onboardHTTP.Do(req)
	if err != nil {
		return nil, fmt.Errorf("portal sponsorship lookup: %w", err)
	}
	defer resp.Body.Close()

	var resolved *sponsorSession
	switch resp.StatusCode {
	case http.StatusNotFound:
		resolved = nil // definitively not a sponsor
	case http.StatusOK:
		var lr sponsorLookupResponse
		if err := json.NewDecoder(io.LimitReader(resp.Body, 1<<16)).Decode(&lr); err != nil {
			return nil, fmt.Errorf("portal sponsorship decode: %w", err)
		}
		if !lr.SponsorAuthorized {
			resolved = nil
		} else {
			maxTier := strings.ToLower(strings.TrimSpace(lr.SponsorMaxTier))
			if maxTier == "" {
				// Authorized with no recorded max → least-privileged tier.
				if h, herr := loadIntakeHints(); herr == nil {
					maxTier = h.lowestTier()
				}
			}
			resolved = &sponsorSession{Email: email, Prefix: lr.Org, Alias: lr.Alias, MaxTier: maxTier}
		}
	default:
		// 409 ambiguous binding, 401 bad service token, 5xx — all fail closed
		// and uncached so a fix bites immediately.
		return nil, fmt.Errorf("portal sponsorship lookup: HTTP %d", resp.StatusCode)
	}
	sponsorCacheMu.Lock()
	sponsorCache[email] = sponsorCacheEntry{sp: resolved, expires: time.Now().Add(sponsorCacheTTL)}
	sponsorCacheMu.Unlock()
	return resolved, nil
}

// onboardGate admits matrix-admin sessions as full admins — the same
// group→service rows adminGate consults (ADR 1073: never a hardcoded group
// literal), so ADMINS and IAM-ADMIN both pass — and non-admin sessions as
// portal-resolved sponsors; everyone else is refused. Only the human
// /onboarding routes use it — every other console page stays behind
// adminGate, and the machine realm has its own key gate. The operator-key
// queue is the deliberate exception under /onboarding: it is a credential
// mint, not an onboarding step, so it takes adminGate (#210).
func (s *Server) onboardGate(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		actor := s.actorFromHeaders(r)
		if s.Cfg.DevBypassAuth || s.Permissions.AnyGroupGrants(actor.Groups, "id-admin-console", "admin") {
			s.adminGate(next)(w, r)
			return
		}
		// The sponsor branch authorizes on the portal's answer for the
		// cookie email, so check the identity behind the cookie first: an
		// offboarded sponsor's open tab must not mint invites or vouch.
		live, status, lerr := s.liveActor(r.Context(), actor)
		if lerr != nil {
			http.Error(w, lerr.Error(), status)
			return
		}
		actor = live
		sp, err := s.resolveSponsorByEmail(r.Context(), actor.Email)
		if err != nil {
			http.Error(w, "sponsorship could not be verified (portal unavailable) — try again", http.StatusBadGateway)
			return
		}
		if sp == nil {
			http.Error(w, "forbidden: your login is not sponsorship-authorized — ask an admin to grant sponsorship bound to your login email", http.StatusForbidden)
			return
		}
		ctx := context.WithValue(r.Context(), ctxKeyActor, actor)
		ctx = context.WithValue(ctx, ctxKeySponsor, sp)

		idpKey := r.URL.Query().Get("idp")
		if idpKey == "" {
			idpKey = s.Cfg.DefaultIdP
		}
		idp, ok := s.Cfg.IdP(idpKey)
		if !ok {
			http.Error(w, "unknown idp: "+idpKey, http.StatusBadRequest)
			return
		}
		ctx = context.WithValue(ctx, ctxKeyIdP, idp)

		next.ServeHTTP(w, r.WithContext(ctx))
	}
}
