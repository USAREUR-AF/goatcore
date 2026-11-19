package handlers

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/selfcheck"
)

// liveActorTTL bounds how long a removed admin's open console tab keeps
// working: adminGate re-reads the identity from Kratos at most this often.
const liveActorTTL = 15 * time.Second

type liveActorEntry struct {
	state  string
	groups []string
	at     time.Time
}

type liveActorCache struct {
	mu sync.Mutex
	m  map[string]liveActorEntry
}

var liveActors = &liveActorCache{m: map[string]liveActorEntry{}}

func (c *liveActorCache) get(id string, now time.Time) (liveActorEntry, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	e, ok := c.m[id]
	if !ok || now.Sub(e.at) > liveActorTTL {
		return liveActorEntry{}, false
	}
	return e, true
}

func (c *liveActorCache) put(id string, e liveActorEntry) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.m) > 4096 {
		c.m = map[string]liveActorEntry{}
	}
	c.m[id] = e
}

// liveActor checks the signed-in admin against their identity as it is now.
// The session cookie carries the groups the person held at sign-in and
// oauth2-proxy never re-checks them, so offboarding, deactivation or a
// demotion would otherwise not reach a session that is already open. The
// identity must exist and be active; the returned actor carries its current
// groups. A Kratos failure refuses the request.
func (s *Server) liveActor(ctx context.Context, actor Actor) (Actor, int, error) {
	if !s.Cfg.LiveIdentityCheck {
		return actor, 0, nil
	}
	if actor.IdentityID == "" {
		return actor, http.StatusForbidden, errors.New("forbidden: your sign-in carries no identity id, so it cannot be checked against your current access; sign in again")
	}
	now := time.Now()
	e, ok := liveActors.get(actor.IdentityID, now)
	if !ok {
		cli := s.Kratos[s.Cfg.StepUpSSOIdP]
		if cli == nil {
			return actor, http.StatusServiceUnavailable, fmt.Errorf("sign-in realm %q is unavailable", s.Cfg.StepUpSSOIdP)
		}
		ident, err := cli.GetIdentity(ctx, actor.IdentityID)
		if err != nil {
			var httpErr *kratos.HTTPError
			if errors.As(err, &httpErr) && httpErr.StatusCode == http.StatusNotFound {
				return actor, http.StatusForbidden, errors.New("forbidden: your identity no longer exists; sign in again")
			}
			return actor, http.StatusServiceUnavailable, errors.New("your current access could not be checked (identity service unavailable); try again")
		}
		e = liveActorEntry{state: ident.State, groups: ident.Groups(), at: now}
		liveActors.put(actor.IdentityID, e)
	}
	if e.state != "active" {
		return actor, http.StatusForbidden, errors.New("forbidden: your identity is not active")
	}
	actor.Groups = e.groups
	return actor, 0, nil
}

// actorIsSelf reports whether the signed-in actor is the identity a change is
// about. The identity id is the answer; the address compare behind it folds
// case, plus-addressing and lookalike characters (selfcheck), so a session
// whose id is missing still cannot pass itself off as somebody else.
func actorIsSelf(actor Actor, id string, target *kratos.Identity) bool {
	if actor.IdentityID != "" && strings.EqualFold(actor.IdentityID, strings.TrimSpace(id)) {
		return true
	}
	if target == nil {
		return false
	}
	if actor.IdentityID != "" && strings.EqualFold(actor.IdentityID, target.ID) {
		return true
	}
	return selfcheck.AnySame([]string{actor.Email}, identityAddresses(target))
}

// identityAddresses is every address an identity is known by: its email trait
// and each verifiable address.
func identityAddresses(ident *kratos.Identity) []string {
	if ident == nil {
		return nil
	}
	out := []string{ident.Email()}
	for _, a := range ident.VerifiableAddresses {
		out = append(out, a.Value)
	}
	return out
}

// errActorUnresolved is returned when the session carries an identity id the
// console cannot read back, so it cannot say which addresses are the
// person's own. Callers refuse the decision.
var errActorUnresolved = errors.New("your identity could not be read back from the identity service, so the console cannot confirm you are not the person this request is about; try again")

// actorAddresses is every address the signed-in person is known by now: the
// session email, plus the current email and verifiable addresses of the
// identity the session's subject names. The session email is the one the
// person had at sign-in; the identity's addresses are the ones it has now, so
// a person whose email changed is still recognized under either.
//
// A session with no identity id, or a console with no sign-in realm client
// (LiveIdentityCheck off), yields the session email alone. A session WITH an
// identity id whose identity cannot be read is an error: the caller is about
// to decide a two-person rule, and a guess is a self-approval.
func (s *Server) actorAddresses(ctx context.Context, actor Actor) ([]string, error) {
	out := []string{actor.Email}
	if actor.IdentityID == "" || s.Cfg == nil {
		return out, nil
	}
	cli := s.Kratos[s.Cfg.StepUpSSOIdP]
	if cli == nil {
		if s.Cfg.LiveIdentityCheck {
			return nil, errActorUnresolved
		}
		return out, nil
	}
	ident, err := cli.GetIdentity(ctx, actor.IdentityID)
	if err != nil {
		return nil, errActorUnresolved
	}
	return append(out, identityAddresses(ident)...), nil
}

// actorForms is every name the signed-in person is known by: their addresses
// (actorAddresses) and the onboarding handle each address resolves to. The
// handle stays the same when the address changes. An address that resolves
// to no handle is normal (admins who predate the onboarding store); a store
// failure is an error.
func (s *Server) actorForms(ctx context.Context, actor Actor) ([]string, error) {
	addrs, err := s.actorAddresses(ctx, actor)
	if err != nil {
		return nil, err
	}
	forms := append([]string(nil), addrs...)
	if s.Onboard == nil {
		return forms, nil
	}
	seen := map[string]bool{}
	for _, a := range addrs {
		k := selfcheck.EmailKey(a)
		if k == "" || seen[k] {
			continue
		}
		seen[k] = true
		h, herr := s.handleForLoginEmail(ctx, strings.TrimSpace(a))
		switch {
		case herr == nil && strings.TrimSpace(h) != "":
			forms = append(forms, h)
		case herr != nil && !errors.Is(herr, onboardstore.ErrNotFound):
			return nil, herr
		}
	}
	return forms, nil
}

// deciderIsApplicant reports whether the person deciding proposal p is the
// person p is about. It compares every form the decider is known by (the
// decided-by value, which is the session email or an operator key's handle;
// the session identity's current addresses; the handles those resolve to; and
// a machine principal's handle, alias, sponsor email and onboarding email) against the proposal's email and
// handle, folding case, plus-addressing and lookalike characters.
//
// It cannot recognize the same person behind an unrelated second mailbox
// with an unrelated handle: nothing on a proposal ties it to an identity that
// does not exist yet. The tier ceiling and the escalated-invite step-up are
// what bound that case.
func (s *Server) deciderIsApplicant(r *http.Request, p *onboardstore.Proposal, decidedBy string) (bool, error) {
	ctx := r.Context()
	applicant := []string{p.Email, p.Handle}
	decider := []string{decidedBy}
	if actor := actorFromCtx(ctx); actor.Email != "" || actor.IdentityID != "" {
		forms, err := s.actorForms(ctx, actor)
		if err != nil {
			return false, err
		}
		decider = append(decider, forms...)
	}
	// A machine principal (operator or participant key) is known by its
	// handle, its alias and the sponsor email the portal recorded for it,
	// and a handle by the onboarding email filed under it.
	if pr := onboardPrincipalFromCtx(ctx); strings.TrimSpace(pr.Handle) != "" {
		decider = append(decider, pr.Handle, pr.Alias, pr.SponsorEmail)
		if !selfcheck.IsEmail(pr.Handle) && s.Onboard != nil {
			e, err := s.Onboard.ProposalEmailForHandle(ctx, pr.Handle)
			if err != nil {
				return false, err
			}
			decider = append(decider, e)
		}
	}
	return selfcheck.AnySame(decider, applicant), nil
}
