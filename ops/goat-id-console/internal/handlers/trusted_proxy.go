package handlers

import (
	"bufio"
	"context"
	"encoding/binary"
	"encoding/hex"
	"log/slog"
	"net"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"
)

// trustedProxyTTL is how long a resolved sidecar address is reused before the
// name is looked up again. Docker's embedded DNS is local and fast, and Docker
// hands a freed address to the next container started, so the window in which
// a recreated sidecar's old address is still trusted is kept short.
const trustedProxyTTL = time.Second

// trustedProxyKeepGood is how long after its last successful lookup a name's
// addresses are still trusted while lookups of it fail (the remove-create gap
// of a sidecar recreate). After it the name contributes nothing.
const trustedProxyKeepGood = 3 * time.Second

// trustedProxyMinRefresh rate-limits lookups: at most one refresh starts per
// interval, whether it is due to expiry or to a peer not in the set (a miss).
const trustedProxyMinRefresh = 250 * time.Millisecond

// trustedProxyLookupTimeout bounds one resolution of Cfg.TrustedProxyHosts, so
// a hung resolver delays a request by at most this long and then fails closed.
var trustedProxyLookupTimeout = 2 * time.Second

// proxyNow is the clock, replaceable in tests.
var proxyNow = time.Now

// proxyName is one configured name's resolved addresses.
type proxyName struct {
	addrs      map[string]bool
	validUntil time.Time // trusted until then
	goodAt     time.Time // last lookup of the name that succeeded
}

type trustedProxySet struct {
	mu          sync.Mutex // guards the fields below; never held across a lookup
	names       map[string]*proxyName
	lastAttempt time.Time
	refreshing  bool

	netsOnce sync.Once
	nets     []*net.IPNet    // this container's own (non-loopback) networks
	gateways map[string]bool // never trusted: each network's gateway
}

// lookupHost is the resolver, replaceable in tests.
var lookupHost = net.DefaultResolver.LookupHost

// localNetworks lists the networks of this container's interfaces, replaceable
// in tests.
var localNetworks = func() ([]*net.IPNet, error) {
	addrs, err := net.InterfaceAddrs()
	if err != nil {
		return nil, err
	}
	var out []*net.IPNet
	for _, a := range addrs {
		n, ok := a.(*net.IPNet)
		if !ok || n.IP.IsLoopback() || n.IP.IsLinkLocalUnicast() {
			continue
		}
		out = append(out, &net.IPNet{IP: n.IP.Mask(n.Mask), Mask: n.Mask})
	}
	return out, nil
}

// defaultGateways lists this container's IPv4 route gateways from
// /proc/net/route, replaceable in tests. Missing file: none.
var defaultGateways = func() []net.IP {
	f, err := os.Open("/proc/net/route")
	if err != nil {
		return nil
	}
	defer f.Close()
	var out []net.IP
	sc := bufio.NewScanner(f)
	sc.Scan() // header
	for sc.Scan() {
		fields := strings.Fields(sc.Text())
		if len(fields) < 3 {
			continue
		}
		b, err := hex.DecodeString(fields[2])
		if err != nil || len(b) != 4 {
			continue
		}
		v := binary.LittleEndian.Uint32(b)
		if v == 0 {
			continue
		}
		ip := make(net.IP, 4)
		binary.BigEndian.PutUint32(ip, v)
		out = append(out, ip)
	}
	return out
}

// firstHost is the first address of n (its network address plus one), which
// Docker gives the bridge gateway by default.
func firstHost(n *net.IPNet) net.IP {
	ip := make(net.IP, len(n.IP))
	copy(ip, n.IP)
	for i := len(ip) - 1; i >= 0; i-- {
		ip[i]++
		if ip[i] != 0 {
			break
		}
	}
	return ip
}

// localNets reads this container's networks and their gateways once
// (NewServer calls it at startup). A resolved trusted-proxy address must fall
// inside one of the networks: the sidecar shares the console's Docker
// network. A gateway address is never trusted: it is the source address of
// every host process reaching the console through a published port.
func (t *trustedProxySet) localNets() ([]*net.IPNet, map[string]bool) {
	t.netsOnce.Do(func() {
		t.gateways = map[string]bool{}
		nets, err := localNetworks()
		if err != nil || len(nets) == 0 {
			slog.Error("trusted proxy: cannot read this container's networks; addresses resolved from TRUSTED_PROXY_HOSTS names are refused",
				slog.Any("err", err))
		}
		ranges := make([]string, 0, len(nets))
		for _, n := range nets {
			ranges = append(ranges, n.String())
			t.gateways[firstHost(n).String()] = true
		}
		for _, g := range defaultGateways() {
			t.gateways[g.String()] = true
		}
		gws := make([]string, 0, len(t.gateways))
		for g := range t.gateways {
			gws = append(gws, g)
		}
		if len(nets) > 0 {
			slog.Info("trusted proxy: names must resolve inside this container's networks; gateways are never trusted",
				slog.String("networks", strings.Join(ranges, ",")), slog.String("gateways", strings.Join(gws, ",")))
		}
		t.nets = nets
	})
	return t.nets, t.gateways
}

// fromTrustedProxy reports whether the request's peer is one of
// Cfg.TrustedProxyHosts. With none configured every peer is trusted (tests,
// dev bypass), unless Load() found TRUSTED_PROXY_HOSTS set to a value naming
// no host (Cfg.TrustNoProxy), which trusts no peer. DevBypassAuth trusts every
// peer.
func (s *Server) fromTrustedProxy(r *http.Request) bool {
	if s.Cfg.DevBypassAuth {
		return true
	}
	if s.Cfg.TrustNoProxy {
		return false
	}
	if len(s.Cfg.TrustedProxyHosts) == 0 {
		return true
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		host = r.RemoteAddr
	}
	ip := net.ParseIP(host)
	if ip == nil {
		return false
	}
	peer := ip.String()
	_, gateways := s.trustedProxies.localNets()
	if gateways[peer] {
		return false
	}
	if s.trustedProxyAddrs(r.Context(), "")[peer] {
		return true
	}
	// A miss may be a recreated sidecar at a new address: re-resolve once
	// (rate-limited) before refusing.
	return s.trustedProxyAddrs(r.Context(), peer)[peer]
}

// trustedProxyAddrs returns the trusted set. It refreshes the names first
// when one is due (or, for a miss, when missPeer is set) and no refresh
// started within trustedProxyMinRefresh.
func (s *Server) trustedProxyAddrs(ctx context.Context, missPeer string) map[string]bool {
	t := &s.trustedProxies
	t.mu.Lock()
	now := proxyNow()
	due := missPeer != ""
	if !due {
		for _, h := range s.Cfg.TrustedProxyHosts {
			if net.ParseIP(h) != nil {
				continue
			}
			if e := t.names[h]; e == nil || !now.Before(e.validUntil) {
				due = true
				break
			}
		}
	}
	if due && !t.refreshing && now.Sub(t.lastAttempt) >= trustedProxyMinRefresh {
		t.refreshing, t.lastAttempt = true, now
		t.mu.Unlock()
		results := s.resolveTrustedProxies(ctx)
		t.mu.Lock()
		t.store(results, proxyNow())
		t.refreshing = false
	}
	set := s.currentSetLocked(proxyNow())
	t.mu.Unlock()
	return set
}

// lookupResult is one name's lookup: its in-network addresses, or failed.
type lookupResult struct {
	addrs  map[string]bool
	failed bool
}

// store records per-name results. A name that resolved replaces its
// addresses; a name whose lookup failed keeps its last good addresses until
// trustedProxyKeepGood after that lookup, then contributes nothing.
func (t *trustedProxySet) store(results map[string]lookupResult, now time.Time) {
	if t.names == nil {
		t.names = map[string]*proxyName{}
	}
	for h, r := range results {
		if !r.failed {
			t.names[h] = &proxyName{addrs: r.addrs, validUntil: now.Add(trustedProxyTTL), goodAt: now}
			continue
		}
		e := t.names[h]
		if e == nil || !now.Before(e.goodAt.Add(trustedProxyKeepGood)) {
			delete(t.names, h)
			slog.Warn("trusted proxy: lookup failed and no recent good addresses; name trusts nobody", slog.String("name", h))
			continue
		}
		e.validUntil = e.goodAt.Add(trustedProxyKeepGood)
		slog.Warn("trusted proxy: lookup failed; keeping the last good addresses briefly",
			slog.String("name", h), slog.Time("resolved_at", e.goodAt))
	}
}

// currentSetLocked is the union of IP literals and every name's addresses
// that are still valid, minus gateways. t.mu is held.
func (s *Server) currentSetLocked(now time.Time) map[string]bool {
	t := &s.trustedProxies
	set := map[string]bool{}
	for _, h := range s.Cfg.TrustedProxyHosts {
		if ip := net.ParseIP(h); ip != nil {
			if !t.gateways[ip.String()] {
				set[ip.String()] = true
			}
			continue
		}
		if e := t.names[h]; e != nil && now.Before(e.validUntil) {
			for a := range e.addrs {
				set[a] = true
			}
		}
	}
	return set
}

// resolveTrustedProxies resolves each name in Cfg.TrustedProxyHosts on its
// own. A name is resolved as rooted ("name."), so the resolver never tries it
// against the inherited DNS search list, and an answer outside this
// container's networks, or a gateway address, is refused and logged.
func (s *Server) resolveTrustedProxies(ctx context.Context) map[string]lookupResult {
	// The deadline is detached from the request: a client that hangs up must
	// not cut the lookup short for everyone.
	ctx, cancel := context.WithTimeout(context.WithoutCancel(ctx), trustedProxyLookupTimeout)
	defer cancel()
	nets, gateways := s.trustedProxies.localNets()
	out := map[string]lookupResult{}
	for _, h := range s.Cfg.TrustedProxyHosts {
		if net.ParseIP(h) != nil {
			continue
		}
		name := h
		if !strings.HasSuffix(name, ".") {
			name += "."
		}
		ips, err := lookupHost(ctx, name)
		if err != nil {
			out[h] = lookupResult{failed: true}
			continue
		}
		addrs := map[string]bool{}
		for _, a := range ips {
			ip := net.ParseIP(a)
			if ip == nil {
				continue
			}
			switch {
			case gateways[ip.String()]:
				slog.Error("trusted proxy: name resolved to a network gateway; address refused",
					slog.String("name", h), slog.String("addr", ip.String()))
			case !inNets(ip, nets):
				slog.Error("trusted proxy: name resolved outside this container's networks; address refused",
					slog.String("name", h), slog.String("addr", ip.String()))
			default:
				addrs[ip.String()] = true
			}
		}
		out[h] = lookupResult{addrs: addrs}
	}
	return out
}

func inNets(ip net.IP, nets []*net.IPNet) bool {
	for _, n := range nets {
		if n.Contains(ip) {
			return true
		}
	}
	return false
}
