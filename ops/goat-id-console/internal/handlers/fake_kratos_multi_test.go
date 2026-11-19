package handlers

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/kratos"
)

// fakeIdent is one identity in fakeKratosMulti. Groups live in
// metadata_public, as every mint writes them.
type fakeIdent struct {
	id, email, name, state string
	groups                 []string
}

// multiKratosRT serves several identities: list, get, patch (state and
// metadata_public groups) and delete. Session lookups answer an empty list.
type multiKratosRT struct {
	mu    sync.Mutex
	order []string
	byID  map[string]map[string]interface{}
}

func (rt *multiKratosRT) RoundTrip(req *http.Request) (*http.Response, error) {
	rt.mu.Lock()
	defer rt.mu.Unlock()
	w := httptest.NewRecorder()
	w.Header().Set("Content-Type", "application/json")
	path := req.URL.Path
	switch {
	case req.Method == http.MethodGet && path == "/admin/identities":
		out := []interface{}{}
		if req.URL.Query().Get("page") == "" || req.URL.Query().Get("page") == "0" {
			for _, id := range rt.order {
				if c, ok := rt.byID[id]; ok {
					out = append(out, c)
				}
			}
		}
		_ = json.NewEncoder(w).Encode(out)
	case strings.HasPrefix(path, "/admin/identities/") && strings.HasSuffix(path, "/sessions"):
		_, _ = w.Write([]byte("[]"))
	case strings.HasPrefix(path, "/admin/identities/"):
		id := strings.TrimPrefix(path, "/admin/identities/")
		cur, ok := rt.byID[id]
		if !ok {
			w.WriteHeader(http.StatusNotFound)
			_, _ = w.Write([]byte(`{"error":{"code":404}}`))
			break
		}
		switch req.Method {
		case http.MethodGet:
			_ = json.NewEncoder(w).Encode(cur)
		case http.MethodDelete:
			delete(rt.byID, id)
			w.WriteHeader(http.StatusNoContent)
		case http.MethodPatch:
			var ops []map[string]interface{}
			_ = json.NewDecoder(req.Body).Decode(&ops)
			for _, op := range ops {
				switch op["path"] {
				case "/state":
					cur["state"] = op["value"]
				case "/metadata_public/groups", "/traits/groups":
					cur["metadata_public"].(map[string]interface{})["groups"] = op["value"]
				}
			}
			_ = json.NewEncoder(w).Encode(cur)
		default:
			w.WriteHeader(http.StatusMethodNotAllowed)
		}
	default:
		w.WriteHeader(http.StatusNotFound)
	}
	return w.Result(), nil
}

// fakeKratosMulti returns a client over the given identities and a reader for
// one identity's current groups and state ("" state once deleted).
func fakeKratosMulti(t *testing.T, idents ...fakeIdent) (*kratos.Client, func(id string) ([]string, string)) {
	t.Helper()
	rt := &multiKratosRT{byID: map[string]map[string]interface{}{}}
	for _, fi := range idents {
		gi := make([]interface{}, len(fi.groups))
		for i, g := range fi.groups {
			gi[i] = g
		}
		rt.order = append(rt.order, fi.id)
		rt.byID[fi.id] = map[string]interface{}{
			"id": fi.id, "state": fi.state,
			"traits":          map[string]interface{}{"email": fi.email, "name": fi.name},
			"metadata_public": map[string]interface{}{"groups": gi},
		}
	}
	read := func(id string) ([]string, string) {
		rt.mu.Lock()
		defer rt.mu.Unlock()
		cur, ok := rt.byID[id]
		if !ok {
			return nil, ""
		}
		var out []string
		arr, _ := cur["metadata_public"].(map[string]interface{})["groups"].([]interface{})
		for _, g := range arr {
			out = append(out, g.(string))
		}
		return out, cur["state"].(string)
	}
	cli := kratos.NewClient("https://kratos.example.test")
	cli.HTTP = &http.Client{Transport: rt}
	return cli, read
}
