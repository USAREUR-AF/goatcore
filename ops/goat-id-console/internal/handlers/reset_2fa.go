package handlers

import (
	"errors"
	"fmt"
	"net/http"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
)

// reset2FACredentialTypes are the second-factor credential types the reset
// flow blind-deletes. Kept in lockstep with kratos.HasSecondFactor's notion
// of "2FA"; a 404 on any of them is a benign no-op (handled in the client).
var reset2FACredentialTypes = []string{"totp", "webauthn", "lookup_secret"}

// handleUserReset2FA clears every second factor (TOTP / WebAuthn / backup
// codes) from an identity so an admin can unlock a user who lost their
// authenticator. The user keeps their first factor; after reset the admin
// mints a recovery link (nudged in the UI) so the user re-enrolls a factor.
//
// Modeled on handleUserRevokeSessions. Stripping a second factor is inherently
// sensitive, so the AAL2 step-up gate is UNCONDITIONAL (not matrix-gated) and
// every outcome — refused, error, ok — is audited as "reset_2fa".
func (s *Server) handleUserReset2FA(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, 400, err.Error())
		return
	}
	idp := idpFromCtx(r.Context())
	id := r.PathValue("id")
	cli := s.Kratos[idp.Key]
	// Fail closed on the target fetch (matching the revoke-sessions handler,
	// which now fails closed the same way): this action's whole justification
	// is auditability, and proceeding without the target would (a) emit an "ok"
	// audit row with no target attribution on a transient Kratos failure, and
	// (b) report success for a nonexistent identity (every DELETE would 404
	// into the benign no-op path).
	target, err := cli.GetIdentity(r.Context(), id)
	if err != nil {
		var httpErr *kratos.HTTPError
		if errors.As(err, &httpErr) && httpErr.StatusCode == http.StatusNotFound {
			s.renderError(w, r, 404, "identity not found")
			return
		}
		s.renderError(w, r, 502, fmt.Sprintf("Kratos get failed: %v", err))
		return
	}
	actor := actorFromCtx(r.Context())
	if targetOutranksActor(actor, target) {
		s.emitAudit(r, actor, "reset_2fa", target, idp.Key, nil, nil, false, "refused", tierCeilingRefusal)
		s.renderError(w, r, 403, tierCeilingRefusal)
		return
	}

	// Unconditional step-up — no matrix lookup. See doc comment.
	//
	// #174 upgraded this from a dead-end 403 to a redirect into the
	// explain-then-verify flow: an admin who has not stepped up recently now
	// lands on the interstitial rather than on an error page. The honesty rule
	// this handler already followed — step_up_verified reflects a real second
	// factor, not merely that the gate ran — is now the gate's own verdict,
	// and is shared by every gated path.
	out := s.requireStepUp(w, r, actor, "reset_2fa")
	if out.Handled {
		s.emitAudit(r, actor, "reset_2fa", target, idp.Key, nil, nil, false, "refused", stepUpChallengeAudit)
		return
	}
	stepUpVerified := out.Verified
	actor.AAL = aalLabel(stepUpVerified)

	for _, ct := range reset2FACredentialTypes {
		if err := cli.DeleteIdentityCredential(r.Context(), id, ct); err != nil {
			s.emitAudit(r, actor, "reset_2fa", target, idp.Key, nil, nil, stepUpVerified, "error", err.Error())
			s.renderError(w, r, 502, fmt.Sprintf("Kratos reset-2FA failed on %q: %v", ct, err))
			return
		}
	}
	s.emitAudit(r, actor, "reset_2fa", target, idp.Key, nil, nil, stepUpVerified, "ok", "")
	http.Redirect(w, r, fmt.Sprintf("/users/%s?idp=%s&reset2fa=ok", id, idp.Key), http.StatusSeeOther)
}
