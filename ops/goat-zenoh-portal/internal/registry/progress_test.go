package registry_test

import (
	"context"
	"fmt"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

func TestL1DerivedFromBundleMintedAt(t *testing.T) {
	r := open(t)
	ctx := context.Background()

	mintedAt := int64(1716000000)
	bToken := "tok-aardwolf"
	bPath := "/bundles/aardwolf.tar.gz"
	if err := r.CreateVendor(ctx, &registry.Vendor{
		Prefix:         "aardwolf",
		APIKeyHash:     "h",
		BundleToken:    &bToken,
		BundlePath:     &bPath,
		BundleMintedAt: &mintedAt,
	}); err != nil {
		t.Fatalf("create vendor with bundle: %v", err)
	}
	if err := r.CreateVendor(ctx, &registry.Vendor{
		Prefix:     "otter",
		APIKeyHash: "h",
		// no bundle minted — should NOT get an l1_at
	}); err != nil {
		t.Fatalf("create vendor without bundle: %v", err)
	}

	rows, err := r.ListProgress(ctx, true)
	if err != nil {
		t.Fatalf("list progress: %v", err)
	}
	byPrefix := map[string]registry.VendorProgressRow{}
	for _, row := range rows {
		byPrefix[row.Prefix] = row
	}
	if got, ok := byPrefix["aardwolf"]; !ok {
		t.Fatalf("aardwolf missing from progress list")
	} else if got.L1At == nil || *got.L1At != mintedAt {
		t.Fatalf("aardwolf L1At = %v, want %d", got.L1At, mintedAt)
	}
	if got := byPrefix["otter"]; got.L1At != nil {
		t.Fatalf("otter L1At should be nil (no bundle minted); got %v", *got.L1At)
	}
}

func TestMarkL2StickyAndPeerCount(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create: %v", err)
	}

	// peer_count 0 — should NOT set l2_at.
	if err := r.MarkL2(ctx, "aardwolf", 0); err != nil {
		t.Fatalf("mark l2 zero: %v", err)
	}
	rows, _ := r.ListProgress(ctx, true)
	if rows[0].L2At != nil {
		t.Fatalf("L2At should be nil with peer_count=0")
	}
	if rows[0].PeerCount != 0 {
		t.Fatalf("PeerCount = %d, want 0", rows[0].PeerCount)
	}

	// peer_count > 0 — set l2_at.
	if err := r.MarkL2(ctx, "aardwolf", 2); err != nil {
		t.Fatalf("mark l2 nonzero: %v", err)
	}
	rows, _ = r.ListProgress(ctx, true)
	first := rows[0].L2At
	if first == nil {
		t.Fatalf("L2At should be set after peer_count=2")
	}
	if rows[0].PeerCount != 2 {
		t.Fatalf("PeerCount = %d, want 2", rows[0].PeerCount)
	}

	// Re-mark: l2_at stays the same (sticky), peer_count updates.
	time.Sleep(time.Second + 100*time.Millisecond)
	if err := r.MarkL2(ctx, "aardwolf", 5); err != nil {
		t.Fatalf("mark l2 again: %v", err)
	}
	rows, _ = r.ListProgress(ctx, true)
	if rows[0].L2At == nil || *rows[0].L2At != *first {
		t.Fatalf("L2At should be sticky; was %v, now %v", first, rows[0].L2At)
	}
	if rows[0].PeerCount != 5 {
		t.Fatalf("PeerCount = %d, want 5", rows[0].PeerCount)
	}
}

func TestMarkPublishL3L4Sticky(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create: %v", err)
	}

	ts1 := int64(1716100000)
	if err := r.MarkPublish(ctx, "aardwolf", ts1, true); err != nil {
		t.Fatalf("mark hello: %v", err)
	}
	rows, _ := r.ListProgress(ctx, true)
	if rows[0].L3At == nil || *rows[0].L3At != ts1 {
		t.Fatalf("L3At = %v, want %d", rows[0].L3At, ts1)
	}
	if rows[0].L4At != nil {
		t.Fatalf("L4At should still be nil after hello-world")
	}

	// Real publish lands later.
	ts2 := int64(1716200000)
	if err := r.MarkPublish(ctx, "aardwolf", ts2, false); err != nil {
		t.Fatalf("mark real: %v", err)
	}
	rows, _ = r.ListProgress(ctx, true)
	if rows[0].L4At == nil || *rows[0].L4At != ts2 {
		t.Fatalf("L4At = %v, want %d", rows[0].L4At, ts2)
	}
	if rows[0].L3At == nil || *rows[0].L3At != ts1 {
		t.Fatalf("L3At should stay at first value, got %v", rows[0].L3At)
	}
	if rows[0].LastPublishAt == nil || *rows[0].LastPublishAt != ts2 {
		t.Fatalf("LastPublishAt = %v, want %d", rows[0].LastPublishAt, ts2)
	}

	// Stale ts on L3 doesn't overwrite the later l3_at.
	older := int64(1715000000)
	if err := r.MarkPublish(ctx, "aardwolf", older, true); err != nil {
		t.Fatalf("mark older: %v", err)
	}
	rows, _ = r.ListProgress(ctx, true)
	if *rows[0].L3At != ts1 {
		t.Fatalf("L3At should stay at first value after older ts, got %d", *rows[0].L3At)
	}
	if *rows[0].LastPublishAt != ts2 {
		t.Fatalf("LastPublishAt should stay at later value, got %d", *rows[0].LastPublishAt)
	}
}

func TestListProgressSyntheticFilter(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create real: %v", err)
	}
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "notional", APIKeyHash: "h", Synthetic: true}); err != nil {
		t.Fatalf("create synth: %v", err)
	}

	rows, err := r.ListProgress(ctx, false)
	if err != nil {
		t.Fatalf("list non-synthetic: %v", err)
	}
	if len(rows) != 1 || rows[0].Prefix != "aardwolf" {
		t.Fatalf("non-synthetic filter: got %+v", rows)
	}

	all, err := r.ListProgress(ctx, true)
	if err != nil {
		t.Fatalf("list all: %v", err)
	}
	if len(all) != 2 {
		t.Fatalf("expected 2 vendors when include=synthetic, got %d", len(all))
	}
}

func TestListProgressOrderingByMaxLevel(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	for _, p := range []string{"alpha", "bravo", "charlie"} {
		if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: p, APIKeyHash: "h"}); err != nil {
			t.Fatalf("create %s: %v", p, err)
		}
	}
	// alpha → L1 only (bundle minted)
	at := int64(1716000000)
	_, err := r.DB().ExecContext(ctx, `UPDATE vendors SET bundle_minted_at = $1 WHERE prefix = 'alpha'`, at)
	if err != nil {
		t.Fatalf("update alpha: %v", err)
	}
	// bravo → L4
	if err := r.MarkPublish(ctx, "bravo", 1716200000, false); err != nil {
		t.Fatalf("mark bravo: %v", err)
	}
	// charlie → L2 only
	if err := r.MarkL2(ctx, "charlie", 1); err != nil {
		t.Fatalf("mark charlie l2: %v", err)
	}

	rows, err := r.ListProgress(ctx, true)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	if len(rows) != 3 {
		t.Fatalf("expected 3 rows, got %d", len(rows))
	}
	// max_level desc: bravo (4) > charlie (2) > alpha (1)
	if rows[0].Prefix != "bravo" || rows[1].Prefix != "charlie" || rows[2].Prefix != "alpha" {
		t.Fatalf("ordering wrong: %+v", []string{rows[0].Prefix, rows[1].Prefix, rows[2].Prefix})
	}
}

func TestAuditCursorRoundTrip(t *testing.T) {
	r := open(t)
	ctx := context.Background()

	c, err := r.GetAuditCursor(ctx, "site-A")
	if err != nil {
		t.Fatalf("get empty: %v", err)
	}
	if c.LastKey != "" {
		t.Fatalf("empty cursor should have empty LastKey, got %q", c.LastKey)
	}

	if err := r.SetAuditCursor(ctx, "site-A", "site-A/2026/05/21/14/1716340000000-256.ndjson"); err != nil {
		t.Fatalf("set: %v", err)
	}
	c2, err := r.GetAuditCursor(ctx, "site-A")
	if err != nil {
		t.Fatalf("get after set: %v", err)
	}
	if c2.LastKey != "site-A/2026/05/21/14/1716340000000-256.ndjson" {
		t.Fatalf("LastKey = %q", c2.LastKey)
	}
	if c2.LastSeenAt == 0 {
		t.Fatalf("LastSeenAt should be set")
	}
}

// --- audit-object application ------------------------------------
//
// These replace the per-topic activity tests. The behaviour they pin is the
// behaviour that stopped the 110 GiB write-ahead log: one transaction per
// object, counters kept per ORG rather than per (org, topic), and a replay
// that cannot double-count.

// applyObject is the common call shape, so a signature change surfaces once.
func applyObject(t *testing.T, r *registry.Registry, key string, updates ...registry.AuditProgressUpdate) error {
	t.Helper()
	return r.ApplyAuditObject(context.Background(), "site-A", key, updates)
}

func TestApplyAuditObjectKeepsOneRowPerOrg(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	for _, p := range []string{"aardwolf", "otter"} {
		if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: p, APIKeyHash: "h"}); err != nil {
			t.Fatalf("create %s: %v", p, err)
		}
	}

	// No activity yet: zero counts, no last topic. Zero is a real answer here
	// — the org exists and has published nothing.
	rows, _ := r.ListProgress(ctx, true)
	for _, row := range rows {
		if row.PublishCount != 0 || row.LastTopic != nil {
			t.Fatalf("%s: expected empty activity, got count=%d last=%v",
				row.Prefix, row.PublishCount, row.LastTopic)
		}
	}

	t1 := int64(1716200000)
	// One object carrying many publishes for one org across several distinct
	// key expressions. The old shape wrote a row per key; this must write one
	// row per org no matter how many keys were involved.
	if err := applyObject(t, r, "site-A/1.ndjson",
		registry.AuditProgressUpdate{
			Prefix: "aardwolf", HelloFirstAt: t1, LastPublishAt: t1 + 9,
			PublishDelta: 2500, LastTopic: "aardwolf/telemetry/2500",
		},
		registry.AuditProgressUpdate{
			Prefix: "otter", OtherFirstAt: t1, LastPublishAt: t1 + 3,
			PublishDelta: 4, LastTopic: "otter/edge",
		},
	); err != nil {
		t.Fatalf("apply: %v", err)
	}

	by := map[string]registry.VendorProgressRow{}
	rows, _ = r.ListProgress(ctx, true)
	for _, row := range rows {
		by[row.Prefix] = row
	}
	aw := by["aardwolf"]
	if aw.PublishCount != 2500 {
		t.Errorf("aardwolf PublishCount = %d, want 2500", aw.PublishCount)
	}
	if aw.LastTopic == nil || *aw.LastTopic != "aardwolf/telemetry/2500" {
		t.Errorf("aardwolf LastTopic = %v, want aardwolf/telemetry/2500", aw.LastTopic)
	}
	if aw.L3At == nil || *aw.L3At != t1 {
		t.Errorf("aardwolf L3At = %v, want %d", aw.L3At, t1)
	}
	if by["otter"].PublishCount != 4 {
		t.Errorf("otter PublishCount = %d, want 4", by["otter"].PublishCount)
	}

	// The property the whole change exists for: 2,504 publishes across many
	// distinct key expressions left exactly two rows of activity state.
	var progressRows int
	if err := r.DB().Get(&progressRows, `SELECT COUNT(*) FROM vendor_progress`); err != nil {
		t.Fatalf("count vendor_progress: %v", err)
	}
	if progressRows != 2 {
		t.Fatalf("vendor_progress holds %d rows for 2 orgs; activity state must not grow with traffic", progressRows)
	}
}

func TestApplyAuditObjectNeverWritesThePerTopicTable(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create: %v", err)
	}
	// The table still exists — it is dropped in a later, separately soaked
	// change so that a rollback of this image finds its database unchanged.
	// Nothing may write to it in the meantime, and this is the assertion that
	// keeps it that way.
	for i := 0; i < 5; i++ {
		if err := applyObject(t, r, fmt.Sprintf("site-A/%d.ndjson", i),
			registry.AuditProgressUpdate{
				Prefix: "aardwolf", LastPublishAt: int64(1716200000 + i),
				PublishDelta: 100, LastTopic: fmt.Sprintf("aardwolf/k/%d", i),
			}); err != nil {
			t.Fatalf("apply %d: %v", i, err)
		}
	}
	var n int
	if err := r.DB().Get(&n, `SELECT COUNT(*) FROM vendor_topic_activity`); err != nil {
		t.Fatalf("count vendor_topic_activity: %v", err)
	}
	if n != 0 {
		t.Fatalf("vendor_topic_activity gained %d rows; the per-topic write is back", n)
	}
}

func TestApplyAuditObjectLastTopicFollowsLatestPublish(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create: %v", err)
	}
	newer := int64(1716200500)
	if err := applyObject(t, r, "site-A/1.ndjson", registry.AuditProgressUpdate{
		Prefix: "aardwolf", LastPublishAt: newer, PublishDelta: 1, LastTopic: "aardwolf/newest",
	}); err != nil {
		t.Fatalf("apply first: %v", err)
	}
	// An object replayed out of order, carrying an OLDER publish. Neither the
	// timestamp nor the name it goes with may rewind.
	if err := applyObject(t, r, "site-A/2.ndjson", registry.AuditProgressUpdate{
		Prefix: "aardwolf", LastPublishAt: newer - 400, PublishDelta: 1, LastTopic: "aardwolf/older",
	}); err != nil {
		t.Fatalf("apply older: %v", err)
	}
	rows, _ := r.ListProgress(ctx, true)
	if *rows[0].LastPublishAt != newer {
		t.Errorf("LastPublishAt rewound to %d, want %d", *rows[0].LastPublishAt, newer)
	}
	if rows[0].PublishCount != 2 {
		t.Errorf("PublishCount = %d, want 2 (an older object still counts)", rows[0].PublishCount)
	}

	// An object with publishes but no topic name must not blank the name the
	// surface is showing.
	if err := applyObject(t, r, "site-A/3.ndjson", registry.AuditProgressUpdate{
		Prefix: "aardwolf", LastPublishAt: newer + 10, PublishDelta: 1,
	}); err != nil {
		t.Fatalf("apply nameless: %v", err)
	}
	rows, _ = r.ListProgress(ctx, true)
	if rows[0].LastTopic == nil || *rows[0].LastTopic != "aardwolf/newest" {
		t.Errorf("LastTopic = %v, want it preserved as aardwolf/newest", rows[0].LastTopic)
	}
}

func TestApplyAuditObjectRetryAppliesExactlyOnce(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "h"}); err != nil {
		t.Fatalf("create: %v", err)
	}
	key := "site-A/1.ndjson"
	upd := registry.AuditProgressUpdate{
		Prefix: "aardwolf", LastPublishAt: 1716200000, PublishDelta: 7, LastTopic: "aardwolf/t",
	}

	// A cancelled apply must leave nothing behind — counts AND cursor roll
	// back together, which is what lets the retry below be safe.
	cancelled, cancel := context.WithCancel(ctx)
	cancel()
	if err := r.ApplyAuditObject(cancelled, "site-A", key, []registry.AuditProgressUpdate{upd}); err == nil {
		t.Fatal("cancelled apply returned nil; the failure path is not exercised")
	}
	rows, _ := r.ListProgress(ctx, true)
	if rows[0].PublishCount != 0 {
		t.Fatalf("cancelled apply left PublishCount = %d, want 0", rows[0].PublishCount)
	}
	if c, _ := r.GetAuditCursor(ctx, "site-A"); c.LastKey == key {
		t.Fatal("cancelled apply advanced the cursor; a retry would skip the object")
	}

	// The retry applies it once.
	if err := applyObject(t, r, key, upd); err != nil {
		t.Fatalf("retry: %v", err)
	}
	rows, _ = r.ListProgress(ctx, true)
	if rows[0].PublishCount != 7 {
		t.Fatalf("after retry PublishCount = %d, want 7", rows[0].PublishCount)
	}
	// Re-applying the SAME object is what the cursor prevents in the tailer;
	// at this layer, applying twice legitimately counts twice, and the tailer
	// test (TestPullOnceRepullDoesNotDoubleCount) covers the guard.
	if c, _ := r.GetAuditCursor(ctx, "site-A"); c.LastKey != key {
		t.Fatalf("cursor = %q, want %q", c.LastKey, key)
	}
}
