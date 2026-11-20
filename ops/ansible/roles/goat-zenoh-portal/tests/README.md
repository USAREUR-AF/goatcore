# goat-zenoh-portal role tests

Parse-only fixture. Real behavioral coverage lives in the Go test suite at
`ops/goat-zenoh-portal/` (`go test
./...` — registry validation, ACL render correctness, cert-mint
tarball assembly, end-to-end HTTP smoke through every API surface).

This role is a thin deployment wrapper. Its risk surface is the
template rendering (which `syntax-check.sh` covers) and the
docker-compose wiring (covered live at deploy time).

`render-release-sets.sh` (run by `syntax-check.sh`) renders
`templates/release-sets.yaml.j2` from `release-sets-vars.yml` and diffs it
against `ops/goat-zenoh-portal/internal/access/testdata/release-sets.role-render.yaml`,
which `TestLoadReleaseSets_RoleRender` parses with the portal's own
`LoadReleaseSets`. Change the template, then run it with `--update` and the Go
test.

`docker-socket.sh` (run by `syntax-check.sh`) runs `tasks/docker-socket.yml`
on localhost against scratch files, as the invoking user, never the host's
`/var/run/docker.sock`. It checks that the role defaults (no socket for the
portal, GHSA-6828) do not fail, that a socket an earlier deploy gave to the
portal's gid is given back to the Docker group with mode 0660 and that a
second run changes nothing, that a socket the portal did not regroup is left
alone, and (check mode) that setting `portal_acl_docker_socket` still grants it.
