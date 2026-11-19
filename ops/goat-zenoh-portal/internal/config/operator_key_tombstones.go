package config

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// GHSA-2w7v: console revocation is fail-closed across both key tiers.
//
// Before this, revoking a person in the console deleted their directory file
// and nothing else. A key for the same handle pinned in PORTAL_OPERATOR_API_KEYS
// (env wins a collision) kept working, the console and its offboarding check
// reported the person revoked, and the shadowed gauge dropped to zero at the
// moment the revocation failed.
//
// Now the console writes a tombstone per revoked handle and never removes it.
// On every load the portal:
//   - adds every tombstoned handle, and the SHA-256 of its env key value, to a
//     revocation LEDGER that only grows: kept in memory and written to a file
//     beside (never inside) the key directory. A key directory that goes
//     missing or empty therefore loses no revocation, at reload or at restart;
//     the handles the ledger holds with no tombstone on disk are logged at
//     ERROR and counted on /api/health (RevokedWithoutTombstone);
//   - refuses every env key whose handle is in the ledger, and every key (env
//     or directory) whose value hash is, so one value pinned under a second
//     handle is refused too. A directory key for a revoked handle with a NEW
//     value is accepted: a re-issue through the console keeps working;
//   - keeps a handle in Shadowed() after its directory file goes, for as long
//     as its env key is still accepted, so a hand delete is never silent;
//   - reports newly shadowed and newly env-revoked handles to the notice
//     callback, so a change made while the portal runs is logged when it
//     happens.
//
// Clearing a revocation is a deliberate host action: remove the env pin (or
// keep it on purpose), delete the tombstone, remove the handle and its hash
// from the ledger file, and restart the portal.

// RevokedFilePrefix names a revocation tombstone in the operator-key
// directory. Dot-prefixed so a portal that predates tombstones skips the file
// instead of failing to parse it.
const RevokedFilePrefix = ".revoked-"

// TombstoneFileName is the only tombstone name the portal reads: the prefix
// followed by the lowercase hex of the handle, exactly as the console writes
// it. Any other ".revoked-*" file, a key file under a dotted handle included,
// is ignored, so it can never plant a key line in the ledger or the log.
func TombstoneFileName(handle string) string {
	return RevokedFilePrefix + hex.EncodeToString([]byte(handle))
}

type tombstoneState struct {
	seen       map[string]struct{} // handles ever shadowed since boot
	shadowed   []string            // last published Shadowed()
	envRevoked []string
	notice     func(newlyShadowed, newlyEnvRevoked []string)

	ledgerPath string              // "" = in memory only
	ledger     map[string]struct{} // revoked handles; only grows
	ledgerKeys map[string]struct{} // sha256 hex of revoked key values; only grows
	unbacked   []string            // ledger handles with no tombstone on disk
	ledgerErr  error               // last ledger write failure
	// ledgerUnknown is set when the ledger file was empty at boot (a write
	// interrupted before it reached the disk). The ledger is rebuilt from the
	// tombstones on disk; revocations whose tombstone is gone are lost, so the
	// state is reported (ERROR, health) until a restart with a good ledger.
	ledgerUnknown bool
}

// revocationLedger is the on-disk form of the ledger.
type revocationLedger struct {
	Handles   []string `json:"handles"`
	KeySHA256 []string `json:"key_sha256"`
}

// DefaultRevocationLedgerPath is where the ledger lives when no path is
// configured: a dotfile beside the key directory, so it survives the
// directory's volume being absent or emptied. "" for an empty dir.
func DefaultRevocationLedgerPath(dir string) string {
	dir = strings.TrimSpace(dir)
	if dir == "" {
		return ""
	}
	clean := filepath.Clean(dir)
	return filepath.Join(filepath.Dir(clean), "."+filepath.Base(clean)+".revoked.json")
}

func keyHash(v string) string {
	h := sha256.Sum256([]byte(v))
	return hex.EncodeToString(h[:])
}

// loadRevocationLedger reads the ledger before the first load. A missing file
// is an empty ledger; one that cannot be read or parsed is an error.
func (s *OperatorKeyStore) loadRevocationLedger(path string) error {
	if strings.TrimSpace(path) == "" {
		path = DefaultRevocationLedgerPath(s.dir)
	}
	s.tomb.ledgerPath = path
	s.tomb.ledger = map[string]struct{}{}
	s.tomb.ledgerKeys = map[string]struct{}{}
	if path == "" {
		return nil
	}
	// A ledger inside the key directory is lost with the directory, which is
	// exactly what it exists to survive.
	if s.dir != "" {
		if rel, rerr := filepath.Rel(filepath.Clean(s.dir), filepath.Clean(path)); rerr == nil && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
			return fmt.Errorf("operator-key revocation ledger %q is inside the key directory %q; it must be outside it", path, s.dir)
		}
	}
	body, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		// No ledger. A first boot with no key directory has nothing to lose.
		// With a key directory present, a missing ledger cannot be told from
		// a wiped one (the role creates an empty ledger at deploy), so it is
		// the same unknown state as an empty ledger: rebuilt from the
		// tombstones, and reported.
		if fi, serr := os.Stat(s.dir); serr == nil && fi.IsDir() {
			s.tomb.ledgerUnknown = true
			slog.Error("operator-key revocation ledger is MISSING while the key directory exists; rebuilding it from the tombstones — a revocation whose tombstone is also gone is lost",
				"ledger", path, "dir", s.dir)
		}
		return nil
	}
	if err != nil {
		return fmt.Errorf("read operator-key revocation ledger %q: %w", path, err)
	}
	if len(strings.TrimSpace(string(body))) == 0 {
		// Zero-length: unknown state, not corruption. Refusing to boot would
		// lock every operator out, break-glass included. Every handle that
		// still has a tombstone is refused (the first load rebuilds the
		// ledger from them); the loss is said loudly.
		s.tomb.ledgerUnknown = true
		slog.Error("operator-key revocation ledger is EMPTY (interrupted write?); rebuilding it from the tombstones in the key directory — a revocation whose tombstone is also gone is lost",
			"ledger", path, "dir", s.dir)
		return nil
	}
	var l revocationLedger
	if err := json.Unmarshal(body, &l); err != nil {
		return fmt.Errorf("parse operator-key revocation ledger %q: %w", path, err)
	}
	for _, h := range l.Handles {
		s.tomb.ledger[h] = struct{}{}
	}
	for _, k := range l.KeySHA256 {
		s.tomb.ledgerKeys[k] = struct{}{}
	}
	return nil
}

// writeLedgerLocked persists the ledger (temp file + rename). Caller holds s.mu.
func (s *OperatorKeyStore) writeLedgerLocked() error {
	if s.tomb.ledgerPath == "" {
		return nil
	}
	l := revocationLedger{Handles: sortedKeys(s.tomb.ledger), KeySHA256: sortedKeys(s.tomb.ledgerKeys)}
	body, err := json.MarshalIndent(l, "", "  ")
	if err != nil {
		return err
	}
	// temp file, fsync, rename, fsync the directory: a crash leaves either the
	// old ledger or the new one, never an empty file.
	tmp := s.tomb.ledgerPath + ".tmp"
	f, err := os.OpenFile(tmp, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o600)
	if err != nil {
		return fmt.Errorf("write operator-key revocation ledger: %w", err)
	}
	if _, err := f.Write(append(body, '\n')); err != nil {
		_ = f.Close()
		return fmt.Errorf("write operator-key revocation ledger: %w", err)
	}
	if err := f.Sync(); err != nil {
		_ = f.Close()
		return fmt.Errorf("sync operator-key revocation ledger: %w", err)
	}
	if err := f.Close(); err != nil {
		return fmt.Errorf("close operator-key revocation ledger: %w", err)
	}
	if err := os.Rename(tmp, s.tomb.ledgerPath); err != nil {
		return fmt.Errorf("place operator-key revocation ledger: %w", err)
	}
	if d, err := os.Open(filepath.Dir(s.tomb.ledgerPath)); err == nil {
		serr := d.Sync()
		_ = d.Close()
		if serr != nil {
			return fmt.Errorf("sync operator-key revocation ledger dir: %w", serr)
		}
	}
	return nil
}

func sortedKeys(m map[string]struct{}) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

// SetNotice registers the callback told about newly shadowed and newly
// env-revoked handles on every later load. Handles only, never keys.
func (s *OperatorKeyStore) SetNotice(fn func(newlyShadowed, newlyEnvRevoked []string)) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.tomb.notice = fn
}

// EnvRevoked returns, sorted, the env-pinned handles whose env key is refused
// because the handle, or the key's value, is in the revocation ledger.
func (s *OperatorKeyStore) EnvRevoked() []string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return append([]string(nil), s.tomb.envRevoked...)
}

// RevokedWithoutTombstone returns, sorted, the revoked handles the ledger
// holds that have no tombstone in the key directory right now: the directory
// is missing, emptied or rebuilt. The revocations still hold; this says the
// directory no longer backs them.
func (s *OperatorKeyStore) RevokedWithoutTombstone() []string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return append([]string(nil), s.tomb.unbacked...)
}

// RevocationLedgerError is the last failure to write the ledger, or the
// ledger having been empty at boot (unknown state), or nil. The revocations
// stay enforced in memory; see tombstoneState.ledgerUnknown.
func (s *OperatorKeyStore) RevocationLedgerError() error {
	s.mu.RLock()
	defer s.mu.RUnlock()
	if s.tomb.ledgerErr != nil {
		return s.tomb.ledgerErr
	}
	if s.tomb.ledgerUnknown {
		return fmt.Errorf("revocation ledger %q was empty or missing at boot; rebuilt from tombstones", s.tomb.ledgerPath)
	}
	return nil
}

// applyTombstones adjusts a freshly merged key set before it is swapped in:
// revoked keys are removed (an env key of a revoked handle is replaced by the
// handle's directory key when that has a new value), and *shadowed becomes
// the sticky shadowed set. A tombstone read failure is returned and nothing
// is changed, so the caller keeps the last good set.
func (s *OperatorKeyStore) applyTombstones(next map[string]string, shadowed *[]string) error {
	onDisk, err := readRevokedHandles(s.dir)
	if err != nil {
		return err
	}
	var dirKeys map[string]string
	loadDirKeys := func() error {
		if dirKeys != nil {
			return nil
		}
		dirKeys = map[string]string{}
		return mergeOperatorKeysFromDir(s.dir, dirKeys)
	}

	s.mu.Lock()
	if s.tomb.ledger == nil {
		s.tomb.ledger = map[string]struct{}{}
		s.tomb.ledgerKeys = map[string]struct{}{}
	}
	grew := false
	for h := range onDisk {
		if _, ok := s.tomb.ledger[h]; !ok {
			s.tomb.ledger[h] = struct{}{}
			grew = true
		}
	}
	// Hash the env value of EVERY revoked handle the ledger holds, whether or
	// not its tombstone is on disk now, so the value stays refused if the
	// deployment later renames the env handle.
	for h := range s.tomb.ledger {
		if v, ok := s.env[h]; ok {
			if _, ok := s.tomb.ledgerKeys[keyHash(v)]; !ok {
				s.tomb.ledgerKeys[keyHash(v)] = struct{}{}
				grew = true
			}
		}
	}
	if grew {
		s.tomb.ledgerErr = s.writeLedgerLocked()
		if s.tomb.ledgerErr != nil {
			slog.Error("operator-key revocation ledger could not be written; revocations hold in memory but a restart relies on the tombstones alone",
				"ledger", s.tomb.ledgerPath, "err", s.tomb.ledgerErr)
		}
	}
	revokedHandle := cloneSet(s.tomb.ledger)
	revokedKey := cloneSet(s.tomb.ledgerKeys)
	var unbacked []string
	for h := range s.tomb.ledger {
		if _, ok := onDisk[h]; !ok {
			unbacked = append(unbacked, h)
		}
	}
	sort.Strings(unbacked)
	newUnbacked := newMembers(unbacked, toSet(s.tomb.unbacked))
	s.tomb.unbacked = unbacked
	s.mu.Unlock()
	if len(newUnbacked) > 0 {
		slog.Error("operator-key directory no longer holds the tombstone for revoked handles; the revocations are kept from the ledger",
			"handles", newUnbacked, "dir", s.dir, "ledger", s.tomb.ledgerPath)
	}

	var envRevoked []string
	for h, envKey := range s.env {
		_, byHandle := revokedHandle[h]
		_, byValue := revokedKey[keyHash(envKey)]
		if !byHandle && !byValue {
			continue
		}
		envRevoked = append(envRevoked, h)
		if next[h] != envKey {
			continue
		}
		if err := loadDirKeys(); err != nil {
			return err
		}
		delete(next, h)
		if k, ok := dirKeys[h]; ok && k != envKey {
			if _, refused := revokedKey[keyHash(k)]; !refused {
				next[h] = k
			}
		}
	}
	sort.Strings(envRevoked)
	// A revoked key value is refused under any handle, directory ones included.
	for h, k := range next {
		if _, refused := revokedKey[keyHash(k)]; refused {
			delete(next, h)
		}
	}

	s.mu.Lock()
	if s.tomb.seen == nil {
		s.tomb.seen = map[string]struct{}{}
	}
	for _, h := range *shadowed {
		s.tomb.seen[h] = struct{}{}
	}
	envRefused := toSet(envRevoked)
	var sticky []string
	for h := range s.tomb.seen {
		_, pinned := s.env[h]
		_, refused := envRefused[h]
		if !pinned || refused {
			delete(s.tomb.seen, h) // its env key is no longer accepted
			continue
		}
		sticky = append(sticky, h)
	}
	sort.Strings(sticky)
	newShadowed := newMembers(sticky, toSet(s.tomb.shadowed))
	newRevoked := newMembers(envRevoked, toSet(s.tomb.envRevoked))
	s.tomb.shadowed = sticky
	s.tomb.envRevoked = envRevoked
	notice := s.tomb.notice
	s.mu.Unlock()

	*shadowed = sticky
	if notice != nil && (len(newShadowed) > 0 || len(newRevoked) > 0) {
		notice(newShadowed, newRevoked)
	}
	return nil
}

func cloneSet(in map[string]struct{}) map[string]struct{} {
	out := make(map[string]struct{}, len(in))
	for k := range in {
		out[k] = struct{}{}
	}
	return out
}

// readRevokedHandles reads every revocation tombstone in dir: files named
// exactly TombstoneFileName(handle) whose content names that handle. The
// console writes a tombstone through a ".tmp-revoked-" temp name, which this
// prefix never matches, so a handle ending in ".tmp" is honoured.
func readRevokedHandles(dir string) (map[string]struct{}, error) {
	out := map[string]struct{}{}
	if strings.TrimSpace(dir) == "" {
		return out, nil
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return out, nil
		}
		return nil, fmt.Errorf("read operator-keys dir %q: %w", dir, err)
	}
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || !strings.HasPrefix(name, RevokedFilePrefix) {
			continue
		}
		raw, herr := hex.DecodeString(strings.TrimPrefix(name, RevokedFilePrefix))
		handle := string(raw)
		if herr != nil || handle == "" || TombstoneFileName(handle) != name || strings.ContainsAny(handle, ":\n") {
			continue // not a tombstone the console wrote (a dotted key file, say)
		}
		path := filepath.Join(dir, name)
		body, err := os.ReadFile(path)
		if err != nil {
			return nil, fmt.Errorf("read operator-key tombstone %q: %w", path, err)
		}
		// The handle is the one the file is named for; the content must name
		// it too, as a guard against a truncated or foreign file.
		for _, line := range strings.Split(string(body), "\n") {
			if strings.TrimSpace(line) == handle {
				out[handle] = struct{}{}
				break
			}
		}
	}
	return out, nil
}

func toSet(in []string) map[string]struct{} {
	out := make(map[string]struct{}, len(in))
	for _, v := range in {
		out[v] = struct{}{}
	}
	return out
}

func newMembers(now []string, prev map[string]struct{}) []string {
	var out []string
	for _, v := range now {
		if _, ok := prev[v]; !ok {
			out = append(out, v)
		}
	}
	return out
}
