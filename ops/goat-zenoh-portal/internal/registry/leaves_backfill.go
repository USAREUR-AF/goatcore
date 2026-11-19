package registry

import (
	"archive/tar"
	"compress/gzip"
	"context"
	"crypto/x509"
	"encoding/hex"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/jmoiron/sqlx"
)

// GHSA-rr3p migration step 2: record the leaves whose onboarding tarballs are
// still on disk, with their real serial and validity. Migration 2 already
// holds every slot with history in quarantine through an "unknown leaf" row
// (latest stamp + 366 days, no serial); those rows stay. This only adds the
// leaves that can be identified, so the record shows which serials were
// issued for which slot. A leaf minted at collection left no tarball and stays
// covered by the unknown row alone.

// BundleLeaf is one leaf read out of an onboarding tarball.
type BundleLeaf struct {
	SlotID     string
	Generation int64
	Serial     string
	NotBefore  int64
	NotAfter   int64
}

var bundleCertRe = regexp.MustCompile(`(?:^|/)([0-9a-f]{32})-cert\.pem$`)

// ReadBundleLeaves walks dir for onboarding tarballs (*.tar.gz) and returns
// the slot leaf in each one. A tarball that is unreadable or holds no slot
// leaf is skipped and counted in skipped.
func ReadBundleLeaves(dir string) (leaves []BundleLeaf, skipped int, err error) {
	err = filepath.WalkDir(dir, func(p string, d fs.DirEntry, werr error) error {
		if werr != nil {
			return werr
		}
		if d.IsDir() || !strings.HasSuffix(p, ".tar.gz") {
			return nil
		}
		l, ok := readBundleLeaf(p)
		if !ok {
			skipped++
			return nil
		}
		leaves = append(leaves, l)
		return nil
	})
	return leaves, skipped, err
}

func readBundleLeaf(path string) (BundleLeaf, bool) {
	f, err := os.Open(path)
	if err != nil {
		return BundleLeaf{}, false
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		return BundleLeaf{}, false
	}
	tr := tar.NewReader(gz)
	for {
		h, err := tr.Next()
		if err != nil {
			return BundleLeaf{}, false
		}
		m := bundleCertRe.FindStringSubmatch(h.Name)
		if m == nil {
			continue
		}
		raw, err := io.ReadAll(io.LimitReader(tr, 1<<20))
		if err != nil {
			return BundleLeaf{}, false
		}
		blk, _ := pem.Decode(raw)
		if blk == nil || blk.Type != "CERTIFICATE" {
			return BundleLeaf{}, false
		}
		c, err := x509.ParseCertificate(blk.Bytes)
		if err != nil {
			return BundleLeaf{}, false
		}
		anchor, gen, ok := AnchorOfCN(c.Subject.CommonName)
		if !ok || anchor != m[1] {
			return BundleLeaf{}, false
		}
		return BundleLeaf{SlotID: anchor, Generation: gen, Serial: hex.EncodeToString(c.SerialNumber.Bytes()),
			NotBefore: c.NotBefore.Unix(), NotAfter: c.NotAfter.Unix()}, true
	}
}

// RecordBundleLeaves inserts each leaf not already on file (by slot and
// serial). A leaf is current only when its slot is assigned at that leaf's
// generation; any other is recorded as superseded. Leaves for anchors not in
// the pool are ignored. Returns how many rows were added.
func (r *Registry) RecordBundleLeaves(ctx context.Context, leaves []BundleLeaf) (int, error) {
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return 0, err
	}
	defer func() { _ = tx.Rollback() }()
	added, err := recordBundleLeavesTx(ctx, tx, leaves)
	if err != nil {
		return 0, err
	}
	if err := tx.Commit(); err != nil {
		return 0, err
	}
	return added, nil
}

// ReissueCandidatesWithBundles is ReissueCandidates as it would be after
// RecordBundleLeaves(leaves), without keeping anything: the leaves are written
// inside a transaction that is rolled back. It is how a dry run counts slots
// whose only evidence of an earlier leaf is a tarball on disk.
func (r *Registry) ReissueCandidatesWithBundles(ctx context.Context, leaves []BundleLeaf) ([]ReissueCandidate, error) {
	tx, err := r.beginLocked(ctx)
	if err != nil {
		return nil, err
	}
	defer func() { _ = tx.Rollback() }()
	if _, err := recordBundleLeavesTx(ctx, tx, leaves); err != nil {
		return nil, err
	}
	return reissueCandidates(ctx, tx)
}

func recordBundleLeavesTx(ctx context.Context, tx *sqlx.Tx, leaves []BundleLeaf) (int, error) {
	added := 0
	now := time.Now().Unix()
	for _, l := range leaves {
		var cur struct {
			Generation int64   `db:"generation"`
			AssignedTo *string `db:"assigned_to"`
			EnrolledBy *string `db:"enrolled_by"`
			IdentityID *string `db:"identity_id"`
		}
		err := tx.GetContext(ctx, &cur, `SELECT generation, assigned_to, enrolled_by, identity_id FROM slots WHERE slot_id = $1`, l.SlotID)
		if errors.Is(err, sqlErrNoRows()) {
			continue
		}
		if err != nil {
			return added, fmt.Errorf("record bundle leaves: %w", err)
		}
		var superseded any
		if cur.AssignedTo == nil || cur.Generation != l.Generation {
			superseded = now
		}
		custodian := ""
		if cur.EnrolledBy != nil {
			custodian = *cur.EnrolledBy // the machine's custodian; a revoke leaves it in place
		}
		issuer := "" // a current leaf is the linked person's; an older one is unknown
		if superseded == nil && cur.IdentityID != nil {
			issuer = *cur.IdentityID
		}
		res, err := tx.ExecContext(ctx,
			`INSERT INTO slot_leaves (slot_id, generation, serial, not_before, not_after, issued_for_identity, issued_for_custodian, issued_at, superseded_at)
			 SELECT $1, $2, $3, $4, $5, $8, $7, $4, $6::BIGINT
			  WHERE NOT EXISTS (SELECT 1 FROM slot_leaves WHERE slot_id = $1 AND serial = $3)`,
			l.SlotID, l.Generation, l.Serial, l.NotBefore, l.NotAfter, superseded, custodian, issuer)
		if err != nil {
			return added, fmt.Errorf("record bundle leaves: %w", err)
		}
		if n, _ := res.RowsAffected(); n > 0 {
			added++
		}
	}
	return added, nil
}
