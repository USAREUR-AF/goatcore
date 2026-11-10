// Tests for transport-quality measurement.
//
// Every `ss` fixture below is a REAL capture from the RCA
// of two production failures, not an invention. That matters: the point of this probe is
// to recognize those two failures, so the tests assert against the actual
// bytes the kernel emitted while each failure was happening. A synthetic
// fixture would only prove the parser matches my idea of the format.
package measure

import (
	"testing"
)

// The shape of a congestion collapse: 5 Mbps on a link
// carrying 30 Mbps of UDP. cwnd pinned at the floor with data queued.
const ssCollapsed = `State Recv-Q Send-Q Local Address:Port   Peer Address:Port
ESTAB 0      3229312 100.127.0.217:5401   100.127.0.228:65513
	 cubic wscale:7,7 rto:276 rtt:72.235/28.649 ato:40 mss:1188 pmtu:1280 rcvmss:1228 advmss:1228 cwnd:2 ssthresh:2 bytes_sent:13491015 bytes_retrans:2060584 bytes_acked:11430431 segs_out:11354 segs_in:5521 data_segs_out:11350 send 263000bps lastsnd:4 pacing_rate 526000bps delivery_rate 331680bps delivered:9672 busy:38268ms unacked:2 retrans:2/1678 lost:2 notsent:38068 minrtt:46.829`

// Captured on the same host minutes later under BBR: the fix working.
const ssHealthy = `State Recv-Q Send-Q Local Address:Port   Peer Address:Port
ESTAB 0      1310720 100.127.0.217:5401   100.127.0.228:65530
	 bbr wscale:7,7 rto:264 rtt:63.112/4.201 ato:40 mss:1188 pmtu:1280 rcvmss:1228 advmss:1228 cwnd:906 ssthresh:64 bytes_sent:44201984 bytes_retrans:1517184 bytes_acked:42684800 segs_out:37207 segs_in:12004 data_segs_out:37203 send 136000000bps pacing_rate 149000000bps delivery_rate 34240000bps delivered:35921 busy:10400ms retrans:0/1277 notsent:1310720 minrtt:60.104`

// An idle-but-established connection: the app has nothing to send. cwnd is
// small for a legitimate reason and MUST NOT be flagged.
const ssAppLimited = `State Recv-Q Send-Q Local Address:Port   Peer Address:Port
ESTAB 0      0      100.127.0.217:22   100.127.0.228:65513
	 cubic wscale:6,7 rto:276 rtt:72.235/28.649 ato:40 mss:1228 pmtu:1280 cwnd:10 bytes_sent:4342 bytes_acked:4270 segs_out:15 delivered:11 app_limited busy:660ms minrtt:46.829`

func TestParseSS_CollapsedWindowIsFlagged(t *testing.T) {
	q := parseSSOutput(ssCollapsed)
	if q == nil {
		t.Fatal("expected a sample from real ss output")
	}
	if q.MinCwnd != 2 {
		t.Errorf("MinCwnd = %d, want 2", q.MinCwnd)
	}
	if q.MinSsthresh != 2 {
		t.Errorf("MinSsthresh = %d, want 2", q.MinSsthresh)
	}
	if q.CongestionControl != "cubic" {
		t.Errorf("CongestionControl = %q, want cubic", q.CongestionControl)
	}
	// 2060584/13491015 = 15.3% — the number that showed the loss was real.
	if q.RetransPct < 15 || q.RetransPct > 16 {
		t.Errorf("RetransPct = %.2f, want ~15.3", q.RetransPct)
	}
	if q.NotSentBytes != 38068 {
		t.Errorf("NotSentBytes = %d, want 38068", q.NotSentBytes)
	}
	// THE ASSERTION THAT MATTERS. This is the state that cost ten hours to
	// find by hand; the probe must name it without a human interpreting.
	if !q.CwndCollapsed {
		t.Error("CwndCollapsed = false on the real F-301 capture — this probe " +
			"would not have caught the defect it exists to catch")
	}
}

func TestParseSS_HealthyWindowIsNotFlagged(t *testing.T) {
	q := parseSSOutput(ssHealthy)
	if q == nil {
		t.Fatal("expected a sample")
	}
	if q.MinCwnd != 906 {
		t.Errorf("MinCwnd = %d, want 906", q.MinCwnd)
	}
	if q.CongestionControl != "bbr" {
		t.Errorf("CongestionControl = %q, want bbr", q.CongestionControl)
	}
	if q.CwndCollapsed {
		t.Error("CwndCollapsed on a 906-segment window — false positive")
	}
	// minrtt vs rtt is the queueing/jitter signal on a satellite leg.
	if q.MinRTTMs != 60.104 || q.RTTMs != 63.112 {
		t.Errorf("rtt=%.3f minrtt=%.3f, want 63.112/60.104", q.RTTMs, q.MinRTTMs)
	}
}

func TestParseSS_AppLimitedIsNotACollapse(t *testing.T) {
	// A small cwnd because the app sent nothing is NORMAL. Flagging it would
	// make the alert worthless — every idle SSH session would fire it.
	q := parseSSOutput(ssAppLimited)
	if q == nil {
		t.Fatal("expected a sample")
	}
	if !q.AppLimited {
		t.Error("AppLimited not detected")
	}
	if q.CwndCollapsed {
		t.Error("CwndCollapsed on an app-limited socket — this is the false " +
			"positive that would train operators to ignore the alert")
	}
}

func TestParseSS_NoConnectionsReturnsNilNotZero(t *testing.T) {
	// A link carrying no TCP has no TCP quality. Emitting a zero-valued
	// sample would poison the time series with fake healthy readings.
	if q := parseSSOutput("State Recv-Q Send-Q Local Address:Port Peer Address:Port\n"); q != nil {
		t.Errorf("expected nil for no established connections, got %+v", q)
	}
}

func TestParseSS_WorstCaseWinsAcrossConnections(t *testing.T) {
	// One collapsed flow beside a healthy one IS the failure. An average
	// would hide it, so the parser must keep the minimum.
	q := parseSSOutput(ssHealthy + "\n" + ssCollapsed)
	if q == nil {
		t.Fatal("expected a sample")
	}
	if q.MinCwnd != 2 {
		t.Errorf("MinCwnd = %d, want 2 (the worst of the two)", q.MinCwnd)
	}
}

// ── kernel-measured reordering ──────────────────────────────────────────────

func TestParseSS_ReadsKernelReorderingEstimate(t *testing.T) {
	// Real shape, carrying `reordering:8`. The kernel measures displacement on live traffic;
	// reading it costs nothing and needs no listening port.
	const withReord = `State Recv-Q Send-Q Local Address:Port   Peer Address:Port
ESTAB 0      0      100.127.0.217:5401   100.127.0.228:65513
	 cubic wscale:7,7 rto:276 rtt:72.2/28.6 mss:1188 cwnd:40 reordering:8 bytes_sent:100 bytes_acked:90 minrtt:46.8`
	q := parseSSOutput(withReord)
	if q == nil {
		t.Fatal("expected a sample")
	}
	if q.Reordering != 8 {
		t.Errorf("Reordering = %d, want 8", q.Reordering)
	}
	if !q.ReorderTripsFastRetransmit {
		t.Error("degree 8 is well past the 3-dupACK threshold and must trip")
	}
}

func TestParseSS_DefaultReorderingDegreeIsNotAFinding(t *testing.T) {
	// Linux SEEDS the estimate at 3. Treating the seed as evidence would flag
	// every healthy socket on the fleet and train operators to ignore it.
	const seeded = `State Recv-Q Send-Q Local Address:Port   Peer Address:Port
ESTAB 0      0      100.127.0.217:22   100.127.0.228:65530
	 bbr wscale:7,7 rto:264 rtt:63.1/4.2 mss:1188 cwnd:900 reordering:3 bytes_sent:100 bytes_acked:100 minrtt:60.1`
	q := parseSSOutput(seeded)
	if q == nil {
		t.Fatal("expected a sample")
	}
	if q.ReorderTripsFastRetransmit {
		t.Error("degree 3 is the kernel's default, not observed reordering — " +
			"flagging it makes the signal noise")
	}
}

func TestParseSS_WorstReorderingWins(t *testing.T) {
	// Same rule as cwnd: one badly-reordering path is the finding, and an
	// average against well-behaved connections would bury it.
	const two = `State Recv-Q Send-Q Local Address:Port   Peer Address:Port
ESTAB 0 0 100.127.0.217:22 100.127.0.228:1
	 bbr wscale:7,7 rtt:63.1/4.2 mss:1188 cwnd:900 reordering:3 bytes_sent:1 bytes_acked:1 minrtt:60.1
ESTAB 0 0 100.127.0.217:5401 100.127.0.228:2
	 cubic wscale:7,7 rtt:72.2/28.6 mss:1188 cwnd:40 reordering:11 bytes_sent:1 bytes_acked:1 minrtt:46.8`
	q := parseSSOutput(two)
	if q == nil {
		t.Fatal("expected a sample")
	}
	if q.Reordering != 11 {
		t.Errorf("Reordering = %d, want 11 (the worst of the two)", q.Reordering)
	}
}
