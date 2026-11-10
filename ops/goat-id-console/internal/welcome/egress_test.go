package welcome

import (
	"reflect"
	"strings"
	"testing"
)

// One row per hostname, whatever mix of roles it plays. On the goatnets that
// exist today the management name and the relay names are different records
// pointing at the same servers, and a relay is also a STUN host — so the merge
// is by NAME, and the ports accumulate.
func TestEgressHostsFrom_mergesRolesPerHostname(t *testing.T) {
	got := EgressHostsFrom("https://netbird.example.net",
		[]string{"rels://relay-a.example.net:33073", "rels://relay-b.example.net:33073", "not a url"},
		[]string{"relay-a.example.net", "stun.example.net:3479"})
	want := []EgressHost{
		{Host: "netbird.example.net", Ports: []string{"TCP 443"}},
		{Host: "relay-a.example.net", Ports: []string{"TCP 33073", "UDP 3478", "UDP 33073"}},
		{Host: "relay-b.example.net", Ports: []string{"TCP 33073", "UDP 33073"}},
		{Host: "stun.example.net", Ports: []string{"UDP 3479"}},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got  %+v\nwant %+v", got, want)
	}
}

// A firewall on the reader's side can do nothing with a private or mesh
// address. They appear when the console resolves through a local override, and
// printing one would send a network team to allowlist a meaningless range.
func TestPublicIPs_dropsAddressesARemoteRuleCannotUse(t *testing.T) {
	got := PublicIPs([]string{"8.8.8.8", "10.0.0.5", "100.127.1.23", "127.0.0.1", "192.168.1.1", "8.8.8.8", "2600:1f18::1", "junk"})
	want := []string{"2600:1f18::1", "8.8.8.8"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got %v want %v", got, want)
	}
}

func firewallCat() Catalog {
	c := guideCat()
	c.RelayHealth = "https://relay-health.zagros.netbird.zagros.example"
	c.Egress = []EgressHost{
		{Host: "netbird.zagros.example", Ports: []string{"TCP 443"}, IPs: []string{"198.51.100.10", "198.51.100.11"}},
		{Host: "relay-a.zagros.example", Ports: []string{"TCP 33073", "UDP 3478", "UDP 33073"}, IPs: []string{"198.51.100.10"}},
		{Host: "relay-b.zagros.example", Ports: []string{"TCP 33073", "UDP 33073"}, Unresolved: true},
	}
	return c
}

func TestGuide_firewallTopicPrintsThisGoatnetsAddresses(t *testing.T) {
	out, err := RenderGuidePage(BuildGuidePage(firewallCat(), "partner", "firewall", ""))
	if err != nil {
		t.Fatal(err)
	}
	out = unescaped(out)
	for _, want := range []string{
		"netbird.zagros.example", "relay-a.zagros.example", "TCP 33073, UDP 3478, UDP 33073", "198.51.100.10  198.51.100.11",
		"198.51.100.10/32\n198.51.100.11/32",
		"did not resolve to a public address",
		"Allow ALL of them",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("firewall topic missing %q", want)
		}
	}
}

// Without the relay list the topic still teaches the ports and settings, and
// says plainly that the addresses are missing rather than rendering an empty
// table that reads as "nothing to allow".
func TestGuide_firewallTopicWithoutRelayListSaysSo(t *testing.T) {
	out := renderGuide(t, "partner", "firewall", "")
	if !strings.Contains(out, "was not configured with its relay list") {
		t.Error("an unconfigured goatnet must say the addresses are missing")
	}
	if strings.Contains(out, "ready for a rule") {
		t.Error("no address list should render without addresses")
	}
}

// The question that produced this page was whether a client certificate could
// stand in for an allowlist. The page must answer it, and must not drift into
// promising inbound rules or a certificate path that does not exist.
func TestGuide_firewallTopicAnswersTheCertificateQuestion(t *testing.T) {
	out := renderGuide(t, "partner", "firewall", "")
	for _, want := range []string{"X.509 certificate is not a substitute", "OUTBOUND rules only", "TLS inspection", "UDP 3478"} {
		if !strings.Contains(out, want) {
			t.Errorf("firewall topic missing %q", want)
		}
	}
}

// Anyone can be behind a corporate firewall, so every role's index reaches it.
func TestGuide_everyRoleLinksTheFirewallTopic(t *testing.T) {
	for _, r := range Roles {
		found := false
		for _, tk := range r.Topics {
			if tk == "firewall" {
				found = true
			}
		}
		if !found {
			t.Errorf("%s index does not link the firewall topic", r.Key)
		}
	}
}
