package mailer

import (
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/textproto"
	"os"
	"strings"
	"testing"
	"time"
)

// noAuthRelay is an SMTP server that, like the smtp-sink recipe's Mailpit,
// advertises no AUTH and refuses one. It returns the DATA of each message.
func noAuthRelay(t *testing.T) (string, <-chan string) {
	t.Helper()
	l, err := (&net.ListenConfig{}).Listen(t.Context(), "tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = l.Close() })
	got := make(chan string, 4)
	go func() {
		for {
			conn, err := l.Accept()
			if err != nil {
				return
			}
			go func() {
				defer func() { _ = conn.Close() }()
				_ = conn.SetDeadline(time.Now().Add(5 * time.Second))
				c := textproto.NewConn(conn)
				_ = c.PrintfLine("220 sink")
				for {
					line, err := c.ReadLine()
					if err != nil {
						return
					}
					switch {
					case strings.HasPrefix(line, "EHLO"), strings.HasPrefix(line, "HELO"):
						_ = c.PrintfLine("250-sink\r\n250 8BITMIME")
					case strings.HasPrefix(line, "AUTH"):
						_ = c.PrintfLine("502 no AUTH here")
					case strings.HasPrefix(line, "DATA"):
						_ = c.PrintfLine("354 go")
						body, err := c.ReadDotLines()
						if err != nil {
							return
						}
						got <- strings.Join(body, "\n")
						_ = c.PrintfLine("250 queued")
					case strings.HasPrefix(line, "QUIT"):
						_ = c.PrintfLine("221 bye")
						return
					default:
						_ = c.PrintfLine("250 ok")
					}
				}
			}()
		}
	}()
	return l.Addr().String(), got
}

// An empty SMTP username sends without AUTH, so a relay on the goatnet's own
// network delivers; a username against the same relay is refused (control).
func TestSendInvite_emptyUsernameSendsWithoutAuth(t *testing.T) {
	addr, got := noAuthRelay(t)
	host, port, _ := net.SplitHostPort(addr)
	m := New(host, port, "", "", "onboard@example.org", "admin@example.org")
	if err := m.SendInvite("cand@example.test", "https://intake.pasture.lan/i/abc", "Pat Admin", "pasture", "", "", 0); err != nil {
		t.Fatalf("send through a no-AUTH relay with an empty username: %v", err)
	}
	select {
	case body := <-got:
		if !strings.Contains(body, "https://intake.pasture.lan/i/abc") {
			t.Fatalf("delivered message does not carry the link:\n%s", body)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the relay received no message")
	}
	withUser := New(host, port, "relay-user", "relay-pass", "onboard@example.org", "admin@example.org")
	if err := withUser.SendInvite("cand@example.test", "https://x.test/i/1", "Pat Admin", "pasture", "", "", 0); err == nil {
		t.Fatal("control: a username against a relay without AUTH should fail, and it sent")
	}
}

// The smtp-sink recipe end to end: the real Mailpit container the role runs
// (ops/recipes/smtp-sink/test.sh starts it and sets both variables). Skipped
// when they are unset; failed, not skipped, when they are set and it is down.
func TestSendInvite_smtpSinkContainer(t *testing.T) {
	smtpAddr, api := os.Getenv("GOAT_SMTP_SINK_SMTP"), strings.TrimRight(os.Getenv("GOAT_SMTP_SINK_API"), "/")
	if smtpAddr == "" || api == "" {
		t.Skip("GOAT_SMTP_SINK_SMTP / GOAT_SMTP_SINK_API unset: run ops/recipes/smtp-sink/test.sh")
	}
	host, port, err := net.SplitHostPort(smtpAddr)
	if err != nil {
		t.Fatal(err)
	}
	to := fmt.Sprintf("cand-%d@example.test", time.Now().UnixNano())
	link := "https://intake.pasture.lan/i/" + fmt.Sprint(time.Now().UnixNano())
	m := New(host, port, "", "", "onboard@example.org", "admin@example.org")
	if err := m.SendInvite(to, link, "Pat Admin", "pasture", "", "", 0); err != nil {
		t.Fatalf("send to the sink: %v", err)
	}
	var list struct {
		Messages []struct {
			ID      string
			Subject string
			To      []struct{ Address string }
		}
	}
	id := ""
	for i := 0; i < 20 && id == ""; i++ {
		resp, err := sinkGet(t, api+"/api/v1/messages")
		if err != nil {
			t.Fatalf("sink API: %v", err)
		}
		_ = json.NewDecoder(resp.Body).Decode(&list)
		_ = resp.Body.Close()
		for _, msg := range list.Messages {
			if len(msg.To) > 0 && msg.To[0].Address == to {
				id = msg.ID
			}
		}
		if id == "" {
			time.Sleep(250 * time.Millisecond)
		}
	}
	if id == "" {
		t.Fatalf("the sink holds no message to %s", to)
	}
	resp, err := sinkGet(t, api+"/api/v1/message/"+id)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	var one struct{ Text string }
	if err := json.NewDecoder(resp.Body).Decode(&one); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(one.Text, link) {
		t.Fatalf("the captured invite does not carry the link %s", link)
	}
	withUser := New(host, port, "relay-user", "relay-pass", "onboard@example.org", "admin@example.org")
	if err := withUser.SendInvite(to, link, "Pat Admin", "pasture", "", "", 0); err == nil {
		t.Fatal("control: a username against the sink (no AUTH) should fail, and it sent")
	}
}

// sinkGet reads the test sink's own API, at the address the test harness set.
func sinkGet(t *testing.T, url string) (*http.Response, error) {
	t.Helper()
	req, err := http.NewRequestWithContext(t.Context(), http.MethodGet, url, nil) //nolint:gosec // G704: the test sink's own API address, from the test harness
	if err != nil {
		return nil, err
	}
	return http.DefaultClient.Do(req) //nolint:gosec // G704: the test sink's own API address, from the test harness
}
