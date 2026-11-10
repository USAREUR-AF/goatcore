package main

import (
	"flag"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
)

func TestSatlinkIntervalFlagAlias(t *testing.T) {
	for _, tc := range []struct {
		args []string
		want time.Duration
	}{
		{nil, 24 * time.Hour},
		{[]string{"--satlink-interval", "1h"}, time.Hour},
		{[]string{"--" + compat.SatlinkIntervalFlag, "2h"}, 2 * time.Hour},
		// both set: the earlier name wins
		{[]string{"--satlink-interval", "1h", "--" + compat.SatlinkIntervalFlag, "3h"}, 3 * time.Hour},
	} {
		fs := flag.NewFlagSet("serve", flag.ContinueOnError)
		cur, old := satlinkIntervalFlags(fs)
		if err := fs.Parse(tc.args); err != nil {
			t.Fatal(err)
		}
		if got := resolveSatlinkInterval(fs, *cur, *old); got != tc.want {
			t.Errorf("%v: got %s, want %s", tc.args, got, tc.want)
		}
	}
}

// TestSatlinkProbeCommandAlias runs the built binary under both subcommand
// names against an unreachable database: each must reach the store step,
// not the unknown-command path.
func TestSatlinkProbeCommandAlias(t *testing.T) {
	if testing.Short() {
		t.Skip("builds the binary")
	}
	bin := filepath.Join(t.TempDir(), "snitch")
	if out, err := exec.CommandContext(t.Context(), "go", "build", "-o", bin, ".").CombinedOutput(); err != nil { //nolint:gosec // test builds this package into its own temp dir
		t.Fatalf("build: %v\n%s", err, out)
	}
	for _, name := range []string{"satlink-probe", compat.SatlinkProbeCommand} {
		out, _ := exec.CommandContext(t.Context(), bin, name, "--db-url", "postgres://u:p@127.0.0.1:1/x?sslmode=disable&connect_timeout=1").CombinedOutput() //nolint:gosec // runs the binary this test just built
		if strings.Contains(string(out), "unknown command") || !strings.Contains(string(out), "store:") {
			t.Errorf("%s: want the store step, got:\n%s", name, out)
		}
	}
}
