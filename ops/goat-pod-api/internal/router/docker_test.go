package router

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDockerRestartGrace(t *testing.T) {
	dir := t.TempDir()
	log, bin := filepath.Join(dir, "args"), filepath.Join(dir, "docker")
	if err := os.WriteFile(bin, []byte("#!/bin/sh\necho \"$*\" > "+log+"\n"), 0o700); err != nil { // #nosec G306 -- test stub must be executable
		t.Fatal(err)
	}
	if err := (Docker{Container: "router", Binary: bin}).Restart(context.Background()); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(log) // #nosec G304 -- test temp dir
	if err != nil {
		t.Fatal(err)
	}
	if got := strings.TrimSpace(string(raw)); got != "restart -t 1 router" {
		t.Fatalf("docker called with %q", got)
	}
}
