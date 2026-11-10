// presign.go mints short-TTL presigned S3 GET URLs for audit objects.
//
// This is the server side of goat-cli Phase G.6 (ADR 1038): an operator
// reads the Object-Lock audit-tail (goat replay / sub --since / narrate)
// without holding MinIO credentials. The portal — which already has the
// bucket creds for the live tail — lists the objects covering a time window
// and signs a GET URL for each, scoped to a short TTL. The bytes then flow
// laptop→MinIO directly; the portal is not in the data path, and no standing
// object-store credential leaves the server.
//
// Query-string (presigned) SigV4 form, reusing the same HMAC chain as the
// header-form signer in audittail.go.
package audittail

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

// Presigner signs presigned GET URLs against the audit bucket. It shares
// Config with the Tailer but needs no registry.
type Presigner struct {
	cfg Config
	cli *http.Client
}

// NewPresigner builds a Presigner. Call Enabled() before using it.
func NewPresigner(cfg Config) *Presigner {
	if cfg.Region == "" {
		cfg.Region = "us-east-1"
	}
	if cfg.SubscriberID == "" {
		cfg.SubscriberID = "site-A"
	}
	return &Presigner{cfg: cfg, cli: &http.Client{Timeout: 30 * time.Second}}
}

// Enabled is true when the bucket credentials are present.
func (p *Presigner) Enabled() bool {
	return p != nil && p.cfg.Endpoint != "" && p.cfg.Bucket != "" &&
		p.cfg.AccessKey != "" && p.cfg.SecretKey != ""
}

// ListSince returns audit object keys at or after `since`, capped at maxKeys.
// A zero `since` lists from the bucket start.
func (p *Presigner) ListSince(ctx context.Context, since time.Time, maxKeys int) ([]string, error) {
	prefix := p.cfg.SubscriberID + "/"
	startAfter := sinceStartAfter(prefix, since)
	if maxKeys <= 0 {
		maxKeys = 500
	}
	return listObjectKeys(ctx, p.cli, p.cfg, prefix, startAfter, maxKeys)
}

// sinceStartAfter builds the S3 ListObjectsV2 `start-after` bound for a `since`
// time. Audit object keys are `{id}/{YYYY}/{MM}/{DD}/{HH}/{epoch_ms}-{n}.ndjson`,
// lex-monotonic in time, so the bound is the per-millisecond opening of the
// filename — NOT just the hour boundary. An hour-granular bound is
// fatal on a busy fabric: ListObjectsV2 returns ascending order capped at
// maxKeys, so starting at the hour boundary returns the OLDEST objects of the
// since-hour and the cap then drops the recent tail the operator asked for —
// the client's exact-`since` filter then matches nothing. Bounding at the
// millisecond starts the listing AT `since`, so a narrow window returns the
// events it should. A zero `since` returns "" (list from the bucket start).
func sinceStartAfter(prefix string, since time.Time) string {
	if since.IsZero() {
		return ""
	}
	u := since.UTC()
	// e.g. "site-A/2026/06/12/14/1781279545000" — the same hour-partition path
	// the audit-subscriber writes, then the epoch-millis (fixed 13-digit
	// width, so lexical order == chronological order).
	return prefix + u.Format("2006/01/02/15/") + strconv.FormatInt(u.UnixMilli(), 10)
}

// PresignGet returns a presigned (query-string SigV4) GET URL for one object
// key, valid for ttl. Path-style addressing, UNSIGNED-PAYLOAD.
func (p *Presigner) PresignGet(key string, ttl time.Duration) (string, error) {
	endpoint := strings.TrimRight(p.cfg.Endpoint, "/")
	u, err := url.Parse(endpoint)
	if err != nil {
		return "", fmt.Errorf("parse endpoint %q: %w", endpoint, err)
	}
	host := u.Host

	now := time.Now().UTC()
	amzDate := now.Format("20060102T150405Z")
	dateStamp := now.Format("20060102")
	scope := fmt.Sprintf("%s/%s/s3/aws4_request", dateStamp, p.cfg.Region)

	q := url.Values{}
	q.Set("X-Amz-Algorithm", "AWS4-HMAC-SHA256")
	q.Set("X-Amz-Credential", p.cfg.AccessKey+"/"+scope)
	q.Set("X-Amz-Date", amzDate)
	q.Set("X-Amz-Expires", fmt.Sprintf("%d", int(ttl.Seconds())))
	q.Set("X-Amz-SignedHeaders", "host")
	canonicalQuery := canonicalQueryString(q)

	canonicalURI := "/" + p.cfg.Bucket + "/" + s3EscapePath(key)
	canonicalHeaders := "host:" + host + "\n"
	signedHeaders := "host"

	canonicalRequest := strings.Join([]string{
		http.MethodGet,
		canonicalURI,
		canonicalQuery,
		canonicalHeaders,
		signedHeaders,
		"UNSIGNED-PAYLOAD",
	}, "\n")
	crHash := sha256.Sum256([]byte(canonicalRequest))
	stringToSign := strings.Join([]string{
		"AWS4-HMAC-SHA256",
		amzDate,
		scope,
		hex.EncodeToString(crHash[:]),
	}, "\n")

	kDate := hmacSHA256([]byte("AWS4"+p.cfg.SecretKey), []byte(dateStamp))
	kRegion := hmacSHA256(kDate, []byte(p.cfg.Region))
	kService := hmacSHA256(kRegion, []byte("s3"))
	kSigning := hmacSHA256(kService, []byte("aws4_request"))
	sig := hex.EncodeToString(hmacSHA256(kSigning, []byte(stringToSign)))

	return fmt.Sprintf("%s%s?%s&X-Amz-Signature=%s", endpoint, canonicalURI, canonicalQuery, sig), nil
}

// s3EscapePath URI-encodes each path segment per SigV4 while keeping the
// '/' separators. awsURIEscape escapes '/' too (correct for a query value,
// wrong for a path), so we split and rejoin.
func s3EscapePath(p string) string {
	parts := strings.Split(p, "/")
	for i, seg := range parts {
		parts[i] = awsURIEscape(seg)
	}
	return strings.Join(parts, "/")
}
