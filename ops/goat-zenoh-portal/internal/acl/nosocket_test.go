package acl_test

import (
	"context"
	"errors"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
)

// GHSA-6828: with no Docker socket configured (the default), a real
// (non-fake) render and remove must not dial any socket. The dialer is
// replaced, so the test never reaches the host's /var/run/docker.sock: a
// fallback to that path shows up here as a recorded dial, not a real one.
func TestRender_noDockerSocketDialsNothing(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ContainerName:    "goat-zenoh-router",
	}
	var mu sync.Mutex
	var dialled []string
	acl.SetDialForTest(r, func(_ context.Context, path string) (net.Conn, error) {
		mu.Lock()
		dialled = append(dialled, path)
		mu.Unlock()
		return nil, errors.New("test dialer: no socket may be dialled")
	})

	if err := r.Render(context.Background(), []string{"acme"}); err != nil {
		t.Fatalf("render without a Docker socket: %v", err)
	}
	if err := r.RemoveVendor(context.Background(), "acme"); err != nil {
		t.Fatalf("remove without a Docker socket: %v", err)
	}
	if len(dialled) != 0 {
		t.Fatalf("no DockerSocketPath set, yet render/remove dialled %q", dialled)
	}
}

// The opt-in path still works: with DockerSocketPath set, render and remove
// each POST the SIGHUP to that socket, and to no other. The socket is a unix
// listener the test owns.
func TestRender_dockerSocketOptInSendsSIGHUP(t *testing.T) {
	configPath, storageDir := setupFixture(t)
	// A short directory: unix socket paths are limited to about 108 bytes.
	dir, err := os.MkdirTemp("", "acl-sock")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	sock := filepath.Join(dir, "docker.sock")
	ln, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatalf("listen %s: %v", sock, err)
	}
	var mu sync.Mutex
	var got []string
	srv := &http.Server{Handler: http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		mu.Lock()
		got = append(got, req.Method+" "+req.URL.RequestURI())
		mu.Unlock()
		w.WriteHeader(http.StatusNoContent)
	})}
	go srv.Serve(ln)
	t.Cleanup(func() { srv.Close() })

	r := &acl.Renderer{
		ConfigPath:       configPath,
		StorageDirPrefix: storageDir,
		ContainerName:    "goat-zenoh-router",
		DockerSocketPath: sock,
	}
	if err := r.Render(context.Background(), []string{"acme"}); err != nil {
		t.Fatalf("render: %v", err)
	}
	if err := r.RemoveVendor(context.Background(), "acme"); err != nil {
		t.Fatalf("remove: %v", err)
	}
	want := "POST /containers/goat-zenoh-router/kill?signal=SIGHUP"
	mu.Lock()
	defer mu.Unlock()
	if len(got) != 2 || got[0] != want || got[1] != want {
		t.Fatalf("socket saw %q, want two of %q", got, want)
	}
}
