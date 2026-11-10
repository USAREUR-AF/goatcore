// Command bundle-create issues a signed offline-enrollment bundle for a
// single device. It is the operator-facing CLI on the air-gapped issuing
// workstation; the output bundle.cbor is what gets scp'd to the device
// for first-boot consumption (see wg-cp0-bundle-apply on the device side).
//
// Without this CLI, wg-cp0 silent-control-plane peers would enroll via
// inventory-trust + local `wg genkey`, contradicting
// ADRs 0308 / 0316 / 0406 which root trust in the offline CA. This CLI
// generates the device's wg-cp0 Curve25519 keypair internally during the
// signing ceremony, embeds both halves in the bundle, and signs the whole
// thing with the offline CA.
//
// Usage (dev mode — age-encrypted CA key, pre-YubiKey-procurement):
//
//	bundle-create \
//	    --site <site> \
//	    --device-id <device-id> \
//	    --wg-cp0-address <address> \
//	    --wg-cp0-relays ops/enrollment/wg-cp0-relays.json \
//	    --acl-groups <SITE>,<SITE>-MGMT \
//	    --ca-id dev-<site>-ca-YYYY-MM-DD \
//	    --ca-key ops/enrollment/ca/dev/ca.key.age \
//	    --age-identity ~/.config/age/keys.txt \
//	    --tls-trust-anchor ops/enrollment/ca/dev/root-cert.pem \
//	    --out <site>-<device-id>.bundle.cbor
//
// Production mode (YubiKey signer) is gated on hardware procurement; see
// ops/enrollment/ca/yubikey_ca.go ErrYubikeyNotImplemented.
//
// The --wg-cp0-relays file is operator-curated JSON listing each relay's
// public endpoint, control-plane mesh address, and wg-cp0 pubkey. Format:
//
//	[
//	  {
//	    "name":     "relay-rla",
//	    "endpoint": "203.0.113.1:51821",
//	    "address":  "198.18.0.2",
//	    "pubkey":   "<base64-32-byte-curve25519-pubkey>"
//	  },
//	  ...
//	]
//
// The address field is informational (used to compute AllowedIPs in the
// device's wg-cp0.conf via wg-cp0-bundle-apply); the bundle's
// known_endpoints[*] carries the (endpoint, pubkey) pair plus kind="cp-relay".
//
// Defaults:
//
//	--ttl-days         90  (matches ADR 0308 expires_at horizon)
//	--activation-days  14  (matches ADR 0308 §3.1 first-enrollment window)
//
// Operator discipline:
//   - Run on an air-gapped workstation. The CA key is sensitive material.
//   - The output bundle.cbor contains the device's wg-cp0 PRIVATE key.
//     Treat as mode 0600 throughout transit; shred after the device
//     extracts it (wg-cp0-bundle-apply does this automatically).
//   - Do NOT commit bundle.cbor or wg-cp0-relays.json to git. The relays
//     file is endpoint+pubkey only (no privkey) so it isn't strictly
//     secret, but it carries enough information to map peers to relays
//     and shouldn't be in public surface either.
package main

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/pem"
	"errors"
	"flag"
	"fmt"
	"net"
	"net/http"
	"os"
	"strings"
	"time"

	"filippo.io/age"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
	"github.com/usareur-af/goatcore/ops/enrollment/ca"
)

const (
	defaultTTLDays        = 90
	defaultActivationDays = 14
	nonceSize             = 16
	wgPubkeySize          = 32
	peerPubkeySize        = 32
)

func main() {
	site := flag.String("site", "", "Site identifier (e.g. site-a, site-b). Required.")
	deviceID := flag.String("device-id", "", "Device identifier (e.g. mgmt-site-a-01-orin). Required.")
	wgCP0Address := flag.String("wg-cp0-address", "", "Device's wg-cp0 interface address as CIDR (e.g. 198.18.0.6/24). Required.")
	relaysPath := flag.String("wg-cp0-relays", "", "Path to JSON file listing wg-cp0 relays (name, endpoint, address, pubkey). Required.")
	aclGroupsRaw := flag.String("acl-groups", "", "Comma-separated ACL group names (e.g. SITE-A,SITE-A-MGMT). Required unless --from-machine is used (mutually exclusive). Gear-class groups (ADR 1116) are REFUSED here — they are derived from the machine's node-class via --from-machine, never hand-typed. Infra/site groups stay allowed.")
	fromMachine := flag.String("from-machine", "", "machine-mesh-alignment D6a. Portal-registry slot anchor of an enrolled machine. The bundle's acl_groups are DERIVED from the machine's born-as node-class (GET /api/machines/{anchor} on --portal-url) and the bundle is stamped with enrollment_anchor + node_class so the joined peer ties back to its custody edge. Mutually exclusive with --acl-groups. Requires --portal-url + $GOAT_PORTAL_OPERATOR_KEY.")
	portalURL := flag.String("portal-url", "", "Portal base URL (https) for --from-machine, e.g. https://portal.galaxy.example. The operator API key is read from $GOAT_PORTAL_OPERATOR_KEY — an env var, never a flag.")
	taxonomyPath := flag.String("taxonomy", defaultTaxonomyRelPath, "Path to netbird_acl_taxonomy.yml, used by the hand-typed --acl-groups path to refuse gear-class groups. The default resolves repo-relative by walking up from the working directory.")
	caID := flag.String("ca-id", "", "CA identifier matching the public-keys distribution (e.g. dev-galaxy-ca-2026-04-20). Required.")

	// Signing-mode selector + per-mode flags. Default `dev` until an
	// operator explicitly switches to YubiKey (Block 22K.4 cutover).
	mode := flag.String("mode", "dev", "Signing mode: dev (age-encrypted file) or prod (YubiKey PIV slot 9c per ADR 0406).")
	caKeyPath := flag.String("ca-key", "", "Dev mode. Path to age-encrypted CA private key. Required when --mode=dev.")
	ageIdentityPath := flag.String("age-identity", "", "Dev mode. Path to age identity file (decrypts --ca-key). Required when --mode=dev.")
	pivSlotName := flag.String("piv-slot", "9c", "Prod mode. PIV slot for the CA key. Default 9c (Digital Signature). Hex string per PIV spec.")
	cardSelector := flag.String("card-selector", "", "Prod mode. Substring match against PCSC reader names; empty = first card found.")
	pinFlag := flag.String("pin", "", "Prod mode. YubiKey PIV PIN. If empty, reads from GOAT_CA_PIN env var, else prompts on stdin.")
	tlsTrustAnchorPath := flag.String("tls-trust-anchor", "", "Optional. Path to PEM-encoded internal TLS root cert (ADR 0407). Embedded in bundle as tls_trust_anchor.")
	peerPubkeyB64 := flag.String("peer-pubkey", "", "Optional. Base64 32-byte mesh peer pubkey. If absent, a placeholder is generated (mesh-enrollment-via-bundle is Block 21/22H scope; today's bundles populate this for forward compatibility only).")
	updateAllowlistPath := flag.String("update-allowlist", "", "Optional. Path to a JSON allowlist file (operator-curated). After issuance, append this device's (name, pubkey, address) entry to the file (idempotent — replaces any existing entry by name). The relay-side wg-cp0 orchestrator reads this file as wg_cp0_bundle_allowlist_file at Pass 2A.")
	meshRouteSubnetRaw := flag.String("mesh-route-subnet", "", "Optional. Comma-separated CIDRs to add to EVERY relay's AllowedIPs in the device's wg-cp0.conf (e.g. \"198.18.0.0/24\"). When set, every relay peer becomes a valid forwarder for the listed subnets — eliminates the first-relay-special asymmetry class that prevented multi-peer consumers (mgmt-host Orins via wg-cp0-bundle-apply) from accepting traffic via any relay other than the one with the override. The relay's MeshAddr/32 is always retained alongside the subnet override. See docs/findings/F-114-wg-cp0-rotation-serialization-gap.md for the asymmetry-class incident that motivated the rename + every-relay semantics.")
	firstRelayRouteSubnetRaw := flag.String("first-relay-route-subnet", "", "Deprecated alias for --mesh-route-subnet. Pre-F-114 semantics applied the override to the FIRST relay only and produced the receive-side asymmetry that motivated the rename. The deprecated flag still accepts input but applies the new every-relay semantics (so existing operator scripts continue to produce working bundles); the old single-relay shape is no longer reachable. Will be removed in a future release.")
	outPath := flag.String("out", "", "Output path for signed bundle.cbor (mode 0600). Required.")
	ttlDays := flag.Int("ttl-days", defaultTTLDays, "Bundle expires_at horizon in days from now (default 90, per ADR 0308).")
	activationDays := flag.Int("activation-days", defaultActivationDays, "Bundle activation_deadline in days from now (default 14, per ADR 0308 §3.1).")
	snitch := flag.Bool("snitch", false, "Block 61. When true, generates a 32-byte snitch_probe_key and embeds it in the bundle. The device's snitch-bundle role hex-encodes + prefixes \"snitch_\" to form the bearer the probe presents to the collector.")
	snitchCollector := flag.String("snitch-collector", "", "Block 61. Optional. When --snitch=true and this is set, POSTs the generated key to <collector>/api/keys for registration. Auth via --snitch-admin-key or $SNITCH_ADMIN_KEY. Without this, the operator must register the key manually.")
	snitchAdminKey := flag.String("snitch-admin-key", "", "Block 61. Snitch admin API key (Bearer) for /api/keys POST. If empty, falls back to $SNITCH_ADMIN_KEY.")

	// Block 76 v0.2 / ADR 0840 Amendment populate
	// inner_mesh_setup so the issued bundle can drive `netbird-only`
	// or `combined` mode on the goat-client. All four flags are
	// optional; if any is set, --inner-mesh-mgmt-url and
	// --inner-mesh-setup-key are required.
	innerMeshMgmtURL := flag.String("inner-mesh-mgmt-url", "", "Block 76 v0.2. netbird management plane URL (https). Required when other --inner-mesh-* flags are set.")
	innerMeshSetupKey := flag.String("inner-mesh-setup-key", "", "Block 76 v0.2. netbird setup key. Required when other --inner-mesh-* flags are set.")
	innerMeshAdminToken := flag.String("inner-mesh-admin-token", "", "Block 76 v0.2. Optional. Bearer token for the netbird mgmt-API. Surfaced by getInnerMeshDiagnostics.")
	innerMeshPSKB64 := flag.String("inner-mesh-pre-shared-key", "", "Block 76 v0.2. Optional. Base64-encoded 32-byte WireGuard pre-shared key for the inner-mesh userspace device.")
	// Rosenpass (post-quantum) policy for the inner mesh — the goat-client
	// analogue of the `--enable-rosenpass` enrollment flag (rosenpass_enabled
	// group_var). Stamped into the bundle at mint so a goat-client device
	// brings its inner mesh up with PQC. Permissive defaults true (mixed-fleet
	// safe: a peer never strands itself against not-yet-enabled peers);
	// pass -inner-mesh-rosenpass-permissive=false for a fully-enabled fleet.
	innerMeshEnableRosenpass := flag.Bool("inner-mesh-enable-rosenpass", false, "Block 76 v0.2. Optional. Enable Rosenpass PQC on the inner mesh. Requires --inner-mesh-mgmt-url + --inner-mesh-setup-key.")
	innerMeshRosenpassPermissive := flag.Bool("inner-mesh-rosenpass-permissive", true, "Block 76 v0.2. Optional. When enabling Rosenpass, keep accepting plain WireGuard from not-yet-enabled peers (rollout-safe). Only meaningful with --inner-mesh-enable-rosenpass.")

	// Block 80F — when set, embeds the PEM-bundled mTLS client cert
	// + key (leaf cert, then any intermediate chain, then the
	// private key block) for reaching the Block 80 / ADR 0843
	// public-mTLS crutch tier in `netbird-only` mode.
	mobileCertPath := flag.String("mobile-cert", "", "Block 80F. Optional. Path to PEM file containing the per-device mTLS client cert + key (leaf + intermediates + private key, concatenated). Embedded in the bundle as mobile_cert.")

	// ADR 1019 / docs/design/goat-cli.md — operator-profile material
	// for the goat-cli (unified operator binary). All six flags are
	// optional; if any is set, ALL six are required (atomicity), and
	// the bundle gets a populated goat_cli_extras map covered by the
	// CA signature. Devices/relays don't carry this; only operator-
	// onboard bundles do.
	goatCLIPortalEndpoint := flag.String("goat-cli-portal-endpoint", "", "ADR 1019. Optional. Operator-facing portal URL (https), e.g. https://portal.galaxy.example. Required when any other --goat-cli-* flag is set.")
	goatCLIPortalTokenPath := flag.String("goat-cli-portal-token", "", "ADR 1019. Optional. Path to a file containing the opaque Bearer API key for the portal REST API (one line). Required when any other --goat-cli-* flag is set.")
	goatCLIRouterEndpoint := flag.String("goat-cli-router-endpoint", "", "ADR 1019. Optional. Zenoh router locator the goat-cli connects to, e.g. tls/router.galaxy.example:7447. Required when any other --goat-cli-* flag is set.")
	goatCLIOnboardEndpoint := flag.String("goat-cli-onboard-endpoint", "", "Optional. Onboarding-engine URL (https), e.g. https://onboard.galaxy.example — the goat-id-console machine realm the CLI targets for /api/onboard/* (docs/design/onboarding-extraction-plan.md). NOT part of the atomic --goat-cli-* set: omit it on a goatnet without the onboard route and the CLI falls back to the portal endpoint.")
	goatCLIMTLSCertPath := flag.String("goat-cli-mtls-cert", "", "ADR 1019. Optional. Path to PEM file containing the per-operator client cert (leaf, chain). Required when any other --goat-cli-* flag is set.")
	goatCLIMTLSKeyPath := flag.String("goat-cli-mtls-key", "", "ADR 1019. Optional. Path to PEM file containing the per-operator client private key. Required when any other --goat-cli-* flag is set.")
	goatCLICARootsPath := flag.String("goat-cli-ca-roots", "", "ADR 1019. Optional. Path to PEM bundle containing the CA root(s) the goat-cli verifies the portal + router server certs against. Required when any other --goat-cli-* flag is set.")

	// Block 22R.2 / ADR 0322 Property 4 — keypair lifecycle intent.
	// See keypair_reuse.go for the validation surface. Mutually
	// exclusive; either or neither, never both. When neither is set
	// AND the device-id is already in the peerset (via --update-
	// allowlist), bundle-create refuses to mint silently — the
	// previous F-114 trigger class is closed by structural fail-loud.
	reuseKeypairFrom := flag.String("reuse-keypair-from", "", "Block 22R.2. Optional. Reuse the Curve25519 pubkey of the named peer from --update-allowlist's peerset rather than minting a fresh one. The killer flag for additive-metadata operations (endpoint adds, TTL extensions, ACL group changes). Mutually exclusive with --rotate-keypair.")
	rotateKeypair := flag.Bool("rotate-keypair", false, "Block 22R.2. Optional. Mint a fresh keypair for an existing peer, record the rotation in pubkey_history[] with --reason \"<text>\". Mutually exclusive with --reuse-keypair-from. Requires --update-allowlist.")
	rotateReason := flag.String("reason", "", "Block 22R.2. Rotation audit reason. Required with --rotate-keypair; written to peerset.pubkey_history[].")

	flag.Parse()

	if err := run(runArgs{
		Site:                         *site,
		DeviceID:                     *deviceID,
		WGCP0Address:                 *wgCP0Address,
		RelaysPath:                   *relaysPath,
		ACLGroupsRaw:                 *aclGroupsRaw,
		FromMachine:                  *fromMachine,
		PortalURL:                    *portalURL,
		PortalOperatorKey:            os.Getenv("GOAT_PORTAL_OPERATOR_KEY"),
		TaxonomyPath:                 *taxonomyPath,
		CAID:                         *caID,
		Mode:                         *mode,
		CAKeyPath:                    *caKeyPath,
		AgeIdentityPath:              *ageIdentityPath,
		PIVSlotName:                  *pivSlotName,
		CardSelector:                 *cardSelector,
		PINFlag:                      *pinFlag,
		TLSTrustAnchorPath:           *tlsTrustAnchorPath,
		PeerPubkeyB64:                *peerPubkeyB64,
		UpdateAllowlistPath:          *updateAllowlistPath,
		MeshRouteSubnetRaw:           *meshRouteSubnetRaw,
		FirstRelayRouteSubnetRaw:     *firstRelayRouteSubnetRaw,
		OutPath:                      *outPath,
		TTLDays:                      *ttlDays,
		ActivationDays:               *activationDays,
		Snitch:                       *snitch,
		SnitchCollector:              *snitchCollector,
		SnitchAdminKey:               resolveSnitchAdminKey(*snitchAdminKey),
		InnerMeshMgmtURL:             *innerMeshMgmtURL,
		InnerMeshSetupKey:            *innerMeshSetupKey,
		InnerMeshAdminToken:          *innerMeshAdminToken,
		InnerMeshPSKB64:              *innerMeshPSKB64,
		InnerMeshEnableRosenpass:     *innerMeshEnableRosenpass,
		InnerMeshRosenpassPermissive: *innerMeshRosenpassPermissive,
		MobileCertPath:               *mobileCertPath,
		GoatCLIPortalEndpoint:        *goatCLIPortalEndpoint,
		GoatCLIPortalTokenPath:       *goatCLIPortalTokenPath,
		GoatCLIRouterEndpoint:        *goatCLIRouterEndpoint,
		GoatCLIOnboardEndpoint:       *goatCLIOnboardEndpoint,
		GoatCLIMTLSCertPath:          *goatCLIMTLSCertPath,
		GoatCLIMTLSKeyPath:           *goatCLIMTLSKeyPath,
		GoatCLICARootsPath:           *goatCLICARootsPath,
		ReuseKeypairFrom:             *reuseKeypairFrom,
		RotateKeypair:                *rotateKeypair,
		RotateReason:                 *rotateReason,
		Now:                          time.Now().UTC(),
	}); err != nil {
		fmt.Fprintf(os.Stderr, "bundle-create: %v\n", err)
		os.Exit(1)
	}
}

// resolveSnitchAdminKey returns the flag value if non-empty, otherwise
// falls back to $SNITCH_ADMIN_KEY. Empty result means "no admin key
// resolved"; validateArgs decides whether that's a fatal condition
// (only when --snitch-collector is set).
func resolveSnitchAdminKey(flagVal string) string {
	if flagVal != "" {
		return flagVal
	}
	return os.Getenv("SNITCH_ADMIN_KEY")
}

type runArgs struct {
	Site         string
	DeviceID     string
	WGCP0Address string
	RelaysPath   string
	ACLGroupsRaw string
	// machine-mesh-alignment D6a — issue-from-registry (see frommachine.go).
	// FromMachine is the portal slot anchor; mutually exclusive with
	// ACLGroupsRaw. PortalOperatorKey comes from $GOAT_PORTAL_OPERATOR_KEY,
	// never a flag. TaxonomyPath guards the hand-typed --acl-groups path
	// against gear-class groups.
	FromMachine       string
	PortalURL         string
	PortalOperatorKey string
	TaxonomyPath      string
	// portalHTTPClient overrides the portal HTTP client in tests (httptest
	// TLS server certs); nil = a default 30s-timeout client.
	portalHTTPClient         *http.Client
	CAID                     string
	Mode                     string // "dev" or "prod"
	CAKeyPath                string // dev mode
	AgeIdentityPath          string // dev mode
	PIVSlotName              string // prod mode
	CardSelector             string // prod mode
	PINFlag                  string // prod mode
	TLSTrustAnchorPath       string
	PeerPubkeyB64            string
	UpdateAllowlistPath      string
	MeshRouteSubnetRaw       string
	FirstRelayRouteSubnetRaw string // deprecated alias for MeshRouteSubnetRaw; see F-114
	OutPath                  string
	TTLDays                  int
	ActivationDays           int
	Snitch                   bool
	SnitchCollector          string
	SnitchAdminKey           string
	// Block 76 v0.2 / ADR 0840 Amendment inner-mesh
	// extension fields. All four are optional; if any is set,
	// InnerMeshMgmtURL + InnerMeshSetupKey are required.
	InnerMeshMgmtURL    string
	InnerMeshSetupKey   string
	InnerMeshAdminToken string
	InnerMeshPSKB64     string // base64-encoded 32 bytes when set
	// InnerMeshEnableRosenpass stamps InnerMeshSetup.RosenpassEnabled;
	// InnerMeshRosenpassPermissive stamps RosenpassPermissive (only when
	// enabling). Mint-time Rosenpass (PQC) policy for the inner mesh.
	InnerMeshEnableRosenpass     bool
	InnerMeshRosenpassPermissive bool
	// Block 80F — path to PEM-bundled mTLS client cert + key.
	MobileCertPath string
	// ADR 1019 — goat-cli operator-profile material. All six optional;
	// if any is set, ALL six are required (atomicity). See goatcli.go.
	GoatCLIPortalEndpoint  string
	GoatCLIPortalTokenPath string
	GoatCLIRouterEndpoint  string
	// GoatCLIOnboardEndpoint is OPTIONAL and NOT part of the atomic set above —
	// the onboarding-engine URL (goat-id-console) for the CLI's /api/onboard/*.
	GoatCLIOnboardEndpoint string
	GoatCLIMTLSCertPath    string
	GoatCLIMTLSKeyPath     string
	GoatCLICARootsPath     string
	// Block 22R.2 — keypair lifecycle intent. See keypair_reuse.go.
	// One or none of {ReuseKeypairFrom, RotateKeypair}; both set is a
	// validation error.
	ReuseKeypairFrom string
	RotateKeypair    bool
	RotateReason     string
	Now              time.Time
}

func run(args runArgs) error {
	if err := validateArgs(&args); err != nil {
		return err
	}

	// machine-mesh-alignment D6a — resolve the ACL groups before anything is
	// minted. --from-machine derives them from the portal registry (and yields
	// the provenance pair stamped into the bundle below); a hand-typed
	// --acl-groups list is refused if it names a gear class.
	var enrollmentAnchor, nodeClass string
	if args.FromMachine != "" {
		groupsRaw, anchor, nc, err := resolveFromMachine(args)
		if err != nil {
			return err
		}
		args.ACLGroupsRaw = groupsRaw
		enrollmentAnchor, nodeClass = anchor, nc
	} else if err := refuseHandTypedGearGroups(args.TaxonomyPath, parseACLGroups(args.ACLGroupsRaw)); err != nil {
		return err
	}

	// Operator-bundle mode (no --wg-cp0-relays): skip relay-roster
	// loading + keypair generation entirely. The bundle carries an
	// empty KnownEndpoints + zero-valued CPDevice* fields, and the
	// GoatCLIExtras nested map is the load-bearing content. Validated
	// in validateArgs (must have at least --goat-cli-portal-endpoint).
	var relays []relayEntry
	var intent *keypairIntent
	var cpPub, cpPriv []byte
	if args.RelaysPath != "" {
		loaded, err := loadRelaysFile(args.RelaysPath)
		if err != nil {
			return fmt.Errorf("load relays file: %w", err)
		}
		if len(loaded) == 0 {
			return errors.New("relays file is empty")
		}
		relays = loaded

		// Block 22R.2 — resolve keypair lifecycle intent BEFORE minting.
		// resolveKeypairIntent enforces mutual exclusion, fail-loud on
		// bare-for-existing-peer, and (for --reuse-keypair-from) pulls
		// the stored pubkey from the peerset. See keypair_reuse.go.
		intent, err = resolveKeypairIntent(args)
		if err != nil {
			return err
		}

		switch intent.Kind {
		case "reuse":
			// Reuse the stored pubkey; do NOT mint a privkey. The bundle
			// goes out with CPDevicePubkey set + CPDevicePrivkey nil; the
			// device-side handler preserves its existing privkey for
			// metadata-refresh bundles. See keypair_reuse.go header.
			cpPub = intent.ReusedPubkey
			cpPriv = nil
		default:
			// "new" and "rotate" both mint fresh keypairs. "rotate" also
			// records the rotation in peerset.pubkey_history[] after the
			// bundle is built (below).
			cpPub, cpPriv, err = generateWGKeypair()
			if err != nil {
				return fmt.Errorf("generate wg-cp0 keypair: %w", err)
			}
		}
	}

	var snitchProbeKey []byte
	if args.Snitch {
		var err error
		snitchProbeKey, err = generateSnitchProbeKey()
		if err != nil {
			return fmt.Errorf("generate snitch probe key: %w", err)
		}
	}

	peerPubkey, peerPlaceholder, err := resolvePeerPubkey(args.PeerPubkeyB64)
	if err != nil {
		return fmt.Errorf("peer pubkey: %w", err)
	}

	tlsTrustAnchor, err := loadTLSTrustAnchorIfSet(args.TLSTrustAnchorPath)
	if err != nil {
		return fmt.Errorf("load tls trust anchor: %w", err)
	}

	innerMeshSetup, err := buildInnerMeshSetup(args)
	if err != nil {
		return fmt.Errorf("inner mesh setup: %w", err)
	}

	mobileCert, err := loadMobileCertIfSet(args.MobileCertPath)
	if err != nil {
		return fmt.Errorf("load mobile cert: %w", err)
	}

	goatCLIExtras, err := buildGoatCLIExtras(args)
	if err != nil {
		return fmt.Errorf("goat-cli extras: %w", err)
	}

	nonce := make([]byte, nonceSize)
	if _, err := rand.Read(nonce); err != nil {
		return fmt.Errorf("gen nonce: %w", err)
	}

	// Prefer the new --mesh-route-subnet flag; fall through to the deprecated
	// --first-relay-route-subnet alias for back-compat. Both apply the new
	// every-relay semantics per F-114; the old flag name is preserved only so
	// existing operator scripts keep producing working bundles.
	routeSubnetRaw := args.MeshRouteSubnetRaw
	if routeSubnetRaw == "" {
		routeSubnetRaw = args.FirstRelayRouteSubnetRaw
	}
	routeSubnets, err := parseMeshRouteSubnets(routeSubnetRaw)
	if err != nil {
		return fmt.Errorf("--mesh-route-subnet: %w", err)
	}

	knownEndpoints := make([]bundle.KnownEndpoint, 0, len(relays))
	for _, r := range relays {
		ep := bundle.KnownEndpoint{
			Addr:     r.Endpoint,
			Pubkey:   r.PubkeyBytes,
			Kind:     bundle.Kind("cp-relay"),
			MeshAddr: r.Address,
		}
		// ADR 0321 — wg-cp0 is a perimeter, not an ACL layer. Every
		// relay peer block in every bundle carries the canonical
		// AllowedIPs = [MeshAddr/32, WGCP0FleetSubnet]. Membership in
		// wg-cp0 (bundle signed by the offline CA) = full mesh reach
		// at the WireGuard layer; fine-grained ACLs live one altitude
		// up at netbird-mesh, Ory OIDC, mTLS, Spock per-DB, Zenoh, and
		// the egress proxy.
		//
		// The --mesh-route-subnet / --first-relay-route-subnet flags
		// (Block 22K.7 / F-114 era) become ADDITIVE: any operator-
		// supplied extras append after the canonical pair, preserving
		// forward-compatibility with future route extensions without
		// re-introducing the asymmetric-/24 failure mode.
		// Pre-ADR-0321 the flag gated the catchall entirely; that
		// gate is now removed.
		ep.AllowedIPs = append([]string{r.Address + "/32", bundle.WGCP0FleetSubnet}, routeSubnets...)
		knownEndpoints = append(knownEndpoints, ep)
	}

	issued := args.Now
	b := &bundle.EnrollmentBundle{
		Version:            bundle.Version,
		DeviceID:           args.DeviceID,
		PeerPubkey:         peerPubkey,
		ACLGroups:          parseACLGroups(args.ACLGroupsRaw),
		Site:               args.Site,
		KnownEndpoints:     knownEndpoints,
		IssuedAt:           issued,
		ActivationDeadline: issued.Add(time.Duration(args.ActivationDays) * 24 * time.Hour),
		ExpiresAt:          issued.Add(time.Duration(args.TTLDays) * 24 * time.Hour),
		Nonce:              nonce,
		CAID:               args.CAID,
		CPDevicePubkey:     cpPub,
		CPDevicePrivkey:    cpPriv,
		CPDeviceAddress:    args.WGCP0Address,
		TLSTrustAnchor:     tlsTrustAnchor,
		SnitchProbeKey:     snitchProbeKey,
		InnerMeshSetup:     innerMeshSetup,
		MobileCert:         mobileCert,
		GoatCLIExtras:      goatCLIExtras,
		EnrollmentAnchor:   enrollmentAnchor,
		NodeClass:          nodeClass,
	}

	// The pairing invariant ("both halves present together or both
	// absent") holds for the "new" and "rotate" paths (mint produces
	// both halves) and for "reuse" (pub present + priv absent is
	// deliberately rejected by CheckCPDeviceKeypair; we skip the check
	// in that case — the device-side handler preserves its existing
	// privkey for metadata-refresh bundles per keypair_reuse.go).
	// In operator-bundle mode intent is nil (no wg-cp0 keypair was
	// minted); the pairing invariant is vacuously satisfied (both
	// halves absent).
	if intent != nil && intent.Kind != "reuse" {
		if err := b.CheckCPDeviceKeypair(); err != nil {
			return fmt.Errorf("internal pairing-invariant violation (bug): %w", err)
		}
	}

	signer, err := openSigner(args)
	if err != nil {
		return fmt.Errorf("open ca signer: %w", err)
	}
	defer signer.Close() //nolint:errcheck // signing result is complete before signer cleanup

	if err := b.Sign(signer.Sign); err != nil {
		return fmt.Errorf("sign bundle: %w", err)
	}
	if err := b.Verify(signer.PublicKey()); err != nil {
		return fmt.Errorf("verify just-signed bundle (sanity check): %w", err)
	}

	wire, err := b.Marshal()
	if err != nil {
		return fmt.Errorf("marshal bundle: %w", err)
	}

	// Register the probe key with the snitch collector BEFORE writing the
	// bundle file. If registration fails, we leave no bundle on disk so a
	// re-run regenerates a fresh key + retries cleanly. CreateKeyWithPlaintext
	// is idempotent on label (device-id) so a successful retry replaces the
	// prior registration.
	registered := false
	if args.Snitch && args.SnitchCollector != "" {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		err := registerSnitchKey(ctx, args.SnitchCollector, args.SnitchAdminKey, args.DeviceID, snitchProbeKey)
		cancel()
		if err != nil {
			return fmt.Errorf("register snitch probe key with %s: %w", args.SnitchCollector, err)
		}
		registered = true
	}

	if err := writeFileMode(args.OutPath, wire, 0o600); err != nil {
		return fmt.Errorf("write %s: %w", args.OutPath, err)
	}

	if args.UpdateAllowlistPath != "" && intent != nil {
		// Block 22R.1 — write through ops/enrollment/peerset so the JSON
		// of-record stays at schema v2. Bundle expiry mirrors the bundle
		// file's own expires_at horizon so an operator can audit
		// per-peer expiry from the allowlist alone.
		//
		// Block 22R.2 — keypair lifecycle intent determines the write path:
		//   - "reuse": no peerset mutation (the peer's CurrentPubkey is
		//     already what the new bundle carries; nothing changes).
		//   - "rotate": peerset.RotatePubkey records the rotation in
		//     pubkey_history[] with the operator-supplied --reason.
		//   - "new": peerset.Upsert appends (or replaces by name for the
		//     edge case where someone re-runs onboard without --reuse).
		// In operator-bundle mode intent is nil (no wg-cp0 keypair was
		// minted); allowlist update is meaningless and skipped above.
		switch intent.Kind {
		case "reuse":
			// Pubkey unchanged; metadata-refresh only. Skip the peerset
			// write so PubkeyHistory + LastUpdatedAt aren't perturbed.
		case "rotate":
			if err := rotateAllowlist(
				args.UpdateAllowlistPath,
				args.DeviceID,
				base64.StdEncoding.EncodeToString(cpPub),
				intent.RotateReason,
			); err != nil {
				return fmt.Errorf("rotate allowlist %s: %w", args.UpdateAllowlistPath, err)
			}
		default:
			if err := upsertAllowlist(
				args.UpdateAllowlistPath,
				allowlistEntry{
					Name:    args.DeviceID,
					Pubkey:  base64.StdEncoding.EncodeToString(cpPub),
					Address: addressIPOnly(args.WGCP0Address),
				},
				args.Site,
				"",
				b.ExpiresAt,
			); err != nil {
				return fmt.Errorf("update allowlist %s: %w", args.UpdateAllowlistPath, err)
			}
		}
	}

	emitSummary(os.Stderr, b, signer, args.OutPath, peerPlaceholder, len(relays), args.UpdateAllowlistPath, snitchProbeKey, registered, args.SnitchCollector)
	return nil
}

// addressIPOnly strips the prefix length from a CIDR string so the
// allowlist entry carries only the IP. The Ansible-side accumulator
// renders AllowedIPs = address/32, which would otherwise produce
// "198.18.0.6/24/32" for a CIDR-shaped input.
func addressIPOnly(cidr string) string {
	if i := strings.Index(cidr, "/"); i >= 0 {
		return cidr[:i]
	}
	return cidr
}

func validateArgs(a *runArgs) error {
	mode := a.Mode
	if mode == "" {
		mode = "dev"
		a.Mode = mode
	}
	if mode != "dev" && mode != "prod" {
		return fmt.Errorf("--mode must be dev or prod (got %q)", mode)
	}
	// Operator-bundle mode: when --goat-cli-portal-endpoint is set the
	// bundle is intended for an operator (no silent control-plane
	// participation), so --wg-cp0-address + --wg-cp0-relays become
	// optional. Operators run goat-cli + goat-client; they do not
	// terminate wg-cp0. When both modes are simultaneously requested
	// (the operator happens to also be on a wg-cp0-mgmt host), supply
	// the wg-cp0 flags explicitly and the bundle carries both shapes.
	operatorBundle := a.GoatCLIPortalEndpoint != ""
	// D6a — exactly one source of ACL groups: derived from the registry
	// (--from-machine) or hand-typed (--acl-groups, gear classes refused).
	if a.FromMachine != "" && a.ACLGroupsRaw != "" {
		return fmt.Errorf("--from-machine and --acl-groups are mutually exclusive: issuing from the registry means acl_groups are DERIVED from the machine's node-class, not hand-typed")
	}
	if a.FromMachine == "" && a.ACLGroupsRaw == "" {
		return fmt.Errorf("one of --acl-groups or --from-machine is required")
	}
	if a.FromMachine != "" {
		if a.PortalURL == "" {
			return fmt.Errorf("--portal-url is required with --from-machine")
		}
		if !strings.HasPrefix(a.PortalURL, "https://") {
			return fmt.Errorf("--portal-url must be https; got %q", a.PortalURL)
		}
		if a.PortalOperatorKey == "" {
			return fmt.Errorf("--from-machine needs the portal operator API key in $GOAT_PORTAL_OPERATOR_KEY (an env var, never a flag)")
		}
	}
	required := []struct {
		name string
		val  string
	}{
		{"--site", a.Site},
		{"--device-id", a.DeviceID},
		{"--ca-id", a.CAID},
		{"--out", a.OutPath},
	}
	if !operatorBundle {
		required = append(required,
			struct {
				name string
				val  string
			}{"--wg-cp0-address", a.WGCP0Address},
			struct {
				name string
				val  string
			}{"--wg-cp0-relays", a.RelaysPath},
		)
	}
	for _, r := range required {
		if r.val == "" {
			return fmt.Errorf("%s is required", r.name)
		}
	}
	if operatorBundle && (a.WGCP0Address != "") != (a.RelaysPath != "") {
		return fmt.Errorf("--wg-cp0-address and --wg-cp0-relays must be set together (or both omitted in operator-bundle mode)")
	}
	if mode == "dev" {
		if a.CAKeyPath == "" {
			return fmt.Errorf("--ca-key is required in dev mode")
		}
		if a.AgeIdentityPath == "" {
			return fmt.Errorf("--age-identity is required in dev mode")
		}
	}
	if a.TTLDays <= 0 {
		return fmt.Errorf("--ttl-days must be positive")
	}
	if a.ActivationDays <= 0 {
		return fmt.Errorf("--activation-days must be positive")
	}
	if a.ActivationDays > a.TTLDays {
		return fmt.Errorf("--activation-days (%d) cannot exceed --ttl-days (%d)", a.ActivationDays, a.TTLDays)
	}
	if a.WGCP0Address != "" {
		if _, _, err := net.ParseCIDR(a.WGCP0Address); err != nil {
			return fmt.Errorf("--wg-cp0-address must be a CIDR (e.g. 198.18.0.6/24): %w", err)
		}
	}
	if a.SnitchCollector != "" {
		if !a.Snitch {
			return fmt.Errorf("--snitch-collector requires --snitch=true")
		}
		if a.SnitchAdminKey == "" {
			return fmt.Errorf("--snitch-collector set but admin key empty: pass --snitch-admin-key or set $SNITCH_ADMIN_KEY")
		}
	}
	return nil
}

// resolvePeerPubkey returns a 32-byte mesh peer pubkey. If the operator
// supplied --peer-pubkey, it is decoded and validated. Otherwise a random
// placeholder is generated and the second return is true so the summary
// can flag it. Block 21/22H lands real mesh-enrollment-via-bundle support.
func resolvePeerPubkey(b64 string) ([]byte, bool, error) {
	if b64 == "" {
		buf := make([]byte, peerPubkeySize)
		if _, err := rand.Read(buf); err != nil {
			return nil, false, fmt.Errorf("gen placeholder peer pubkey: %w", err)
		}
		return buf, true, nil
	}
	raw, err := base64.StdEncoding.DecodeString(b64)
	if err != nil {
		return nil, false, fmt.Errorf("base64 decode peer pubkey: %w", err)
	}
	if len(raw) != peerPubkeySize {
		return nil, false, fmt.Errorf("peer pubkey wrong size: got %d want %d", len(raw), peerPubkeySize)
	}
	return raw, false, nil
}

func loadTLSTrustAnchorIfSet(path string) ([]byte, error) {
	if path == "" {
		return nil, nil
	}
	pemBytes, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	block, _ := pem.Decode(pemBytes)
	if block == nil {
		return nil, fmt.Errorf("no PEM block in %s", path)
	}
	if block.Type != "CERTIFICATE" {
		return nil, fmt.Errorf("expected CERTIFICATE block, got %q in %s", block.Type, path)
	}
	return block.Bytes, nil
}

// buildInnerMeshSetup constructs the v0.2 InnerMeshSetup nested struct
// from the --inner-mesh-* flags. Returns the zero value when no flags
// are set (so the CBOR omitempty drops the field cleanly). When any
// flag is set, --inner-mesh-mgmt-url + --inner-mesh-setup-key are
// required (the others are optional). PSK is base64-decoded and must
// be exactly 32 bytes when present.
func buildInnerMeshSetup(args runArgs) (bundle.InnerMeshSetup, error) {
	anySet := args.InnerMeshMgmtURL != "" ||
		args.InnerMeshSetupKey != "" ||
		args.InnerMeshAdminToken != "" ||
		args.InnerMeshPSKB64 != "" ||
		args.InnerMeshEnableRosenpass
	// Note: InnerMeshRosenpassPermissive defaults true, so it is NOT part of
	// anySet — it is a modifier that only takes effect when Rosenpass is
	// enabled (and the mesh setup is otherwise present).
	if !anySet {
		return bundle.InnerMeshSetup{}, nil
	}
	if args.InnerMeshMgmtURL == "" {
		return bundle.InnerMeshSetup{}, errors.New("--inner-mesh-mgmt-url is required when any other --inner-mesh-* flag is set")
	}
	if args.InnerMeshSetupKey == "" {
		return bundle.InnerMeshSetup{}, errors.New("--inner-mesh-setup-key is required when any other --inner-mesh-* flag is set")
	}
	if !strings.HasPrefix(args.InnerMeshMgmtURL, "https://") {
		return bundle.InnerMeshSetup{}, fmt.Errorf("--inner-mesh-mgmt-url must be https; got %q", args.InnerMeshMgmtURL)
	}
	setup := bundle.InnerMeshSetup{
		ManagementURL:    args.InnerMeshMgmtURL,
		SetupKey:         args.InnerMeshSetupKey,
		AdminAccessToken: args.InnerMeshAdminToken,
	}
	if args.InnerMeshEnableRosenpass {
		setup.RosenpassEnabled = true
		// Permissive is only meaningful when Rosenpass is on; stamp it only
		// then so a non-PQC bundle stays clean (CBOR omitempty drops false).
		setup.RosenpassPermissive = args.InnerMeshRosenpassPermissive
	}
	if args.InnerMeshPSKB64 != "" {
		psk, err := base64.StdEncoding.DecodeString(args.InnerMeshPSKB64)
		if err != nil {
			return bundle.InnerMeshSetup{}, fmt.Errorf("--inner-mesh-pre-shared-key decode: %w", err)
		}
		if len(psk) != 32 {
			return bundle.InnerMeshSetup{}, fmt.Errorf("--inner-mesh-pre-shared-key must decode to 32 bytes; got %d", len(psk))
		}
		setup.PreSharedKey = psk
	}
	return setup, nil
}

// loadMobileCertIfSet reads the PEM-bundled mTLS client cert + key
// from path, returning the raw PEM bytes for embedding in the bundle.
// Returns (nil, nil) when path is empty. Validates that at least one
// CERTIFICATE block + one PRIVATE KEY-shaped block are present (the
// device-side parser does the strict PEM walk; this is a smoke check).
func loadMobileCertIfSet(path string) ([]byte, error) {
	if path == "" {
		return nil, nil
	}
	pemBytes, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var hasCert, hasKey bool
	rest := pemBytes
	for {
		var block *pem.Block
		block, rest = pem.Decode(rest)
		if block == nil {
			break
		}
		switch block.Type {
		case "CERTIFICATE":
			hasCert = true
		case "PRIVATE KEY", "EC PRIVATE KEY", "RSA PRIVATE KEY":
			hasKey = true
		}
	}
	if !hasCert {
		return nil, fmt.Errorf("--mobile-cert: no CERTIFICATE block found in %s", path)
	}
	if !hasKey {
		return nil, fmt.Errorf("--mobile-cert: no PRIVATE KEY block found in %s (need leaf + key in same PEM)", path)
	}
	return pemBytes, nil
}

func parseACLGroups(raw string) []string {
	parts := strings.Split(raw, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p != "" {
			out = append(out, p)
		}
	}
	return out
}

// parseMeshRouteSubnets parses the comma-separated --mesh-route-subnet (or
// deprecated --first-relay-route-subnet) flag value into validated CIDR strings.
// Empty input returns (nil, nil) — the flag is optional. Each non-empty entry
// must parse via net.ParseCIDR; canonicalized to the form CIDR.String() returns
// so issuer + validator agree byte-for-byte. Block 22K.7 (Path W) + F-114.
func parseMeshRouteSubnets(raw string) ([]string, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil, nil
	}
	parts := strings.Split(raw, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p == "" {
			continue
		}
		_, ipnet, err := net.ParseCIDR(p)
		if err != nil {
			return nil, fmt.Errorf("invalid CIDR %q: %w", p, err)
		}
		out = append(out, ipnet.String())
	}
	if len(out) == 0 {
		return nil, fmt.Errorf("no valid CIDRs in %q", raw)
	}
	return out, nil
}

// openSigner dispatches to the dev or prod signer backend per
// args.Mode. Both backends satisfy ca.Signer; bundle-create stays
// backend-agnostic from the call site.
func openSigner(args runArgs) (ca.Signer, error) {
	switch args.Mode {
	case "", "dev":
		return openAgeSigner(args.CAID, args.CAKeyPath, args.AgeIdentityPath)
	case "prod":
		return openYubikeySigner(args.CAID, args.PIVSlotName, args.CardSelector, args.PINFlag)
	default:
		return nil, fmt.Errorf("unknown mode %q", args.Mode)
	}
}

func openAgeSigner(caID, keyPath, ageIdentityPath string) (ca.Signer, error) {
	f, err := os.Open(ageIdentityPath)
	if err != nil {
		return nil, fmt.Errorf("open age identity: %w", err)
	}
	defer f.Close()
	identities, err := age.ParseIdentities(f)
	if err != nil {
		return nil, fmt.Errorf("parse age identity: %w", err)
	}
	return ca.NewAgeSigner(caID, keyPath, identities)
}

func writeFileMode(path string, data []byte, mode os.FileMode) error {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, mode)
	if err != nil {
		return err
	}
	if _, err := f.Write(data); err != nil {
		return errors.Join(err, f.Close())
	}
	return f.Close()
}

func emitSummary(w *os.File, b *bundle.EnrollmentBundle, signer ca.Signer, outPath string, peerPlaceholder bool, relayCount int, allowlistPath string, snitchProbeKey []byte, snitchRegistered bool, snitchCollector string) {
	fmt.Fprintf(w, "\nBundle issued.\n")
	fmt.Fprintf(w, "  device_id:           %s\n", b.DeviceID)
	fmt.Fprintf(w, "  site:                %s\n", b.Site)
	fmt.Fprintf(w, "  ca_id:               %s\n", b.CAID)
	fmt.Fprintf(w, "  acl_groups:          %s\n", strings.Join(b.ACLGroups, ","))
	if b.EnrollmentAnchor != "" {
		fmt.Fprintf(w, "  enrollment_anchor:   %s (issued from the portal registry; acl_groups derived from node_class=%s)\n", b.EnrollmentAnchor, b.NodeClass)
	}
	if len(b.CPDevicePubkey) > 0 {
		fmt.Fprintf(w, "  cp_device_pubkey:    %s\n", base64.StdEncoding.EncodeToString(b.CPDevicePubkey))
		fmt.Fprintf(w, "  cp-relays:           %d\n", relayCount)
	} else {
		fmt.Fprintf(w, "  cp_device_pubkey:    (none — operator-bundle mode)\n")
	}
	fmt.Fprintf(w, "  issued_at:           %s\n", b.IssuedAt.UTC().Format(time.RFC3339))
	fmt.Fprintf(w, "  activation_deadline: %s\n", b.ActivationDeadline.UTC().Format(time.RFC3339))
	fmt.Fprintf(w, "  expires_at:          %s\n", b.ExpiresAt.UTC().Format(time.RFC3339))
	fmt.Fprintf(w, "  out:                 %s (mode 0600)\n", outPath)
	if allowlistPath != "" {
		fmt.Fprintf(w, "  allowlist:           %s (entry upserted by name)\n", allowlistPath)
	}
	if len(snitchProbeKey) > 0 {
		// Bearer string is the literal value the device-side env-template
		// will render (snitch_<hex>); print it once so the operator can
		// register manually if the auto-register path is not used.
		fmt.Fprintf(w, "  snitch_probe_key:    snitch_%s (32 random bytes; bundle-extract --hex)\n", hex.EncodeToString(snitchProbeKey))
		switch {
		case snitchRegistered:
			fmt.Fprintf(w, "  snitch_collector:    %s (key registered as label=%q)\n", snitchCollector, b.DeviceID)
		case snitchCollector != "":
			// Unreachable on the success path (registration failure aborts
			// before file write), but kept for defensive symmetry in case
			// the order is changed later.
			fmt.Fprintf(w, "  snitch_collector:    %s (registration NOT attempted on this run)\n", snitchCollector)
		default:
			fmt.Fprintf(w, "  snitch_collector:    (none — register the bearer above with `curl -X POST <collector>/api/keys`)\n")
		}
	}
	if b.HasGoatCLIExtras() {
		fmt.Fprintf(w, "  goat_cli_extras:     embedded (portal=%s, router=%s, mtls_cert+key+ca_roots, opaque token)\n",
			b.GoatCLIExtras.PortalEndpoint, b.GoatCLIExtras.RouterEndpoint)
	}
	if peerPlaceholder {
		fmt.Fprintf(w, "\n  WARNING: peer_pubkey is a randomly-generated placeholder.\n")
		fmt.Fprintf(w, "    Mesh-enrollment-via-bundle is Block 21 / 22H scope; until those\n")
		fmt.Fprintf(w, "    land, the field is forward-compat material only. The wg-cp0\n")
		fmt.Fprintf(w, "    keypair (cp_device_*) IS the load-bearing material in this bundle.\n")
	}
	fmt.Fprintf(w, "\nNext steps:\n")
	if len(b.CPDevicePubkey) == 0 {
		// Operator-bundle mode — the device-side renderer is goat-cli,
		// not wg-cp0-bundle-apply. Direct the operator at `goat profile
		// init` rather than the wg-cp0-bootstrap playbook.
		fmt.Fprintf(w, "  1. Deliver %s to the operator out-of-band (mode 0600).\n", outPath)
		fmt.Fprintf(w, "  2. Operator runs: goat profile init %s\n", outPath)
		fmt.Fprintf(w, "     → cert + key + CA roots land in ~/.goat/contexts/default/.\n")
		fmt.Fprintf(w, "  3. Verify: goat doctor (4/4 green expected when on-mesh).\n")
		_ = signer
		return
	}
	if allowlistPath == "" {
		fmt.Fprintf(w, "  1. Register cp_device_pubkey in each relay's wg-cp0 allowlist\n")
		fmt.Fprintf(w, "     (operator-fired Phase 2A; Spock-replicated in Phase 3) — OR re-run\n")
		fmt.Fprintf(w, "     with --update-allowlist <path> to skip the manual `wg set` step.\n")
	} else {
		fmt.Fprintf(w, "  1. Re-run wg-cp0-bootstrap.yml with -e wg_cp0_bundle_allowlist_file=%s\n", allowlistPath)
		fmt.Fprintf(w, "     to push the new peer into every relay's wg-cp0 allowlist.\n")
	}
	fmt.Fprintf(w, "  2. ansible-playbook wg-cp0-bootstrap.yml \\\n")
	fmt.Fprintf(w, "       -e wg_cp0_bundle_path=%s \\\n", outPath)
	fmt.Fprintf(w, "       -e wg_cp0_ca_pubkey_local_path=<path-to-%s.pem>\n", b.CAID)
	fmt.Fprintf(w, "  3. Verify: wg show wg-cp0 (handshake < 3 min on every relay).\n")
	_ = signer
}
