package acl

import (
	"context"
	"net"
)

// SetDialForTest replaces the Docker socket dialer so a test can see whether,
// and to which path, a SIGHUP dials.
func SetDialForTest(r *Renderer, dial func(ctx context.Context, path string) (net.Conn, error)) {
	r.dial = dial
}
