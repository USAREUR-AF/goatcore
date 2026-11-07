# Onboarding transport


Status: built. Pass configured delegate SSH settings and the resolved management URL into onboarding operations using the existing --api-url interface. Keep deployment-owned goatnet_* maps from main.

The alternate operator-onboard-sites directory from PR #12 is omitted: main already supplies deployment-owned maps. Existing delegate overrides retain precedence. A real SSH/identity-provider integration test remains outstanding.

Offline regression evidence and remaining release gates are recorded in the corresponding draft PR. No live deployment is claimed.
