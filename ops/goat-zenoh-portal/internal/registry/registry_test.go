package registry_test

import (
	"context"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// helper — fresh DB per test, auto-cleanup via t.TempDir.
func open(t *testing.T) *registry.Registry {
	t.Helper()
	r, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { _ = r.Close() })
	return r
}

func TestVendorPrefixValidation(t *testing.T) {
	tests := []struct {
		prefix string
		ok     bool
	}{
		{"acme", true},
		{"goat", true},
		{"a", true},
		{"community-test", true},
		{"abc123", true},
		// rejected
		{"", false},
		{"-leading", false},
		{"trailing-", false},
		{"UPPER", false},
		{"under_score", false},
		{"with space", false},
		{"ops", false},    // reserved (canonical operator namespace, ADR 1035)
		{"system", false}, // reserved (deprecated operator namespace, transitional)
		{"attest", false}, // reserved
		{strings.Repeat("a", 33), false},
	}
	for _, tc := range tests {
		err := registry.ValidVendorPrefix(tc.prefix)
		if tc.ok && err != nil {
			t.Errorf("ValidVendorPrefix(%q) unexpectedly errored: %v", tc.prefix, err)
		}
		if !tc.ok && err == nil {
			t.Errorf("ValidVendorPrefix(%q) expected error, got nil", tc.prefix)
		}
	}
}

func TestCreateAndGetVendor(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	v := &registry.Vendor{Prefix: "acme", APIKeyHash: "h", DisplayName: "Acme"}
	if err := r.CreateVendor(ctx, v); err != nil {
		t.Fatalf("create: %v", err)
	}
	got, err := r.GetVendor(ctx, "acme")
	if err != nil {
		t.Fatalf("get: %v", err)
	}
	if got.Prefix != "acme" || got.DisplayName != "Acme" {
		t.Errorf("got %+v, want prefix=acme name=Acme", got)
	}
	if got.ApprovedAt != nil {
		t.Errorf("expected approved_at nil at creation, got %v", *got.ApprovedAt)
	}
}

func TestCreateVendorDuplicate(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	v := &registry.Vendor{Prefix: "acme", APIKeyHash: "h"}
	if err := r.CreateVendor(ctx, v); err != nil {
		t.Fatalf("first create: %v", err)
	}
	err := r.CreateVendor(ctx, &registry.Vendor{Prefix: "acme", APIKeyHash: "h2"})
	if err != registry.ErrVendorExists {
		t.Errorf("expected ErrVendorExists, got %v", err)
	}
}

func TestApproveVendor(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	_ = r.CreateVendor(ctx, &registry.Vendor{Prefix: "acme", APIKeyHash: "h"})
	if err := r.ApproveVendor(ctx, "acme", "operator"); err != nil {
		t.Fatalf("approve: %v", err)
	}
	v, _ := r.GetVendor(ctx, "acme")
	if v.ApprovedAt == nil {
		t.Errorf("expected approved_at set")
	}
	approver := ""
	if v.Approver != nil {
		approver = *v.Approver
	}
	if approver != "operator" {
		t.Errorf("expected approver=operator, got %q", approver)
	}
	// Idempotency: re-approve doesn't change approved_at.
	prevAt := *v.ApprovedAt
	if err := r.ApproveVendor(ctx, "acme", "operator2"); err != nil {
		t.Fatalf("re-approve: %v", err)
	}
	v2, _ := r.GetVendor(ctx, "acme")
	if *v2.ApprovedAt != prevAt {
		t.Errorf("approved_at changed on re-approve: %d → %d", prevAt, *v2.ApprovedAt)
	}
}

func TestApprovedVendorPrefixes(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	_ = r.CreateVendor(ctx, &registry.Vendor{Prefix: "acme", APIKeyHash: "h"})
	_ = r.CreateVendor(ctx, &registry.Vendor{Prefix: "community", APIKeyHash: "h"})
	_ = r.CreateVendor(ctx, &registry.Vendor{Prefix: "goat", APIKeyHash: "h"})
	_ = r.ApproveVendor(ctx, "acme", "operator")
	_ = r.ApproveVendor(ctx, "goat", "operator")
	ps, err := r.ApprovedVendorPrefixes(ctx)
	if err != nil {
		t.Fatalf("approved: %v", err)
	}
	want := []string{"acme", "goat"}
	if len(ps) != len(want) {
		t.Fatalf("got %v, want %v", ps, want)
	}
	for i, w := range want {
		if ps[i] != w {
			t.Errorf("idx %d: got %q want %q", i, ps[i], w)
		}
	}
}

func TestTopicValidation(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	_ = r.CreateVendor(ctx, &registry.Vendor{Prefix: "acme", APIKeyHash: "h"})

	goodCases := []struct {
		name string
		t    *registry.Topic
	}{
		{"json", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/lattice/entity/track/v1", Format: "json"}},
		{"opaque xml", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/tak/cot/v1", Format: "xml"}},
		{"schema-bound xml", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/tak/event/v1", Format: "xml:event"}},
		{"protobuf with msg", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/lattice/pa/v1", Format: "protobuf:PositionAttitude"}},
	}
	for _, tc := range goodCases {
		if _, err := r.CreateTopic(ctx, tc.t); err != nil {
			t.Errorf("good topic %q create failed: %v", tc.name, err)
		}
	}

	badCases := []struct {
		name string
		t    *registry.Topic
	}{
		{"wrong vendor prefix segment", &registry.Topic{VendorPrefix: "acme", KeyPattern: "example-co/foo/v1", Format: "json"}},
		{"too few segments", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/foo", Format: "json"}},
		// A complete mid-pattern `*` segment is now registrable (wildcard
		// topic registration); embedded/multi-segment wildcards stay out.
		{"embedded wildcard in segment", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/lat*ice/x/v1", Format: "json"}},
		{"multi-segment wildcard", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/lattice/**", Format: "json"}},
		{"first-segment wildcard", &registry.Topic{VendorPrefix: "*", KeyPattern: "*/lattice/x/v1", Format: "json"}},
		{"empty segment", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme//foo/v1", Format: "json"}},
		{"unknown format", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/x/y/v1", Format: "yaml"}},
		{"protobuf without msg", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/x/y/v1", Format: "protobuf:"}},
		{"xml without root element", &registry.Topic{VendorPrefix: "acme", KeyPattern: "acme/x/y/v1", Format: "xml:"}},
	}
	for _, tc := range badCases {
		if _, err := r.CreateTopic(ctx, tc.t); err == nil {
			t.Errorf("expected error for %s, got nil", tc.name)
		}
	}
}

func TestTopicCRUD(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	_ = r.CreateVendor(ctx, &registry.Vendor{Prefix: "community", APIKeyHash: "h"})
	id, err := r.CreateTopic(ctx, &registry.Topic{
		VendorPrefix: "community",
		KeyPattern:   "community/jane-test/foo/v1",
		Format:       "json",
		Description:  "test",
	})
	if err != nil {
		t.Fatalf("create: %v", err)
	}
	// list
	ts, err := r.ListVendorTopics(ctx, "community")
	if err != nil || len(ts) != 1 {
		t.Fatalf("list: %v (count %d)", err, len(ts))
	}
	// update
	tOpt, _ := r.GetTopic(ctx, id)
	tOpt.Format = "cbor"
	tOpt.Description = "updated"
	if err := r.UpdateTopic(ctx, tOpt); err != nil {
		t.Fatalf("update: %v", err)
	}
	got, _ := r.GetTopic(ctx, id)
	if got.Format != "cbor" {
		t.Errorf("expected format=cbor, got %q", got.Format)
	}
	if got.Description != "updated" {
		t.Errorf("expected description=updated, got %q", got.Description)
	}
	// retire
	if err := r.RetireTopic(ctx, id); err != nil {
		t.Fatalf("retire: %v", err)
	}
	ts, _ = r.ListVendorTopics(ctx, "community")
	if len(ts) != 0 {
		t.Errorf("expected empty after retire, got %d", len(ts))
	}
	// After retire, re-create with the same key_pattern is REFUSED — the
	// UNIQUE(vendor_prefix, key_pattern) slot stays occupied. This is the
	// audit-preserving posture; callers wanting name reuse take the
	// DeleteTopic path below.
	if _, err := r.CreateTopic(ctx, &registry.Topic{
		VendorPrefix: "community",
		KeyPattern:   "community/jane-test/foo/v1",
		Format:       "json",
	}); err != registry.ErrTopicExists {
		t.Errorf("re-create after retire: expected ErrTopicExists, got %v", err)
	}
	// DeleteTopic on the retired row frees the slot.
	if err := r.DeleteTopic(ctx, id); err != nil {
		t.Fatalf("delete: %v", err)
	}
	if _, err := r.GetTopic(ctx, id); err != registry.ErrNotFound {
		t.Errorf("after delete: expected ErrNotFound, got %v", err)
	}
	// Same key_pattern is now creatable again.
	newID, err := r.CreateTopic(ctx, &registry.Topic{
		VendorPrefix: "community",
		KeyPattern:   "community/jane-test/foo/v1",
		Format:       "json",
	})
	if err != nil {
		t.Fatalf("re-create after delete: %v", err)
	}
	if newID == id {
		t.Errorf("expected new topic id, got same id %d", newID)
	}
	// DeleteTopic on a vanished row returns ErrNotFound.
	if err := r.DeleteTopic(ctx, 99999); err != registry.ErrNotFound {
		t.Errorf("delete missing: expected ErrNotFound, got %v", err)
	}
}

func TestBundleTokenMultiUse(t *testing.T) {
	// Bundle tokens are multi-use by design — siblings at the same vendor
	// share one cert+api_key (per docs/design/operator-onboard-utilities.md
	// §83E), so the URL must keep returning the tarball across downloads.
	// Token rotation is an explicit operator action (sqlite UPDATE), not an
	// implicit side-effect of any single download.
	r := open(t)
	ctx := context.Background()
	_ = r.CreateVendor(ctx, &registry.Vendor{Prefix: "community", APIKeyHash: "h"})
	_ = r.MarkBundle(ctx, "community", "tok123", "/tmp/community.tar.gz")
	for i := 0; i < 3; i++ {
		path, err := r.ConsumeBundleToken(ctx, "community", "tok123")
		if err != nil {
			t.Fatalf("consume #%d: %v", i+1, err)
		}
		if path != "/tmp/community.tar.gz" {
			t.Errorf("consume #%d: got %q", i+1, path)
		}
	}
	// Wrong token still fails.
	if _, err := r.ConsumeBundleToken(ctx, "community", "wrong"); err != registry.ErrNotFound {
		t.Errorf("expected ErrNotFound on bad token, got %v", err)
	}
}

func TestAuditLog(t *testing.T) {
	r := open(t)
	ctx := context.Background()
	if err := r.LogAuditEvent(ctx, "operator", "vendor-register", "acme", `{"x":1}`); err != nil {
		t.Fatalf("log: %v", err)
	}
	// Empty details should round-trip as `{}`.
	if err := r.LogAuditEvent(ctx, "acme", "topic-create", "acme/x/v1", ""); err != nil {
		t.Fatalf("log empty: %v", err)
	}
	var count int
	if err := r.DB().Get(&count, "SELECT COUNT(*) FROM audit_events"); err != nil {
		t.Fatalf("query: %v", err)
	}
	if count != 2 {
		t.Errorf("expected 2 audit rows, got %d", count)
	}
}
