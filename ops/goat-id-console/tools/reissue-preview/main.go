// Command reissue-preview renders a corrected (reissue) welcome package from the
// SAME internal/welcome renderer the console uses, so an operator can deliver a
// corrected package out of band before the console carrying the fix is deployed.
// Values come from flags mirroring the console's NOTIFY_* config; nothing is
// hardcoded. It only prints — sending is the operator's separate, deliberate step.
package main

import (
	"flag"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/welcome"
)

func main() {
	var (
		env       = flag.String("env", "", "goatnet env (e.g. galaxy)")
		mgmt      = flag.String("mgmt-url", "", "NOTIFY_MGMT_URL")
		get       = flag.String("get-url", "", "NOTIFY_GET_URL")
		portal    = flag.String("portal-url", "", "NOTIFY_PORTAL_URL")
		tagline   = flag.String("tagline", "", "NOTIFY_GOATNET_TAGLINE")
		purpose   = flag.String("purpose", "", "NOTIFY_GOATNET_PURPOSE")
		rosenpass = flag.Bool("rosenpass", false, "NOTIFY_ROSENPASS")
		asHTML    = flag.Bool("html", false, "print the HTML part instead of the text part")
		name      = flag.String("name", "", "recipient display name")
		handle    = flag.String("handle", "", "operator handle")
		email     = flag.String("email", "", "recipient email")
		tier      = flag.String("tier", "", "tier")
		groups    = flag.String("groups", "", "comma-separated NetBird auto-groups")
		github    = flag.String("github", "", "github username")
		setupKey  = flag.String("setup-key", "", "NetBird setup key")
		portalKey = flag.String("portal-key", "", "portal API key (vp_...)")
		recLink   = flag.String("recovery-link", "", "Kratos recovery link (admin POST /admin/recovery/code)")
		recCode   = flag.String("recovery-code", "", "Kratos recovery code that pairs with -recovery-link")
		recExp    = flag.String("recovery-expires", "", "RFC3339 expires_at Kratos returned for that link+code (e.g. 2026-09-05T07:11:33Z) — the package states this deadline verbatim instead of assuming the console's 24h")
		fabric    = flag.Bool("fabric", false, "recipient has a fabric (Zenoh) identity")
		publisher = flag.Bool("publisher", false, "recipient is a fabric publisher")
		fresh     = flag.Bool("fresh", false, "render a FIRST-time package instead of a reissue")
		subject   = flag.Bool("subject", false, "print the subject line instead of the body")
		affil     = flag.String("affiliation", "", "affiliation kind (organic|vendor|partner|civil) — a partner recipient is pointed at the partner view")
	)
	flag.Parse()

	cat := welcome.DeriveCatalog(*env, *mgmt, *get, *portal)
	cat.RosenpassEnabled = *rosenpass
	cat.Tagline = *tagline
	cat.Purpose = *purpose

	if *subject {
		goatnet := welcome.DisplayLabel(*env, *tagline)
		if *fresh {
			fmt.Println(welcome.SubjectFresh(goatnet))
		} else {
			fmt.Println(welcome.SubjectReissue(goatnet))
		}
		return
	}
	var gs []string
	if strings.TrimSpace(*groups) != "" {
		gs = strings.Split(*groups, ",")
	}
	rcpt := welcome.Recipient{Name: *name, Handle: *handle, Email: *email, Tier: *tier,
		Groups: gs, GitHubUsername: *github, FabricAccess: *fabric, Publisher: *publisher,
		AffiliationKind: *affil}
	// DERIVED, never a flag: the console derives these from the tier, so a flag
	// here would be a second home for the rule and a chance for an operator to
	// render an admin package that disagrees with what the console would send.
	rcpt.AdminConsole = welcome.TierGrantsAdminConsole(*tier)
	rcpt.AdminOperator = rcpt.AdminConsole
	// Without these the tool can only deliver the mesh half of a reissue, and the
	// password half falls back to hand-assembly — the exact thing the generated
	// package exists to prevent. The console mints these from Kratos admin; an
	// out-of-band reissue mints them the same way and passes them through here.
	art := welcome.Artifacts{SetupKey: *setupKey, PortalAPIKey: *portalKey,
		RecoveryLink: *recLink, RecoveryCode: *recCode}
	if v := strings.TrimSpace(*recExp); v != "" {
		t, err := time.Parse(time.RFC3339, v)
		if err != nil {
			fmt.Fprintf(os.Stderr, "-recovery-expires %q: %v\n", v, err)
			os.Exit(2)
		}
		art.RecoveryExpiresAt = t
	}
	if *fresh {
		if *asHTML {
			fmt.Print(welcome.RenderPackage(cat, rcpt, art).HTML)
			return
		}
		fmt.Print(welcome.Render(cat, rcpt, art))
		return
	}
	if *asHTML {
		fmt.Print(welcome.RenderReissuePackage(cat, rcpt, art).HTML)
		return
	}
	fmt.Print(welcome.RenderReissue(cat, rcpt, art))
}
