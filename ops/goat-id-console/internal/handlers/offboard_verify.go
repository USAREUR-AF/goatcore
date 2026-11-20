package handlers

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/onboardstore"
)

// offboard_verify.go — the offboard-verification view (goat#319): a
// read-only, admin-gated page that answers "is this person actually gone?"
// across the four identity planes. Verification is a different job from the
// cut (#260 / the CLI's revoke): the planes don't stay in sync on their own,
// and residue — an offboarded admin still holding an op-key or a shell key —
// is the known failure mode this page exists to make visible.
//
// Honesty rules, in order of importance:
//   - No silent green. A plane the console cannot read renders "unknown"
//     and the verdict degrades to "unverified" — an outage never looks
//     like a completed offboard.
//   - The receipt is what the server can actually know. The CLI's rich
//     revoke outcome is age-encrypted to the operator roster on the fabric
//     by design; the console reconstructs provenance from its own proposal
//     row instead of pretending to read what it cannot.

// Plane and verdict vocabulary — rendered as data attributes so the page's
// claims are machine-checkable (and test-pinned) independent of prose.
const (
	planePresent = "present"
	planeAbsent  = "absent"
	planeUnknown = "unknown"

	verdictLingering  = "lingering"
	verdictUnverified = "unverified"
	verdictClear      = "clear"

	receiptOffboarded    = "offboarded"
	receiptNotOffboarded = "not-offboarded"
	receiptNone          = "none"
)

// offboardPlane is one row of the view: a plane, its still-present flag, and
// the evidence line backing the flag.
type offboardPlane struct {
	Key    string // login | operator-keys | ssh-keys | mesh | fabric
	Label  string
	Status string // present | absent | unknown
	Detail string
}

// offboardVerifyVars is the page payload.
type offboardVerifyVars struct {
	Person  string
	Planes  []offboardPlane
	Verdict string
	// Receipt provenance from the console's own proposal row.
	Receipt     string // offboarded | not-offboarded | none
	ReceiptBy   string
	ReceiptAt   int64
	ReceiptNote string
	RowStatus   string
}

// personPresence mirrors the portal's per-person WhoResult slice
// (GET /api/lookup/person-presence, goat#319).
type personPresence struct {
	Resolved        string `json:"resolved"`
	OnMesh          bool   `json:"on_mesh"`
	MeshAttribution string `json:"mesh_attribution"`
	Connected       bool   `json:"connected"`
	Writes          []struct {
		Topic string `json:"topic"`
	} `json:"writes"`
}

// handleOffboardVerify renders GET /offboard-verify?person=<email|handle>.
func (s *Server) handleOffboardVerify(w http.ResponseWriter, r *http.Request) {
	person := strings.TrimSpace(r.URL.Query().Get("person"))
	vars := &pageVars{Title: "Offboard verification", OffboardVerify: &offboardVerifyVars{Person: person}}
	if person == "" {
		s.render(w, r, "offboard_verify", vars)
		return
	}
	ov := vars.OffboardVerify
	ctx := r.Context()

	// Console record first: it is the receipt AND the email<->handle join for
	// the credential-store planes.
	var email, handle string
	if strings.Contains(person, "@") {
		email = person
	} else {
		handle = person
	}
	p, found, err := s.Onboard.LatestProposalFor(ctx, person)
	switch {
	case err != nil:
		ov.Receipt = receiptNone
		ov.ReceiptNote = "console record unreadable: " + err.Error()
	case !found:
		ov.Receipt = receiptNone
	case p.Status == onboardstore.ProposalOffboarded:
		ov.Receipt = receiptOffboarded
		if p.DecidedBy != nil {
			ov.ReceiptBy = *p.DecidedBy
		}
		if p.DecidedAt != nil {
			ov.ReceiptAt = *p.DecidedAt
		}
		ov.ReceiptNote = p.Note
	default:
		ov.Receipt = receiptNotOffboarded
		ov.RowStatus = p.Status
	}
	if found {
		if email == "" {
			email = p.Email
		}
		if handle == "" {
			handle = p.Handle
		}
	}

	login := s.loginPlane(ctx, email)
	opKeys := s.operatorKeyPlane(ctx, handle, email)
	sshKeys := s.sshKeyPlane(ctx, handle, email)
	mesh, fabric := s.portalPlanes(ctx, person)

	ov.Planes = []offboardPlane{login, opKeys, sshKeys, mesh, fabric}
	ov.Verdict = verdictClear
	for _, pl := range ov.Planes {
		if pl.Status == planeUnknown && ov.Verdict != verdictLingering {
			ov.Verdict = verdictUnverified
		}
		if pl.Status == planePresent {
			ov.Verdict = verdictLingering
		}
	}
	s.render(w, r, "offboard_verify", vars)
}

// loginPlane checks the Ory identity plane. Disabled-but-retained is absent
// with a note: revocation is deliberately reversible (mesh-first), so a
// deactivated identity is a completed cut, not residue.
func (s *Server) loginPlane(ctx context.Context, email string) offboardPlane {
	pl := offboardPlane{Key: "login", Label: "Login (Ory identity)"}
	if email == "" {
		pl.Status = planeUnknown
		pl.Detail = "no email on record — the login plane cannot be checked by handle alone"
		return pl
	}
	cli := s.Kratos[s.Cfg.DefaultIdP]
	if cli == nil {
		pl.Status = planeUnknown
		pl.Detail = "no login-plane client configured"
		return pl
	}
	ids, err := cli.ListIdentities(ctx)
	if err != nil {
		pl.Status = planeUnknown
		pl.Detail = "login plane unreachable: " + err.Error()
		return pl
	}
	for _, id := range ids {
		if !strings.EqualFold(id.Email(), email) {
			continue
		}
		if strings.EqualFold(id.State, "active") {
			pl.Status = planePresent
			pl.Detail = "an ACTIVE login identity still exists"
			return pl
		}
		pl.Status = planeAbsent
		pl.Detail = "identity exists but is deactivated (retained for reversibility)"
		return pl
	}
	pl.Status = planeAbsent
	pl.Detail = "no login identity matches this email"
	return pl
}

// matchesPerson says whether a credential row belongs to the person under
// either identity form. Blank forms never match.
func matchesPerson(rowHandle, rowEmail, handle, email string) bool {
	if handle != "" && strings.EqualFold(rowHandle, handle) {
		return true
	}
	if email != "" && strings.EqualFold(rowEmail, email) {
		return true
	}
	return false
}

// operatorKeyPlane checks the console's own op_-key rows. Approval is the
// grant (the portal hot-reloads its key dir), so `active` is the one live
// state; requested/rejected/revoked rows grant nothing.
func (s *Server) operatorKeyPlane(ctx context.Context, handle, email string) offboardPlane {
	pl := offboardPlane{Key: "operator-keys", Label: "Console-issued operator keys (portal op_)"}
	rows, err := s.Onboard.ListOperatorKeys(ctx, onboardstore.OperatorKeyActive)
	if err != nil {
		pl.Status = planeUnknown
		pl.Detail = "operator-key store unreadable: " + err.Error()
		return pl
	}
	live := 0
	for _, k := range rows {
		if matchesPerson(k.Handle, k.Email, handle, email) {
			live++
		}
	}
	if live > 0 {
		pl.Status = planePresent
		pl.Detail = fmt.Sprintf("%d live operator key(s) — the portal still accepts them", live)
		return pl
	}
	// GHSA-2w7v: a revoke is verifiable against an env pin on the portal
	// (PORTAL_OPERATOR_API_KEYS) only if the revocation tombstone is on disk.
	// A revoked row without one (revoked before tombstones existed, or the
	// file was deleted) is not verifiably revoked.
	if dir := s.operatorKeyDir(); dir.Managed() {
		revoked, rerr := s.Onboard.ListOperatorKeys(ctx, onboardstore.OperatorKeyRevoked)
		if rerr != nil {
			pl.Status = planeUnknown
			pl.Detail = "operator-key store unreadable: " + rerr.Error()
			return pl
		}
		var missing []string
		for _, k := range revoked {
			// Only a key the console issued gets a tombstone; a withdrawn
			// request never granted anything.
			if !k.Issued() || !matchesPerson(k.Handle, k.Email, handle, email) {
				continue
			}
			ok, terr := dir.Tombstoned(k.Handle)
			if terr != nil || !ok {
				missing = append(missing, k.Handle)
			}
		}
		if len(missing) > 0 {
			pl.Status = planeUnknown
			pl.Detail = fmt.Sprintf("not verifiably revoked: no revocation tombstone for %s, so an env-pinned key for that handle would still be accepted by the portal; run `goat admin operator-key reconcile --apply` to write it",
				strings.Join(missing, ", "))
			return pl
		}
	}
	pl.Status = planeAbsent
	// The console cannot read the portal's env tier: an env-pinned key for a
	// handle the console never issued or revoked is not visible here.
	pl.Detail = "no live console-issued operator keys, and every revoked one is tombstoned. Env-pinned keys (PORTAL_OPERATOR_API_KEYS) under any other handle are NOT checked: the console cannot see the portal's env tier"
	return pl
}

// sshKeyPlane checks the shell plane. Approved counts the same as installed:
// approval is the decision the installer acts on, so an approved key is
// standing authorization even before it lands in authorized_keys.
func (s *Server) sshKeyPlane(ctx context.Context, handle, email string) offboardPlane {
	pl := offboardPlane{Key: "ssh-keys", Label: "SSH keys (host shell)"}
	rows, err := s.Onboard.ListSSHKeys(ctx, "")
	if err != nil {
		pl.Status = planeUnknown
		pl.Detail = "ssh-key store unreadable: " + err.Error()
		return pl
	}
	live := 0
	for _, k := range rows {
		if k.Status != onboardstore.SSHKeyApproved && k.Status != onboardstore.SSHKeyInstalled {
			continue
		}
		if matchesPerson(k.Handle, k.Email, handle, email) {
			live++
		}
	}
	if live > 0 {
		pl.Status = planePresent
		pl.Detail = fmt.Sprintf("%d approved/installed SSH key(s)", live)
		return pl
	}
	pl.Status = planeAbsent
	pl.Detail = "no approved or installed SSH keys"
	return pl
}

// portalPlanes asks the portal for the person's mesh + fabric slice. Any
// failure to get an answer marks BOTH planes unknown — the portal is the
// only reader of those layers, and unknown is the honest rendering of
// "could not look".
func (s *Server) portalPlanes(ctx context.Context, person string) (mesh, fabric offboardPlane) {
	mesh = offboardPlane{Key: "mesh", Label: "Mesh (enrolled peers)"}
	fabric = offboardPlane{Key: "fabric", Label: "Fabric (Zenoh write grants)"}
	base := strings.TrimRight(strings.TrimSpace(s.Cfg.PortalInternalURL), "/")
	token := strings.TrimSpace(s.Cfg.PortalLookupToken)
	if base == "" || token == "" {
		mesh.Status, fabric.Status = planeUnknown, planeUnknown
		mesh.Detail = "portal lookup not configured"
		fabric.Detail = mesh.Detail
		return mesh, fabric
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, //nolint:gosec // G704: host is operator config (PortalInternalURL); person is only a QueryEscape'd query value
		base+"/api/lookup/person-presence?id="+url.QueryEscape(person), nil)
	if err != nil {
		mesh.Status, fabric.Status = planeUnknown, planeUnknown
		mesh.Detail = "portal lookup failed: " + err.Error()
		fabric.Detail = mesh.Detail
		return mesh, fabric
	}
	req.Header.Set("Authorization", "Bearer "+token)
	resp, err := onboardHTTP.Do(req) //nolint:gosec // G704: same request as above — destination fixed by config
	if err != nil {
		mesh.Status, fabric.Status = planeUnknown, planeUnknown
		mesh.Detail = "portal unreachable: " + err.Error()
		fabric.Detail = mesh.Detail
		return mesh, fabric
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		mesh.Status, fabric.Status = planeUnknown, planeUnknown
		mesh.Detail = fmt.Sprintf("portal lookup returned %d", resp.StatusCode)
		fabric.Detail = mesh.Detail
		return mesh, fabric
	}
	var pp personPresence
	if err := json.NewDecoder(io.LimitReader(resp.Body, 1<<20)).Decode(&pp); err != nil {
		mesh.Status, fabric.Status = planeUnknown, planeUnknown
		mesh.Detail = "portal response unreadable: " + err.Error()
		fabric.Detail = mesh.Detail
		return mesh, fabric
	}

	if pp.Resolved == "" {
		// Nothing in the access graph matched — no roster row, no peer, no
		// grant. For a verifier that IS the good answer, and it is a real
		// absence (the graph reads the live layers), not a can't-know.
		mesh.Status = planeAbsent
		mesh.Detail = "not in the access graph — no enrolled peer attributes to this person"
		fabric.Status = planeAbsent
		fabric.Detail = "not in the access graph — no write grants"
		return mesh, fabric
	}

	switch pp.MeshAttribution {
	case "peer":
		if pp.OnMesh {
			mesh.Status = planePresent
			mesh.Detail = "the person's own enrolled device is on the mesh"
		} else {
			mesh.Status = planeAbsent
			mesh.Detail = "the person's device(s) are attributed but not on the mesh"
		}
	default:
		// "org" answers for the org, not the person; "none"/empty has nothing
		// to attribute. Neither is a confident no (identity-access reference:
		// a device enrolled under a non-conventional name is invisible here).
		mesh.Status = planeUnknown
		mesh.Detail = "no device attributes to this person — unattributed is not the same as absent"
	}

	if len(pp.Writes) > 0 {
		fabric.Status = planePresent
		fabric.Detail = fmt.Sprintf("%d write grant(s) still reachable through the org spine", len(pp.Writes))
	} else {
		fabric.Status = planeAbsent
		fabric.Detail = "no write grants"
	}
	return mesh, fabric
}
