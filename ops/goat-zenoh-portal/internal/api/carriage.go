package api

// Pod carriage (ADR 1251, docs/design/pod-carriage-and-provenance.md §2, §4,
// §7.1, §7.5): the portal routes that keep carriage rules, render and sign each
// pod's rule generation, take a pod's local-authority ledger, and let an org
// lead ratify or let lapse what a cut-off pod did locally.
//
// Org membership is the data roster's (the same definition governance uses):
// a source belongs to the orgs its own slot or its custodian is declared under.
// The router ACL never sees an org: a generation names certificate CNs only.

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/carriage"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

const (
	defaultCarriageMaxRuleDays      = 90
	defaultCarriageReactivationDays = 183
	maxLedgerEntries                = 1000
)

var (
	carriageSlotRe     = regexp.MustCompile(`^[0-9a-f]{32}$`)
	carriageIdentityRe = regexp.MustCompile(`^[a-z0-9][a-z0-9-]{0,31}:[^\s]{1,128}$`)
	carriageCNRe       = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._:@-]{0,127}$`)
	carriageOrgRe      = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`)
)

// carriageSignerCache loads the configured signer once per process.
type carriageSignerCache struct {
	once   sync.Once
	signer *carriage.Signer
	err    error
}

func (s *Server) carriageSigner() (*carriage.Signer, error) {
	c := &s.carriageSign
	c.once.Do(func() {
		if s.cfg.CarriageSignerKey == "" || s.cfg.CarriageSignerCert == "" {
			c.err = errors.New("carriage signer not configured (PORTAL_CARRIAGE_SIGNER_KEY / PORTAL_CARRIAGE_SIGNER_CERT)")
			return
		}
		c.signer, c.err = carriage.LoadSigner(s.cfg.CarriageSignerKey, s.cfg.CarriageSignerCert)
	})
	return c.signer, c.err
}

func (s *Server) carriageLimits() registry.CarriageLimits {
	maxDays := s.cfg.CarriageMaxRuleDays
	if maxDays <= 0 {
		maxDays = defaultCarriageMaxRuleDays
	}
	window := s.cfg.CarriageReactivationDays
	if window <= 0 {
		window = defaultCarriageReactivationDays
	}
	return registry.CarriageLimits{
		MaxRule:            time.Duration(maxDays) * 24 * time.Hour,
		ReactivationWindow: time.Duration(window) * 24 * time.Hour,
	}
}

// carriageRead is the read set every generation carries; never a bare **.
func (s *Server) carriageRead() ([]string, error) {
	raw := strings.TrimSpace(s.cfg.CarriageRead)
	if raw == "" {
		raw = "*/**"
	}
	var out []string
	for _, k := range strings.Split(raw, ",") {
		k = strings.TrimSpace(k)
		if k == "" {
			continue
		}
		if k == "**" {
			return nil, errors.New("PORTAL_CARRIAGE_READ contains a bare ** (ADR 1123 forbids it)")
		}
		out = append(out, k)
	}
	if len(out) == 0 {
		return nil, errors.New("PORTAL_CARRIAGE_READ is empty")
	}
	return out, nil
}

// carriageOrgs answers which orgs a slot belongs to: the roster orgs of the
// slot itself and of its custodian. No roster configured = no org membership,
// so org rules cover nothing (they never cover everything).
type carriageOrgs func(slotID string, enrolledBy *string) []string

func (s *Server) carriageOrgResolver() (carriageOrgs, error) {
	return carriageOrgResolverFor(s.cfg, &s.rosters)
}

// writeCarriageError maps registry carriage refusals to stable codes. The goat
// CLI's error catalog keys on these codes.
func writeCarriageError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, registry.ErrNotFound):
		writeError(w, http.StatusNotFound, "not_found", err.Error())
	case errors.Is(err, registry.ErrCarriageNoUntil):
		writeError(w, http.StatusBadRequest, "carriage_until_required", err.Error())
	case errors.Is(err, registry.ErrCarriageTooLong):
		writeError(w, http.StatusBadRequest, "carriage_over_max_length", err.Error())
	case errors.Is(err, registry.ErrCarriagePast):
		writeError(w, http.StatusBadRequest, "carriage_until_past", err.Error())
	case errors.Is(err, registry.ErrCarriageExclusion):
		writeError(w, http.StatusBadRequest, "carriage_exclusion_no_end", err.Error())
	case errors.Is(err, registry.ErrCarriageOrgRuleNoEnd):
		writeError(w, http.StatusBadRequest, "carriage_org_rule_no_end", err.Error())
	case errors.Is(err, registry.ErrCarriageBeyondWindow):
		writeError(w, http.StatusConflict, "carriage_beyond_reactivation_window", err.Error())
	case errors.Is(err, registry.ErrCarriageNotArchived):
		writeError(w, http.StatusConflict, "carriage_not_archived", err.Error())
	case errors.Is(err, registry.ErrCarriageReactivatedTwice):
		writeError(w, http.StatusConflict, "carriage_already_reactivated", err.Error())
	case errors.Is(err, registry.ErrCarriageNotActive):
		writeError(w, http.StatusConflict, "carriage_rule_archived", err.Error())
	case errors.Is(err, registry.ErrCarriageNotProposed):
		writeError(w, http.StatusConflict, "carriage_not_proposed", err.Error())
	case errors.Is(err, registry.ErrCarriagePodExists):
		writeError(w, http.StatusConflict, "carriage_pod_org_mismatch", err.Error())
	case errors.Is(err, registry.ErrCarriageLedgerConflict):
		writeError(w, http.StatusConflict, "ledger_conflict", err.Error())
	case errors.Is(err, registry.ErrCarriageLocalNotPending):
		writeError(w, http.StatusConflict, "local_rule_not_pending", err.Error())
	case errors.Is(err, registry.ErrInvalid):
		writeError(w, http.StatusBadRequest, "invalid", err.Error())
	default:
		writeError(w, http.StatusInternalServerError, "db", err.Error())
	}
}

func carriageForbidden(w http.ResponseWriter, msg string) {
	writeError(w, http.StatusForbidden, "carriage_not_authorized", msg)
}

// liveSlot returns a slot that is assigned and not revoked, or nil.
func (s *Server) liveSlot(ctx context.Context, slotID string) *registry.Slot {
	sl, err := s.reg.SlotByID(ctx, slotID)
	if err != nil || sl.AssignedTo == nil || *sl.AssignedTo == "" || sl.RevokedAt != nil {
		return nil
	}
	return sl
}

// isSlotCustodian reports whether the caller may act for a slot: an operator,
// the slot's own holder, or the custodian that enrolled it.
func (s *Server) isSlotCustodian(r *http.Request, slotID string) bool {
	p := PrincipalFromContext(r.Context())
	if p == nil {
		return false
	}
	if p.Operator {
		return true
	}
	if p.Vendor == "" {
		return false
	}
	if p.Vendor == slotID {
		return true
	}
	sl, err := s.reg.SlotByID(r.Context(), slotID)
	if err != nil || sl.EnrolledBy == nil || *sl.EnrolledBy == "" {
		return false
	}
	if strings.EqualFold(*sl.EnrolledBy, p.Vendor) {
		return true
	}
	if a, err := s.reg.AnchorForHandle(r.Context(), *sl.EnrolledBy); err == nil && a == p.Vendor {
		return true
	}
	return false
}

// isSourceCustodian: the caller may grant for this rule's source side.
func (s *Server) isSourceCustodian(r *http.Request, c *registry.CarriageRule) bool {
	if c.SourceSlot != nil {
		return s.isSlotCustodian(r, *c.SourceSlot)
	}
	return c.SourceOrg != nil && s.mayGovernOrg(r, *c.SourceOrg)
}

// isPodCustodian: the caller may accept for this rule's pod side.
func (s *Server) isPodCustodian(r *http.Request, podSlot, podOrg string) bool {
	if podOrg != "" {
		return s.mayGovernOrg(r, podOrg)
	}
	if s.isSlotCustodian(r, podSlot) {
		return true
	}
	if pod, err := s.reg.CarriagePod(r.Context(), podSlot); err == nil {
		return s.mayGovernOrg(r, pod.Org)
	}
	return false
}

func parseCarriageTime(v string) (int64, error) {
	t, err := time.Parse(time.RFC3339, strings.TrimSpace(v))
	if err != nil {
		return 0, fmt.Errorf("until %q is not an RFC 3339 time", v)
	}
	return t.Unix(), nil
}

// ── pods ────────────────────────────────────────────────────────────────

type carriagePodView struct {
	registry.CarriagePod
	LocalAuthority []registry.PodAuthority `json:"local_authority"`
	Health         *carriagePodHealth      `json:"health,omitempty"`
}

// carriagePodHealth is what the portal knows of a pod's carriage: the last
// generation it served the pod and how long ago, whether the rules changed
// since, and the pod's last ledger upload (ADR 1251 phase 4).
type carriagePodHealth struct {
	// State: never_pulled (no generation served yet), current (the pod holds
	// what the rules say), behind (the rules changed since its last pull).
	State                 string `json:"state"`
	LastPulledGeneration  *int64 `json:"last_pulled_generation,omitempty"`
	LastPulledAt          *int64 `json:"last_pulled_at,omitempty"`
	PullAgeSeconds        *int64 `json:"pull_age_seconds,omitempty"`
	RulesChangedSincePull bool   `json:"rules_changed_since_pull"`
	LastLedgerAt          *int64 `json:"last_ledger_at,omitempty"`
	LedgerAgeSeconds      *int64 `json:"ledger_age_seconds,omitempty"`
	PendingLocalRules     int64  `json:"pending_local_rules"`
}

func (s *Server) carriagePodHealth(ctx context.Context, pod *registry.CarriagePod, pending, now int64) (*carriagePodHealth, error) {
	h := &carriagePodHealth{
		State: "never_pulled", LastPulledGeneration: pod.LastPulledGeneration, LastPulledAt: pod.LastPulledAt,
		LastLedgerAt: pod.LastLedgerAt, PendingLocalRules: pending,
	}
	if pod.LastLedgerAt != nil {
		age := now - *pod.LastLedgerAt
		h.LedgerAgeSeconds = &age
	}
	if pod.LastPulledAt == nil {
		return h, nil
	}
	age := now - *pod.LastPulledAt
	h.PullAgeSeconds = &age
	g, err := s.buildGeneration(ctx, pod)
	if err != nil {
		return nil, err
	}
	hash, err := carriage.ContentHash(g)
	if err != nil {
		return nil, err
	}
	h.RulesChangedSincePull = hash != pod.ContentHash || pod.LastPulledGeneration == nil || *pod.LastPulledGeneration != pod.Generation
	h.State = "current"
	if h.RulesChangedSincePull {
		h.State = "behind"
	}
	return h, nil
}

func podView(p *registry.CarriagePod) carriagePodView {
	la := p.Authorities()
	if la == nil {
		la = []registry.PodAuthority{}
	}
	return carriagePodView{CarriagePod: *p, LocalAuthority: la}
}

type registerPodRequest struct {
	PodSlot        string                  `json:"pod_slot"`
	Org            string                  `json:"org"`
	LocalAuthority []registry.PodAuthority `json:"local_authority"`
}

// handleRegisterCarriagePod serves POST /api/carriage/pods (operator).
func (s *Server) handleRegisterCarriagePod(w http.ResponseWriter, r *http.Request) {
	var req registerPodRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_json", err.Error())
		return
	}
	req.PodSlot, req.Org = strings.TrimSpace(req.PodSlot), strings.TrimSpace(req.Org)
	if !carriageSlotRe.MatchString(req.PodSlot) {
		writeError(w, http.StatusBadRequest, "invalid", "pod_slot must be a 32-hex slot id")
		return
	}
	if !carriageOrgRe.MatchString(req.Org) {
		writeError(w, http.StatusBadRequest, "invalid", "org is required (letters, digits, '.', '_', '-')")
		return
	}
	if s.liveSlot(r.Context(), req.PodSlot) == nil {
		writeError(w, http.StatusNotFound, "pod_slot_not_live", "pod_slot is not an assigned slot; enroll the pod as a machine first")
		return
	}
	for i, a := range req.LocalAuthority {
		a.Identity, a.CertCN, a.Slot = strings.TrimSpace(a.Identity), strings.TrimSpace(a.CertCN), strings.TrimSpace(a.Slot)
		if !carriageIdentityRe.MatchString(a.Identity) {
			writeError(w, http.StatusBadRequest, "invalid", fmt.Sprintf("local_authority[%d].identity must be <pki>:<identifier>", i))
			return
		}
		if (a.CertCN == "") == (a.Slot == "") {
			writeError(w, http.StatusBadRequest, "invalid", fmt.Sprintf("local_authority[%d] names exactly one of cert_cn or slot", i))
			return
		}
		if a.CertCN != "" && !carriageCNRe.MatchString(a.CertCN) {
			writeError(w, http.StatusBadRequest, "invalid", fmt.Sprintf("local_authority[%d].cert_cn is not a certificate CN", i))
			return
		}
		if a.Slot != "" && s.liveSlot(r.Context(), a.Slot) == nil {
			writeError(w, http.StatusBadRequest, "invalid", fmt.Sprintf("local_authority[%d].slot is not an assigned slot", i))
			return
		}
		req.LocalAuthority[i] = a
	}
	pod, rule, err := s.reg.RegisterCarriagePod(r.Context(), req.PodSlot, req.Org, req.LocalAuthority, actorOf(r))
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	ruleID := ""
	if rule != nil {
		ruleID = rule.ID
	}
	s.syncSlotFragment(r.Context()) // the core's carry-<anchor> subjects follow the rules (ADR 1251 phase 4)
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "carriage.pod.register", req.PodSlot,
		fmt.Sprintf(`{"org":%q,"org_rule":%q,"local_authority":%d}`, req.Org, ruleID, len(req.LocalAuthority)))
	writeJSON(w, http.StatusCreated, map[string]any{"pod": podView(pod), "org_rule": rule})
}

// handleListCarriagePods serves GET /api/carriage/pods.
func (s *Server) handleListCarriagePods(w http.ResponseWriter, r *http.Request) {
	pods, err := s.reg.CarriagePods(r.Context())
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	pending, err := s.reg.PendingLocalRuleCounts(r.Context())
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	now := registry.CarriageClock().Unix()
	out := make([]carriagePodView, 0, len(pods))
	for i := range pods {
		v := podView(&pods[i])
		h, err := s.carriagePodHealth(r.Context(), &pods[i], pending[pods[i].PodSlot], now)
		if err != nil {
			writeError(w, http.StatusInternalServerError, "db", err.Error())
			return
		}
		v.Health = h
		out = append(out, v)
	}
	writeJSON(w, http.StatusOK, map[string]any{"pods": out})
}

// ── rules ───────────────────────────────────────────────────────────────

type createRuleRequest struct {
	SourceSlot string `json:"source_slot"`
	SourceOrg  string `json:"source_org"`
	PodSlot    string `json:"pod_slot"`
	PodOrg     string `json:"pod_org"`
	Kind       string `json:"kind"`
	Until      string `json:"until"`
	Reason     string `json:"reason"`
}

// handleCreateCarriageRule serves POST /api/carriage/rules. The caller grants
// as the source's custodian; when the caller is also the pod's custodian the
// rule is active at once, otherwise it waits for the pod's custodian to accept.
func (s *Server) handleCreateCarriageRule(w http.ResponseWriter, r *http.Request) {
	var req createRuleRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_json", err.Error())
		return
	}
	if req.Kind == "" {
		req.Kind = registry.CarriageKindNamed
	}
	resp, code, msg := s.createCarriageRule(r, req)
	if resp == nil {
		writeError(w, code, msg[0], msg[1])
		return
	}
	writeJSON(w, http.StatusCreated, resp)
}

// createCarriageRule is the rule-creation core shared with machine enrollment's
// `pods` field. On refusal it returns nil, the status and [code, message].
func (s *Server) createCarriageRule(r *http.Request, req createRuleRequest) (*registry.CarriageRule, int, [2]string) {
	ctx := r.Context()
	for _, v := range []*string{&req.SourceSlot, &req.SourceOrg, &req.PodSlot, &req.PodOrg, &req.Reason} {
		*v = strings.TrimSpace(*v)
	}
	if req.Kind == registry.CarriageKindOrg {
		return nil, http.StatusBadRequest, [2]string{"invalid", "the org rule is created when a pod is registered for its org (POST /api/carriage/pods)"}
	}
	if req.SourceSlot != "" && !carriageSlotRe.MatchString(req.SourceSlot) {
		return nil, http.StatusBadRequest, [2]string{"invalid", "source_slot must be a 32-hex slot id"}
	}
	if req.PodSlot != "" {
		if _, err := s.reg.CarriagePod(ctx, req.PodSlot); err != nil {
			return nil, http.StatusNotFound, [2]string{"pod_not_registered", "pod_slot is not a registered pod (POST /api/carriage/pods)"}
		}
	}
	if req.SourceSlot != "" && s.liveSlot(ctx, req.SourceSlot) == nil {
		return nil, http.StatusNotFound, [2]string{"source_not_live", "source_slot is not an assigned slot"}
	}
	probe := &registry.CarriageRule{}
	if req.SourceSlot != "" {
		probe.SourceSlot = &req.SourceSlot
	}
	if req.SourceOrg != "" {
		probe.SourceOrg = &req.SourceOrg
	}
	if !s.isSourceCustodian(r, probe) {
		return nil, http.StatusForbidden, [2]string{"carriage_not_authorized", "only the source's custodian (or a governance admin of the source org) may grant carriage"}
	}
	var until int64
	if strings.TrimSpace(req.Until) != "" {
		u, err := parseCarriageTime(req.Until)
		if err != nil {
			return nil, http.StatusBadRequest, [2]string{"invalid", err.Error()}
		}
		until = u
	}
	accepted := ""
	if s.isPodCustodian(r, req.PodSlot, req.PodOrg) {
		accepted = actorOrVendor(r)
	}
	c, err := s.reg.CreateCarriageRule(ctx, registry.NewCarriageRule{
		Kind: req.Kind, SourceSlot: req.SourceSlot, SourceOrg: req.SourceOrg, PodSlot: req.PodSlot, PodOrg: req.PodOrg,
		Until: until, Reason: req.Reason, GrantedBy: actorOrVendor(r), AcceptedBy: accepted,
	}, s.carriageLimits())
	if err != nil {
		rec := httptestRecorder{}
		writeCarriageError(&rec, err)
		return nil, rec.status, [2]string{rec.code, err.Error()}
	}
	s.syncSlotFragment(ctx) // the core's carry-<anchor> subjects follow the rules (ADR 1251 phase 4)
	_ = s.reg.LogAuditEvent(ctx, actorOf(r), "carriage.rule.create", c.ID,
		fmt.Sprintf(`{"kind":%q,"state":%q,"source_slot":%q,"source_org":%q,"pod_slot":%q,"pod_org":%q}`,
			c.Kind, c.State, req.SourceSlot, req.SourceOrg, req.PodSlot, req.PodOrg))
	return c, 0, [2]string{}
}

// httptestRecorder captures the status and code writeCarriageError chose,
// so createCarriageRule can return them instead of writing.
type httptestRecorder struct {
	h      http.Header
	status int
	code   string
}

func (h *httptestRecorder) Header() http.Header {
	if h.h == nil {
		h.h = http.Header{}
	}
	return h.h
}
func (h *httptestRecorder) WriteHeader(s int) { h.status = s }
func (h *httptestRecorder) Write(b []byte) (int, error) {
	var p struct {
		Code string `json:"code"`
	}
	_ = json.Unmarshal(b, &p)
	h.code = p.Code
	return len(b), nil
}

// actorOrVendor names the caller for granted_by / accepted_by: the person
// handle for an operator, the caller's own slot otherwise.
func actorOrVendor(r *http.Request) string {
	if p := PrincipalFromContext(r.Context()); p != nil && !p.Operator && p.Vendor != "" {
		return p.Vendor
	}
	return actorOf(r)
}

// handleAcceptCarriageRule serves POST /api/carriage/rules/{id}/accept: the
// pod's custodian accepts a proposed rule.
func (s *Server) handleAcceptCarriageRule(w http.ResponseWriter, r *http.Request) {
	c, err := s.reg.CarriageRule(r.Context(), r.PathValue("id"))
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	if !s.isPodCustodian(r, deref(c.PodSlot), deref(c.PodOrg)) {
		carriageForbidden(w, "only the pod's custodian (or a governance admin of the pod's org) may accept carriage")
		return
	}
	c, err = s.reg.AcceptCarriageRule(r.Context(), c.ID, actorOrVendor(r))
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	s.syncSlotFragment(r.Context()) // the core's carry-<anchor> subjects follow the rules (ADR 1251 phase 4)
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "carriage.rule.accept", c.ID, "{}")
	writeJSON(w, http.StatusOK, c)
}

// handleListCarriageRules serves GET /api/carriage/rules?pod=&source=&state=.
func (s *Server) handleListCarriageRules(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	f := registry.CarriageRuleFilter{State: q.Get("state"), SourceSlot: q.Get("source")}
	switch f.State {
	case "", registry.CarriageActive, registry.CarriageArchived, registry.CarriageProposed:
	default:
		writeError(w, http.StatusBadRequest, "invalid", "state must be active, archived or proposed")
		return
	}
	if pod := q.Get("pod"); pod != "" {
		f.PodSlot = pod
		if p, err := s.reg.CarriagePod(r.Context(), pod); err == nil {
			f.PodOrg = p.Org
		}
	}
	rules, err := s.reg.ListCarriageRules(r.Context(), f)
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"rules": rules})
}

type untilRequest struct {
	Until string `json:"until"`
}

// archivedRefusal answers a change to a rule that is no longer active, saying
// whether it expired (reactivate it) or was revoked.
func archivedRefusal(w http.ResponseWriter, c *registry.CarriageRule) {
	if c.ArchiveReason != nil && *c.ArchiveReason == registry.CarriageExpired {
		writeError(w, http.StatusConflict, "carriage_rule_expired",
			fmt.Sprintf("rule %s expired; reactivate it (goat carriage reactivate %s) inside the reactivation window", c.ID, c.ID))
		return
	}
	writeError(w, http.StatusConflict, "carriage_rule_archived", fmt.Sprintf("rule %s was revoked; reactivate it or grant a new rule", c.ID))
}

// handleExtendCarriageRule serves POST /api/carriage/rules/{id}/extend.
func (s *Server) handleExtendCarriageRule(w http.ResponseWriter, r *http.Request) {
	var req untilRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_json", err.Error())
		return
	}
	until, err := parseCarriageTime(req.Until)
	if err != nil {
		writeError(w, http.StatusBadRequest, "invalid", err.Error())
		return
	}
	if _, err := s.reg.ArchiveExpiredCarriageRules(r.Context()); err != nil {
		writeCarriageError(w, err)
		return
	}
	c, err := s.reg.CarriageRule(r.Context(), r.PathValue("id"))
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	if !s.isSourceCustodian(r, c) {
		carriageForbidden(w, "only the rule's owner (the source's custodian) may extend it")
		return
	}
	if c.State == registry.CarriageArchived {
		archivedRefusal(w, c)
		return
	}
	c, err = s.reg.ExtendCarriageRule(r.Context(), c.ID, until, s.carriageLimits())
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "carriage.rule.extend", c.ID, fmt.Sprintf(`{"until":%d}`, until))
	writeJSON(w, http.StatusOK, c)
}

// handleRevokeCarriageRule serves POST /api/carriage/rules/{id}/revoke. Either
// custodian may revoke. Pending local rules the revoked rule covered lapse:
// goat's revocations win.
func (s *Server) handleRevokeCarriageRule(w http.ResponseWriter, r *http.Request) {
	c, err := s.reg.CarriageRule(r.Context(), r.PathValue("id"))
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	if c.Kind == registry.CarriageKindExclusion {
		if !s.isSourceCustodian(r, c) {
			carriageForbidden(w, "only the source's custodian may lift an exclusion")
			return
		}
	} else if !s.isSourceCustodian(r, c) && !s.isPodCustodian(r, deref(c.PodSlot), deref(c.PodOrg)) {
		carriageForbidden(w, "only the source's or the pod's custodian may revoke a rule")
		return
	}
	c, err = s.reg.RevokeCarriageRule(r.Context(), c.ID, actorOrVendor(r))
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	lapsed := s.lapseLocalRulesCoveredBy(r, c)
	s.syncSlotFragment(r.Context()) // the core's carry-<anchor> subjects follow the rules (ADR 1251 phase 4)
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "carriage.rule.revoke", c.ID, fmt.Sprintf(`{"local_rules_lapsed":%d}`, lapsed))
	writeJSON(w, http.StatusOK, map[string]any{"rule": c, "local_rules_lapsed": lapsed})
}

// handleReactivateCarriageRule serves POST /api/carriage/rules/{id}/reactivate.
func (s *Server) handleReactivateCarriageRule(w http.ResponseWriter, r *http.Request) {
	var req untilRequest
	if r.ContentLength != 0 {
		if err := decodeJSON(w, r, &req); err != nil {
			writeBodyError(w, err, "bad_json", err.Error())
			return
		}
	}
	var until int64
	if strings.TrimSpace(req.Until) != "" {
		u, err := parseCarriageTime(req.Until)
		if err != nil {
			writeError(w, http.StatusBadRequest, "invalid", err.Error())
			return
		}
		until = u
	}
	c, err := s.reg.CarriageRule(r.Context(), r.PathValue("id"))
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	if !s.isSourceCustodian(r, c) {
		carriageForbidden(w, "only the rule's owner (the source's custodian) may reactivate it")
		return
	}
	n, err := s.reg.ReactivateCarriageRule(r.Context(), c.ID, until, actorOrVendor(r), s.carriageLimits())
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	s.syncSlotFragment(r.Context()) // the core's carry-<anchor> subjects follow the rules (ADR 1251 phase 4)
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "carriage.rule.reactivate", n.ID, fmt.Sprintf(`{"reactivated_from":%q}`, c.ID))
	writeJSON(w, http.StatusCreated, n)
}

type optOutRequest struct {
	OptOut *bool `json:"opt_out"`
}

// handleCarriageOptOut serves PUT /api/carriage/sources/{slot}/org-rule.
func (s *Server) handleCarriageOptOut(w http.ResponseWriter, r *http.Request) {
	slot := r.PathValue("slot")
	var req optOutRequest
	if err := decodeJSON(w, r, &req); err != nil {
		writeBodyError(w, err, "bad_json", err.Error())
		return
	}
	if req.OptOut == nil {
		writeError(w, http.StatusBadRequest, "invalid", "opt_out (true or false) is required")
		return
	}
	if s.liveSlot(r.Context(), slot) == nil {
		writeError(w, http.StatusNotFound, "source_not_live", "the slot is not an assigned slot")
		return
	}
	if !s.isSlotCustodian(r, slot) {
		carriageForbidden(w, "only the source's custodian may opt it out of the org rule")
		return
	}
	if err := s.reg.SetCarriageOptOut(r.Context(), slot, *req.OptOut, actorOrVendor(r)); err != nil {
		writeCarriageError(w, err)
		return
	}
	s.syncSlotFragment(r.Context()) // the core's carry-<anchor> subjects follow the rules (ADR 1251 phase 4)
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "carriage.source.opt-out", slot, fmt.Sprintf(`{"opt_out":%t}`, *req.OptOut))
	writeJSON(w, http.StatusOK, map[string]any{"source_slot": slot, "opt_out": *req.OptOut})
}

// ── coverage and the generation ─────────────────────────────────────────

// ruleCovers reports whether rule c lets pod carry source.
func ruleCovers(c *registry.CarriageRule, source string, sourceOrgs []string, pod *registry.CarriagePod, optedOut bool) bool {
	if c.Kind == registry.CarriageKindExclusion {
		return false // an exclusion takes coverage away; it never grants it
	}
	if c.PodSlot != nil && *c.PodSlot != pod.PodSlot {
		return false
	}
	if c.PodOrg != nil && *c.PodOrg != pod.Org {
		return false
	}
	if c.SourceSlot != nil {
		return *c.SourceSlot == source
	}
	if c.SourceOrg == nil {
		return false
	}
	if c.Kind == registry.CarriageKindOrg && optedOut {
		return false
	}
	for _, o := range sourceOrgs {
		if o == *c.SourceOrg {
			return true
		}
	}
	return false
}

// buildGeneration renders the unsigned generation for one pod.
func (s *Server) buildGeneration(ctx context.Context, pod *registry.CarriagePod) (carriage.Generation, error) {
	read, err := s.carriageRead()
	if err != nil {
		return carriage.Generation{}, err
	}
	sn, err := loadCarriageSnapshot(ctx, s.reg, s.cfg, &s.rosters)
	if err != nil {
		return carriage.Generation{}, err
	}
	g := carriage.Generation{
		Schema:         carriage.Schema,
		PodSlot:        pod.PodSlot,
		LocalAuthority: []carriage.Authority{},
		Read:           read,
		Sources:        sn.podSources(pod),
		LocalDecisions: []carriage.LocalDecision{},
	}
	decided, err := s.reg.ListLocalRules(ctx, "", pod.PodSlot)
	if err != nil {
		return carriage.Generation{}, err
	}
	for _, lr := range decided {
		if lr.State != registry.LocalPending {
			g.LocalDecisions = append(g.LocalDecisions, carriage.LocalDecision{Seq: lr.Seq, LocalRuleID: lr.ID, State: lr.State})
		}
	}
	for _, a := range pod.Authorities() {
		cn := a.CertCN
		if a.Slot != "" {
			sl, ok := sn.bySlot[a.Slot]
			if !ok {
				continue // the authority's slot was revoked: they can no longer sign in with it
			}
			cn = registry.LeafCN(sl.SlotID, sl.Generation)
		}
		g.LocalAuthority = append(g.LocalAuthority, carriage.Authority{Identity: a.Identity, CertCN: cn})
	}
	return g, nil
}

// requirePodPrincipal admits only the pod's own key: the bearer the portal
// issued to the pod's slot. Operators are refused too; the document is the
// pod's.
func (s *Server) requirePodPrincipal(w http.ResponseWriter, r *http.Request) (*registry.CarriagePod, bool) {
	podSlot := r.PathValue("pod_slot")
	p := PrincipalFromContext(r.Context())
	if p == nil || p.Operator || p.Vendor != podSlot {
		carriageForbidden(w, "only the pod's own key may use this route")
		return nil, false
	}
	pod, err := s.reg.CarriagePod(r.Context(), podSlot)
	if err != nil {
		if errors.Is(err, registry.ErrNotFound) {
			writeError(w, http.StatusNotFound, "pod_not_registered", "this slot is not a registered pod")
			return nil, false
		}
		writeCarriageError(w, err)
		return nil, false
	}
	return pod, true
}

// handleCarriageGeneration serves GET /api/carriage/pods/{pod_slot}/generation.
func (s *Server) handleCarriageGeneration(w http.ResponseWriter, r *http.Request) {
	pod, ok := s.requirePodPrincipal(w, r)
	if !ok {
		return
	}
	signer, err := s.carriageSigner()
	if err != nil {
		writeError(w, http.StatusServiceUnavailable, "config", err.Error())
		return
	}
	g, err := s.buildGeneration(r.Context(), pod)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	hash, err := carriage.ContentHash(g)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	gen, issued, err := s.reg.IssueCarriageGeneration(r.Context(), pod.PodSlot, hash)
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	if err := s.reg.RecordCarriagePull(r.Context(), pod.PodSlot, gen); err != nil {
		writeCarriageError(w, err)
		return
	}
	g.Generation = gen
	g.IssuedAt = carriage.FormatTime(time.Unix(issued, 0))
	body, err := signer.Sign(g)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(body)
}

// ── ledger, ratification ────────────────────────────────────────────────

type ledgerUpload struct {
	PodCert string            `json:"pod_cert"`
	Entries []json.RawMessage `json:"entries"`
}

type ledgerEntryResult struct {
	Seq         int64  `json:"seq"`
	LocalRuleID string `json:"local_rule_id"`
	State       string `json:"state"`
	Duplicate   bool   `json:"duplicate,omitempty"`
	Note        string `json:"note,omitempty"`
}

func (s *Server) caRoots() (*x509.CertPool, error) {
	if s.cfg.CARootCert == "" {
		return nil, errors.New("no CA root configured (PORTAL_CA_ROOT_CERT)")
	}
	raw, err := os.ReadFile(s.cfg.CARootCert)
	if err != nil {
		return nil, err
	}
	pool := x509.NewCertPool()
	if !pool.AppendCertsFromPEM(raw) {
		return nil, errors.New("the CA root file holds no certificate")
	}
	return pool, nil
}

// revokedFrom reports why goat no longer lets pod carry source, or "".
// Goat's revocations win: a source whose slot was revoked, or whose most
// recent central event covering this pod is a revoke, lapses every local rule.
func (s *Server) revokedFrom(ctx context.Context, source string, pod *registry.CarriagePod, orgsOf carriageOrgs) (string, error) {
	sl, err := s.reg.SlotByID(ctx, source)
	if err != nil || sl.AssignedTo == nil || sl.RevokedAt != nil {
		return "goat revoked the source's slot", nil
	}
	rules, err := s.reg.ListCarriageRules(ctx, registry.CarriageRuleFilter{PodSlot: pod.PodSlot, PodOrg: pod.Org})
	if err != nil {
		return "", err
	}
	optOuts, err := s.reg.CarriageOptOuts(ctx)
	if err != nil {
		return "", err
	}
	orgs := orgsOf(source, sl.EnrolledBy)
	var lastRevoke, lastActive int64
	revokedRule := ""
	for i := range rules {
		c := &rules[i]
		if !ruleCovers(c, source, orgs, pod, optOuts[source]) {
			continue
		}
		switch {
		case c.State == registry.CarriageActive && c.CreatedAt > lastActive:
			lastActive = c.CreatedAt
		case c.State == registry.CarriageArchived && c.ArchiveReason != nil && *c.ArchiveReason == registry.CarriageRevoked &&
			c.ArchivedAt != nil && *c.ArchivedAt >= lastRevoke:
			lastRevoke, revokedRule = *c.ArchivedAt, c.ID
		}
	}
	if lastRevoke > 0 && lastRevoke >= lastActive {
		return "goat revoked carriage rule " + revokedRule + " for this source on this pod", nil
	}
	return "", nil
}

// handleCarriageLedger serves POST /api/carriage/pods/{pod_slot}/ledger.
func (s *Server) handleCarriageLedger(w http.ResponseWriter, r *http.Request) {
	pod, ok := s.requirePodPrincipal(w, r)
	if !ok {
		return
	}
	ctx := r.Context()
	var up ledgerUpload
	if err := decodeJSON(w, r, &up); err != nil {
		writeBodyError(w, err, "bad_json", err.Error())
		return
	}
	if len(up.Entries) == 0 {
		writeError(w, http.StatusBadRequest, "ledger_invalid", "entries is empty")
		return
	}
	if len(up.Entries) > maxLedgerEntries {
		writeError(w, http.StatusBadRequest, "ledger_invalid", fmt.Sprintf("at most %d entries per upload", maxLedgerEntries))
		return
	}
	roots, err := s.caRoots()
	if err != nil {
		writeError(w, http.StatusServiceUnavailable, "config", err.Error())
		return
	}
	cert, err := carriage.ParseCertPEM([]byte(up.PodCert))
	if err != nil {
		writeError(w, http.StatusBadRequest, "ledger_invalid", "pod_cert: "+err.Error())
		return
	}
	if err := carriage.VerifyChain(cert, roots, time.Now()); err != nil {
		writeError(w, http.StatusBadRequest, "ledger_invalid", err.Error())
		return
	}
	podLive := s.liveSlot(ctx, pod.PodSlot)
	if podLive == nil || cert.Subject.CommonName != registry.LeafCN(pod.PodSlot, podLive.Generation) {
		writeError(w, http.StatusBadRequest, "ledger_invalid", "pod_cert is not the pod slot's current leaf")
		return
	}
	type verified struct {
		e     *carriage.LedgerEntry
		canon []byte
	}
	var entries []verified
	for i, raw := range up.Entries {
		e, canon, err := carriage.VerifyLedgerEntry(raw, cert)
		if err != nil {
			writeError(w, http.StatusBadRequest, "ledger_invalid", fmt.Sprintf("entries[%d]: %v", i, err))
			return
		}
		if e.PodSlot != pod.PodSlot {
			writeError(w, http.StatusBadRequest, "ledger_invalid", fmt.Sprintf("entries[%d] is for pod %q", i, e.PodSlot))
			return
		}
		if !carriageSlotRe.MatchString(e.SourceSlot) {
			writeError(w, http.StatusBadRequest, "ledger_invalid", fmt.Sprintf("entries[%d].source_slot is not a slot id", i))
			return
		}
		entries = append(entries, verified{e, canon})
	}
	orgsOf, err := s.carriageOrgResolver()
	if err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	authorities := pod.Authorities()
	maxRule := int64(s.carriageLimits().MaxRule / time.Second)
	resp := struct {
		Accepted  int                 `json:"accepted"`
		Duplicate int                 `json:"duplicate"`
		Pending   int                 `json:"pending"`
		Lapsed    int                 `json:"lapsed"`
		LastSeq   int64               `json:"last_seq"`
		Entries   []ledgerEntryResult `json:"entries"`
	}{Entries: []ledgerEntryResult{}}
	for _, v := range entries {
		e := v.e
		ts, _ := time.Parse(time.RFC3339, e.TS)
		var untilUnix int64
		if e.Until != "" {
			u, _ := time.Parse(time.RFC3339, e.Until)
			untilUnix = u.Unix()
		}
		state, note := registry.LocalPending, ""
		named := false
		for _, a := range authorities {
			if a.Identity == e.Authority.Identity && (a.CertCN == "" || e.Authority.CertCN == "" || a.CertCN == e.Authority.CertCN) {
				named = true
			}
		}
		switch {
		case !named:
			state, note = registry.LocalLapsed, "not a named local authority of this pod"
		case e.Action != carriage.ActionRevoke && untilUnix > ts.Unix()+maxRule:
			state, note = registry.LocalLapsed, "end date beyond the longest rule length"
		case e.Action != carriage.ActionRevoke:
			why, err := s.revokedFrom(ctx, e.SourceSlot, pod, orgsOf)
			if err != nil {
				writeError(w, http.StatusInternalServerError, "db", err.Error())
				return
			}
			if why != "" {
				state, note = registry.LocalLapsed, why
			}
		}
		lr, dup, err := s.reg.RecordLocalRule(ctx, registry.NewLocalRule{
			PodSlot: pod.PodSlot, Seq: e.Seq, Entry: string(v.canon), Action: e.Action, PodRuleID: e.RuleID,
			SourceSlot: e.SourceSlot, Until: untilUnix, Reason: e.Reason, AuthorityIdentity: e.Authority.Identity,
			AuthorityCN: e.Authority.CertCN, EntryTS: ts.Unix(), State: state, StateNote: note,
		})
		if err != nil {
			writeCarriageError(w, err)
			return
		}
		res := ledgerEntryResult{Seq: e.Seq, LocalRuleID: lr.ID, State: lr.State, Duplicate: dup, Note: lr.StateNote}
		if dup {
			resp.Duplicate++
		} else {
			resp.Accepted++
			if lr.State == registry.LocalLapsed {
				resp.Lapsed++
			} else {
				resp.Pending++
			}
		}
		resp.Entries = append(resp.Entries, res)
	}
	if err := s.reg.RecordCarriageLedger(ctx, pod.PodSlot); err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	if resp.LastSeq, err = s.reg.LastLocalSeq(ctx, pod.PodSlot); err != nil {
		writeError(w, http.StatusInternalServerError, "db", err.Error())
		return
	}
	_ = s.reg.LogAuditEvent(ctx, "pod:"+pod.PodSlot, "carriage.ledger.ingest", pod.PodSlot,
		fmt.Sprintf(`{"accepted":%d,"duplicate":%d,"pending":%d,"lapsed":%d}`, resp.Accepted, resp.Duplicate, resp.Pending, resp.Lapsed))
	writeJSON(w, http.StatusOK, resp)
}

// lapseLocalRulesCoveredBy lapses pending local grants that a just-revoked
// central rule covered. Returns how many lapsed.
func (s *Server) lapseLocalRulesCoveredBy(r *http.Request, c *registry.CarriageRule) int {
	ctx := r.Context()
	pending, err := s.reg.ListLocalRules(ctx, registry.LocalPending, "")
	if err != nil {
		return 0
	}
	orgsOf, err := s.carriageOrgResolver()
	if err != nil {
		return 0
	}
	n := 0
	for _, lr := range pending {
		if lr.Action == carriage.ActionRevoke {
			continue
		}
		pod, err := s.reg.CarriagePod(ctx, lr.PodSlot)
		if err != nil {
			continue
		}
		var enrolledBy *string
		if sl, err := s.reg.SlotByID(ctx, lr.SourceSlot); err == nil {
			enrolledBy = sl.EnrolledBy
		}
		if !ruleCovers(c, lr.SourceSlot, orgsOf(lr.SourceSlot, enrolledBy), pod, false) {
			continue
		}
		if _, err := s.reg.DecideLocalRule(ctx, lr.ID, registry.LocalLapsed, "goat revoked carriage rule "+c.ID, "portal", ""); err == nil {
			n++
		}
	}
	return n
}

// handleListLocalRules serves GET /api/carriage/local-rules?state=pending&pod=.
// A caller sees the local rules of pods whose org it governs; an operator sees
// all.
func (s *Server) handleListLocalRules(w http.ResponseWriter, r *http.Request) {
	state := r.URL.Query().Get("state")
	switch state {
	case "":
		state = registry.LocalPending
	case "all":
		state = ""
	case registry.LocalPending, registry.LocalRatified, registry.LocalLapsed:
	default:
		writeError(w, http.StatusBadRequest, "invalid", "state must be pending, ratified, lapsed or all")
		return
	}
	rules, err := s.reg.ListLocalRules(r.Context(), state, r.URL.Query().Get("pod"))
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	pods, err := s.reg.CarriagePods(r.Context())
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	governs := map[string]bool{}
	for _, p := range pods {
		governs[p.PodSlot] = s.mayGovernOrg(r, p.Org)
	}
	orgOf := map[string]string{}
	for _, p := range pods {
		orgOf[p.PodSlot] = p.Org
	}
	type localRuleView struct {
		registry.LocalRule
		PodOrg string `json:"pod_org"`
	}
	out := []localRuleView{}
	for _, lr := range rules {
		if governs[lr.PodSlot] {
			out = append(out, localRuleView{LocalRule: lr, PodOrg: orgOf[lr.PodSlot]})
		}
	}
	writeJSON(w, http.StatusOK, map[string]any{"local_rules": out})
}

// loadLocalForDecision fetches a local rule and its pod and checks the caller
// leads the pod's org.
func (s *Server) loadLocalForDecision(w http.ResponseWriter, r *http.Request) (*registry.LocalRule, *registry.CarriagePod, bool) {
	lr, err := s.reg.LocalRule(r.Context(), r.PathValue("id"))
	if err != nil {
		writeCarriageError(w, err)
		return nil, nil, false
	}
	pod, err := s.reg.CarriagePod(r.Context(), lr.PodSlot)
	if err != nil {
		writeCarriageError(w, err)
		return nil, nil, false
	}
	if !s.mayGovernOrg(r, pod.Org) {
		carriageForbidden(w, "only an org lead of the pod's org ("+pod.Org+") may ratify or lapse its local rules")
		return nil, nil, false
	}
	if lr.State != registry.LocalPending {
		writeCarriageError(w, registry.ErrCarriageLocalNotPending)
		return nil, nil, false
	}
	return lr, pod, true
}

// handleRatifyLocalRule serves POST /api/carriage/local-rules/{id}/ratify. A
// ratified grant becomes a central named rule for that source on that pod (an
// extend of a central named rule moves that rule's end date); a ratified
// revoke archives the named rules for that source on that pod.
func (s *Server) handleRatifyLocalRule(w http.ResponseWriter, r *http.Request) {
	lr, pod, ok := s.loadLocalForDecision(w, r)
	if !ok {
		return
	}
	ctx := r.Context()
	actor := actorOrVendor(r)
	lim := s.carriageLimits()
	now := registry.CarriageClock().Unix()
	ruleID, note := "", ""
	if lr.Action != carriage.ActionRevoke {
		orgsOf, err := s.carriageOrgResolver()
		if err != nil {
			writeError(w, http.StatusInternalServerError, "db", err.Error())
			return
		}
		why, err := s.revokedFrom(ctx, lr.SourceSlot, pod, orgsOf)
		if err != nil {
			writeError(w, http.StatusInternalServerError, "db", err.Error())
			return
		}
		if why != "" {
			_, _ = s.reg.DecideLocalRule(ctx, lr.ID, registry.LocalLapsed, why, "portal", "")
			writeError(w, http.StatusConflict, "carriage_source_revoked", why+"; the local rule has lapsed")
			return
		}
		until := lr.Until // a ratified grant keeps the end date the pod gave it
		if until <= now {
			writeError(w, http.StatusConflict, "local_rule_expired", "the local rule's end date has passed; let it lapse, or grant a new rule")
			return
		}
		if lr.Action == carriage.ActionExtend && strings.HasPrefix(lr.PodRuleID, "cr_") {
			if c, err := s.reg.CarriageRule(ctx, lr.PodRuleID); err == nil && c.State == registry.CarriageActive &&
				c.Kind != registry.CarriageKindOrg && deref(c.SourceSlot) == lr.SourceSlot && deref(c.PodSlot) == pod.PodSlot {
				if _, err := s.reg.ExtendCarriageRule(ctx, c.ID, until, lim); err != nil {
					writeCarriageError(w, err)
					return
				}
				ruleID, note = c.ID, "extended central rule "+c.ID
			}
		}
		if ruleID == "" {
			c, err := s.reg.CreateCarriageRule(ctx, registry.NewCarriageRule{
				Kind: registry.CarriageKindNamed, SourceSlot: lr.SourceSlot, PodSlot: pod.PodSlot, Until: until,
				Reason:    "ratified local rule " + lr.ID + ": " + lr.Reason,
				GrantedBy: "local:" + lr.AuthorityIdentity, AcceptedBy: actor, Origin: "ratified",
			}, lim)
			if err != nil {
				writeCarriageError(w, err)
				return
			}
			ruleID, note = c.ID, "central named rule "+c.ID
		}
	} else {
		rules, err := s.reg.ListCarriageRules(ctx, registry.CarriageRuleFilter{State: registry.CarriageActive, PodSlot: pod.PodSlot, SourceSlot: lr.SourceSlot})
		if err != nil {
			writeCarriageError(w, err)
			return
		}
		var ids []string
		for _, c := range rules {
			if c.Kind == registry.CarriageKindOrg || c.Kind == registry.CarriageKindExclusion {
				continue
			}
			if _, err := s.reg.RevokeCarriageRule(ctx, c.ID, actor); err == nil {
				ids = append(ids, c.ID)
			}
		}
		ex, err := s.reg.CreateCarriageExclusion(ctx, lr.SourceSlot, pod.PodSlot, "ratified local revoke "+lr.ID+": "+lr.Reason, actor)
		if err != nil {
			writeCarriageError(w, err)
			return
		}
		ruleID = ex.ID
		note = "exclusion " + ex.ID + " takes the source off this pod's org rule"
		if len(ids) > 0 {
			note += "; revoked named rules: " + strings.Join(ids, ",")
		}
	}
	out, err := s.reg.DecideLocalRule(ctx, lr.ID, registry.LocalRatified, note, actor, ruleID)
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	s.syncSlotFragment(ctx) // the core's carry-<anchor> subjects follow the rules (ADR 1251 phase 4)
	_ = s.reg.LogAuditEvent(ctx, actorOf(r), "carriage.local.ratify", lr.ID, fmt.Sprintf(`{"pod":%q,"rule":%q}`, pod.PodSlot, ruleID))
	writeJSON(w, http.StatusOK, out)
}

// handleLapseLocalRule serves POST /api/carriage/local-rules/{id}/lapse.
func (s *Server) handleLapseLocalRule(w http.ResponseWriter, r *http.Request) {
	lr, pod, ok := s.loadLocalForDecision(w, r)
	if !ok {
		return
	}
	out, err := s.reg.DecideLocalRule(r.Context(), lr.ID, registry.LocalLapsed, "let lapse by an org lead", actorOrVendor(r), "")
	if err != nil {
		writeCarriageError(w, err)
		return
	}
	_ = s.reg.LogAuditEvent(r.Context(), actorOf(r), "carriage.local.lapse", lr.ID, fmt.Sprintf(`{"pod":%q}`, pod.PodSlot))
	writeJSON(w, http.StatusOK, out)
}

// registerCarriageRoutes wires §7.5 onto mux.
func (s *Server) registerCarriageRoutes(mux *http.ServeMux) {
	mux.HandleFunc("POST /api/carriage/pods", s.requireOperator(s.handleRegisterCarriagePod))
	mux.HandleFunc("GET /api/carriage/pods", s.AuthMiddleware(true, s.handleListCarriagePods))
	mux.HandleFunc("GET /api/carriage/pods/{pod_slot}/generation", noStore(s.AuthMiddleware(true, s.handleCarriageGeneration)))
	mux.HandleFunc("POST /api/carriage/pods/{pod_slot}/ledger", s.AuthMiddleware(true, s.handleCarriageLedger))
	mux.HandleFunc("POST /api/carriage/rules", s.AuthMiddleware(true, s.handleCreateCarriageRule))
	mux.HandleFunc("GET /api/carriage/rules", s.AuthMiddleware(true, s.handleListCarriageRules))
	mux.HandleFunc("POST /api/carriage/rules/{id}/accept", s.AuthMiddleware(true, s.handleAcceptCarriageRule))
	mux.HandleFunc("POST /api/carriage/rules/{id}/extend", s.AuthMiddleware(true, s.handleExtendCarriageRule))
	mux.HandleFunc("POST /api/carriage/rules/{id}/revoke", s.AuthMiddleware(true, s.handleRevokeCarriageRule))
	mux.HandleFunc("POST /api/carriage/rules/{id}/reactivate", s.AuthMiddleware(true, s.handleReactivateCarriageRule))
	mux.HandleFunc("PUT /api/carriage/sources/{slot}/org-rule", s.AuthMiddleware(true, s.handleCarriageOptOut))
	mux.HandleFunc("GET /api/carriage/local-rules", s.AuthMiddleware(true, s.handleListLocalRules))
	mux.HandleFunc("GET /api/carriage/drift", noStore(s.requireOperator(s.handleCarriageDrift)))
	mux.HandleFunc("POST /api/carriage/local-rules/{id}/ratify", s.AuthMiddleware(true, s.handleRatifyLocalRule))
	mux.HandleFunc("POST /api/carriage/local-rules/{id}/lapse", s.AuthMiddleware(true, s.handleLapseLocalRule))
}
