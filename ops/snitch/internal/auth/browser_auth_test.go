package auth

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
)

func TestBrowserAuthenticationChecksLiveRoleAndRevocation(t *testing.T) {
	dsn := os.Getenv("TEST_PG_DSN")
	if dsn == "" {
		t.Skip("TEST_PG_DSN is not set")
	}
	ctx := context.Background()
	admin, err := pgxpool.New(ctx, dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer admin.Close()
	schema := fmt.Sprintf("browser_auth_%d", time.Now().UnixNano())
	if _, err := admin.Exec(ctx, "CREATE SCHEMA "+schema); err != nil {
		t.Fatal(err)
	}
	defer func() {
		if _, err := admin.Exec(ctx, "DROP SCHEMA "+schema+" CASCADE"); err != nil {
			t.Error(err)
		}
	}()
	cfg, err := pgxpool.ParseConfig(dsn)
	if err != nil {
		t.Fatal(err)
	}
	cfg.ConnConfig.RuntimeParams["search_path"] = schema
	pool, err := pgxpool.NewWithConfig(ctx, cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()
	if _, err := pool.Exec(ctx, `CREATE TABLE api_keys (
  id BIGSERIAL PRIMARY KEY, key_hash TEXT UNIQUE NOT NULL,
  role TEXT NOT NULL, label TEXT NOT NULL, created_at BIGINT NOT NULL, last_used BIGINT
 )`); err != nil {
		t.Fatal(err)
	}
	auth := New(pool)
	key, err := auth.CreateKey(ctx, RoleViewer, "synthetic-browser-user")
	if err != nil {
		t.Fatal(err)
	}
	login := sessionRequest(http.MethodPost)
	login.Header.Set("Authorization", "Bearer "+key)
	created := httptest.NewRecorder()
	auth.BrowserSession(created, login)
	if created.Code != http.StatusNoContent {
		t.Fatalf("login: %d %s", created.Code, created.Body.String())
	}
	cookie := created.Result().Cookies()[0]
	request := sessionRequest(http.MethodGet)
	request.AddCookie(cookie)
	ok := func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusNoContent) }
	viewer := httptest.NewRecorder()
	auth.Require(RoleViewer, ok).ServeHTTP(viewer, request)
	if viewer.Code != http.StatusNoContent {
		t.Fatalf("viewer: %d", viewer.Code)
	}
	forbidden := httptest.NewRecorder()
	auth.Require(RoleAdmin, ok).ServeHTTP(forbidden, request)
	if forbidden.Code != http.StatusForbidden {
		t.Fatal("cookie bypassed role check")
	}
	if _, err := pool.Exec(ctx, "DELETE FROM api_keys"); err != nil {
		t.Fatal(err)
	}
	revoked := httptest.NewRecorder()
	auth.Require(RoleViewer, ok).ServeHTTP(revoked, request)
	if revoked.Code != http.StatusUnauthorized {
		t.Fatal("revoked key remained valid through session")
	}
}
