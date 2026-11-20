// Package router applies a rendered configuration to the pod's Zenoh router: write it, restart
// the router (zenohd 1.9 loads an ACL change only on restart, F-156), wait for health, and roll
// back to the last known good configuration on failure. The files are the ones
// zagros-moon-pod host/first-boot.sh uses, so first-boot and this service agree on what is applied.
package router

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

// Runtime restarts the router and reports its health.
type Runtime interface {
	Restart(ctx context.Context) error
	// Healthy returns nil when the router is running and healthy, ErrStopped when it has
	// stopped (it will not become healthy by waiting), or another error while it is starting.
	Healthy(ctx context.Context) error
}

// ErrStopped means the router container is not running.
var ErrStopped = errors.New("router is not running")

// Docker drives the router container with the docker CLI.
type Docker struct {
	Container string
	Binary    string // default "docker"
}

func (d Docker) bin() string {
	if d.Binary != "" {
		return d.Binary
	}
	return "docker"
}

// Restart restarts the container.
func (d Docker) Restart(ctx context.Context) error {
	out, err := exec.CommandContext(ctx, d.bin(), "restart", d.Container).CombinedOutput() // #nosec G204 -- configured container name
	if err != nil {
		return fmt.Errorf("docker restart %s: %w: %s", d.Container, err, strings.TrimSpace(string(out)))
	}
	return nil
}

// Healthy reads the container state and health.
func (d Docker) Healthy(ctx context.Context) error {
	out, err := exec.CommandContext(ctx, d.bin(), "inspect", "-f", // #nosec G204 -- configured container name
		"{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{end}}", d.Container).Output()
	if err != nil {
		return fmt.Errorf("docker inspect %s: %w", d.Container, err)
	}
	fields := strings.Fields(string(out))
	state, health := "", ""
	if len(fields) > 0 {
		state = fields[0]
	}
	if len(fields) > 1 {
		health = fields[1]
	}
	switch {
	case state == "running" && (health == "healthy" || health == ""):
		return nil
	case state == "running" || state == "restarting":
		return fmt.Errorf("router %s (%s)", state, health)
	default:
		return fmt.Errorf("%w (state %q)", ErrStopped, state)
	}
}

// Files names the configuration files.
type Files struct {
	Config     string // the live configuration the router container mounts
	LastGood   string // copy of the configuration that ran before the current one
	AppliedSHA string // SHA-256 of the configuration the router has loaded
}

// Applier applies configurations.
type Applier struct {
	Files   Files
	Runtime Runtime
	Timeout time.Duration
	Poll    time.Duration
}

// Result reports one apply.
type Result struct {
	Changed    bool
	RolledBack bool
}

func sum(b []byte) string {
	h := sha256.Sum256(b)
	return hex.EncodeToString(h[:])
}

// Current returns the live configuration, or nil when there is none.
func (a *Applier) Current() ([]byte, error) {
	raw, err := os.ReadFile(a.Files.Config)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	return raw, err
}

// Apply makes rendered the running configuration. When it is already applied, nothing restarts.
// On failure the previous configuration is restored and restarted, and the error is returned.
func (a *Applier) Apply(ctx context.Context, rendered []byte) (Result, error) {
	var res Result
	current, err := a.Current()
	if err != nil {
		return res, fmt.Errorf("read router config: %w", err)
	}
	applied, err := os.ReadFile(a.Files.AppliedSHA)
	if err != nil && !errors.Is(err, os.ErrNotExist) {
		return res, fmt.Errorf("read applied config record: %w", err)
	}
	if current != nil && bytes.Equal(current, rendered) && strings.TrimSpace(string(applied)) == sum(rendered) {
		return res, nil
	}
	res.Changed = true
	if current != nil {
		if err := writeFile(a.Files.LastGood, current); err != nil {
			return res, fmt.Errorf("save last known good: %w", err)
		}
	}
	if err := writeFile(a.Files.Config, rendered); err != nil {
		return res, fmt.Errorf("write router config: %w", err)
	}
	applyErr := a.restartAndWait(ctx)
	if applyErr == nil {
		if err := writeFile(a.Files.AppliedSHA, []byte(sum(rendered)+"\n")); err != nil {
			return res, fmt.Errorf("record applied config: %w", err)
		}
		return res, nil
	}
	if current == nil {
		return res, fmt.Errorf("router rejected the configuration and there is none to roll back to: %w", applyErr)
	}
	res.RolledBack = true
	if err := writeFile(a.Files.Config, current); err != nil {
		return res, fmt.Errorf("router rejected the configuration (%w); restoring the previous one failed: %w", applyErr, err)
	}
	if err := a.restartAndWait(ctx); err != nil {
		return res, fmt.Errorf("router rejected the configuration (%w); the restored one also failed: %w", applyErr, err)
	}
	if err := writeFile(a.Files.AppliedSHA, []byte(sum(current)+"\n")); err != nil {
		return res, fmt.Errorf("record restored config: %w", err)
	}
	return res, fmt.Errorf("router rejected the configuration; the previous one is running again: %w", applyErr)
}

func (a *Applier) restartAndWait(ctx context.Context) error {
	timeout := a.Timeout
	if timeout == 0 {
		timeout = 120 * time.Second
	}
	poll := a.Poll
	if poll == 0 {
		poll = 2 * time.Second
	}
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	if err := a.Runtime.Restart(ctx); err != nil {
		return err
	}
	var last error
	for {
		last = a.Runtime.Healthy(ctx)
		if last == nil {
			return nil
		}
		if errors.Is(last, ErrStopped) {
			return last
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("router did not become healthy in %s: %w", timeout, last)
		case <-time.After(poll):
		}
	}
}

func writeFile(path string, data []byte) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(filepath.Dir(path), filepath.Base(path)+".tmp.*")
	if err != nil {
		return err
	}
	name := tmp.Name()
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		_ = os.Remove(name)
		return err
	}
	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		_ = os.Remove(name)
		return err
	}
	if err := tmp.Close(); err != nil {
		_ = os.Remove(name)
		return err
	}
	return os.Rename(name, path)
}
