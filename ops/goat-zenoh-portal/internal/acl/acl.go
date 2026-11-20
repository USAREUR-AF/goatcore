// Package acl applies per-vendor ACL + storage entries to the live
// zenohd config and applies them to the running router.
//
// The trial-fabric router config (rendered by Ansible role
// goat-zenoh) has four BEGIN/END marker pairs inside its top-level
// access_control + plugins.storage_manager blocks:
//
//	      // BEGIN: per-vendor write rules
//	      // END: per-vendor write rules
//	      // BEGIN: per-vendor subjects
//	      // END: per-vendor subjects
//	      // BEGIN: per-vendor policies
//	      // END: per-vendor policies
//	        // BEGIN: per-vendor storages
//	        // END: per-vendor storages
//
// On vendor approval, the portal writes one stanza per marker (ACL
// rule + subject + policy + rocksdb storage) and pushes the change to the
// running router through admin-REST (adminrest.go) when AdminREST is set;
// otherwise it takes effect at the next router restart. The shape MUST match what
// ops/ansible/roles/goat-zenoh/files/add-vendor.sh writes when
// fired manually from the host — the two paths are interchangeable.
//
// The on-disk path edits the live config in place. A separate
// access_control.json5 in the deprecated cert_uri_san_equals /
// cert_uri_san_matches shape is dead since the Zenoh 1.x migration, and a
// renderer writing it would return 201 + cert + tarball while the router config
// had zero ACL or storage entries for the new vendor.
//
// A SIGHUP to the router container is opt-in and off by default. With
// DockerSocketPath set, the portal sends it through the Docker Engine API
// (POST /containers/{name}/kill?signal=SIGHUP). zenohd 1.9 ignores SIGHUP,
// and a mesh-facing service holding the host's Docker socket is root on the
// host (GHSA-6828), so the role mounts no socket unless asked to. Do not
// reinstate a default socket path.
package acl

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"
)

// Renderer applies per-vendor ACL + storage entries to the live router
// config and applies them to the running router.
//
// Concurrent Render calls serialize via mu; the last writer wins.
type Renderer struct {
	mu sync.Mutex

	// ConfigPath is the live zenohd config.json5 the router reads.
	// Typically /etc/goat-zenoh/config.json5 (bind-mounted
	// r/w into the portal container).
	//
	// Backward compatibility note: the old field name was OutputPath
	// and referred to a separate access_control.json5 file. Renamed
	// to reflect that we now edit the live config in place.
	ConfigPath string

	// ContainerName is the docker container running zenohd. SIGHUP
	// is sent to this container via the Docker Engine API.
	// Typically goat-zenoh-router.
	ContainerName string

	// DockerSocketPath is the unix socket for the Docker Engine API,
	// used only to SIGHUP the router. Empty (the default) sends no
	// signal and opens no socket; there is no fallback path.
	DockerSocketPath string

	// dial opens DockerSocketPath. Nil means a plain unix dial; tests
	// replace it to observe whether a socket is dialled at all.
	dial func(ctx context.Context, path string) (net.Conn, error)

	// ReloadFake skips the SIGHUP step (the marker edit still runs).
	// Used in unit tests + when the operator is intentionally
	// editing the config without a live router (lab spike, etc.).
	ReloadFake bool

	// StorageDirPrefix is the rocksdb-storage parent directory the
	// per-vendor stanza references. Typically
	// /var/lib/goat-zenoh/rocksdb. Each vendor's storage lives
	// at <StorageDirPrefix>/<vendor>.
	StorageDirPrefix string

	// StorageTTLMs is the per-storage time_to_live in milliseconds.
	// Default 604800000 (7 days) when zero.
	StorageTTLMs int64

	// AdminREST, when non-nil, enables live config reload via
	// `@/<zid>/router/config/...` admin-space PUTs. The on-disk
	// write happens unconditionally; AdminREST.AddStorage is best-
	// effort on top, with verify-via-GET. When nil OR when the router
	// has `adminspace.permissions.write: false`, mint still succeeds
	// (on-disk in place) and operator-driven router restart is
	// required for the change to take effect on the live router.
	AdminREST *AdminRESTClient

	// SlotFragmentPath is where ApplySlotFragment writes the slot fragment
	// file. Empty = slot-fragment.json5 beside ConfigPath, which is where the
	// router role reads it from.
	SlotFragmentPath string
}

// Render reconciles the live config against approvedVendors: for each
// vendor not already present (idempotency by rule-id match), inserts
// the four marker-block stanzas + (opt-in) SIGHUPs the router. Vendors not in
// the list are NOT removed by this method — removals go through the
// future deleteVendor path.
//
// Caller (API layer) invokes this after a vendor is approved on
// creation or re-approved.
func (r *Renderer) Render(ctx context.Context, approvedVendors []string) error {
	r.mu.Lock()
	defer r.mu.Unlock()

	if r.ConfigPath == "" {
		return errors.New("acl renderer: ConfigPath is empty")
	}
	if r.StorageDirPrefix == "" {
		return errors.New("acl renderer: StorageDirPrefix is empty")
	}

	current, err := os.ReadFile(r.ConfigPath)
	if err != nil {
		return fmt.Errorf("acl renderer: read %s: %w", r.ConfigPath, err)
	}

	// Pre-flight: every marker must exist.
	for _, m := range markerEndLines() {
		if !strings.Contains(string(current), m) {
			return fmt.Errorf("acl renderer: missing marker in config: %q", m)
		}
	}

	updated := string(current)
	added := 0
	addedVendors := make([]string, 0, len(approvedVendors))
	for _, vendor := range approvedVendors {
		if vendor == "" {
			continue
		}
		if vendorAlreadyPresent(updated, vendor) {
			continue
		}
		updated = applyVendorStanzas(updated, vendor, r.StorageDirPrefix, r.storageTTLMs())
		added++
		addedVendors = append(addedVendors, vendor)
		// Best-effort dir create — zenohd's rocksdb backend won't
		// spawn a storage whose parent dir doesn't exist (it only
		// creates the rocksdb files inside an existing dir).
		if err := os.MkdirAll(filepath.Join(r.StorageDirPrefix, vendor), 0o755); err != nil {
			return fmt.Errorf("acl renderer: mkdir %s: %w", filepath.Join(r.StorageDirPrefix, vendor), err)
		}
	}

	if added == 0 {
		// Nothing to do; skip the rewrite + reload.
		slog.Info("acl render: no-op", "vendors", len(approvedVendors))
		return nil
	}

	if err := atomicWrite(r.ConfigPath, []byte(updated)); err != nil {
		return fmt.Errorf("acl renderer: write %s: %w", r.ConfigPath, err)
	}

	if r.ReloadFake {
		slog.Info("acl render: fake mode — skipping SIGHUP",
			"config", r.ConfigPath,
			"added", added)
		return nil
	}
	if r.ContainerName == "" {
		return errors.New("acl renderer: ContainerName empty and not in fake mode")
	}

	// F-156 live-add: try admin-REST first. Best-effort + lazy — failure
	// here doesn't fail mint; the on-disk write means the next restart
	// picks up the change. When AdminREST is wired AND the router has
	// adminspace.permissions.write: true, this is what makes the new
	// vendor's storage active without an operator restart.
	for _, v := range addedVendors {
		r.liveAddVendor(ctx, v)
	}

	// SIGHUP is opt-in (DockerSocketPath; GHSA-6828) and a no-op when it
	// is unset. zenohd v1.9.0 ignores it; it is kept for a router build
	// whose handler reloads config. Failure here doesn't fail mint since
	// the live-add path already ran.
	if err := r.sighup(ctx); err != nil {
		slog.Warn("acl render: SIGHUP failed (expected no-op on zenohd 1.9.0; non-fatal)",
			"container", r.ContainerName,
			"err", err)
	} else if r.DockerSocketPath != "" {
		slog.Info("acl render: SIGHUP delivered",
			"config", r.ConfigPath,
			"container", r.ContainerName,
			"added", added)
	}
	return nil
}

// RemoveVendor is the symmetric path to vendor approval: tears down the
// four wire-layer marker stanzas (rule + subject + policy + storage) the
// create path wrote for this vendor, (opt-in) SIGHUPs the router, then best-effort
// removes the per-vendor rocksdb storage directory (closing 83F revoke-
// path symmetry).
//
// Idempotent: if the vendor's stanzas aren't present in the live config,
// returns nil after at most a SIGHUP. The storage-dir removal is also
// best-effort; if it fails (open file handle, perm error) we log + ignore
// so the wire-layer revocation still takes effect.
//
// Caller (API layer) invokes this after the registry-row delete succeeds.
// Order is intentional: registry-row first so a half-failure (acl gone,
// row still there) can be reconciled by re-running RemoveVendor; the
// reverse order (acl-still-there, row gone) would leak privilege.
func (r *Renderer) RemoveVendor(ctx context.Context, vendor string) error {
	r.mu.Lock()
	defer r.mu.Unlock()

	if r.ConfigPath == "" {
		return errors.New("acl renderer: ConfigPath is empty")
	}
	if r.StorageDirPrefix == "" {
		return errors.New("acl renderer: StorageDirPrefix is empty")
	}
	if vendor == "" {
		return errors.New("acl renderer: vendor is empty")
	}

	current, err := os.ReadFile(r.ConfigPath)
	if err != nil {
		return fmt.Errorf("acl renderer: read %s: %w", r.ConfigPath, err)
	}

	if !vendorAlreadyPresent(string(current), vendor) {
		// Nothing in the live config for this vendor; still SIGHUP in
		// case a sibling session wrote something we don't see yet.
		slog.Info("acl remove: vendor not present in live config", "vendor", vendor)
		return nil
	}

	updated := removeVendorStanzas(string(current), vendor)
	if updated == string(current) {
		// Stanzas matched the idempotency check but nothing was actually
		// removed — shape drift, refuse rather than write a no-op.
		return fmt.Errorf("acl renderer: vendor %q present by idempotency check but stanza removal matched zero blocks (shape drift?)", vendor)
	}

	if err := atomicWrite(r.ConfigPath, []byte(updated)); err != nil {
		return fmt.Errorf("acl renderer: write %s: %w", r.ConfigPath, err)
	}

	if r.ReloadFake {
		slog.Info("acl remove: fake mode — skipping SIGHUP",
			"config", r.ConfigPath,
			"vendor", vendor)
	} else {
		if r.ContainerName == "" {
			return errors.New("acl renderer: ContainerName empty and not in fake mode")
		}
		// F-156 live-remove: best-effort admin-REST DELETE, symmetric
		// to the live-add path in Render(). Failure here doesn't fail
		// the revoke; on-disk write is in place.
		r.liveRemoveVendor(ctx, vendor)

		// Opt-in SIGHUP (see Render() comment).
		if err := r.sighup(ctx); err != nil {
			slog.Warn("acl remove: SIGHUP failed (expected no-op on zenohd 1.9.0; non-fatal)",
				"container", r.ContainerName,
				"err", err)
		} else if r.DockerSocketPath != "" {
			slog.Info("acl remove: SIGHUP delivered",
				"config", r.ConfigPath,
				"container", r.ContainerName,
				"vendor", vendor)
		}
	}

	// Best-effort rocksdb dir removal — if it fails the wire-layer
	// revocation is still in effect (no ACL → no writes); operator can
	// rm -rf the dir manually. Log but don't fail.
	storageDir := filepath.Join(r.StorageDirPrefix, vendor)
	if err := os.RemoveAll(storageDir); err != nil {
		slog.Warn("acl remove: storage dir cleanup failed (wire-layer revocation already effective)",
			"vendor", vendor,
			"dir", storageDir,
			"err", err)
	}

	return nil
}

func (r *Renderer) storageTTLMs() int64 {
	if r.StorageTTLMs > 0 {
		return r.StorageTTLMs
	}
	return 604800000 // 7d
}

// sighup posts to the Docker Engine API to send SIGHUP to the router
// container. Engine API endpoint: POST /containers/{id}/kill?signal=...
// HTTP over the unix socket — no Docker SDK dependency needed.
//
// It is opt-in: with no DockerSocketPath it sends nothing and returns nil.
// zenohd 1.9 ignores SIGHUP, so the signal carries no effect today, and the
// socket it needs makes the portal root on the host.
func (r *Renderer) sighup(ctx context.Context) error {
	sock := r.DockerSocketPath
	if sock == "" {
		return nil
	}
	client := &http.Client{
		Transport: &http.Transport{
			DialContext: func(ctx context.Context, _ string, _ string) (net.Conn, error) {
				if r.dial != nil {
					return r.dial(ctx, sock)
				}
				var d net.Dialer
				return d.DialContext(ctx, "unix", sock)
			},
		},
		Timeout: 10 * time.Second,
	}
	// Engine API requires the version prefix on newer daemons but
	// accepts an unversioned path on most. Path includes the
	// container name (engine resolves to id internally).
	u := "http://docker/containers/" + url.PathEscape(r.ContainerName) + "/kill?signal=SIGHUP"
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, u, nil)
	if err != nil {
		return err
	}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	// 204 No Content is success; anything else is an error.
	if resp.StatusCode == http.StatusNoContent {
		return nil
	}
	return fmt.Errorf("docker engine API returned %d: %s", resp.StatusCode, strings.TrimSpace(string(body)))
}

// vendorAlreadyPresent uses the rule id "trial-peer-w-<vendor>" as the
// idempotency anchor — unique enough across all four marker blocks.
func vendorAlreadyPresent(config, vendor string) bool {
	return strings.Contains(config, `"trial-peer-w-`+vendor+`"`)
}

// markerEndLines returns the literal END-marker lines the four
// per-vendor sections use. Order is the insert order.
func markerEndLines() []string {
	return []string{
		"      // END: per-vendor write rules",
		"      // END: per-vendor subjects",
		"      // END: per-vendor policies",
		"        // END: per-vendor storages",
	}
}

// applyVendorStanzas inserts rule + subject + policy + storage for
// one vendor. Indent matches the role's template; the storage stanza
// lives at 8-space indent (inside storages: object), the ACL stanzas
// at 6-space indent (inside rules:/subjects:/policies: arrays).
func applyVendorStanzas(config, vendor, storageDirPrefix string, ttlMs int64) string {
	rule := fmt.Sprintf(`      {
        id: "trial-peer-w-%s",
        messages: ["put", "delete"],
        flows: ["egress", "ingress"],
        permission: "allow",
        key_exprs: ["%s/**"],
      },
`, vendor, vendor)

	subject := fmt.Sprintf(`      {
        id: "vendor-%s",
        cert_common_names: ["%s-trial-peer"],
      },
`, vendor, vendor)

	policy := fmt.Sprintf(`      {
        rules: ["trial-peer-w-%s", "trial-peer-w-attest-schema"],
        subjects: ["vendor-%s"],
      },
`, vendor, vendor)

	// JSON5 unquoted identifiers must match [A-Za-z_$][A-Za-z_$0-9]* — hyphens
	// are not allowed. Vendor prefixes can contain hyphens (org-navy, dept-1,
	// cpe-demo, …) so the storage key MUST be quoted. The unquoted form parsed
	// fine for hyphen-free vendors but produced an unparseable config the moment
	// a hyphenated vendor was minted, silently wedging every subsequent router
	// restart — see F-155.
	storage := fmt.Sprintf(`        "%s": {
          key_expr: "%s/**",
          volume: {
            id: "rocksdb",
            dir: "%s/%s",
            on_closure: "do_nothing",
            create_db: true,
            db_recovery_enabled: false,
          },
          garbage_collect_period: 86400,
          time_to_live: %d,
        },
`, vendor, vendor, storageDirPrefix, vendor, ttlMs)

	config = strings.Replace(config,
		"      // END: per-vendor write rules",
		rule+"      // END: per-vendor write rules",
		1)
	config = strings.Replace(config,
		"      // END: per-vendor subjects",
		subject+"      // END: per-vendor subjects",
		1)
	config = strings.Replace(config,
		"      // END: per-vendor policies",
		policy+"      // END: per-vendor policies",
		1)
	config = strings.Replace(config,
		"        // END: per-vendor storages",
		storage+"        // END: per-vendor storages",
		1)
	return config
}

// removeVendorStanzas is the inverse of applyVendorStanzas: strips the
// four marker-block stanzas the create path inserted for this vendor.
// Anchors match the literal indent + JSON5 shape applyVendorStanzas
// emits, so the two stay in lockstep by construction (pinned by
// TestRemoveVendor_RoundTrip in acl_test.go).
//
// Each pattern is non-greedy and re-anchored on the closing `      },\n`
// (six-space indent for ACL blocks, eight for storage) so it can't run
// past one stanza into the next.
func removeVendorStanzas(config, vendor string) string {
	v := regexp.QuoteMeta(vendor)

	// rule { id: "trial-peer-w-<vendor>", ... }, — six-space indent.
	ruleRe := regexp.MustCompile(`(?m)^      \{\n        id: "trial-peer-w-` + v + `",\n(?:        [^\n]*\n)*?      \},\n`)
	config = ruleRe.ReplaceAllString(config, "")

	// subject { id: "vendor-<vendor>", ... }, — six-space indent.
	subjectRe := regexp.MustCompile(`(?m)^      \{\n        id: "vendor-` + v + `",\n(?:        [^\n]*\n)*?      \},\n`)
	config = subjectRe.ReplaceAllString(config, "")

	// policy { rules: ["trial-peer-w-<vendor>", ...], subjects: ["vendor-<vendor>"], }, — six-space indent.
	// Anchor on the rules: line that names this vendor's write rule.
	policyRe := regexp.MustCompile(`(?m)^      \{\n        rules: \["trial-peer-w-` + v + `"[^]]*\],\n        subjects: \["vendor-` + v + `"\],\n      \},\n`)
	config = policyRe.ReplaceAllString(config, "")

	// storage "<vendor>": { ... }, — eight-space indent (nested in storages: object).
	// First line matches the literal vendor-name key; closing `        },\n`.
	// Tolerates both quoted ("org-navy": {) and unquoted (aardwolf: {) forms so
	// it strips stanzas left over from before the F-155 fix that landed key
	// quoting unconditionally.
	storageRe := regexp.MustCompile(`(?m)^        "?` + v + `"?: \{\n(?:          [^\n]*\n)*?        \},\n`)
	config = storageRe.ReplaceAllString(config, "")

	return config
}

// atomicWrite is the standard tmp-file-rename pattern. Required for the
// router's config edit because the role-rendered file is bind-mounted
// into the zenoh-router container — partial writes during reload would
// break parsing.
func atomicWrite(path string, data []byte) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, ".zenoh-config-*.json5")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Chmod(tmp.Name(), 0o644); err != nil {
		return err
	}
	return os.Rename(tmp.Name(), path)
}
