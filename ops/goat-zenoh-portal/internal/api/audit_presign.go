package api

// Operators read metadata through an authenticated projection of the fixed
// audit bucket. Body replay remains gated pending owner governance.

import (
	"fmt"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/audittail"
)

const (
	// Presigned URLs live just long enough for a slow MinIO reply; a leaked
	// URL ages out fast.
	auditPresignTTL = 5 * time.Minute
	// Ceiling on objects (and therefore signed URLs) per call. A wider
	// window than this should be split client-side.
	auditPresignMaxObjects = 500
	// Hardest a single call may reach back, so one request can't try to
	// presign weeks of objects. The MinIO middle tier retains ~30d.
	auditPresignMaxWindow = 7 * 24 * time.Hour
)

type auditPresignObject struct {
	Key           string `json:"key"`
	URL           string `json:"url"`
	Source        string `json:"source,omitempty"`
	SchemaVersion int    `json:"schema_version,omitempty"`
	AuditURL      string `json:"audit_url,omitempty"`
}

type auditPresignResponse struct {
	Bucket     string               `json:"bucket"`
	Since      string               `json:"since"`
	Key        string               `json:"key,omitempty"`
	TTLSeconds int                  `json:"ttl_seconds"`
	Count      int                  `json:"count"`
	Truncated  bool                 `json:"truncated"`
	Objects    []auditPresignObject `json:"objects"`
}

// All normal history reads pass through a metadata projection, including V2.
func (s *Server) auditReader() *audittail.Presigner {
	return audittail.NewPresigner(audittail.Config{Endpoint: s.cfg.AuditS3Endpoint, CABundle: s.cfg.AuditS3CABundle, Bucket: s.cfg.AuditS3Bucket, Region: s.cfg.AuditS3Region, AccessKey: s.cfg.AuditS3AccessKey, SecretKey: s.cfg.AuditS3SecretKey, SubscriberID: s.cfg.AuditSubscriberID})
}

func (s *Server) handleReplayPresign(w http.ResponseWriter, r *http.Request) {
	writeError(w, http.StatusServiceUnavailable, "replay_gated", "replay is unavailable pending owner governance")
}

func (s *Server) handleAuditPresign(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("projection") != "1" {
		writeError(w, http.StatusUpgradeRequired, "upgrade_required", "metadata projection support is required")
		return
	}
	pre := s.auditReader()
	if !pre.Enabled() {
		writeError(w, http.StatusServiceUnavailable, "audit", "audit history is not configured")
		return
	}
	since, err := parseAuditSince(strings.TrimSpace(r.URL.Query().Get("since")))
	if err != nil {
		writeError(w, http.StatusBadRequest, "decode", err.Error())
		return
	}
	if err := s.reg.LogAuditEvent(r.Context(), actorOf(r), "audit.metadata.list", r.URL.Query().Get("since"), ""); err != nil {
		writeError(w, http.StatusInternalServerError, "audit", "cannot record history access")
		return
	}
	keys, err := pre.ListAfter(r.Context(), since, auditPresignMaxObjects+1, r.URL.Query().Get("after"))
	if err != nil {
		writeError(w, http.StatusBadGateway, "audit", "cannot list audit metadata")
		return
	}
	truncated := len(keys) > auditPresignMaxObjects
	if truncated {
		keys = keys[:auditPresignMaxObjects]
	}
	objects := make([]auditPresignObject, 0, len(keys))
	for _, key := range keys {
		source, version, err := audittail.ObjectSource(key, s.cfg.AuditSubscriberID)
		if err != nil {
			writeError(w, http.StatusBadGateway, "audit", "invalid stored audit key")
			return
		}
		projection := "metadata-projection"
		if source == "legacy" {
			projection = "legacy-projection"
		}
		objects = append(objects, auditPresignObject{Key: key, URL: "/api/audit/object?key=" + url.QueryEscape(key), Source: projection, SchemaVersion: version})
	}
	writeJSON(w, http.StatusOK, auditPresignResponse{Bucket: s.cfg.AuditS3Bucket, Since: since.Format(time.RFC3339Nano), Key: r.URL.Query().Get("key"), Count: len(objects), Truncated: truncated, Objects: objects})
}

func (s *Server) handleAuditObject(w http.ResponseWriter, r *http.Request) {
	key := r.URL.Query().Get("key")
	if _, _, err := audittail.ObjectSource(key, s.cfg.AuditSubscriberID); err != nil {
		writeError(w, http.StatusBadRequest, "audit", "invalid audit object key")
		return
	}
	pre := s.auditReader()
	if !pre.Enabled() {
		writeError(w, http.StatusServiceUnavailable, "audit", "audit history is not configured")
		return
	}
	if err := s.reg.LogAuditEvent(r.Context(), actorOf(r), "audit.metadata.read", key, ""); err != nil {
		writeError(w, http.StatusInternalServerError, "audit", "cannot record history access")
		return
	}
	body, err := pre.MetadataObject(r.Context(), key)
	if err != nil {
		writeError(w, http.StatusBadGateway, "audit", "cannot project audit metadata")
		return
	}
	w.Header().Set("Content-Type", "application/x-ndjson")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(body)
}

func parseAuditSince(s string) (time.Time, error) {
	now := time.Now().UTC()
	floor := now.Add(-auditPresignMaxWindow)
	clamp := func(t time.Time) time.Time {
		if t.Before(floor) {
			return floor
		}
		if t.After(now) {
			return now
		}
		return t
	}
	if s == "" {
		return now.Add(-1 * time.Hour), nil
	}
	if d, err := time.ParseDuration(s); err == nil {
		if d < 0 {
			d = -d
		}
		return clamp(now.Add(-d)), nil
	}
	if t, err := time.Parse(time.RFC3339, s); err == nil {
		return clamp(t.UTC()), nil
	}
	return time.Time{}, fmt.Errorf("invalid since %q (use a duration like 1h, or an RFC3339 timestamp)", s)
}

// A replay window can span legacy audit and new replay buckets; per-object
// source + URL are authoritative, never a misleading top-level bucket name.
func historyBucket(replay bool, audit string) string {
	if replay {
		return ""
	}
	return audit
}
