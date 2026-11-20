package certmint

import (
	"testing"
	"time"
)

// IsStageDirName matches exactly the name Mint gives its stage directory.
func TestIsStageDirName(t *testing.T) {
	minted := stageDirName("fra-land-bde", time.Now().UTC().Format(stampLayout))
	if !IsStageDirName(minted) {
		t.Fatalf("Mint's own stage name %q not matched", minted)
	}
	for name, want := range map[string]bool{
		".stage-acme-20250101T000000Z":        true,
		".stage-a-b-c-20250101T000000Z":       true,
		".stage--20250101T000000Z":            false, // empty prefix
		".stage-acme-20250101T000000":         false, // stamp missing Z
		".stage-acme-2025010100000Z":          false,
		".stage-acme":                         false,
		".stage-":                             false,
		"stage-acme-20250101T000000Z":         false,
		".stage-acme-20250101T000000Z.tar.gz": false,
		".stagex-acme-20250101T000000Z":       false,
		".stage-acme-20251301T000000Z":        false, // month 13
		"goat-acme-20250101T000000Z.tar.gz":   false,
	} {
		if got := IsStageDirName(name); got != want {
			t.Errorf("IsStageDirName(%q) = %v, want %v", name, got, want)
		}
	}
}
