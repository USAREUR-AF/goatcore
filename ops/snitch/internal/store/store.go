// Package store provides PostgreSQL-backed persistence for SNITCH analytics.
//
// Tables:
//   - peer_identities: token -> encrypted name/IP (ZTA data at rest)
//   - reports: per-probe-cycle metadata (network fingerprint)
//   - measurements: per-link metrics (latency, jitter, loss, path type)
//   - events: state changes (peer online/offline, path changes)
//
// Uses BRIN indexes on timestamp columns for efficient time-range scans.
// Retention: 30 days raw data, 90 days events. Cleanup runs hourly.
package store

import (
	"context"
	"fmt"
	"log"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
)

// Store is the PostgreSQL persistence layer.
type Store struct {
	pool *pgxpool.Pool
	done chan struct{}
}

const schema = `
CREATE TABLE IF NOT EXISTS peer_identities (
	token TEXT PRIMARY KEY,
	encrypted_name BYTEA NOT NULL,
	encrypted_ip BYTEA NOT NULL,
	first_seen BIGINT NOT NULL,
	last_seen BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS reports (
	id BIGSERIAL PRIMARY KEY,
	token TEXT NOT NULL,
	ts BIGINT NOT NULL,
	interface_name TEXT,
	interface_type TEXT,
	gateway_ip_token TEXT,
	gateway_rtt_ms DOUBLE PRECISION,
	public_ip_hash TEXT,
	asn TEXT,
	asn_org TEXT,
	subnet TEXT,
	gateway_mac TEXT,
	gateway_manufacturer TEXT,
	gateway_model TEXT
);

CREATE TABLE IF NOT EXISTS measurements (
	id BIGSERIAL PRIMARY KEY,
	report_id BIGINT NOT NULL,
	from_token TEXT NOT NULL,
	to_token TEXT NOT NULL,
	ts BIGINT NOT NULL,
	connected BOOLEAN NOT NULL DEFAULT FALSE,
	direct BOOLEAN NOT NULL DEFAULT FALSE,
	latency_avg DOUBLE PRECISION,
	latency_p50 DOUBLE PRECISION,
	latency_p95 DOUBLE PRECISION,
	jitter DOUBLE PRECISION,
	loss DOUBLE PRECISION,
	samples INTEGER,
	path_type TEXT
);

CREATE TABLE IF NOT EXISTS events (
	id BIGSERIAL PRIMARY KEY,
	ts BIGINT NOT NULL,
	token TEXT NOT NULL,
	event_type TEXT NOT NULL,
	target_token TEXT DEFAULT '',
	old_value TEXT DEFAULT '',
	new_value TEXT DEFAULT '',
	details TEXT DEFAULT ''
);

CREATE TABLE IF NOT EXISTS api_keys (
	id BIGSERIAL PRIMARY KEY,
	key_hash TEXT NOT NULL UNIQUE,
	role TEXT NOT NULL CHECK(role IN ('admin','operator','viewer','probe')),
	label TEXT NOT NULL,
	created_at BIGINT NOT NULL,
	last_used BIGINT
);

CREATE TABLE IF NOT EXISTS aliases (
	entity_type TEXT NOT NULL CHECK(entity_type IN ('peer','gateway')),
	entity_key TEXT NOT NULL,
	alias TEXT NOT NULL,
	created_at BIGINT NOT NULL,
	updated_at BIGINT NOT NULL,
	PRIMARY KEY (entity_type, entity_key)
);

CREATE INDEX IF NOT EXISTS idx_reports_ts ON reports USING BRIN (ts);
CREATE INDEX IF NOT EXISTS idx_reports_token_ts ON reports (token, ts);
CREATE INDEX IF NOT EXISTS idx_measurements_ts ON measurements USING BRIN (ts);
CREATE INDEX IF NOT EXISTS idx_measurements_link_ts ON measurements (from_token, to_token, ts);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events USING BRIN (ts);
CREATE INDEX IF NOT EXISTS idx_events_token_ts ON events (token, ts);

`

// Open connects to PostgreSQL and runs migrations.
func Open(ctx context.Context, connString string) (*Store, error) {
	pool, err := pgxpool.New(ctx, connString)
	if err != nil {
		return nil, fmt.Errorf("connect: %w", err)
	}
	if err := pool.Ping(ctx); err != nil {
		pool.Close()
		return nil, fmt.Errorf("ping: %w", err)
	}
	if _, err := pool.Exec(ctx, schema); err != nil {
		pool.Close()
		return nil, fmt.Errorf("migrate: %w", err)
	}
	if _, err := pool.Exec(ctx, satlinkSchema); err != nil {
		pool.Close()
		return nil, fmt.Errorf("migrate satlink: %w", err)
	}
	s := &Store{pool: pool, done: make(chan struct{})}
	// retentionLoop is a long-lived background goroutine bound to the
	// Store's lifetime, not to the Open caller's ctx. It owns its own
	// per-cleanup ctx via context.Background(); shutdown signals via
	// the s.done channel.
	go s.retentionLoop() //nolint:contextcheck // see comment
	return s, nil
}

// Pool returns the connection pool for shared use (auth, etc.).
func (s *Store) Pool() *pgxpool.Pool { return s.pool }

// Close stops retention and closes the pool.
func (s *Store) Close() {
	close(s.done)
	s.pool.Close()
}

// UpsertIdentity inserts or updates a peer's encrypted identity.
func (s *Store) UpsertIdentity(ctx context.Context, token string, encName, encIP []byte) error {
	now := time.Now().Unix()
	_, err := s.pool.Exec(ctx, `
		INSERT INTO peer_identities (token, encrypted_name, encrypted_ip, first_seen, last_seen)
		VALUES ($1, $2, $3, $4, $5)
		ON CONFLICT (token) DO UPDATE SET
			encrypted_name = EXCLUDED.encrypted_name,
			encrypted_ip = EXCLUDED.encrypted_ip,
			last_seen = EXCLUDED.last_seen`,
		token, encName, encIP, now, now)
	return err
}

// AllIdentities returns all token -> [encName, encIP] mappings.
func (s *Store) AllIdentities(ctx context.Context) (map[string][2][]byte, error) {
	rows, err := s.pool.Query(ctx, `SELECT token, encrypted_name, encrypted_ip FROM peer_identities`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := make(map[string][2][]byte)
	for rows.Next() {
		var token string
		var encName, encIP []byte
		if err := rows.Scan(&token, &encName, &encIP); err != nil {
			return nil, err
		}
		result[token] = [2][]byte{encName, encIP}
	}
	return result, rows.Err()
}

// InsertReport stores a probe report and returns its row ID.
func (s *Store) InsertReport(ctx context.Context, token string, ts int64,
	ifName, ifType, gwToken string, gwRTT float64,
	pubIPHash, asn, asnOrg, subnet, gwMAC, gwMfg, gwModel string) (int64, error) {
	var id int64
	err := s.pool.QueryRow(ctx, `
		INSERT INTO reports (token, ts, interface_name, interface_type, gateway_ip_token,
			gateway_rtt_ms, public_ip_hash, asn, asn_org, subnet, gateway_mac,
			gateway_manufacturer, gateway_model)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
		RETURNING id`,
		token, ts, ifName, ifType, gwToken, gwRTT, pubIPHash, asn, asnOrg, subnet,
		gwMAC, gwMfg, gwModel).Scan(&id)
	return id, err
}

// InsertMeasurement stores a single peer-to-peer measurement.
func (s *Store) InsertMeasurement(ctx context.Context, reportID int64,
	fromToken, toToken string, ts int64,
	connected, direct bool, latAvg, latP50, latP95, jitter, loss float64,
	samples int, pathType string) error {
	_, err := s.pool.Exec(ctx, `
		INSERT INTO measurements (report_id, from_token, to_token, ts, connected, direct,
			latency_avg, latency_p50, latency_p95, jitter, loss, samples, path_type)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)`,
		reportID, fromToken, toToken, ts, connected, direct,
		latAvg, latP50, latP95, jitter, loss, samples, pathType)
	return err
}

// InsertEvent stores a state change event.
func (s *Store) InsertEvent(ctx context.Context, ts int64,
	token, eventType, targetToken, oldVal, newVal, details string) error {
	_, err := s.pool.Exec(ctx, `
		INSERT INTO events (ts, token, event_type, target_token, old_value, new_value, details)
		VALUES ($1,$2,$3,$4,$5,$6,$7)`,
		ts, token, eventType, targetToken, oldVal, newVal, details)
	return err
}

// Alias is a human-readable name for a peer or gateway.
type Alias struct {
	EntityType string `json:"entity_type"` // "peer" or "gateway"
	EntityKey  string `json:"entity_key"`  // IP address
	Alias      string `json:"alias"`
	CreatedAt  int64  `json:"created_at"`
	UpdatedAt  int64  `json:"updated_at"`
}

// UpsertAlias sets a human-readable alias for a peer or gateway IP.
func (s *Store) UpsertAlias(ctx context.Context, entityType, entityKey, alias string) error {
	now := time.Now().Unix()
	_, err := s.pool.Exec(ctx, `
		INSERT INTO aliases (entity_type, entity_key, alias, created_at, updated_at)
		VALUES ($1, $2, $3, $4, $5)
		ON CONFLICT (entity_type, entity_key) DO UPDATE SET
			alias = EXCLUDED.alias,
			updated_at = EXCLUDED.updated_at`,
		entityType, entityKey, alias, now, now)
	return err
}

// AllAliases returns all registered aliases.
func (s *Store) AllAliases(ctx context.Context) ([]Alias, error) {
	rows, err := s.pool.Query(ctx, `SELECT entity_type, entity_key, alias, created_at, updated_at FROM aliases ORDER BY entity_type, alias`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []Alias
	for rows.Next() {
		var a Alias
		if err := rows.Scan(&a.EntityType, &a.EntityKey, &a.Alias, &a.CreatedAt, &a.UpdatedAt); err != nil {
			return nil, err
		}
		result = append(result, a)
	}
	return result, rows.Err()
}

// DeleteAlias removes an alias.
func (s *Store) DeleteAlias(ctx context.Context, entityType, entityKey string) error {
	_, err := s.pool.Exec(ctx, `DELETE FROM aliases WHERE entity_type = $1 AND entity_key = $2`, entityType, entityKey)
	return err
}

// HasAlias checks whether an alias exists for the given entity.
func (s *Store) HasAlias(ctx context.Context, entityType, entityKey string) (bool, error) {
	var exists bool
	err := s.pool.QueryRow(ctx,
		`SELECT EXISTS(SELECT 1 FROM aliases WHERE entity_type = $1 AND entity_key = $2)`,
		entityType, entityKey).Scan(&exists)
	return exists, err
}

// AliasMap returns a map of "type:key" -> alias for quick lookups.
func (s *Store) AliasMap(ctx context.Context) (map[string]string, error) {
	aliases, err := s.AllAliases(ctx)
	if err != nil {
		return nil, err
	}
	m := make(map[string]string, len(aliases))
	for _, a := range aliases {
		m[a.EntityType+":"+a.EntityKey] = a.Alias
	}
	return m, nil
}

func (s *Store) retentionLoop() {
	ticker := time.NewTicker(1 * time.Hour)
	defer ticker.Stop()
	for {
		select {
		case <-s.done:
			return
		case <-ticker.C:
			s.cleanup()
		}
	}
}

func (s *Store) cleanup() {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	now := time.Now().Unix()
	cut30d := now - 30*24*3600
	cut90d := now - 90*24*3600

	for _, q := range []struct {
		label string
		sql   string
		cut   int64
	}{
		{"measurements", `DELETE FROM measurements WHERE ts < $1`, cut30d},
		{"reports", `DELETE FROM reports WHERE ts < $1`, cut30d},
		{"events", `DELETE FROM events WHERE ts < $1`, cut90d},
		{compat.SatlinkProbesTable, `DELETE FROM ` + compat.SatlinkProbesTable + ` WHERE ts < $1`, cut90d},
	} {
		tag, err := s.pool.Exec(ctx, q.sql, q.cut)
		if err != nil {
			log.Printf("cleanup %s: %v", q.label, err)
		} else if tag.RowsAffected() > 0 {
			log.Printf("cleaned %d %s", tag.RowsAffected(), q.label)
		}
	}
}
