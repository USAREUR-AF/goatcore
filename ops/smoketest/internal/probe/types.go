// Package probe defines the core types and interfaces for the smoketest probe system.
package probe

import (
	"context"
	"fmt"
	"os"
	"time"

	"gopkg.in/yaml.v3"
)

// Status represents the outcome of a single check.
type Status string

const (
	Pass Status = "PASS"
	Fail Status = "FAIL"
	Warn Status = "WARN"
	Skip Status = "SKIP"
)

// Category groups related checks.
type Category string

const (
	CatDNS     Category = "dns"
	CatRelay   Category = "relay"
	CatSTUN    Category = "stun"
	CatTLS     Category = "tls"
	CatMesh    Category = "mesh"
	CatService Category = "service"
	CatLatency Category = "latency"
	CatLoad    Category = "load"
)

// Result captures the outcome of a single check.
type Result struct {
	Category  Category       `json:"category"`
	Name      string         `json:"name"`
	Status    Status         `json:"status"`
	Detail    string         `json:"detail"`
	LatencyMs float64        `json:"latency_ms,omitempty"`
	Timestamp time.Time      `json:"timestamp"`
	Metadata  map[string]any `json:"metadata,omitempty"`
}

// RunResult is the aggregate output of a full probe run.
type RunResult struct {
	RunID     string    `json:"run_id"`
	Timestamp time.Time `json:"timestamp"`
	Mode      string    `json:"mode"`
	DurationS float64   `json:"duration_s"`
	Pass      int       `json:"pass"`
	Fail      int       `json:"fail"`
	Warn      int       `json:"warn"`
	Checks    []Result  `json:"checks"`
}

// Probe is the interface every probe module implements.
type Probe interface {
	// Category returns the probe's category.
	Category() Category
	// Run executes all checks in this probe and returns results.
	Run(ctx context.Context, cfg *Config) []Result
}

// RelayInfo describes a single VPS relay.
type RelayInfo struct {
	Name       string `yaml:"name" json:"name"`
	IP         string `yaml:"ip" json:"ip"`
	Region     string `yaml:"region" json:"region"`
	InstanceID string `yaml:"instance_id" json:"instance_id"`
}

// Config holds all configuration needed by probes.
type Config struct {
	Domain       string      `yaml:"domain" json:"domain"`
	Relays       []RelayInfo `yaml:"relays" json:"relays"`
	HostMeshIP   string      `yaml:"host_mesh_ip" json:"host_mesh_ip"`
	RelayDNS     []string    `yaml:"relay_dns" json:"relay_dns"`
	ExpectedIPs  []string    `yaml:"expected_ips" json:"expected_ips"`
	NetbirdToken string      `yaml:"-" json:"-"`
	Mode         string      `yaml:"-" json:"-"` // "quick" or "full"
	Categories   []Category  `yaml:"-" json:"-"` // filter to these categories (empty = all)
}

// DefaultConfig returns the built-in configuration: quick mode and no targets.
// The goatnet's domain, relays and mesh host come from a config file (LoadConfig).
func DefaultConfig() *Config {
	return &Config{Mode: "quick"}
}

// LoadConfig returns DefaultConfig overlaid with the YAML file at path
// (keys: domain, relays, host_mesh_ip, relay_dns, expected_ips). An empty path
// returns DefaultConfig unchanged.
func LoadConfig(path string) (*Config, error) {
	cfg := DefaultConfig()
	if path == "" {
		return cfg, nil
	}
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	if err := yaml.Unmarshal(b, cfg); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return cfg, nil
}
