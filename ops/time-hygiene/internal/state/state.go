package state

import (
	"math"
	"time"
)

// NtpClass is the health verdict published by ntp-health-agent.
// The integer values are stable and used in the 5-byte Zenoh attachment.
type NtpClass uint8

const (
	ClassHealthy  NtpClass = 0
	ClassWarning  NtpClass = 1
	ClassDegraded NtpClass = 2
	ClassUnknown  NtpClass = 3
)

func (c NtpClass) String() string {
	switch c {
	case ClassHealthy:
		return "healthy"
	case ClassWarning:
		return "warning"
	case ClassDegraded:
		return "degraded"
	default:
		return "unknown"
	}
}

// SelectedSource identifies which tier of time source chrony is currently using.
type SelectedSource string

const (
	SourceGNSS    SelectedSource = "gnss"
	SourcePeerNTS SelectedSource = "peer_nts"
	SourceWanNTS  SelectedSource = "wan_nts"
	SourceOrphan  SelectedSource = "orphan"
	SourceNone    SelectedSource = "none"
)

// DriftModel holds the per-node adaptive drift state (Block 12E, Tier 3).
// In v1 (Block 12A), these fields are zeroed and documented as not-yet-active.
type DriftModel struct {
	PpmMean          float64        `json:"ppm_mean"`
	PpmStddev        float64        `json:"ppm_stddev"`
	LastSyncUTC      time.Time      `json:"last_sync_utc"`
	SecondsSinceSync float64        `json:"seconds_since_sync"`
	ExpectedSkewUsec float64        `json:"expected_skew_usec"`
	SkewUncertainty  float64        `json:"skew_uncertainty_usec"`
	SampleCount      int            `json:"sample_count"`
	SourceOfLastSync SelectedSource `json:"source_of_last_sync"`
}

// NtpState is the authoritative snapshot exported via Unix socket, HTTP, and Prometheus.
// Fields match universal-ntp-hygiene.md §4.3.
type NtpState struct {
	HostID            string         `json:"host_id"`
	NtpClass          NtpClass       `json:"ntp_class"`
	NtpClassStr       string         `json:"ntp_class_str"`
	SelectedSource    SelectedSource `json:"selected_source"`
	Stratum           uint16         `json:"stratum"`
	RefID             string         `json:"ref_id"`
	SystemOffsetNs    int64          `json:"system_time_offset_ns"`
	RmsOffsetNs       int64          `json:"rms_offset_ns"`
	FreqPPM           float64        `json:"freq_ppm"`
	LeapStatus        uint16         `json:"leap_status"`
	LastUpdatedUTC    time.Time      `json:"last_updated_utc"`
	DriftModel        DriftModel     `json:"drift_model"`

	// Signed-attestation envelope (Block 12D / increment 2a). Populated by
	// internal/attest at each poll so any consumer can verify "this host
	// asserted this verdict, valid until ValidUntil" without trusting the
	// transport — the precondition for cross-site peer consensus
	// (universal-ntp-hygiene.md §5) and the @/ntp-health bus feed (increment
	// 2b). ValidUntil bounds the signed validity window; Signature is base64
	// Ed25519 over the canonical signing view (internal/attest); AttestationAlg
	// versions that view; AttestationPubkey is the base64 32-byte public key;
	// Attested is false when the agent runs without an attestation key (honest
	// unattested-publisher, universal-ntp-hygiene.md §2).
	//
	// Adding these is backward-compatible: a socket reader that decodes only
	// ntp_class_str and does not set DisallowUnknownFields ignores the extra
	// fields. ntp_class_str stays a top-level field — do not nest it.
	ValidUntil        time.Time `json:"valid_until,omitempty"`
	Attested          bool      `json:"attested"`
	AttestationAlg    string    `json:"attestation_alg,omitempty"`
	AttestationPubkey string    `json:"attestation_pubkey,omitempty"`
	Signature         string    `json:"signature,omitempty"`
}

// Classify returns the NtpClass based on tracking data.
//
// v1 classification uses stratum and RMS offset; v2 (Block 12E) will extend this
// with self-clock skew from DriftModel when no Tier 2 source is available.
func Classify(stratum uint16, leapStatus uint16, rmsOffsetSec float64) NtpClass {
	rmsNs := math.Abs(rmsOffsetSec) * 1e9

	// Unsynchronized: chrony leap-status 3 means "not synced" / stratum 16 means unreachable.
	if leapStatus == 3 || stratum >= 16 {
		return ClassUnknown
	}
	// Orphan mode (chrony local stratum 10+).
	if stratum >= 10 {
		return ClassDegraded
	}

	switch {
	case rmsNs <= 1_000_000 && stratum <= 4:
		return ClassHealthy
	case rmsNs <= 10_000_000 && stratum <= 6:
		return ClassWarning
	case rmsNs <= 100_000_000:
		return ClassDegraded
	default:
		return ClassUnknown
	}
}

// ClassifySource maps a chrony RefID string and stratum to a SelectedSource.
// This is heuristic for v1; v2 will use actual source-mode data from GetSources.
func ClassifySource(refID string, stratum uint16) SelectedSource {
	if stratum >= 16 {
		return SourceNone
	}
	if stratum >= 10 {
		return SourceOrphan
	}
	// Hardware refclocks typically have a 4-char ASCII refID like "GPS", "PPS", "GNSS".
	switch refID {
	case "GPS", "PPS", "GNSS", "GAL", "GLO", "BDS", "GPSDO", "TSIP":
		return SourceGNSS
	}
	return SourceWanNTS
}
