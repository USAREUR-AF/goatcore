package api

// ADR 1251 additions to machine enrollment (single and bulk): keyless sources
// (§7.3) and carriage rules requested at enrollment (§7.5).

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

var receiveInterfaceRe = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,31}$`)

// validateCarriageSpec checks the ADR 1251 fields before anything is
// allocated, so a refusal costs no slot and no quota.
func (s *Server) validateCarriageSpec(ctx context.Context, spec machineSpec) *mintFailure {
	bad := func(msg string) *mintFailure {
		return &mintFailure{http.StatusBadRequest, "invalid_keyless", msg}
	}
	for _, p := range spec.Pods {
		if !carriageSlotRe.MatchString(p) {
			return &mintFailure{http.StatusBadRequest, "invalid_pods", fmt.Sprintf("pods entry %q is not a 32-hex pod slot", p)}
		}
	}
	if !spec.Keyless {
		if len(spec.Adapters) > 0 || spec.Receive != nil {
			return bad("adapters and receive describe a keyless source; set keyless: true or leave them out")
		}
		return nil
	}
	if len(spec.Workloads) > 0 {
		return bad("a keyless source has no workloads")
	}
	if len(spec.Adapters) == 0 {
		return bad("a keyless source names at least one ingress adapter (adapters: [<adapter slot>])")
	}
	seen := map[string]bool{}
	for _, a := range spec.Adapters {
		if !carriageSlotRe.MatchString(a) {
			return bad(fmt.Sprintf("adapter %q is not a 32-hex slot", a))
		}
		if seen[a] {
			return bad(fmt.Sprintf("adapter %s is listed twice", a))
		}
		seen[a] = true
		if s.liveSlot(ctx, a) == nil {
			return bad(fmt.Sprintf("adapter %s is not an enrolled machine; enroll the adapter first", a))
		}
	}
	rc := spec.Receive
	if rc == nil {
		return bad("a keyless source needs receive: {interface, port, sender}")
	}
	if !receiveInterfaceRe.MatchString(rc.Interface) {
		return bad("receive.interface is required (letters, digits, '.', '_', '-'), e.g. udp")
	}
	if rc.Port < 1 || rc.Port > 65535 {
		return bad("receive.port must be 1-65535")
	}
	if !validSender(rc.Sender) {
		return bad("receive.sender must be an IP address or IP:port, the only sender the adapter accepts")
	}
	return nil
}

func validSender(v string) bool {
	if net.ParseIP(v) != nil {
		return true
	}
	host, port, err := net.SplitHostPort(v)
	if err != nil || net.ParseIP(host) == nil {
		return false
	}
	n, err := strconv.Atoi(port)
	return err == nil && n >= 1 && n <= 65535
}

// enrollKeyless finishes a keyless enrollment: the slot and custody edge
// exist; record how the source is received, and a credential-less org row so
// its topics can be catalogued. No key, certificate, API key or mesh key.
func (s *Server) enrollKeyless(ctx context.Context, actor, custodian string, grant *registry.MachineGrant, spec machineSpec, anchor string) (*enrollMachineResponse, *mintFailure) {
	if err := s.reg.RecordKeylessSource(ctx, registry.KeylessSource{
		Slot: anchor, Adapters: strings.Join(spec.Adapters, ","), Interface: spec.Receive.Interface,
		Port: spec.Receive.Port, Sender: spec.Receive.Sender, CreatedBy: actor,
	}); err != nil {
		_ = s.reg.RevokeSlot(ctx, anchor)
		return nil, &mintFailure{http.StatusInternalServerError, "record_keyless", err.Error()}
	}
	display := spec.DisplayName
	if display == "" {
		display = spec.MachineHandle
	}
	now := time.Now().Unix()
	if err := s.reg.CreateVendor(ctx, &registry.Vendor{
		Prefix: anchor, DisplayName: display, ApprovedAt: &now, Approver: &actor,
	}); err != nil {
		_ = s.reg.RevokeSlot(ctx, anchor)
		return nil, &mintFailure{http.StatusInternalServerError, "create", err.Error()}
	}
	_ = s.reg.LogAuditEvent(ctx, actor, "machine-enroll", spec.MachineHandle,
		fmt.Sprintf(`{"anchor":%q,"custodian":%q,"node_class":%q,"autonomy":%q,"keyless":true,"adapters":%q}`,
			anchor, custodian, spec.NodeClass, spec.Autonomy, strings.Join(spec.Adapters, ",")))
	return &enrollMachineResponse{
		SlotID:       anchor,
		SystemHandle: spec.MachineHandle,
		Custodian:    custodian,
		NodeClass:    spec.NodeClass,
		Autonomy:     spec.Autonomy,
		Affiliation:  grant.Affiliation,
		LeafStatus:   "none",
		MeshGroups:   []string{},
		MeshNote:     "keyless source: no mesh key, it is not a mesh peer; its adapters are.",
		NoteToShare: "Keyless source (ADR 1251 §7.3): no key, certificate or API key was minted. " +
			"Its ingress adapters publish under this slot and sign adapter-attested provenance; a pod carries it through rules naming this slot.",
		Keyless:  true,
		Adapters: spec.Adapters,
		Receive:  spec.Receive,
	}, nil
}

// requestEnrollCarriage creates the named rules a new machine asked for, one
// per pod, with the same consent rule as POST /api/carriage/rules.
func (s *Server) requestEnrollCarriage(r *http.Request, resp *enrollMachineResponse, pods []string) {
	if resp == nil || len(pods) == 0 {
		return
	}
	until := time.Now().Add(s.carriageLimits().MaxRule).UTC().Format(time.RFC3339)
	for _, pod := range pods {
		res := enrollCarriageResult{PodSlot: pod}
		c, _, msg := s.createCarriageRule(r, createRuleRequest{
			SourceSlot: resp.SlotID, PodSlot: pod, Kind: registry.CarriageKindNamed,
			Until: until, Reason: "requested at enrollment of " + resp.SystemHandle,
		})
		if c == nil {
			res.ErrorCode, res.Error = msg[0], msg[1]
		} else {
			res.RuleID, res.State = c.ID, c.State
		}
		resp.Carriage = append(resp.Carriage, res)
	}
}
