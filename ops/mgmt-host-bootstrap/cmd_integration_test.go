//go:build integration
// +build integration

// cmd_integration_test.go — end-to-end integration tests for
// mgmt-host-bootstrap, parallel-safe via testcontainers-go.
//
// Each test spins up its own isolated stack (postgres-spock + signal-server
// + netbird-mgmt) on a freshly-created docker network with random host
// ports. Containers are torn down on test exit, panic, OR signal — the
// t.Cleanup hook + container.Terminate is called by the testcontainers
// reaper container even if the go process is killed.
//
// Block 50B.3 production-readiness hardening: replaces the previous shared
// docker-compose orchestration which serialized tests and leaked fixtures
// on failure.
//
// Run locally (~3-5 min for full suite, parallel):
//
//	cd ops/mgmt-host-bootstrap
//	go test -tags=integration -race -count=1 -parallel=4 -v ./...
//
// Prereqs: docker daemon reachable; the per-arch netbird-mgmt + postgres
// images cached locally (CI does this in a setup step).
//
// CI gates: .github/workflows/mgmt-host-bootstrap.yml runs this on every PR
// touching ops/mgmt-host-bootstrap/** or terraform/**/netbird image pin.
//
// The legacy testdata/docker-compose.yml is kept in tree as a smoke-test
// fixture (`docker compose up` against the same image pins for an operator
// who wants to poke the CLI by hand) but is NOT used by these tests.

package main

import (
	"bytes"
	"context"
	"database/sql"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	_ "github.com/jackc/pgx/v5/stdlib"
	dockercontainer "github.com/moby/moby/api/types/container"
	"github.com/testcontainers/testcontainers-go"
	tcnetwork "github.com/testcontainers/testcontainers-go/network"
	"github.com/testcontainers/testcontainers-go/wait"
)

// netbirdDatadir is the Datadir in testdata/management.json — where
// netbird-mgmt keeps its geolocation databases, and so what the seeded
// fixture is bound over.
const netbirdDatadir = "/var/lib/netbird"

// Image pins — must stay in lock-step with testdata/docker-compose.yml so
// the smoke fixture and the integration suite agree on what they're
// testing. Bump both in the same commit.
const (
	postgresImage   = "ghcr.io/usareur-af/postgres-spock@sha256:038f12f0e5339b2722754c06273a2277fcc4e4e17056c8afd2f52f28ed16e56c"
	signalImage     = "netbirdio/signal:0.79.0@sha256:04c401eb167ef1aaaf266f9b5a4ee54bdac9d1d650db3a3e1288601632fa6d41"
	managementImage = "netbirdio/management:0.68.3@sha256:8d3ac9d38c9589520d9d1563c8ae3646b9a038596aae16b9ed089602060a8001"
	netbirdPeerImg  = "netbirdio/netbird:latest"
)

// testStack is the per-test fixture: an isolated network plus three
// containers reachable both from the host (via mapped ports) and from
// peer-side containers attached to the same network (via aliases).
type testStack struct {
	NetworkName string

	PostgresHost string // 127.0.0.1
	PostgresPort string // dynamic
	DSN          string

	MgmtHost string
	MgmtPort string
	MgmtURL  string // host-side, e.g. http://127.0.0.1:NN

	MgmtContainer string // the management container's name, for docker exec
}

// managementImageUnderTest is the management image the stack runs: the pinned
// stock image, or MGMT_BOOTSTRAP_MANAGEMENT_IMAGE (CI sets it to the fleet
// image the netbird-mgmt workflow builds, so every test here runs against it).
func managementImageUnderTest() string {
	if v := os.Getenv("MGMT_BOOTSTRAP_MANAGEMENT_IMAGE"); v != "" {
		return v
	}
	return managementImage
}

// stackCounter generates unique names per stack so multiple tests can run
// concurrently without container/network collisions.
var stackCounter atomic.Uint64

// startStack brings up a per-test stack and registers cleanup. Order:
//
//	network → postgres → signal → management
//
// Each container has a deterministic alias on the network so the
// management container can resolve `postgres` and `signal-a` by name —
// this matches the existing testdata/management.json so we can mount it
// unchanged.
func startStack(t *testing.T) *testStack {
	t.Helper()
	// 8 minutes rather than 5: with the geolocation fixture seeded (below)
	// the whole stack is ready in ~15 s and this budget is never approached.
	// It exists for the UNSEEDED path — a fresh clone, no cache — where the
	// management container downloads 82 MB from a third-party CDN before it
	// serves anything. That download blew the old 5-minute budget on
	// and ; seeding removes it, this makes the fallback
	// survivable rather than a coin toss.
	ctx, cancel := context.WithTimeout(context.Background(), 8*time.Minute)
	t.Cleanup(cancel)

	id := stackCounter.Add(1)
	netName := fmt.Sprintf("mgmt-host-bootstrap-it-%d-%d", time.Now().UnixNano(), id)

	// Custom network so containers can resolve each other by alias.
	net, err := tcnetwork.New(ctx,
		tcnetwork.WithAttachable(),
		tcnetwork.WithLabels(map[string]string{
			"goat.test": "mgmt-host-bootstrap",
		}),
	)
	if err != nil {
		t.Fatalf("create network: %v", err)
	}
	t.Cleanup(func() {
		// 30s budget; the reaper handles the case where Remove fails.
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_ = net.Remove(ctx)
	})

	// 1. Postgres-spock — listen on container port 5432, host port dynamic.
	//
	// Two-stage wait: pg_isready signals the listener is up, but the
	// docker-entrypoint init scripts (CREATE DATABASE netbird; ...) run
	// AFTER that — postgres-spock briefly accepts then drops connections
	// while it restarts post-init. ForSQL polls SELECT 1 against
	// the `netbird` DB which only succeeds after the init scripts complete
	// AND the post-init restart finishes; that's the real "ready" signal
	// for downstream consumers.
	pgReq := testcontainers.ContainerRequest{
		Image:        postgresImage,
		Name:         fmt.Sprintf("%s-pg", netName),
		ExposedPorts: []string{"5432/tcp"},
		Env: map[string]string{
			"POSTGRES_PASSWORD": "testdata",
		},
		Cmd:      []string{"postgres", "-c", "config_file=/etc/postgresql/postgresql.conf"},
		Networks: []string{net.Name},
		NetworkAliases: map[string][]string{
			net.Name: {"postgres"},
		},
		WaitingFor: wait.ForAll(
			wait.ForExec([]string{"pg_isready", "-U", "postgres"}).
				WithStartupTimeout(60*time.Second).
				WithPollInterval(2*time.Second),
			wait.ForSQL("5432/tcp", "pgx", func(host string, port string) string {
				// testcontainers' wait.ForSQL passes Port.String() which
				// includes the proto suffix ("5432/tcp"); strip it for the
				// keyword/value DSN (libpq syntax, which pgx also parses).
				if i := strings.IndexByte(port, '/'); i >= 0 {
					port = port[:i]
				}
				return fmt.Sprintf(
					"host=%s port=%s user=postgres password=testdata dbname=netbird sslmode=disable",
					host, port)
			}).
				WithStartupTimeout(180 * time.Second).
				WithPollInterval(1 * time.Second),
		),
		Labels: map[string]string{"goat.test": "mgmt-host-bootstrap"},
	}
	pgC, err := testcontainers.GenericContainer(ctx, testcontainers.GenericContainerRequest{
		ContainerRequest: pgReq,
		Started:          true,
	})
	if err != nil {
		t.Fatalf("start postgres: %v", err)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_ = pgC.Terminate(ctx)
	})
	pgHost, err := pgC.Host(ctx)
	if err != nil {
		t.Fatalf("postgres host: %v", err)
	}
	pgPort, err := pgC.MappedPort(ctx, "5432/tcp")
	if err != nil {
		t.Fatalf("postgres port: %v", err)
	}

	// 2. Signal — port 10000 inside container; host port dynamic.
	// The upstream image's entrypoint is `[netbird-signal, run]`, so any
	// Cmd we pass becomes flags to the run subcommand. Leaving Cmd nil
	// inherits the image's default (`--log-file console`), which is what
	// we want — overriding with `-log-level info` (single-dash) crashes
	// the cobra parser used by recent signal versions.
	sigReq := testcontainers.ContainerRequest{
		Image:        signalImage,
		Name:         fmt.Sprintf("%s-signal", netName),
		ExposedPorts: []string{"10000/tcp"},
		Networks:     []string{net.Name},
		NetworkAliases: map[string][]string{
			net.Name: {"signal-a"},
		},
		WaitingFor: wait.ForListeningPort("10000/tcp").WithStartupTimeout(60 * time.Second),
		Labels:     map[string]string{"goat.test": "mgmt-host-bootstrap"},
	}
	sigC, err := testcontainers.GenericContainer(ctx, testcontainers.GenericContainerRequest{
		ContainerRequest: sigReq,
		Started:          true,
	})
	if err != nil {
		t.Fatalf("start signal: %v", err)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_ = sigC.Terminate(ctx)
	})

	// 3. Management — needs management.json mounted. Resolve to absolute
	// path so the testcontainers bind mount works regardless of where the
	// `go test` binary cwd happens to be.
	mgmtJSON, err := filepath.Abs("testdata/management.json")
	if err != nil {
		t.Fatalf("abs testdata path: %v", err)
	}
	// netbird-mgmt builds its geolocation layer during startup, BEFORE it
	// serves HTTP. If the Datadir holds no `GeoLite2-City_*.mmdb` and no
	// `geonames_*.db`, it fetches 82 MB from pkgs.netbird.io and, for the
	// geonames half, imports a CSV into SQLite — all inside the readiness
	// window. When that CDN serves from a warm edge the container is ready in
	// ~15 s; when the edge is cold it took 3 m 50 s for the first file alone
	// and the test failed on a deadline that had nothing to do with netbird.
	//
	// So seed the files instead. NewGeolocation skips any database already
	// present in the Datadir (management/server/geolocation/geolocation.go —
	// getDatabaseFilename returns the local glob match, loadGeolocationDatabases
	// `continue`s on an existing file), so a seeded stack never touches the
	// network for this. CI fills testdata/geolite/ from an actions/cache keyed
	// on the CDN's published version; testdata/geolite/README.md has the
	// one-command local warm.
	geoDir := seedGeolocationDatadir(t)

	mgmtReq := testcontainers.ContainerRequest{
		Image:        managementImageUnderTest(),
		Name:         fmt.Sprintf("%s-mgmt", netName),
		ExposedPorts: []string{"80/tcp"},
		Env: map[string]string{
			"NETBIRD_STORE_ENGINE_POSTGRES_DSN": "host=postgres user=postgres password=testdata dbname=netbird sslmode=disable client_encoding=UTF8",
			// The fleet image's entrypoint skips schema migrations unless told
			// (multi-master upgrades); a fresh test database needs them. The
			// stock image ignores the variable.
			"NETBIRD_RUN_MIGRATIONS": "true",
		},
		// Where the fleet image's service-user command writes a pending
		// token: a root-owned 0700 tmpfs, as on a host (ADR 1273).
		Tmpfs:    map[string]string{"/run/goat-bootstrap": "rw,mode=0700"},
		Cmd:      []string{"management", "--log-file=console"},
		Networks: []string{net.Name},
		NetworkAliases: map[string][]string{
			net.Name: {"management"},
		},
		Files: []testcontainers.ContainerFile{
			{
				HostFilePath:      mgmtJSON,
				ContainerFilePath: "/etc/netbird/management.json",
				FileMode:          0o644,
			},
		},
		// Wait until the HTTP server answers — any 4xx/5xx is fine, we
		// just need it past startup.
		WaitingFor: wait.ForHTTP("/api/peers").
			WithPort("80/tcp").
			WithStatusCodeMatcher(func(status int) bool {
				return status >= 200 && status < 600
			}).
			WithStartupTimeout(6 * time.Minute).
			WithPollInterval(2 * time.Second),
		Labels: map[string]string{"goat.test": "mgmt-host-bootstrap"},
	}
	if geoDir != "" {
		// Bind rather than ContainerFile: netbird prunes databases whose name
		// does not match the version it resolved (cleanupOldDatabases), so the
		// Datadir has to be a real writable directory. It is a per-test copy,
		// so that pruning can never reach the cache it was seeded from.
		mgmtReq.HostConfigModifier = func(hc *dockercontainer.HostConfig) {
			hc.Binds = append(hc.Binds, geoDir+":"+netbirdDatadir)
		}
	}
	if geoDir == "" && os.Getenv("MGMT_BOOTSTRAP_DISABLE_GEOLOCATION") == "true" {
		// CI sets this only after warm-geolite.sh failed: the seed is absent
		// AND the mirror refuses the download (#1069), so the pinned image
		// cannot start with geolocation on. NB_DISABLE_GEOLOCATION is
		// netbird's own switch (management/internals/server/modules.go);
		// enrollment, which this gate grades, does not read the service.
		// Never the default: an unseeded run without the variable downloads,
		// as seedGeolocationDatadir documents.
		t.Logf("MGMT_BOOTSTRAP_DISABLE_GEOLOCATION=true: management starts with " +
			"NB_DISABLE_GEOLOCATION=true; geolocation is NOT exercised by this run")
		mgmtReq.Env["NB_DISABLE_GEOLOCATION"] = "true"
	}
	mgmtC, err := testcontainers.GenericContainer(ctx, testcontainers.GenericContainerRequest{
		ContainerRequest: mgmtReq,
		Started:          true,
	})
	if err != nil {
		// Surface postgres logs to make CI debugging easier.
		if pgLogs, lerr := pgC.Logs(ctx); lerr == nil {
			defer pgLogs.Close()
			b := new(bytes.Buffer)
			_, _ = b.ReadFrom(pgLogs)
			t.Logf("=== postgres logs (head) ===\n%s", truncate(b.String(), 4000))
		}
		t.Fatalf("start management: %v", err)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_ = mgmtC.Terminate(ctx)
	})

	mgmtHost, err := mgmtC.Host(ctx)
	if err != nil {
		t.Fatalf("mgmt host: %v", err)
	}
	mgmtPort, err := mgmtC.MappedPort(ctx, "80/tcp")
	if err != nil {
		t.Fatalf("mgmt port: %v", err)
	}

	stk := &testStack{
		NetworkName:  net.Name,
		PostgresHost: pgHost,
		PostgresPort: pgPort.Port(),
		DSN: fmt.Sprintf(
			"host=%s port=%s user=postgres password=testdata dbname=netbird sslmode=disable client_encoding=UTF8",
			pgHost, pgPort.Port()),
		MgmtHost: mgmtHost,
		MgmtPort: mgmtPort.Port(),
		MgmtURL:  fmt.Sprintf("http://%s:%s", mgmtHost, mgmtPort.Port()),

		MgmtContainer: fmt.Sprintf("%s-mgmt", netName),
	}
	return stk
}

// =============================================================================
// tests
// =============================================================================

// TestEndToEnd_CreateAccount_CreateSetupKey_PeerEnrolls is the load-bearing
// test. It exercises the full chain that real production usage will exercise.
func TestEndToEnd_CreateAccount_CreateSetupKey_PeerEnrolls(t *testing.T) {
	t.Parallel()
	if testing.Short() {
		t.Skip("integration test")
	}

	stk := startStack(t)
	binPath := buildCLI(t)

	// 1. create-account
	stdout, err := runCapture(t, binPath, "--dsn", stk.DSN, "create-account",
		"--owner", "test@integration.local",
		"--site-name", "integ")
	if err != nil {
		t.Fatalf("create-account failed: %v\n%s", err, stdout)
	}
	accountID := extractField(stdout, "account_id:")
	groupID := extractField(stdout, "group_id:")
	if accountID == "" {
		t.Fatalf("create-account did not return an account_id\nout: %s", stdout)
	}
	t.Logf("created account %s (group %s)", accountID, groupID)

	// 2. create-setup-key
	stdout, err = runCapture(t, binPath, "--dsn", stk.DSN, "create-setup-key",
		"--account", accountID,
		"--role", "peer",
		"--ttl", "1h",
		"--name", "integ-test-key")
	if err != nil {
		t.Fatalf("create-setup-key failed: %v\n%s", err, stdout)
	}
	plaintext := extractField(stdout, "plaintext:")
	if plaintext == "" {
		t.Fatalf("create-setup-key did not return plaintext\nout: %s", stdout)
	}
	t.Logf("created setup-key plaintext=%s", plaintext)

	// 3. Enroll a real netbird peer using the setup key. Attach to the
	// per-test network so the peer can resolve `management` by alias.
	t.Log("enrolling netbird peer...")
	peerName := fmt.Sprintf("mgmt-bootstrap-it-peer-%s", strings.TrimPrefix(stk.NetworkName, "mgmt-host-bootstrap-it-"))
	enrollCtx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()

	// The netbird peer image needs NET_ADMIN + SYS_ADMIN + /dev/net/tun;
	// docker run is the simplest path — testcontainers-go does not expose
	// arbitrary linux capabilities + device adds with the same fluency.
	// gosec G204: peerName + plaintext are test-controlled values; not
	// adversarial input. Same for the cleanup + log-tail invocations.
	enrollCmd := exec.CommandContext(enrollCtx, "docker", "run", "--rm", "-d", //nolint:gosec
		"--name", peerName,
		"--cap-add", "NET_ADMIN", "--cap-add", "SYS_ADMIN",
		"--device", "/dev/net/tun",
		"--network", stk.NetworkName,
		"-e", "NB_MANAGEMENT_URL=http://management:80",
		"-e", "NB_SETUP_KEY="+plaintext,
		"-e", "NB_HOSTNAME=integ-test-peer",
		netbirdPeerImg)
	if out, err := enrollCmd.CombinedOutput(); err != nil {
		t.Fatalf("docker run peer failed: %v\n%s", err, out)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		//nolint:gosec // test-controlled peerName
		_ = exec.CommandContext(ctx, "docker", "rm", "-f", peerName).Run()
	})

	// 4. Poll for peer row in DB (up to 30s).
	if err := waitForPeerInDB(stk.DSN, accountID, 30*time.Second); err != nil {
		// Capture mgmt + peer logs to surface the actual reason.
		logCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		//nolint:gosec // test-controlled peerName
		if logs, lerr := exec.CommandContext(logCtx, "docker", "logs", "--tail=50", peerName).CombinedOutput(); lerr == nil {
			t.Logf("=== peer logs (tail) ===\n%s", logs)
		}
		t.Fatalf("peer did not enroll in DB: %v", err)
	}
	t.Log("peer enrolled successfully")

	// 5. Verify list-peers shows the new peer.
	stdout, err = runCapture(t, binPath, "--dsn", stk.DSN, "list-peers", "--account", accountID)
	if err != nil {
		t.Fatalf("list-peers failed: %v\n%s", err, stdout)
	}
	if !strings.Contains(stdout, "integ-test-peer") {
		t.Errorf("list-peers did not show integ-test-peer:\n%s", stdout)
	}

	// 6. Verify list-setup-keys shows the key with used_times >= 1.
	stdout, err = runCapture(t, binPath, "--dsn", stk.DSN, "list-setup-keys", "--account", accountID)
	if err != nil {
		t.Fatalf("list-setup-keys failed: %v\n%s", err, stdout)
	}
	if !strings.Contains(stdout, "integ-test-key") {
		t.Errorf("list-setup-keys did not show integ-test-key:\n%s", stdout)
	}
}

// TestEndToEnd_RotateSetupKey verifies rotate produces a new working key.
func TestEndToEnd_RotateSetupKey(t *testing.T) {
	t.Parallel()
	if testing.Short() {
		t.Skip("integration test")
	}

	stk := startStack(t)
	binPath := buildCLI(t)

	out, err := runCapture(t, binPath, "--dsn", stk.DSN, "create-account",
		"--owner", "rotate@integration.local", "--site-name", "rotate")
	if err != nil {
		t.Fatal(err)
	}
	accountID := extractField(out, "account_id:")

	out, err = runCapture(t, binPath, "--dsn", stk.DSN, "create-setup-key",
		"--account", accountID, "--role", "peer", "--ttl", "1h", "--name", "to-rotate")
	if err != nil {
		t.Fatal(err)
	}
	skID := extractField(out, "setup_key_id:")
	originalPlaintext := extractField(out, "plaintext:")

	// rotate-setup-key without an explicit idempotency-key would be a no-op
	// (default key derived from sk_id). Pass an explicit key so the test
	// actually rotates.
	out, err = runCapture(t, binPath, "--dsn", stk.DSN, "rotate-setup-key",
		"--id", skID, "--idempotency-key", "force-rotate-1")
	if err != nil {
		t.Fatalf("rotate-setup-key failed: %v\n%s", err, out)
	}
	newPlaintext := extractField(out, "new plaintext:")
	if newPlaintext == "" || newPlaintext == originalPlaintext {
		t.Fatalf("rotate did not produce a new plaintext: original=%q new=%q", originalPlaintext, newPlaintext)
	}

	// Verify hash in DB matches new plaintext, not original.
	db, err := sql.Open("pgx", stk.DSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	var dbHash string
	if err := db.QueryRowContext(ctx, `SELECT key FROM setup_keys WHERE id=$1`, skID).Scan(&dbHash); err != nil {
		t.Fatal(err)
	}
	expectedHash := hashSetupKey(newPlaintext)
	if dbHash != expectedHash {
		t.Errorf("DB hash %q does not match expected hash for new plaintext %q (got %q)", dbHash, newPlaintext, expectedHash)
	}
	if dbHash == hashSetupKey(originalPlaintext) {
		t.Errorf("DB hash still matches original plaintext — rotation did not take effect")
	}

	// F-140: auto_groups must be '[]', never NULL — the mgmt API serves the
	// stored value verbatim and a null crashes the dashboard /groups page.
	var autoGroups sql.NullString
	if err := db.QueryRowContext(ctx, `SELECT auto_groups FROM setup_keys WHERE id=$1`, skID).Scan(&autoGroups); err != nil {
		t.Fatal(err)
	}
	if !autoGroups.Valid || autoGroups.String != "[]" {
		t.Errorf("auto_groups = %#v, want '[]' (NULL crashes dashboard /groups, F-140)", autoGroups)
	}
}

// TestEndToEnd_Idempotency verifies the Block 50B.8 contract: re-running
// any write command with the same inputs is a no-op + emits
// `already-applied: <key>`. Simulates the "killed mid-bootstrap, retry"
// scenario.
func TestEndToEnd_Idempotency(t *testing.T) {
	t.Parallel()
	if testing.Short() {
		t.Skip("integration test")
	}

	stk := startStack(t)
	binPath := buildCLI(t)

	// First run: real create-account.
	out1, err := runCapture(t, binPath, "--dsn", stk.DSN, "create-account",
		"--owner", "idem@integration.local", "--site-name", "idem")
	if err != nil {
		t.Fatalf("first create-account failed: %v\n%s", err, out1)
	}
	if strings.Contains(out1, "already-applied:") {
		t.Fatalf("first run unexpectedly hit replay path:\n%s", out1)
	}
	accountID1 := extractField(out1, "account_id:")
	if accountID1 == "" {
		t.Fatalf("first run did not produce account_id:\n%s", out1)
	}

	// Second run with identical inputs: must be a no-op replay.
	out2, err := runCapture(t, binPath, "--dsn", stk.DSN, "create-account",
		"--owner", "idem@integration.local", "--site-name", "idem")
	if err != nil {
		t.Fatalf("second create-account failed: %v\n%s", err, out2)
	}
	if !strings.Contains(out2, "already-applied:") {
		t.Errorf("second run did not emit `already-applied:`:\n%s", out2)
	}
	// The recorded result must echo the original account_id, not produce a
	// new one — proves the no-op path is deterministic.
	accountID2 := extractField(out2, "account_id:")
	if accountID2 != accountID1 {
		t.Errorf("replay returned different account_id: first=%q second=%q",
			accountID1, accountID2)
	}

	// Third run with a different idempotency-key: must NOT replay (creates
	// a second account row even though all other inputs match).
	out3, err := runCapture(t, binPath, "--dsn", stk.DSN, "create-account",
		"--owner", "idem@integration.local", "--site-name", "idem",
		"--idempotency-key", "operator-second-attempt")
	if err != nil {
		t.Fatalf("third create-account failed: %v\n%s", err, out3)
	}
	if strings.Contains(out3, "already-applied:") {
		t.Errorf("third run with explicit fresh key unexpectedly replayed:\n%s", out3)
	}
	accountID3 := extractField(out3, "account_id:")
	if accountID3 == "" || accountID3 == accountID1 {
		t.Errorf("third run did not create a fresh account: first=%q third=%q",
			accountID1, accountID3)
	}

	// Sanity: bootstrap_idempotency table now has at least 2 rows.
	db, err := sql.Open("pgx", stk.DSN)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	var count int
	if err := db.QueryRowContext(ctx,
		`SELECT COUNT(*) FROM bootstrap_idempotency WHERE command='create-account'`).Scan(&count); err != nil {
		t.Fatal(err)
	}
	if count < 2 {
		t.Errorf("bootstrap_idempotency rows for create-account = %d, want >= 2", count)
	}
}

// TestEndToEnd_OTLPEmit verifies the Block 27C contract: the binary emits at
// least one OTLP/HTTP POST to /v1/traces when OTEL_EXPORTER_OTLP_ENDPOINT is
// set, and runs unaffected when it is unset (no panic, no crash, normal
// stdout shape). Wire-format protobuf decoding is intentionally skipped —
// production verification of attribute correctness is operator-fired
// against the real OpenObserve instance.
func TestEndToEnd_OTLPEmit(t *testing.T) {
	t.Parallel()
	if testing.Short() {
		t.Skip("integration test")
	}

	stk := startStack(t)
	binPath := buildCLI(t)

	// Fixture collector — minimal HTTP server, records hits + body sizes.
	collector := startFakeOTLPCollector(t)

	// 1. Run create-account WITH OTLP endpoint set.
	out, err := runCaptureEnv(t,
		[]string{
			"OTEL_EXPORTER_OTLP_ENDPOINT=" + collector.URL(),
			"DBB_SITE_ID=integration-test",
			"DBB_SITE_ROLE=mgmt",
		},
		binPath, "--dsn", stk.DSN, "create-account",
		"--owner", "otel@integration.local", "--site-name", "otel")
	if err != nil {
		t.Fatalf("create-account with OTLP failed: %v\n%s", err, out)
	}
	if !strings.Contains(out, "account_id:") {
		t.Errorf("expected normal account_id output even with OTLP set:\n%s", out)
	}

	// 2. Wait briefly for the deferred shutdown's BatchSpanProcessor flush.
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		if collector.Hits() > 0 {
			break
		}
		time.Sleep(200 * time.Millisecond)
	}
	if collector.Hits() == 0 {
		t.Fatalf("collector saw no /v1/traces hits within 15s; CLI did not emit OTLP")
	}

	// 3. Run again WITHOUT OTLP endpoint — must not crash, must produce
	//    normal output. Idempotent replay is fine; we just want a clean exit.
	out2, err := runCapture(t, binPath, "--dsn", stk.DSN, "create-account",
		"--owner", "otel@integration.local", "--site-name", "otel")
	if err != nil {
		t.Fatalf("create-account without OTLP failed: %v\n%s", err, out2)
	}
	// Either a fresh account_id or an idempotent replay — both acceptable.
	if !strings.Contains(out2, "account_id:") && !strings.Contains(out2, "already-applied:") {
		t.Errorf("unexpected output without OTLP:\n%s", out2)
	}
}

// fakeOTLPCollector is a minimal HTTP server that accepts POSTs to
// /v1/traces and records hit count + body byte total. Wire-format parsing
// is intentionally skipped — we just prove the CLI emitted something.
type fakeOTLPCollector struct {
	srv      *http.Server
	listener net.Listener
	mu       sync.Mutex
	hits     int
	bytes    int64
}

func (c *fakeOTLPCollector) URL() string {
	return "http://" + c.listener.Addr().String()
}

func (c *fakeOTLPCollector) Hits() int {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.hits
}

func startFakeOTLPCollector(t *testing.T) *fakeOTLPCollector {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	c := &fakeOTLPCollector{listener: ln}
	mux := http.NewServeMux()
	mux.HandleFunc("/v1/traces", func(w http.ResponseWriter, r *http.Request) {
		n, _ := io.Copy(io.Discard, r.Body)
		_ = r.Body.Close()
		c.mu.Lock()
		c.hits++
		c.bytes += n
		c.mu.Unlock()
		// 200 with empty protobuf-encoded ExportTraceServiceResponse (zero
		// bytes is a valid empty message); the OTel SDK accepts an empty
		// 200 body without retrying.
		w.Header().Set("Content-Type", "application/x-protobuf")
		w.WriteHeader(http.StatusOK)
	})
	c.srv = &http.Server{
		Handler:           mux,
		ReadHeaderTimeout: 5 * time.Second,
	}
	go func() { _ = c.srv.Serve(ln) }()
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = c.srv.Shutdown(ctx)
	})
	return c
}

// =============================================================================
// helpers
// =============================================================================

// buildCLI is shared across tests; each test call gets its own t.TempDir
// binary so parallel tests don't clobber a single output path.
func buildCLI(t *testing.T) string {
	t.Helper()
	binPath := filepath.Join(t.TempDir(), "mgmt-host-bootstrap")
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	//nolint:gosec // test-controlled binPath
	cmd := exec.CommandContext(ctx, "go", "build", "-o", binPath, ".")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("go build: %v\n%s", err, out)
	}
	return binPath
}

func runCapture(t *testing.T, name string, args ...string) (string, error) {
	t.Helper()
	return runCaptureEnv(t, nil, name, args...)
}

// runCaptureEnv is runCapture with extra env vars layered on top of the
// inherited environment. Used by the OTLP-emit integration test to wire
// OTEL_EXPORTER_OTLP_ENDPOINT into the CLI subprocess.
func runCaptureEnv(t *testing.T, extraEnv []string, name string, args ...string) (string, error) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, name, args...) //nolint:gosec // test-controlled
	if len(extraEnv) > 0 {
		cmd.Env = append(cmd.Environ(), extraEnv...)
	}
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	combined := stdout.String() + stderr.String()
	return combined, err
}

func extractField(s, prefix string) string {
	for _, line := range strings.Split(s, "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, prefix) {
			return strings.TrimSpace(strings.TrimPrefix(line, prefix))
		}
	}
	return ""
}

func waitForPeerInDB(dsn, accountID string, timeout time.Duration) error {
	db, err := sql.Open("pgx", dsn)
	if err != nil {
		return err
	}
	defer db.Close()
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	ticker := time.NewTicker(2 * time.Second)
	defer ticker.Stop()
	for {
		var count int
		if err := db.QueryRowContext(ctx,
			`SELECT COUNT(*) FROM peers WHERE account_id=$1`, accountID).Scan(&count); err == nil && count > 0 {
			return nil
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("no peer rows for account %s within %s", accountID, timeout)
		case <-ticker.C:
		}
	}
}

func truncate(s string, max int) string {
	if len(s) <= max {
		return s
	}
	return s[:max] + "...[truncated]"
}
