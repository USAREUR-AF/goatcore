package api

import (
	"crypto/rand"
	"encoding/hex"
	"testing"
)

// GHSA-fgm6 review finding 1. The legacy-row prefixes are public, so an
// anonymous caller can present any number of wrong tokens for one. Those
// failures must not lock the vendor's own correct key out: it still
// authenticates, and the row is backfilled.
func TestResolvePrincipal_AnonymousFailuresDoNotLockOutLegacyKey(t *testing.T) {
	s, reg := newAuthTestServer(t)
	key := legacyVendor(t, reg, "lockout-victim")

	for i := range 20 {
		junk := make([]byte, 32)
		if _, err := rand.Read(junk); err != nil {
			t.Fatal(err)
		}
		p, err := s.resolvePrincipal(t.Context(), "vp_lockout-victim_"+hex.EncodeToString(junk))
		if err != nil || p != nil {
			t.Fatalf("junk token %d: principal=%+v err=%v", i, p, err)
		}
	}

	p, err := s.resolvePrincipal(t.Context(), key)
	if err != nil {
		t.Fatalf("correct key: %v", err)
	}
	if p == nil || p.Vendor != "lockout-victim" {
		t.Fatalf("correct key after anonymous failures resolved to %+v; want vendor lockout-victim", p)
	}
	v, err := reg.GetVendor(t.Context(), "lockout-victim")
	if err != nil {
		t.Fatal(err)
	}
	if v.APIKeySHA256 != apiKeyDigest(key) {
		t.Fatalf("row not backfilled after the correct key: digest=%q", v.APIKeySHA256)
	}
}
