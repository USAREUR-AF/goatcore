# ACL declaration and trust selection


Status: built. Derive missing site taxonomy from the declared default and allow management API callers to use public trust without disabling TLS verification. Preserve the existing netbird_api_ca option with precedence over the compatibility option and environment value.

Security review must confirm intended default-site scope and Rust/Ansible agreement. No policies were applied to a live management API. A taxonomy fallback does not prove a deployment has the required matrix policies.

Offline regression evidence and remaining release gates are recorded in the corresponding draft PR. No live deployment is claimed.
