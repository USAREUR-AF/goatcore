package api

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/pem"
	"fmt"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// Reserved top-level Zenoh key prefixes on the trial fabric.
// Trial-fabric scope only; the production envelope ([ADR 0844](docs/adr/0844-zenoh-deployment-topology-and-acl-v1.md))
// declares a 12-prefix list which is not yet load-bearing here. Anything
// outside this set is allocatable to a vendor / partner prefix at mint
// time. Kept in sync with registry.ValidVendorPrefix's reserved-list so
// the surface advertised at GET /api/fabric/config matches what the
// register-vendor path actually blocks.
var reservedTopLevelPrefixes = []string{
	"ops",    // canonical operator/control namespace (ADR 0844 / ADR 1035)
	"attest", // schema declarations
	"system", // deprecated operator namespace; reserved transitionally (ADR 1035)
}

// fabricConfigResponse is the JSON shape served at GET /api/fabric/config.
// Stable, intentionally narrow — partners + first-time-Zenoh participants
// read this to understand how to connect and what's already taken. No
// auth; this surface is meant to be discoverable.
type fabricConfigResponse struct {
	FabricName        string                `json:"fabric_name"`
	Maturity          string                `json:"maturity"`
	Router            routerInfo            `json:"router"`
	TLS               tlsPosture            `json:"tls"`
	Zenoh             zenohInfo             `json:"zenoh"`
	Namespacing       namespacingInfo       `json:"namespacing"`
	Registration      registrationInfo      `json:"registration"`
	WireFormats       wireFormatsInfo       `json:"wire_formats"`
	OnboardingFlavors []onboardingFlavor    `json:"onboarding_flavors"`
	FeedbackChannel   string                `json:"feedback_channel,omitempty"`
}

type routerInfo struct {
	Endpoint string `json:"endpoint"`
}

type tlsPosture struct {
	MTLSRequired         bool          `json:"mtls_required"`
	VerifyNameOnConnect  bool          `json:"verify_name_on_connect"`
	RootCA               rootCAInfo    `json:"root_ca"`
	ClientCertOrigin     string        `json:"client_cert_origin"`
}

type rootCAInfo struct {
	BundleURL         string `json:"bundle_url,omitempty"`
	FingerprintSHA256 string `json:"fingerprint_sha256,omitempty"`
}

type zenohInfo struct {
	Version                    string `json:"version"`
	RecommendedPartnerListenMode string `json:"recommended_partner_listen_mode"`
}

type namespacingInfo struct {
	VendorGrammar             string   `json:"vendor_grammar"`
	ReservedTopLevelPrefixes  []string `json:"reserved_top_level_prefixes"`
	TakenVendorPrefixes       []string `json:"taken_vendor_prefixes"`
	URISANPrefix              string   `json:"uri_san_prefix"`
}

type registrationInfo struct {
	PortalURL                string `json:"portal_url,omitempty"`
	CertMintEndpoint         string `json:"cert_mint_endpoint"`
	TopicRegistrationEndpoint string `json:"topic_registration_endpoint"`
	SchemaUploadEndpoint     string `json:"schema_upload_endpoint"`
}

type wireFormatsInfo struct {
	Recommended []string `json:"recommended"`
	Supported   []string `json:"supported"`
	Note        string   `json:"note"`
}

type onboardingFlavor struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	Audience    string `json:"audience"`
}

// rootCAFingerprintCache memoizes the sha256 of the root CA file. Bound to
// the Server in a sync.Once so the disk read happens at most once per
// process lifetime; subsequent /api/fabric/config requests hit the cached
// string. If the path is empty or the file is unreadable, the cached
// value is "" and the endpoint omits the field rather than 500'ing.
type rootCAFingerprintCache struct {
	once sync.Once
	hex  string
}

var serverFingerprints sync.Map // *Server -> *rootCAFingerprintCache

func (s *Server) rootCAFingerprintHex() string {
	v, _ := serverFingerprints.LoadOrStore(s, &rootCAFingerprintCache{})
	cache := v.(*rootCAFingerprintCache)
	cache.once.Do(func() {
		path := s.cfg.CARootCert
		if path == "" {
			return
		}
		raw, err := os.ReadFile(path)
		if err != nil {
			return
		}
		// PEM-encoded cert is the common case. Decode and hash the DER
		// body so the fingerprint matches what `openssl x509 -fingerprint
		// -sha256 -noout -in root.pem` returns.
		der := raw
		if block, _ := pem.Decode(raw); block != nil && block.Type == "CERTIFICATE" {
			der = block.Bytes
		}
		sum := sha256.Sum256(der)
		cache.hex = strings.ToUpper(hex.EncodeToString(sum[:]))
	})
	return cache.hex
}

// handleFabricConfig serves the public self-description of this Zenoh fabric.
// No auth — the whole point is that prospective partners + first-time-Zenoh
// participants can read this to know how to connect and what's taken. The
// payload is composed from already-known portal config + a live query of
// currently-approved vendor prefixes.
//
// The shape is intentionally generic so a
// production portal can lift it untouched.
func (s *Server) handleFabricConfig(w http.ResponseWriter, r *http.Request) {
	taken, err := s.reg.ApprovedVendorPrefixes(r.Context())
	if err != nil {
		// Surface as 503 — the response would be misleading without the
		// "what's taken" list, and partners would design their prefix
		// proposal against a phantom-clean namespace.
		writeError(w, http.StatusServiceUnavailable, "registry_unavailable",
			"could not list approved org prefixes")
		return
	}
	if taken == nil {
		taken = []string{}
	}

	resp := fabricConfigResponse{
		FabricName: s.cfg.FabricName,
		Maturity:   "two-week-hackathon",
		Router: routerInfo{
			Endpoint: s.cfg.RouterEndpoint,
		},
		TLS: tlsPosture{
			MTLSRequired:        true,
			VerifyNameOnConnect: true,
			RootCA: rootCAInfo{
				BundleURL:         rootCABundleURLFromConfig(s.cfg.PortalPublicURL),
				FingerprintSHA256: s.rootCAFingerprintHex(),
			},
			ClientCertOrigin: "minted by this portal at vendor registration; URI-SAN-pinned",
		},
		Zenoh: zenohInfo{
			Version:                      "1.9.0",
			RecommendedPartnerListenMode: "client (your router dials ours; mTLS required)",
		},
		Namespacing: namespacingInfo{
			VendorGrammar:            "<prefix>/<system>/<arbitrary>/v<N>",
			ReservedTopLevelPrefixes: reservedTopLevelPrefixes,
			TakenVendorPrefixes:      taken,
			URISANPrefix:             s.cfg.CertSignURIPrefix,
		},
		Registration: registrationInfo{
			PortalURL:                 s.cfg.PortalPublicURL,
			CertMintEndpoint:          "POST /api/vendors",
			TopicRegistrationEndpoint: "POST /api/vendors/{prefix}/topics",
			SchemaUploadEndpoint:      "POST /api/vendors/{prefix}/topics/{id}/schemas",
		},
		WireFormats: wireFormatsInfo{
			Recommended: []string{"protobuf:<MsgName>"},
			Supported:   []string{"protobuf:<MsgName>", "json", "cbor", "msgpack", "text", "raw"},
			Note:        "Topic format field declares the wire shape. protobuf:<MsgName> + an uploaded .fds descriptor lights up the portal's schema viewer + per-language code examples + inspector decode.",
		},
		OnboardingFlavors: []onboardingFlavor{
			{
				Name:        "publisher",
				Description: "Single publisher process connects directly to our router in client mode. Smallest footprint — install eclipse-zenoh, point at our endpoint, publish.",
				Audience:    "Vendor with one (or a few) processes to publish, no infrastructure on their side.",
			},
			{
				Name:        "router-peer",
				Description: "Your own zenohd connects to our router; your downstream stack connects to YOUR router. Same cert-mint flow, but the generated bundle contains a zenohd.json5 instead of a Python publisher snippet.",
				Audience:    "Partner who already runs their own Zenoh deployment and wants federation rather than per-process tenancy.",
			},
		},
		FeedbackChannel: s.cfg.FeedbackChannel,
	}

	writeJSON(w, http.StatusOK, resp)
}

// rootCABundleURLFromConfig computes a public URL for the root CA file
// based on the portal's public URL. Returns "" if no public URL is set
// (dev environments), in which case the JSON omits the field.
func rootCABundleURLFromConfig(portalPublicURL string) string {
	if portalPublicURL == "" {
		return ""
	}
	return strings.TrimRight(portalPublicURL, "/") + "/peering/root-ca.pem"
}

// zenohdConfigTemplate is the json5 fragment served at /api/fabric/zenohd-config.
// Placeholders are %s-substituted (no template engine — too easy to break the
// json5 with stray escaping). The %s order is: routerEndpoint, mountedRootCA,
// mountedKey, mountedCert, mountedKey, mountedCert, generatedFor, prefix.
//
// json5 is JSON-with-comments-and-trailing-commas — zenohd accepts it
// natively. We embed comments so the partner can read what each block does
// without flipping to the upstream Zenoh docs.
const zenohdConfigTemplate = `// zenohd.json5 generated for prefix=%[8]s on %[7]s.
//
// What this is: a router-mode zenohd config that connects OUT to the
// fabric router over mTLS and listens for your local
// downstream stack on tcp/127.0.0.1:7447. Your publishers + subscribers
// connect to YOUR router; your router peers with ours. Drop this into a
// container or process. Cert paths assume bind-mounts at /cert/.
//
// The fabric router enforces "you may only publish under %[8]s/**"
// via the URI-SAN in your client cert — that's the trust boundary, not
// this file's access_control block.
{
  mode: "router",
  connect: {
    endpoints: ["%[1]s"],
  },
  listen: {
    // Your downstream publishers + subscribers connect here. The trust
    // boundary is the upstream mTLS to us; local stack is trusted.
    endpoints: ["tcp/127.0.0.1:7447"],
  },
  transport: {
    link: {
      tls: {
        enable_mtls: true,
        verify_name_on_connect: true,
        root_ca_certificate: "%[2]s",
        listen_private_key: "%[3]s",
        listen_certificate: "%[4]s",
        connect_private_key: "%[5]s",
        connect_certificate: "%[6]s",
      },
    },
  },
  access_control: {
    // Local stack is trusted; per-prefix enforcement happens on our
    // side via the URI-SAN matcher.
    enabled: false,
  },
}
`

// handleFabricZenohdConfig returns a ready-to-paste zenohd.json5 config
// for partners who want to run their own router and federate with ours
// (Flavor B in the Peering Guide). The prefix query param is the
// partner's intended vendor prefix; it's validated against the same
// regex registry.ValidVendorPrefix uses + checked for reserved-prefix
// collision.
//
// Response contracts:
//   - 200 + Content-Type "application/json5; charset=utf-8" — config body
//   - 400 "invalid_prefix"   — prefix fails the format check
//   - 409 "reserved_prefix"  — prefix collides with ops/attest/system/etc.
//   - 405 if a method other than GET is used
//
// The endpoint deliberately does NOT 409 on collision-with-existing-
// vendor — a partner may legitimately re-fetch their own existing config,
// and the mint step elsewhere is the authoritative gate on prefix
// allocation. The header X-Prefix-Status carries "available" / "taken-
// existing-vendor" so the caller can disambiguate if they want.
func (s *Server) handleFabricZenohdConfig(w http.ResponseWriter, r *http.Request) {
	prefix := strings.TrimSpace(r.URL.Query().Get("prefix"))
	if prefix == "" {
		writeError(w, http.StatusBadRequest, "invalid_prefix",
			"query param 'prefix' is required (e.g. ?prefix=my-vendor)")
		return
	}
	// Check reserved-prefix collision BEFORE the generic format check so
	// "ops" / "attest" / "system" return 409 (well-formed but reserved) rather
	// than 400 (malformed). ValidVendorPrefix's own reserved-check would
	// otherwise collapse both into a single 400.
	for _, reserved := range reservedTopLevelPrefixes {
		if prefix == reserved {
			writeError(w, http.StatusConflict, "reserved_prefix",
				"prefix '"+prefix+"' is reserved by the fabric; pick another")
			return
		}
	}
	if err := registry.ValidVendorPrefix(prefix); err != nil {
		writeError(w, http.StatusBadRequest, "invalid_prefix", err.Error())
		return
	}

	// Collision-with-existing-vendor — informational only; we still
	// return the config so the existing vendor can re-fetch theirs.
	prefixStatus := "available"
	if taken, err := s.reg.ApprovedVendorPrefixes(r.Context()); err == nil {
		for _, t := range taken {
			if t == prefix {
				prefixStatus = "taken-existing-vendor"
				break
			}
		}
	}

	routerEndpoint := s.cfg.RouterEndpoint
	if routerEndpoint == "" {
		routerEndpoint = "tls/zenoh.goat.example:7447"
	}
	body := fmt.Sprintf(zenohdConfigTemplate,
		routerEndpoint,                  // %[1]s
		"/cert/root-cert.pem",           // %[2]s
		"/cert/key.pem",                 // %[3]s
		"/cert/cert.pem",                // %[4]s
		"/cert/key.pem",                 // %[5]s
		"/cert/cert.pem",                // %[6]s
		time.Now().UTC().Format(time.RFC3339), // %[7]s
		prefix,                          // %[8]s
	)

	w.Header().Set("Content-Type", "application/json5; charset=utf-8")
	w.Header().Set("Content-Disposition", `inline; filename="zenohd.json5"`)
	w.Header().Set("X-Prefix-Status", prefixStatus)
	_, _ = w.Write([]byte(body))
}

// handleRootCABundle serves the fabric CA root certificate PEM as
// application/x-pem-file. No auth — a public trust anchor is meant to be
// publicly readable so partners can pin it before initiating any mTLS
// handshake (chicken-and-egg without this).
//
// 404 if the cert path isn't configured (dev mode). 500 if it's configured
// but unreadable — that's an operator misconfiguration, not a vendor-
// facing condition we should mask.
func (s *Server) handleRootCABundle(w http.ResponseWriter, r *http.Request) {
	path := s.cfg.CARootCert
	if path == "" {
		writeError(w, http.StatusNotFound, "no_root_ca",
			"this portal has no CA root cert configured (--ca-root-cert); ask the operator")
		return
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		writeError(w, http.StatusInternalServerError, "root_ca_unreadable",
			"the root cert path is configured but unreadable; tell the operator")
		return
	}
	w.Header().Set("Content-Type", "application/x-pem-file")
	w.Header().Set("Content-Disposition", fmt.Sprintf(`inline; filename=%q`, rootCADownloadName(s.cfg.OnboardingRootCertName)))
	_, _ = w.Write(raw)
}

// rootCADownloadName is the filename the root-CA download is saved under:
// the goatnet's in-tarball CA-root name, so the file a partner downloads
// matches the one the onboarding tarball and sample scripts reference.
func rootCADownloadName(name string) string {
	if name = strings.TrimSpace(name); name != "" && !strings.ContainsAny(name, "/\\\"") {
		return name
	}
	return "root-ca.pem"
}

// Ensure context import stays referenced if the linter is strict about
// transitively-needed imports in future refactors of this file.
var _ context.Context
