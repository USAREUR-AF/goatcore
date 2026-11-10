//go:build integration

package store

// Round trip of the satellite-link PoP history against a real PostgreSQL:
// the schema creates the compat table/column names existing collectors
// already hold, and every query reads and writes them.

import (
	"context"
	"testing"
	"time"

	"github.com/testcontainers/testcontainers-go"
	"github.com/testcontainers/testcontainers-go/wait"

	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

func startStore(t *testing.T) *Store {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	pgC, err := testcontainers.GenericContainer(ctx, testcontainers.GenericContainerRequest{
		ContainerRequest: testcontainers.ContainerRequest{
			Image:        "postgres:16-alpine",
			ExposedPorts: []string{"5432/tcp"},
			Env:          map[string]string{"POSTGRES_PASSWORD": "snitchtest", "POSTGRES_DB": "snitch"},
			WaitingFor: wait.ForLog("database system is ready to accept connections").
				WithOccurrence(2).WithStartupTimeout(60 * time.Second),
		},
		Started: true,
	})
	if err != nil {
		t.Fatalf("start postgres: %v", err)
	}
	t.Cleanup(func() { _ = pgC.Terminate(context.Background()) })
	host, err := pgC.Host(ctx)
	if err != nil {
		t.Fatalf("container host: %v", err)
	}
	port, err := pgC.MappedPort(ctx, "5432/tcp")
	if err != nil {
		t.Fatalf("container port: %v", err)
	}
	st, err := Open(ctx, "postgres://postgres:snitchtest@"+host+":"+port.Port()+"/snitch?sslmode=disable")
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	t.Cleanup(st.Close)
	return st
}

func TestSatlinkHistoryRoundTrip(t *testing.T) {
	st := startStore(t)
	ctx := context.Background()

	for _, tc := range []struct{ table, column string }{
		{compat.SatlinkProbesTable, compat.SatlinkRangeColumn},
		{compat.SatlinkMeasurementsTable, "probe_id"},
	} {
		var n int
		if err := st.pool.QueryRow(ctx,
			`SELECT count(*) FROM information_schema.columns WHERE table_name = $1 AND column_name = $2`,
			tc.table, tc.column).Scan(&n); err != nil || n != 1 {
			t.Fatalf("column %s.%s: n=%d err=%v", tc.table, tc.column, n, err)
		}
	}

	now := time.Now().Unix()
	for i, inRange := range []bool{false, true} {
		id, err := st.InsertSatlinkProbe(ctx, &types.SatlinkProbeResult{
			Timestamp: now - int64(10-i), EgressIP: "192.0.2.10", ASN: "AS64500",
			City: "Example City", Country: "ZZ", InSatlinkRange: inRange,
		})
		if err != nil {
			t.Fatalf("InsertSatlinkProbe: %v", err)
		}
		if err := st.InsertSatlinkMeasurement(ctx, id, &types.SatlinkMeasurement{
			Target: "relay-a", TargetIP: "198.51.100.1", PingAvg: 42,
		}); err != nil {
			t.Fatalf("InsertSatlinkMeasurement: %v", err)
		}
	}

	latest, err := st.SatlinkLatest(ctx)
	if err != nil || !latest.InSatlinkRange || len(latest.Measurements) != 1 {
		t.Fatalf("SatlinkLatest: %+v err=%v", latest, err)
	}
	prev, err := st.SatlinkPrevious(ctx)
	if err != nil || prev.InSatlinkRange {
		t.Fatalf("SatlinkPrevious: %+v err=%v", prev, err)
	}
	pts, err := st.SatlinkTrend(ctx, now-100, now)
	if err != nil || len(pts) != 2 {
		t.Fatalf("SatlinkTrend: %d points err=%v", len(pts), err)
	}
	pops, err := st.SatlinkPops(ctx, now-100, now)
	if err != nil || len(pops) != 2 || !pops[1].InSatlinkRange {
		t.Fatalf("SatlinkPops: %+v err=%v", pops, err)
	}
}
