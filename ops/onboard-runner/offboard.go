package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

// The offboard leg (goat#372): poll offboard-requested proposals, claim
// atomically, run the reversible native revoke, write the terminal result
// back. Same engine as the CLI cut — the runner execs `goat admin onboard
// --revoke`, it does not reimplement it. See
// docs/design/credential-revocation-and-offboarding.md.

// processOffboard handles one queued offboard end to end. A failure is
// recorded back on the proposal (offboard-failed) rather than crashing the
// loop — the row stays loud until the cut actually lands.
func (r *runner) processOffboard(ctx context.Context, p proposal) {
	l := r.log.With("id", p.ID, "handle", p.Handle, "env", p.Env, "requested_by", p.OffboardRequestedBy)

	if r.dryRun {
		l.Info("dry-run: would offboard (no claim / revoke / writeback)")
		return
	}

	won, err := r.pc.offboardClaim(ctx, p.ID)
	if err != nil {
		l.Error("offboard claim failed", "err", err)
		return
	}
	if !won {
		l.Debug("offboard claim lost (already claimed / not requested) — skipping")
		return
	}
	l.Info("offboard claimed")

	if err := checkHandleEnv(p.Handle, p.Env); err != nil {
		l.Error("refusing offboard request", "err", err)
		_ = r.pc.offboardResult(ctx, p.ID, "offboard-failed", truncErr(err.Error()), "", "")
		return
	}

	// The admin.offboard operator-action records bracket the revoke. They
	// never gate it: the cut runs whatever the audit fabric says, and the
	// audit outcome is written back with the result so an unrecorded cut is
	// visible on the console, not only in this log.
	var auditProblems []string
	audit := func(outcome, errMsg string) {
		if aerr := r.emitOffboardAudit(ctx, p, outcome, errMsg); aerr != nil {
			auditProblems = append(auditProblems, outcome+": "+aerr.Error())
		}
	}
	auditResult := func() (string, string) {
		if len(auditProblems) == 0 {
			return "recorded", ""
		}
		return "failed", truncErr(strings.Join(auditProblems, "; "))
	}

	revoke := r.revoke
	if revoke == nil {
		revoke = r.execRevoke
	}
	audit("started", "")
	if err := revoke(ctx, p); err != nil {
		l.Error("revoke failed", "err", err)
		audit("failed", truncErr(err.Error()))
		oa, oaErr := auditResult()
		_ = r.pc.offboardResult(ctx, p.ID, "offboard-failed", truncErr(err.Error()), oa, oaErr)
		return
	}
	l.Info("revoked")
	audit("ok", "")
	oa, oaErr := auditResult()
	if oa != "recorded" {
		l.Error("offboarded, but the operator-action record is incomplete", "operator_audit_error", oaErr)
	}
	if err := r.pc.offboardResult(ctx, p.ID, "offboarded", "", oa, oaErr); err != nil {
		l.Error("writeback (offboarded) failed", "err", err)
	}
}

// revokeArgs builds the argv for the reversible native revoke. Never --purge:
// permanent delete stays CLI-only (goat#372 spec B). --native is forced for
// the same reason the mint forces it — the runner cannot run Ansible, so a
// non-native-serviceable case must fail fast with the CLI's own error.
func revokeArgs(p proposal, repoRoot string) []string {
	args := []string{"admin", "onboard", p.Handle, "--revoke", "--native"}
	if p.Env != "" {
		args = append(args, "--env", p.Env)
	}
	return append(args, "--repo-root", repoRoot)
}

// execRevoke runs the reversible native revoke, stamping GOAT_OPERATOR_HANDLE
// with the runner's ID — the same runner-executed/operator-approved
// attribution as the mint. The revoke's own operator-action record therefore
// names the runner, the executor; processOffboard brackets it with
// admin.offboard records whose actor is the approving admin's session
// identity, so the operator-action trail names both parties to the cut.
func (r *runner) execRevoke(ctx context.Context, p proposal) error {
	args := revokeArgs(p, r.repoRoot)
	cmd := exec.CommandContext(ctx, r.goatBin, args...)
	cmd.Dir = r.repoRoot
	cmd.Env = append(os.Environ(), "GOAT_OPERATOR_HANDLE="+r.runnerID)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("%s %s: %w: %s", r.goatBin, strings.Join(args, " "), err, mintErrSnippet(out))
	}
	return nil
}

// departureReason matches the three reason classes of a departure and
// departureDay its effective day; anything else from the console is left out
// of the operator-action record rather than copied into it.
var (
	departureReason = regexp.MustCompile(`^(new-position|left-service|other)$`)
	departureDay    = regexp.MustCompile(`^\d{4}-\d{2}-\d{2}$`)
)

// departureDigest is the departure's part of the admin.offboard record:
// "reason=<class> effective=<day>", so the audit trail says why the person
// left. "" when the console recorded none.
func departureDigest(p proposal) string {
	if !departureReason.MatchString(p.OffboardReasonClass) {
		return ""
	}
	out := "reason=" + p.OffboardReasonClass
	if departureDay.MatchString(p.OffboardEffective) {
		out += " effective=" + p.OffboardEffective
	}
	return out
}

// identityUUID matches a Kratos identity id.
var identityUUID = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)

// offboardApproverActor is the operator-action actor naming the approving
// admin: "kratos-<identity id>", the session identity the console recorded
// at the click. ok is false when the row carries no well-formed identity.
func offboardApproverActor(p proposal) (string, bool) {
	id := strings.ToLower(strings.TrimSpace(p.OffboardRequestedByIdentity))
	if !identityUUID.MatchString(id) {
		return "", false
	}
	return "kratos-" + id, true
}

// emitOffboardAudit records one admin.offboard operator-action event. It is
// always attempted: under the approving admin's identity when the row carries
// one, else under this runner with the missing approver stated in the
// record's error field. When `goat audit emit` fails (it is the fallible path
// and keeps no fallback of its own), the record is appended to the runner's
// fallback log. The returned error says what is wrong with the trail (no
// approver, not on the fabric, not in the fallback either); nil means the
// record reached the fabric naming the approver. It never blocks the cut.
func (r *runner) emitOffboardAudit(ctx context.Context, p proposal, outcome, errMsg string) error {
	l := r.log.With("id", p.ID, "handle", p.Handle, "outcome", outcome, "requested_by", p.OffboardRequestedBy)
	var problems []string
	actor, ok := offboardApproverActor(p)
	if !ok {
		actor = r.runnerID
		missing := "approver identity not recorded (requested_by=" + strings.TrimSpace(p.OffboardRequestedBy) + ")"
		problems = append(problems, missing)
		if errMsg == "" {
			errMsg = missing
		} else {
			errMsg = truncErr(missing + "; " + errMsg)
		}
		l.Error("offboard approver identity missing: the admin.offboard record is written under the runner",
			"identity", p.OffboardRequestedByIdentity)
	}
	digest := "proposal=" + p.ID + " requested_by=" + strings.TrimSpace(p.OffboardRequestedBy) + " executed_by=" + r.runnerID
	if reason := departureDigest(p); reason != "" {
		digest += " " + reason
	}
	args := []string{"audit", "emit", "--actor", actor, "--surface", "id-admin-console",
		"--action", "admin.offboard", "--target", p.Handle, "--env", p.Env, "--outcome", outcome,
		"--args-digest", digest}
	if errMsg != "" {
		args = append(args, "--error", errMsg)
	}
	// #nosec G204 -- argv without a shell; goatBin is operator configuration.
	cmd := exec.CommandContext(ctx, r.goatBin, args...)
	cmd.Dir = r.repoRoot
	cmd.Env = append(os.Environ(), "GOAT_OPERATOR_HANDLE="+actor)
	if out, err := cmd.CombinedOutput(); err != nil {
		reason := strings.TrimSpace(fmt.Sprintf("%v: %s", err, strings.TrimSpace(string(out))))
		path, ferr := r.appendAuditFallback(operatorActionRecord{Actor: actor, Surface: "id-admin-console",
			Action: "admin.offboard", Target: p.Handle, Env: p.Env, Outcome: outcome, ErrorMsg: errMsg,
			Src: r.repoRoot, ArgsDigest: argsDigest(digest)})
		if ferr != nil {
			problems = append(problems, "not on the audit fabric ("+truncErr(reason)+") and not in the fallback log ("+ferr.Error()+")")
			l.Error("offboard operator-action record LOST: fabric emit and fallback both failed",
				"actor", actor, "err", reason, "fallback_err", ferr)
		} else {
			problems = append(problems, "not on the audit fabric ("+truncErr(reason)+"); kept in "+path)
			l.Error("offboard operator-action record not on the fabric; appended to the fallback log",
				"actor", actor, "err", reason, "fallback", path)
		}
	}
	if len(problems) == 0 {
		return nil
	}
	return errors.New(strings.Join(problems, "; "))
}

// operatorActionRecord is one fallback-log line, the field set and names of
// the CLI's own fallback log (goat-cli audit.rs fallback_json).
type operatorActionRecord struct {
	Actor      string `json:"actor"`
	Surface    string `json:"surface"`
	Action     string `json:"action"`
	Target     string `json:"target"`
	Env        string `json:"env"`
	Outcome    string `json:"outcome"`
	ErrorMsg   string `json:"error_msg"`
	Host       string `json:"host"`
	Src        string `json:"src"`
	ArgsDigest string `json:"args_digest"`
	TsUnixMs   int64  `json:"ts_unix_ms"`
}

// argsDigest is the SHA-256 hex of an already-redacted summary, as the CLI
// puts in args_digest.
func argsDigest(summary string) string {
	sum := sha256.Sum256([]byte(summary))
	return hex.EncodeToString(sum[:])
}

// appendAuditFallback appends rec as one JSON line to the fallback log
// (directory 0700, file 0600) and returns the path written.
func (r *runner) appendAuditFallback(rec operatorActionRecord) (string, error) {
	path := r.auditFallbackPath
	if path == "" {
		path = filepath.Join(r.repoRoot, "out", "audit", "operator-actions.log")
	}
	if rec.Host == "" {
		rec.Host, _ = os.Hostname()
	}
	if rec.TsUnixMs == 0 {
		rec.TsUnixMs = time.Now().UnixMilli()
	}
	line, err := json.Marshal(rec)
	if err != nil {
		return path, err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return path, err
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600) // #nosec G304 -- operator-configured path
	if err != nil {
		return path, err
	}
	if _, err := f.Write(append(line, '\n')); err != nil {
		_ = f.Close()
		return path, err
	}
	return path, f.Close()
}
