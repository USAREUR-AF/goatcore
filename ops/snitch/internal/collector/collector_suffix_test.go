package collector

import (
	"reflect"
	"testing"

	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
)

func TestPeerSuffixesFromEnv(t *testing.T) {
	if got := PeerSuffixesFromEnv(""); !reflect.DeepEqual(got, compat.PeerFQDNSuffixes) {
		t.Fatalf("unset: got %v, want built-in %v", got, compat.PeerFQDNSuffixes)
	}
	got := PeerSuffixesFromEnv(" .netbird.goat.example , .local.netbird.goat.example,")
	want := []string{".netbird.goat.example", ".local.netbird.goat.example"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("set: got %v, want %v", got, want)
	}
}
