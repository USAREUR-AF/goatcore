package api

import (
	"context"
	"net/http/httptest"
	"testing"
)

// X-Goat-Operator-Actor names the human only for a service key; a
// per-person key always records its own holder.
func TestActorOf_delegatedHeaderOnlyForServiceKeys(t *testing.T) {
	cases := []struct {
		actor, header, want string
	}{
		{"id-console", "alice", "alice"},
		{"operator", "alice", "alice"},
		{"jane-operator", "alice", "jane-operator"},
		{"jane-operator", "", "jane-operator"},
		{"id-console", "", "id-console"},
	}
	for _, c := range cases {
		r := httptest.NewRequest("POST", "/api/orgs", nil)
		if c.header != "" {
			r.Header.Set("X-Goat-Operator-Actor", c.header)
		}
		r = r.WithContext(context.WithValue(r.Context(), principalKey, &Principal{Operator: true, Actor: c.actor}))
		if got := actorOf(r); got != c.want {
			t.Errorf("key %q with header %q: actorOf = %q, want %q", c.actor, c.header, got, c.want)
		}
	}
}
