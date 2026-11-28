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
	"sync"
	"time"
)

// Runtime restarts the router and reports its health.
type Runtime interface {
	Restart(ctx context.Context) error
	// Stop stops the router; it stays stopped until the next Restart.
	Stop(ctx context.Context) error
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

// stopGraceSeconds is how long docker waits after SIGTERM before it kills the router. zenohd 1.9
// is PID 1 in its container and installs no SIGTERM handler, so the kernel drops the signal and
// the router keeps enforcing the old rules for the whole grace period (docker's default is 10 s).
// The pod router holds no storage, so there is nothing to flush.
const stopGraceSeconds = "1"

// Restart restarts the container.
func (d Docker) Restart(ctx context.Context) error {
	out, err := exec.CommandContext(ctx, d.bin(), "restart", "-t", stopGraceSeconds, d.Container).CombinedOutput() // #nosec G204 -- configured container name
	if err != nil {
		return fmt.Errorf("docker restart %s: %w: %s", d.Container, err, strings.TrimSpace(string(out)))
	}
	return nil
}

// Stop stops the container.
func (d Docker) Stop(ctx context.Context) error {
	out, err := exec.CommandContext(ctx, d.bin(), "stop", "-t", stopGraceSeconds, d.Container).CombinedOutput() // #nosec G204 -- configured container name
	if err != nil {
		return fmt.Errorf("docker stop %s: %w: %s", d.Container, err, strings.TrimSpace(string(out)))
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

	mu      sync.Mutex
	running []byte
	known   bool
	loaded  bool
}

// Running returns the configuration the router process was started with, which is not the file
// while a restart is under way: the old process keeps its rules until it ends. It is empty when
// the router was stopped. known is false when a restart failed or was made by someone else, and
// the last configuration known is returned.
func (a *Applier) Running() (conf []byte, known bool) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.load()
	return a.running, a.known
}

// load takes the first answer from the files: the live configuration, if it is recorded as applied.
func (a *Applier) load() {
	if a.loaded {
		return
	}
	a.loaded = true
	conf, err := os.ReadFile(a.Files.Config)
	if err != nil {
		return
	}
	applied, err := os.ReadFile(a.Files.AppliedSHA)
	a.running, a.known = conf, err == nil && strings.TrimSpace(string(applied)) == sum(conf)
}

func (a *Applier) setRunning(conf []byte, known bool) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.load()
	if known {
		a.running = conf
	}
	a.known = known
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
	a.Running() // settle the first answer before the files change under it
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

// ErrSuspended means a configuration that takes a grant away could not be applied, and the
// router runs the base configuration, which carries no source, or is stopped.
var ErrSuspended = errors.New("carriage is suspended")

// ApplyRemoval makes rendered the running configuration when it takes a grant away from the one
// that runs now. On failure the previous configuration is never restored: the router falls back
// to base, and is stopped when base fails too. base also becomes the last known good
// configuration, so a later rollback by first-boot cannot bring the grant back. A nil rendered
// goes straight to base.
func (a *Applier) ApplyRemoval(ctx context.Context, rendered, base []byte) error {
	a.Running() // settle the first answer before the files change under it
	if base == nil {
		// Nothing safe to roll back to: first-boot restores nothing when the file is absent.
		if err := os.Remove(a.Files.LastGood); err != nil && !errors.Is(err, os.ErrNotExist) {
			return a.halt(ctx, err, errors.New("the base configuration is missing"))
		}
	}
	cause := errors.New("no configuration could be rendered")
	if rendered != nil {
		if cause = a.install(ctx, rendered, base); cause == nil {
			return nil
		}
	}
	baseErr := errors.New("the base configuration is missing")
	if base != nil {
		if baseErr = a.install(ctx, base, base); baseErr == nil {
			return fmt.Errorf("%w: the configuration failed (%w); the base configuration is running and carries no source", ErrSuspended, cause)
		}
	}
	return a.halt(ctx, cause, baseErr)
}

// halt stops the router because no configuration can be run safely.
func (a *Applier) halt(ctx context.Context, cause, baseErr error) error {
	ctx, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	if err := a.Runtime.Stop(ctx); err != nil {
		a.setRunning(nil, false)
		return fmt.Errorf("%w: the configuration failed (%w), the base configuration failed (%w), and the router COULD NOT BE STOPPED: %w; stop it by hand", ErrSuspended, cause, baseErr, err)
	}
	a.setRunning(nil, true)
	return fmt.Errorf("%w: the configuration failed (%w) and the base configuration failed (%w); the router is stopped", ErrSuspended, cause, baseErr)
}

func (a *Applier) install(ctx context.Context, conf, lastGood []byte) error {
	if lastGood != nil {
		if err := writeFile(a.Files.LastGood, lastGood); err != nil {
			return fmt.Errorf("save last known good: %w", err)
		}
	}
	if err := writeFile(a.Files.Config, conf); err != nil {
		return fmt.Errorf("write router config: %w", err)
	}
	if err := a.restartAndWait(ctx); err != nil {
		return err
	}
	if err := writeFile(a.Files.AppliedSHA, []byte(sum(conf)+"\n")); err != nil {
		return fmt.Errorf("record applied config: %w", err)
	}
	return nil
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
	conf, err := a.Current()
	if err != nil {
		return err
	}
	if err := a.Runtime.Restart(ctx); err != nil {
		a.setRunning(nil, false)
		return err
	}
	a.setRunning(conf, true)
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
