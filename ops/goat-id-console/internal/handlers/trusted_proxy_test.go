package handlers

import (
	"bytes"
	"context"
	"encoding/binary"
	"errors"
	"log/slog"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
)

func gateGetFrom(t *testing.T, s *Server, remote string) int {
	t.Helper()
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/users/x?idp=tenant", nil)
	req.RemoteAddr = remote
	req.Header.Set("X-Forwarded-Email", "forged@example.org")
	req.Header.Set("X-Forwarded-Groups", "PLATFORM-ADMIN")
	rec := httptest.NewRecorder()
	s.Routes().ServeHTTP(rec, req)
	return rec.Code
}

// Identity headers count only on a request from the oauth2-proxy sidecar; the
// same headers from any other peer carry no identity and pass no admin gate.
func TestActorFromHeaders_onlyFromTrustedProxy(t *testing.T) {
	kc, _ := fakeKratos(t, "id-1", "x@example.org", "X", nil, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.TrustedProxyHosts = []string{"oauth2-proxy-id-console"}
	stubLocalNets(t, "172.20.0.0/16")
	saved := lookupHost
	lookupHost = func(_ context.Context, h string) ([]string, error) {
		if h == "oauth2-proxy-id-console." {
			return []string{"172.20.0.7"}, nil
		}
		return nil, &lookupErr{}
	}
	t.Cleanup(func() { lookupHost = saved })

	if code := gateGetFrom(t, s, "172.20.0.9:40000"); code != http.StatusForbidden {
		t.Fatalf("forged headers from another container: status %d, want 403", code)
	}
	if code := gateGetFrom(t, s, "127.0.0.1:40000"); code != http.StatusForbidden {
		t.Fatalf("forged headers over the loopback publish: status %d, want 403", code)
	}
	if code := gateGetFrom(t, s, "172.20.0.7:40000"); code == http.StatusForbidden {
		t.Fatalf("headers from the sidecar refused: status %d", code)
	}
}

type lookupErr struct{}

func (*lookupErr) Error() string { return "no such host" }

// TRUSTED_PROXY_HOSTS set to a value naming no host (Cfg.TrustNoProxy) trusts
// no peer, not every peer: forged headers are refused even from an address
// that would otherwise be the sidecar's.
func TestActorFromHeaders_trustNoProxyFailsClosed(t *testing.T) {
	kc, _ := fakeKratos(t, "id-1", "x@example.org", "X", nil, "active")
	s := newUsersAPITestServer(t, kc)
	s.Cfg.TrustedProxyHosts = nil
	s.Cfg.TrustNoProxy = true
	saved := lookupHost
	lookupHost = func(context.Context, string) ([]string, error) {
		t.Error("resolver called with TrustNoProxy set")
		return nil, &lookupErr{}
	}
	t.Cleanup(func() { lookupHost = saved })

	for _, peer := range []string{"172.20.0.7:40000", "172.20.0.9:40000", "127.0.0.1:40000"} {
		if code := gateGetFrom(t, s, peer); code != http.StatusForbidden {
			t.Fatalf("forged headers from %s with TrustNoProxy: status %d, want 403", peer, code)
		}
	}
}

// A hung resolver delays a request by at most trustedProxyLookupTimeout, the
// request then fails closed, and the cache mutex is not held while the lookup
// runs.
func TestTrustedProxyAddrs_lookupDeadlineNoLockHeld(t *testing.T) {
	s := &Server{Cfg: &config.Config{TrustedProxyHosts: []string{"oauth2-proxy-id-console"}}}
	savedLookup, savedTimeout := lookupHost, trustedProxyLookupTimeout
	t.Cleanup(func() { lookupHost, trustedProxyLookupTimeout = savedLookup, savedTimeout })
	trustedProxyLookupTimeout = 200 * time.Millisecond
	started := make(chan struct{})
	lookupHost = func(ctx context.Context, _ string) ([]string, error) {
		close(started)
		<-ctx.Done() // a resolver that never answers
		return nil, ctx.Err()
	}

	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/", nil)
	req.RemoteAddr = "172.20.0.7:40000"
	done := make(chan bool, 1)
	begin := time.Now()
	go func() { done <- s.fromTrustedProxy(req) }()

	<-started
	if !s.trustedProxies.mu.TryLock() {
		t.Fatal("cache mutex held while the lookup runs")
	}
	s.trustedProxies.mu.Unlock()

	select {
	case ok := <-done:
		if ok {
			t.Fatal("peer trusted although the lookup timed out")
		}
		if d := time.Since(begin); d > 2*time.Second {
			t.Fatalf("request waited %v on a hung resolver", d)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("lookup has no deadline: request still blocked after 5s")
	}
}

// A client hanging up does not cut the lookup short (which would cache an
// empty, deny-all set for every other request).
func TestTrustedProxyAddrs_lookupDetachedFromRequest(t *testing.T) {
	s := &Server{Cfg: &config.Config{TrustedProxyHosts: []string{"oauth2-proxy-id-console"}}}
	stubLocalNets(t, "172.20.0.0/16")
	saved := lookupHost
	t.Cleanup(func() { lookupHost = saved })
	lookupHost = func(ctx context.Context, _ string) ([]string, error) {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		return []string{"172.20.0.7"}, nil
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	req := httptest.NewRequestWithContext(ctx, http.MethodGet, "/", nil)
	req.RemoteAddr = "172.20.0.7:40000"
	if !s.fromTrustedProxy(req) {
		t.Fatal("sidecar refused: a canceled request context cut the lookup short")
	}
}

func stubLocalNets(t *testing.T, cidrs ...string) {
	t.Helper()
	saved := localNetworks
	t.Cleanup(func() { localNetworks = saved })
	localNetworks = func() ([]*net.IPNet, error) {
		var out []*net.IPNet
		for _, c := range cidrs {
			_, n, err := net.ParseCIDR(c)
			if err != nil {
				t.Fatal(err)
			}
			out = append(out, n)
		}
		return out, nil
	}
}

func fromPeer(s *Server, peer string) bool {
	req := httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/", nil)
	req.RemoteAddr = peer
	return s.fromTrustedProxy(req)
}

// stubDNS is an in-process DNS server for a pure-Go net.Resolver: it records
// every question and answers A records from answers, NXDOMAIN for any other
// name.
type stubDNS struct {
	mu      sync.Mutex
	asked   []string
	answers map[string]net.IP // rooted name -> IPv4
}

func (d *stubDNS) resolver() *net.Resolver {
	return &net.Resolver{PreferGo: true, Dial: func(context.Context, string, string) (net.Conn, error) {
		a, b := net.Pipe()
		go d.serve(b)
		return a, nil
	}}
}

func (d *stubDNS) serve(c net.Conn) {
	defer c.Close()
	buf := make([]byte, 4096)
	n, err := c.Read(buf)
	if err != nil || n < 14 {
		return
	}
	q := buf[2:n] // stream conn: 2-byte length prefix
	var labels []string
	i := 12
	for i < len(q) && q[i] != 0 {
		l := int(q[i])
		labels = append(labels, string(q[i+1:i+1+l]))
		i += 1 + l
	}
	name := strings.Join(labels, ".") + "."
	qend := i + 1 + 4
	qtype := binary.BigEndian.Uint16(q[qend-4:])
	d.mu.Lock()
	d.asked = append(d.asked, name)
	ip, known := d.answers[name]
	d.mu.Unlock()
	resp := append([]byte{}, q[:qend]...)
	switch {
	case known && qtype == 1:
		binary.BigEndian.PutUint16(resp[2:], 0x8180)
		binary.BigEndian.PutUint16(resp[6:], 1)
		resp = append(resp, 0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 30, 0, 4)
		resp = append(resp, ip.To4()...)
	case known:
		binary.BigEndian.PutUint16(resp[2:], 0x8180) // NOERROR, no AAAA
		binary.BigEndian.PutUint16(resp[6:], 0)
	default:
		binary.BigEndian.PutUint16(resp[2:], 0x8183) // NXDOMAIN
		binary.BigEndian.PutUint16(resp[6:], 0)
	}
	out := make([]byte, 2+len(resp))
	binary.BigEndian.PutUint16(out, uint16(len(resp)))
	copy(out[2:], resp)
	_, _ = c.Write(out)
}

// The configured name is resolved rooted, so the inherited DNS search list
// (netbird.selfhosted on a mesh host, where a peer picks its own hostname) is
// never consulted. Through Go's real pure resolver against a stub DNS that
// answers only the search-list form with a mesh address, that address must
// not become the trusted proxy, and no search-list name may be asked. On a
// host whose resolv.conf has a search list (the mesh hosts) the unrooted
// lookup asks the search form and trusts 100.64.9.9.
func TestTrustedProxy_searchListNeverConsulted(t *testing.T) {
	// The mesh range is made local here so that only the rooting is tested.
	stubLocalNets(t, "172.20.0.0/16", "100.64.0.0/10")
	saved := lookupHost
	t.Cleanup(func() { lookupHost = saved })

	var searchForms []string
	if rc, err := os.ReadFile("/etc/resolv.conf"); err == nil {
		for _, l := range strings.Split(string(rc), "\n") {
			if f := strings.Fields(l); len(f) > 1 && (f[0] == "search" || f[0] == "domain") {
				for _, dom := range f[1:] {
					searchForms = append(searchForms, "oauth2-proxy-id-console."+strings.TrimSuffix(dom, ".")+".")
				}
			}
		}
	}
	if len(searchForms) == 0 {
		searchForms = []string{"oauth2-proxy-id-console.netbird.selfhosted."}
		t.Log("this host's resolv.conf has no search list; the stub answer is never reachable either way")
	}

	// Sidecar absent: the bare name is NXDOMAIN, each search form answers.
	d := &stubDNS{answers: map[string]net.IP{}}
	for _, f := range searchForms {
		d.answers[f] = net.ParseIP("100.64.9.9")
	}
	lookupHost = d.resolver().LookupHost
	s := &Server{Cfg: &config.Config{TrustedProxyHosts: []string{"oauth2-proxy-id-console"}}}
	if fromPeer(s, "100.64.9.9:40000") {
		t.Fatal("a search-list answer (a mesh peer's chosen hostname) became the trusted proxy")
	}
	d.mu.Lock()
	for _, q := range d.asked {
		if q != "oauth2-proxy-id-console." {
			t.Errorf("resolver asked %q: the search list was consulted", q)
		}
	}
	d.mu.Unlock()

	// Sidecar present: the rooted name still resolves through the same path.
	d2 := &stubDNS{answers: map[string]net.IP{"oauth2-proxy-id-console.": net.ParseIP("172.20.0.7")}}
	lookupHost = d2.resolver().LookupHost
	s2 := &Server{Cfg: &config.Config{TrustedProxyHosts: []string{"oauth2-proxy-id-console"}}}
	if !fromPeer(s2, "172.20.0.7:40000") {
		t.Fatal("the sidecar, answered for the rooted name, was refused")
	}
}

// Names are passed to the resolver rooted exactly once; IP literals are not
// looked up.
func TestTrustedProxy_namesResolvedRooted(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	saved := lookupHost
	t.Cleanup(func() { lookupHost = saved })
	var asked []string
	lookupHost = func(_ context.Context, h string) ([]string, error) {
		asked = append(asked, h)
		return []string{"172.20.0.7"}, nil
	}
	s := &Server{Cfg: &config.Config{TrustedProxyHosts: []string{"oauth2-proxy-id-console", "already.rooted.", "172.20.0.8"}}}
	fromPeer(s, "172.20.0.7:1")
	if got := strings.Join(asked, ","); got != "oauth2-proxy-id-console.,already.rooted." {
		t.Fatalf("resolver asked %q", got)
	}
}

// A name that resolves outside this container's networks is refused and the
// refusal is logged; the same address named as an IP literal is trusted.
func TestTrustedProxy_refusesAddressOutsideLocalNets(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	saved := lookupHost
	t.Cleanup(func() { lookupHost = saved })
	lookupHost = func(context.Context, string) ([]string, error) {
		return []string{"100.64.9.9", "172.20.0.7"}, nil
	}
	var buf bytes.Buffer
	savedLog := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&buf, nil)))
	t.Cleanup(func() { slog.SetDefault(savedLog) })

	s := &Server{Cfg: &config.Config{TrustedProxyHosts: []string{"oauth2-proxy-id-console"}}}
	if fromPeer(s, "100.64.9.9:40000") {
		t.Fatal("an address outside the container's networks was trusted")
	}
	if !fromPeer(s, "172.20.0.7:40000") {
		t.Fatal("the in-network sidecar address was refused")
	}
	if !strings.Contains(buf.String(), "level=ERROR") || !strings.Contains(buf.String(), "addr=100.64.9.9") {
		t.Fatalf("refusal not logged:\n%s", buf.String())
	}
	lit := &Server{Cfg: &config.Config{TrustedProxyHosts: []string{"100.64.9.9"}}}
	if !fromPeer(lit, "100.64.9.9:40000") {
		t.Fatal("an explicitly configured IP literal was refused")
	}

	// No readable networks: every name-resolved address is refused.
	localNetworks = func() ([]*net.IPNet, error) { return nil, errors.New("no interfaces") }
	none := &Server{Cfg: &config.Config{TrustedProxyHosts: []string{"oauth2-proxy-id-console"}}}
	if fromPeer(none, "172.20.0.7:40000") {
		t.Fatal("trusted a resolved address with no known local networks")
	}
}

type fakeClock struct {
	mu sync.Mutex
	t  time.Time
}

func (c *fakeClock) now() time.Time      { c.mu.Lock(); defer c.mu.Unlock(); return c.t }
func (c *fakeClock) add(d time.Duration) { c.mu.Lock(); c.t = c.t.Add(d); c.mu.Unlock() }

func stubClock(t *testing.T) *fakeClock {
	t.Helper()
	c := &fakeClock{t: time.Unix(1_800_000_000, 0)}
	saved := proxyNow
	proxyNow = c.now
	t.Cleanup(func() { proxyNow = saved })
	return c
}

func stubGateways(t *testing.T, ips ...string) {
	t.Helper()
	saved := defaultGateways
	t.Cleanup(func() { defaultGateways = saved })
	defaultGateways = func() []net.IP {
		var out []net.IP
		for _, s := range ips {
			out = append(out, net.ParseIP(s))
		}
		return out
	}
}

// stubDNSMap is a lookupHost that answers from a mutable map (rooted names),
// failing for absent ones, and counts calls.
type stubDNSMap struct {
	mu    sync.Mutex
	m     map[string][]string
	calls int
}

func (d *stubDNSMap) set(name string, ips ...string) {
	d.mu.Lock()
	defer d.mu.Unlock()
	if ips == nil {
		delete(d.m, name)
		return
	}
	d.m[name] = ips
}

func (d *stubDNSMap) lookup(_ context.Context, h string) ([]string, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.calls++
	if ips, ok := d.m[h]; ok {
		return ips, nil
	}
	return nil, &lookupErr{}
}

func stubResolver(t *testing.T) *stubDNSMap {
	t.Helper()
	d := &stubDNSMap{m: map[string][]string{}}
	saved := lookupHost
	lookupHost = d.lookup
	t.Cleanup(func() { lookupHost = saved })
	return d
}

func proxyServer(hosts ...string) *Server {
	return &Server{Cfg: &config.Config{TrustedProxyHosts: hosts}}
}

// Recreate: the sidecar moves from A to B. A request from B is accepted at
// once (a miss triggers a re-resolve), and A is no longer trusted. (Before,
// A stayed trusted and B refused for the whole 30 s TTL.)
func TestTrustedProxy_recreateRefreshesOnMiss(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	stubGateways(t)
	c := stubClock(t)
	d := stubResolver(t)
	d.set("oauth2-proxy-id-console.", "172.20.0.2")
	s := proxyServer("oauth2-proxy-id-console")
	if !fromPeer(s, "172.20.0.2:1") {
		t.Fatal("sidecar A refused")
	}
	d.set("oauth2-proxy-id-console.", "172.20.0.5") // recreated at B
	c.add(300 * time.Millisecond)                   // within the TTL
	if !fromPeer(s, "172.20.0.5:1") {
		t.Fatal("recreated sidecar B refused within the TTL: a miss did not re-resolve")
	}
	if fromPeer(s, "172.20.0.2:1") {
		t.Fatal("old address A (handed to the next container by Docker IPAM) still trusted after the re-resolve")
	}
}

// The trusted set expires after about a second: a stale address is not
// trusted past trustedProxyTTL once the name answers differently.
func TestTrustedProxy_shortTTL(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	stubGateways(t)
	c := stubClock(t)
	d := stubResolver(t)
	d.set("oauth2-proxy-id-console.", "172.20.0.2")
	s := proxyServer("oauth2-proxy-id-console")
	fromPeer(s, "172.20.0.2:1")
	d.set("oauth2-proxy-id-console.", "172.20.0.5")
	c.add(trustedProxyTTL + 10*time.Millisecond)
	if fromPeer(s, "172.20.0.2:1") {
		t.Fatal("stale address trusted after the TTL")
	}
	if trustedProxyTTL > 2*time.Second || trustedProxyKeepGood > 5*time.Second {
		t.Fatalf("windows too wide: ttl %v keep-good %v", trustedProxyTTL, trustedProxyKeepGood)
	}
}

// Sidecar absent (NXDOMAIN): its last good address is trusted only until
// trustedProxyKeepGood after the last good lookup (before: 60 s).
func TestTrustedProxy_absentWindowIsSeconds(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	stubGateways(t)
	c := stubClock(t)
	d := stubResolver(t)
	d.set("oauth2-proxy-id-console.", "172.20.0.2")
	s := proxyServer("oauth2-proxy-id-console")
	if !fromPeer(s, "172.20.0.2:1") {
		t.Fatal("sidecar refused")
	}
	d.set("oauth2-proxy-id-console.") // removed
	c.add(trustedProxyTTL + 10*time.Millisecond)
	if !fromPeer(s, "172.20.0.2:1") {
		t.Fatal("last good address dropped on the first failed lookup")
	}
	c.add(4 * time.Second) // 5 s after the last good lookup
	if fromPeer(s, "172.20.0.2:1") {
		t.Fatalf("address of an absent sidecar trusted more than %v after its last good lookup", trustedProxyKeepGood)
	}
	fresh := proxyServer("oauth2-proxy-id-console")
	if fromPeer(fresh, "172.20.0.2:1") {
		t.Fatal("trusted a peer although no lookup ever succeeded")
	}
}

// An answer that is a network gateway (the .1 of a local subnet, or a route
// gateway) is never trusted: it is the source of every host process through a
// published port. A gateway named as an IP literal is refused too.
func TestTrustedProxy_gatewayNeverTrusted(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	stubGateways(t, "172.20.0.254")
	stubClock(t)
	d := stubResolver(t)
	d.set("oauth2-proxy-id-console.", "172.20.0.1", "172.20.0.254", "172.20.0.7")
	s := proxyServer("oauth2-proxy-id-console")
	for _, gw := range []string{"172.20.0.1:1", "172.20.0.254:1"} {
		if fromPeer(s, gw) {
			t.Fatalf("gateway peer %s trusted", gw)
		}
	}
	if !fromPeer(s, "172.20.0.7:1") {
		t.Fatal("sidecar refused")
	}
	lit := proxyServer("172.20.0.1")
	if fromPeer(lit, "172.20.0.1:1") {
		t.Fatal("gateway trusted as an IP literal")
	}
}

// A lookup that succeeds with only refused answers ends the keep-good: a
// following failure does not revive the old address.
func TestTrustedProxy_outOfNetAnswerEndsKeepGood(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	stubGateways(t)
	c := stubClock(t)
	d := stubResolver(t)
	d.set("oauth2-proxy-id-console.", "172.20.0.2")
	s := proxyServer("oauth2-proxy-id-console")
	fromPeer(s, "172.20.0.2:1")
	d.set("oauth2-proxy-id-console.", "100.64.9.9")
	c.add(trustedProxyTTL + 10*time.Millisecond)
	if fromPeer(s, "172.20.0.2:1") || fromPeer(s, "100.64.9.9:1") {
		t.Fatal("trusted after an out-of-network answer")
	}
	d.set("oauth2-proxy-id-console.")
	c.add(trustedProxyTTL + 10*time.Millisecond)
	if fromPeer(s, "172.20.0.2:1") {
		t.Fatal("a failed lookup revived the old address")
	}
}

// Results are per name: one failing name does not discard another name's
// fresh answer, and the failing name keeps its own last good addresses only
// briefly.
func TestTrustedProxy_perNameResults(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	stubGateways(t)
	c := stubClock(t)
	d := stubResolver(t)
	d.set("a.", "172.20.0.2")
	d.set("b.", "172.20.0.3")
	s := proxyServer("a", "b")
	if !fromPeer(s, "172.20.0.2:1") || !fromPeer(s, "172.20.0.3:1") {
		t.Fatal("initial addresses refused")
	}
	d.set("a.", "172.20.0.4") // a moved
	d.set("b.")               // b fails
	c.add(trustedProxyTTL + 10*time.Millisecond)
	if !fromPeer(s, "172.20.0.4:1") {
		t.Fatal("a's fresh answer discarded because b failed")
	}
	if fromPeer(s, "172.20.0.2:1") {
		t.Fatal("a's stale address still trusted")
	}
	if !fromPeer(s, "172.20.0.3:1") {
		t.Fatal("b's last good address dropped on its first failure")
	}
	c.add(4 * time.Second) // 5 s after b's last good lookup
	if fromPeer(s, "172.20.0.3:1") {
		t.Fatal("b's address trusted past the keep-good window")
	}
}

// Misses are rate-limited: a stream of requests from an unknown peer starts
// at most one lookup per trustedProxyMinRefresh.
func TestTrustedProxy_missRefreshRateLimited(t *testing.T) {
	stubLocalNets(t, "172.20.0.0/16")
	stubGateways(t)
	c := stubClock(t)
	d := stubResolver(t)
	d.set("oauth2-proxy-id-console.", "172.20.0.2")
	s := proxyServer("oauth2-proxy-id-console")
	fromPeer(s, "172.20.0.2:1")
	base := d.calls
	for i := 0; i < 50; i++ {
		if fromPeer(s, "172.20.0.9:1") {
			t.Fatal("unknown peer trusted")
		}
	}
	if n := d.calls - base; n != 0 {
		t.Fatalf("%d lookups within the rate-limit interval, want 0", n)
	}
	c.add(trustedProxyMinRefresh)
	for i := 0; i < 50; i++ {
		fromPeer(s, "172.20.0.9:1")
	}
	if n := d.calls - base; n != 1 {
		t.Fatalf("%d lookups after one interval, want 1", n)
	}
}
