package x509ca

// Subject defaults applied when a caller leaves the field empty. They are
// baked into every certificate issued without an explicit value, so a change
// here changes the subject of every root, intermediate and leaf minted after
// it. Callers that need a different subject pass it in the options: cert-sign
// and ca-init take --org, and ca-init takes --root-cn and --intermediate-cn.
const (
	// DefaultOrganization is the Subject O of roots, intermediates and
	// leaves issued without an explicit Organization.
	DefaultOrganization = "goat"
	// DefaultRootCommonName is the Subject CN of a root issued without an
	// explicit CommonName.
	DefaultRootCommonName = "goat Root CA"
	// DefaultIntermediateCommonName is the Subject CN of an intermediate
	// issued without an explicit CommonName.
	DefaultIntermediateCommonName = "goat Intermediate CA"
)
