// `goat admin storm <subcmd>` — control-plane storm resilience operator surface.
//
// ADR 1081 Layer 4: the MANUAL storm brake. When the control plane storms
// (signaling futility ratio ≥0.5 at ≥100 msgs/min on the relay-health host-net
// band), the operator fires ONE audited command that runs four steps in order:
//
//   FLUSH    — restart the signal container on the mgmt host (drops zombie
//              streams so registration state converges). M-085 proves this is
//              insufficient ALONE — layers 1-2 are the cure — which is why the
//              brake is layer 4, not layer 1.
//   TIGHTEN  — clamp the L2 signal limiter for a bounded, operator-reverted
//              window IF an L2-capable signal build is running (probed via the
//              server's /metrics capability counter, per the L2 owner). The two
//              knobs tighten in OPPOSITE directions: SIGNAL_SENDER_RATE_LIMIT is
//              halved (lower = stricter), SIGNAL_NEGATIVE_CACHE_TTL is doubled
//              (higher = more shedding); either enables from off at the L2
//              baseline (30 / 20). A graceful no-op (Skipped, clear message)
//              when the metric is absent (pre-L2 image / signal down).
//   SHED     — pause declared non-essential publisher systemd units (the list
//              comes from `storm_brake_shed_units` in the context profile.toml
//              and/or repeated `--shed-unit` flags).
//   ANNOUNCE — emit one OperatorActionV1 through the audited `goat admin`
//              chokepoint (crate::cmd::audit) carrying the per-step outcomes.
//
// MANUAL PHASE ONLY (ADR 1081): there is NO automation, NO timer, NO
// auto-engage here. The bounded TIGHTEN window is operator-reverted (the
// remote command prints the revert line); auto-mode with hysteresis + a
// once-per-hour cap is a separately-gated follow-on that does not exist yet.
//
// Remote execution follows the strongest existing goat-cli precedent — the
// native-onboard Kratos leg's `run_ssh_curl` / `ssh_kratos_argv` (cmd::admin):
// shell out to `ssh <BatchMode opts> <user>@<host> <remote_cmd>`, capture the
// child's exit code + stdout/stderr. The mgmt host + user come from the goat
// context (flag > env > profile.toml). The executor is behind a trait so plan
// construction and outcome aggregation unit-test against a mock.

use clap::Subcommand;
use i18n_embed_fl::fl;
use std::process::Stdio;
use tokio::process::Command;

use crate::i18n::LANGUAGE_LOADER;

/// Default signal container / compose service name on the mgmt host (ADR 1081
/// / ops/compose/mgmt-host/docker-compose.yml — the container_name and the
/// compose service key coincide here).
const DEFAULT_SIGNAL_CONTAINER: &str = "signal-a";
/// Default mgmt-host compose directory (ops/ansible/mgmt-host.yml `compose_dir`).
const DEFAULT_COMPOSE_DIR: &str = "/opt/mgmt-host";
/// Default bounded TIGHTEN window, minutes (advisory — operator-reverted, no timer).
const DEFAULT_TIGHTEN_WINDOW_MIN: u32 = 30;
/// Remote-command exit code the TIGHTEN step uses to signal "flags not
/// deployed — skip cleanly" (distinct from 0 = applied and other = failed).
const STORM_SKIP_EXIT: i32 = 42;

#[derive(Subcommand, Debug)]
pub enum StormCmd {
    /// Fire the manual control-plane storm brake (ADR 1081 Layer 4): flush the
    /// signal container, tighten L2 limits (if deployed), shed declared
    /// non-essential publishers, and record one audited operator action.
    ///
    /// Fire it when the relay-health host-net band shows a signaling futility
    /// ratio ≥0.5 at ≥100 msgs/min (and/or uplink loss ≥5% sustained). The
    /// goatnet is the active context (`--context` / `$GOAT_CONTEXT`). Preview
    /// with `--dry-run`; skip the confirmation with `--yes`.
    ///
    /// NOT a cure on its own — M-085: a signal flush alone does not restore the
    /// mesh. Layers 1-2 (client backoff + signal shedding) are the fix; the
    /// brake buys recovery time. See docs/operations/storm-brake-runbook.md.
    Brake {
        /// Print the plan + trigger-signal guidance and exit WITHOUT executing
        /// anything. Safe to run any time to learn the sequence.
        #[arg(long)]
        dry_run: bool,
        /// Skip the interactive confirmation prompt (for scripted/non-interactive
        /// use). The plan is still printed first.
        #[arg(long)]
        yes: bool,
        /// Signal container / compose service name on the mgmt host.
        #[arg(long, default_value = DEFAULT_SIGNAL_CONTAINER)]
        signal_container: String,
        /// A systemd unit to pause during SHED (repeatable). Augments the
        /// `storm_brake_shed_units` list in the context profile.toml.
        #[arg(long = "shed-unit")]
        shed_unit: Vec<String>,
        /// Override the mgmt-host SSH host. Precedence: this flag >
        /// `$GOAT_MGMT_SSH_HOST` > `$<CONTEXT>_MGMT_SSH_HOST` > `mgmt_ssh_host`
        /// in the context profile.toml.
        #[arg(long)]
        mgmt_host: Option<String>,
        /// Override the mgmt-host SSH user. Precedence: this flag >
        /// `$GOAT_MGMT_SSH_USER` > `mgmt_ssh_user` in profile.toml (goatnet.toml).
        #[arg(long)]
        mgmt_user: Option<String>,
        /// mgmt-host compose directory (holds docker-compose.yml + .env) the
        /// TIGHTEN recreate runs from.
        #[arg(long, default_value = DEFAULT_COMPOSE_DIR)]
        compose_dir: String,
        /// Advisory bounded-window length (minutes) shown for the TIGHTEN step.
        /// Manual phase has no timer — the operator reverts (the step prints how).
        #[arg(long, default_value_t = DEFAULT_TIGHTEN_WINDOW_MIN)]
        tighten_window_min: u32,
    },
}

pub async fn run(ctx: &super::Ctx, op: StormCmd) -> anyhow::Result<()> {
    match op {
        StormCmd::Brake {
            dry_run,
            yes,
            signal_container,
            shed_unit,
            mgmt_host,
            mgmt_user,
            compose_dir,
            tighten_window_min,
        } => {
            run_brake(
                ctx,
                BrakeArgs {
                    dry_run,
                    yes,
                    signal_container,
                    shed_unit,
                    mgmt_host,
                    mgmt_user,
                    compose_dir,
                    tighten_window_min,
                },
            )
            .await
        }
    }
}

struct BrakeArgs {
    dry_run: bool,
    yes: bool,
    signal_container: String,
    shed_unit: Vec<String>,
    mgmt_host: Option<String>,
    mgmt_user: Option<String>,
    compose_dir: String,
    tighten_window_min: u32,
}

// ---------------------------------------------------------------------------
// Plan model (pure — unit-tested)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Flush,
    Tighten,
    Shed,
}

impl StepKind {
    /// Fixed, locale-invariant label (an identifier, not prose — same rule as
    /// the machine surface).
    fn label(self) -> &'static str {
        match self {
            StepKind::Flush => "FLUSH",
            StepKind::Tighten => "TIGHTEN",
            StepKind::Shed => "SHED",
        }
    }
}

/// One ordered brake step: what it is, what it targets, and the exact shell
/// command run on the mgmt host. Building the `remote_cmd` here (not at execute
/// time) keeps the whole plan inspectable by `--dry-run` and testable without a
/// host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrakeStep {
    pub kind: StepKind,
    pub target: String,
    pub remote_cmd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrakePlan {
    pub host: String,
    pub user: String,
    pub steps: Vec<BrakeStep>,
}

/// Resolved SSH target for the mgmt host.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MgmtTarget {
    host: String,
    user: String,
}

/// Validate a docker container / compose service / systemd unit name before it
/// is interpolated into a remote shell command. Rejects anything outside a
/// conservative safe set so a crafted name can't inject shell metacharacters
/// over ssh. (`[A-Za-z0-9._@-]`, 1..=200 chars — covers `signal-a`,
/// `foo.service`, `docker.socket`.)
fn is_safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'@'))
}

/// Build the ordered brake plan. Pure: no host contact, no env reads — every
/// input is passed in, so plan construction is fully unit-testable. Validates
/// each interpolated token and errors (rather than silently dropping) on a
/// bad one.
fn build_plan(
    host: &str,
    user: &str,
    signal_container: &str,
    shed_units: &[String],
    compose_dir: &str,
) -> anyhow::Result<BrakePlan> {
    if !is_safe_token(signal_container) {
        anyhow::bail!("unsafe signal container name `{signal_container}`");
    }
    // The compose dir is a path, not a token; allow `/`, reject shell-breakers.
    if compose_dir.is_empty() || compose_dir.bytes().any(|c| matches!(c, b'\'' | b'"' | b'`' | b'$' | b';' | b'\n')) {
        anyhow::bail!("unsafe compose dir `{compose_dir}`");
    }

    let mut steps = Vec::new();
    steps.push(BrakeStep {
        kind: StepKind::Flush,
        target: signal_container.to_string(),
        remote_cmd: flush_remote_cmd(signal_container),
    });
    steps.push(BrakeStep {
        kind: StepKind::Tighten,
        target: signal_container.to_string(),
        remote_cmd: tighten_remote_cmd(signal_container, compose_dir),
    });
    for unit in shed_units {
        if !is_safe_token(unit) {
            anyhow::bail!("unsafe shed unit name `{unit}`");
        }
        steps.push(BrakeStep {
            kind: StepKind::Shed,
            target: unit.clone(),
            remote_cmd: shed_remote_cmd(unit),
        });
    }

    Ok(BrakePlan {
        host: host.to_string(),
        user: user.to_string(),
        steps,
    })
}

/// FLUSH: restart the signal container. `docker restart` drops the zombie
/// signaling streams so registration state re-converges.
fn flush_remote_cmd(container: &str) -> String {
    format!("docker restart {container}")
}

/// SHED: pause a non-essential publisher unit. `systemctl stop` is the pause
/// (systemd has no suspend); the operator restarts it when the storm clears
/// (documented in the runbook — no timer). Day-1 mgmt hosts grant NOPASSWD
/// sudo, so `sudo` is safe non-interactively.
fn shed_remote_cmd(unit: &str) -> String {
    format!("sudo systemctl stop {unit}")
}

/// TIGHTEN: clamp the L2 signal limiter for a bounded window, IF an L2-capable
/// signal build is actually RUNNING. The remote command:
///   1. probes the signal server's own `/metrics` for
///      `signal_server_send_rate_limited_total` — a counter the L2 build
///      registers unconditionally at startup, so it exists only when an
///      L2-capable binary is live. This is the true capability signal (per the
///      L2 owner): env-var presence does NOT discriminate, because once the L2
///      compose block merges the vars are always set (defaulting to 0) even on
///      a host still running a pre-L2 image. The probe retries briefly so it
///      tolerates the container FLUSH just restarted;
///   2. if the metric is absent → pre-L2 image (or signal down) → prints a skip
///      line and exits `STORM_SKIP_EXIT` for a clean Skipped — the common,
///      today-true path;
///   3. if present → reads the current SIGNAL_SENDER_RATE_LIMIT /
///      SIGNAL_NEGATIVE_CACHE_TTL and tightens each knob in its OWN direction
///      (per the L2 owner): rate-limit halved (lower = stricter), neg-cache TTL
///      doubled (higher = more shedding); a knob that is off (0) is ENABLED at
///      the L2 baseline (rate 30 / ttl 20). It writes the new values to
///      `<compose_dir>/storm-brake.env` and recreates the service with that
///      override layered over `.env` (`docker compose --env-file .env
///      --env-file storm-brake.env up -d`), then prints the revert line.
///      Container env is fixed at create time, so a recreate is the only way to
///      change it (L2 confirms no hot reload).
///
/// `METRICS_PORT` (9090) is signal-a's published metrics port
/// (ops/compose/mgmt-host/docker-compose.yml `--metrics-addr=:9090`, bound
/// 0.0.0.0, host-published), reachable from the mgmt host at 127.0.0.1.
fn tighten_remote_cmd(container: &str, compose_dir: &str) -> String {
    // Rate-limit baseline (lower=stricter) and neg-cache-TTL baseline
    // (higher=more shedding) used to ENABLE a knob that is currently off — the
    // L2 owner's recommended values.
    const RATE_BASELINE: u32 = 30;
    const TTL_BASELINE: u32 = 20;
    // signal-a's published Prometheus metrics port.
    const METRICS_PORT: u32 = 9090;
    // Single-quote the compose dir for the remote shell; the path is validated
    // free of quotes/metachars in build_plan.
    format!(
        "set -e; c={container}; \
cap=''; i=0; while [ $i -lt 5 ]; do \
if curl -fsS --max-time 3 http://127.0.0.1:{metrics_port}/metrics 2>/dev/null | grep -q '^signal_server_send_rate_limited_total'; then cap=1; break; fi; \
i=$((i+1)); sleep 2; done; \
if [ -z \"$cap\" ]; then \
echo \"signal_server_send_rate_limited_total not exported on $c:{metrics_port} — L2 signal-shedding build not running\"; exit {skip}; fi; \
env=$(docker inspect --format '{{{{range .Config.Env}}}}{{{{println .}}}}{{{{end}}}}' \"$c\" 2>/dev/null || true); \
rl=$(printf '%s\\n' \"$env\" | sed -n 's/^SIGNAL_SENDER_RATE_LIMIT=//p' | head -n1); \
tt=$(printf '%s\\n' \"$env\" | sed -n 's/^SIGNAL_NEGATIVE_CACHE_TTL=//p' | head -n1); \
isnum() {{ case \"$1\" in ''|*[!0-9]*) return 1;; *) return 0;; esac; }}; \
if isnum \"$rl\" && [ \"$rl\" -gt 0 ]; then nrl=$((rl/2)); [ \"$nrl\" -lt 1 ] && nrl=1; else nrl={rate_base}; fi; \
if isnum \"$tt\" && [ \"$tt\" -gt 0 ]; then ntt=$((tt*2)); else ntt={ttl_base}; fi; \
cd '{dir}'; \
{{ echo \"SIGNAL_SENDER_RATE_LIMIT=$nrl\"; echo \"SIGNAL_NEGATIVE_CACHE_TTL=$ntt\"; }} > storm-brake.env; \
docker compose --env-file .env --env-file storm-brake.env up -d \"$c\"; \
echo \"tightened rate-limit=$rl->$nrl neg-cache-ttl=$tt->$ntt (revert: rm '{dir}'/storm-brake.env && cd '{dir}' && docker compose up -d $c)\"",
        container = container,
        skip = STORM_SKIP_EXIT,
        rate_base = RATE_BASELINE,
        ttl_base = TTL_BASELINE,
        metrics_port = METRICS_PORT,
        dir = compose_dir,
    )
}

/// Merge the profile's `storm_brake_shed_units` (a comma-separated string —
/// the hand-rolled profile.toml reader has no list type) with any repeated
/// `--shed-unit` flags. Order-preserving, de-duplicated, blanks dropped.
fn merge_shed_units(profile_csv: Option<&str>, flag_units: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |raw: &str| {
        let u = raw.trim();
        if !u.is_empty() && !out.iter().any(|e| e == u) {
            out.push(u.to_string());
        }
    };
    if let Some(csv) = profile_csv {
        for part in csv.split(',') {
            push(part);
        }
    }
    for u in flag_units {
        push(u);
    }
    out
}

/// Pick the mgmt SSH host+user from the layered sources (flag > env > profile,
/// user defaulting to `goat`). Pure: the caller resolves each source to
/// an `Option<String>` first, so precedence is unit-testable without env/fs. An
/// unresolved host is an error naming every way to set it.
fn pick_mgmt_target(
    host_flag: Option<String>,
    host_env: Option<String>,
    host_ctx_env: Option<String>,
    host_profile: Option<String>,
    user_flag: Option<String>,
    user_env: Option<String>,
    user_profile: Option<String>,
) -> anyhow::Result<MgmtTarget> {
    let clean = |o: Option<String>| o.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let host = clean(host_flag)
        .or_else(|| clean(host_env))
        .or_else(|| clean(host_ctx_env))
        .or_else(|| clean(host_profile))
        .ok_or_else(|| {
            anyhow::anyhow!(fl!(
                LANGUAGE_LOADER,
                "storm-brake-mgmt-unresolved"
            ))
        })?;
    let user = clean(user_flag)
        .or_else(|| clean(user_env))
        .or_else(|| clean(user_profile))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no SSH user for the management host: pass --mgmt-user, set GOAT_MGMT_SSH_USER, \
                 or run `goat context refresh` (it writes mgmt_ssh_user)"
            )
        })?;
    if !is_safe_token(&host) {
        anyhow::bail!("unsafe mgmt host `{host}`");
    }
    if !is_safe_token(&user) {
        anyhow::bail!("unsafe mgmt user `{user}`");
    }
    Ok(MgmtTarget { host, user })
}

// ---------------------------------------------------------------------------
// Executor (trait so plan execution is testable against a mock)
// ---------------------------------------------------------------------------

/// Result of running one remote command: the child's exit code plus captured
/// output. `code` is the ssh child's exit status, which for a completed
/// connection IS the remote command's exit code.
#[derive(Debug, Clone)]
struct RemoteOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs a single shell command on the mgmt host. Private trait, driven through
/// generics (never `dyn`), so a mock can stand in for the real ssh executor in
/// unit tests without dyn-compatibility or an async_trait dep.
trait BrakeExecutor {
    async fn run(&self, remote_cmd: &str) -> anyhow::Result<RemoteOutput>;
}

/// The real executor: `ssh <BatchMode opts> user@host <remote_cmd>`. Mirrors
/// cmd::admin's `ssh_kratos_argv` hardening (BatchMode so it never blocks on a
/// password prompt, bounded connect + keepalive).
struct SshExecutor {
    host: String,
    user: String,
}

/// Build the ssh argv for a mgmt-host remote command. Pure, for unit testing —
/// same shape as cmd::admin::ssh_kratos_argv.
fn ssh_argv(user: &str, host: &str, remote_cmd: &str) -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
        "-o".into(),
        "ServerAliveInterval=10".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        format!("{user}@{host}"),
        remote_cmd.to_string(),
    ]
}

impl BrakeExecutor for SshExecutor {
    async fn run(&self, remote_cmd: &str) -> anyhow::Result<RemoteOutput> {
        let args = ssh_argv(&self.user, &self.host, remote_cmd);
        let out = Command::new("ssh")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| anyhow::anyhow!("spawn ssh {}@{}: {e}", self.user, self.host))?;
        Ok(RemoteOutput {
            // ssh returns 255 on its own transport failure; a killed-by-signal
            // child has no code — map that to -1 so it classifies as Failed.
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Outcome model + aggregation (pure — unit-tested)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepStatus {
    Ok,
    Failed(String),
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepOutcome {
    pub kind: StepKind,
    pub target: String,
    pub status: StepStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrakeReport {
    pub outcomes: Vec<StepOutcome>,
}

impl BrakeReport {
    fn counts(&self) -> (usize, usize, usize) {
        let mut ok = 0;
        let mut failed = 0;
        let mut skipped = 0;
        for o in &self.outcomes {
            match o.status {
                StepStatus::Ok => ok += 1,
                StepStatus::Failed(_) => failed += 1,
                StepStatus::Skipped(_) => skipped += 1,
            }
        }
        (ok, failed, skipped)
    }

    /// True when the FLUSH step failed — the load-bearing step, so it carries
    /// its own localized message and mesh recovery is not expected.
    /// `non_flush_failures` covers the other steps.
    fn flush_failed(&self) -> bool {
        self.outcomes.iter().any(|o| {
            o.kind == StepKind::Flush && matches!(o.status, StepStatus::Failed(_))
        })
    }

    /// Every non-FLUSH step that FAILED (not Skipped — a Skipped step is an
    /// expected, non-fatal outcome such as "L2 not deployed"), rendered
    /// `KIND[target]=reason`. Non-empty means the brake is partial and the
    /// command exits nonzero even though flush itself succeeded.
    fn non_flush_failures(&self) -> Vec<String> {
        self.outcomes
            .iter()
            .filter(|o| o.kind != StepKind::Flush)
            .filter_map(|o| match &o.status {
                StepStatus::Failed(r) => Some(format!("{}[{}]={r}", o.kind.label(), o.target)),
                _ => None,
            })
            .collect()
    }

    /// The audit outcome word: `failed` if flush failed, `partial` if any other
    /// step failed/skipped, else `ok`.
    fn audit_outcome(&self) -> &'static str {
        if self.flush_failed() {
            "failed"
        } else if self
            .outcomes
            .iter()
            .any(|o| !matches!(o.status, StepStatus::Ok))
        {
            "partial"
        } else {
            "ok"
        }
    }
}

/// Classify one remote result into a step status. `STORM_SKIP_EXIT` → Skipped
/// (reason = stdout); 0 → Ok; anything else → Failed (reason = stderr, or
/// stdout when stderr is empty). Pure so the mapping is unit-tested directly.
fn classify(out: &RemoteOutput) -> StepStatus {
    match out.code {
        0 => StepStatus::Ok,
        STORM_SKIP_EXIT => StepStatus::Skipped(first_line(&out.stdout)),
        _ => {
            let reason = if out.stderr.trim().is_empty() {
                first_line(&out.stdout)
            } else {
                first_line(&out.stderr)
            };
            StepStatus::Failed(if reason.is_empty() {
                format!("remote command exited {}", out.code)
            } else {
                reason
            })
        }
    }
}

fn first_line(s: &str) -> String {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()
}

/// Run every step in order through the executor, printing each outcome as it
/// lands, and return the aggregated report. Generic over the executor so tests
/// drive it with a mock. An executor error (e.g. ssh spawn failure) becomes a
/// Failed outcome for that step — the brake keeps going so a later shed still
/// runs even if tighten's transport hiccupped.
async fn execute_plan<E: BrakeExecutor>(exec: &E, plan: &BrakePlan) -> BrakeReport {
    let mut outcomes = Vec::with_capacity(plan.steps.len());
    for step in &plan.steps {
        let status = match exec.run(&step.remote_cmd).await {
            Ok(out) => classify(&out),
            Err(e) => StepStatus::Failed(e.to_string()),
        };
        print_step_result(step.kind, &step.target, &status);
        outcomes.push(StepOutcome {
            kind: step.kind,
            target: step.target.clone(),
            status,
        });
    }
    BrakeReport { outcomes }
}

// ---------------------------------------------------------------------------
// Command flow
// ---------------------------------------------------------------------------

async fn run_brake(ctx: &super::Ctx, args: BrakeArgs) -> anyhow::Result<()> {
    let ctx_env = ctx.profile.context.to_uppercase().replace('-', "_");
    let target = pick_mgmt_target(
        args.mgmt_host.clone(),
        std::env::var("GOAT_MGMT_SSH_HOST").ok(),
        std::env::var(format!("{ctx_env}_MGMT_SSH_HOST")).ok(),
        ctx.profile.profile_field("mgmt_ssh_host"),
        args.mgmt_user.clone(),
        std::env::var("GOAT_MGMT_SSH_USER").ok(),
        ctx.profile.profile_field("mgmt_ssh_user"),
    )?;

    let shed_units = merge_shed_units(
        ctx.profile.profile_field("storm_brake_shed_units").as_deref(),
        &args.shed_unit,
    );

    let plan = build_plan(
        &target.host,
        &target.user,
        &args.signal_container,
        &shed_units,
        &args.compose_dir,
    )?;

    print_plan(ctx, &plan, args.tighten_window_min);

    if args.dry_run {
        print_trigger_guidance(ctx);
        println!("{}", fl!(LANGUAGE_LOADER, "storm-brake-dryrun-note"));
        return Ok(());
    }

    if !args.yes {
        use std::io::Write as _;
        eprint!(
            "{} ",
            fl!(
                LANGUAGE_LOADER,
                "storm-brake-confirm",
                user = plan.user.clone(),
                host = plan.host.clone()
            )
        );
        std::io::stderr().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            anyhow::bail!("{}", fl!(LANGUAGE_LOADER, "storm-brake-aborted"));
        }
    }

    println!(
        "{}",
        fl!(
            LANGUAGE_LOADER,
            "storm-brake-executing",
            user = plan.user.clone(),
            host = plan.host.clone()
        )
    );

    let exec = SshExecutor {
        host: target.host.clone(),
        user: target.user.clone(),
    };
    let report = execute_plan(&exec, &plan).await;

    // ANNOUNCE — one OperatorActionV1 through the audited goat-admin chokepoint
    // (best-effort: an audit emit failure never changes the brake's own exit).
    announce(ctx, &args.signal_container, &report).await;

    let (ok, failed, skipped) = report.counts();
    println!(
        "{}",
        fl!(
            LANGUAGE_LOADER,
            "storm-brake-summary",
            ok = ok.to_string(),
            failed = failed.to_string(),
            skipped = skipped.to_string()
        )
    );
    println!("{}", fl!(LANGUAGE_LOADER, "storm-brake-not-cure"));

    // Exit nonzero when FLUSH failed (fatal — mesh recovery is not expected)
    // OR when any other step genuinely FAILED (not Skipped — a Skipped step
    // is an expected, non-fatal outcome, e.g. "L2 not deployed" on a target
    // it doesn't apply to). Before this, a failed TIGHTEN or SHED was
    // printed as `[FAILED]` in the per-step lines and counted in the
    // summary, but the command still returned Ok(()) → exit 0 — a script
    // driving this during an active storm could not tell a partial brake
    // from a clean one via the exit code alone.
    if report.flush_failed() {
        let reason = report
            .outcomes
            .iter()
            .find(|o| o.kind == StepKind::Flush)
            .and_then(|o| match &o.status {
                StepStatus::Failed(r) => Some(r.clone()),
                _ => None,
            })
            .unwrap_or_default();
        anyhow::bail!(
            "{}",
            fl!(LANGUAGE_LOADER, "storm-brake-flush-failed", reason = reason)
        );
    }
    let other_failures = report.non_flush_failures();
    if !other_failures.is_empty() {
        anyhow::bail!(
            "brake completed with {} non-flush step failure(s): {} — the brake is PARTIAL, \
             not clean; investigate the failed target(s) directly.",
            other_failures.len(),
            other_failures.join(", ")
        );
    }
    Ok(())
}

/// Emit the ANNOUNCE OperatorActionV1 with the per-step outcome summary in the
/// (redacted) args digest. Mirrors the vendor/onboard audit call sites in
/// cmd::admin.
async fn announce(ctx: &super::Ctx, container: &str, report: &BrakeReport) {
    let actor = crate::cmd::audit::actor_handle(&ctx.profile);
    let (ok, failed, skipped) = report.counts();
    let digest = format!(
        "storm.brake container={container} steps_ok={ok} steps_failed={failed} steps_skipped={skipped}"
    );
    let action = crate::cmd::audit::OperatorActionV1::new(
        actor.as_str(),
        "goat-cli",
        "admin.storm.brake",
        container,
        ctx.profile.context.as_str(),
        report.audit_outcome(),
    )
    .with_args_digest(&digest);
    crate::cmd::audit::emit(&ctx.profile, action).await;
    println!(
        "{}",
        fl!(LANGUAGE_LOADER, "storm-brake-audit-note", actor = actor)
    );
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn print_plan(ctx: &super::Ctx, plan: &BrakePlan, tighten_window_min: u32) {
    println!("{}", fl!(LANGUAGE_LOADER, "storm-brake-plan-header"));
    println!(
        "  {}",
        fl!(
            LANGUAGE_LOADER,
            "storm-brake-plan-target",
            user = plan.user.clone(),
            host = plan.host.clone(),
            context = ctx.profile.context.clone()
        )
    );
    let mut shed_seen = false;
    for step in &plan.steps {
        let line = match step.kind {
            StepKind::Flush => fl!(
                LANGUAGE_LOADER,
                "storm-brake-step-flush",
                container = step.target.clone()
            ),
            StepKind::Tighten => fl!(
                LANGUAGE_LOADER,
                "storm-brake-step-tighten",
                container = step.target.clone(),
                window = tighten_window_min.to_string()
            ),
            StepKind::Shed => {
                shed_seen = true;
                fl!(
                    LANGUAGE_LOADER,
                    "storm-brake-step-shed",
                    unit = step.target.clone()
                )
            }
        };
        println!("  {} {}", step.kind.label(), line);
    }
    if !shed_seen {
        println!(
            "  {} {}",
            StepKind::Shed.label(),
            fl!(LANGUAGE_LOADER, "storm-brake-shed-none")
        );
    }
    println!(
        "  ANNOUNCE {}",
        fl!(LANGUAGE_LOADER, "storm-brake-announce-note")
    );
}

fn print_step_result(kind: StepKind, target: &str, status: &StepStatus) {
    let (word, detail) = match status {
        StepStatus::Ok => (fl!(LANGUAGE_LOADER, "storm-brake-outcome-ok"), String::new()),
        StepStatus::Failed(r) => (
            fl!(LANGUAGE_LOADER, "storm-brake-outcome-failed"),
            format!(" — {r}"),
        ),
        StepStatus::Skipped(r) => (
            fl!(LANGUAGE_LOADER, "storm-brake-outcome-skipped"),
            format!(" — {r}"),
        ),
    };
    println!("  [{word}] {} {target}{detail}", kind.label());
}

fn print_trigger_guidance(ctx: &super::Ctx) {
    println!("{}", fl!(LANGUAGE_LOADER, "storm-brake-triggers-header"));
    println!("  {}", fl!(LANGUAGE_LOADER, "storm-brake-trigger-futility"));
    println!("  {}", fl!(LANGUAGE_LOADER, "storm-brake-trigger-loss"));
    // Show the host-band URL PATTERN (not a fabricated resolved URL) plus the
    // site hint from the active context.
    let site = ctx
        .profile
        .derive_site()
        .unwrap_or_else(|| "<site>".to_string());
    println!(
        "  {}",
        fl!(
            LANGUAGE_LOADER,
            "storm-brake-triggers-band-url",
            site = site
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- token safety --------------------------------------------------------

    #[test]
    fn safe_tokens_accept_real_names_reject_injection() {
        assert!(is_safe_token("signal-a"));
        assert!(is_safe_token("goat-publisher.service"));
        assert!(is_safe_token("docker.socket"));
        assert!(!is_safe_token(""));
        assert!(!is_safe_token("signal-a; rm -rf /"));
        assert!(!is_safe_token("a b"));
        assert!(!is_safe_token("$(whoami)"));
        assert!(!is_safe_token("a`b`"));
    }

    // -- plan construction ---------------------------------------------------

    #[test]
    fn plan_is_flush_tighten_then_one_shed_per_unit() {
        let plan = build_plan(
            "100.127.0.1",
            "goat",
            "signal-a",
            &["pub-a.service".to_string(), "pub-b.service".to_string()],
            "/opt/mgmt-host",
        )
        .unwrap();
        let kinds: Vec<StepKind> = plan.steps.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                StepKind::Flush,
                StepKind::Tighten,
                StepKind::Shed,
                StepKind::Shed
            ]
        );
        assert_eq!(plan.steps[0].remote_cmd, "docker restart signal-a");
        assert_eq!(plan.steps[2].remote_cmd, "sudo systemctl stop pub-a.service");
        assert_eq!(plan.steps[3].target, "pub-b.service");
    }

    #[test]
    fn plan_with_no_shed_units_is_flush_then_tighten_only() {
        let plan = build_plan("h", "u", "signal-a", &[], "/opt/mgmt-host").unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].kind, StepKind::Flush);
        assert_eq!(plan.steps[1].kind, StepKind::Tighten);
    }

    #[test]
    fn plan_rejects_unsafe_container_and_unit() {
        assert!(build_plan("h", "u", "signal-a; reboot", &[], "/opt/mgmt-host").is_err());
        assert!(build_plan("h", "u", "signal-a", &["a; rm b".to_string()], "/opt/mgmt-host").is_err());
        assert!(build_plan("h", "u", "signal-a", &[], "/opt/$(x)").is_err());
    }

    #[test]
    fn tighten_cmd_gates_on_metrics_capability_and_recreates_via_compose() {
        let cmd = tighten_remote_cmd("signal-a", "/opt/mgmt-host");
        assert!(cmd.contains("SIGNAL_SENDER_RATE_LIMIT"));
        assert!(cmd.contains("SIGNAL_NEGATIVE_CACHE_TTL"));
        // Deploy gate is the /metrics capability counter — the true "L2 build
        // running" signal — not env-var presence (the L2 block defaults to 0
        // once merged, so presence stops discriminating).
        assert!(cmd.contains("http://127.0.0.1:9090/metrics"));
        assert!(cmd.contains("signal_server_send_rate_limited_total"));
        assert!(cmd.contains(&format!("exit {STORM_SKIP_EXIT}")));
        assert!(cmd.contains("docker compose --env-file .env --env-file storm-brake.env up -d"));
        assert!(cmd.contains("'/opt/mgmt-host'"));
    }

    #[test]
    fn tighten_cmd_halves_rate_but_doubles_ttl() {
        // The two knobs tighten in OPPOSITE directions (L2 owner's correction):
        // rate-limit halved (lower=stricter), neg-cache TTL doubled
        // (higher=more shedding). An off knob enables at the L2 baseline.
        let cmd = tighten_remote_cmd("signal-a", "/opt/mgmt-host");
        assert!(cmd.contains("nrl=$((rl/2))"), "rate limit is halved");
        assert!(cmd.contains("ntt=$((tt*2))"), "neg-cache TTL is doubled, not halved");
        assert!(cmd.contains("nrl=30"), "rate enables at baseline 30 when off");
        assert!(cmd.contains("ntt=20"), "ttl enables at baseline 20 when off");
    }

    // -- shed-unit merge -----------------------------------------------------

    #[test]
    fn merge_shed_units_dedups_and_preserves_order() {
        let merged = merge_shed_units(
            Some("a.service, b.service ,a.service"),
            &["b.service".to_string(), "c.service".to_string()],
        );
        assert_eq!(merged, vec!["a.service", "b.service", "c.service"]);
    }

    #[test]
    fn merge_shed_units_handles_empty_sources() {
        assert!(merge_shed_units(None, &[]).is_empty());
        assert_eq!(
            merge_shed_units(Some(" , x.service , "), &[]),
            vec!["x.service"]
        );
    }

    // -- mgmt target precedence ---------------------------------------------

    #[test]
    fn mgmt_target_flag_beats_env_beats_profile() {
        let t = pick_mgmt_target(
            Some("flag-host".into()),
            Some("env-host".into()),
            None,
            Some("profile-host".into()),
            None,
            None,
            Some("profile-user".into()),
        )
        .unwrap();
        assert_eq!(t.host, "flag-host");
        assert_eq!(t.user, "profile-user");
    }

    #[test]
    fn mgmt_target_without_a_user_from_anywhere_is_refused() {
        let e = pick_mgmt_target(Some("h".into()), None, None, None, None, None, None).unwrap_err();
        assert!(e.to_string().contains("goat context refresh"));
    }

    #[test]
    fn mgmt_target_falls_through_to_profile_and_ctx_env() {
        let t = pick_mgmt_target(
            None,
            None,
            Some("ctx-env-host".into()),
            Some("profile-host".into()),
            None,
            None,
            Some("alice".into()),
        )
        .unwrap();
        assert_eq!(t.host, "ctx-env-host");
        assert_eq!(t.user, "alice");
    }

    #[test]
    fn mgmt_target_unresolved_host_errors() {
        let err = pick_mgmt_target(None, None, None, None, None, None, None).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("mgmt"));
    }

    #[test]
    fn ssh_argv_is_batchmode_hardened() {
        let a = ssh_argv("goat", "100.127.0.1", "docker restart signal-a");
        assert!(a.contains(&"BatchMode=yes".to_string()));
        assert!(a.contains(&"goat@100.127.0.1".to_string()));
        assert_eq!(a.last().unwrap(), "docker restart signal-a");
    }

    // -- outcome classification + aggregation --------------------------------

    #[test]
    fn classify_maps_exit_codes() {
        assert_eq!(
            classify(&RemoteOutput { code: 0, stdout: "".into(), stderr: "".into() }),
            StepStatus::Ok
        );
        assert_eq!(
            classify(&RemoteOutput {
                code: STORM_SKIP_EXIT,
                stdout: "L2 flags not set on signal-a".into(),
                stderr: "".into(),
            }),
            StepStatus::Skipped("L2 flags not set on signal-a".into())
        );
        assert_eq!(
            classify(&RemoteOutput {
                code: 1,
                stdout: "".into(),
                stderr: "no such container: signal-a".into(),
            }),
            StepStatus::Failed("no such container: signal-a".into())
        );
    }

    #[test]
    fn classify_failure_falls_back_to_stdout_then_code() {
        assert_eq!(
            classify(&RemoteOutput { code: 7, stdout: "boom".into(), stderr: "".into() }),
            StepStatus::Failed("boom".into())
        );
        assert_eq!(
            classify(&RemoteOutput { code: 255, stdout: "".into(), stderr: "".into() }),
            StepStatus::Failed("remote command exited 255".into())
        );
    }

    // Mock executor: returns a canned RemoteOutput keyed by which step's
    // remote_cmd is running (matched on a substring), so execute_plan's
    // aggregation is tested without any ssh.
    struct MockExec {
        flush: RemoteOutput,
        tighten: RemoteOutput,
        shed: RemoteOutput,
    }
    impl BrakeExecutor for MockExec {
        async fn run(&self, remote_cmd: &str) -> anyhow::Result<RemoteOutput> {
            let out = if remote_cmd.starts_with("docker restart") {
                &self.flush
            } else if remote_cmd.starts_with("sudo systemctl stop") {
                &self.shed
            } else {
                &self.tighten
            };
            Ok(out.clone())
        }
    }

    fn out(code: i32, stdout: &str, stderr: &str) -> RemoteOutput {
        RemoteOutput { code, stdout: stdout.into(), stderr: stderr.into() }
    }

    #[tokio::test]
    async fn execute_all_ok_reports_ok() {
        let plan = build_plan("h", "u", "signal-a", &["p.service".to_string()], "/opt/mgmt-host").unwrap();
        let exec = MockExec {
            flush: out(0, "", ""),
            tighten: out(0, "tightened", ""),
            shed: out(0, "", ""),
        };
        let report = execute_plan(&exec, &plan).await;
        assert_eq!(report.counts(), (3, 0, 0));
        assert!(!report.flush_failed());
        assert_eq!(report.audit_outcome(), "ok");
    }

    #[tokio::test]
    async fn execute_tighten_skipped_is_partial_not_failed() {
        let plan = build_plan("h", "u", "signal-a", &[], "/opt/mgmt-host").unwrap();
        let exec = MockExec {
            flush: out(0, "", ""),
            tighten: out(STORM_SKIP_EXIT, "L2 not deployed", ""),
            shed: out(0, "", ""),
        };
        let report = execute_plan(&exec, &plan).await;
        assert_eq!(report.counts(), (1, 0, 1));
        assert!(!report.flush_failed());
        assert_eq!(report.audit_outcome(), "partial");
        assert_eq!(
            report.outcomes[1].status,
            StepStatus::Skipped("L2 not deployed".into())
        );
    }

    #[tokio::test]
    async fn execute_flush_failure_is_fatal_outcome() {
        let plan = build_plan("h", "u", "signal-a", &[], "/opt/mgmt-host").unwrap();
        let exec = MockExec {
            flush: out(1, "", "cannot connect to docker daemon"),
            tighten: out(0, "", ""),
            shed: out(0, "", ""),
        };
        let report = execute_plan(&exec, &plan).await;
        assert!(report.flush_failed());
        assert_eq!(report.audit_outcome(), "failed");
    }

    #[tokio::test]
    async fn execute_tighten_failure_is_a_non_flush_failure() {
        let plan = build_plan("h", "u", "signal-a", &["p.service".to_string()], "/opt/mgmt-host").unwrap();
        let exec = MockExec {
            flush: out(0, "", ""),
            tighten: out(1, "", "iptables: permission denied"),
            shed: out(STORM_SKIP_EXIT, "not deployed", ""),
        };
        let report = execute_plan(&exec, &plan).await;
        assert!(!report.flush_failed());
        assert_eq!(
            report.non_flush_failures(),
            vec![format!("TIGHTEN[{}]=iptables: permission denied", report.outcomes[1].target)]
        );
    }

    #[test]
    fn non_flush_failures_ignores_flush_and_skipped_steps() {
        let report = BrakeReport {
            outcomes: vec![
                StepOutcome {
                    kind: StepKind::Flush,
                    target: "signal-a".into(),
                    status: StepStatus::Failed("cannot connect to docker daemon".into()),
                },
                StepOutcome {
                    kind: StepKind::Tighten,
                    target: "l2".into(),
                    status: StepStatus::Skipped("L2 not deployed".into()),
                },
                StepOutcome {
                    kind: StepKind::Shed,
                    target: "p.service".into(),
                    status: StepStatus::Ok,
                },
            ],
        };
        assert!(report.non_flush_failures().is_empty());
        assert!(report.flush_failed());
    }
}
