package compat

import (
	"os"
	"strings"
)

// defaults.go: settings that name a deployment. None has a built-in value:
// each comes from its environment variable, and unset means the feature it
// serves has no source (no suffix stripped, no gatewatch link, no collector
// URL in the served /install script, no satellite-link targets).

// PeerFQDNSuffixes are stripped from a peer's NetBird FQDN to make its
// auto-alias. Override: SNITCH_PEER_DOMAIN_SUFFIXES (comma-separated).
var PeerFQDNSuffixes = []string{}

// GatewatchURL is the gatewatch service the dashboard reads bridge status
// from. Override: SNITCH_GATEWATCH_URL.
const GatewatchURL = ""

// InstallCollectorURL is the collector URL written into the served
// /install script when the installing host does not set SNITCH_COLLECTOR.
// Override: SNITCH_PUBLIC_URL.
const InstallCollectorURL = ""

// SatlinkTarget is a relay used as a latency reference by the satellite-link
// PoP benchmark.
type SatlinkTarget struct {
	Name   string
	IP     string
	Region string
}

// SatlinkTargets are the relays the satellite-link PoP benchmark measures.
var SatlinkTargets = []SatlinkTarget{}

// SatlinkTargetsEnv overrides SatlinkTargets: a comma-separated list of
// name=ip=region entries, e.g. "fra=192.0.2.10=eu-central-1".
const SatlinkTargetsEnv = "SNITCH_SATLINK_TARGETS"

// ConfiguredSatlinkTargets returns the targets SNITCH_SATLINK_TARGETS names,
// or SatlinkTargets when it is unset or names none. An entry that is not
// name=ip=region is skipped.
func ConfiguredSatlinkTargets() []SatlinkTarget {
	return satlinkTargetsFrom(os.Getenv(SatlinkTargetsEnv))
}

func satlinkTargetsFrom(v string) []SatlinkTarget {
	var out []SatlinkTarget
	for _, entry := range strings.Split(v, ",") {
		parts := strings.Split(strings.TrimSpace(entry), "=")
		if len(parts) != 3 {
			continue
		}
		name, ip, region := strings.TrimSpace(parts[0]), strings.TrimSpace(parts[1]), strings.TrimSpace(parts[2])
		if name == "" || ip == "" {
			continue
		}
		out = append(out, SatlinkTarget{Name: name, IP: ip, Region: region})
	}
	if len(out) == 0 {
		return SatlinkTargets
	}
	return out
}
