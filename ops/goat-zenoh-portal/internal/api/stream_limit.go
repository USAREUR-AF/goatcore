package api

import "sync"

// streamSlots counts live /api/topics/stream connections per non-operator
// principal. Each stream polls the router's REST plugin once a second for as
// long as it stays open, and the proxy puts no limiter on the stream router,
// so this count is what keeps one key from multiplying that load.
//
// The zero value is ready to use.
type streamSlots struct {
	mu   sync.Mutex
	open map[string]int
}

// acquire takes a stream slot for p, returning a release func and true, or
// false when p already holds max streams. Operators are exempt: they are
// trusted, and every operator on the legacy shared key would otherwise share
// one budget. max <= 0 disables the cap.
func (ss *streamSlots) acquire(p *Principal, max int) (release func(), ok bool) {
	if max <= 0 || p == nil || p.Operator || p.Vendor == "" {
		return func() {}, true
	}
	key := p.Vendor
	ss.mu.Lock()
	defer ss.mu.Unlock()
	if ss.open == nil {
		ss.open = map[string]int{}
	}
	if ss.open[key] >= max {
		return nil, false
	}
	ss.open[key]++
	var once sync.Once
	return func() {
		once.Do(func() {
			ss.mu.Lock()
			defer ss.mu.Unlock()
			if ss.open[key]--; ss.open[key] <= 0 {
				delete(ss.open, key)
			}
		})
	}, true
}

// count reports p's open streams (tests and diagnostics).
func (ss *streamSlots) count(vendor string) int {
	ss.mu.Lock()
	defer ss.mu.Unlock()
	return ss.open[vendor]
}
