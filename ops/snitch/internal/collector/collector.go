// Package collector aggregates probe reports into a mesh performance view.
//
// Dual persistence: in-memory map for real-time dashboard, PostgreSQL for
// historical analytics. Detects state changes (peer online/offline, path
// changes) and records events.
package collector

import (
	"context"
	"fmt"
	"log"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
	"github.com/usareur-af/goatcore/ops/snitch/internal/identity"
	"github.com/usareur-af/goatcore/ops/snitch/internal/store"
	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

type peerState struct {
	links       map[string]bool // target IP -> direct?
	lastSeen    time.Time
	mgmtUp      bool
	signalUp    bool
	relaysAvail int
}

// Collector receives probe reports, stores them, and maintains mesh state.
type Collector struct {
	mu     sync.RWMutex
	latest map[string]types.ProbeReport // in-memory current state, keyed by peer IP
	prev   map[string]peerState         // previous state for change detection
	store  *store.Store
	idm    *identity.Manager

	// peerSuffixes are stripped from peer FQDNs to form auto-aliases.
	peerSuffixes []string
}

// New creates a collector backed by PostgreSQL and identity encryption.
func New(s *store.Store, idm *identity.Manager) *Collector {
	return &Collector{
		latest: make(map[string]types.ProbeReport),
		prev:   make(map[string]peerState),
		store:  s,
		idm:    idm,

		peerSuffixes: PeerSuffixesFromEnv(os.Getenv("SNITCH_PEER_DOMAIN_SUFFIXES")),
	}
}

// PeerSuffixesFromEnv parses SNITCH_PEER_DOMAIN_SUFFIXES (comma-separated
// FQDN suffixes, each with its leading dot). Unset or empty yields the
// built-in compat.PeerFQDNSuffixes.
func PeerSuffixesFromEnv(v string) []string {
	var out []string
	for _, p := range strings.Split(v, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	if len(out) == 0 {
		return compat.PeerFQDNSuffixes
	}
	return out
}

// Ingest processes an incoming probe report: stores in memory, persists to PG,
// detects state changes, and records events.
func (c *Collector) Ingest(report types.ProbeReport) error {
	c.mu.Lock()
	defer c.mu.Unlock()

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	now := time.Now().Unix()
	fromIP := report.PeerIP
	fromToken := c.idm.Tokenize(fromIP)

	// Upsert reporter identity.
	encName, err := c.idm.Encrypt(report.PeerName)
	if err != nil {
		return fmt.Errorf("encrypt reporter name: %w", err)
	}
	encIP, err := c.idm.Encrypt(report.PeerIP)
	if err != nil {
		return fmt.Errorf("encrypt reporter address: %w", err)
	}
	if err := c.store.UpsertIdentity(ctx, fromToken, encName, encIP); err != nil {
		log.Printf("upsert identity %s: %v", fromToken[:8], err)
	}

	// Store in-memory for real-time dashboard.
	c.latest[fromIP] = report

	// Persist report metadata.
	fp := report.Network
	gwToken := ""
	if fp.GatewayIP != "" {
		gwToken = c.idm.Tokenize(fp.GatewayIP)
	}
	reportID, err := c.store.InsertReport(ctx, fromToken, now,
		fp.Interface, fp.InterfaceType,
		gwToken, fp.GatewayRTT, c.idm.Tokenize(fp.PublicIP),
		fp.ASN, fp.ASNOrg, fp.Subnet,
		fp.Gateway.MAC, fp.Gateway.Manufacturer, fp.Gateway.Model)
	if err != nil {
		return err
	}

	// Auto-alias gateway if fingerprint data is available and no alias exists.
	if fp.GatewayIP != "" && (fp.Gateway.Manufacturer != "" || fp.Gateway.Model != "") {
		c.autoAliasGateway(ctx, fp)
	}

	// Auto-alias reporting peer from Netbird FQDN if no alias exists.
	if report.PeerName != "" && report.PeerIP != "" {
		c.autoAliasPeer(ctx, report.PeerIP, report.PeerName)
	}

	// Persist per-peer measurements and detect events.
	newLinks := make(map[string]bool, len(report.Peers))
	networks := c.networkMap()
	for _, m := range report.Peers {
		toToken := c.idm.Tokenize(m.PeerIP)

		// Upsert target identity.
		toEncName, err := c.idm.Encrypt(m.PeerName)
		if err != nil {
			return fmt.Errorf("encrypt target name: %w", err)
		}
		toEncIP, err := c.idm.Encrypt(m.PeerIP)
		if err != nil {
			return fmt.Errorf("encrypt target address: %w", err)
		}
		if err := c.store.UpsertIdentity(ctx, toToken, toEncName, toEncIP); err != nil {
			log.Printf("upsert identity %s: %v", toToken[:8], err)
		}

		toNet := networks[m.PeerIP]
		pathType := types.InferPathType(m.Direct, m.LatencyMs, fp, toNet)

		if err := c.store.InsertMeasurement(ctx, reportID, fromToken, toToken, now,
			m.Connected, m.Direct,
			m.LatencyMs, m.P50Ms, m.P95Ms, m.JitterMs, m.LossPct,
			m.Samples, string(pathType)); err != nil {
			log.Printf("insert measurement %s->%s: %v", fromToken[:8], toToken[:8], err)
		}

		if m.Connected {
			newLinks[m.PeerIP] = m.Direct
		}
	}

	c.detectEvents(ctx, fromIP, fromToken, newLinks, now)
	c.detectNetbirdEvents(ctx, fromIP, fromToken, report.Netbird, now)
	return nil
}

// MeshState computes the current mesh topology from latest probe reports.
func (c *Collector) MeshState() types.MeshState {
	c.mu.RLock()
	defer c.mu.RUnlock()

	state := types.MeshState{
		Timestamp: time.Now().UTC(),
		Probes:    make(map[string]types.ProbeReport, len(c.latest)),
	}

	networks := make(map[string]types.NetworkFingerprint)
	for ip, r := range c.latest {
		state.Probes[ip] = r
		networks[r.PeerIP] = r.Network
	}

	seen := make(map[string]bool)
	for _, report := range c.latest {
		for _, m := range report.Peers {
			key := report.PeerIP + ">" + m.PeerIP
			if seen[key] {
				continue
			}
			seen[key] = true

			toNet := networks[m.PeerIP]
			pathType := types.InferPathType(m.Direct, m.LatencyMs, report.Network, toNet)

			state.Links = append(state.Links, types.MeshLink{
				FromID:    report.PeerID,
				FromName:  report.PeerName,
				FromIP:    report.PeerIP,
				ToID:      m.PeerID,
				ToName:    m.PeerName,
				ToIP:      m.PeerIP,
				Direct:    m.Direct,
				PathType:  pathType,
				LatencyMs: m.LatencyMs,
				JitterMs:  m.JitterMs,
				LossPct:   m.LossPct,
				LastSeen:  report.Timestamp,
			})
		}
	}

	// Gateway summaries.
	gwMap := make(map[string]*types.GatewaySummary)
	for _, report := range c.latest {
		gwIP := report.Network.GatewayIP
		if gwIP == "" {
			continue
		}
		gw, exists := gwMap[gwIP]
		if !exists {
			gw = &types.GatewaySummary{
				IP:           gwIP,
				MAC:          report.Network.Gateway.MAC,
				Manufacturer: report.Network.Gateway.Manufacturer,
				Model:        report.Network.Gateway.Model,
				Subnet:       report.Network.Subnet,
			}
			gwMap[gwIP] = gw
		}
		gw.Peers = append(gw.Peers, report.PeerIP)
		if report.Network.GatewayRTT > 0 {
			gw.AvgRTT = (gw.AvgRTT*float64(len(gw.Peers)-1) + report.Network.GatewayRTT) / float64(len(gw.Peers))
		}
		if gw.Manufacturer == "" && report.Network.Gateway.Manufacturer != "" {
			gw.Manufacturer = report.Network.Gateway.Manufacturer
		}
		if gw.Model == "" && report.Network.Gateway.Model != "" {
			gw.Model = report.Network.Gateway.Model
		}
	}
	if len(gwMap) > 0 {
		state.Gateways = make(map[string]types.GatewaySummary, len(gwMap))
		for ip, gw := range gwMap {
			state.Gateways[ip] = *gw
		}
	}

	return state
}

// ProbeCount returns the number of active probes.
func (c *Collector) ProbeCount() int {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return len(c.latest)
}

// Store returns the underlying store for analytics queries.
func (c *Collector) Store() *store.Store { return c.store }

// Identity returns the identity manager for decrypting peer names.
func (c *Collector) Identity() *identity.Manager { return c.idm }

// networkMap returns a copy of known network fingerprints keyed by peer IP.
// Caller must hold at least mu.RLock.
func (c *Collector) networkMap() map[string]types.NetworkFingerprint {
	m := make(map[string]types.NetworkFingerprint, len(c.latest))
	for _, r := range c.latest {
		m[r.PeerIP] = r.Network
	}
	return m
}

// detectEvents compares new link state with previous state and records events.
func (c *Collector) detectEvents(ctx context.Context, fromIP, fromToken string, newLinks map[string]bool, now int64) {
	prev, existed := c.prev[fromIP]

	if !existed {
		if err := c.store.InsertEvent(ctx, now, fromToken, "peer_online", "", "", "", ""); err != nil {
			log.Printf("insert peer_online %s: %v", fromToken[:8], err)
		}
	} else {
		// Check for lost links and path changes.
		for targetIP, wasDirect := range prev.links {
			targetToken := c.idm.Tokenize(targetIP)
			isDirect, stillExists := newLinks[targetIP]
			if !stillExists {
				if err := c.store.InsertEvent(ctx, now, fromToken, "link_lost", targetToken, "", "", ""); err != nil {
					log.Printf("insert link_lost %s->%s: %v", fromToken[:8], targetToken[:8], err)
				}
			} else if wasDirect && !isDirect {
				if err := c.store.InsertEvent(ctx, now, fromToken, "path_change", targetToken, "direct", "relayed", ""); err != nil {
					log.Printf("insert path_change %s->%s: %v", fromToken[:8], targetToken[:8], err)
				}
			} else if !wasDirect && isDirect {
				if err := c.store.InsertEvent(ctx, now, fromToken, "path_change", targetToken, "relayed", "direct", ""); err != nil {
					log.Printf("insert path_change %s->%s: %v", fromToken[:8], targetToken[:8], err)
				}
			}
		}
		// Check for new links.
		for targetIP := range newLinks {
			if _, had := prev.links[targetIP]; !had {
				targetToken := c.idm.Tokenize(targetIP)
				if err := c.store.InsertEvent(ctx, now, fromToken, "link_new", targetToken, "", "", ""); err != nil {
					log.Printf("insert link_new %s->%s: %v", fromToken[:8], targetToken[:8], err)
				}
			}
		}
	}

	c.prev[fromIP] = peerState{
		links:       newLinks,
		lastSeen:    time.Now(),
		mgmtUp:      c.prev[fromIP].mgmtUp,
		signalUp:    c.prev[fromIP].signalUp,
		relaysAvail: c.prev[fromIP].relaysAvail,
	}
}

// detectNetbirdEvents compares netbird management/signal/relay state with previous
// state and records outage start/end events.
func (c *Collector) detectNetbirdEvents(ctx context.Context, fromIP, fromToken string, nb types.NetbirdHealth, now int64) {
	prev, existed := c.prev[fromIP]
	if !existed {
		// First report — just record initial state, no events.
		prev := c.prev[fromIP]
		prev.mgmtUp = nb.ManagementConnected
		prev.signalUp = nb.SignalConnected
		prev.relaysAvail = nb.RelaysAvailable
		c.prev[fromIP] = prev
		return
	}

	// Management transitions.
	if prev.mgmtUp && !nb.ManagementConnected {
		if err := c.store.InsertEvent(ctx, now, fromToken, "netbird_mgmt_down", "", "", "", nb.ManagementURL); err != nil {
			log.Printf("insert netbird_mgmt_down %s: %v", fromToken[:8], err)
		}
	} else if !prev.mgmtUp && nb.ManagementConnected {
		if err := c.store.InsertEvent(ctx, now, fromToken, "netbird_mgmt_up", "", "", "", nb.ManagementURL); err != nil {
			log.Printf("insert netbird_mgmt_up %s: %v", fromToken[:8], err)
		}
	}

	// Signal transitions.
	if prev.signalUp && !nb.SignalConnected {
		if err := c.store.InsertEvent(ctx, now, fromToken, "netbird_signal_down", "", "", "", nb.SignalURL); err != nil {
			log.Printf("insert netbird_signal_down %s: %v", fromToken[:8], err)
		}
	} else if !prev.signalUp && nb.SignalConnected {
		if err := c.store.InsertEvent(ctx, now, fromToken, "netbird_signal_up", "", "", "", nb.SignalURL); err != nil {
			log.Printf("insert netbird_signal_up %s: %v", fromToken[:8], err)
		}
	}

	// Relay availability transitions.
	if prev.relaysAvail > 0 && nb.RelaysAvailable == 0 {
		if err := c.store.InsertEvent(ctx, now, fromToken, "netbird_relays_down", "", "", "", ""); err != nil {
			log.Printf("insert netbird_relays_down %s: %v", fromToken[:8], err)
		}
	} else if prev.relaysAvail == 0 && nb.RelaysAvailable > 0 {
		if err := c.store.InsertEvent(ctx, now, fromToken, "netbird_relays_up", "", "", "", ""); err != nil {
			log.Printf("insert netbird_relays_up %s: %v", fromToken[:8], err)
		}
	}

	p := c.prev[fromIP]
	p.mgmtUp = nb.ManagementConnected
	p.signalUp = nb.SignalConnected
	p.relaysAvail = nb.RelaysAvailable
	c.prev[fromIP] = p
}

// autoAliasGateway creates a human-readable alias for a gateway from its
// fingerprint data if no manual alias has been set. Manual aliases always
// take precedence — this never overwrites.
func (c *Collector) autoAliasGateway(ctx context.Context, fp types.NetworkFingerprint) {
	has, err := c.store.HasAlias(ctx, "gateway", fp.GatewayIP)
	if err != nil || has {
		return
	}
	gw := fp.Gateway
	var alias string
	switch {
	case gw.Model != "" && gw.Manufacturer != "":
		alias = gw.Manufacturer + " " + gw.Model
	case gw.Model != "":
		alias = gw.Model
	case gw.Manufacturer != "":
		alias = gw.Manufacturer + " gateway"
	}
	if alias == "" {
		return
	}
	if fp.Subnet != "" {
		alias += " (" + fp.Subnet + ")"
	}
	if err := c.store.UpsertAlias(ctx, "gateway", fp.GatewayIP, alias); err != nil {
		log.Printf("auto-alias gateway %q: %v", fp.GatewayIP, err)
	} else {
		log.Printf("auto-aliased gateway %q → %q", fp.GatewayIP, alias)
	}
}

// autoAliasPeer creates a human-readable alias for a peer from its Netbird
// FQDN if no manual alias has been set. Strips the mesh-zone
// suffix to produce a short name.
func (c *Collector) autoAliasPeer(ctx context.Context, peerIP, peerName string) {
	has, err := c.store.HasAlias(ctx, "peer", peerIP)
	if err != nil || has {
		return
	}
	// Strip FQDN suffix to get short hostname.
	alias := peerName
	for _, suffix := range c.peerSuffixes {
		if idx := len(alias) - len(suffix); idx > 0 && alias[idx:] == suffix {
			alias = alias[:idx]
			break
		}
	}
	if alias == "" {
		return
	}
	if err := c.store.UpsertAlias(ctx, "peer", peerIP, alias); err != nil {
		log.Printf("auto-alias peer %q: %v", peerIP, err)
	} else {
		log.Printf("auto-aliased peer %q → %q", peerIP, alias)
	}
}
