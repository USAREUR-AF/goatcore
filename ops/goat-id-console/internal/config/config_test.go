package config

import (
	"strings"
	"testing"
)

func TestLoadDefaultsTenantOnly(t *testing.T) {
	// IDP_OPERATOR_KRATOS_ADMIN_URL deliberately unset — operator IdP is
	// optional and should be omitted when its admin URL isn't provided.
	t.Setenv("IDP_TENANT_KRATOS_ADMIN_URL", "")
	t.Setenv("IDP_OPERATOR_KRATOS_ADMIN_URL", "")
	c, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := c.IdP("tenant"); !ok {
		t.Error("tenant IdP missing")
	}
	if _, ok := c.IdP("operator"); ok {
		t.Error("operator IdP unexpectedly present (URL was empty)")
	}
	if c.DefaultIdP != "tenant" {
		t.Errorf("DefaultIdP = %q, want tenant", c.DefaultIdP)
	}
	if len(c.IdPOrder) != 1 {
		t.Errorf("IdPOrder = %v, want 1 entry (operator omitted)", c.IdPOrder)
	}
}

// SMTP_REPLY_TO is the monitored mailbox an invite's Reply-To points at. The
// sender itself is send-only (no MX on the public zone), so a blank value must
// stay blank: the mailer then tells the candidate a reply is not delivered
// rather than inventing an address.
func TestLoadSMTPReplyTo(t *testing.T) {
	t.Setenv("IDP_TENANT_KRATOS_ADMIN_URL", "")
	t.Setenv("IDP_OPERATOR_KRATOS_ADMIN_URL", "")
	t.Setenv("SMTP_REPLY_TO", "")
	c, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	if c.SMTPReplyTo != "" {
		t.Errorf("SMTPReplyTo = %q, want empty when unset", c.SMTPReplyTo)
	}
	t.Setenv("SMTP_REPLY_TO", "sponsors@example.test")
	c, err = Load()
	if err != nil {
		t.Fatal(err)
	}
	if c.SMTPReplyTo != "sponsors@example.test" {
		t.Errorf("SMTPReplyTo = %q, want the configured address", c.SMTPReplyTo)
	}
}

// SMTP_FROM and SMTP_REPLY_TO are written raw into the From and Reply-To
// header lines and the Reply-To is printed in the invite body. A value that is
// not an address (a typo, a stray word, a trailing "@") would ship a malformed
// header on every invite and defeat the reply route the knob exists for, so
// Load refuses to start on one and names the variable. Display-name forms and
// bare addresses both parse; empty stays empty (email off, or no Reply-To).
func TestLoadSMTPAddressesMustParse(t *testing.T) {
	t.Setenv("IDP_TENANT_KRATOS_ADMIN_URL", "")
	t.Setenv("IDP_OPERATOR_KRATOS_ADMIN_URL", "")
	for _, tc := range []struct {
		name, from, replyTo, wantErr string
	}{
		{"both empty", "", "", ""},
		{"bare addresses", "onboarding@example.test", "sponsors@example.test", ""},
		{"display-name form", "GOAT Onboarding <onboarding@example.test>", "Sponsors <sponsors@example.test>", ""},
		{"from is not an address", "onboarding at example", "", "SMTP_FROM"},
		{"reply-to is truncated", "onboarding@example.test", "sponsors@", "SMTP_REPLY_TO"},
		{"reply-to is two addresses", "onboarding@example.test", "a@example.test, b@example.test", "SMTP_REPLY_TO"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			t.Setenv("SMTP_FROM", tc.from)
			t.Setenv("SMTP_REPLY_TO", tc.replyTo)
			c, err := Load()
			if tc.wantErr == "" {
				if err != nil {
					t.Fatalf("Load: %v", err)
				}
				if c.SMTPFrom != tc.from || c.SMTPReplyTo != tc.replyTo {
					t.Errorf("addresses must be kept verbatim: from %q reply-to %q", c.SMTPFrom, c.SMTPReplyTo)
				}
				return
			}
			if err == nil {
				t.Fatalf("Load accepted from %q reply-to %q; want an error naming %s", tc.from, tc.replyTo, tc.wantErr)
			}
			if !strings.Contains(err.Error(), tc.wantErr) {
				t.Errorf("error must name the variable %s: %v", tc.wantErr, err)
			}
		})
	}
}

// The mailer trims both values before they reach a header line, so Load must
// validate and store the trimmed form: otherwise a padded address is refused
// though it would have sent fine, and a whitespace-only value counts as set
// here while the mailer reads it as unset (no Reply-To, or email off).
func TestLoadSMTPAddressesAreTrimmed(t *testing.T) {
	t.Setenv("IDP_TENANT_KRATOS_ADMIN_URL", "")
	t.Setenv("IDP_OPERATOR_KRATOS_ADMIN_URL", "")
	t.Setenv("SMTP_FROM", "  onboarding@example.test\t")
	t.Setenv("SMTP_REPLY_TO", " Sponsors <sponsors@example.test> ")
	c, err := Load()
	if err != nil {
		t.Fatalf("Load refused a padded address: %v", err)
	}
	if c.SMTPFrom != "onboarding@example.test" {
		t.Errorf("SMTPFrom = %q, want it trimmed", c.SMTPFrom)
	}
	if c.SMTPReplyTo != "Sponsors <sponsors@example.test>" {
		t.Errorf("SMTPReplyTo = %q, want it trimmed", c.SMTPReplyTo)
	}

	t.Setenv("SMTP_FROM", "   ")
	t.Setenv("SMTP_REPLY_TO", "\t\n")
	c, err = Load()
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if c.SMTPFrom != "" || c.SMTPReplyTo != "" {
		t.Errorf("whitespace-only must read as unset: from %q reply-to %q", c.SMTPFrom, c.SMTPReplyTo)
	}
}

func TestLoadWithOperatorIdP(t *testing.T) {
	t.Setenv("IDP_OPERATOR_KRATOS_ADMIN_URL", "http://kratos-operator.example:4436")
	t.Setenv("IDP_OPERATOR_LABEL", "Custom Operator Label")
	c, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := c.IdP("operator"); !ok {
		t.Fatal("operator IdP missing — URL was set")
	}
	if got := c.IdPs["operator"].Label; got != "Custom Operator Label" {
		t.Errorf("label override not applied: %q", got)
	}
	if got := c.IdPs["operator"].KratosAdminURL; got != "http://kratos-operator.example:4436" {
		t.Errorf("URL not applied: %q", got)
	}
	if len(c.IdPOrder) != 2 {
		t.Errorf("IdPOrder = %v, want 2 entries", c.IdPOrder)
	}
}

func TestLoadTenantOverride(t *testing.T) {
	t.Setenv("IDP_TENANT_KRATOS_ADMIN_URL", "http://kratos-tenant.example:4434")
	t.Setenv("IDP_OPERATOR_KRATOS_ADMIN_URL", "")
	c, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	if got := c.IdPs["tenant"].KratosAdminURL; got != "http://kratos-tenant.example:4434" {
		t.Errorf("tenant URL override not applied: %q", got)
	}
}

func TestUnknownIdPLookup(t *testing.T) {
	t.Setenv("IDP_OPERATOR_KRATOS_ADMIN_URL", "")
	c, _ := Load()
	if _, ok := c.IdP("nonsense"); ok {
		t.Error("IdP() returned ok=true for nonsense key")
	}
}

func TestPublicDoorAdminPWAlias(t *testing.T) {
	cases := []struct{ old, neutral, want string }{
		{"", "", ""},
		{"old-pw", "", "old-pw"},
		{"", "neutral-pw", "neutral-pw"},
		{"old-pw", "neutral-pw", "old-pw"},
	}
	const oldName, neutralName = "GOAT_IDM_ADMIN_PW", "GOAT_IDM_ADMIN_PW"
	for _, tc := range cases {
		// In a build where both names are the same string there is one
		// variable, so only the cases that set it once apply.
		if oldName == neutralName && tc.old != "" {
			continue
		}
		t.Setenv(oldName, tc.old)
		t.Setenv(neutralName, tc.neutral)
		c, err := Load()
		if err != nil {
			t.Fatal(err)
		}
		if c.PublicDoorAdminPW != tc.want {
			t.Errorf("old=%q neutral=%q: PublicDoorAdminPW = %q, want %q", tc.old, tc.neutral, c.PublicDoorAdminPW, tc.want)
		}
	}
}
