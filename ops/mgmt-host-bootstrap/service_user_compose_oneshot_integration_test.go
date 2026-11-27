//go:build integration
// +build integration

// The exact shape standup phase 11 runs (ops/scripts/standup-bootstrap-token.py,
// ADR 1274): a compose project with a running management-a, and the
// service-user commands as one-shot `docker compose run --rm --no-deps
// --pull never -T` containers of that service beside it, with a root-owned
// 0700 tmpfs host directory mounted. The token is checked through the running
// server's API straight after install, with no restart. Runs only against the
// fleet image (MGMT_BOOTSTRAP_MANAGEMENT_IMAGE), as the other fleet test does.

package main

import (
	"bytes"
	"context"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

const oneShotCompose = `services:
  postgres:
    image: %s
    command: ["postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"]
    environment:
      POSTGRES_PASSWORD: testdata
    ports: ["127.0.0.1::5432"]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres -h 127.0.0.1"]
      interval: 2s
      retries: 60
  signal-a:
    image: %s
  management-a:
    image: %s
    depends_on:
      postgres: {condition: service_healthy}
      signal-a: {condition: service_started}
    environment:
      NETBIRD_STORE_ENGINE_POSTGRES_DSN: "host=postgres user=postgres password=testdata dbname=netbird sslmode=disable client_encoding=UTF8"
      NETBIRD_RUN_MIGRATIONS: "true"
      NB_DISABLE_GEOLOCATION: "true"
    volumes:
      - ./management.json:/etc/netbird/management.json:ro
      - management-data:/var/lib/netbird
    ports: ["127.0.0.1::80"]
    command: ["management", "--log-file=console"]
volumes:
  management-data:
`

func run(t *testing.T, dir, stdin string, name string, args ...string) (string, string, error) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Minute)
	defer cancel()
	cmd := exec.CommandContext(ctx, name, args...) //nolint:gosec // test-controlled
	cmd.Dir = dir
	cmd.Stdin = strings.NewReader(stdin)
	var o, e bytes.Buffer
	cmd.Stdout, cmd.Stderr = &o, &e
	err := cmd.Run()
	return o.String(), e.String(), err
}

func TestEndToEnd_FleetServiceUserOneShotBesideTheRunningServer(t *testing.T) {
	image := os.Getenv("MGMT_BOOTSTRAP_MANAGEMENT_IMAGE")
	if image == "" {
		if os.Getenv("MGMT_BOOTSTRAP_REQUIRE_SERVICE_USER") == "1" {
			t.Fatal("MGMT_BOOTSTRAP_REQUIRE_SERVICE_USER=1 but no MGMT_BOOTSTRAP_MANAGEMENT_IMAGE")
		}
		t.Skip("set MGMT_BOOTSTRAP_MANAGEMENT_IMAGE to the fleet management image")
	}
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "docker-compose.yml"), []byte(fmt.Sprintf(oneShotCompose, postgresImage, signalImage, image)), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg, err := os.ReadFile("testdata/management.json")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "management.json"), cfg, 0o644); err != nil {
		t.Fatal(err)
	}
	project := fmt.Sprintf("su-oneshot-%d", time.Now().UnixNano())
	compose := func(stdin string, args ...string) (string, string, error) {
		return run(t, dir, stdin, "docker", append([]string{"compose", "-p", project, "-f", "docker-compose.yml"}, args...)...)
	}
	t.Cleanup(func() { _, _, _ = compose("", "down", "-v", "--remove-orphans") })

	// The host directory: root-owned, mode 0700, on a tmpfs (/dev/shm), made by
	// a root container so the test needs no sudo; removed the same way.
	shm := filepath.Join("/dev/shm", project)
	if o, e, err := run(t, dir, "", "docker", "run", "--rm", "--user", "0", "--entrypoint", "", "-v", "/dev/shm:/s", postgresImage,
		"sh", "-c", "mkdir -m 0700 /s/"+project+" && chown 0:0 /s/"+project); err != nil {
		t.Fatalf("make the tmpfs directory: %v %s %s", err, o, e)
	}
	t.Cleanup(func() {
		_, _, _ = run(t, dir, "", "docker", "run", "--rm", "--user", "0", "--entrypoint", "", "-v", "/dev/shm:/s", postgresImage, "rm", "-rf", "/s/"+project)
	})

	if o, e, err := compose("", "up", "-d", "--wait", "management-a"); err != nil {
		t.Fatalf("compose up: %v %s %s", err, o, e)
	}
	port, _, err := compose("", "port", "management-a", "80")
	if err != nil {
		t.Fatalf("compose port: %v", err)
	}
	pgPort, _, err := compose("", "port", "postgres", "5432")
	if err != nil {
		t.Fatalf("compose port postgres: %v", err)
	}
	stk := &testStack{MgmtURL: "http://" + strings.TrimSpace(port)}
	deadline := time.Now().Add(3 * time.Minute)
	for {
		resp, err := http.Get(stk.MgmtURL + "/api/peers")
		if err == nil {
			resp.Body.Close()
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("management-a never answered: %v", err)
		}
		time.Sleep(2 * time.Second)
	}
	pg := strings.TrimSpace(pgPort)
	host, p, _ := strings.Cut(pg, ":")
	dsn := fmt.Sprintf("host=%s port=%s user=postgres password=testdata dbname=netbird sslmode=disable client_encoding=UTF8", host, p)
	if out, err := runCapture(t, buildCLI(t), "--dsn", dsn, "create-account", "--owner", "owner@example.org", "--site-name", "zagros"); err != nil {
		t.Fatalf("create-account: %v\n%s", err, out)
	}

	oneShot := []string{"run", "--rm", "--no-deps", "--pull", "never", "-T", "-v", shm + ":/run/goat-bootstrap", "management-a", "service-user"}
	var captured strings.Builder
	out, e, err := compose("", append(oneShot, "bootstrap-user", "--name", suIntegrationName, "--dir", "/run/goat-bootstrap")...)
	captured.WriteString(out + e)
	if err != nil || suState(t, out) != "pending" {
		t.Fatalf("one-shot bootstrap-user: %v %q %q", err, out, e)
	}
	// The test plays the store's read-back by reading the pending file.
	token, _, err := run(t, dir, "", "docker", "run", "--rm", "--user", "0", "--entrypoint", "", "-v", shm+":/d", postgresImage, "cat", "/d/"+suIntegrationName+".token")
	if err != nil || len(token) != 40 {
		t.Fatalf("read the pending token: %v (length %d)", err, len(token))
	}
	if code, _ := apiGet(t, stk, token, "/api/users"); code == http.StatusOK {
		t.Fatal("a pending token works before install")
	}
	out, e, err = compose(token+"\n", append(oneShot, "bootstrap-token-install", "--name", suIntegrationName, "--dir", "/run/goat-bootstrap")...)
	captured.WriteString(out + e)
	if err != nil || suState(t, out) != "installed" {
		t.Fatalf("one-shot bootstrap-token-install: %v %q %q", err, out, e)
	}
	// At once, no restart: the running server accepts the token and shows the
	// one-shot's activity events.
	code, body := apiGet(t, stk, token, "/api/users?service_user=true")
	if code != http.StatusOK || !strings.Contains(body, `"name":"`+suIntegrationName+`"`) {
		t.Fatalf("the running server did not accept the token at once: %d %s", code, truncate(body, 400))
	}
	code, body = apiGet(t, stk, token, "/api/events/audit")
	if code == http.StatusNotFound {
		code, body = apiGet(t, stk, token, "/api/events")
	}
	if code != http.StatusOK || !strings.Contains(body, "service.user.create") || !strings.Contains(body, "personal.access.token.create") {
		t.Fatalf("the one-shot's events are not in the running server's activity log: %d %s", code, truncate(body, 400))
	}
	logs, _, _ := compose("", "logs", "--no-color")
	all := captured.String() + logs
	for _, f := range tokenForms(token) {
		if strings.Contains(all, f) {
			t.Fatal("the token (or a base64 form of it) is in a one-shot's output or the project's logs")
		}
	}
}
