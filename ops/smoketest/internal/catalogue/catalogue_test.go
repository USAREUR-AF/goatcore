package catalogue

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/smoketest/internal/probe"
)

func TestCheckStatus(t *testing.T) {
	tests := []struct {
		name  string
		check Check
		want  int
	}{
		{name: "http default", check: Check{Type: "http"}, want: 200},
		{name: "https default", check: Check{Type: "https"}, want: 200},
		{name: "http explicit", check: Check{Type: "http", Expect: Expect{Status: 204}}, want: 204},
		{name: "https explicit", check: Check{Type: "https", Expect: Expect{Status: 418}}, want: 418},
		{name: "tcp", check: Check{Type: "tcp"}, want: 0},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := tt.check.Status(); got != tt.want {
				t.Fatalf("Status() = %d, want %d", got, tt.want)
			}
		})
	}
}

func TestLoadMissingFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "missing.yaml")

	_, err := Load(path)
	if err == nil {
		t.Fatal("Load() error = nil, want a missing-file error")
	}
	if !strings.Contains(err.Error(), "read catalogue") {
		t.Fatalf("Load() error = %q, want read catalogue context", err)
	}
	var pathErr *os.PathError
	if !errors.As(err, &pathErr) {
		t.Fatalf("Load() error = %v, want wrapped *os.PathError", err)
	}
	if pathErr.Path != path {
		t.Fatalf("Load() path = %q, want %q", pathErr.Path, path)
	}
}

func TestLoadMalformedYAML(t *testing.T) {
	path := filepath.Join(t.TempDir(), "services.yaml")
	data := []byte("services:\n  - name: broken\n    checks: [\n")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}

	_, err := Load(path)
	if err == nil {
		t.Fatal("Load() error = nil, want a malformed-YAML error")
	}
	if !strings.Contains(err.Error(), "parse catalogue") {
		t.Fatalf("Load() error = %q, want parse catalogue context", err)
	}
	cause := errors.Unwrap(err)
	if cause == nil || !strings.Contains(cause.Error(), "yaml:") {
		t.Fatalf("Load() cause = %v, want YAML parser context", cause)
	}
}

func TestToServiceChecks(t *testing.T) {
	path := filepath.Join(t.TempDir(), "services.yaml")
	data := []byte(`services:
  - name: first-service
    checks:
      - type: https
        url: https://first.example.org/ready
        ca_file: /etc/example/first-ca.pem
        expect:
          status: 204
          body_contains: ready
      - type: tcp
        host: first.example.org
        port: 443
  - name: second-service
    checks:
      - type: http
        url: http://second.example.org/health
`)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}

	catalogue, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}

	want := []probe.ServiceCheck{
		{
			CAFile: "/etc/example/first-ca.pem",
			Name:   "first-service",
			Type:   "https",
			URL:    "https://first.example.org/ready",
			Expect: probe.ServiceExpect{Status: 204, BodyContains: "ready"},
		},
		{
			Name:   "first-service",
			Type:   "tcp",
			Host:   "first.example.org",
			Port:   443,
			Expect: probe.ServiceExpect{Status: 0},
		},
		{
			Name:   "second-service",
			Type:   "http",
			URL:    "http://second.example.org/health",
			Expect: probe.ServiceExpect{Status: 200},
		},
	}
	if got := catalogue.ToServiceChecks(); !reflect.DeepEqual(got, want) {
		t.Fatalf("ToServiceChecks() = %#v, want %#v", got, want)
	}
}
