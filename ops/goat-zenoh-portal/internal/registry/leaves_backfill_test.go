package registry_test

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"math/big"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

func writeBundle(t *testing.T, dir, name, member, cn string, serial int64, notAfter time.Time) {
	t.Helper()
	key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	tmpl := &x509.Certificate{SerialNumber: big.NewInt(serial), Subject: pkix.Name{CommonName: cn},
		NotBefore: notAfter.Add(-365 * 24 * time.Hour), NotAfter: notAfter}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	body := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	_ = tw.WriteHeader(&tar.Header{Name: member, Mode: 0o644, Size: int64(len(body))})
	_, _ = tw.Write(body)
	_ = tw.Close()
	_ = gz.Close()
	if err := os.WriteFile(filepath.Join(dir, name), buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

// Migration step 2: leaves still in onboarding tarballs are recorded with
// their serial and validity; a tarball without a slot leaf is skipped.
func TestBundleLeafBackfill(t *testing.T) {
	ctx := context.Background()
	r := open(t)
	if _, err := r.InitializeSlotPool(ctx, "bf-test", 2); err != nil {
		t.Fatal(err)
	}
	a0 := registry.SlotAnchor("bf-test", 0)
	if _, err := r.DB().ExecContext(ctx, `UPDATE slots SET assigned_to='p0', assigned_at=1 WHERE slot_id=$1`, a0); err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	exp := time.Now().Add(100 * 24 * time.Hour).Truncate(time.Second)
	writeBundle(t, dir, "goat-onboarding-"+a0+"-1.tar.gz", a0+"-cert.pem", a0, 0xabc, exp)
	writeBundle(t, dir, "goat-onboarding-other.tar.gz", "vendor-cert.pem", "acme-trial-peer", 1, exp)
	writeBundle(t, dir, "goat-onboarding-unknown.tar.gz", "ffffffffffffffffffffffffffffffff-cert.pem", "ffffffffffffffffffffffffffffffff", 2, exp)
	leaves, skipped, err := registry.ReadBundleLeaves(dir)
	if err != nil || len(leaves) != 2 || skipped != 1 {
		t.Fatalf("ReadBundleLeaves = %d leaves, %d skipped, %v; want 2, 1", len(leaves), skipped, err)
	}
	n, err := r.RecordBundleLeaves(ctx, leaves)
	if err != nil || n != 1 {
		t.Fatalf("RecordBundleLeaves = %d, %v; want 1 (the off-pool anchor ignored)", n, err)
	}
	got, _ := r.SlotLeaves(ctx, a0)
	if len(got) != 1 || got[0].Serial != "0abc" || got[0].NotAfter != exp.Unix() || got[0].SupersededAt != nil {
		t.Fatalf("recorded leaf = %+v", got)
	}
	if n, _ := r.RecordBundleLeaves(ctx, leaves); n != 0 {
		t.Fatalf("second run added %d, want 0", n)
	}
}
