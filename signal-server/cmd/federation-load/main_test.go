package main

import (
	"context"
	"net"
	"sync"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"

	signalpb "github.com/usareur-af/goatcore/signal-server/gen/signalexchange"
)

// fabric is the registry two delivering stand-ins share: a Send to a key is
// put on that key's stream, wherever it registered, as a federation would.
type fabric struct {
	mu      sync.Mutex
	streams map[string]signalpb.SignalExchange_ConnectStreamServer
}

// delivering is a stand-in signal server that delivers every Send.
type delivering struct {
	signalpb.UnimplementedSignalExchangeServer
	f *fabric
}

func (d delivering) Send(_ context.Context, m *signalpb.EncryptedMessage) (*signalpb.EncryptedMessage, error) {
	d.f.mu.Lock()
	st := d.f.streams[m.GetRemoteKey()]
	d.f.mu.Unlock()
	if st != nil {
		_ = st.Send(m)
	}
	return &signalpb.EncryptedMessage{}, nil
}

func (d delivering) ConnectStream(stream signalpb.SignalExchange_ConnectStreamServer) error {
	md, _ := metadata.FromIncomingContext(stream.Context())
	if keys := md.Get("x-wiretrustee-peer-id"); len(keys) == 1 {
		d.f.mu.Lock()
		d.f.streams[keys[0]] = stream
		d.f.mu.Unlock()
	}
	for {
		if _, err := stream.Recv(); err != nil {
			return nil
		}
	}
}

// standIn is a signal server that accepts every peer's stream and answers
// every Send with OK, but delivers nothing: what a real server answers when a
// federation forward fails ("delivery unknown", relay outcome forward_failed).
type standIn struct {
	signalpb.UnimplementedSignalExchangeServer
}

func (standIn) Send(context.Context, *signalpb.EncryptedMessage) (*signalpb.EncryptedMessage, error) {
	return &signalpb.EncryptedMessage{}, nil
}

func (standIn) ConnectStream(stream signalpb.SignalExchange_ConnectStreamServer) error {
	for {
		if _, err := stream.Recv(); err != nil {
			return nil
		}
	}
}

func startStandIn(t *testing.T, srv signalpb.SignalExchangeServer) string {
	t.Helper()
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	s := grpc.NewServer()
	signalpb.RegisterSignalExchangeServer(s, srv)
	go func() { _ = s.Serve(lis) }()
	t.Cleanup(s.Stop)
	return lis.Addr().String()
}

func loadAgainst(t *testing.T, a, b signalpb.SignalExchangeServer) finalSummary {
	t.Helper()
	cfg := config{
		servers:               []string{startStandIn(t, a), startStandIn(t, b)},
		rate:                  50,
		duration:              2 * time.Second,
		peersPerServer:        2,
		offlinePeersPerServer: 0,
		sendTimeout:           time.Second,
		seed:                  1,
	}
	s, err := run(context.Background(), cfg)
	if err != nil {
		t.Fatal(err)
	}
	if s.SentOnline == 0 {
		t.Fatalf("no sends to online peers: %+v", s)
	}
	return s
}

// A forward counts as delivered only when the target's own stream received
// it. Against servers that answer every Send OK and deliver nothing (a failed
// federation forward, delivery unknown), every send to an online peer is a
// miss, and the run fails.
func TestDeliveryUnknownIsAMissAndFailsTheRun(t *testing.T) {
	s := loadAgainst(t, standIn{}, standIn{})
	if s.TotalDelivered != 0 {
		t.Fatalf("nothing was received, so nothing is delivered: %+v", s)
	}
	if s.Unconfirmed == 0 || s.PathologicalMissRate == 0 {
		t.Fatalf("an OK nobody received is a miss: %+v", s)
	}
	if s.Verdict != "FAIL" {
		t.Fatalf("verdict %q, want FAIL: %+v", s.Verdict, s)
	}
}

// The control: servers that deliver every Send pass, every OK confirmed by
// the target's stream.
func TestConfirmedDeliveryPasses(t *testing.T) {
	f := &fabric{streams: map[string]signalpb.SignalExchange_ConnectStreamServer{}}
	s := loadAgainst(t, delivering{f: f}, delivering{f: f})
	if s.Unconfirmed != 0 || s.PathologicalMissRate != 0 || s.TotalDelivered != s.TotalSent-s.TotalError || s.TotalDelivered == 0 {
		t.Fatalf("every OK was received: %+v", s)
	}
	if s.Verdict != "PASS" {
		t.Fatalf("verdict %q, want PASS: %+v", s.Verdict, s)
	}
}
