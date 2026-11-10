package config

import "testing"

// --ca-root-cert / PORTAL_CA_ROOT_CERT are neutral aliases of the legacy
// --nova-root-cert / PORTAL_NOVA_ROOT_CERT. The legacy name keeps
// working and wins when both are set.
func TestLoadFromFlags_CARootCertAlias(t *testing.T) {
	cases := []struct {
		name string
		env  map[string]string
		args []string
		want string
	}{
		{"alias flag alone", nil, []string{"--ca-root-cert=/etc/ca/new.pem"}, "/etc/ca/new.pem"},
		{"alias env alone", map[string]string{"PORTAL_CA_ROOT_CERT": "/etc/ca/env.pem"}, nil, "/etc/ca/env.pem"},
		{"legacy flag alone", nil, []string{"--nova-root-cert=/etc/ca/old.pem"}, "/etc/ca/old.pem"},
		{"both flags: legacy wins", nil, []string{"--nova-root-cert=/etc/ca/old.pem", "--ca-root-cert=/etc/ca/new.pem"}, "/etc/ca/old.pem"},
		{"both env: legacy wins", map[string]string{"PORTAL_NOVA_ROOT_CERT": "/etc/ca/old.pem", "PORTAL_CA_ROOT_CERT": "/etc/ca/new.pem"}, nil, "/etc/ca/old.pem"},
		{"legacy env + alias flag: legacy wins", map[string]string{"PORTAL_NOVA_ROOT_CERT": "/etc/ca/old.pem"}, []string{"--ca-root-cert=/etc/ca/new.pem"}, "/etc/ca/old.pem"},
		{"neither", nil, nil, ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			t.Setenv("PORTAL_NOVA_ROOT_CERT", "")
			t.Setenv("PORTAL_CA_ROOT_CERT", "")
			for k, v := range c.env {
				t.Setenv(k, v)
			}
			cfg, err := LoadFromFlags(c.args)
			if err != nil {
				t.Fatalf("LoadFromFlags: %v", err)
			}
			if cfg.CARootCert != c.want {
				t.Errorf("CARootCert = %q; want %q", cfg.CARootCert, c.want)
			}
		})
	}
}

// PORTAL_NETBIRD_API_TOKEN is the neutral alias of NOVA_NB_API_TOKEN; the legacy
// name wins when both are set.
func TestLoadFromFlags_NetbirdAPITokenAlias(t *testing.T) {
	cases := []struct {
		name          string
		legacy, alias string
		want          string
	}{
		{"alias alone", "", "nb-new", "nb-new"},
		{"legacy alone", "nb-old", "", "nb-old"},
		{"both: legacy wins", "nb-old", "nb-new", "nb-old"},
		{"neither", "", "", ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			t.Setenv("NOVA_NB_API_TOKEN", c.legacy)
			t.Setenv("PORTAL_NETBIRD_API_TOKEN", c.alias)
			cfg, err := LoadFromFlags(nil)
			if err != nil {
				t.Fatalf("LoadFromFlags: %v", err)
			}
			if cfg.NetbirdAPIToken != c.want {
				t.Errorf("NetbirdAPIToken = %q; want %q", cfg.NetbirdAPIToken, c.want)
			}
		})
	}
}

// --fabric-name / PORTAL_FABRIC_NAME set the fabric_name /api/fabric/config
// reports; unset keeps the value every goatnet reports today.
func TestLoadFromFlags_FabricName(t *testing.T) {
	t.Setenv("PORTAL_FABRIC_NAME", "")
	cfg, err := LoadFromFlags(nil)
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	if cfg.FabricName != "nova-trial" {
		t.Errorf("default FabricName = %q; want the unchanged built-in value", cfg.FabricName)
	}
	cfg, err = LoadFromFlags([]string{"--fabric-name=galaxy-fabric"})
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	if cfg.FabricName != "galaxy-fabric" {
		t.Errorf("FabricName = %q; want galaxy-fabric", cfg.FabricName)
	}
	t.Setenv("PORTAL_FABRIC_NAME", "env-fabric")
	cfg, err = LoadFromFlags(nil)
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	if cfg.FabricName != "env-fabric" {
		t.Errorf("FabricName = %q; want env-fabric", cfg.FabricName)
	}
}
