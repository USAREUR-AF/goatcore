package handlers

import (
	"context"
	"errors"
	"net/http"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/selfcheck"
)

// SSH-key browser realm — the human half of "self-service submit, admin-gated
// install" (docs/design/ssh-key-self-service.md). The machine half is
// sshkeys_api.go.
//
// Two pages, two audiences:
//
//	GET/POST /ssh-key              — any signed-in person, their OWN key only
//	GET/POST /onboarding/ssh-keys  — operators, the approval queue
//
// The split matters. Submitting a key and being granted a shell are different
// events on purpose: the console is reachable with an Ory session, so if
// uploading authorized, a stolen web session would become a root-capable shell
// on a production host.

// handleSSHKeySelf renders the self-service page for the signed-in person.
func (s *Server) handleSSHKeySelf(w http.ResponseWriter, r *http.Request) {
	s.renderSSHKeySelf(w, r, nil)
}

// handleSSHKeySelfSubmit records a key for the SIGNED-IN person.
//
// The handle is resolved from the session, NEVER from the form. A form-supplied
// handle would let anyone with a console session park their own public key
// against someone else's account and wait for an admin to approve it — the
// approval step would be intact and would still grant the wrong person a shell.
func (s *Server) handleSSHKeySelfSubmit(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, http.StatusForbidden, err.Error())
		return
	}
	actor := actorFromCtx(r.Context())
	handle, herr := s.sshKeyHandleFor(r, actor.Email)
	if herr != nil {
		s.renderError(w, r, http.StatusForbidden, herr.Error())
		return
	}
	keyType, body, comment, perr := onboardstore.ParseSSHPublicKey(r.FormValue("pubkey"))
	if perr != nil {
		// The parser names the specific problem (a pasted PRIVATE key, a
		// wrapped copy, a type that disagrees with the body). Show that, not a
		// generic "invalid key" the person cannot act on.
		s.renderSSHKeySelf(w, r, &flash{Level: "error", Message: perr.Error()})
		return
	}
	k, serr := s.Onboard.SubmitSSHKey(r.Context(), &onboardstore.SSHKey{
		Handle:  handle,
		Email:   actor.Email,
		Env:     s.Cfg.NotifyGoatnet,
		KeyType: keyType,
		KeyBody: body,
		Comment: comment,
		Note:    strings.TrimSpace(r.FormValue("note")),
	}, s.Cfg.SSHKeyTTLDays)
	if serr != nil {
		if errors.Is(serr, onboardstore.ErrSSHKeyBadFormat) {
			s.renderSSHKeySelf(w, r, &flash{Level: "error", Message: serr.Error()})
			return
		}
		s.renderSSHKeySelf(w, r, &flash{Level: "error", Message: "could not record the key"})
		return
	}
	s.emitAudit(r, actor, "ssh_key_submit", nil, "", nil,
		map[string]interface{}{"handle": k.Handle, "fingerprint": k.Fingerprint, "id": k.ID, "status": k.Status, "via": "console-self"},
		false, "ok", "")
	s.renderSSHKeySelf(w, r, sshKeySubmitFlash(k))
}

// sshKeySubmitFlash says what ACTUALLY happened to the record, keyed on the
// state the store left it in. The old banner assumed the happy path and
// reported "recorded, awaiting approval" even when the row it refreshed was
// already installed — or, before the rejected-reopen fix, stuck in `rejected`
// where no queue would ever show it.
func sshKeySubmitFlash(k *onboardstore.SSHKey) *flash {
	fp := " (" + k.Fingerprint + ")"
	switch k.Status {
	case onboardstore.SSHKeyApproved:
		return &flash{Level: "success", Message: "This key" + fp + " is already approved — " +
			"it is waiting on the installer; no new approval is needed."}
	case onboardstore.SSHKeyInstalled:
		return &flash{Level: "success", Message: "This key" + fp + " is already installed — " +
			"nothing changed, and your access is unaffected."}
	case onboardstore.SSHKeyRevoked:
		return &flash{Level: "error", Message: "This key" + fp + " was revoked and stays revoked — " +
			"generate a new keypair and submit that instead."}
	default:
		// submitted — fresh, or a rejected key reopened as a fresh application.
		// Say what happens NEXT and what has NOT happened. The old welcome
		// email's defect was implying access existed the moment a key was
		// recorded.
		return &flash{Level: "success", Message: "Key recorded" + fp +
			". It grants no access yet — an admin has to approve it, and you will not be able to log in until they do."}
	}
}

// handleSSHKeysQueue is the operator approval queue.
func (s *Server) handleSSHKeysQueue(w http.ResponseWriter, r *http.Request) {
	s.renderSSHKeyQueue(w, r, nil)
}

// handleSSHKeyDecision is the queue's approve / reject / revoke buttons.
func (s *Server) handleSSHKeyDecision(w http.ResponseWriter, r *http.Request) {
	if err := s.validateCSRF(r); err != nil {
		s.renderError(w, r, http.StatusForbidden, err.Error())
		return
	}
	id := r.PathValue("id")
	action := r.PathValue("action")
	actor := actorFromCtx(r.Context())
	switch action {
	case "approve", "reject", "revoke":
	default:
		s.renderError(w, r, http.StatusBadRequest, "unknown action")
		return
	}
	// Read immutable audit facts before committing the decision. The queue does
	// not need a second read, so an audit event cannot be lost to a read-back.
	before, err := s.Onboard.SSHKeyByID(r.Context(), id)
	switch {
	case errors.Is(err, onboardstore.ErrNotFound):
		s.renderSSHKeyQueue(w, r, &flash{Level: "error", Message: "that key no longer exists"})
		return
	case err != nil:
		s.renderSSHKeyQueue(w, r, &flash{Level: "error", Message: "could not load the key"})
		return
	}
	switch action {
	case "approve":
		err = s.Onboard.ApproveSSHKey(r.Context(), id, actor.Email)
	case "reject":
		err = s.Onboard.RejectSSHKey(r.Context(), id, actor.Email)
	case "revoke":
		err = s.Onboard.RevokeSSHKey(r.Context(), id, actor.Email)
	}
	switch {
	case errors.Is(err, onboardstore.ErrNotFound):
		s.renderSSHKeyQueue(w, r, &flash{Level: "error", Message: "that key no longer exists"})
	case errors.Is(err, onboardstore.ErrSSHKeyWrongState):
		// Name the state. Two admins working the queue at once is normal, and
		// "cannot approve a key that is currently revoked" explains itself.
		cur := "in another state"
		if k, gerr := s.Onboard.SSHKeyByID(r.Context(), id); gerr == nil {
			cur = "currently " + k.Status
		}
		s.renderSSHKeyQueue(w, r, &flash{Level: "error", Message: "cannot " + action + " a key that is " + cur})
	case err != nil:
		s.renderSSHKeyQueue(w, r, &flash{Level: "error", Message: "could not " + action + " the key"})
	default:
		s.emitAudit(r, actor, "ssh_key_"+action, nil, "", nil,
			map[string]interface{}{"id": id, "handle": before.Handle, "fingerprint": before.Fingerprint, "via": "console"},
			false, "ok", "")
		msg := action + "ed " + before.Handle
		if action == "approve" {
			// APPROVED IS NOT INSTALLED. The installer still has to run, and
			// until it does the person has no shell. Reporting "approved"
			// alone would recreate the defect this track exists to remove.
			// Do not promise "the panel below": the panel is gated on the
			// viewer holding an installed key for this goatnet, so the
			// approving admin may not see it.
			msg = "Approved " + before.Handle + " (" + before.Fingerprint + "). Not installed yet — " +
				"a shell-holding admin runs goat admin ssh-key install to place it on the in-scope hosts."
		}
		s.renderSSHKeyQueue(w, r, &flash{Level: "success", Message: msg})
	}
}

func (s *Server) renderSSHKeySelf(w http.ResponseWriter, r *http.Request, f *flash) {
	actor := actorFromCtx(r.Context())
	if strings.TrimSpace(actor.Email) == "" {
		s.renderError(w, r, http.StatusUnauthorized, "no signed-in identity on this request")
		return
	}
	handle, err := s.sshKeyHandleFor(r, actor.Email)
	if err != nil {
		s.renderError(w, r, http.StatusForbidden, err.Error())
		return
	}
	mine, lerr := s.mySSHKeys(r, handle)
	if lerr != nil {
		s.renderError(w, r, http.StatusInternalServerError, "could not read your submitted keys")
		return
	}
	s.render(w, r, "ssh_keys", &pageVars{
		Title:        "Your SSH key",
		SSHKeySelf:   true,
		SSHKeyHandle: handle,
		SSHKeys:      mine,
		Flashes:      flashes(f),
	})
}

func (s *Server) renderSSHKeyQueue(w http.ResponseWriter, r *http.Request, f *flash) {
	status := strings.TrimSpace(r.URL.Query().Get("status"))
	if status == "" {
		status = onboardstore.SSHKeySubmitted
	}
	if status == "all" {
		status = ""
	}
	ctx := r.Context()
	ks, err := s.Onboard.ListSSHKeys(ctx, status)
	if err != nil {
		s.renderError(w, r, http.StatusInternalServerError, "could not list ssh keys")
		return
	}
	actor := actorFromCtx(ctx)
	sp := sponsorFromCtx(ctx)
	installEnv := strings.TrimSpace(s.Cfg.NotifyGoatnet)
	// Queue depth for the install panel: count of APPROVED keys for THIS
	// console's goatnet, including skip-annotated rows. Independent of the
	// status filter the queue view currently shows. The env filter matches
	// both the panel's promise ("every approved key for this console's
	// goatnet") and the installer's per-row env gate — counting cross-env or
	// empty-env rows would advertise a depth no install run can ever drain.
	approvedDepth := 0
	approvedDepthError := ""
	approved, aerr := s.Onboard.ListSSHKeys(ctx, onboardstore.SSHKeyApproved)
	if aerr == nil {
		for _, k := range approved {
			if strings.TrimSpace(k.Env) == installEnv {
				approvedDepth++
			}
		}
	} else {
		approvedDepthError = "could not read approved queue depth"
	}
	installEnvCmd := "'" + strings.ReplaceAll(installEnv, "'", "'\\''") + "'"
	installPanel := false
	installHint := false
	// Panel gate: admins only (onboardGate already admitted them), goatnet
	// known, and the signed-in email carrying at least one installed key for
	// this goatnet. Sponsors see neither panel nor hint. Every other admin —
	// including one on a console with no NOTIFY_GOATNET configured, where the
	// panel cannot render a command at all — sees the static hint instead of
	// nothing.
	if sp == nil {
		installHint = true
		if installEnv != "" && strings.TrimSpace(actor.Email) != "" {
			installed, ierr := s.Onboard.ListSSHKeys(ctx, onboardstore.SSHKeyInstalled)
			if ierr == nil {
				for _, k := range installed {
					if strings.EqualFold(strings.TrimSpace(k.Email), strings.TrimSpace(actor.Email)) &&
						strings.TrimSpace(k.Env) == installEnv {
						installPanel = true
						installHint = false
						break
					}
				}
			}
		}
	}

	s.render(w, r, "ssh_keys", &pageVars{
		Title:                   "SSH keys",
		SSHKeys:                 ks,
		SSHKeyStatus:            status,
		SSHKeyInstallPanel:      installPanel,
		SSHKeyInstallHint:       installHint,
		SSHKeyInstallEnv:        installEnv,
		SSHKeyInstallEnvQ:       installEnvCmd,
		SSHKeyInstallQueueDepth: approvedDepth,
		SSHKeyInstallQueueError: approvedDepthError,
		Flashes:                 flashes(f),
	})
}

func flashes(f *flash) []flash {
	if f == nil {
		return nil
	}
	return []flash{*f}
}

// sshKeyHandleFor resolves the signed-in email to the handle the rest of the
// onboarding system knows the person by.
//
// It REFUSES rather than inventing one. A guessed handle (the email local-part,
// say) would key the grant to an identity that may not exist, or worse may
// belong to someone else; the install would then either land nowhere or land
// under the wrong name.
func (s *Server) sshKeyHandleFor(r *http.Request, email string) (string, error) {
	// For the signed-in person's own address, bind through the session's
	// identity (its current addresses), not the cookie email alone.
	var handle string
	var err error
	if actor := actorFromCtx(r.Context()); strings.TrimSpace(actor.Email) != "" &&
		(strings.EqualFold(strings.TrimSpace(actor.Email), strings.TrimSpace(email)) || selfcheck.Same(actor.Email, email)) {
		handle, err = s.sessionRosterHandle(r.Context(), actor)
	} else {
		handle, err = s.handleForLoginEmail(r.Context(), email)
	}
	if errors.Is(err, onboardstore.ErrNotFound) {
		return "", errors.New("no onboarding record is associated with " + email +
			", so there is no handle to attach a key to. Ask an admin to finish your onboarding first.")
	}
	return handle, err
}

// errAmbiguousRosterHandle refuses a person whose settled onboarding records
// carry more than one handle: the console cannot say which is theirs.
var errAmbiguousRosterHandle = errors.New("your settled onboarding records carry more than one handle; ask an admin to settle which one is yours")

// handleForLoginEmail is the one login-email → handle binding every
// self-service surface makes: the SSH-key and operator-key browser pages
// (through sessionRosterHandle), and the operator-key machine door from the
// login the portal recorded on a participant's identity (#1373). One
// resolver, so two doors cannot disagree about who a person is.
//
// It reads SETTLED records only (past the admin gate, not rejected, failed or
// offboarded) for this console's goatnet, and the handle must be unique among
// them. When the newest settled record belongs to another goatnet the answer
// is not-found, as before. A pending record (proposed, sponsor-approved) never decides it: an
// intake row can carry anyone's email with any handle, and the newest such
// row used to steer that person's SSH-key or operator-key request to the
// planted handle (GHSA-mxf4-8rv7-774w). More than one handle refuses.
func (s *Server) handleForLoginEmail(ctx context.Context, email string) (string, error) {
	ps, err := s.Onboard.ProposalsForEmail(ctx, email)
	if err != nil {
		return "", errors.New("could not look up your onboarding record")
	}
	goatnet := ""
	if s.Cfg != nil {
		goatnet = strings.TrimSpace(s.Cfg.NotifyGoatnet)
	}
	settled, handles := 0, []string{}
	for _, p := range ps {
		if !onboardstore.ProposalNonTerminal(p.Status) || onboardstore.ProposalPending(p.Status) {
			continue
		}
		settled++
		if goatnet == "" || strings.TrimSpace(p.Env) != goatnet {
			if settled == 1 {
				// The newest settled record is another goatnet's: fail closed
				// as a missing mapping rather than fall back to an older one.
				return "", onboardstore.ErrNotFound
			}
			continue
		}
		if strings.TrimSpace(p.Handle) == "" {
			continue
		}
		handles = append(handles, p.Handle)
	}
	handles = uniqueFolded(handles)
	switch {
	case len(handles) == 1:
		return handles[0], nil
	case len(handles) > 1:
		return "", errAmbiguousRosterHandle
	case settled > 0:
		return "", onboardstore.ErrNotFound
	}
	// Legacy backfill path: fall back to an installed ssh-key row keyed by this
	// login email. The mapping is admin-confirmed when the backfill runs; we
	// refuse ambiguous cases rather than guessing.
	handle, err := s.legacySSHKeyHandleForEmail(ctx, email)
	if err != nil {
		return "", errors.New("could not look up your onboarding record")
	}
	if handle != "" {
		return handle, nil
	}
	return "", onboardstore.ErrNotFound
}

// sessionRosterHandle is the signed-in person's roster handle, bound to the
// session's Kratos identity: every address the identity is known by now
// (actorAddresses: the session email plus the identity's current addresses)
// resolves through handleForLoginEmail, and they must agree on exactly one
// handle. None is ErrNotFound; two refuse. A session whose identity cannot be
// read refuses.
func (s *Server) sessionRosterHandle(ctx context.Context, actor Actor) (string, error) {
	addrs, err := s.actorAddresses(ctx, actor)
	if err != nil {
		return "", err
	}
	var handles []string
	seen := map[string]bool{}
	for _, a := range addrs {
		k := selfcheck.EmailKey(a)
		if strings.TrimSpace(a) == "" || seen[k] {
			continue
		}
		seen[k] = true
		h, herr := s.handleForLoginEmail(ctx, strings.TrimSpace(a))
		switch {
		case herr == nil:
			handles = append(handles, h)
		case errors.Is(herr, onboardstore.ErrNotFound):
		default:
			return "", herr
		}
	}
	handles = uniqueFolded(handles)
	switch len(handles) {
	case 0:
		return "", onboardstore.ErrNotFound
	case 1:
		return handles[0], nil
	}
	return "", errAmbiguousRosterHandle
}

func (s *Server) legacySSHKeyHandleForEmail(ctx context.Context, email string) (string, error) {
	email = strings.ToLower(strings.TrimSpace(email))
	if email == "" {
		return "", nil
	}
	// Same trust scope as the install-panel gate: only installed rows recorded
	// for THIS console's goatnet may vouch for an email→handle mapping. The
	// store is env-agnostic and CLI submits accept any slug, so without this
	// filter a row minted for another goatnet would resolve a handle here.
	// No configured goatnet → no legacy join, fail closed.
	installEnv := ""
	if s.Cfg != nil {
		installEnv = strings.TrimSpace(s.Cfg.NotifyGoatnet)
	}
	if installEnv == "" {
		return "", nil
	}
	ks, err := s.Onboard.ListSSHKeys(ctx, onboardstore.SSHKeyInstalled)
	if err != nil {
		return "", err
	}
	var handle string
	for _, k := range ks {
		if strings.ToLower(strings.TrimSpace(k.Email)) != email {
			continue
		}
		if strings.TrimSpace(k.Env) != installEnv {
			continue
		}
		if handle == "" {
			handle = k.Handle
			continue
		}
		if handle != k.Handle {
			// Ambiguous legacy mapping; refuse rather than guessing.
			return "", nil
		}
	}
	return handle, nil
}

// mySSHKeys returns only the signed-in person's rows.
func (s *Server) mySSHKeys(r *http.Request, handle string) ([]onboardstore.SSHKey, error) {
	all, err := s.Onboard.ListSSHKeys(r.Context(), "")
	if err != nil {
		return nil, err
	}
	mine := []onboardstore.SSHKey{}
	for _, k := range all {
		if k.Handle == handle {
			mine = append(mine, k)
		}
	}
	return mine, nil
}
