package main

// Collab inventory reconcile + the durable record.
//
// The collab plane is deliberately severed from the mesh — but its identity
// INVENTORY was severed too: the only list of who holds collab access was the
// Kanidm instance itself, which nothing queried or mirrored, so a revoked person
// kept chat/wiki/file access invisibly. This file closes that with the smallest
// honest mechanism:
//
//   READER   — the manifests' schema-defined `collab` block
//              (ops/operators/schema/operator-manifest.schema.json §collab:
//              enabled / provider / activated / activated_at / account) plus the
//              committed roster projection (docs/operators/roster.json) as the
//              fallback source, since manifests are gitignored and may be absent
//              on the host running this.
//   RECONCILE— provider accounts × identity records → a per-handle delta with a
//              verdict, so "who has collab that the stores don't know about" and
//              "who is revoked but still has collab" are both one command away.
//   WRITER   — the backfill: reconcile EMITS a patch document; a separate,
//              -yes-gated apply writes the observed account into each manifest's
//              collab block. Emit and apply are split on purpose — the live
//              backfill of ~200 real manifests is a deploy-pass act, not a
//              side effect of looking.
//
// Deliberately NOT here: disabling anything. A live account belonging to a
// revoked person is reported as an action candidate; cutting it stays the
// operator's explicit `-collab revoke -handle <h> -yes`
// (docs/design/collab-onboarding-provisioner.md §"Revocation is a separate
// decision").
//
// Output carries handles, never email addresses: the reconcile is meant to be
// pasteable into a ticket, and the roster's own sanitization posture (hashed
// emails, ops/ci/check-no-identity-spillage.sh) applies here too.

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

// collabLister is the read-only half of a provider that can enumerate accounts.
// Kept as a local interface (not a CollabProvider method) so the reconcile is
// unit-testable against a fixture list and never needs a live IdP.
type collabLister interface {
	List(ctx context.Context) ([]CollabAccount, error)
}

// ---------------------------------------------------------------- reader ---

// collabBlock mirrors the manifest schema's `collab` object. Absent block ⇒ zero
// value ⇒ "no collab decision recorded", which is exactly the F-274 state.
type collabBlock struct {
	Enabled     bool   `yaml:"enabled" json:"enabled"`
	Provider    string `yaml:"provider,omitempty" json:"provider,omitempty"`
	Activated   bool   `yaml:"activated,omitempty" json:"activated,omitempty"`
	ActivatedAt string `yaml:"activated_at,omitempty" json:"activated_at,omitempty"`
	Account     string `yaml:"account,omitempty" json:"account,omitempty"`
}

// identityRecord is one person as the mesh identity stores declare them — the
// half of the reconcile that is NOT the collab provider.
type identityRecord struct {
	Handle string
	Env    string // manifest subdir ("" = flat/legacy layout, or a roster-only record)
	// Manifest is the repo-relative manifest path; "" when this record came from
	// the roster (manifests are gitignored — a host may hold none).
	Manifest string
	Status   string // active | inactive | revoked | "" when the source states none
	// EmailHash is sha256(lowercased, trimmed email) — the same sanitization-safe
	// join key gen-roster.py publishes, so an account can be matched to a person
	// whose collab username differs from their mesh handle without either side
	// handling a plaintext address.
	EmailHash string
	Collab    collabBlock
}

// manifestFile is the slice of an operator manifest this reconcile reads.
type manifestFile struct {
	Identity struct {
		Handle string `yaml:"handle"`
		Email  string `yaml:"email"`
		Status string `yaml:"status"`
	} `yaml:"identity"`
	Collab collabBlock `yaml:"collab"`
}

// skipDirs are the ops/operators subdirectories that hold no manifests.
var skipDirs = map[string]bool{
	"out":         true, // generated bundle/collab hand-off files
	"schema":      true,
	"test-revoke": true, // mock-endpoint test fixtures, not real people
	".git":        true,
}

// emailHash is gen-roster.py's `_email_hash`, byte-for-byte: SHA-256 over the
// trimmed, lowercased address. Divergence here would silently break the join.
func emailHash(email string) string {
	e := strings.ToLower(strings.TrimSpace(email))
	if e == "" {
		return ""
	}
	sum := sha256.Sum256([]byte(e))
	return hex.EncodeToString(sum[:])
}

// loadManifestRecords reads every operator manifest under <repoRoot>/ops/operators.
// The handle is the FILE STEM (the authoritative key the onboard/revoke paths
// use), not identity.handle, so a manifest whose inner handle drifted still
// reconciles against the surface that named it. An unparseable manifest is
// skipped with a warning row rather than failing the run — one bad file must not
// blind the whole inventory.
func loadManifestRecords(repoRoot string) ([]identityRecord, []string, error) {
	base := filepath.Join(repoRoot, "ops", "operators")
	if _, err := os.Stat(base); err != nil {
		// No manifests on this host is normal (they are gitignored); the roster
		// fallback carries the reconcile.
		return nil, nil, nil
	}
	var out []identityRecord
	var warns []string
	err := filepath.WalkDir(base, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			if path != base && skipDirs[d.Name()] {
				return fs.SkipDir
			}
			return nil
		}
		if filepath.Ext(path) != ".yml" { // .yml.example ends in .example — skipped
			return nil
		}
		b, err := os.ReadFile(path)
		if err != nil {
			warns = append(warns, fmt.Sprintf("read %s: %v", path, err))
			return nil
		}
		var m manifestFile
		if err := yaml.Unmarshal(b, &m); err != nil {
			warns = append(warns, fmt.Sprintf("parse %s: %v", path, err))
			return nil
		}
		rel, _ := filepath.Rel(repoRoot, path)
		env, _ := filepath.Rel(base, filepath.Dir(path))
		if env == "." {
			env = ""
		}
		out = append(out, identityRecord{
			Handle:    strings.TrimSuffix(d.Name(), ".yml"),
			Env:       env,
			Manifest:  filepath.ToSlash(rel),
			Status:    strings.ToLower(strings.TrimSpace(m.Identity.Status)),
			EmailHash: emailHash(m.Identity.Email),
			Collab:    m.Collab,
		})
		return nil
	})
	if err != nil {
		return nil, warns, fmt.Errorf("walk %s: %w", base, err)
	}
	return out, warns, nil
}

// rosterFile is the committed registry projection gen-roster.py generates.
type rosterFile struct {
	Members []struct {
		Handle    string `json:"handle"`
		Type      string `json:"type"`
		Status    string `json:"status"`
		EmailHash string `json:"email_hash"`
	} `json:"members"`
}

// loadRosterRecords reads docs/operators/roster.json — the committed, sanitized
// projection of the manifests. It is the fallback identity source (and the only
// one on a host without the gitignored manifests). It carries no collab block:
// the roster projection does not yet include one, so a roster-only match can say
// "this person exists and is revoked" but never "their collab is recorded".
func loadRosterRecords(repoRoot string) ([]identityRecord, error) {
	path := filepath.Join(repoRoot, "docs", "operators", "roster.json")
	b, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var rf rosterFile
	if err := json.Unmarshal(b, &rf); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	out := make([]identityRecord, 0, len(rf.Members))
	for _, m := range rf.Members {
		out = append(out, identityRecord{
			Handle:    m.Handle,
			Status:    strings.ToLower(strings.TrimSpace(m.Status)),
			EmailHash: m.EmailHash,
		})
	}
	return out, nil
}

// identityIndex joins the two identity sources. Manifests win over the roster
// (the roster is derived FROM them, so a disagreement means the roster is stale).
type identityIndex struct {
	byHandle    map[string]*identityRecord
	byEmailHash map[string]*identityRecord
}

func newIdentityIndex(manifests, roster []identityRecord) *identityIndex {
	ix := &identityIndex{
		byHandle:    map[string]*identityRecord{},
		byEmailHash: map[string]*identityRecord{},
	}
	add := func(recs []identityRecord, isRoster bool) {
		for i := range recs {
			r := recs[i]
			if r.Handle == "" {
				continue
			}
			if _, taken := ix.byHandle[r.Handle]; taken && isRoster {
				continue // manifest already claimed this handle
			}
			ix.byHandle[r.Handle] = &r
			if r.EmailHash != "" {
				if _, taken := ix.byEmailHash[r.EmailHash]; !taken || !isRoster {
					ix.byEmailHash[r.EmailHash] = &r
				}
			}
		}
	}
	add(manifests, false)
	add(roster, true)
	return ix
}

// lookup resolves a provider account to a declared identity: by handle first
// (the normal case — the provisioner names the account after the handle), then
// by email hash (a person whose collab username differs, e.g. a second personal
// account or a pre-provisioner hand-made one). Returns the record and whether
// the match was by email rather than handle.
func (ix *identityIndex) lookup(handle, email string) (*identityRecord, bool) {
	if r, ok := ix.byHandle[handle]; ok {
		return r, false
	}
	if h := emailHash(email); h != "" {
		if r, ok := ix.byEmailHash[h]; ok {
			return r, true
		}
	}
	return nil, false
}

// ------------------------------------------------------------- reconcile ---

// Verdicts. Ordered by how much they want an operator's attention.
const (
	// A live account whose person is revoked/inactive in the identity stores —
	// the F-274 headline case. Action candidate; the cut stays operator-gated.
	verdictRevokedLiveAccount = "revoked-identity-live-account"
	// A live account matching no manifest and no roster member at all.
	verdictOrphanAccount = "account-no-identity"
	// A live account for an active person that no identity store records —
	// the backfill set.
	verdictUnrecorded = "account-unrecorded"
	// The manifest records an account the provider does not have. Reported, never
	// auto-cleared: clearing a record asserts something the reconcile cannot see
	// (was it cut deliberately, or is the provider unreachable-but-answering?).
	verdictStaleRecord = "recorded-no-account"
	// The manifest opted in to collab but no account exists — a provisioning gap
	// (the F-274 second defect produced these before the 409 fix).
	verdictDeclaredNoAccount = "declared-no-account"
	// Live account, active person, account recorded. Nothing to do.
	verdictOK = "ok"
)

// verdictRank orders the report: loudest first.
var verdictRank = map[string]int{
	verdictRevokedLiveAccount: 0,
	verdictOrphanAccount:      1,
	verdictUnrecorded:         2,
	verdictStaleRecord:        3,
	verdictDeclaredNoAccount:  4,
	verdictOK:                 5,
}

// reconcileRow is one person's cross-plane state. No email addresses by design.
type reconcileRow struct {
	Handle          string `json:"handle"`
	Verdict         string `json:"verdict"`
	LiveAccount     bool   `json:"live_account"`
	IdentityStatus  string `json:"identity_status"` // active|inactive|revoked|unknown
	Env             string `json:"env,omitempty"`
	Manifest        string `json:"manifest,omitempty"`
	RecordedAccount string `json:"recorded_account,omitempty"`
	Note            string `json:"note,omitempty"`
}

// reconcileReport is the whole delta, self-dating so a pasted copy says when it
// was true and against what.
type reconcileReport struct {
	GeneratedAt   string         `json:"generated_at"`
	Provider      string         `json:"provider"`
	Source        string         `json:"source"`
	LiveAccounts  int            `json:"live_accounts"`
	ManifestsRead int            `json:"manifests_read"`
	RosterMembers int            `json:"roster_members"`
	Counts        map[string]int `json:"counts"`
	Warnings      []string       `json:"warnings,omitempty"`
	Rows          []reconcileRow `json:"rows"`
}

// reconcileCollab computes the delta. Pure: no I/O, no clock beyond the passed
// timestamp — the whole point is that it is testable against fixtures.
func reconcileCollab(
	accts []CollabAccount,
	manifests, roster []identityRecord,
	provider, source, now string,
	warnings []string,
) reconcileReport {
	ix := newIdentityIndex(manifests, roster)
	rep := reconcileReport{
		GeneratedAt:   now,
		Provider:      provider,
		Source:        source,
		LiveAccounts:  len(accts),
		ManifestsRead: len(manifests),
		RosterMembers: len(roster),
		Counts:        map[string]int{},
		Warnings:      warnings,
	}

	seen := map[string]bool{} // identity handles already covered by a live account
	for _, a := range accts {
		row := reconcileRow{Handle: a.Handle, LiveAccount: true, IdentityStatus: "unknown"}
		rec, byEmail := ix.lookup(a.Handle, a.Email)
		switch {
		case rec == nil:
			row.Verdict = verdictOrphanAccount
			row.Note = "no manifest and no roster member for this account"
		default:
			seen[rec.Handle] = true
			row.IdentityStatus = rec.Status
			if row.IdentityStatus == "" {
				row.IdentityStatus = "unknown"
			}
			row.Env, row.Manifest, row.RecordedAccount = rec.Env, rec.Manifest, rec.Collab.Account
			if byEmail {
				row.Note = "matched by email hash to identity `" + rec.Handle + "` (collab username differs)"
			}
			switch {
			case rec.Status == "revoked" || rec.Status == "inactive":
				row.Verdict = verdictRevokedLiveAccount
			case rec.Manifest == "":
				// Roster-only: the roster projection carries no collab block, so we
				// cannot claim the account is recorded anywhere.
				row.Verdict = verdictUnrecorded
				row.Note = joinNote(row.Note, "roster-only record (manifest not present on this host) — collab state cannot be read or backfilled here")
			case rec.Collab.Account == "":
				row.Verdict = verdictUnrecorded
			default:
				row.Verdict = verdictOK
			}
		}
		rep.Rows = append(rep.Rows, row)
	}

	// Identity-side gaps: declared collab with no account, and records naming an
	// account the provider does not have.
	for i := range manifests {
		m := manifests[i]
		if seen[m.Handle] {
			continue
		}
		status := m.Status
		if status == "" {
			status = "unknown"
		}
		switch {
		case m.Collab.Account != "":
			rep.Rows = append(rep.Rows, reconcileRow{
				Handle: m.Handle, Verdict: verdictStaleRecord, IdentityStatus: status,
				Env: m.Env, Manifest: m.Manifest, RecordedAccount: m.Collab.Account,
				Note: "manifest records an account the provider does not list",
			})
		case m.Collab.Enabled && status != "revoked" && status != "inactive":
			rep.Rows = append(rep.Rows, reconcileRow{
				Handle: m.Handle, Verdict: verdictDeclaredNoAccount, IdentityStatus: status,
				Env: m.Env, Manifest: m.Manifest,
				Note: "collab.enabled with no account at the provider — provisioning gap",
			})
		}
	}

	for _, r := range rep.Rows {
		rep.Counts[r.Verdict]++
	}
	sort.SliceStable(rep.Rows, func(i, j int) bool {
		ri, rj := verdictRank[rep.Rows[i].Verdict], verdictRank[rep.Rows[j].Verdict]
		if ri != rj {
			return ri < rj
		}
		return rep.Rows[i].Handle < rep.Rows[j].Handle
	})
	return rep
}

func joinNote(a, b string) string {
	if a == "" {
		return b
	}
	return a + "; " + b
}

// writeReport renders the human form: one line per row, loudest first, then a
// count summary. Machine consumers take -json instead.
func writeReport(w io.Writer, rep reconcileReport) {
	fmt.Fprintf(w, "collab reconcile — provider %s, source %s, %s\n",
		rep.Provider, rep.Source, rep.GeneratedAt)
	fmt.Fprintf(w, "%d live account(s); identity sources: %d manifest(s), %d roster member(s)\n\n",
		rep.LiveAccounts, rep.ManifestsRead, rep.RosterMembers)
	for _, r := range rep.Rows {
		if r.Verdict == verdictOK {
			continue // the report is the delta; `ok` rows are the counts line
		}
		fmt.Fprintf(w, "%-30s %-24s identity=%-8s", r.Verdict, r.Handle, r.IdentityStatus)
		if r.Manifest != "" {
			fmt.Fprintf(w, " manifest=%s", r.Manifest)
		}
		if r.Note != "" {
			fmt.Fprintf(w, "\n%34s%s", "", r.Note)
		}
		fmt.Fprintln(w)
	}
	fmt.Fprintln(w)
	keys := make([]string, 0, len(rep.Counts))
	for k := range rep.Counts {
		keys = append(keys, k)
	}
	sort.Slice(keys, func(i, j int) bool { return verdictRank[keys[i]] < verdictRank[keys[j]] })
	for _, k := range keys {
		fmt.Fprintf(w, "%-30s %d\n", k, rep.Counts[k])
	}
	for _, warn := range rep.Warnings {
		fmt.Fprintf(w, "warning: %s\n", warn)
	}
}

// -------------------------------------------------------------- backfill ---

// backfillDoc is the durable-record patch set: what the provider observably has,
// expressed in the manifest schema's own `collab` vocabulary so applying it is a
// mechanical write, not a judgement call.
//
// activated_at is deliberately ABSENT: the provider's person list carries no
// creation date, and stamping today's date would assert a provisioning date we
// did not observe. The document's own observed_at records when the accounts were
// seen; the schema's activated_at stays for the provisioner, which does know.
type backfillDoc struct {
	Version    int             `yaml:"version" json:"version"`
	ObservedAt string          `yaml:"observed_at" json:"observed_at"`
	Provider   string          `yaml:"provider" json:"provider"`
	Source     string          `yaml:"source" json:"source"`
	Note       string          `yaml:"note" json:"note"`
	Patches    []backfillPatch `yaml:"patches" json:"patches"`
}

type backfillPatch struct {
	Handle   string       `yaml:"handle" json:"handle"`
	Env      string       `yaml:"env,omitempty" json:"env,omitempty"`
	Manifest string       `yaml:"manifest" json:"manifest"`
	Collab   backfillPart `yaml:"collab" json:"collab"`
}

// backfillPart is the subset of the schema's collab block a reconcile can assert
// from observation alone.
type backfillPart struct {
	Enabled   bool   `yaml:"enabled" json:"enabled"`
	Provider  string `yaml:"provider" json:"provider"`
	Account   string `yaml:"account" json:"account"`
	Activated bool   `yaml:"activated" json:"activated"`
}

const backfillNote = "Generated by `onboard-runner -collab reconcile -emit-backfill`. " +
	"Each patch records an account the provider DOES have against the person's manifest " +
	"collab block. Apply with `onboard-runner -collab backfill -file <this> -yes`. " +
	"Stale records (manifest names an account the provider lacks) are reported by the " +
	"reconcile but never patched here — clearing a record is an operator judgement."

// buildBackfill turns a report into the patch set: every row where the provider
// has an account, the person has a manifest on this host, and what the manifest
// records differs from what was observed. Revoked people are INCLUDED — recording
// that they hold an account is precisely the fact that makes the sweep possible;
// the decision to cut it is separate and stays with the operator.
func buildBackfill(rep reconcileReport, observedAt string) backfillDoc {
	doc := backfillDoc{
		Version: 1, ObservedAt: observedAt, Provider: rep.Provider,
		Source: rep.Source, Note: backfillNote,
	}
	for _, r := range rep.Rows {
		if !r.LiveAccount || r.Manifest == "" {
			continue
		}
		if r.RecordedAccount == r.Handle {
			continue // already recorded, identically
		}
		doc.Patches = append(doc.Patches, backfillPatch{
			Handle: r.Handle, Env: r.Env, Manifest: r.Manifest,
			Collab: backfillPart{
				Enabled: true, Provider: rep.Provider, Account: r.Handle, Activated: true,
			},
		})
	}
	sort.Slice(doc.Patches, func(i, j int) bool { return doc.Patches[i].Handle < doc.Patches[j].Handle })
	return doc
}

func writeBackfill(path string, doc backfillDoc) error {
	b, err := yaml.Marshal(doc)
	if err != nil {
		return err
	}
	return os.WriteFile(path, b, 0o600)
}

func readBackfill(path string) (backfillDoc, error) {
	var doc backfillDoc
	b, err := os.ReadFile(path)
	if err != nil {
		return doc, err
	}
	if err := yaml.Unmarshal(b, &doc); err != nil {
		return doc, fmt.Errorf("parse %s: %w", path, err)
	}
	if doc.Version != 1 {
		return doc, fmt.Errorf("%s: unsupported backfill version %d (want 1)", path, doc.Version)
	}
	return doc, nil
}

// applyBackfill writes each patch's collab block into its manifest. It edits the
// parsed YAML node tree rather than the text, so comments and unrelated keys
// survive, and it touches ONLY the five schema-defined collab keys.
//
// apply=false is a real dry run: every would-be change is printed, nothing is
// written. Idempotent — a manifest that already carries the observed values is
// counted as unchanged, so a re-run after a partial failure converges.
func applyBackfill(repoRoot string, doc backfillDoc, apply bool, w io.Writer) (changed, unchanged int, errs []string) {
	operatorsRoot, err := filepath.Abs(filepath.Join(repoRoot, "ops", "operators"))
	if err != nil {
		return 0, 0, []string{fmt.Sprintf("resolve operators root: %v", err)}
	}
	for _, p := range doc.Patches {
		path, err := filepath.Abs(filepath.Join(repoRoot, filepath.FromSlash(p.Manifest)))
		if err != nil {
			errs = append(errs, fmt.Sprintf("%s: resolve %s: %v", p.Handle, p.Manifest, err))
			continue
		}
		// A patch document is operator-supplied input; refuse any path that leaves
		// the manifests tree rather than trusting it.
		if path != operatorsRoot && !strings.HasPrefix(path, operatorsRoot+string(os.PathSeparator)) {
			errs = append(errs, fmt.Sprintf("%s: refusing path outside ops/operators: %s", p.Handle, p.Manifest))
			continue
		}
		did, err := patchManifestCollab(path, p.Collab, apply, w, p.Handle)
		switch {
		case err != nil:
			errs = append(errs, fmt.Sprintf("%s: %v", p.Handle, err))
		case did:
			changed++
		default:
			unchanged++
		}
	}
	return changed, unchanged, errs
}

// patchManifestCollab sets the collab keys on one manifest, preserving the rest
// of the document (including comments). Returns whether anything differed.
func patchManifestCollab(path string, part backfillPart, apply bool, w io.Writer, handle string) (bool, error) {
	info, err := os.Stat(path)
	if err != nil {
		return false, err
	}
	b, err := os.ReadFile(path)
	if err != nil {
		return false, err
	}
	var doc yaml.Node
	if err := yaml.Unmarshal(b, &doc); err != nil {
		return false, fmt.Errorf("parse: %w", err)
	}
	if len(doc.Content) == 0 || doc.Content[0].Kind != yaml.MappingNode {
		return false, fmt.Errorf("not a YAML mapping")
	}
	root := doc.Content[0]
	collab := mappingValue(root, "collab")
	if collab == nil {
		collab = &yaml.Node{Kind: yaml.MappingNode, Tag: "!!map"}
		root.Content = append(root.Content,
			&yaml.Node{Kind: yaml.ScalarNode, Tag: "!!str", Value: "collab",
				HeadComment: "Cloud collaboration environment (durable record)."},
			collab)
	}
	dirty := false
	set := func(key, val, tag string) {
		if setScalar(collab, key, val, tag) {
			dirty = true
			fmt.Fprintf(w, "  %s: collab.%s -> %s\n", handle, key, val)
		}
	}
	set("enabled", boolStr(part.Enabled), "!!bool")
	if part.Provider != "" {
		set("provider", part.Provider, "!!str")
	}
	set("activated", boolStr(part.Activated), "!!bool")
	if part.Account != "" {
		set("account", part.Account, "!!str")
	}
	if !dirty || !apply {
		return dirty, nil
	}
	var buf strings.Builder
	enc := yaml.NewEncoder(&buf)
	enc.SetIndent(2)
	if err := enc.Encode(&doc); err != nil {
		return false, fmt.Errorf("encode: %w", err)
	}
	_ = enc.Close()
	// Manifests carry identity material — keep whatever restrictive mode they
	// already have (0600 for runner-materialized ones).
	if err := os.WriteFile(path, []byte(buf.String()), info.Mode().Perm()); err != nil {
		return false, fmt.Errorf("write: %w", err)
	}
	return true, nil
}

// mappingValue returns the value node for `key` in a mapping node, or nil.
func mappingValue(m *yaml.Node, key string) *yaml.Node {
	for i := 0; i+1 < len(m.Content); i += 2 {
		if m.Content[i].Value == key {
			return m.Content[i+1]
		}
	}
	return nil
}

// setScalar sets key=val (with the given YAML tag) on a mapping, appending the
// key when absent. Reports whether the document changed.
func setScalar(m *yaml.Node, key, val, tag string) bool {
	if v := mappingValue(m, key); v != nil {
		if v.Kind == yaml.ScalarNode && v.Value == val && v.Tag == tag {
			return false
		}
		v.Kind, v.Tag, v.Value, v.Style = yaml.ScalarNode, tag, val, 0
		v.Content = nil
		return true
	}
	m.Content = append(m.Content,
		&yaml.Node{Kind: yaml.ScalarNode, Tag: "!!str", Value: key},
		&yaml.Node{Kind: yaml.ScalarNode, Tag: tag, Value: val})
	return true
}

func boolStr(b bool) string {
	if b {
		return "true"
	}
	return "false"
}

// nowUTC is the reconcile's clock, indirected so tests get a fixed stamp.
var nowUTC = func() string { return time.Now().UTC().Format(time.RFC3339) }
