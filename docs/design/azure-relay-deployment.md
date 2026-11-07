# Azure relay infrastructure


Status: built. Add an opt-in Azure relay module, Bastion inventory and tunnel tools, relay play branches and managed-identity DNS renewal. AWS remains the default.

The standard standup planners do not select the Azure relay generator automatically. Use the explicit inventory and Bastion commands in ops/ansible/README.md. Live Azure provisioning, RBAC, issuance, renewal and recovery remain unmeasured. No cloud resources were created.

Offline regression evidence and remaining release gates are recorded in the corresponding draft PR. No live deployment is claimed.
