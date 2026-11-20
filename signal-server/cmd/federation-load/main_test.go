package main

import (
	"context"
	"net"
	"testing"
	"time"

	"google.golang.org/grpc"

	signalpb "github.com/usareur-af/goatcore/signal-server/gen/signalexchange"
)

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

func startStandIn(t *testing.T) string {
	t.Helper()
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	s := grpc.NewServer()
	signalpb.RegisterSignalExchangeServer(s, standIn{})
	go func() { _ = s.Serve(lis) }()
	t.Cleanup(s.Stop)
	return lis.Addr().String()
}

func loadAgainstStandIns(t *testing.T) finalSummary {
	t.Helper()
	cfg := config{
		servers:               []string{startStandIn(t), startStandIn(t)},
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

// The pass mark the tool reports, pathological_miss_rate, reads a Send that
// answered OK as delivered. Against servers whose every forward fails with
// delivery unknown, it still reports a clean run: the failure the mark is for
// is invisible to it.
func TestTheMissRateReadsDeliveryUnknownAsDelivered(t *testing.T) {
	s := loadAgainstStandIns(t)
	// A Send cut off by the end of the run is an error, not a miss.
	if s.PathologicalMissRate != 0 || s.TotalDelivered != s.TotalSent-s.TotalError || s.TotalDelivered == 0 {
		t.Fatalf("expected the old mark to read every OK as delivered: %+v", s)
	}
}
