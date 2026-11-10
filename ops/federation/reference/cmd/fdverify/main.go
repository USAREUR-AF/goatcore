// Command fdverify is the operator surface for the federation package
// specification, version 1.
//
// The specification is the contract; this is one implementation of it. A
// partner must be able to implement the specification from scratch, in any
// language, without running this program.
//
// Exit status
//
//	0  the operation succeeded, or diff found no difference
//	1  a refusal, a lint finding, or a difference — the mechanism working
//	2  a usage error, an unreadable file, or an unusable key
//
// A refusal is an outcome, not a crash: it exits 1 and says which rule refused.
package main

import (
	"bytes"
	"crypto/ed25519"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/federation/reference/internal/fedpkg"
)

const usage = `fdverify — federation package v1

  fdverify canonical <file>
        RFC 8785 canonical form of any package or agreement, on stdout, with no
        trailing newline. A canonical form is bytes, not a text file.

  fdverify digest <file>
        sha256 over the canonical form. This is the value the two parties
        compare, and the channel carrying it does not need to be secure.

  fdverify lint [-now DATE] <package>
        Check one package against itself: the members always required, and the
        internal completeness of whatever optional blocks are present.

  fdverify verify -key <sid>=<file> | -keydir <dir> <package>
        Check the package's Ed25519 signature under the key bound to its
        sovereign_id.

  fdverify compose [-now DATE] [-digest] -key <sid>=<file> | -keydir <dir> <a> <b>
        Compose two packages into one agreement, on stdout in canonical form.
        Refuses under the rules of section 6 and names the rule.

  fdverify diff <agreement-a> <agreement-b>
        Name the fields two agreements differ on. This is what to reach for when
        two digests disagree.

Keys

  The verifying key is an input, never fetched. It is the key bound to a party's
  sovereign_id over a channel authenticated independently of the relationship
  being established. A directory may cache that binding; it is never its source.

  -key xaa=/path/to/xaa.public.hex   repeatable; the file holds hex or base64
  -keydir <dir>                      reads <dir>/<sid>.public.hex or .public.b64

Dates

  -now YYYY-MM-DD   the evaluation date for expiry, defaulting to today. When
                    running the conformance vectors, pass the date in their
                    now.txt: expiry fixtures change verdict as time passes.
`

func main() {
	if len(os.Args) < 2 {
		fmt.Fprint(os.Stderr, usage)
		os.Exit(2)
	}
	verb := os.Args[1]
	args := os.Args[2:]

	var err error
	switch verb {
	case "canonical":
		err = cmdCanonical(args)
	case "digest":
		err = cmdDigest(args)
	case "lint":
		err = cmdLint(args)
	case "verify":
		err = cmdVerify(args)
	case "compose":
		err = cmdCompose(args)
	case "diff":
		err = cmdDiff(args)
	case "-h", "--help", "help":
		fmt.Print(usage)
		return
	default:
		fmt.Fprintf(os.Stderr, "fdverify: unknown verb %q\n\n%s", verb, usage)
		os.Exit(2)
	}

	if err == nil {
		return
	}
	fmt.Fprintf(os.Stderr, "%v\n", err)
	var o outcome
	if errors.As(err, &o) {
		os.Exit(1)
	}
	os.Exit(2)
}

// outcome is a refusal, a finding or a difference: the mechanism working, and
// distinguished from an error so that the exit status can tell them apart.
type outcome struct{ msg string }

func (o outcome) Error() string { return o.msg }

func refused(format string, a ...any) error { return outcome{fmt.Sprintf(format, a...)} }

// keyFlags collects the §5.1 bindings the caller supplies.
type keyFlags struct {
	pairs  []string
	keydir string
}

func (k *keyFlags) register(fs *flag.FlagSet) {
	fs.Var(sliceFlag{&k.pairs}, "key", "a binding, <sovereign_id>=<file>; repeatable")
	fs.StringVar(&k.keydir, "keydir", "", "a directory of <sovereign_id>.public.hex or .public.b64 files")
}

func (k *keyFlags) keyring() (fedpkg.Keyring, error) {
	ring := fedpkg.Keyring{}
	if k.keydir != "" {
		for _, ext := range []string{"*.public.hex", "*.public.b64"} {
			paths, _ := filepath.Glob(filepath.Join(k.keydir, ext))
			for _, p := range paths {
				sid := strings.TrimSuffix(strings.TrimSuffix(filepath.Base(p), ".public.hex"), ".public.b64")
				pub, err := readKey(p)
				if err != nil {
					return nil, err
				}
				ring[sid] = pub
			}
		}
		if len(ring) == 0 {
			return nil, fmt.Errorf("-keydir %s holds no .public.hex or .public.b64 files", k.keydir)
		}
	}
	for _, pair := range k.pairs {
		sid, path, ok := strings.Cut(pair, "=")
		if !ok || sid == "" || path == "" {
			return nil, fmt.Errorf("-key %q is not <sovereign_id>=<file>", pair)
		}
		pub, err := readKey(path)
		if err != nil {
			return nil, err
		}
		ring[sid] = pub
	}
	if len(ring) == 0 {
		return nil, fmt.Errorf("no key was supplied; a signature check needs the key bound to each party's sovereign_id (-key or -keydir)")
	}
	return ring, nil
}

func readKey(path string) (ed25519.PublicKey, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading key: %w", err)
	}
	pub, err := fedpkg.ParsePublicKey(string(b))
	if err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return pub, nil
}

type sliceFlag struct{ dst *[]string }

func (s sliceFlag) String() string {
	if s.dst == nil {
		return ""
	}
	return strings.Join(*s.dst, ",")
}
func (s sliceFlag) Set(v string) error { *s.dst = append(*s.dst, v); return nil }

func nowFlag(fs *flag.FlagSet) *string {
	return fs.String("now", time.Now().UTC().Format(fedpkg.DateLayout), "evaluation date, YYYY-MM-DD")
}

func parseNow(s string) (time.Time, error) {
	t, err := fedpkg.ParseDate(s)
	if err != nil {
		return time.Time{}, fmt.Errorf("-now: %w", err)
	}
	return t, nil
}

func loadValue(path string) (map[string]any, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	m, err := fedpkg.DecodeJSON(b)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return m, nil
}

func loadPackage(path string) (*fedpkg.Package, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	p, err := fedpkg.Load(b)
	if err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return p, nil
}

func cmdCanonical(args []string) error {
	fs := flag.NewFlagSet("canonical", flag.ContinueOnError)
	fs.Usage = func() { fmt.Fprint(os.Stderr, usage) }
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("canonical takes one file")
	}
	m, err := loadValue(fs.Arg(0))
	if err != nil {
		return err
	}
	b, err := fedpkg.Canonical(m)
	if err != nil {
		return err
	}
	// No trailing newline: a canonical form is bytes, and a tool that appends
	// one produces a different digest.
	_, err = os.Stdout.Write(b)
	return err
}

func cmdDigest(args []string) error {
	fs := flag.NewFlagSet("digest", flag.ContinueOnError)
	fs.Usage = func() { fmt.Fprint(os.Stderr, usage) }
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("digest takes one file")
	}
	m, err := loadValue(fs.Arg(0))
	if err != nil {
		return err
	}
	d, err := fedpkg.Digest(m)
	if err != nil {
		return err
	}
	fmt.Println(d)
	return nil
}

func cmdLint(args []string) error {
	fs := flag.NewFlagSet("lint", flag.ContinueOnError)
	fs.Usage = func() { fmt.Fprint(os.Stderr, usage) }
	now := nowFlag(fs)
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("lint takes one package")
	}
	at, err := parseNow(*now)
	if err != nil {
		return err
	}
	p, err := loadPackage(fs.Arg(0))
	if err != nil {
		return err
	}
	findings := p.Lint(at)
	if len(findings) == 0 {
		fmt.Printf("ok  %s  %s expires %s\n", fs.Arg(0), p.SovereignID(), p.Expires())
		return nil
	}
	var b strings.Builder
	fmt.Fprintf(&b, "%d finding(s) in %s", len(findings), fs.Arg(0))
	for _, f := range findings {
		fmt.Fprintf(&b, "\n  %s\n    %s", f.Code, f.Msg)
	}
	return refused("%s", b.String())
}

func cmdVerify(args []string) error {
	fs := flag.NewFlagSet("verify", flag.ContinueOnError)
	fs.Usage = func() { fmt.Fprint(os.Stderr, usage) }
	var keys keyFlags
	keys.register(fs)
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 1 {
		return fmt.Errorf("verify takes one package")
	}
	ring, err := keys.keyring()
	if err != nil {
		return err
	}
	p, err := loadPackage(fs.Arg(0))
	if err != nil {
		return err
	}
	pub, ok := ring[p.SovereignID()]
	if !ok {
		return fmt.Errorf("no key is bound to %q; supply it with -key %s=<file>", p.SovereignID(), p.SovereignID())
	}
	if err := p.VerifySignature(pub); err != nil {
		return refused("%s: %v", fs.Arg(0), err)
	}
	fmt.Printf("ok  %s  signature verifies under the key bound to %s\n", fs.Arg(0), p.SovereignID())
	return nil
}

func cmdCompose(args []string) error {
	fs := flag.NewFlagSet("compose", flag.ContinueOnError)
	fs.Usage = func() { fmt.Fprint(os.Stderr, usage) }
	var keys keyFlags
	keys.register(fs)
	now := nowFlag(fs)
	wantDigest := fs.Bool("digest", false, "print the digest on stderr as well")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 2 {
		return fmt.Errorf("compose takes two packages")
	}
	at, err := parseNow(*now)
	if err != nil {
		return err
	}
	ring, err := keys.keyring()
	if err != nil {
		return err
	}
	a, err := loadPackage(fs.Arg(0))
	if err != nil {
		return err
	}
	b, err := loadPackage(fs.Arg(1))
	if err != nil {
		return err
	}

	ag, findings := fedpkg.Compose(a, b, ring, at)
	if len(findings) > 0 {
		var s strings.Builder
		// A refusal produces no agreement and no digest. Naming the rule is the
		// point: a refusal that does not say which rule fired is a broken link
		// that looks like a working one.
		fmt.Fprintf(&s, "refused: %s and %s do not compose", fs.Arg(0), fs.Arg(1))
		for _, f := range findings {
			fmt.Fprintf(&s, "\n  %s\n    %s", f.Code, f.Msg)
		}
		return refused("%s", s.String())
	}

	out, err := ag.Canonical()
	if err != nil {
		return err
	}
	if _, err := os.Stdout.Write(out); err != nil {
		return err
	}
	if *wantDigest {
		d, err := ag.Digest()
		if err != nil {
			return err
		}
		fmt.Fprintf(os.Stderr, "\n%s\n", d)
	}
	return nil
}

func cmdDiff(args []string) error {
	fs := flag.NewFlagSet("diff", flag.ContinueOnError)
	fs.Usage = func() { fmt.Fprint(os.Stderr, usage) }
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 2 {
		return fmt.Errorf("diff takes two agreements")
	}
	pathA, pathB := fs.Arg(0), fs.Arg(1)
	a, err := loadValue(pathA)
	if err != nil {
		return err
	}
	b, err := loadValue(pathB)
	if err != nil {
		return err
	}

	da, err := fedpkg.Digest(a)
	if err != nil {
		return fmt.Errorf("%s: %w", pathA, err)
	}
	db, err := fedpkg.Digest(b)
	if err != nil {
		return fmt.Errorf("%s: %w", pathB, err)
	}

	if da == db {
		fmt.Printf("identical\n  %s\n\nBoth sides hold the same agreement. Transcription error is now impossible.\n", da)
		return nil
	}

	var d []difference
	diffValues("", a, b, &d)
	sort.Slice(d, func(i, j int) bool { return d[i].path < d[j].path })

	var s strings.Builder
	fmt.Fprintf(&s, "%d field(s) differ\n\n  a  %s\n     %s\n  b  %s\n     %s\n",
		len(d), pathA, da, pathB, db)
	for _, x := range d {
		fmt.Fprintf(&s, "\n  %s\n", x.path)
		switch {
		case x.onlyIn == "a":
			fmt.Fprintf(&s, "    only in a: %s\n", x.a)
		case x.onlyIn == "b":
			fmt.Fprintf(&s, "    only in b: %s\n", x.b)
		default:
			fmt.Fprintf(&s, "    a: %s\n    b: %s\n", x.a, x.b)
		}
	}
	if len(d) == 0 {
		// Two documents with equal fields and unequal digests means one of them
		// is not in canonical form, which is a defect in whatever wrote it.
		fmt.Fprint(&s, "\nNo field differs, yet the digests do: at least one file is not in canonical form.\nRun `fdverify canonical` on each and compare the bytes.\n")
	}
	return refused("%s", s.String())
}

type difference struct {
	path   string
	a, b   string
	onlyIn string // "a", "b", or empty when both hold a value
}

// diffValues walks two decoded documents together and records where they part
// company. The offending field names itself, which is what §9 promises an
// operator staring at two unequal digests.
func diffValues(path string, a, b any, out *[]difference) {
	at := path
	if at == "" {
		at = "(root)"
	}

	ma, aIsMap := a.(map[string]any)
	mb, bIsMap := b.(map[string]any)
	if aIsMap && bIsMap {
		seen := map[string]bool{}
		var keys []string
		for k := range ma {
			keys = append(keys, k)
			seen[k] = true
		}
		for k := range mb {
			if !seen[k] {
				keys = append(keys, k)
			}
		}
		// The one comparison this format uses (§4). A report ordering does not
		// change any digest, but a second comparator in the codebase is one
		// that can drift from the first, which is the reason §4 gives for
		// having only one.
		sort.Slice(keys, func(i, j int) bool { return fedpkg.LessUTF16(keys[i], keys[j]) })
		for _, k := range keys {
			va, okA := ma[k]
			vb, okB := mb[k]
			child := k
			if path != "" {
				child = path + "." + k
			}
			switch {
			case okA && !okB:
				*out = append(*out, difference{child, render(va), "", "a"})
			case !okA && okB:
				*out = append(*out, difference{child, "", render(vb), "b"})
			default:
				diffValues(child, va, vb, out)
			}
		}
		return
	}

	sa, aIsArr := a.([]any)
	sb, bIsArr := b.([]any)
	if aIsArr && bIsArr {
		n := len(sa)
		if len(sb) > n {
			n = len(sb)
		}
		for i := 0; i < n; i++ {
			child := fmt.Sprintf("%s[%d]", path, i)
			switch {
			case i >= len(sb):
				*out = append(*out, difference{child, render(sa[i]), "", "a"})
			case i >= len(sa):
				*out = append(*out, difference{child, "", render(sb[i]), "b"})
			default:
				diffValues(child, sa[i], sb[i], out)
			}
		}
		return
	}

	ra, rb := render(a), render(b)
	if ra != rb {
		*out = append(*out, difference{at, ra, rb, ""})
	}
}

// render shows a value the way it appears in the canonical form, so that a
// reported difference is in the same terms as the bytes being compared.
func render(v any) string {
	if b, err := fedpkg.Canonical(v); err == nil {
		return string(b)
	}
	if n, ok := v.(json.Number); ok {
		return fmt.Sprintf("%q (not a value this format can carry)", n.String())
	}
	if v == nil {
		return "null (not a value this format can carry)"
	}
	var buf bytes.Buffer
	fmt.Fprintf(&buf, "%v", v)
	return buf.String()
}
