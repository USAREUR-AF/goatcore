package handlers

import (
	"fmt"
	"net/url"
	"os"
	"strings"

	"gopkg.in/yaml.v3"
)

const (
	authorityUngoverned = "ungoverned"
	authorityReport     = "report"
	authorityEnforce    = "enforce"
	authorityUnknown    = "unknown"
)

type membershipAuthority struct {
	Mode      string
	Detail    string
	Procedure string
}

func (a membershipAuthority) AllowsLiveRelease() bool {
	return a.Mode == authorityUngoverned || a.Mode == authorityReport
}

// membershipAuthority reads the explicit deployment mode and, for warrant
// scopes, the installed reviewed declaration on each request. Missing or
// inconsistent configuration is unknown and cannot authorize a live write.
func (s *Server) membershipAuthority() membershipAuthority {
	site := strings.TrimSpace(s.Cfg.NotifyGoatnet)
	procedure := strings.TrimRight(s.Cfg.PlatformRepoURL, "/") + "/tree/main/ops/warrant/membership/" + url.PathEscape(site)
	switch strings.TrimSpace(s.Cfg.MembershipAuthority) {
	case authorityUngoverned:
		return membershipAuthority{Mode: authorityUngoverned, Detail: "live membership is not warrant-governed", Procedure: procedure}
	case "scope":
		// Continue below and require a valid declaration.
	default:
		return membershipAuthority{Mode: authorityUnknown, Detail: "membership authority is not explicitly configured", Procedure: procedure}
	}
	path := strings.TrimSpace(s.Cfg.MembershipScopePath)
	if path == "" {
		return membershipAuthority{Mode: authorityUnknown, Detail: "membership authority scope path is not configured", Procedure: procedure}
	}
	body, err := os.ReadFile(path)
	if err != nil {
		return membershipAuthority{Mode: authorityUnknown, Detail: fmt.Sprintf("membership authority cannot be read: %v", err), Procedure: procedure}
	}
	var scope struct {
		Version  int    `yaml:"version"`
		Scope    string `yaml:"scope"`
		Converge string `yaml:"converge"`
	}
	if err := yaml.Unmarshal(body, &scope); err != nil {
		return membershipAuthority{Mode: authorityUnknown, Detail: fmt.Sprintf("membership authority cannot be parsed: %v", err), Procedure: procedure}
	}
	if scope.Version != 1 || site == "" || strings.TrimSpace(scope.Scope) != site {
		return membershipAuthority{Mode: authorityUnknown, Detail: "membership authority declaration does not match this goatnet", Procedure: procedure}
	}
	switch strings.TrimSpace(scope.Converge) {
	case authorityReport:
		return membershipAuthority{Mode: authorityReport, Detail: "warrant membership is report-only; live release is allowed", Procedure: procedure}
	case authorityEnforce:
		return membershipAuthority{Mode: authorityEnforce, Detail: "membership is warrant-governed; remove the hold from the reviewed declaration and let the converger apply it", Procedure: procedure}
	default:
		return membershipAuthority{Mode: authorityUnknown, Detail: "membership authority declaration has an unknown converge mode", Procedure: procedure}
	}
}
