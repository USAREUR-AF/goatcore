package api

import (
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/kratosadmin"
)

func TestNationalityIsReadFromAdminMetadataOnly(t *testing.T) {
	cases := []struct {
		name string
		id   kratosadmin.Identity
		want string
	}{
		{"stamped", kratosadmin.Identity{MetadataPublic: map[string]interface{}{"nationality": " XCC "}}, "XCC"},
		{"absent", kratosadmin.Identity{MetadataPublic: map[string]interface{}{"aff": map[string]interface{}{"nation": "XAA"}}}, ""},
		// A self-editable trait is never read: a person cannot assert it.
		{"trait only", kratosadmin.Identity{Traits: map[string]interface{}{"nationality": "XCC"}}, ""},
	}
	for _, c := range cases {
		if got := identityNationality(c.id); got != c.want {
			t.Errorf("%s: identityNationality = %q, want %q", c.name, got, c.want)
		}
	}
}
