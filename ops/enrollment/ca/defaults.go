package ca

import (
	"os"
	"path/filepath"
)

const (
	// DefaultCAIDStem is the middle of the CA id ca-init generates when no
	// --id is given: "<mode>-" + DefaultCAIDStem + "-<date>".
	DefaultCAIDStem = "goat-ca"
	// YubikeyCertSubjectPrefix prefixes the CA id in the CN of the
	// self-signed cert YubikeyGenerate installs in the PIV slot when no
	// CertSubject is given.
	YubikeyCertSubjectPrefix = "goat-ca-"
	// auditLogDir is the directory under the home directory that holds the
	// signing tools' default audit log.
	auditLogDir = ".goat"
)

// DefaultAuditLogPath is the default append-only JSON audit log shared by
// cert-sign, intent-sign and name-snapshot-sign. Without a home directory it
// falls back to ./ca-audit.jsonl. A project that keeps its audit log
// elsewhere passes --audit-log on every call; the drivers in ops/scripts and
// goatbuilder pass the file beside the project's state directory.
func DefaultAuditLogPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return "./ca-audit.jsonl"
	}
	return filepath.Join(home, auditLogDir, "ca-audit.jsonl")
}
