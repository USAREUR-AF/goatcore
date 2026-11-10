package activity

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

// promStub serves one canned instant-query response and counts the calls, so
// the caching tests can assert how many times the upstream was actually asked.
func promStub(t *testing.T, body string, status int) (*httptest.Server, *atomic.Int64) {
	t.Helper()
	var calls atomic.Int64
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		calls.Add(1)
		if r.URL.Path != "/api/v1/query" {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(status)
		_, _ = w.Write([]byte(body))
	}))
	t.Cleanup(srv.Close)
	return srv, &calls
}

const twoOrgs = `{"status":"success","data":{"resultType":"vector","result":[
 {"metric":{"vendor":"aardwolf"},"value":[1789798547.547,"12"]},
 {"metric":{"vendor":"otter"},"value":[1789798547.547,"3"]}]}}`

func TestPrometheusCountsDeclaredPatternsPerOrg(t *testing.T) {
	srv, _ := promStub(t, twoOrgs, http.StatusOK)
	p, err := NewPrometheus(srv.URL, time.Second)
	if err != nil {
		t.Fatalf("new: %v", err)
	}
	got, err := p.ForOrgs(context.Background(), []string{"aardwolf", "otter", "never-published"})
	if err != nil {
		t.Fatalf("for orgs: %v", err)
	}
	if !got["aardwolf"].Available || got["aardwolf"].DistinctTopics != 12 {
		t.Errorf("aardwolf = %+v, want 12 available", got["aardwolf"])
	}
	if got["otter"].DistinctTopics != 3 {
		t.Errorf("otter = %+v, want 3", got["otter"])
	}
	// An org the metrics do not mention has published nothing. That is a real
	// answer, not an unavailable one — the distinction the old per-topic table
	// could not make.
	nv := got["never-published"]
	if !nv.Available || nv.DistinctTopics != 0 {
		t.Errorf("never-published = %+v, want available with zero", nv)
	}
}

func TestPrometheusUnreachableReturnsErrorNotZeros(t *testing.T) {
	// A port nothing listens on: the failure an operator will actually hit
	// when the obs stack is down.
	p, err := NewPrometheus("http://127.0.0.1:1", 200*time.Millisecond)
	if err != nil {
		t.Fatalf("new: %v", err)
	}
	got, err := p.ForOrgs(context.Background(), []string{"aardwolf"})
	if err == nil {
		t.Fatal("unreachable prometheus returned no error; the caller cannot tell a failure from an empty fleet")
	}
	// And critically, no fabricated summaries: a map of zeros here would reach
	// the swimlane as "this org has published to zero topics", which is a
	// different and wrong statement.
	if got != nil {
		t.Errorf("failure returned %d summaries, want none", len(got))
	}
}

func TestPrometheusQueryErrorReturnsErrorWithReason(t *testing.T) {
	srv, _ := promStub(t, `{"status":"error","error":"parse error"}`, http.StatusOK)
	p, _ := NewPrometheus(srv.URL, time.Second)
	_, err := p.ForOrgs(context.Background(), []string{"aardwolf"})
	if err == nil {
		t.Fatal("a failed query returned no error")
	}
	// The operator has to be able to act on it, so the upstream's own words
	// survive into the message.
	if !strings.Contains(err.Error(), "parse error") {
		t.Errorf("error %q does not carry the upstream reason", err)
	}
}

func TestPrometheusRejectsUnusableURLs(t *testing.T) {
	for _, bad := range []string{"", "not-a-url", "ftp://host:9090", "//host:9090"} {
		if _, err := NewPrometheus(bad, time.Second); err == nil {
			t.Errorf("NewPrometheus(%q) accepted an unusable url", bad)
		}
	}
}

func TestDisabledAnswersUnavailableForEveryOrg(t *testing.T) {
	got, err := Disabled{}.ForOrgs(context.Background(), []string{"a", "b"})
	if err != nil {
		t.Fatalf("disabled must not error: %v", err)
	}
	for _, p := range []string{"a", "b"} {
		if got[p].Available {
			t.Errorf("%s reported available from the disabled source", p)
		}
		if got[p].Reason == "" {
			t.Errorf("%s has no reason; the surface cannot explain the blank", p)
		}
	}
}

func TestCachedAsksUpstreamOncePerTTL(t *testing.T) {
	srv, calls := promStub(t, twoOrgs, http.StatusOK)
	p, _ := NewPrometheus(srv.URL, time.Second)
	c := NewCached(p, time.Minute)
	for i := 0; i < 5; i++ {
		if _, err := c.ForOrgs(context.Background(), []string{"aardwolf"}); err != nil {
			t.Fatalf("call %d: %v", i, err)
		}
	}
	if n := calls.Load(); n != 1 {
		t.Fatalf("upstream asked %d times for 5 reads inside one ttl, want 1", n)
	}
}

func TestCachedServesStaleRatherThanNothing(t *testing.T) {
	var fail atomic.Bool
	var calls atomic.Int64
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		calls.Add(1)
		if fail.Load() {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(twoOrgs))
	}))
	defer srv.Close()

	p, _ := NewPrometheus(srv.URL, time.Second)
	c := NewCached(p, time.Millisecond)
	if _, err := c.ForOrgs(context.Background(), []string{"aardwolf"}); err != nil {
		t.Fatalf("warm: %v", err)
	}
	fail.Store(true)
	time.Sleep(5 * time.Millisecond) // let the entry go stale

	got, err := c.ForOrgs(context.Background(), []string{"aardwolf"})
	if err != nil {
		t.Fatalf("a failing refresh must not drop a usable answer: %v", err)
	}
	// An activity count minutes out of date beats a blank column; the number
	// is a reading, not a fact of record.
	if !got["aardwolf"].Available || got["aardwolf"].DistinctTopics != 12 {
		t.Errorf("stale answer = %+v, want the previous 12", got["aardwolf"])
	}
}

func TestCachedRespectsCallerDeadline(t *testing.T) {
	// An upstream that never answers must not hold a caller past its own
	// deadline: the swimlane's other columns are registry-owned and fine.
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		<-r.Context().Done()
	}))
	defer srv.Close()
	p, _ := NewPrometheus(srv.URL, 5*time.Second)
	c := NewCached(p, time.Minute)

	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Millisecond)
	defer cancel()
	start := time.Now()
	got, _ := c.ForOrgs(ctx, []string{"aardwolf"})
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Fatalf("held the caller %v past a 150ms deadline", elapsed)
	}
	if got["aardwolf"].Available {
		t.Error("a timed-out query reported Available=true")
	}
}

func TestSampleValueParsesCounterStrings(t *testing.T) {
	// Prometheus sends the value as a STRING. Decoding it as a float would
	// silently round a large counter, so it is parsed explicitly.
	for _, tc := range []struct {
		in   string
		want int64
		ok   bool
	}{
		{`[1.0,"224"]`, 224, true},
		{`[1.0,"1.7e4"]`, 17000, true},
		// NaN and Inf are legal Prometheus values and converting either to an
		// integer is undefined in Go — 0 on arm64, minimum-int64 on amd64. The
		// first version of this test asserted the arm64 answer and CI caught
		// it. Both are rejected, so the field reads unknown on either.
		{`[1.0,"NaN"]`, 0, false},
		{`[1.0,"+Inf"]`, 0, false},
		{`[1.0,"-1"]`, 0, false},
		{`[1.0,"bogus"]`, 0, false},
		{`[1.0]`, 0, false},
	} {
		var v []any
		if err := json.Unmarshal([]byte(tc.in), &v); err != nil {
			t.Fatalf("fixture %s: %v", tc.in, err)
		}
		got, ok := sampleValue(v)
		if ok != tc.ok {
			t.Errorf("sampleValue(%s) ok = %v, want %v", tc.in, ok, tc.ok)
			continue
		}
		if ok && got != tc.want {
			t.Errorf("sampleValue(%s) = %d, want %d", tc.in, got, tc.want)
		}
	}
}
