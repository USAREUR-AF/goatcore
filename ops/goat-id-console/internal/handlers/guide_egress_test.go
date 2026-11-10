package handlers

import (
	"context"
	"errors"
	"sync/atomic"
	"testing"
	"time"
)

func TestEgressResolver_keepsPublicAddressesAndCaches(t *testing.T) {
	var calls atomic.Int32
	r := &egressResolver{ttl: time.Minute, timeout: time.Second,
		lookup: func(_ context.Context, host string) ([]string, error) {
			calls.Add(1)
			switch host {
			case "netbird.example.net":
				return []string{"198.51.100.10", "10.0.0.1"}, nil
			case "relay-a.example.net":
				return []string{"100.127.1.23"}, nil // mesh-only answer
			}
			return nil, errors.New("nxdomain")
		}}
	hosts := r.hosts(context.Background(), "https://netbird.example.net",
		[]string{"rels://relay-a.example.net:33073", "rels://gone.example.net:33073"}, nil)
	if len(hosts) != 3 {
		t.Fatalf("want 3 hosts, got %+v", hosts)
	}
	if got := hosts[0].IPs; len(got) != 1 || got[0] != "198.51.100.10" || hosts[0].Unresolved {
		t.Errorf("management host: %+v", hosts[0])
	}
	if !hosts[1].Unresolved || !hosts[2].Unresolved {
		t.Errorf("a mesh-only answer and a failed lookup must both read as unresolved: %+v", hosts[1:])
	}
	before := calls.Load()
	r.hosts(context.Background(), "https://netbird.example.net", []string{"rels://relay-a.example.net:33073", "rels://gone.example.net:33073"}, nil)
	// Two successful answers are cached; the failed lookup is retried.
	if n := calls.Load() - before; n != 1 {
		t.Errorf("second page load made %d lookups, want 1 (only the failed name)", n)
	}
}
