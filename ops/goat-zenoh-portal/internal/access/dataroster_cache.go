package access

import (
	"crypto/sha256"
	"fmt"
	"os"
	"sync"
)

// DataRosterCache serves LoadDataRoster without re-indexing the roster on every
// request. The data-access handlers used to call LoadDataRoster per request (a
// dozen routes, some polled by grant-keyd), re-parsing and re-indexing the
// whole roster.json each time.
//
// Freshness is exact: every Load reads the file and compares a SHA-256 of its
// bytes with the cached copy's, re-parsing on any difference. So marking a
// member inactive in the roster still cuts that member's wrap access on the
// next derivation, which is the property dataroles.go relies on. (A stat-based
// key was rejected: file timestamps are jiffy-coarse, so an in-place rewrite of
// the same size inside one tick would keep the stale roster.) Reading and
// hashing the bytes is the cheap part; the parse and index is what is saved.
//
// Failure stays closed: a read or parse error is returned exactly as
// LoadDataRoster would return it, and drops the cached copy — a caller never
// gets a stale "good" roster in place of an error.
//
// The zero value is ready to use. A DataRoster is read-only after load (its
// slice-returning accessors return copies), so one may be shared by every
// concurrent request.
type DataRosterCache struct {
	mu     sync.Mutex
	path   string
	sum    [sha256.Size]byte
	roster *DataRoster
}

// Load returns the roster at path, re-parsing only when its bytes changed.
func (c *DataRosterCache) Load(path string) (*DataRoster, error) {
	raw, err := os.ReadFile(path)
	c.mu.Lock()
	defer c.mu.Unlock()
	if err != nil {
		c.roster = nil
		return nil, fmt.Errorf("read data-roster %s: %w", path, err)
	}
	sum := sha256.Sum256(raw)
	if c.roster != nil && c.path == path && c.sum == sum {
		return c.roster, nil
	}
	dr, err := parseDataRoster(path, raw)
	if err != nil {
		c.roster = nil
		return nil, err
	}
	c.path, c.sum, c.roster = path, sum, dr
	return dr, nil
}
