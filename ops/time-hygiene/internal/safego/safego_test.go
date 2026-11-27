package safego

import (
	"sync"
	"sync/atomic"
	"testing"
	"time"

	dto "github.com/prometheus/client_model/go"
)

// counterValue reads the current value of the
// time_hygiene_panic_total{service=svc,goroutine=name} counter.
func counterValue(t *testing.T, svc, name string) float64 {
	t.Helper()
	c, err := PanicTotal.GetMetricWithLabelValues(svc, name)
	if err != nil {
		t.Fatalf("GetMetricWithLabelValues: %v", err)
	}
	var m dto.Metric
	if err := c.Write(&m); err != nil {
		t.Fatalf("Write: %v", err)
	}
	return m.GetCounter().GetValue()
}

func TestSpawnRecoversPanicAndIncrementsCounter(t *testing.T) {
	SetService("other-service")
	t.Cleanup(func() { SetService("unknown") })

	const name = "test-goroutine"
	before := counterValue(t, "other-service", name)

	done := make(chan struct{})
	Spawn(name, func() {
		defer close(done)
		panic("synthetic boom")
	})
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("goroutine did not complete within 2s")
	}

	// Tiny sleep to let the deferred recover finish its slog.Error +
	// counter increment before we read the counter.
	time.Sleep(10 * time.Millisecond)

	after := counterValue(t, "other-service", name)
	if delta := after - before; delta != 1 {
		t.Errorf("panic counter delta = %v, want 1", delta)
	}
}

func TestSpawnNoPanicDoesNotIncrement(t *testing.T) {
	SetService("other-service")
	t.Cleanup(func() { SetService("unknown") })

	const name = "happy-goroutine"
	before := counterValue(t, "other-service", name)

	var ran atomic.Bool
	done := make(chan struct{})
	Spawn(name, func() {
		ran.Store(true)
		close(done)
	})
	<-done

	if !ran.Load() {
		t.Fatal("Spawn'ed function did not execute")
	}

	after := counterValue(t, "other-service", name)
	if delta := after - before; delta != 0 {
		t.Errorf("panic counter delta = %v on no-panic path, want 0", delta)
	}
}

func TestSpawnDoesNotKillProcess(t *testing.T) {
	// Ten panics in parallel; we must reach the assertion below.
	SetService("other-service")
	t.Cleanup(func() { SetService("unknown") })

	var wg sync.WaitGroup
	for i := 0; i < 10; i++ {
		wg.Add(1)
		Spawn("parallel-panic", func() {
			defer wg.Done()
			panic("parallel boom")
		})
	}
	wg.Wait()
	// If we got here, panics were recovered and the process is alive.
}

func TestRecoverDirectUsage(t *testing.T) {
	// Recover() is exported for callers who already own their go-statement.
	SetService("ntp-health-agent")
	t.Cleanup(func() { SetService("unknown") })

	const name = "explicit-recover"
	before := counterValue(t, "ntp-health-agent", name)

	done := make(chan struct{})
	go func() {
		defer close(done)
		defer Recover(name)
		panic("explicit boom")
	}()
	<-done
	time.Sleep(10 * time.Millisecond)

	after := counterValue(t, "ntp-health-agent", name)
	if delta := after - before; delta != 1 {
		t.Errorf("Recover counter delta = %v, want 1", delta)
	}
}

func TestSetServiceLabelsCorrectService(t *testing.T) {
	SetService("ntp-health-agent")
	t.Cleanup(func() { SetService("unknown") })

	const name = "service-label-test"
	beforeAgent := counterValue(t, "ntp-health-agent", name)
	beforeSidecar := counterValue(t, "other-service", name)

	done := make(chan struct{})
	Spawn(name, func() {
		defer close(done)
		panic("label boom")
	})
	<-done
	time.Sleep(10 * time.Millisecond)

	afterAgent := counterValue(t, "ntp-health-agent", name)
	afterSidecar := counterValue(t, "other-service", name)

	if d := afterAgent - beforeAgent; d != 1 {
		t.Errorf("agent counter delta = %v, want 1", d)
	}
	if d := afterSidecar - beforeSidecar; d != 0 {
		t.Errorf("sidecar counter delta = %v, want 0 (service was ntp-health-agent)", d)
	}
}
