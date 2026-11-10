# e2e-canary — the smallest image the deployment engineering can move.
# busybox so the container can sleep; one file so each build has its own digest.
# Built by .github/workflows/e2e-canary-image.yml (ghcr.io/usareur-af/e2e-canary)
# and, in the hermetic tier, by ops/e2e/hermetic.sh into a local registry.
FROM busybox:1.38.0-musl@sha256:80e38f9a1b39188b8bdb28879178edb322a4402b081a9aedda1924609decd6f6
ARG CANARY_STAMP=unstamped
RUN printf 'e2e-canary %s\n' "$CANARY_STAMP" > /canary.txt
CMD ["sh", "-c", "cat /canary.txt; exec sleep 2147483647"]
