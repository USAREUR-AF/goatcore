package server

import (
	"context"
	"errors"
	"io"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"

	signalpb "github.com/usareur-af/goatcore/signal-server/gen/signalexchange"
)

const (
	// Upstream Netbird client ↔ signal handshake headers. Clients send
	// peerIDHeader carrying their WireGuard public key in the initial
	// ConnectStream metadata; the server replies with peerRegisteredHeader=1
	// to confirm registration.
	peerIDHeader         = "x-wiretrustee-peer-id"
	peerRegisteredHeader = "x-wiretrustee-peer-registered"
)

// Send performs one-shot relay of msg to the peer addressed by
// msg.RemoteKey. Local delivery is tried first; on miss, if federation
// peers are configured, the message is fanned out via Forward and any
// peer that accepts it counts as delivery.
func (s *Server) Send(ctx context.Context, msg *signalpb.EncryptedMessage) (*signalpb.EncryptedMessage, error) {
	if msg.GetRemoteKey() == "" {
		s.metrics.relayTotal.WithLabelValues(outcomeInvalidArg, pathUnarySend).Inc()
		return nil, status.Error(codes.InvalidArgument, "remoteKey is required")
	}

	// ADR 1081 L2 — per-sender token bucket. Keyed on the sender peer key so
	// one abusive sender is throttled without touching healthy peers. Default
	// off; a breach returns ResourceExhausted + a RetryInfo backoff hint
	// instead of silently accepting the offer. This is the only relay path
	// that consumes a token — it is the path the storm arrived on and the
	// only one that can carry a retry hint back to the caller.
	if admit, retryAfter := s.rateLimiter.allow(msg.GetKey()); !admit {
		return nil, rateLimitError(retryAfter)
	}

	outcome, err := s.relay(ctx, msg, pathUnarySend)
	switch outcome {
	case outcomeDelivered, outcomeForwarded:
		return &signalpb.EncryptedMessage{}, nil
	case outcomeOutboundFull:
		return nil, status.Error(codes.ResourceExhausted, err.Error())
	default: // outcomeNotFound, outcomeForwardFailed, and anything unexpected
		// Match upstream netbird signal-server's silent-drop contract:
		// return OK with empty payload when the target peer is unknown.
		// Returning codes.NotFound here breaks vanilla daemon's
		// IsHealthy() liveness probe (shared/signal/client/grpc.go:270),
		// which sends Send(RemoteKey="dummy") expecting OK and treats
		// any error as "signal disconnected" — even though the
		// bidirectional ConnectStream is fine. See F-105 for the full
		// regression context.
		//
		// outcomeForwardFailed is
		// DELIBERATELY handled here too: returning an error would fail the
		// IsHealthy probe of EVERY peer the moment one federation peer is
		// unreachable, churning the whole fleet. The failure is surfaced
		// instead through the distinct relayTotal{outcome="forward_failed"}
		// metric + "relay forward-failed" log line recorded in relay() — the
		// signal an operator needs — not through the client-visible Send
		// result. The client retries offers on its own cadence; when the
		// federation path heals (or the dual-path transport routes around the
		// break), the retried offer gets through.
		_ = err
		return &signalpb.EncryptedMessage{}, nil
	}
}

// ConnectStream holds a bidirectional stream open for one peer. The client
// registers its WireGuard public key via the peerIDHeader metadata; the
// server responds with peerRegisteredHeader=1 and then runs two concurrent
// pumps:
//
//   - inbound:  stream.Recv() → relay(msg, pathStream) — local delivery
//     then federation fan-out on miss
//   - outbound: <- registry's outbound chan → stream.Send()
//
// The handler returns when either side closes, when the stream context is
// canceled, or when a later registration for the same key displaces us
// (detected by the outbound channel being closed).
func (s *Server) ConnectStream(stream signalpb.SignalExchange_ConnectStreamServer) error {
	md, ok := metadata.FromIncomingContext(stream.Context())
	if !ok {
		return status.Error(codes.InvalidArgument, "missing request metadata")
	}
	ids := md.Get(peerIDHeader)
	if len(ids) == 0 || ids[0] == "" {
		return status.Errorf(codes.InvalidArgument, "missing %q header", peerIDHeader)
	}
	key := ids[0]

	ps := s.registry.register(key)
	// ADR 1081 L2 — the recipient is back; drop any negative-cache offline
	// mark for this key the moment it registers, so a mass-reconnect makes
	// every reconnecting peer immediately reachable again (no TTL wait).
	s.negCache.purge(key)
	// ADR 1108 — local presence changed; nudge the pusher so peers learn this
	// key's owner (debounced; coalesces a reconnect burst into one push).
	s.presence.localChanged()
	defer func() {
		s.registry.unregister(key, ps)
		s.presence.localChanged()
	}()

	if err := stream.SendHeader(metadata.Pairs(peerRegisteredHeader, "1")); err != nil {
		return status.Error(codes.Internal, "failed to send registration header: "+err.Error())
	}

	s.Logger.Info("peer registered", "peer_key", key, "node", s.Node)

	pumpErr := make(chan error, 1)
	safeGo("connect-stream-recv-pump", func() {
		for {
			msg, err := stream.Recv()
			if errors.Is(err, io.EOF) {
				pumpErr <- nil
				return
			}
			if err != nil {
				pumpErr <- err
				return
			}
			_, _ = s.relay(stream.Context(), msg, pathStream) //nolint:errcheck // relay logs each per-message refusal and the stream must continue
			// Per-message logging (cid, from, to, outcome) happens inside
			// relay(); no duplicate log here.
		}
	})

	for {
		select {
		case <-stream.Context().Done():
			return stream.Context().Err()
		case err := <-pumpErr:
			return err
		case <-ps.evicted:
			return status.Error(codes.Aborted, "peer re-registered; closing stale stream")
		case msg := <-ps.outbound:
			if err := sendWithTimeout(stream, msg); err != nil {
				return err
			}
		}
	}
}

// outboundSendTimeout bounds a single stream.Send to a receiver.
//
// grpc-go's stream.Send blocks in the transport's flow-control wait until
// the peer's window frees or the stream ends. A client that stops calling
// Recv (wedged handler, stalled decryption worker, half-open transport)
// therefore parks this pump FOREVER: the peer stays registered, deliver()
// keeps filling its 16-deep queue, and once full every sender is told
// "outbound full" while the registry still reports the peer present — a
// permanent blackhole with no self-heal.
//
// Upstream netbird bounds the equivalent send at 10s and then CANCELS the
// peer, forcing a reconnect. That self-heal was dropped in the rewrite; this
// restores it. 10s matches upstream and is far above any healthy RTT.
const outboundSendTimeout = 10 * time.Second

// sendWithTimeout performs stream.Send under outboundSendTimeout. On timeout
// it returns a DeadlineExceeded error so ConnectStream returns, the deferred
// unregister runs, and the client reconnects into a fresh registration —
// rather than the pump parking indefinitely on a receiver that will never
// drain.
//
// The Send runs in its own goroutine because grpc-go offers no way to
// interrupt a blocked Send. On timeout that goroutine is abandoned; it
// unblocks and exits when the stream is torn down (which the returned error
// causes), so it cannot accumulate. The buffered channel keeps it from
// leaking on the timeout path.
func sendWithTimeout(stream signalpb.SignalExchange_ConnectStreamServer, msg *signalpb.EncryptedMessage) error {
	done := make(chan error, 1)
	safeGo("connect-stream-send", func() {
		sent := false
		defer func() {
			if !sent {
				done <- status.Error(codes.Internal, "internal server error")
			}
		}()
		err := stream.Send(msg)
		sent = true
		done <- err
	})
	select {
	case err := <-done:
		return err
	case <-time.After(outboundSendTimeout):
		return status.Errorf(codes.DeadlineExceeded,
			"outbound send exceeded %s; closing stream so the peer reconnects", outboundSendTimeout)
	}
}

// relay tries local delivery, falls back to federation on miss, and
// records metrics under a consistent outcome/path label pair. Every call
// emits exactly one log line tagged with the correlation ID so an
// operator can grep a single cid across nodes to trace a message end to
// end.
func (s *Server) relay(ctx context.Context, msg *signalpb.EncryptedMessage, path string) (outcome string, err error) {
	// F-338 — RECORD THE WIRE SIZE OF EVERY MESSAGE, on every exit path.
	//
	// The histogram was defined, registered and never observed. That is worse
	// than having no metric: an empty histogram exports all buckets at zero and
	// reads as "we measured, and there is nothing there". Every Rosenpass byte
	// figure quoted during the F-338 investigation came from one-off analysis
	// because this was the missing piece and it LOOKED present.
	//
	// Deferred on the NAMED return so it fires once with the final outcome,
	// including the negative-cache short-circuit below which returns before
	// anything else in this function runs. A recording placed inline after the
	// shed check would silently miss exactly the messages the shed exists for.
	//
	// proto.Size is the marshaled wire size — what the pipe actually carried,
	// not the in-memory struct. A Rosenpass offer is ~524 KB of Classic
	// McEliece key and an ordinary ICE offer is ~260 B, so the 512-and-1024
	// bucket edges separate them without arithmetic.
	size := float64(proto.Size(msg))
	defer func() {
		s.metrics.messageBytes.WithLabelValues(outcome, path).Observe(size)
	}()

	// ADR 1081 L2 — negative-cache short-circuit. If this recipient was found
	// offline within the TTL, decline to re-probe: no registry deliver, no
	// federation fan-out (the expensive part), no per-message relay-miss log
	// line (the log flood the storm produced). Cheapest possible early exit,
	// before even deriving the correlation ID. The client-visible result is
	// identical to an uncached miss (Send maps outcomeNotFound → OK/empty),
	// so the F-105 silent-drop contract and the health probe are unaffected.
	if s.negCache.offline(msg.GetRemoteKey()) {
		s.metrics.relayShedTotal.WithLabelValues(path).Inc()
		return outcomeNotFound, errUnknownPeer
	}

	cid := correlationID(msg)

	start := time.Now()
	localErr := s.registry.deliver(msg.GetRemoteKey(), msg)
	localElapsed := time.Since(start).Seconds()

	if localErr == nil {
		s.metrics.relayTotal.WithLabelValues(outcomeDelivered, path).Inc()
		s.metrics.deliverDuration.WithLabelValues(outcomeDelivered, path).Observe(localElapsed)
		s.Logger.Info("relay delivered",
			"cid", cid,
			"to", msg.GetRemoteKey(),
			"path", path,
			"node", s.Node)
		return outcomeDelivered, nil
	}

	// Local miss. Try federation if configured (nil-safe: standalone
	// returns delivered=false, complete=true — a local miss IS a full miss).
	//
	// ADR 1108: if presence knows which peer owns this key, forward DIRECTLY
	// there (O(1) instead of broadcasting to every peer). If presence is stale
	// (the owner moved/left → direct miss) or the owner is unknown, fall back to
	// Broadcast — the authoritative full attempt. Correctness never depends on
	// presence being fresh or present; with presence off, this is pure broadcast.
	var forwarded, fedComplete bool
	if addr, ok := s.presence.ownerAddr(msg.GetRemoteKey()); ok {
		s.metrics.presenceRouteTotal.WithLabelValues("direct").Inc()
		forwarded, fedComplete = s.federation.forwardOne(ctx, addr, msg)
		if !forwarded {
			forwarded, fedComplete = s.federation.Broadcast(ctx, msg)
		}
	} else {
		s.metrics.presenceRouteTotal.WithLabelValues("broadcast").Inc()
		forwarded, fedComplete = s.federation.Broadcast(ctx, msg)
	}
	if forwarded {
		s.metrics.relayTotal.WithLabelValues(outcomeForwarded, path).Inc()
		// No deliverDuration observation — federation has its own
		// forward_duration histogram; double-counting would be misleading.
		s.Logger.Info("relay forwarded",
			"cid", cid,
			"to", msg.GetRemoteKey(),
			"path", path,
			"node", s.Node)
		return outcomeForwarded, nil
	}

	// Neither local nor federation delivered. Distinguish three very different
	// cases — conflating them hides a fault (a silently-broken
	// federation looked identical to peers merely being offline):
	//   - outbound_full: the peer IS local but its receive queue is saturated.
	//   - forward_failed: the federation broadcast did NOT complete (an RPC
	//     error, timeout, breaker skip, or budget cut) — delivery status is
	//     UNKNOWN; the recipient may be online on an unreachable shard.
	//   - notfound: every peer gave a definitive answer and none had the peer
	//     (a genuine miss — the recipient really is offline).
	switch {
	case errors.Is(localErr, errOutboundFull):
		outcome = outcomeOutboundFull
	case !fedComplete:
		outcome = outcomeForwardFailed
	default:
		outcome = outcomeNotFound
	}
	s.metrics.relayTotal.WithLabelValues(outcome, path).Inc()
	s.metrics.deliverDuration.WithLabelValues(outcome, path).Observe(localElapsed)
	logMsg := "relay miss"
	if outcome == outcomeForwardFailed {
		// A distinct log line so an operator can grep federation breakage apart
		// from ordinary offline-peer misses.
		logMsg = "relay forward-failed (federation incomplete; delivery unknown)"
	}
	s.Logger.Info(logMsg,
		"cid", cid,
		"from", msg.GetKey(),
		"to", msg.GetRemoteKey(),
		"outcome", outcome,
		"path", path,
		"node", s.Node)

	// ADR 1081 L2 — record the offline mark only on a genuine full miss
	// (recipient absent both locally and across federation). outcomeOutboundFull
	// means the peer IS here but its queue is saturated — not offline, so it is
	// deliberately not cached. fedComplete (Phase 3, ADR 1085 Amendment 1 §3):
	// only cache when THIS broadcast got a definitive answer from every peer —
	// a breaker skip, RPC error, timeout, or budget cut means some peer was
	// never actually asked, so "absent across federation" is unknowable and
	// must not be cached. Per-call, so one dead peer in a multi-peer fleet
	// only suppresses caching for the misses it actually contaminated.
	if outcome == outcomeNotFound && fedComplete {
		s.negCache.record(msg.GetRemoteKey())
	}
	return outcome, localErr
}
