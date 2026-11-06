# goat — Test, lint, and build targets

SHELL := /bin/bash
.DEFAULT_GOAL := check

# SCRIPTS are shellchecked at FULL severity. Every other tracked shell script
# under ops/ is swept at warning severity by ops/ci/check-shellcheck-sweep.sh,
# which derives its list from git and states what it does not cover (#486).
SCRIPTS := deploy.sh provisioning/build.sh ops/snitch/internal/server/install.sh ops/scripts/worktree-reap.sh ops/work/work ops/ci/check-bash-portability.sh
YAML_FILES := provisioning/cloud-init/network-config.yaml

# ── Lint ────────────────────────────────────────────────

.PHONY: lint
lint: lint-shell lint-yaml

.PHONY: lint-shell
lint-shell:
	@echo "==> ShellCheck"
	@shellcheck -x -e SC1091,SC2016 $(SCRIPTS)
	@echo "==> ShellCheck sweep (tracked ops shell tree, warning severity)"
	@ops/ci/check-shellcheck-sweep.sh
	@echo "==> Bash portability (macOS /bin/bash 3.2 builtin sweep)"
	@ops/ci/check-bash-portability.sh
	@echo "    All scripts pass."

.PHONY: lint-yaml
lint-yaml:
	@echo "==> yamllint"
	@yamllint -c .yamllint.yml $(YAML_FILES)
	@echo "==> Validating cloud-init templates render to valid YAML"
	@NODE_ROLE=client NODE_HOSTNAME=test NETBIRD_DOMAIN=test.example.com \
		NETBIRD_SETUP_KEY=test CF_API_TOKEN=test \
		SSH_PUBLIC_KEY="ssh-ed25519 AAAA test" HASHED_PASSWORD='$$6$$test$$hash' \
		envsubst < provisioning/cloud-init/user-data.yaml.tpl | yamllint -c .yamllint.yml -
	@NODE_HOSTNAME=test \
		envsubst < provisioning/cloud-init/meta-data.yaml.tpl | yamllint -c .yamllint.yml -
	@echo "    All YAML valid."

# ── Test ────────────────────────────────────────────────

.PHONY: test
test:
	@echo "==> Running tests"
	@bats tests/
	@echo "==> Session hook guard suites (.claude/hooks/tests) — the only defence for their failure classes"
	@for t in .claude/hooks/tests/*.sh; do bash "$$t" >/dev/null 2>&1 || { echo "FAIL: $$t"; bash "$$t" 2>&1 | tail -20; exit 1; }; echo "    ok  $$t"; done
	@echo "==> Python suites (catalog and offboard subject tests require PyYAML)"
	@for t in $(PY_SUITES); do (cd "$$(dirname "$$t")" && python3 "$$(basename "$$t")" >/dev/null 2>&1) || { echo "FAIL: $$t"; (cd "$$(dirname "$$t")" && python3 "$$(basename "$$t")" 2>&1 | tail -20); exit 1; }; echo "    ok  $$t"; done

# Self-running Python suites; catalog and offboard subject tests require PyYAML.
# Excluded on purpose:
# ops/serve/tests/test_blast.py (needs pytest).
PY_SUITES := \
	ops/scripts/test_external_service_finish.py \
	ops/node-net-health/test_inventory.py \
	ops/relay/health/test_atomic_ingest.py \
	ops/issue-tracks/test_issue_tracks.py \
	ops/issue-tracks/test_pickup.py \
	ops/scripts/test_pr_readiness_checks.py \
	ops/scripts/test_relay_fleet_healthcheck.py \
	ops/scripts/test_pr_readiness_ci.py \
	ops/ci/test_pr_head_checks.py \
	ops/node-net-health/test_attribution.py \
	ops/node-net-health/test_undeclared.py \
	ops/identity/test_backfill_aff_trait.py \
	ops/ci/test_check_org_catalog.py \
	ops/ci/test_hydra_client_invariants.py \
	ops/identity/test_peerowner.py \
	ops/federation/standup/test_preflight.py \
	ops/identity/test_gen_platform_spire_entries.py \
	ops/peerview/test_peerview.py \
	ops/relay/health/test_relay_metrics_ingest.py \
	ops/scripts/test_offboard_subject.py \
	ops/scripts/test_reap_orphans.py \
	ops/scripts/test_lifecycle_sweep.py \
	ops/scripts/test_lifecycle_sweep_ledger.py \
	ops/scripts/test_sync_slot_anchors.py \
	ops/peering-denials/test_pd_snapshot.py \
	ops/zenoh/wedge-repro/test_openprobe.py \
	ops/zenoh-trial-peers/budget-notify/test_budget_notify.py

# ── Combined ────────────────────────────────────────────

.PHONY: check
check: lint test
	@echo ""
	@echo "All checks passed."

# ── Build (convenience wrappers) ────────────────────────

.PHONY: build-infra
build-infra:
	@provisioning/build.sh --role infrastructure --platform x86_64 --hostname nethost-01

.PHONY: build-client
build-client:
	@test -n "$(HOSTNAME)" || { echo "Usage: make build-client HOSTNAME=sensor-01 [SETUP_KEY=nbsk_...]"; exit 1; }
	@provisioning/build.sh --role client --platform arm64 --hostname $(HOSTNAME) \
		$(if $(SETUP_KEY),--setup-key $(SETUP_KEY),)

# ── Clean ───────────────────────────────────────────────

.PHONY: clean
clean:
	rm -rf provisioning/.build/

# ── Multi-agent substrate (Block 48O + ADR 0017) ────────
# Per-clone install of the session-isolation pilot pieces. Each piece
# is independently revertable (see ADR 0017 §"Rollback procedure").

.PHONY: multi-agent-substrate-install
multi-agent-substrate-install: multi-agent-substrate-hooks-install \
                                multi-agent-substrate-skill-install \
                                multi-agent-substrate-memory-install \
                                multi-agent-substrate-bus-init
	@echo ""
	@echo "==> Multi-agent substrate installation complete."
	@echo "    Hooks: .git/hooks/{post-checkout,pre-commit}"
	@echo "    Skill: ~/.claude/skills/iso/SKILL.md"
	@echo "    Memory: ~/.claude/projects/<this-repo>/memory/feedback_master_worktree_readonly.md"
	@echo "    Session bus: ~/.claude/state/session-bus-<repo-fingerprint>.jsonl"
	@echo ""
	@echo "    Optional: symlink launcher to PATH —"
	@echo "      ln -s $$(pwd)/ops/scripts/claude-session ~/bin/claude-session"

.PHONY: multi-agent-substrate-hooks-install
multi-agent-substrate-hooks-install:
	@echo "==> Installing master-worktree hooks (post-checkout + pre-commit)"
	@./ops/git-hooks/install.sh

.PHONY: multi-agent-substrate-skill-install
multi-agent-substrate-skill-install:
	@echo "==> Installing /iso slash command skill at ~/.claude/skills/iso/SKILL.md"
	@mkdir -p ~/.claude/skills/iso
	@ln -sfn "$$(pwd)/ops/scripts/iso.skill.md" ~/.claude/skills/iso/SKILL.md
	@echo "    /iso skill installed (verify: type /iso status next session)"

.PHONY: multi-agent-substrate-memory-install
multi-agent-substrate-memory-install:
	@echo "==> Installing memory hooks from ops/scripts/feedback-memory-hooks/"
	@MEMORY_DIR=$$(find ~/.claude/projects -maxdepth 1 -type d -name "*$$(basename $$(git rev-parse --show-toplevel))*" | head -1); \
	if [ -z "$$MEMORY_DIR" ]; then \
		echo "    WARN: per-repo memory dir not found at ~/.claude/projects/."; \
		echo "    The dir is created by Claude Code on first session in this repo."; \
		echo "    Run this target again after starting one Claude Code session."; \
	else \
		mkdir -p "$$MEMORY_DIR/memory"; \
		touch "$$MEMORY_DIR/memory/MEMORY.md"; \
		installed=0; \
		for hook_path in ops/scripts/feedback-memory-hooks/feedback_*.md; do \
			hook=$$(basename "$$hook_path"); \
			cp "$$hook_path" "$$MEMORY_DIR/memory/"; \
			installed=$$((installed + 1)); \
			grep -qF "($$hook)" "$$MEMORY_DIR/memory/MEMORY.md" && continue; \
			summary=$$(awk '/^description:[[:space:]]/ {sub(/^description:[[:space:]]+/,""); print; exit}' "$$hook_path" | sed 's/\.[[:space:]].*//'); \
			[ -z "$$summary" ] && summary="(no description in frontmatter)"; \
			printf -- '- [%s](%s) — %s\n' "$$hook" "$$hook" "$$summary" >> "$$MEMORY_DIR/memory/MEMORY.md"; \
		done; \
		echo "    $$installed hooks installed at $$MEMORY_DIR/memory/"; \
	fi

.PHONY: multi-agent-substrate-bus-init
multi-agent-substrate-bus-init:
	@echo "==> Initializing session-bus state directory"
	@mkdir -p ~/.claude/state
	@REPO_FP=$$(git rev-parse --show-toplevel | shasum | cut -c1-12); \
	BUS_FILE=~/.claude/state/session-bus-$$REPO_FP.jsonl; \
	touch "$$BUS_FILE"; \
	echo "    Session bus at $$BUS_FILE (per-repo via path fingerprint)"
	@echo "    Tail with: ops/work/work bus tail"

# ── PR readiness gate (Block 48Q) ───────────────────────
# Pre-push hard gate. Operates on the CURRENT branch (no args). Hard-
# fails on stale base, orphan-resurrection, runaway diff, dirty working
# tree, or missing DCO sign-off. Run from any feature-branch worktree
# before `git push` / `gh pr create`.

.PHONY: pr-readiness
pr-readiness:
	@./ops/scripts/pr-readiness.sh

.PHONY: pr-readiness-test
pr-readiness-test:
	@./ops/scripts/pr-readiness_test.sh

.PHONY: multi-agent-substrate-uninstall
multi-agent-substrate-uninstall:
	@echo "==> Removing hooks (.git/hooks/post-checkout + pre-commit)"
	@rm -f .git/hooks/post-checkout .git/hooks/pre-commit
	@echo "==> Removing skill (~/.claude/skills/iso/)"
	@if [ -L ~/.claude/skills/iso/SKILL.md ] && [ "$$(readlink ~/.claude/skills/iso/SKILL.md)" = "$$(pwd)/ops/scripts/iso.skill.md" ]; then \
		rm ~/.claude/skills/iso/SKILL.md; rmdir ~/.claude/skills/iso 2>/dev/null || true; \
	fi
	@echo "==> Removing memory hook"
	@MEMORY_DIR=$$(find ~/.claude/projects -maxdepth 1 -type d -name "*$$(basename $$(git rev-parse --show-toplevel))*" | head -1); \
	[ -n "$$MEMORY_DIR" ] && rm -f "$$MEMORY_DIR/memory/feedback_master_worktree_readonly.md" || true
	@echo "    Memory hook MEMORY.md row left in place (manual edit recommended if removing)"
	@echo ""
	@echo "==> Substrate uninstalled. Per-clone session-bus.jsonl preserved at ~/.claude/state/."
	@echo "    Per-worktree .iso-mode.env files left in place (use 'git worktree remove' for full cleanup)."

.PHONY: review
review: ## the daily review-and-act: what is soaking, ready, or owed on the ladder (ADR 1177)
	@ops/scripts/daily-review.sh --ack "$${GOAT_ACK:-$$(git config user.name)}"
