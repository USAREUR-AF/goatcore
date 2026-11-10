package kratosadmin

import "testing"

// ADR 1052 Phase 4 — VendorShortTag reads traits.org first, falling back to the
// legacy traits.vendor for identities not yet backfilled.
func TestVendorShortTag_PrefersOrgFallsBackToVendor(t *testing.T) {
	org := func(tag string) map[string]interface{} {
		return map[string]interface{}{"org": map[string]interface{}{"short_tag": tag}}
	}
	vendor := func(tag string) map[string]interface{} {
		return map[string]interface{}{"vendor": map[string]interface{}{"short_tag": tag}}
	}
	cases := []struct {
		name   string
		traits map[string]interface{}
		want   string
	}{
		{"org only", org("example-org-sam"), "example-org-sam"},
		{"vendor only (legacy)", vendor("demo-vendor-pat"), "demo-vendor-pat"},
		{"both present prefers org", map[string]interface{}{
			"org":    map[string]interface{}{"short_tag": "new-tag"},
			"vendor": map[string]interface{}{"short_tag": "old-tag"},
		}, "new-tag"},
		{"empty org falls back to vendor", map[string]interface{}{
			"org":    map[string]interface{}{"short_tag": ""},
			"vendor": map[string]interface{}{"short_tag": "legacy"},
		}, "legacy"},
		{"neither", map[string]interface{}{}, ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := Identity{Traits: tc.traits}.VendorShortTag()
			if got != tc.want {
				t.Fatalf("VendorShortTag() = %q, want %q", got, tc.want)
			}
		})
	}
}

// F-572 / ADR 1188: authorization fields come from admin-managed
// metadata_public; once an identity is migrated (metadata_public.groups set) a
// self-added traits org or aff is never read.
func TestAuthz_MetadataPublicAndNoFallbackOnceMigrated(t *testing.T) {
	selfOrg := map[string]interface{}{"short_tag": "self"}
	selfAff := map[string]interface{}{"id": "self", "kind": "partner"}

	migrated := Identity{
		MetadataPublic: map[string]interface{}{"groups": []interface{}{}},
		Traits:         map[string]interface{}{"org": selfOrg, "aff": selfAff},
	}
	if got := migrated.VendorShortTag(); got != "" {
		t.Fatalf("migrated VendorShortTag() = %q, want empty", got)
	}
	if id, _, _ := migrated.Aff(); id != "" {
		t.Fatalf("migrated Aff() id = %q, want empty", id)
	}

	meta := Identity{
		MetadataPublic: map[string]interface{}{
			"groups": []interface{}{},
			"org":    map[string]interface{}{"short_tag": "acme"},
			"aff":    map[string]interface{}{"id": "acme", "kind": "vendor"},
		},
		Traits: map[string]interface{}{"org": selfOrg, "aff": selfAff},
	}
	if got := meta.VendorShortTag(); got != "acme" {
		t.Fatalf("VendorShortTag() = %q, want acme", got)
	}
	if id, _, _ := meta.Aff(); id != "acme" {
		t.Fatalf("Aff() id = %q, want acme", id)
	}

	legacy := Identity{Traits: map[string]interface{}{"org": selfOrg}}
	if got := legacy.VendorShortTag(); got != "self" {
		t.Fatalf("not-yet-backfilled VendorShortTag() = %q, want traits fallback", got)
	}
}
