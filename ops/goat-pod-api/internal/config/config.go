// Package config reads the goat-pod-api configuration file. Every deployment value comes from
// this file (written by the pod's first-boot from its profile); nothing about a deployment is
// compiled in.
package config

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
)

// Router names the router files and container.
type Router struct {
	// BaseConfig is the router configuration first-boot rendered from the pod template, with
	// the carriage markers still in place.
	BaseConfig string `json:"base_config"`
	// Config is the live configuration the router container mounts.
	Config               string `json:"config"`
	LastGood             string `json:"last_good"`
	AppliedSHA           string `json:"applied_sha_file"`
	Container            string `json:"container"`
	HealthTimeoutSeconds int    `json:"health_timeout_seconds"`
}

// HealthPublish publishes carriage health through a command (by default the goat CLI).
type HealthPublish struct {
	Enabled bool              `json:"enabled"`
	Command []string          `json:"command"`
	Env     map[string]string `json:"env"`
}

// Config is the whole file.
type Config struct {
	PodSlot string `json:"pod_slot"`
	// StateDir holds the last good generation, the archive of central rules and the ledger.
	StateDir string `json:"state_dir"`
	// PortalURL is the portal base URL; empty runs the pod on local rules and its last
	// generation only.
	PortalURL string `json:"portal_url"`
	// PortalCA verifies the portal's TLS certificate; empty uses the system roots.
	PortalCA string `json:"portal_ca"`
	// PortalTokenFile holds the pod's portal API key (the goat profile's portal.token), sent as
	// a bearer token; the value is never in this file.
	PortalTokenFile string `json:"portal_token_file"`
	// PortalSignerCert verifies generation signatures.
	PortalSignerCert string `json:"portal_signer_cert"`
	PodCert          string `json:"pod_cert"`
	PodKey           string `json:"pod_key"`
	// CARoots verifies client certificates on the local API.
	CARoots string   `json:"ca_roots"`
	Listen  []string `json:"listen"`
	// FabricRouterCN is the upstream router's certificate CN; it is never admitted as a carried
	// principal. Read from the router configuration when empty.
	FabricRouterCN      string        `json:"fabric_router_cn"`
	PullIntervalSeconds int           `json:"pull_interval_seconds"`
	CutoffAfterSeconds  int           `json:"cutoff_after_seconds"`
	MaxRuleDays         int           `json:"max_rule_days"`
	ReactivationDays    int           `json:"reactivation_days"`
	GatepostPage        string        `json:"gatepost_page"`
	Router              Router        `json:"router"`
	HealthPublish       HealthPublish `json:"health_publish"`
}

// Load reads and checks path.
func Load(path string) (*Config, error) {
	raw, err := os.ReadFile(path) // #nosec G304 -- operator-supplied path
	if err != nil {
		return nil, fmt.Errorf("read configuration: %w", err)
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	var c Config
	if err := dec.Decode(&c); err != nil {
		return nil, fmt.Errorf("configuration %s: %w", path, err)
	}
	c.defaults()
	if err := c.check(); err != nil {
		return nil, fmt.Errorf("configuration %s: %w", path, err)
	}
	return &c, nil
}

func (c *Config) defaults() {
	if c.PullIntervalSeconds == 0 {
		c.PullIntervalSeconds = 60
	}
	if c.CutoffAfterSeconds == 0 {
		c.CutoffAfterSeconds = 5 * c.PullIntervalSeconds
	}
	if c.MaxRuleDays == 0 {
		c.MaxRuleDays = 90
	}
	if c.ReactivationDays == 0 {
		c.ReactivationDays = 183
	}
	if c.Router.HealthTimeoutSeconds == 0 {
		c.Router.HealthTimeoutSeconds = 120
	}
	if len(c.HealthPublish.Command) == 0 {
		c.HealthPublish.Command = []string{"goat", "pub"}
	}
}

func (c *Config) check() error {
	var missing []string
	need := map[string]string{
		"pod_slot": c.PodSlot, "state_dir": c.StateDir, "portal_signer_cert": c.PortalSignerCert,
		"pod_cert": c.PodCert, "pod_key": c.PodKey, "ca_roots": c.CARoots,
		"router.base_config": c.Router.BaseConfig, "router.config": c.Router.Config,
		"router.last_good": c.Router.LastGood, "router.applied_sha_file": c.Router.AppliedSHA,
		"router.container": c.Router.Container,
	}
	for k, v := range need {
		if strings.TrimSpace(v) == "" {
			missing = append(missing, k)
		}
	}
	if len(missing) > 0 {
		sort.Strings(missing)
		return fmt.Errorf("missing settings: %s", strings.Join(missing, ", "))
	}
	if !generation.ValidSlot(c.PodSlot) {
		return fmt.Errorf("pod_slot %q is not a slot id", c.PodSlot)
	}
	if c.PortalURL != "" && !strings.HasPrefix(c.PortalURL, "https://") {
		return errors.New("portal_url must be an https:// URL")
	}
	if c.PortalURL != "" && c.PortalTokenFile == "" {
		return errors.New("portal_url is set but portal_token_file is not")
	}
	if c.MaxRuleDays < 1 || c.ReactivationDays < 1 || c.PullIntervalSeconds < 5 {
		return errors.New("max_rule_days and reactivation_days must be 1 or more, pull_interval_seconds 5 or more")
	}
	return nil
}

// PullInterval is how often the pod pulls its generation.
func (c *Config) PullInterval() time.Duration {
	return time.Duration(c.PullIntervalSeconds) * time.Second
}

// CutoffAfter is how long without contact before the pod reports itself cut off.
func (c *Config) CutoffAfter() time.Duration {
	return time.Duration(c.CutoffAfterSeconds) * time.Second
}

// MaxRule is the longest end date a local change may set.
func (c *Config) MaxRule() time.Duration { return time.Duration(c.MaxRuleDays) * 24 * time.Hour }

// Reactivation is how long after its end an archived rule may be reactivated.
func (c *Config) Reactivation() time.Duration {
	return time.Duration(c.ReactivationDays) * 24 * time.Hour
}

// HealthKey is where carriage health is published, or "" when publishing is off.
func (c *Config) HealthKey() string {
	if !c.HealthPublish.Enabled {
		return ""
	}
	return "ops/pod/health/" + c.PodSlot
}
