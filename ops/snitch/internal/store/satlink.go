package store

import (
	"context"

	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"

	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

// satlinkSchema is executed alongside the main schema on Open(). The table,
// column and index names come from compat: they hold existing history.
var satlinkSchema = `
CREATE TABLE IF NOT EXISTS ` + compat.SatlinkProbesTable + ` (
	id BIGSERIAL PRIMARY KEY,
	ts BIGINT NOT NULL,
	egress_ip TEXT NOT NULL DEFAULT '',
	asn TEXT NOT NULL DEFAULT '',
	org TEXT NOT NULL DEFAULT '',
	city TEXT NOT NULL DEFAULT '',
	region TEXT NOT NULL DEFAULT '',
	country TEXT NOT NULL DEFAULT '',
	` + compat.SatlinkRangeColumn + ` BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS ` + compat.SatlinkMeasurementsTable + ` (
	id BIGSERIAL PRIMARY KEY,
	probe_id BIGINT NOT NULL REFERENCES ` + compat.SatlinkProbesTable + `(id) ON DELETE CASCADE,
	target TEXT NOT NULL,
	target_ip TEXT NOT NULL,
	ping_avg DOUBLE PRECISION,
	ping_p50 DOUBLE PRECISION,
	ping_p95 DOUBLE PRECISION,
	ping_jitter DOUBLE PRECISION,
	ping_loss DOUBLE PRECISION,
	tcp_avg DOUBLE PRECISION,
	tcp_p50 DOUBLE PRECISION,
	tcp_p95 DOUBLE PRECISION,
	tcp_jitter DOUBLE PRECISION,
	http_connect DOUBLE PRECISION,
	http_tls DOUBLE PRECISION,
	http_firstbyte DOUBLE PRECISION,
	hop_count INTEGER,
	traceroute TEXT DEFAULT ''
);

CREATE INDEX IF NOT EXISTS ` + compat.SatlinkProbesTSIndex + ` ON ` + compat.SatlinkProbesTable + ` USING BRIN(ts);
CREATE INDEX IF NOT EXISTS ` + compat.SatlinkMeasProbeIndex + ` ON ` + compat.SatlinkMeasurementsTable + `(probe_id);
`

// InsertSatlinkProbe stores a benchmark result and returns its row ID.
func (s *Store) InsertSatlinkProbe(ctx context.Context, r *types.SatlinkProbeResult) (int64, error) {
	var id int64
	err := s.pool.QueryRow(ctx, `
		INSERT INTO `+compat.SatlinkProbesTable+` (ts, egress_ip, asn, org, city, region, country, `+compat.SatlinkRangeColumn+`)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
		RETURNING id`,
		r.Timestamp, r.EgressIP, r.ASN, r.Org, r.City, r.Region, r.Country, r.InSatlinkRange).Scan(&id)
	return id, err
}

// InsertSatlinkMeasurement stores a single target measurement.
func (s *Store) InsertSatlinkMeasurement(ctx context.Context, probeID int64, m *types.SatlinkMeasurement) error {
	_, err := s.pool.Exec(ctx, `
		INSERT INTO `+compat.SatlinkMeasurementsTable+` (
			probe_id, target, target_ip,
			ping_avg, ping_p50, ping_p95, ping_jitter, ping_loss,
			tcp_avg, tcp_p50, tcp_p95, tcp_jitter,
			http_connect, http_tls, http_firstbyte,
			hop_count, traceroute)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)`,
		probeID, m.Target, m.TargetIP,
		m.PingAvg, m.PingP50, m.PingP95, m.PingJitter, m.PingLoss,
		m.TCPAvg, m.TCPP50, m.TCPP95, m.TCPJitter,
		m.HTTPConnect, m.HTTPTLS, m.HTTPFirstByte,
		m.HopCount, m.Traceroute)
	return err
}

// SatlinkLatest returns the most recent benchmark result with measurements.
func (s *Store) SatlinkLatest(ctx context.Context) (*types.SatlinkProbeResult, error) {
	r := &types.SatlinkProbeResult{}
	err := s.pool.QueryRow(ctx, `
		SELECT id, ts, egress_ip, asn, org, city, region, country, `+compat.SatlinkRangeColumn+`
		FROM `+compat.SatlinkProbesTable+` ORDER BY ts DESC LIMIT 1`).Scan(
		&r.ID, &r.Timestamp, &r.EgressIP, &r.ASN, &r.Org, &r.City, &r.Region, &r.Country, &r.InSatlinkRange)
	if err != nil {
		return nil, err
	}
	r.Measurements, err = s.satlinkMeasurements(ctx, r.ID)
	return r, err
}

// SatlinkPrevious returns the second-most-recent benchmark (for day-over-day).
func (s *Store) SatlinkPrevious(ctx context.Context) (*types.SatlinkProbeResult, error) {
	r := &types.SatlinkProbeResult{}
	err := s.pool.QueryRow(ctx, `
		SELECT id, ts, egress_ip, asn, org, city, region, country, `+compat.SatlinkRangeColumn+`
		FROM `+compat.SatlinkProbesTable+` ORDER BY ts DESC OFFSET 1 LIMIT 1`).Scan(
		&r.ID, &r.Timestamp, &r.EgressIP, &r.ASN, &r.Org, &r.City, &r.Region, &r.Country, &r.InSatlinkRange)
	if err != nil {
		return nil, err
	}
	r.Measurements, err = s.satlinkMeasurements(ctx, r.ID)
	return r, err
}

func (s *Store) satlinkMeasurements(ctx context.Context, probeID int64) ([]types.SatlinkMeasurement, error) {
	rows, err := s.pool.Query(ctx, `
		SELECT target, target_ip,
			ping_avg, ping_p50, ping_p95, ping_jitter, ping_loss,
			tcp_avg, tcp_p50, tcp_p95, tcp_jitter,
			http_connect, http_tls, http_firstbyte,
			hop_count, COALESCE(traceroute, '')
		FROM `+compat.SatlinkMeasurementsTable+`
		WHERE probe_id = $1 ORDER BY target`, probeID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var ms []types.SatlinkMeasurement
	for rows.Next() {
		var m types.SatlinkMeasurement
		if err := rows.Scan(
			&m.Target, &m.TargetIP,
			&m.PingAvg, &m.PingP50, &m.PingP95, &m.PingJitter, &m.PingLoss,
			&m.TCPAvg, &m.TCPP50, &m.TCPP95, &m.TCPJitter,
			&m.HTTPConnect, &m.HTTPTLS, &m.HTTPFirstByte,
			&m.HopCount, &m.Traceroute); err != nil {
			return nil, err
		}
		ms = append(ms, m)
	}
	return ms, rows.Err()
}

// SatlinkTrend returns per-target trend data over a time range.
func (s *Store) SatlinkTrend(ctx context.Context, from, to int64) ([]types.SatlinkTrendPoint, error) {
	rows, err := s.pool.Query(ctx, `
		SELECT p.ts, m.target, m.ping_avg, m.ping_p95, m.ping_jitter, m.ping_loss, m.tcp_avg
		FROM `+compat.SatlinkProbesTable+` p
		JOIN `+compat.SatlinkMeasurementsTable+` m ON m.probe_id = p.id
		WHERE p.ts >= $1 AND p.ts <= $2
		ORDER BY p.ts, m.target`, from, to)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var points []types.SatlinkTrendPoint
	for rows.Next() {
		var p types.SatlinkTrendPoint
		if err := rows.Scan(&p.Timestamp, &p.Target, &p.PingAvg, &p.PingP95, &p.Jitter, &p.Loss, &p.TCPAvg); err != nil {
			return nil, err
		}
		points = append(points, p)
	}
	return points, rows.Err()
}

// SatlinkPops returns the egress PoP at each probe time (for tracking PoP changes).
func (s *Store) SatlinkPops(ctx context.Context, from, to int64) ([]types.SatlinkPopEvent, error) {
	rows, err := s.pool.Query(ctx, `
		SELECT ts, egress_ip, city, country, asn, `+compat.SatlinkRangeColumn+`
		FROM `+compat.SatlinkProbesTable+`
		WHERE ts >= $1 AND ts <= $2
		ORDER BY ts`, from, to)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var events []types.SatlinkPopEvent
	for rows.Next() {
		var e types.SatlinkPopEvent
		if err := rows.Scan(&e.Timestamp, &e.EgressIP, &e.City, &e.Country, &e.ASN, &e.InSatlinkRange); err != nil {
			return nil, err
		}
		events = append(events, e)
	}
	return events, rows.Err()
}
