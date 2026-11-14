package access

import (
	"os"
	"path/filepath"
	"sync"
	"testing"
)

const rosterA = `{"orgs":[{"id":"org-a","members":[{"handle":"m1","type":"system","vendor_tag":"prod-a"}]}]}`
const rosterB = `{"orgs":[{"id":"org-b","members":[{"handle":"m1","type":"system","vendor_tag":"prod-b"}]}]}`

func writeRoster(t *testing.T, path, body string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatalf("write roster: %v", err)
	}
}

func TestDataRosterCache_reusesUnchangedFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "roster.json")
	writeRoster(t, path, rosterA)
	var c DataRosterCache
	first, err := c.Load(path)
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	second, err := c.Load(path)
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if first != second {
		t.Fatal("an unchanged roster was re-parsed")
	}
}

func TestDataRosterCache_sameSizeInPlaceRewriteIsSeen(t *testing.T) {
	// rosterA and rosterB are the same length; an immediate in-place rewrite
	// keeps size and inode and, on a coarse clock, mtime. The cache must still
	// see it — this is what "a roster edit applies on the next request" means.
	if len(rosterA) != len(rosterB) {
		t.Fatal("fixture rosters must be the same size")
	}
	path := filepath.Join(t.TempDir(), "roster.json")
	writeRoster(t, path, rosterA)
	var c DataRosterCache
	dr, err := c.Load(path)
	if err != nil || !dr.KnownProducer("prod-a") {
		t.Fatalf("first load: err=%v", err)
	}
	writeRoster(t, path, rosterB)
	dr, err = c.Load(path)
	if err != nil {
		t.Fatalf("second load: %v", err)
	}
	if dr.KnownProducer("prod-a") || !dr.KnownProducer("prod-b") {
		t.Fatal("served the stale roster after an in-place rewrite")
	}
}

func TestDataRosterCache_failsClosedAndDoesNotServeStale(t *testing.T) {
	path := filepath.Join(t.TempDir(), "roster.json")
	writeRoster(t, path, rosterA)
	var c DataRosterCache
	if _, err := c.Load(path); err != nil {
		t.Fatalf("load: %v", err)
	}
	writeRoster(t, path, `{"orgs": [`) // malformed
	if dr, err := c.Load(path); err == nil || dr != nil {
		t.Fatalf("malformed roster: got (%v, %v), want an error and no roster", dr, err)
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if dr, err := c.Load(path); err == nil || dr != nil {
		t.Fatalf("missing roster: got (%v, %v), want an error and no roster", dr, err)
	}
	// Back again with the original bytes: re-read, not resurrected from cache.
	writeRoster(t, path, rosterA)
	if dr, err := c.Load(path); err != nil || !dr.KnownProducer("prod-a") {
		t.Fatalf("restored roster: err=%v", err)
	}
}

func TestDataRosterCache_pathChangeReloads(t *testing.T) {
	dir := t.TempDir()
	a, b := filepath.Join(dir, "a.json"), filepath.Join(dir, "b.json")
	writeRoster(t, a, rosterA)
	writeRoster(t, b, rosterA) // same bytes, different path
	var c DataRosterCache
	first, _ := c.Load(a)
	second, err := c.Load(b)
	if err != nil {
		t.Fatalf("load b: %v", err)
	}
	if first == second {
		t.Fatal("a different path was served from the other path's cache entry")
	}
}

func TestDataRosterCache_concurrentLoads(t *testing.T) {
	path := filepath.Join(t.TempDir(), "roster.json")
	writeRoster(t, path, rosterA)
	var c DataRosterCache
	var wg sync.WaitGroup
	for i := 0; i < 32; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if dr, err := c.Load(path); err != nil || !dr.KnownProducer("prod-a") {
				t.Errorf("concurrent load: err=%v", err)
			}
		}()
	}
	wg.Wait()
}

func TestDataRoster_accessorsReturnCopies(t *testing.T) {
	path := filepath.Join(t.TempDir(), "roster.json")
	writeRoster(t, path, rosterA)
	dr, err := LoadDataRoster(path)
	if err != nil {
		t.Fatal(err)
	}
	orgs := dr.OrgsOfProducer("prod-a")
	if len(orgs) == 0 {
		t.Fatal("fixture: prod-a should belong to org-a")
	}
	orgs[0] = "mutated"
	if got := dr.OrgsOfProducer("prod-a")[0]; got == "mutated" {
		t.Fatal("OrgsOfProducer exposed the roster's backing array")
	}
}
