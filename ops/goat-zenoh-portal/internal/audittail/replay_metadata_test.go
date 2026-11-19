package audittail

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
)

func TestSharedHelloFixtures(t *testing.T) {
	raw, err := os.ReadFile("testdata/hello.json") // same bytes as goatstuff ops/zenoh-trial-peers/audit-subscriber/fixtures/hello.json
	if err != nil {
		t.Fatal(err)
	}
	var cases []struct {
		Key, Body string
		Hello     bool
	}
	if err := json.Unmarshal(raw, &cases); err != nil {
		t.Fatal(err)
	}
	for _, c := range cases {
		prefix := strings.Split(c.Key, "/")[0]
		ev := auditEvent{KeyExpr: c.Key, PayloadBytesB64: base64.StdEncoding.EncodeToString([]byte(c.Body))}
		if isHelloWorld(&ev, prefix) != c.Hello {
			t.Errorf("legacy mismatch for %q", c.Key)
		}
		ev.PayloadBytesB64 = ""
		ev.SchemaVersion = 2
		ev.OnboardingClassification = "other"
		if c.Hello {
			ev.OnboardingClassification = "onboarding_hello"
		}
		if isHelloWorld(&ev, prefix) != c.Hello {
			t.Errorf("metadata mismatch for %q", c.Key)
		}
	}
}

func TestV2OnboardingDoesNotFetchReplayAndRemainsExactlyOnce(t *testing.T) {
	key := "site/2026/10/26/12/1793000000000-2-v2-replay.ndjson"
	objects := map[string]string{key: `{"schema_version":2,"sample_kind":"put","vendor":"aardwolf","key_expr":"aardwolf/hello/v1","ts":"2026-10-26T12:00:00Z","onboarding_classification":"onboarding_hello"}
{"schema_version":2,"sample_kind":"put","vendor":"aardwolf","key_expr":"aardwolf/data/v1","ts":"2026-10-26T12:00:01Z","onboarding_classification":"other"}`}
	store := httptest.NewTLSServer((&fakeS3{bucket: "audit-integrity", objects: objects}).handler())
	defer store.Close()
	reg := openReg(t)
	ctx := context.Background()
	if err := reg.CreateVendor(ctx, &registry.Vendor{Prefix: "aardwolf", APIKeyHash: "fixture"}); err != nil {
		t.Fatal(err)
	}
	tail := New(Config{Endpoint: store.URL, CABundle: fixtureCA(t, store), Bucket: "audit-integrity", AccessKey: "fixture", SecretKey: "fixture", SubscriberID: "site"}, reg)
	n, cursor, err := tail.PullOnce(ctx)
	if err != nil || n != 2 || cursor != key {
		t.Fatalf("first pull: %d %s %v", n, cursor, err)
	}
	rows, err := reg.ListProgress(ctx, true)
	if err != nil || len(rows) != 1 || rows[0].L3At == nil || rows[0].L4At == nil {
		t.Fatalf("progress not preserved: %v", err)
	}
	n, _, err = tail.PullOnce(ctx)
	if err != nil || n != 0 {
		t.Fatalf("duplicate progress on re-pull: %d %v", n, err)
	}
	bad := "site/2026/10/26/12/1793000000001-1-v2-replay.ndjson"
	objects[bad] = `{"schema_version":2,"sample_kind":"put","vendor":"aardwolf","key_expr":"aardwolf/hello/v1","onboarding_classification":"unknown"}`
	_, _, err = tail.PullOnce(ctx)
	if err == nil {
		t.Fatal("unknown classification silently accepted")
	}
	c, _ := reg.GetAuditCursor(ctx, "site")
	if c.LastKey != key {
		t.Fatal("cursor advanced over invalid metadata")
	}
}

func TestObjectSourceRefusesUnknownAndForeignKeys(t *testing.T) {
	for _, suffix := range []string{"1.ndjson", "1-v2-meta.ndjson", "1-v2-replay.ndjson"} {
		if _, _, err := ObjectSource("site/2026/10/26/12/1793000000000-"+suffix, "site"); err != nil {
			t.Fatal(err)
		}
	}
	for _, key := range []string{"foreign/2026/10/26/12/1793000000000-1.ndjson", "site/../../12/1793000000000-1.ndjson", "site/2026/10/26/12/1793000000000-1-v3-replay.ndjson"} {
		if _, _, err := ObjectSource(key, "site"); err == nil {
			t.Fatalf("accepted %q", key)
		}
	}
}

func TestV2InvalidMetadataNeverAdvancesCursor(t *testing.T) {
	for name, body := range map[string]string{
		"empty body field": `{"schema_version":2,"sample_kind":"put","onboarding_classification":"other","payload_bytes_b64":""}`,
		"null body field":  `{"schema_version":2,"sample_kind":"put","onboarding_classification":"other","payload_bytes_b64":null}`,
		"schema mismatch":  `{"schema_version":1,"onboarding_classification":"other"}`,
		"malformed":        `{broken`,
	} {
		t.Run(name, func(t *testing.T) {
			key := "site/2026/10/26/12/1793000000000-1-v2-meta.ndjson"
			store := httptest.NewTLSServer((&fakeS3{bucket: "audit-integrity", objects: map[string]string{key: body}}).handler())
			defer store.Close()
			reg := openReg(t)
			tail := New(Config{Endpoint: store.URL, CABundle: fixtureCA(t, store), Bucket: "audit-integrity", AccessKey: "fixture", SecretKey: "fixture", SubscriberID: "site"}, reg)
			if _, _, err := tail.PullOnce(context.Background()); err == nil {
				t.Fatal("invalid V2 metadata accepted")
			}
			cursor, err := reg.GetAuditCursor(context.Background(), "site")
			if err != nil || cursor.LastKey != "" {
				t.Fatalf("cursor changed: %+v %v", cursor, err)
			}
		})
	}
}
