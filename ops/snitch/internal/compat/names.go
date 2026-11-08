// Package compat holds the names and built-in defaults that existing
// snitch deployments depend on and that therefore cannot change with the
// rest of the source.
//
// names.go: names stored on disk or used from outside the binary. The
// PostgreSQL tables, columns and indexes below hold the satellite-link PoP
// history on every collector, so a collector keeps the names its history
// was written under. The CLI subcommand, serve flag, HTTP routes and JSON
// key below are accepted (or emitted) beside the current names so existing
// scripts, bookmarks and API clients keep working.
//
// All of them are formed from one word, SNITCH_SATLINK_NAME (default
// "satlink"). A deployment whose collectors hold history under another word
// sets it on the collector and on its probes.
package compat

import (
	"os"
	"regexp"
)

// SatlinkNameEnv names the word the satellite-link names below are formed from.
const SatlinkNameEnv = "SNITCH_SATLINK_NAME"

// satlinkNamePattern keeps the word a plain SQL identifier part.
var satlinkNamePattern = regexp.MustCompile(`^[a-z][a-z0-9_]{0,30}$`)

// SatlinkWord is the configured word, or "satlink" when SNITCH_SATLINK_NAME
// is unset or not a lowercase identifier.
var SatlinkWord = satlinkWord(os.Getenv(SatlinkNameEnv))

func satlinkWord(v string) string {
	if satlinkNamePattern.MatchString(v) {
		return v
	}
	return "satlink"
}

// Satellite-link PoP history tables (PostgreSQL).
var (
	SatlinkProbesTable       = SatlinkWord + "_probes"
	SatlinkMeasurementsTable = SatlinkWord + "_measurements"
	SatlinkRangeColumn       = "in_" + SatlinkWord + "_range"
	SatlinkProbesTSIndex     = "idx_" + SatlinkWord + "_probes_ts"
	SatlinkMeasProbeIndex    = "idx_" + SatlinkWord + "_meas_probe"
)

// Interface names accepted beside the current ones.
var (
	// SatlinkProbeCommand is the earlier name of the `satlink-probe` subcommand.
	SatlinkProbeCommand = SatlinkWord + "-probe"
	// SatlinkIntervalFlag is the earlier name of the serve flag `--satlink-interval`.
	SatlinkIntervalFlag = SatlinkWord + "-interval"
	// SatlinkPageRoute is the earlier path of the `/satlink` page.
	SatlinkPageRoute = "/" + SatlinkWord
	// SatlinkAPIPrefix is the earlier prefix of the `/api/satlink/` routes.
	SatlinkAPIPrefix = "/api/" + SatlinkWord + "/"
	// SatlinkRangeJSONKey is the earlier JSON key of `in_satlink_range`;
	// API responses carry both keys.
	SatlinkRangeJSONKey = "in_" + SatlinkWord + "_range"
)
