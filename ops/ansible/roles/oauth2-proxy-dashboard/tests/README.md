# oauth2-proxy-dashboard role tests

Parse-only smoke. End-to-end gate validation is the operator-fired
drill on the deployed host: an account outside the allowed group gets
403, an account in it gets 200.

Run: `./syntax-check.sh`.
