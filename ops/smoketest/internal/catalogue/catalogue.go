// Package catalogue parses the service catalogue YAML file.
package catalogue

import (
	"fmt"
	"os"

	"github.com/usareur-af/goatcore/ops/smoketest/internal/probe"
	"gopkg.in/yaml.v3"
)

// File represents the top-level services.yaml structure.
type File struct {
	Services []Service `yaml:"services" json:"services"`
}

// Service is a single entry in the catalogue.
type Service struct {
	Name        string  `yaml:"name" json:"name"`
	Description string  `yaml:"description" json:"description"`
	Category    string  `yaml:"category" json:"category"`
	Checks      []Check `yaml:"checks" json:"checks"`
}

// Check defines a single health check for a service.
type Check struct {
	CAFile  string `yaml:"ca_file,omitempty" json:"ca_file,omitempty"`
	Type    string `yaml:"type" json:"type"` // http, https, tcp, script
	URL     string `yaml:"url,omitempty" json:"url,omitempty"`
	Host    string `yaml:"host,omitempty" json:"host,omitempty"`
	Port    int    `yaml:"port,omitempty" json:"port,omitempty"`
	Resolve string `yaml:"resolve,omitempty" json:"resolve,omitempty"`
	Command string `yaml:"command,omitempty" json:"command,omitempty"`
	Timeout string `yaml:"timeout,omitempty" json:"timeout,omitempty"`
	Expect  Expect `yaml:"expect,omitempty" json:"expect,omitempty"`
}

// Expect defines expected values for a check.
type Expect struct {
	Status       int    `yaml:"status,omitempty" json:"status,omitempty"`
	BodyContains string `yaml:"body_contains,omitempty" json:"body_contains,omitempty"`
}

// Load reads and parses a services.yaml file.
func Load(path string) (*File, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read catalogue: %w", err)
	}
	var f File
	if err := yaml.Unmarshal(data, &f); err != nil {
		return nil, fmt.Errorf("parse catalogue: %w", err)
	}
	return &f, nil
}

// ToServiceChecks converts catalogue checks to probe.ServiceCheck format.
func (f *File) ToServiceChecks() []probe.ServiceCheck {
	var checks []probe.ServiceCheck
	for _, svc := range f.Services {
		for _, c := range svc.Checks {
			sc := probe.ServiceCheck{
				CAFile: c.CAFile,
				Name:   svc.Name,
				Type:   c.Type,
				URL:    c.URL,
				Host:   c.Host,
				Port:   c.Port,
				Expect: probe.ServiceExpect{
					Status:       c.Status(),
					BodyContains: c.Expect.BodyContains,
				},
			}
			checks = append(checks, sc)
		}
	}
	return checks
}

// Status returns the expected HTTP status, defaulting to 200 for http/https checks.
func (c *Check) Status() int {
	if c.Expect.Status != 0 {
		return c.Expect.Status
	}
	if c.Type == "http" || c.Type == "https" {
		return 200
	}
	return 0
}
