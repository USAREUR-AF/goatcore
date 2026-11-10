// Package mailer sends onboarding email via SMTP (AWS SES). It's intentionally
// tiny — a single templated send for the onboarding invite link. Email is the
// channel that reaches a not-yet-onboarded or locked-out person (chat can't).
package mailer

import (
	"errors"
	"fmt"
	"mime/quotedprintable"
	"net/smtp"
	"strings"
	"time"
)

// ErrNoReplyRoute is returned by SendInvite when the message would tell the
// candidate nobody to reply to: no Reply-To is configured and the sponsor is
// not a named person (a machine-minted invite carrying a registry anchor, or
// "operator"). The caller shows the link and this refusal instead of sending;
// the .eml hand-over goes out from a mailbox that takes replies.
var ErrNoReplyRoute = errors.New("invite names nobody to reply to")

// sponsorIsNamed reports whether sponsor is a person's or org's display name
// worth printing. A raw registry anchor (32 hex) is what a machine-minted
// invite carries when no display name was resolved; "invited by 34ed7992…"
// tells the candidate nothing and reads as a glitch (seen live).
func sponsorIsNamed(sponsor string) bool {
	return sponsor != "" && sponsor != "operator" && !looksLikeAnchor(sponsor)
}

// Mailer holds SMTP config. Enabled() is false when unconfigured (host empty) —
// the caller then falls back to showing the link for manual hand-over.
//
// replyTo is the monitored mailbox a candidate's reply should land in. The
// From address is a send-only SES identity (its zone carries no MX), so without
// a Reply-To every reply bounces with "Domain has no MX records" and the
// candidate reads that as the system being down (vendor onboarding,
// ). Empty = no Reply-To header; the body then says a
// reply is not delivered and points at the sponsor instead, and when the
// sponsor is not a named person either, SendInvite refuses (ErrNoReplyRoute)
// rather than send a mail that names nobody.
type Mailer struct {
	host, port, username, password, from, replyTo string
	// networkNote renders the invite's "Before you start: your network" block
	// for a goatnet and invite link. Nil (the default) omits the block.
	networkNote func(goatnet, link string) []string
}

// SetNetworkNote installs the renderer for the invite's network block. The
// console wires it from its relay config; tests and tools may leave it nil.
func (m *Mailer) SetNetworkNote(f func(goatnet, link string) []string) {
	if m != nil {
		m.networkNote = f
	}
}

func New(host, port, username, password, from, replyTo string) *Mailer {
	return &Mailer{host: strings.TrimSpace(host), port: strings.TrimSpace(port),
		username: username, password: password, from: strings.TrimSpace(from),
		replyTo: strings.TrimSpace(replyTo)}
}

func (m *Mailer) Enabled() bool { return m != nil && m.host != "" && m.from != "" }

// auth is PLAIN with the configured credentials, or nil when SMTP_USERNAME is
// empty: a relay on the goatnet's own network (the smtp-sink recipe,
// ops/recipes/smtp-sink/) takes mail without AUTH, and net/smtp refuses to
// send through a server that does not advertise AUTH when an auth is passed.
// SES always has a username, so its path is unchanged.
func (m *Mailer) auth() smtp.Auth {
	if m.username == "" {
		return nil
	}
	return smtp.PlainAuth("", m.username, m.password, m.host)
}

// ComposeInvite builds the full RFC 5322 invite message. Shared by the SMTP
// send and the .eml download (out-of-band delivery), so the two can never
// drift. Usable even when SMTP is unconfigured (From falls back to a
// placeholder the admin's mail client will replace). tagline/purpose are the
// per-goatnet environment identity (which goatnet is this, what is it for) —
// blank renders exactly the pre-existing name-only invite.
//
// expiresAt (unix seconds, 0 = unknown) is stated in the body in absolute terms.
// "time-limited" is not good enough: a link minted with an unintended one-hour
// TTL reads identically to a three-day one, so neither the candidate nor the
// sponsor (who gets the same wording on the confirmation) can see the mistake
// until the link is already dead. See docs/findings/F-363.
func (m *Mailer) ComposeInvite(to, link, sponsor, goatnet, tagline, purpose string, expiresAt int64) string {
	from, replyTo := "", ""
	if m != nil {
		from, replyTo = m.from, m.replyTo
	}
	if from == "" {
		from = "onboarding@example.invalid"
	}
	label := goatnet
	if t := strings.TrimSpace(tagline); t != "" && goatnet != "" {
		label = goatnet + " (" + t + ")"
	}
	subject := "You've been invited to onboard to " + label
	body := "" +
		"Hello,\r\n\r\n" +
		"You've been invited to onboard to the " + goatnet + " data fabric" +
		func() string {
			// A sponsor is named only when we hold a NAME (sponsorIsNamed).
			if sponsorIsNamed(sponsor) {
				return " by " + sponsor
			}
			return ""
		}() + ".\r\n\r\n" +
		func() string {
			if p := strings.TrimSpace(purpose); p != "" {
				return "What you're joining — " + label + ": " + p + "\r\n\r\n"
			}
			return ""
		}() +
		"Open this one-time link and fill in your details:\r\n\r\n" +
		"    " + link + "\r\n\r\n" +
		// Four fields decide what you actually get, and three of them are
		// marked "optional" on the form, which reads as "unimportant". It is
		// cheaper to say so here than to correct it at review — a wrong
		// organization used to be a dead end for the reviewer, and a blank
		// GitHub username silently means no repositories.
		"Four answers decide what you actually get, so they are worth a moment:\r\n\r\n" +
		"  * Organization — your employer or unit, NOT your own name. It is your\r\n" +
		"    organization of record, it groups you with your colleagues, and your\r\n" +
		"    teammates should all type the same name.\r\n" +
		"  * GitHub username — repository access is granted to this account when you\r\n" +
		"    are approved. Leave it blank and you get no repositories.\r\n" +
		"  * Nationality — YOUR OWN citizenship, required. Which shared data you can be\r\n" +
		"    shown follows from it. Your organization's nation is a separate fact,\r\n" +
		"    recorded by the administrator who registers the organization.\r\n" +
		"  * Machines / devices — how many of your own devices you will put on the\r\n" +
		"    network. It sets the allowance on your enrollment key, so count the\r\n" +
		"    reinstall you will do later.\r\n\r\n" +
		// The site network must allow the goatnet's data ports BEFORE the candidate
		// enrolls; a site that allows only 443 registers the device and then
		// reaches nothing. Said here, before the link is used.
		func() string {
			if m == nil || m.networkNote == nil {
				return ""
			}
			lines := m.networkNote(goatnet, link)
			if len(lines) == 0 {
				return ""
			}
			return strings.Join(lines, "\r\n") + "\r\n\r\n"
		}() +
		"The link is single-use and " + inviteExpiryPhrase(expiresAt) +
		". If you weren't expecting this, ignore it.\r\n\r\n" +
		// The sender takes no mail. Say where questions go BEFORE the candidate
		// hits Reply and gets a bounce they read as an outage.
		func() string {
			if replyTo != "" {
				return "Questions go to " + replyTo + ". A reply to this message is delivered there.\r\n"
			}
			return "A reply to this message is not delivered: the sending address takes no mail.\r\n" +
				"Questions go to the person who invited you.\r\n"
		}()

	return headers(from, to, replyTo, subject) +
		"Content-Type: text/plain; charset=UTF-8\r\n" +
		"\r\n" + body
}

// Send delivers one plaintext email — the generic leg under the candidate
// stage updates (onboarding-notifications.md increment 3). STARTTLS on 587
// (SES).
func (m *Mailer) Send(to, subject, body string) error {
	if !m.Enabled() {
		return fmt.Errorf("mailer disabled")
	}
	if err := checkRecipient(to); err != nil {
		return err
	}
	msg := headers(m.from, to, "", subject) +
		"Content-Type: text/plain; charset=UTF-8\r\n" +
		"\r\n" + body
	addr := m.host + ":" + m.port
	auth := m.auth()
	if err := smtp.SendMail(addr, auth, m.from, []string{to}, []byte(msg)); err != nil {
		return fmt.Errorf("smtp send: %w", err)
	}
	return nil
}

// SendMultipart delivers one email with a plain-text part and an HTML part
// (multipart/alternative, text first). The text part is the record and what a
// text-only client shows; the HTML part is what Gmail and Outlook show. Both
// come from the same rendered document (welcome.Package), so they cannot say
// different things. Parts are quoted-printable so no line exceeds the SMTP
// limit and UTF-8 survives every relay.
func (m *Mailer) SendMultipart(to, subject, text, html string) error {
	if !m.Enabled() {
		return fmt.Errorf("mailer disabled")
	}
	if err := checkRecipient(to); err != nil {
		return err
	}
	msg, err := ComposeMultipart(m.from, to, subject, text, html)
	if err != nil {
		return err
	}
	addr := m.host + ":" + m.port
	auth := m.auth()
	if err := smtp.SendMail(addr, auth, m.from, []string{to}, []byte(msg)); err != nil {
		return fmt.Errorf("smtp send: %w", err)
	}
	return nil
}

// ComposeMultipart builds the RFC 5322 message SendMultipart sends. Exported
// so the .eml download and the preview tool produce the identical bytes.
func ComposeMultipart(from, to, subject, text, html string) (string, error) {
	const boundary = "=_goat_welcome_alt_7f3c9a1b"
	qp := func(s string) (string, error) {
		var b strings.Builder
		w := quotedprintable.NewWriter(&b)
		if _, err := w.Write([]byte(s)); err != nil {
			return "", err
		}
		if err := w.Close(); err != nil {
			return "", err
		}
		return b.String(), nil
	}
	tq, err := qp(text)
	if err != nil {
		return "", err
	}
	hq, err := qp(html)
	if err != nil {
		return "", err
	}
	return headers(from, to, "", subject) +
		"Content-Type: multipart/alternative; boundary=\"" + boundary + "\"\r\n" +
		"\r\n" +
		"--" + boundary + "\r\n" +
		"Content-Type: text/plain; charset=UTF-8\r\n" +
		"Content-Transfer-Encoding: quoted-printable\r\n" +
		"\r\n" + tq + "\r\n" +
		"--" + boundary + "\r\n" +
		"Content-Type: text/html; charset=UTF-8\r\n" +
		"Content-Transfer-Encoding: quoted-printable\r\n" +
		"\r\n" + hq + "\r\n" +
		"--" + boundary + "--\r\n", nil
}

// SendInvite emails the one-time onboarding link to the candidate. STARTTLS on
// 587 (SES). Best-effort — a send error is returned so the handler can still
// show the link instead of failing the mint.
func (m *Mailer) SendInvite(to, link, sponsor, goatnet, tagline, purpose string, expiresAt int64) error {
	if !m.Enabled() {
		return fmt.Errorf("mailer disabled")
	}
	// With no Reply-To and no named sponsor the body can only say "questions
	// go to the person who invited you" about a person it cannot name: the
	// candidate has no route at all, which is the fault Reply-To exists to
	// prevent (a vendor's reply bounced). Refused before the dial,
	// so nothing is half-sent; the caller shows the link and this message,
	// and the .eml hand-over (same composition) goes out from a mailbox that
	// takes replies.
	if m.replyTo == "" && !sponsorIsNamed(sponsor) {
		return fmt.Errorf("not sent to %s: the invite would name nobody to reply to (SMTP_REPLY_TO is empty and the sponsor is not a named person). Set SMTP_REPLY_TO on this console, or download the .eml and send it from a mailbox that takes replies: %w", to, ErrNoReplyRoute)
	}
	if err := checkRecipient(to); err != nil {
		return err
	}
	msg := m.ComposeInvite(to, link, sponsor, goatnet, tagline, purpose, expiresAt)

	addr := m.host + ":" + m.port
	auth := m.auth()
	//nolint:gosec // G707: checkRecipient refused CR/LF in the recipient; headers() passes every value through headerValue
	if err := smtp.SendMail(addr, auth, m.from, []string{to}, []byte(msg)); err != nil {
		return fmt.Errorf("smtp send: %w", err)
	}
	return nil
}

// headers renders the header block every message shares, through MIME-Version.
// Each value passes through headerValue: the subject carries the goatnet name
// and tagline, and the recipient comes from a request, so a CR or LF in either
// would end the field and let the value add headers or start the body early.
func headers(from, to, replyTo, subject string) string {
	h := "From: " + headerValue(from) + "\r\n" +
		"To: " + headerValue(to) + "\r\n"
	if replyTo = headerValue(replyTo); replyTo != "" {
		h += "Reply-To: " + replyTo + "\r\n"
	}
	return h + "Subject: " + headerValue(subject) + "\r\n" +
		"MIME-Version: 1.0\r\n"
}

// headerValue keeps s on one header line by turning each CR and LF into a
// space. Nothing legitimate in these fields spans lines.
func headerValue(s string) string {
	return strings.Map(func(r rune) rune {
		if r == '\r' || r == '\n' {
			return ' '
		}
		return r
	}, s)
}

// checkRecipient refuses an address that spans lines before anything is
// dialed. smtp.SendMail refuses one too, but only after composing; failing
// here keeps the error about the address rather than the transport.
func checkRecipient(to string) error {
	if strings.ContainsAny(to, "\r\n") {
		return fmt.Errorf("refusing recipient %q: an address cannot contain a line break", to)
	}
	return nil
}

// looksLikeAnchor reports whether a sponsor string is a bare registry anchor
// (32 lowercase hex) rather than a person's or org's display name.
func looksLikeAnchor(s string) bool {
	s = strings.TrimSpace(s)
	if len(s) != 32 {
		return false
	}
	for _, c := range s {
		if (c < '0' || c > '9') && (c < 'a' || c > 'f') {
			return false
		}
	}
	return true
}

// inviteExpiryPhrase renders an invite's expiry the way a reader can act on it:
// how long they have AND the wall-clock deadline. A zero expiry (caller has
// none to give) degrades to the old vague wording rather than inventing one.
func inviteExpiryPhrase(expiresAt int64) string {
	if expiresAt <= 0 {
		return "time-limited"
	}
	exp := time.Unix(expiresAt, 0).UTC()
	d := time.Until(exp)
	if d <= 0 {
		return "has already expired (" + exp.Format("2006-01-02 15:04") + " UTC)"
	}
	return "expires in " + humanDuration(d) + " — at " + exp.Format("2006-01-02 15:04") + " UTC"
}

// humanDuration renders a TTL in the largest unit that stays honest: "45
// minutes", "3 hours", "3 days". Rounds down, so it never promises more time
// than the link has.
func humanDuration(d time.Duration) string {
	switch {
	case d < time.Hour:
		return plural(int(d.Minutes()), "minute")
	case d < 48*time.Hour:
		return plural(int(d.Hours()), "hour")
	default:
		return plural(int(d.Hours()/24), "day")
	}
}

func plural(n int, unit string) string {
	if n == 1 {
		return "1 " + unit
	}
	return fmt.Sprintf("%d %ss", n, unit)
}
