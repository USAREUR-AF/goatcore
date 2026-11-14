# Credential classification


Status: built. Exclude Azure DNS credential identifiers from Hydra client classification so validation does not treat a cloud service principal as an identity-provider client.

Coverage uses synthetic identifiers. Cloud credential validity and Hydra reachability are outside this change.

Offline regression evidence and remaining release gates are recorded in the corresponding draft PR. No live deployment is claimed.
