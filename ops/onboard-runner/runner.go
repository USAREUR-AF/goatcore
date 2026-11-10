package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
)

// runner is the pull-model onboard auto-executor: it polls the portal for
// approved proposals, atomically claims each (so no two runners race one),
// materializes the manifest, runs the native `goat admin onboard`, and writes the
// terminal result back. The portal stays credential-free; the mint credentials
// live here (in the process env, handed to `goat`). See
// docs/design/operator-onboard-intake.md §"Fast-follow".
type runner struct {
	pc       *portalClient
	onlyEnv  string // "" = handle any env; else skip proposals for other envs
	repoRoot string // manifest is written to <repoRoot>/ops/operators/<env>/<handle>.yml
	goatBin  string // path to the goat binary (default "goat")
	runnerID string // GOAT_OPERATOR_HANDLE stamped into the mint's audit event
	dryRun   bool   // poll + log only; never claim, mint, or write back
	// deferCredential passes --defer-credential to the mint (mint-on-collect):
	// the slot and the identity are created, the fabric credential is not, and
	// its owner collects it from the console. PER-GOATNET and OFF by default —
	// a goatnet whose console does not yet serve the collection page would
	// otherwise leave every new enrollee with no credential and no way to get
	// one. docs/design/fabric-credential-collection.md.
	deferCredential bool
	log             *slog.Logger

	// mint is injectable so tests can exercise the loop without a real mint.
	// Nil means use the default (execs `goat admin onboard`).
	mint func(ctx context.Context, p proposal, manifestPath string) (mintResult, error)

	// reissue is the injectable executor for a reissue_handset or
	// reissue_device action. Nil means the default, which execs
	// `goat admin onboard <handle> --native --env <env> --reissue-…-key`
	// and returns its combined output.
	reissue func(ctx context.Context, a action) ([]byte, error)
	// release runs the existing lifecycle release lane for one immutable hold.
	// Nil executes lifecycle-sweep.py from the mounted repository.
	release func(ctx context.Context, a action) (releaseActionResult, error)

	// revoke is the offboard leg's counterpart of mint (goat#372): injectable
	// for tests; nil means use the default (execs the reversible native
	// `goat admin onboard <handle> --revoke`).
	revoke func(ctx context.Context, p proposal) error

	// collab provisions the cloud collaboration-environment account for a
	// manifest that opted in (collab.enabled). Nil ⇒ the leg no-ops (goatnets
	// without a collab env, or a runner not configured for it). Best-effort by
	// contract — a collab failure never fails the onboarding.
	collab CollabProvider

	// mgmtURL is this goatnet's NetBird management API as the CONTAINER resolves
	// it (NETBIRD_MGMT_URL, the same value the native mint calls). Before any
	// proposal is claimed the runner proves that address answers; empty
	// disables the check (tests, a goatnet with no mesh leg). mgmtHTTP is the
	// client for that probe.
	mgmtURL  string
	mgmtHTTP *http.Client
	// holdRecordsPath is shared with lifecycle-sweep. Empty selects the
	// deployed path; tests point it at an isolated file.
	holdRecordsPath string
}

// mgmtReachable proves the NetBird management API answers at the address this
// container resolves it to. A claim flips a proposal approved → onboarding, and
// a mint that dies on its first API call flips it to failed with a clipped note
// — five people sat in `failed` because the deploy had rendered
// the API's address wrong. Refusing to claim leaves them `approved`,
// where the next healthy cycle takes them unchanged and the queue shows them
// waiting rather than failed. 200, 401 and 403 all mean the API was reached
// (the probe carries no token); anything else is the fault, named with the URL.
func (r *runner) mgmtReachable(ctx context.Context) error {
	if r.mgmtURL == "" {
		return nil
	}
	url := strings.TrimRight(r.mgmtURL, "/") + "/api/groups"
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return err
	}
	hc := r.mgmtHTTP
	if hc == nil {
		hc = &http.Client{Timeout: 10 * time.Second}
	}
	resp, err := hc.Do(req)
	if err != nil {
		return fmt.Errorf("GET %s: %w", url, err)
	}
	_ = resp.Body.Close()
	switch resp.StatusCode {
	case http.StatusOK, http.StatusUnauthorized, http.StatusForbidden:
		return nil
	}
	return fmt.Errorf("GET %s: HTTP %d (expected 200/401/403 — the address does not serve the NetBird API)", url, resp.StatusCode)
}

// runOnce polls the approved queue and processes every eligible proposal. It
// never returns an error for a single-proposal failure (that's recorded back as
// the proposal's `failed` result); it returns an error only when the queue poll
// itself fails.
func (r *runner) runOnce(ctx context.Context) error {
	ps, err := r.pc.approved(ctx)
	if err != nil {
		return fmt.Errorf("poll approved queue: %w", err)
	}
	if len(ps) > 0 && !r.dryRun {
		if err := r.mgmtReachable(ctx); err != nil {
			// Leave every approved proposal exactly where it is. Claiming now
			// would turn a deploy fault into N `failed` rows a person has to
			// retry by hand.
			r.log.Error("NetBird management API unreachable from this container — NOT claiming approved proposals (they stay approved until the API answers)",
				"approved", len(ps), "err", err)
			return fmt.Errorf("mgmt API preflight: %w", err)
		}
	}
	eligible, skipped := 0, 0
	for _, p := range ps {
		if r.onlyEnv != "" && p.Env != r.onlyEnv {
			// This runner won't touch the proposal — but an approved proposal with a
			// blank or mismatched env would otherwise vanish silently (the exact
			// well-oiled-machine strand). WARN so it surfaces in the runner log.
			r.log.Warn("skipping approved proposal — env does not match this runner (proposal will not onboard here)",
				"id", p.ID, "handle", p.Handle, "proposal_env", p.Env, "runner_env", r.onlyEnv)
			skipped++
			continue
		}
		eligible++
		r.process(ctx, p)
	}
	if skipped > 0 && r.log != nil {
		r.log.Warn("approved proposals were skipped on env mismatch", "skipped", skipped, "runner_env", r.onlyEnv)
	}
	r.log.Debug("poll complete", "approved", len(ps), "eligible", eligible, "skipped_env", skipped, "dry_run", r.dryRun)
	// Follow-up actions for people the proposals loop is finished with. A poll
	// failure here is logged, not returned: the proposals half already ran and
	// a console without the action queue simply has none.
	if err := r.runActionsOnce(ctx); err != nil {
		r.log.Warn("poll follow-up actions", "err", err)
	}

	offs, err := r.pc.listByStatus(ctx, "offboard-requested")
	if err != nil {
		return fmt.Errorf("poll offboard queue: %w", err)
	}
	// Filter to this runner's env BEFORE the preflight, so another goatnet's
	// queued cut cannot fail this runner's cycle on a mgmt API it was never
	// going to call.
	mine := make([]proposal, 0, len(offs))
	for _, p := range offs {
		if r.onlyEnv != "" && p.Env != r.onlyEnv {
			// Same well-oiled-machine strand as the onboarding leg: a queued
			// offboard this runner won't touch must not vanish silently — the
			// person keeps access until that env's runner cuts it.
			r.log.Warn("skipping offboard request — env does not match this runner (access is NOT cut here)",
				"id", p.ID, "handle", p.Handle, "proposal_env", p.Env, "runner_env", r.onlyEnv)
			continue
		}
		mine = append(mine, p)
	}
	if len(mine) > 0 && !r.dryRun {
		// Same F-512 preflight as the onboarding leg: the revoke calls the same
		// management API, and claiming with the API down turns a deploy fault
		// into N offboard-failed rows an admin has to re-queue by hand.
		if err := r.mgmtReachable(ctx); err != nil {
			r.log.Error("NetBird management API unreachable from this container — NOT claiming queued offboards (they stay offboard-requested until the API answers)",
				"offboard_requested", len(mine), "err", err)
			return fmt.Errorf("mgmt API preflight: %w", err)
		}
	}
	for _, p := range mine {
		r.processOffboard(ctx, p)
	}
	return nil
}

// runActionsOnce claims and runs every queued follow-up action for this
// runner's env. One action's failure is written back on that action; only a
// queue poll failure is returned.
func (r *runner) runActionsOnce(ctx context.Context) error {
	as, err := r.pc.queuedActions(ctx)
	if err != nil {
		return fmt.Errorf("poll action queue: %w", err)
	}
	for _, a := range as {
		if r.onlyEnv != "" && a.Env != r.onlyEnv {
			r.log.Debug("skipping queued action — env does not match this runner", "id", a.ID, "kind", a.Kind, "action_env", a.Env, "runner_env", r.onlyEnv)
			continue
		}
		r.processAction(ctx, a)
	}
	return nil
}

// processAction handles one queued action end to end: claim, run, write back.
func (r *runner) processAction(ctx context.Context, a action) {
	l := r.log.With("action", a.ID, "kind", a.Kind, "handle", a.Handle, "env", a.Env)
	if r.dryRun {
		l.Info("dry-run: would run follow-up action (no claim / exec / writeback)")
		return
	}
	if a.Kind != actionReissueHandset && a.Kind != actionReissueDevice && a.Kind != actionReleaseHold {
		// Unknown kinds are left queued for a runner that knows them; claiming
		// one here would fail it for no reason.
		l.Warn("unknown action kind — leaving it queued")
		return
	}
	claimToken, claimed, err := r.pc.claimAction(ctx, a.ID)
	if err != nil {
		l.Error("claim action failed", "err", err)
		return
	}
	if !claimed {
		l.Debug("action claim lost — skipping")
		return
	}
	if claimToken == "" && a.Kind == actionReleaseHold {
		l.Error("claimed release action response has no claim token — leaving it for stale-claim recovery")
		return
	}
	l.Info("claimed action")
	if a.Kind == actionReleaseHold {
		run := r.release
		if run == nil {
			run = r.execReleaseHold
		}
		result, runErr := run(ctx, a)
		body := actionResultBody{Status: "done", ClaimToken: claimToken, Release: &result}
		if runErr != nil || (result.Outcome != "released" && result.Outcome != "already_released") {
			body.Status = "failed"
			if runErr != nil {
				body.Error = truncErr(runErr.Error())
			} else if strings.TrimSpace(result.Detail) != "" {
				body.Error = truncErr(result.Detail)
			} else {
				body.Error = "release did not reach a verified terminal state"
			}
		}
		if err := r.pc.actionResult(ctx, a.ID, body); err != nil {
			l.Error("writeback (release result) failed", "err", err)
			return
		}
		l.Info("hold release finished", "outcome", result.Outcome, "audit_outcome", result.AuditOutcome)
		return
	}
	run := r.reissue
	if run == nil {
		run = r.execReissue
	}
	out, err := run(ctx, a)
	if err != nil {
		l.Error("key reissue failed", "err", err)
		if writeErr := r.pc.actionResult(ctx, a.ID, actionResultBody{Status: "failed", ClaimToken: claimToken, Error: truncErr(err.Error())}); writeErr != nil {
			l.Error("writeback (action failed) failed; offboarding remains blocked", "err", writeErr)
		}
		return
	}
	res := parseMintResult(out)
	body := actionResultBody{Status: "done", ClaimToken: claimToken}
	switch a.Kind {
	case actionReissueHandset:
		body.HandsetKey = res.HandsetKey
		if body.HandsetKey == "" {
			l.Error("reissue produced no handset key line")
			if writeErr := r.pc.actionResult(ctx, a.ID, actionResultBody{Status: "failed", ClaimToken: claimToken, Error: "goat printed no `handset key:` line: " + mintErrSnippet(out)}); writeErr != nil {
				l.Error("writeback (action failed) failed; offboarding remains blocked", "err", writeErr)
			}
			return
		}
	case actionReissueDevice:
		body.SetupKey = res.SetupKey
		if body.SetupKey == "" {
			l.Error("reissue produced no setup key line")
			if writeErr := r.pc.actionResult(ctx, a.ID, actionResultBody{Status: "failed", ClaimToken: claimToken, Error: "goat printed no `setup key:` line: " + mintErrSnippet(out)}); writeErr != nil {
				l.Error("writeback (action failed) failed; offboarding remains blocked", "err", writeErr)
			}
			return
		}
	}
	if err := r.pc.actionResult(ctx, a.ID, body); err != nil {
		l.Error("writeback (action done) failed", "err", err)
		return
	}
	l.Info("key reissued")
}

type releaseHoldParams struct {
	Site              string   `json:"site"`
	PeerID            string   `json:"peer_id"`
	HoldEnteredAt     string   `json:"hold_entered_at"`
	HoldGroup         string   `json:"hold_group"`
	PriorGroups       []string `json:"prior_groups"`
	RequestedByHandle string   `json:"requested_by_handle"`
}

type releaseActionResult struct {
	Outcome      string `json:"outcome"`
	Detail       string `json:"detail,omitempty"`
	AuditOutcome string `json:"audit_outcome"`
	AuditError   string `json:"audit_error,omitempty"`
}

func classifyReleaseOutput(out string, runErr error) string {
	upper := strings.ToUpper(out)
	if runErr == nil && (strings.Contains(out, "1 peer(s) released, 0 failed.") || strings.Contains(upper, "RELEASE FINISHED")) {
		return "released"
	}
	if runErr == nil && (strings.Contains(upper, "STALE RECORD") || strings.Contains(upper, "ORPHAN RECORD") ||
		strings.Contains(upper, "NOTHING TO RELEASE")) {
		return "already_released"
	}
	for _, marker := range []string{"REFUSED", "NO HOLD RECORD", "NO PEER NAMED", "GOVERNED BY WARRANT", "NO NETBIRD API TOKEN", "REQUIRES LIFECYCLE_HOLD_RECORDS"} {
		if strings.Contains(upper, marker) {
			return "refused"
		}
	}
	return "retryable_partial"
}

func (r *runner) verifyRelease(ctx context.Context, p releaseHoldParams, token string) error {
	holdRecordsPath := r.holdRecordsPath
	if holdRecordsPath == "" {
		holdRecordsPath = "/var/lib/peerview/hold-records.jsonl"
	}
	f, err := os.Open(holdRecordsPath)
	if err != nil {
		return fmt.Errorf("open hold records for verification: %w", err)
	}
	defer f.Close()
	scan := bufio.NewScanner(f)
	skipped := 0
	for scan.Scan() {
		var rec struct {
			PeerID    string `json:"peer_id"`
			EnteredAt string `json:"entered_at"`
		}
		if err := json.Unmarshal(scan.Bytes(), &rec); err != nil {
			skipped++
			continue
		}
		if rec.PeerID == p.PeerID && rec.EnteredAt == p.HoldEnteredAt {
			return errors.New("exact hold record still exists after release")
		}
	}
	if err := scan.Err(); err != nil {
		return fmt.Errorf("read hold records for verification: %w", err)
	}
	if skipped > 0 {
		r.log.Warn("skipped malformed hold-record lines during release verification", "count", skipped)
	}
	if strings.TrimSpace(r.mgmtURL) == "" {
		return errors.New("NETBIRD_MGMT_URL is unset; cannot verify release")
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet,
		strings.TrimRight(r.mgmtURL, "/")+"/api/peers/"+url.PathEscape(p.PeerID), nil)
	if err != nil {
		return fmt.Errorf("build release verification request: %w", err)
	}
	req.Header.Set("Authorization", "Token "+token)
	req.Header.Set("Accept", "application/json")
	hc := r.mgmtHTTP
	if hc == nil {
		hc = &http.Client{Timeout: 10 * time.Second}
	}
	resp, err := hc.Do(req)
	if err != nil {
		return fmt.Errorf("verify live peer: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("verify live peer: HTTP %d", resp.StatusCode)
	}
	var peer struct {
		Groups []struct {
			Name string `json:"name"`
		} `json:"groups"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&peer); err != nil {
		return fmt.Errorf("decode live peer verification: %w", err)
	}
	got := make([]string, 0, len(peer.Groups))
	for _, g := range peer.Groups {
		if strings.TrimSpace(g.Name) != "" && !strings.EqualFold(g.Name, "All") {
			got = append(got, strings.ToLower(strings.TrimSpace(g.Name)))
		}
	}
	want := make([]string, 0, len(p.PriorGroups))
	for _, g := range p.PriorGroups {
		if strings.TrimSpace(g) != "" && !strings.EqualFold(g, "All") {
			want = append(want, strings.ToLower(strings.TrimSpace(g)))
		}
	}
	sort.Strings(got)
	sort.Strings(want)
	if len(got) != len(want) {
		return fmt.Errorf("live groups after release are %v, want exactly %v", got, want)
	}
	for i := range got {
		if got[i] != want[i] {
			return fmt.Errorf("live groups after release are %v, want exactly %v", got, want)
		}
	}
	return nil
}

func (r *runner) emitReleaseTerminalAudit(ctx context.Context, p releaseHoldParams, result releaseActionResult) releaseActionResult {
	terminalOutcome := "failed"
	if result.Outcome == "released" || result.Outcome == "already_released" {
		terminalOutcome = "ok"
	}
	terminalArgs := []string{"audit", "emit", "--actor", p.RequestedByHandle, "--surface", "id-admin-console",
		"--action", "admin.hold.release", "--target", p.PeerID + "@" + p.HoldEnteredAt,
		"--env", p.Site, "--outcome", terminalOutcome,
		"--args-digest", "hold=" + p.HoldGroup + " destinations=" + strings.Join(p.PriorGroups, ",") + " release_outcome=" + result.Outcome}
	if terminalOutcome == "failed" {
		terminalArgs = append(terminalArgs, "--error", truncErr(result.Detail))
	}
	// #nosec G204 -- CommandContext passes validated fields as argv without a shell; goatBin is operator configuration.
	terminalCmd := exec.CommandContext(ctx, r.goatBin, terminalArgs...)
	terminalCmd.Dir = r.repoRoot
	terminalCmd.Env = append(os.Environ(), "GOAT_OPERATOR_HANDLE="+p.RequestedByHandle)
	if auditOut, err := terminalCmd.CombinedOutput(); err != nil {
		result.AuditOutcome = "failed"
		result.AuditError = truncErr(fmt.Sprintf("terminal audit: %v: %s", err, strings.TrimSpace(string(auditOut))))
	}
	return result
}

// execReleaseHold delegates membership and hold-file mutation to the one
// lifecycle engine. That preserves its flock, mark-before-write, exact restore,
// verification, and retry behavior instead of reproducing them in the runner.
func (r *runner) execReleaseHold(ctx context.Context, a action) (releaseActionResult, error) {
	var p releaseHoldParams
	if err := json.Unmarshal([]byte(a.Params), &p); err != nil {
		return releaseActionResult{Outcome: "refused", AuditOutcome: "failed", AuditError: "invalid release scope; audit not attempted"}, fmt.Errorf("decode release params: %w", err)
	}
	if p.Site == "" || p.PeerID == "" || p.HoldEnteredAt == "" || p.RequestedByHandle == "" || p.Site != a.Env {
		return releaseActionResult{Outcome: "refused", AuditOutcome: "failed", AuditError: "invalid release scope; audit not attempted"}, fmt.Errorf("release action has incomplete or mismatched immutable scope")
	}
	if r.onlyEnv == "" || r.onlyEnv != p.Site {
		return releaseActionResult{Outcome: "refused", AuditOutcome: "failed", AuditError: "runner goatnet scope does not match the hold; audit not attempted"},
			fmt.Errorf("runner is not pinned to release goatnet %q", p.Site)
	}
	mgmtURL := strings.TrimRight(strings.TrimSpace(r.mgmtURL), "/")
	if mgmtURL == "" {
		return releaseActionResult{Outcome: "refused", AuditOutcome: "failed", AuditError: "NETBIRD_MGMT_URL is unset; audit not attempted"},
			errors.New("NETBIRD_MGMT_URL is unset; refusing hold release before audit or mutation")
	}
	// Record the start OperatorActionV1 before the membership mutation. A
	// terminal OperatorActionV1 is emitted after the release attempt below.
	// `goat audit emit` is deliberately fallible, so a successful exit means
	// the record reached the audit fabric. Refuse before mutation when it did
	// not.
	auditArgs := []string{"audit", "emit", "--actor", p.RequestedByHandle, "--surface", "id-admin-console",
		"--action", "admin.hold.release", "--target", p.PeerID + "@" + p.HoldEnteredAt,
		"--env", p.Site, "--outcome", "started",
		"--args-digest", "hold=" + p.HoldGroup + " destinations=" + strings.Join(p.PriorGroups, ",")}
	// #nosec G204 -- CommandContext passes validated fields as argv without a shell; goatBin is operator configuration.
	auditCmd := exec.CommandContext(ctx, r.goatBin, auditArgs...)
	auditCmd.Dir = r.repoRoot
	auditCmd.Env = append(os.Environ(), "GOAT_OPERATOR_HANDLE="+p.RequestedByHandle)
	if auditOut, err := auditCmd.CombinedOutput(); err != nil {
		auditErr := truncErr(fmt.Sprintf("%v: %s", err, strings.TrimSpace(string(auditOut))))
		return releaseActionResult{Outcome: "refused", AuditOutcome: "failed", AuditError: auditErr},
			fmt.Errorf("record operator-action audit before release: %s", auditErr)
	}
	tokenName := strings.ToUpper(strings.ReplaceAll(p.Site, "-", "_")) + "_NB_API_TOKEN"
	token := strings.TrimSpace(os.Getenv(tokenName))
	if token == "" {
		err := fmt.Errorf("%s is unset", tokenName)
		result := releaseActionResult{Outcome: "refused", Detail: err.Error(), AuditOutcome: "recorded"}
		return r.emitReleaseTerminalAudit(ctx, p, result), err
	}
	script := filepath.Join(r.repoRoot, "ops", "scripts", "lifecycle-sweep.py")
	holdRecordsPath := r.holdRecordsPath
	if holdRecordsPath == "" {
		holdRecordsPath = "/var/lib/peerview/hold-records.jsonl"
	}
	// #nosec G204 -- the fixed lifecycle script receives the validated immutable action scope as argv, never shell text.
	cmd := exec.CommandContext(ctx, "python3", script, "--apply", "--lane", "release", "--peer", p.PeerID,
		"--hold-entered-at", p.HoldEnteredAt, "--actor", p.RequestedByHandle)
	cmd.Dir = r.repoRoot
	cmd.Env = append(os.Environ(),
		"NB_TOKEN="+token, "NB_API="+mgmtURL, "GOATNET="+p.Site,
		"LIFECYCLE_HOLD_RECORDS="+holdRecordsPath,
		"LIFECYCLE_LEDGER=/var/lib/peerview/lifecycle-sweep.log",
		"LIFECYCLE_MEMBERSHIP_DIR=/etc/goat/warrant/membership/"+p.Site)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	runErr := cmd.Run()
	detail := strings.TrimSpace(strings.Join([]string{stdout.String(), stderr.String()}, "\n"))
	result := releaseActionResult{Outcome: classifyReleaseOutput(detail, runErr), Detail: truncErr(detail), AuditOutcome: "recorded"}
	if runErr == nil {
		if verifyErr := r.verifyRelease(ctx, p, token); verifyErr != nil {
			result.Outcome = "retryable_partial"
			result.Detail = truncErr(strings.TrimSpace(detail + "\npostcondition verification failed: " + verifyErr.Error()))
		} else if result.Outcome == "retryable_partial" {
			// The terminal state, not a process exit or a prose marker, is the
			// authority for success when an older lifecycle engine says less.
			result.Outcome = "released"
		}
	}
	return r.emitReleaseTerminalAudit(ctx, p, result), runErr
}

// reissueArgs builds the goat argv for a reissue action. The requester is
// passed through so goat's audit event names the console operator who asked,
// not only the runner that ran it; the device reissue carries the reuse count
// the operator chose.
func reissueArgs(a action, repoRoot string) []string {
	args := []string{"admin", "onboard", a.Handle, "--native"}
	if a.Env != "" {
		args = append(args, "--env", a.Env)
	}
	args = append(args, "--repo-root", repoRoot)
	switch a.Kind {
	case actionReissueDevice:
		args = append(args, "--reissue-device-key")
		var p struct {
			Uses int `json:"uses"`
		}
		if json.Unmarshal([]byte(a.Params), &p) == nil && p.Uses > 0 {
			args = append(args, "--uses", strconv.Itoa(p.Uses))
		}
	default:
		args = append(args, "--reissue-handset-key")
	}
	if rb := strings.TrimSpace(a.RequestedBy); rb != "" {
		args = append(args, "--requested-by", rb)
	}
	return args
}

// execReissue runs the reissue verb with the same conventions as execMint
// (repo root as cwd, GOAT_OPERATOR_HANDLE stamped for the audit event,
// credentials from the process env). The manifest is already on disk from the
// original mint; the verb reads it and touches nothing but the person's keys.
func (r *runner) execReissue(ctx context.Context, a action) ([]byte, error) {
	args := reissueArgs(a, r.repoRoot)
	cmd := exec.CommandContext(ctx, r.goatBin, args...)
	cmd.Dir = r.repoRoot
	cmd.Env = append(os.Environ(), "GOAT_OPERATOR_HANDLE="+r.runnerID)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return out, fmt.Errorf("%s %s: %w: %s", r.goatBin, strings.Join(args, " "), err, mintErrSnippet(out))
	}
	return out, nil
}

// process handles one eligible proposal end to end. Any failure is recorded back
// on the proposal (failed) rather than crashing the loop.
func (r *runner) process(ctx context.Context, p proposal) {
	l := r.log.With("id", p.ID, "handle", p.Handle, "env", p.Env, "email", p.Email)

	if r.dryRun {
		l.Info("dry-run: would onboard (no claim / mint / writeback)")
		return
	}

	// Claim: atomic approved → onboarding. A 409 means another runner won or the
	// state moved on — skip silently.
	won, err := r.pc.claim(ctx, p.ID)
	if err != nil {
		l.Error("claim failed", "err", err)
		return
	}
	if !won {
		l.Debug("claim lost (already claimed / not approved) — skipping")
		return
	}
	l.Info("claimed")

	// Idempotency: if this person (by email) already has an onboarded proposal,
	// don't mint a second time — finalize this one onboarded carrying the ORIGINAL's
	// setup key + bundle path, so the duplicate points at the same enrollment
	// instead of a keyless onboarded row. (The native mint is also idempotent on the
	// Ory identity + 409s a taken slot; this is the cheap first guard so we don't
	// even run it. The submit-time guard refuses most duplicates before they get
	// here — this backstops the concurrent-submit race.)
	if dup, err := r.findDuplicate(ctx, p); err != nil {
		l.Warn("dedup check failed — proceeding to mint", "err", err)
	} else if dup != nil {
		l.Info("duplicate email already onboarded — skipping mint", "original", dup.ID)
		if err := r.pc.resultWithKeyBundle(ctx, p.ID, "onboarded",
			"duplicate: email already onboarded as "+dup.ID, dup.SetupKey, dup.BundlePath); err != nil {
			l.Error("writeback (duplicate) failed", "err", err)
		}
		return
	}

	// Materialize the manifest to the on-disk location the native onboard reads.
	manifestPath, err := r.writeManifest(p)
	if err != nil {
		l.Error("materialize manifest failed", "err", err)
		_ = r.pc.result(ctx, p.ID, "failed", "materialize manifest: "+err.Error())
		return
	}

	// Mint (native fast path, ~3s).
	mint := r.mint
	if mint == nil {
		mint = r.execMint
	}
	minted, err := mint(ctx, p, manifestPath)
	if err != nil {
		l.Error("mint failed", "err", err)
		_ = r.pc.resultWithKey(ctx, p.ID, "failed", truncErr(err.Error()), "")
		return
	}
	l.Info("minted")
	// Best-effort collab leg (docs/design/collab-onboarding-provisioner.md): if the
	// manifest opted in, provision the collaboration-environment account and get its
	// username + one-time setup link. Runs BEFORE the writeback (not after) so those
	// ride in the SAME welcome email the console sends on the /result callback — the
	// console emails synchronously, so a link produced after the writeback would miss
	// it. Non-fatal: on skip/failure it returns "", the writeback proceeds without
	// collab fields, and the email simply omits the collab section.
	collabAccount, collabLink := r.provisionCollab(ctx, p, l)

	// Hand the fabric bundle's mesh download PATH to the console (the native mint
	// wrote it into the bundle-md), so the welcome email carries a download link
	// the enrollee pulls over the mesh — no operator hand-off. Best-effort.
	bundlePath := readBundlePath(r.repoRoot, p.Handle)
	rb := resultBody{Status: "onboarded", SetupKey: minted.SetupKey, HandsetKey: minted.HandsetKey,
		PortalAPIKey: minted.PortalAPIKey, BundlePath: bundlePath,
		SlotAnchor: minted.SlotAnchor, CredentialDeferred: r.deferCredential}
	if collabAccount != "" {
		rb.CollabAccount, rb.CollabSetupLink, rb.CollabActivated = collabAccount, collabLink, true
	}
	if err := r.pc.postResult(ctx, p.ID, rb); err != nil {
		l.Error("writeback (onboarded) failed", "err", err)
	}
}

// provisionCollab runs the best-effort collaboration-environment leg and returns
// the provisioned account + its one-time setup link (both "" when the manifest
// didn't opt in, no provider is configured, or provisioning failed). Non-fatal by
// contract: a collab failure is logged and dropped, never propagated (the mesh
// identity is already minted). It also stages the account + link to
// ops/operators/out/<handle>-collab.md as crash-insurance / an operator fallback,
// but the primary delivery is the returned pair (folded into the welcome email).
func (r *runner) provisionCollab(ctx context.Context, p proposal, l *slog.Logger) (account, setupURL string) {
	if r.collab == nil {
		return "", ""
	}
	f := parseManifestFacts(p.ManifestJSON)
	if !f.Collab.Enabled {
		return "", ""
	}
	email := p.Email
	if email == "" {
		email = f.Identity.Email
	}
	acct, url, err := r.collab.Provision(ctx, p.Handle, email, f.Identity.Name)
	if err != nil {
		l.Warn("collab provisioning failed (best-effort, non-fatal)", "provider", r.collab.Name(), "err", truncErr(err.Error()))
		return "", ""
	}
	// Already provisioned (re-onboard of someone who kept collab access): the
	// provider returns the account with no setup link. That is ACHIEVED — record
	// the account so the identity record stays true, and skip the out-file (there
	// is no link to stage) and the link half of the welcome email.
	if url == "" {
		l.Info("collab account already provisioned (no new setup link minted)", "provider", r.collab.Name(), "account", acct)
		return acct, ""
	}
	// Stage the out-file too (fallback); a failure there doesn't lose the pair —
	// the welcome email still carries it.
	if err := writeCollabOut(r.repoRoot, p.Handle, r.collab.Name(), acct, url, collabSignInURL(r.collab)); err != nil {
		l.Warn("collab account provisioned, but staging the setup-link file failed", "provider", r.collab.Name(), "account", acct, "err", err)
	}
	// Never log url — it is a credential-bearing one-time link.
	l.Info("collab account provisioned", "provider", r.collab.Name(), "account", acct)
	return acct, url
}

// bundlePathRe pulls the token-gated bundle path (/api/org-bundle/<id>/<tok>.tar.gz)
// out of the native mint's bundle-md, ignoring the internal host in front of it.
var bundlePathRe = regexp.MustCompile(`/api/org-bundle/[A-Za-z0-9]+/[A-Za-z0-9]+\.tar\.gz`)

// readBundlePath returns the fabric bundle's mesh download path for <handle> from
// ops/operators/out/<handle>-bundle.md. "" when absent (best-effort — the welcome
// package then omits the link). The console prepends the mesh portal base.
func readBundlePath(repoRoot, handle string) string {
	md, err := os.ReadFile(filepath.Join(repoRoot, "ops", "operators", "out", handle+"-bundle.md"))
	if err != nil {
		return ""
	}
	return bundlePathRe.FindString(string(md))
}

// findDuplicate returns an existing onboarded proposal for this proposal's email
// (case-insensitive), or nil when none. Empty email → never a duplicate. The
// returned row carries the original's setup key + bundle path so the caller can
// finalize the duplicate pointing at the same enrollment.
func (r *runner) findDuplicate(ctx context.Context, p proposal) (*proposal, error) {
	if strings.TrimSpace(p.Email) == "" {
		return nil, nil
	}
	done, err := r.pc.listByStatus(ctx, "onboarded")
	if err != nil {
		return nil, err
	}
	for i := range done {
		if done[i].ID != p.ID && strings.EqualFold(strings.TrimSpace(done[i].Email), strings.TrimSpace(p.Email)) {
			return &done[i], nil
		}
	}
	return nil, nil
}

// writeManifest renders manifest_json to YAML and writes it to
// <repoRoot>/ops/operators/<env>/<handle>.yml, returning the path.
func (r *runner) writeManifest(p proposal) (string, error) {
	y, err := manifestJSONToYAML(p.ManifestJSON)
	if err != nil {
		return "", err
	}
	dir := filepath.Join(r.repoRoot, "ops", "operators", p.Env)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	path := filepath.Join(dir, p.Handle+".yml")
	if err := os.WriteFile(path, y, 0o600); err != nil {
		return "", err
	}
	return path, nil
}

// execMint runs `goat admin onboard <handle> --env <env> --native` from the
// runner's repo root, stamping GOAT_OPERATOR_HANDLE so the mint's audit event
// reads runner-executed / operator-approved. The mint credentials (NetBird token,
// SSH, portal operator key) are inherited from the process env.
//
// `--native` is FORCED, not left to default: the default would fall back to the
// Ansible path for a manifest the native fast path can't service (e.g. a legacy
// `zenoh.trial.vendor` prefix, an operator-IdP or observer manifest — ZAGROS
// live-fire). The runner cannot run Ansible (no aws-vault wrapper, no
// interactive sudo), so a silent fallback would hang or fail confusingly. Forcing
// --native makes a non-native-serviceable manifest fail FAST with the CLI's own
// eligibility error, recorded cleanly as the proposal's `failed` result.
func (r *runner) execMint(ctx context.Context, p proposal, _ string) (mintResult, error) {
	args := []string{"admin", "onboard", p.Handle, "--native"}
	if r.deferCredential {
		args = append(args, "--defer-credential")
	}
	if p.Env != "" {
		args = append(args, "--env", p.Env)
	}
	args = append(args, "--repo-root", r.repoRoot)
	cmd := exec.CommandContext(ctx, r.goatBin, args...)
	cmd.Dir = r.repoRoot
	cmd.Env = append(os.Environ(), "GOAT_OPERATOR_HANDLE="+r.runnerID)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return mintResult{}, fmt.Errorf("%s %s: %w: %s", r.goatBin, strings.Join(args, " "), err, mintErrSnippet(out))
	}
	return parseMintResult(out), nil
}

// mintResult is what the native mint produced that the candidate's welcome
// email needs. Both fields are best-effort: the mint prints them, we scrape
// them, and a miss degrades the email rather than failing the onboard.
type mintResult struct {
	// SetupKey is the NetBird enrollment key (mesh join, step 1).
	SetupKey string
	// HandsetKey is the second, OBSERVER-only NetBird key for the person's
	// phones and tablets (ADR 1175). Absent when the manifest's
	// netbird.handset_limit is 0 or the mint predates the key.
	HandsetKey string
	// PortalAPIKey is the scoped `vp_…` portal key. It is minted into the
	// bundle but, before this, never reached the console — so the welcome email
	// told people to "find it under your identity once you can reach the
	// Portal", which is circular: the key IS how you reach the Portal. Admins
	// were closing the gap by hand with `goat admin rotate`, which re-issues in
	// place and thereby invalidates the key sitting in the person's own bundle.
	PortalAPIKey string
	// SlotAnchor is the mint's write anchor (= slot_id = cert CN). Scraped so the
	// console can stamp the person→slot linkage from a value the mint STATED,
	// rather than one re-derived by parsing the bundle path — a derivation that
	// yields nothing for a slot with no bundle, and says nothing when it does.
	SlotAnchor string
}

// setupKeyRe matches the native mint's summary line
// ("  setup key:    <KEY>  (key `<name>`)"). Best-effort: no match → the
// final candidate email simply omits the key (the operator hands it over).
var setupKeyRe = regexp.MustCompile(`(?m)setup key:\s+(\S+)`)

// handsetKeyRe matches the mint's "  handset key:  <KEY>  (key `<name>-handset`)"
// line. The two labels differ in their first word, so neither regex can take
// the other's key.
var handsetKeyRe = regexp.MustCompile(`(?m)handset key:\s+(\S+)`)

// portalAPIKeyRe matches the mint's "  api_key:      vp_<slot>_<secret>" line.
// Anchored on the `vp_` shape so it can never scrape an adjacent field.
var portalAPIKeyRe = regexp.MustCompile(`(?m)api_key:\s+(vp_\S+)`)

// slotAnchorRe matches the mint's "  write anchor: <32 hex>" line. Anchored on
// the anchor's shape (the 32-hex-char sha256 prefix SlotAnchor produces) so it
// cannot scrape an adjacent field, and so a per-vendor mint — whose anchor is a
// human prefix, not a slot — leaves it empty rather than reporting a non-slot as
// one.
var slotAnchorRe = regexp.MustCompile(`(?m)write anchor:\s+([0-9a-f]{32})\b`)

// parseMintResult extracts the freshly minted credentials from the mint output
// so the writeback can carry them to the candidate's final email
// (onboarding-notifications.md increments 3 + 7). Never persisted here.
func parseMintResult(out []byte) mintResult {
	var r mintResult
	if m := setupKeyRe.FindSubmatch(out); len(m) == 2 {
		r.SetupKey = string(m[1])
	}
	if m := handsetKeyRe.FindSubmatch(out); len(m) == 2 {
		r.HandsetKey = string(m[1])
	}
	if m := portalAPIKeyRe.FindSubmatch(out); len(m) == 2 {
		r.PortalAPIKey = string(m[1])
	}
	if m := slotAnchorRe.FindSubmatch(out); len(m) == 2 {
		r.SlotAnchor = string(m[1])
	}
	return r
}

// mintErrSnippet picks the part of goat's combined output that says WHY the
// mint failed. goat prints its terminal error as the LAST `error:` line, after
// any number of `warning:` lines (the operator-action audit fallback prints two
// of those on every runner mint). Keeping the head of the output kept only the
// warnings, so the proposal note read "operator-action audit emit failed" for a
// failure that was "NetBird GET /api/groups failed" and the real reason
// lived only in the container's fallback audit log. The last `error:` line
// wins; without one, the tail of the output does.
func mintErrSnippet(b []byte) string {
	const max = 300
	s := strings.TrimSpace(string(b))
	lines := strings.Split(s, "\n")
	for i := len(lines) - 1; i >= 0; i-- {
		l := strings.TrimSpace(lines[i])
		if strings.HasPrefix(l, "error:") || strings.HasPrefix(l, "Error:") {
			if len(l) > max {
				return l[:max]
			}
			return l
		}
	}
	if len(s) > max {
		return "…" + s[len(s)-max:]
	}
	return s
}

func truncErr(s string) string {
	if len(s) > 500 {
		return s[:500]
	}
	return s
}
