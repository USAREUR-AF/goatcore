package config

import "strings"

// Service operator handles. A key under one of these names belongs to a
// service that authenticates the human itself, so the portal lets its holder
// name that human in X-Goat-Operator-Actor (api.actorOf). That makes the
// names worth more than any person's handle: whoever holds a key filed under
// one can record actions as anyone (GHSA-q7qx). Each therefore has exactly one
// place it may come from, and every other source refuses it.
const (
	// ServiceHandleOperator is the legacy shared key. Its only source is
	// PORTAL_OPERATOR_API_KEY (Config.OperatorAPIKey), which the portal
	// registers under this name itself.
	ServiceHandleOperator = "operator"
	// ServiceHandleIDConsole is goat-id-console's outbound portal key. Its
	// only source is an `id-console:` entry in PORTAL_OPERATOR_API_KEYS, set
	// by the deployment alongside the console's PORTAL_LOOKUP_TOKEN.
	ServiceHandleIDConsole = "id-console"
	// ServiceHandleOnboardAPI is goat-id-console's fallback attribution
	// label. No portal key is ever issued under it.
	ServiceHandleOnboardAPI = "onboard-api"
)

// IsServiceOperatorHandle reports whether a handle is one of the service
// labels the portal trusts to delegate. Exact match after trimming: this is
// the trust decision, and it must not widen.
func IsServiceOperatorHandle(handle string) bool {
	switch strings.TrimSpace(handle) {
	case ServiceHandleOperator, ServiceHandleIDConsole, ServiceHandleOnboardAPI:
		return true
	}
	return false
}

// reservedOperatorHandle reports whether a handle read from a key source
// collides with a service label. Case-insensitive, so "Operator" or
// "ID-Console" cannot sit next to the real one and invite a later
// normalisation to merge them.
func reservedOperatorHandle(handle string) bool {
	h := strings.TrimSpace(handle)
	for _, s := range []string{ServiceHandleOperator, ServiceHandleIDConsole, ServiceHandleOnboardAPI} {
		if strings.EqualFold(h, s) {
			return true
		}
	}
	return false
}

// envMayHoldOperatorHandle reports whether PORTAL_OPERATOR_API_KEYS may carry
// a key under this handle. Only id-console's key is provisioned there; the
// legacy shared key has its own variable and onboard-api has no key at all.
func envMayHoldOperatorHandle(handle string) bool {
	if !reservedOperatorHandle(handle) {
		return true
	}
	return strings.TrimSpace(handle) == ServiceHandleIDConsole
}
