package handlers

import (
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/config"
)

// These templates exist because a welcome email once told people their SSH key
// had been added when it had not. So the tests assert the CLAIMS, not the
// formatting: what a message promises is the part that can hurt someone.
func TestSSHKeyNotificationsClaimOnlyWhatIsTrue(t *testing.T) {
	srv := &Server{Cfg: &config.Config{
		NotifyGoatnet:    "zagros-prod",
		ConsolePublicURL: "https://admin.auth.zagros-prod.examplebird.zagros-prod.example/",
		// The intake front desk. Set deliberately to the WRONG-for-this-purpose
		// value so the tests below prove we never build the link from it.
		PortalPublicURL: "https://intake.zagros-prod.example",
	}}

	t.Run("ssh_key_needed does not imply access already exists", func(t *testing.T) {
		subj, body, ok := srv.candidateEmail(notifyRequest{
			Event: "ssh_key_needed", Email: "a@b.test", Name: "Sam",
		})
		if !ok {
			t.Fatal("event not handled")
		}
		for _, must := range []string{
			"You do not yet",          // states the absence plainly
			"does NOT grant access",   // submitting is not authorisation
			"until it says installed", // names the state that means access
			"PRIVATE KEY",             // warns off the one fatal paste
			"/ssh-key",                // an actionable link
			"ssh-keygen -t ed25519",   // a path for people with no key
		} {
			if !strings.Contains(body, must) {
				t.Errorf("missing %q", must)
			}
		}
		if strings.Contains(subj, "installed") {
			t.Errorf("subject must not suggest installation: %q", subj)
		}
		// THE BUG THIS EXISTS FOR: the link must point at the CONSOLE, which
		// serves /ssh-key, not the intake front desk, which 404s. Asserting the
		// body merely contains "/ssh-key" passed while shipping a dead link to
		// three people .
		if !strings.Contains(body, "https://admin.auth.zagros-prod.examplebird.zagros-prod.example/ssh-key") {
			t.Errorf("link must be the console base + /ssh-key; body was:\n%s", body)
		}
		if strings.Contains(body, "intake.zagros-prod.example") {
			t.Error("must never build the submit link from the intake front desk — it does not serve /ssh-key")
		}
	})

	t.Run("ssh_key_installed names the hosts and the attribution", func(t *testing.T) {
		_, body, ok := srv.candidateEmail(notifyRequest{
			Event: "ssh_key_installed", Email: "a@b.test",
			Hosts: []string{"goat@100.127.0.217", "goat@100.127.0.168"},
		})
		if !ok {
			t.Fatal("event not handled")
		}
		// A person told "you have a shell" must be told WHERE.
		for _, h := range []string{"100.127.0.217", "100.127.0.168"} {
			if !strings.Contains(body, h) {
				t.Errorf("host %s not named", h)
			}
		}
		// The shared account is only defensible if attribution is explained.
		if !strings.Contains(body, "fingerprint") {
			t.Error("must explain that logins are attributed by key fingerprint")
		}
	})

	t.Run("ssh_key_installed with no hosts still sends, without inventing any", func(t *testing.T) {
		_, body, ok := srv.candidateEmail(notifyRequest{
			Event: "ssh_key_installed", Email: "a@b.test",
		})
		if !ok || strings.Contains(body, "ssh \r\n") {
			t.Error("empty host list must render no ssh lines rather than a blank one")
		}
	})

	t.Run("ssh_shell_by_identity never tells a network admin to send a key", func(t *testing.T) {
		_, body, ok := srv.candidateEmail(notifyRequest{
			Event: "ssh_shell_by_identity", Email: "a@b.test",
		})
		if !ok {
			t.Fatal("event not handled")
		}
		if !strings.Contains(body, "no key to send") {
			t.Error("must state that no key is needed — they already have access")
		}
		if !strings.Contains(body, "do NOT have shell access to the management hosts") {
			t.Error("must state the boundary explicitly")
		}
		// The failure mode: pasting the platform-admin instructions in here would
		// send someone chasing a key upload that grants them nothing.
		if strings.Contains(body, "ssh-keygen") || strings.Contains(body, "/ssh-key") {
			t.Error("must not send a network admin down the key-submission path")
		}
	})

	t.Run("no console URL configured degrades to a human, not a broken link", func(t *testing.T) {
		bare := &Server{Cfg: &config.Config{NotifyGoatnet: "zagros-prod",
			PortalPublicURL: "https://intake.zagros-prod.example"}} // intake set, console NOT
		_, body, _ := bare.candidateEmail(notifyRequest{Event: "ssh_key_needed", Email: "a@b.test"})
		if strings.Contains(body, "/ssh-key") {
			t.Error("must not render a link when no base URL is configured")
		}
		if !strings.Contains(body, "Send it to an administrator") {
			t.Error("must fall back to the out-of-band path")
		}
	})

	t.Run("an unknown event is refused rather than sent blank", func(t *testing.T) {
		if _, _, ok := srv.candidateEmail(notifyRequest{Event: "ssh_key_whatever"}); ok {
			t.Error("unknown event must not render an email")
		}
	})
}
