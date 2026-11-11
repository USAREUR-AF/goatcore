# Management certificate provider


Status: built. Select Azure DNS explicitly for management-host ACME certificates while preserving the default Cloudflare render and credential handling. Keep the newer served-certificate verification already on main.

The relay cloud provider and management certificate provider are independent. Real certificate issuance and renewal are unmeasured. The Rust and Bash planners must continue to select the same credentials.

Offline regression evidence and remaining release gates are recorded in the corresponding draft PR. No live deployment is claimed.
