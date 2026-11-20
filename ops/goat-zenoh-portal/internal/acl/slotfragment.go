package acl

// The slot fragment (GHSA-rr3p, F-305): the router's per-slot subjects,
// rendered from the registry instead of from the pool range.
//
// zenoh 1.9 decides who a peer is by its leaf's Subject CN alone. When the
// router template rendered `slot-<anchor>` with cert_common_names ["<anchor>"]
// for every index in the pool, any leaf ever issued for an index kept matching
// that index's subject: after a revoke, after a rotate, and after the index was
// handed to someone else. The fragment renders one `slot-<anchor>` subject per
// pool slot, and gives it the CN of the slot's CURRENT generation only while
// the slot is assigned. A free or quarantined slot gets the CN
// "<anchor>.unassigned", which no mint produces (the portal mints only
// "<anchor>" and "<anchor>.g<N>"), so it matches no leaf. It cannot be an empty
// list: zenoh 1.9 types cert_common_names as a non-empty vector and refuses to
// start on `[]` (zenoh-config 1.9.0 lib.rs:200, Option<NEVec<CertCommonName>>),
// and omitting the field would match every CN. Every per-slot grant in the
// template (own-anchor write, the universal and fabric-wide read, partner
// lanes, co-publish and release-lane writes) is bound to `slot-<anchor>`, so a
// CN missing here holds none of them.
//
// The subjects stay declared for free slots because the template's policies
// name them by id and zenoh refuses a policy whose subject is not declared
// (zenoh 1.9.0 net/routing/interceptor/authorization.rs, policy_information_point).
//
// The fragment lives between two markers in the live config, and is also
// written beside it (slot-fragment.json5) so a role re-render splices in the
// same content instead of reverting to a pool-wide grant.

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

// Marker lines bounding the slot fragment in the router's subjects array. The
// router template (roles/goat-zenoh config.json5.j2) emits exactly these.
const (
	SlotFragmentBegin = "      // BEGIN: slot subjects"
	SlotFragmentEnd   = "      // END: slot subjects"
)

// SlotFragmentFile is the fragment's file name beside the live config.
const SlotFragmentFile = "slot-fragment.json5"

// SlotFragmentLoadedFile is where the router host's reload watcher
// (roles/goat-zenoh files/slot-fragment-reload.py) records the fragment the
// router last loaded, beside the live config. The portal only reads it.
const SlotFragmentLoadedFile = "slot-fragment.loaded.json5"

// slotDigestPrefix starts the fragment's first line, which carries its digest.
const slotDigestPrefix = "      // slot fragment sha256:"

// ErrNoSlotMarkers means the live config has no slot-fragment markers: the
// router was rendered by a role that predates the fragment.
var ErrNoSlotMarkers = errors.New("router config carries no slot-fragment markers")

// UnassignedSuffix makes the CN a free slot's subject carries: one no leaf has.
const UnassignedSuffix = ".unassigned"

// SlotBinding is one pool slot as the fragment sees it. CN is the leaf CN of
// the slot's current generation when it is assigned, and "" when it is free.
type SlotBinding struct {
	Anchor string
	CN     string
	// Copub is one subject per co-publish target this anchor was ever
	// granted (ADR 1105), copub-<anchor>-<key>. It carries CN only while a
	// grant for exactly that topic is active and in effect (Live). The
	// router role binds each topic's copub-w rule to its own subject, so a
	// revoked or expired grant stops at the router's next reload of the
	// fragment even while the role-rendered rule is still in the config, and
	// a grant on one topic never opens another (GHSA-rr3p review r8 F1).
	Copub []CopubSubject
	// Carry lists the leaf CNs of other slots the core admits as writers of
	// this slot on its behalf (ADR 1251 phase 4): the pods whose carriage
	// rules cover it and, for a keyless source, its ingress adapters. The
	// subject carry-<anchor> is rendered for every slot, with the unassigned
	// CN when Carry is empty or the slot is free; the router role binds it to
	// slot-w-<anchor>, so a carrier may write exactly the slot's own write
	// set. Sorted, no repeats, never the slot's own CN.
	Carry []string
}

// CopubSubject is one (anchor, topic) co-publish subject.
type CopubSubject struct {
	Key  string // 16 hex: the first 16 of sha256(topic base)
	Live bool
}

var copubKeyRe = regexp.MustCompile(`^[0-9a-f]{16}$`)

var (
	anchorRe = regexp.MustCompile(`^[0-9a-f]{32}$`)
	slotCNRe = regexp.MustCompile(`^([0-9a-f]{32})(\.g[1-9][0-9]*)?$`)
)

// RenderSlotFragment renders the fragment for bindings (one per pool slot, in
// the order given) and returns it with its digest. It refuses an anchor or CN
// that is not slot-shaped, and a CN that names a different anchor than its
// slot, since either would put text the router parses into its config.
func RenderSlotFragment(bindings []SlotBinding) (string, string, error) {
	var body strings.Builder
	assigned, carried := 0, 0
	seen := make(map[string]bool, len(bindings))
	for _, b := range bindings {
		if !anchorRe.MatchString(b.Anchor) {
			return "", "", fmt.Errorf("slot fragment: anchor %q is not slot-shaped", b.Anchor)
		}
		if seen[b.Anchor] {
			return "", "", fmt.Errorf("slot fragment: anchor %s listed twice", b.Anchor)
		}
		seen[b.Anchor] = true
		unassigned := `["` + b.Anchor + UnassignedSuffix + `"]`
		cns := unassigned
		if b.CN != "" {
			m := slotCNRe.FindStringSubmatch(b.CN)
			if m == nil || m[1] != b.Anchor {
				return "", "", fmt.Errorf("slot fragment: CN %q is not a leaf CN of slot %s", b.CN, b.Anchor)
			}
			cns = `["` + b.CN + `"]`
			assigned++
		}
		fmt.Fprintf(&body, "      {\n        id: \"slot-%s\",\n        cert_common_names: %s,\n      },\n", b.Anchor, cns)
		seenKey := map[string]bool{}
		for _, c := range b.Copub {
			if !copubKeyRe.MatchString(c.Key) || seenKey[c.Key] {
				return "", "", fmt.Errorf("slot fragment: bad or repeated co-publish key %q for slot %s", c.Key, b.Anchor)
			}
			seenKey[c.Key] = true
			cc := unassigned
			if c.Live && b.CN != "" {
				cc = cns
			}
			fmt.Fprintf(&body, "      {\n        id: \"copub-%s-%s\",\n        cert_common_names: %s,\n      },\n", b.Anchor, c.Key, cc)
		}
		carry, err := renderCarry(b, unassigned)
		if err != nil {
			return "", "", err
		}
		if carry != unassigned {
			carried++
		}
		fmt.Fprintf(&body, "      {\n        id: \"carry-%s\",\n        cert_common_names: %s,\n      },\n", b.Anchor, carry)
	}
	sum := sha256.Sum256([]byte(body.String()))
	digest := hex.EncodeToString(sum[:])
	head := fmt.Sprintf("%s%s (%d of %d slots assigned, %d carried; rendered from the portal registry, GHSA-rr3p, ADR 1251)\n",
		slotDigestPrefix, digest, assigned, len(bindings), carried)
	return head + body.String(), digest, nil
}

// renderCarry is the cert_common_names list of carry-<anchor>.
func renderCarry(b SlotBinding, unassigned string) (string, error) {
	if b.CN == "" || len(b.Carry) == 0 {
		return unassigned, nil
	}
	var out strings.Builder
	out.WriteString("[")
	for i, cn := range b.Carry {
		m := slotCNRe.FindStringSubmatch(cn)
		if m == nil {
			return "", fmt.Errorf("slot fragment: carrier CN %q of slot %s is not a leaf CN", cn, b.Anchor)
		}
		if m[1] == b.Anchor {
			return "", fmt.Errorf("slot fragment: slot %s lists its own CN as a carrier", b.Anchor)
		}
		if i > 0 && cn <= b.Carry[i-1] {
			return "", fmt.Errorf("slot fragment: carriers of slot %s are not sorted and unique", b.Anchor)
		}
		if i > 0 {
			out.WriteString(", ")
		}
		out.WriteString(`"` + cn + `"`)
	}
	out.WriteString("]")
	return out.String(), nil
}

// SlotFragmentDigest checks a fragment (the text between the markers, or the
// fragment file) against the digest it declares, and returns that digest.
func SlotFragmentDigest(fragment string) (string, error) {
	head, body, ok := strings.Cut(fragment, "\n")
	if !ok || !strings.HasPrefix(head, slotDigestPrefix) {
		return "", errors.New("slot fragment: no digest line")
	}
	declared, _, _ := strings.Cut(strings.TrimPrefix(head, slotDigestPrefix), " ")
	sum := sha256.Sum256([]byte(body))
	if got := hex.EncodeToString(sum[:]); got != declared {
		return "", fmt.Errorf("slot fragment: declared digest %s, content hashes to %s", declared, got)
	}
	return declared, nil
}

// SpliceSlotFragment replaces the text between the slot-fragment markers in a
// router config with fragment. ErrNoSlotMarkers when the markers are absent.
func SpliceSlotFragment(config, fragment string) (string, error) {
	b := strings.Index(config, SlotFragmentBegin)
	if b < 0 {
		return "", ErrNoSlotMarkers
	}
	bodyStart := strings.IndexByte(config[b:], '\n')
	if bodyStart < 0 {
		return "", ErrNoSlotMarkers
	}
	bodyStart += b + 1
	e := strings.Index(config[bodyStart:], SlotFragmentEnd)
	if e < 0 {
		return "", ErrNoSlotMarkers
	}
	e += bodyStart
	if strings.Contains(config[e+len(SlotFragmentEnd):], SlotFragmentBegin) {
		return "", errors.New("router config carries more than one slot-fragment region")
	}
	return config[:bodyStart] + fragment + config[e:], nil
}

// SlotFragmentResult says what ApplySlotFragment did.
type SlotFragmentResult struct {
	Digest string
	// ConfigChanged is true when the live config was rewritten. The change
	// reaches the router only when it reloads its config: on zenoh 1.9 that
	// is a restart (F-156; SIGHUP is a no-op).
	ConfigChanged bool
}

// LoadedLeafCNs returns the leaf CNs in the fragment the router last loaded,
// as the host watcher recorded it. ok is false when there is no record (no
// watcher on this host, or it has not run) or the record fails its digest;
// the caller then cannot tell whether a CN is live.
func (r *Renderer) LoadedLeafCNs() (map[string]bool, bool) {
	if r.ConfigPath == "" {
		return nil, false
	}
	b, err := os.ReadFile(filepath.Join(filepath.Dir(r.ConfigPath), SlotFragmentLoadedFile))
	if err != nil {
		return nil, false
	}
	if _, err := SlotFragmentDigest(string(b)); err != nil {
		return nil, false
	}
	out := map[string]bool{}
	for _, m := range loadedCNRe.FindAllStringSubmatch(string(b), -1) {
		out[m[1]] = true
	}
	return out, true
}

// Only slot- and copub- subjects name a slot's own leaf; carry- subjects name
// other slots' leaves and say nothing about whether this one is loaded.
var loadedCNRe = regexp.MustCompile(`id: "(?:slot|copub)-[^"]+",\n        cert_common_names: \["([0-9a-f]{32}(?:\.g[1-9][0-9]*)?)"\]`)

// slotFragmentPath is where the fragment file goes: beside the live config.
func (r *Renderer) slotFragmentPath() string {
	if r.SlotFragmentPath != "" {
		return r.SlotFragmentPath
	}
	return filepath.Join(filepath.Dir(r.ConfigPath), SlotFragmentFile)
}

// ApplySlotFragment renders the fragment for bindings, writes it beside the
// live config, and splices it into the live config. The file is written even
// when the live config has no markers yet, so the next role render picks the
// fragment up; ErrNoSlotMarkers is returned in that case.
func (r *Renderer) ApplySlotFragment(ctx context.Context, bindings []SlotBinding) (SlotFragmentResult, error) {
	return r.ApplySlotFragmentFrom(ctx, func(context.Context) ([]SlotBinding, error) { return bindings, nil })
}

// ApplySlotFragmentFrom is ApplySlotFragment with the bindings read by read
// WHILE the renderer's lock is held, so the read and the write are one step: a
// render that read the registry before a later change committed can never land
// after that change's own render (GHSA-rr3p review r8 F4).
func (r *Renderer) ApplySlotFragmentFrom(ctx context.Context, read func(context.Context) ([]SlotBinding, error)) (SlotFragmentResult, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	var res SlotFragmentResult
	if r.ConfigPath == "" {
		return res, errors.New("acl renderer: ConfigPath is empty")
	}
	bindings, err := read(ctx)
	if err != nil {
		return res, err
	}
	frag, digest, err := RenderSlotFragment(bindings)
	if err != nil {
		return res, err
	}
	res.Digest = digest
	fpath := r.slotFragmentPath()
	if cur, err := os.ReadFile(fpath); err != nil || string(cur) != frag {
		if err := atomicWrite(fpath, []byte(frag)); err != nil {
			return res, fmt.Errorf("acl renderer: write %s: %w", fpath, err)
		}
	}
	current, err := os.ReadFile(r.ConfigPath)
	if err != nil {
		return res, fmt.Errorf("acl renderer: read %s: %w", r.ConfigPath, err)
	}
	updated, err := SpliceSlotFragment(string(current), frag)
	if err != nil {
		return res, err
	}
	if updated == string(current) {
		return res, nil
	}
	if err := atomicWrite(r.ConfigPath, []byte(updated)); err != nil {
		return res, fmt.Errorf("acl renderer: write %s: %w", r.ConfigPath, err)
	}
	res.ConfigChanged = true
	slog.Info("slot fragment applied to the router config; it takes effect when the router reloads its config (a restart on zenoh 1.9, F-156)",
		"config", r.ConfigPath, "digest", digest)
	if !r.ReloadFake {
		if err := r.sighup(ctx); err != nil {
			slog.Warn("slot fragment: SIGHUP failed (a no-op on zenohd 1.9.0 in any case)", "err", err)
		}
	}
	return res, nil
}
