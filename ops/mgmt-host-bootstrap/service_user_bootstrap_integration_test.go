//go:build integration
// +build integration

// The fleet management image's `service-user` command (ADR 1274,
// ops/netbird/patches/service-user-token-bootstrap.patch), against the image
// itself: a stack of Postgres, signal and the image under test, an account
// made by create-account, then the bootstrap identity minted, switched on with
// the read-back value, checked through the REST API, and minted again to show
// a re-run makes nothing. It runs only against an image that has the command:
// CI's netbird-mgmt workflow builds the fleet image and sets
// MGMT_BOOTSTRAP_MANAGEMENT_IMAGE and MGMT_BOOTSTRAP_REQUIRE_SERVICE_USER=1.

package main

import (
	"bytes"
	"context"
	"database/sql"
	b64 "encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"
)

const (
	suIntegrationName = "svc-zagros-netbird-infra"
	suPendingToken    = "/run/goat-bootstrap/svc-zagros-netbird-infra.token"
	suDir             = "/run/goat-bootstrap"
)

// dockerExec runs argv in the management container, with stdin, and returns
// stdout and stderr apart.
func dockerExec(t *testing.T, stk *testStack, stdin string, argv ...string) (string, string, error) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	args := append([]string{"exec", "-i", stk.MgmtContainer}, argv...)
	cmd := exec.CommandContext(ctx, "docker", args...) //nolint:gosec // test-controlled
	cmd.Stdin = strings.NewReader(stdin)
	var o, e bytes.Buffer
	cmd.Stdout, cmd.Stderr = &o, &e
	err := cmd.Run()
	return o.String(), e.String(), err
}

func suCommand(verb string, extra ...string) []string {
	return append([]string{"netbird-mgmt", "service-user", verb, "--config", "/etc/netbird/management.json", "--name", suIntegrationName}, extra...)
}

func suState(t *testing.T, stdout string) string {
	t.Helper()
	var st struct {
		State string `json:"state"`
	}
	lines := strings.Split(strings.TrimSpace(stdout), "\n")
	if err := json.Unmarshal([]byte(lines[len(lines)-1]), &st); err != nil {
		t.Fatalf("status is not JSON: %q", stdout)
	}
	return st.State
}

func apiGet(t *testing.T, stk *testStack, token, path string) (int, string) {
	t.Helper()
	req, err := http.NewRequest(http.MethodGet, stk.MgmtURL+path, nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Token "+token)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("GET %s: %v", path, err)
	}
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(b)
}

// tokenForms is the token raw and in base64 at each of its three alignments.
func tokenForms(token string) []string {
	forms := []string{token}
	for i := 0; i < 3; i++ {
		enc := b64.StdEncoding.EncodeToString([]byte(strings.Repeat("x", i) + token))
		forms = append(forms, enc[4:len(enc)-4])
	}
	return forms
}

func TestEndToEnd_FleetServiceUserBootstrap(t *testing.T) {
	if os.Getenv("MGMT_BOOTSTRAP_MANAGEMENT_IMAGE") == "" {
		if os.Getenv("MGMT_BOOTSTRAP_REQUIRE_SERVICE_USER") == "1" {
			t.Fatal("MGMT_BOOTSTRAP_REQUIRE_SERVICE_USER=1 but no MGMT_BOOTSTRAP_MANAGEMENT_IMAGE: the stock image has no service-user command")
		}
		t.Skip("set MGMT_BOOTSTRAP_MANAGEMENT_IMAGE to the fleet management image")
	}
	binPath := buildCLI(t)
	stk := startStack(t)
	if out, err := runCapture(t, binPath, "--dsn", stk.DSN, "create-account", "--owner", "owner@example.org", "--site-name", "zagros"); err != nil {
		t.Fatalf("create-account: %v\n%s", err, out)
	}
	var captured strings.Builder
	run := func(stdin string, argv ...string) (string, error) {
		o, e, err := dockerExec(t, stk, stdin, argv...)
		captured.WriteString(o + e)
		return o, err
	}

	// 1. A pending token: the file is there, the token does not work yet.
	out, err := run("", suCommand("bootstrap-user", "--dir", suDir)...)
	if err != nil || suState(t, out) != "pending" {
		t.Fatalf("bootstrap-user: %v %q", err, out)
	}
	perm, _, err := dockerExec(t, stk, "", "stat", "-c", "%a %U", suPendingToken)
	if err != nil || strings.TrimSpace(perm) != "600 root" {
		t.Fatalf("the pending file is %q (%v), not 600 root", perm, err)
	}
	// Only the test reads the plaintext, to play the store's read-back.
	token, _, err := dockerExec(t, stk, "", "cat", suPendingToken)
	if err != nil || len(token) != 40 || !strings.HasPrefix(token, "nbp_") {
		t.Fatalf("read the pending token: %v (length %d)", err, len(token))
	}
	// NetBird answers an unknown token 404 ("pat ... not found") or 401.
	if code, body := apiGet(t, stk, token, "/api/users"); code != http.StatusUnauthorized && code != http.StatusNotFound {
		t.Fatalf("a pending token answered %d, want 401 or 404: %s", code, truncate(body, 300))
	}

	// 2. A value the store did not keep switches nothing on.
	if _, err := run(token[:39]+"x", suCommand("bootstrap-token-install", "--dir", suDir)...); err == nil {
		t.Fatal("install took a value that is not the pending token")
	}
	if code, _ := apiGet(t, stk, token, "/api/users"); code != http.StatusUnauthorized && code != http.StatusNotFound {
		t.Fatalf("after a refused install the token answered %d, want 401 or 404", code)
	}

	// 3. The read-back value switches it on, and the pending files go.
	out, err = run(token+"\n", suCommand("bootstrap-token-install", "--dir", suDir)...)
	if err != nil || suState(t, out) != "installed" {
		t.Fatalf("bootstrap-token-install: %v %q", err, out)
	}
	code, body := apiGet(t, stk, token, "/api/users?service_user=true")
	if code != http.StatusOK || !strings.Contains(body, `"name":"`+suIntegrationName+`"`) || !strings.Contains(body, `"role":"admin"`) {
		t.Fatalf("GET /api/users: %d %s", code, truncate(body, 600))
	}
	if ls, _, _ := dockerExec(t, stk, "", "ls", "-A", suDir); strings.TrimSpace(ls) != "" {
		t.Fatalf("pending files left: %q", ls)
	}
	code, body = apiGet(t, stk, token, "/api/events/audit")
	if code == http.StatusNotFound {
		code, body = apiGet(t, stk, token, "/api/events")
	}
	if code != http.StatusOK || !strings.Contains(body, "service.user.create") || !strings.Contains(body, "personal.access.token.create") {
		t.Fatalf("the activity log does not show the service user and its token: %d %s", code, truncate(body, 600))
	}

	// 4. A re-run mints nothing.
	out, err = run("", suCommand("bootstrap-user", "--dir", suDir)...)
	if err != nil || suState(t, out) != "held" {
		t.Fatalf("re-run: %v %q", err, out)
	}
	db, err := sql.Open("pgx", stk.DSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	var users, pats int
	if err := db.QueryRow(`SELECT COUNT(*) FROM users WHERE is_service_user`).Scan(&users); err != nil {
		t.Fatal(err)
	}
	if err := db.QueryRow(`SELECT COUNT(*) FROM personal_access_tokens p JOIN users u ON u.id = p.user_id WHERE u.is_service_user AND u.service_user_name = $1`, suIntegrationName).Scan(&pats); err != nil {
		t.Fatal(err)
	}
	if users != 1 || pats != 1 {
		t.Fatalf("after a re-run: %d service users, %d tokens; want 1 and 1", users, pats)
	}

	// 5. A rotation: the old token works until the new one is installed, then
	// the old row goes by its id.
	out, err = run("", suCommand("bootstrap-user", "--dir", suDir, "--rotate")...)
	if err != nil || suState(t, out) != "pending" {
		t.Fatalf("bootstrap-user --rotate: %v %q", err, out)
	}
	next, _, err := dockerExec(t, stk, "", "cat", suPendingToken)
	if err != nil || len(next) != 40 || next == token {
		t.Fatalf("read the rotation's pending token: %v", err)
	}
	if code, _ := apiGet(t, stk, token, "/api/users"); code != http.StatusOK {
		t.Fatalf("the old token answered %d during the rotation, want 200", code)
	}
	out, err = run(next, suCommand("bootstrap-token-install", "--dir", suDir, "--rotate")...)
	if err != nil {
		t.Fatalf("bootstrap-token-install --rotate: %v %q", err, out)
	}
	var st struct {
		State      string `json:"state"`
		OldTokenID string `json:"old_token_id"`
	}
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &st); err != nil || st.State != "installed" || st.OldTokenID == "" {
		t.Fatalf("install --rotate status: %q", out)
	}
	for _, tok := range []string{token, next} {
		if code, _ := apiGet(t, stk, tok, "/api/users"); code != http.StatusOK {
			t.Fatalf("a token answered %d in the rotation window, want 200", code)
		}
	}
	if out, err = run("", suCommand("bootstrap-token-revoke", "--token-id", st.OldTokenID)...); err != nil {
		t.Fatalf("revoke the old row: %v %q", err, out)
	}
	if code, _ := apiGet(t, stk, token, "/api/users"); code == http.StatusOK {
		t.Fatal("the old token still works after its row was revoked")
	}
	if code, _ := apiGet(t, stk, next, "/api/users"); code != http.StatusOK {
		t.Fatalf("the new token answered %d after the rotation, want 200", code)
	}

	// 6. Neither token is in any output or the container log.
	logs, err := exec.Command("docker", "logs", stk.MgmtContainer).CombinedOutput() //nolint:gosec // test-controlled
	if err != nil {
		t.Fatalf("docker logs: %v", err)
	}
	all := captured.String() + string(logs)
	for _, f := range append(tokenForms(token), tokenForms(next)...) {
		if strings.Contains(all, f) {
			t.Fatalf("the token (or a base64 form of it) is in a command output or the container log")
		}
	}
	if strings.Contains(all, "nbp_") {
		t.Fatal("an nbp_ token prefix is in a command output or the container log")
	}
}
