package probe

import (
	"os"
	"path/filepath"
	"testing"
)

func TestDefaultConfigHasNoTargets(t *testing.T) {
	c := DefaultConfig()
	if c.Domain != "" || c.HostMeshIP != "" || len(c.Relays) != 0 || len(c.RelayDNS) != 0 || len(c.ExpectedIPs) != 0 || c.Mode != "quick" {
		t.Fatalf("built-in config must carry no goatnet: %+v", c)
	}
}

func TestLoadConfigOverlaysFile(t *testing.T) {
	p := filepath.Join(t.TempDir(), "goatnet.yaml")
	os.WriteFile(p, []byte(`domain: netbird.galaxy.example
host_mesh_ip: 192.0.2.10
relays:
  - { name: rla, ip: 198.51.100.7, region: region-1, instance_id: i-0example }
relay_dns: [relay.galaxy.example]
expected_ips: [198.51.100.7]
`), 0o644)
	c, err := LoadConfig(p)
	if err != nil {
		t.Fatal(err)
	}
	if c.Domain != "netbird.galaxy.example" || c.HostMeshIP != "192.0.2.10" || len(c.Relays) != 1 || c.Relays[0].Name != "rla" ||
		c.RelayDNS[0] != "relay.galaxy.example" || c.ExpectedIPs[0] != "198.51.100.7" || c.Mode != "quick" {
		t.Fatalf("loaded %+v", c)
	}
	if c, err := LoadConfig(""); err != nil || c.Domain != "" {
		t.Fatalf("empty path: %+v %v", c, err)
	}
}
