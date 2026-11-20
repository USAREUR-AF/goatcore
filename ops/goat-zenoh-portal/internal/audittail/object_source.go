package audittail

import (
	"fmt"
	"regexp"
	"strings"
)

var auditObjectName = regexp.MustCompile(`^[0-9]{13}-[1-9][0-9]*(?:-v2-(?:meta|replay))?\.ndjson$`)

// ObjectSource accepts only the site's own chronological objects. This is
// also the server-side allowlist for legacy access; request parameters never
// select a bucket or arbitrary key. Unknown versions fail closed.
func ObjectSource(key, subscriber string) (string, int, error) {
	parts := strings.Split(key, "/")
	if subscriber == "" || len(parts) != 6 || parts[0] != subscriber || !auditObjectName.MatchString(parts[5]) {
		return "", 0, fmt.Errorf("invalid audit object key")
	}
	for i, part := range parts[1:5] {
		width := 2
		if i == 0 {
			width = 4
		}
		if len(part) != width {
			return "", 0, fmt.Errorf("invalid audit date partition")
		}
		for _, c := range part {
			if c < '0' || c > '9' {
				return "", 0, fmt.Errorf("invalid audit date partition")
			}
		}
	}
	if strings.HasSuffix(key, "-v2-replay.ndjson") {
		return "replay", 2, nil
	}
	if strings.HasSuffix(key, "-v2-meta.ndjson") {
		return "metadata", 2, nil
	}
	return "legacy", 1, nil
}
