# CoreDNS controller API calls


Status: built. Disable inherited privilege escalation for controller-side management API requests. Remote host operations retain their existing privilege requirements.

The regression test inspects all localhost-delegated task definitions and requires an explicit privilege-escalation override. It does not exercise live DNS registration or a host deployment.

Offline regression evidence and remaining release gates are recorded in the corresponding draft PR. No live deployment is claimed.
