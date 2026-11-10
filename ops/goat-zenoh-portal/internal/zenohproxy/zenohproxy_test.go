// SPDX-License-Identifier: Apache-2.0
package zenohproxy

import "testing"

func TestValidateStreamKeyBoundsCrossOwnerWildcards(t *testing.T) {
	// Owner-scoped expressions stream fine — bounded by one namespace owner.
	allowed := []string{
		"a2dc159217715948e66e3d77a327ccd7/video/frame",
		"a2dc159217715948e66e3d77a327ccd7/**",
		"ops/fabric/cardinality/galaxy-sandbox",
		"ops/fabric/health/*",
		"notional/foo/v1",
	}
	for _, k := range allowed {
		if err := ValidateStreamKey(k); err != nil {
			t.Errorf("ValidateStreamKey(%q) = %v, want nil", k, err)
		}
	}
	// Cross-owner wildcards = a whole-tree walk per 1 s poll per open tab —
	// the broad-crawl wedge factor. Rejected on the recurring path only.
	rejected := []string{
		"**",
		"*",
		"*/status",
		"**/presence/**",
		"$*/x",
	}
	for _, k := range rejected {
		if err := ValidateStreamKey(k); err == nil {
			t.Errorf("ValidateStreamKey(%q) = nil, want cross-owner rejection", k)
		}
	}
	// The base validateKey rules still apply.
	if err := ValidateStreamKey(""); err == nil {
		t.Error("empty key must be rejected")
	}
}
