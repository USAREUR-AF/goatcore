package main

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

// The startup log is the first place an operator looks when a console-issued
// key is refused. Each shadowed handle gets its own structured WARN line, and
// the key value never appears in it.
func TestWarnShadowedOperatorKeys_logsEachHandleWithoutTheKey(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "ada.key"), []byte("ada:op_dir_ada\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	store, err := config.NewOperatorKeyStore(map[string]string{"ada": "op_env_ada"}, dir)
	if err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	warnShadowedOperatorKeys(slog.New(slog.NewJSONHandler(&buf, nil)), store)

	lines := strings.Split(strings.TrimSpace(buf.String()), "\n")
	if len(lines) != 1 {
		t.Fatalf("got %d log lines, want 1: %s", len(lines), buf.String())
	}
	var rec map[string]any
	if err := json.Unmarshal([]byte(lines[0]), &rec); err != nil {
		t.Fatalf("log line is not JSON: %v", err)
	}
	if rec["level"] != "WARN" {
		t.Errorf("level = %v, want WARN", rec["level"])
	}
	if rec["handle"] != "ada" {
		t.Errorf("handle = %v, want ada", rec["handle"])
	}
	if strings.Contains(buf.String(), "op_") {
		t.Errorf("log line leaked a key: %s", buf.String())
	}
}

func TestWarnShadowedOperatorKeys_silentWithoutCollision(t *testing.T) {
	store, err := config.NewOperatorKeyStore(map[string]string{"ada": "op_env_ada"}, t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	warnShadowedOperatorKeys(slog.New(slog.NewJSONHandler(&buf, nil)), store)
	warnShadowedOperatorKeys(slog.New(slog.NewJSONHandler(&buf, nil)), nil)
	if buf.Len() != 0 {
		t.Errorf("no collision must log nothing, got %s", buf.String())
	}
}

// GHSA-2w7v: an env key refused by a tombstone is in the boot log by handle.
func TestWarnShadowedOperatorKeys_logsEnvRevokedHandle(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, config.TombstoneFileName("ada")), []byte("ada\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	store, err := config.NewOperatorKeyStore(map[string]string{"ada": "op_env_ada"}, dir)
	if err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	warnShadowedOperatorKeys(slog.New(slog.NewJSONHandler(&buf, nil)), store)
	if !strings.Contains(buf.String(), `"handle":"ada"`) || !strings.Contains(buf.String(), "refused") {
		t.Errorf("want a WARN naming ada as refused, got %s", buf.String())
	}
	if strings.Contains(buf.String(), "op_") {
		t.Errorf("log line leaked a key: %s", buf.String())
	}
}
