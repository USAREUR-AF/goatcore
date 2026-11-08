//! `release`: a release candidate from a tag on main through the dev goatnet's
//! live validation to the preprod rung, and after the soak the prod fold —
//! the verb that replaces ops/scripts/rc.sh behind the parity gate. The steps,
//! their order, their refusals and their printed lines are the script's
//! (release-candidate-validation.md, ADR 1173):
//!
//!   1 preflight   deps, a main checkout (C3), admin on the repository, the rc number
//!   2 tag         <image>/vX.Y.Z-rc.N on origin/main; wait for rc-validate and the
//!                 image build; the digest under ghcr.io/usareur-af
//!   3 dev pin     the declared deviation on the dev rung, pins + intents
//!                 regenerated and re-signed, gates, PR landed
//!   4 dev deploy  `deploy` (this binary's push converger), ledger-verified
//!   5 rows        the live acceptance rows the change affects
//!   6 preprod     candidate record opened with the dev evidence, preprod
//!                 deviation, PR landed, deploy, rows, convergence evidence, PR landed
//!   7 soak        the prod command and when it becomes runnable
//!   8 prod        graduate; fold into fleet:; deploy prod; every rung re-verified
//!
//! The pin, record and intent edits are typed: `pins` (the table edits,
//! regeneration, the re-sign of CHANGED intents only) and `promote` (the
//! records) do in-process what the script's embedded Python did, and the
//! parity gate holds the two to the same steps. A re-run whose pin is already
//! on main lands nothing: a tree that differs from main in signatures alone
//! is reported "pins already current on main; nothing to land".
//! `pr-land.sh`, `intent-sign`, `affected-flows.py` and `run-acceptance.py`
//! are still the tools the script calls, called the same way.
//!
//! From an assembled tree (`.goat-assembled` at its root) the goatnets' pins,
//! intents and records belong to the overlay repository the marker names
//! (`overlay_path`), exactly as in rc.sh: the run needs `--tag` and never cuts
//! one; the nominator is an admin of the overlay repository; each step's
//! scratch tree is a fresh assembly built by the overlay's `assemble.sh` from a
//! worktree of the overlay's origin/main; a step's edits land as a pull request
//! on the overlay repository carrying exactly the files the step changed; and
//! after a landing the scratch tree is assembled again and the deploy runs
//! from it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use anyhow::{anyhow, Context, Result};

use crate::pins::read_yaml;
use crate::{pins, promote, push};
use crate::{Check, Report, EXIT_OK, EXIT_REFUSED};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum From {
    Tag,
    Dev,
    Rows,
    Preprod,
    /// Preprod already runs the tag on main: redo only its deploy check, live
    /// rows and evidence, without reopening the record or re-pinning.
    PreprodRows,
    Prod,
}

impl From {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "tag" => From::Tag,
            "dev" => From::Dev,
            "rows" => From::Rows,
            "preprod" => From::Preprod,
            "preprod-rows" => From::PreprodRows,
            "prod" => From::Prod,
            _ => return Err(goat_errors::coded("GB-018", "--from must be tag|dev|rows|preprod|preprod-rows|prod").into()),
        })
    }
}

pub struct Options<'a> {
    pub component: &'a str,
    pub version: Option<&'a str>,
    pub rc: Option<u32>,
    pub ack: &'a str,
    pub from: From,
    pub tag: Option<&'a str>,
    pub dry_run: bool,
}

/// An assembled tree's overlay (rc.sh `ASSEMBLED=1`): where a step's edits land.
struct Overlay {
    /// The overlay checkout the marker names (`OVERLAY`).
    path: PathBuf,
    /// Its origin as `owner/name` (`OVERLAY_SLUG`).
    slug: String,
    /// The overlay worktree a step's edits land from (`OWT`).
    owt: PathBuf,
    /// The scratch tree's content as assembled, a git tree object (`WT_SNAP`).
    snap: String,
}

/// One run's state: the script's variables, named as it names them.
struct Run<'a> {
    repo: &'a Path,
    /// The tree the run reads its facts from and deploys from (rc.sh `REPO`
    /// after an assembly): the invoking checkout, or the newest scratch assembly.
    root: PathBuf,
    overlay: Option<Overlay>,
    o: &'a Options<'a>,
    repo_slug: String,
    image: String,
    fleet: String,
    dev: String,
    pre: String,
    prod: String,
    tag: String,
    rcn: u32,
    reference: String,
    wt: PathBuf,
    rec: String,
    evid: String,
    dev_evid: String,
    checks: Rc<RefCell<Vec<Check>>>,
}

impl Run<'_> {
    /// What the candidate carries (rc.sh `rc_changes`): the one-line summary and
    /// the commit subjects since the component's previous tag.
    fn changes(&self) -> (String, String) {
        let wf = read_yaml(&self.root.join("ops/pins/fleet-pins.yaml"))
            .ok()
            .and_then(|p| p.get("components")?.get(self.o.component)?.get("image_workflow")?.as_str().map(String::from))
            .unwrap_or_default();
        promote::candidate_changes(self.repo, &wf, &self.image, &self.tag, "origin/main")
    }
}

// --- helpers -----------------------------------------------------------------------

fn render(r: &str) -> String {
    match crate::settings::ghcr_org_global() {
        Some(org) => r.replace("{{ ghcr_org }}", &org),
        None => r.to_string(),
    }
}

fn sh(cmd: &str, args: &[&str]) -> Result<String> {
    let mut c = Command::new(cmd);
    c.args(args);
    crate::trace::cmd(&c);
    let out = c.output().with_context(|| format!("{cmd} {}", args.join(" ")))?;
    if !out.status.success() {
        return Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn sh_ok(cmd: &str, args: &[&str]) -> bool {
    let mut c = Command::new(cmd);
    c.args(args).stdout(Stdio::null()).stderr(Stdio::null());
    crate::trace::cmd(&c);
    c.status().map(|s| s.success()).unwrap_or(false)
}

/// The script's facts about the component and the ladder, in its words.
fn facts(repo: &Path, component: &str) -> Result<serde_json::Value> {
    let pins = read_yaml(&repo.join("ops/pins/fleet-pins.yaml"))?;
    let lad = read_yaml(&repo.join("ops/release/ladder.yaml"))?;
    let comp = pins.get("components").and_then(|c| c.get(component)).filter(|c| c.is_object()).ok_or_else(|| goat_errors::coded("GB-001", format!("unknown component '{component}'")))?;
    let wf = comp.get("image_workflow").and_then(|w| w.as_str()).filter(|w| !w.is_empty()).ok_or_else(|| goat_errors::coded("GB-036", format!("{component}: no image_workflow in fleet-pins")))?;
    let image = Path::new(wf).file_name().unwrap_or_default().to_string_lossy().replace("-image.yml", "");
    let rungs: Vec<&serde_json::Value> = lad.get("rungs").and_then(|r| r.as_array()).map(|a| a.iter().filter(|r| r.get("status").and_then(|s| s.as_str()) == Some("live")).collect()).unwrap_or_default();
    let goatnet = |need: &str| -> Result<String> {
        rungs
            .iter()
            .find(|r| r.get("name").and_then(|n| n.as_str()) == Some(need))
            .and_then(|r| r.get("goatnet").and_then(|g| g.as_str()).filter(|g| !g.is_empty()).map(String::from))
            .ok_or_else(|| anyhow::Error::from(goat_errors::coded("GB-037", format!("ladder: rung '{need}' is not live with a goatnet"))))
    };
    let (dev, pre, prod) = (goatnet("dev")?, goatnet("preprod")?, goatnet("prod")?);
    Ok(serde_json::json!({
        "image": image,
        "var": comp.get("var").cloned().unwrap_or(serde_json::Value::Null),
        "ledger": comp.get("ledger_component").and_then(|l| l.as_str()).filter(|l| !l.is_empty()).unwrap_or(component),
        "fleet": comp.get("fleet").cloned().unwrap_or(serde_json::Value::Null),
        "dev": dev, "preprod": pre, "prod": prod,
    }))
}

/// `datetime.date.today()` — the local date, as the script stamps evidence.
fn utc_local_date() -> String {
    chrono::Local::now().date_naive().to_string()
}

/// `os.path.relpath(path, os.getcwd())`.
fn relpath(path: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let p = Path::new(path);
    match p.strip_prefix(&cwd) {
        Ok(r) => r.display().to_string(),
        Err(_) => {
            let a: Vec<_> = cwd.components().collect();
            let b: Vec<_> = p.components().collect();
            let common = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
            let mut out: Vec<String> = vec!["..".into(); a.len() - common];
            out.extend(b[common..].iter().map(|c| c.as_os_str().to_string_lossy().to_string()));
            out.join("/")
        }
    }
}

fn utc_date() -> String {
    let stamp = push_stamp();
    format!("{}-{}-{}", &stamp[0..4], &stamp[4..6], &stamp[6..8])
}

fn push_stamp() -> String {
    // yyyymmddThhmmssZ, shared with push.rs's log names.
    crate::push::stamp_pub()
}

impl<'a> Run<'a> {
    fn step(&mut self, s: &str) {
        self.checks.borrow_mut().push(Check { name: "step".into(), ok: true, detail: format!("==> {s}"), fix: None });
    }
    fn note(&mut self, s: String) {
        self.checks.borrow_mut().push(Check { name: "note".into(), ok: true, detail: s, fix: None });
    }
    /// `run`: in a dry run, print the command; otherwise execute it (a failure is an error).
    fn run(&mut self, cmd: &str, args: &[String]) -> Result<()> {
        if self.o.dry_run {
            self.note(format!("DRY: {cmd} {}", args.join(" ")));
            return Ok(());
        }
        let mut c = Command::new(cmd);
        c.args(args);
        crate::trace::cmd(&c);
        let st = c.status().with_context(|| cmd.to_string())?;
        if !st.success() {
            return Err(goat_errors::coded("GB-028", format!("{cmd} {} failed", args.join(" "))).into());
        }
        Ok(())
    }
    /// A typed edit: in a dry run, say which and with what; otherwise do it.
    fn edit(&mut self, kind: &str, args: &[String], f: impl FnOnce() -> Result<()>) -> Result<()> {
        if self.o.dry_run {
            self.note(format!("DRY: edit {kind} {}", args.join(" ")));
            return Ok(());
        }
        f()
    }
    /// A `promote` step on the worktree's tree: the script's lines become notes; a refusal is an error.
    fn promote(&mut self, dry_line: &str, cmd: &promote::Sub) -> Result<Vec<String>> {
        if self.o.dry_run {
            self.note(format!("DRY: goatbuilder promote {dry_line}"));
            return Ok(Vec::new());
        }
        let (lines, code) = promote::call(&self.wt, cmd);
        if code != 0 {
            return Err(anyhow!("{}", lines.join("\n")));
        }
        Ok(lines)
    }

    fn mkwt(&mut self) -> Result<()> {
        if self.wt.is_dir() {
            return Ok(());
        }
        let branch = format!("chore/rc-{}-{}", self.image, push_stamp().replace('T', "-").trim_end_matches('Z'));
        if let Some(ov) = &self.overlay {
            let (ovp, owt) = (ov.path.display().to_string(), ov.owt.display().to_string());
            self.run("git", &["-C".into(), ovp.clone(), "fetch".into(), "-q".into(), "origin".into(), "main".into()])?;
            self.run("git", &["-C".into(), ovp, "worktree".into(), "add".into(), "-q".into(), owt.clone(), "-b".into(), branch, "origin/main".into()])?;
            if self.o.dry_run {
                self.note(format!("DRY: {owt}/assemble.sh {}  (the scratch tree: goatcore + goatstuff + the overlay worktree)", self.wt.display()));
                return Ok(());
            }
            return self.assemble_wt();
        }
        let args: Vec<String> = ["-C", self.repo.to_str().unwrap_or("."), "worktree", "add", "-q", self.wt.to_str().unwrap_or("."), "-b", &branch, "origin/main"].iter().map(|s| s.to_string()).collect();
        self.run("git", &args)
    }

    /// Assembled tree only (rc.sh `assemble_wt`): build the scratch tree from the
    /// overlay worktree with the overlay's own assemble.sh. Its HEAD stays the
    /// goatcore commit, so its starting content is kept as a tree object and a
    /// step's edits are the diff against it. The run then reads and deploys from it.
    fn assemble_wt(&mut self) -> Result<()> {
        let Some(ov) = self.overlay.as_mut() else { return Ok(()) };
        let wt = self.wt.clone();
        let _ = std::fs::remove_dir_all(&wt);
        let out = Command::new(ov.owt.join("assemble.sh")).arg(&wt).output().with_context(|| format!("{}/assemble.sh", ov.owt.display()))?;
        if !out.status.success() {
            eprint!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            return Err(goat_errors::coded("GB-028", format!("{}/assemble.sh could not build the scratch tree {}", ov.owt.display(), wt.display())).into());
        }
        let wt_s = wt.display().to_string();
        sh("git", &["-C", &wt_s, "add", "-A", "--force"])?;
        ov.snap = sh("git", &["-C", &wt_s, "write-tree"])?;
        sh("git", &["-C", &wt_s, "reset", "-q"])?;
        let short = sh("git", &["-C", &ov.owt.display().to_string(), "rev-parse", "--short", "HEAD"]).unwrap_or_default();
        self.root = wt.clone();
        self.note(format!("scratch tree {wt_s} assembled from overlay {short}"));
        Ok(())
    }

    /// Assembled tree only (rc.sh `land_overlay`): the step's edits in the
    /// scratch tree, as a pull request on the overlay repository; then the next
    /// scratch tree is assembled from the overlay's merged main.
    fn land_overlay(&mut self, title: &str, body: &str) -> Result<()> {
        let Some(ov) = self.overlay.as_ref() else { return Ok(()) };
        let (wt_s, owt) = (self.wt.display().to_string(), ov.owt.clone());
        let owt_s = owt.display().to_string();
        sh("git", &["-C", &wt_s, "add", "-A", "--force"])?;
        let changes = sh("git", &["-C", &wt_s, "diff-index", "--cached", "--name-status", "--no-renames", &ov.snap]).unwrap_or_default();
        sh("git", &["-C", &wt_s, "reset", "-q"])?;
        let rows: Vec<(String, String)> = changes
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(st, p)| (st.to_string(), p.to_string())))
            .filter(|(_, p)| !p.is_empty())
            .collect();
        if !rows.iter().any(|(_, p)| !p.ends_with(".sig")) {
            self.note("pins already current on the overlay's main; nothing to land".into());
            return Ok(());
        }
        for (st, p) in &rows {
            if st == "D" {
                sh("git", &["-C", &owt_s, "rm", "-q", "--ignore-unmatch", "--", p])?;
            } else {
                let dst = owt.join(p);
                if let Some(d) = dst.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::copy(self.wt.join(p), &dst).with_context(|| format!("copy {p} to the overlay worktree"))?;
            }
            self.note(format!("lands {st} {p}"));
        }
        let b = std::env::temp_dir().join(format!("goatbuilder-pr-body-{}", std::process::id()));
        std::fs::write(&b, format!("{body}\n"))?;
        let branch = sh("git", &["-C", &owt_s, "rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|_| "rc".into());
        let land = self.repo.join("ops/scripts/pr-land.sh").display().to_string();
        self.run(&land, &["--worktree".into(), owt_s.clone(), "--branch".into(), branch, "--title".into(), title.into(), "--body-file".into(), b.display().to_string()])?;
        // The overlay's main now carries the edit: build the next scratch tree
        // from it, so the deploy that follows reads a current overlay.
        let _ = sh("git", &["-C", &owt_s, "fetch", "-q", "origin", "main"]);
        let nb = format!("chore/rc-{}-{}", self.image, push_stamp().replace('T', "-").trim_end_matches('Z'));
        sh("git", &["-C", &owt_s, "checkout", "-q", "-B", &nb, "origin/main"])?;
        self.assemble_wt()
    }

    fn set_deviation(&mut self, goatnet: &str, reason: &str) -> Result<()> {
        let pins_path = self.wt.join("ops/pins/fleet-pins.yaml");
        let args = vec![pins_path.display().to_string(), self.o.component.to_string(), goatnet.to_string(), self.reference.clone(), reason.to_string()];
        let (comp, g, r, why) = (self.o.component.to_string(), goatnet.to_string(), self.reference.clone(), reason.to_string());
        self.edit("set-deviation", &args, || pins::edit_set_deviation(&pins_path, &comp, &g, &r, &why))
    }

    /// Before the dev rung's check: every open ladder candidate for the
    /// component that check C6 would refuse — overtaken on preprod, usually by
    /// a hotfix, and not on the dev rung either — is abandoned as superseded,
    /// naming the hotfix record that overtook it, else this candidate's ref.
    /// Without this a component carrying such a record could not start a new
    /// candidate at dev at all (onboard-runner).
    fn reconcile_superseded(&mut self, rung: &str) -> Result<()> {
        let base = if self.wt.join("ops/release").is_dir() { self.wt.clone() } else { self.root.clone() };
        for (old, hotfix) in promote::stale_candidates_c6(&base, self.o.component) {
            let by = hotfix.unwrap_or_else(|| self.reference.clone());
            let reason = format!("Superseded on {rung}: the rung no longer runs this candidate (check C6); overtaken by {by} before release candidate {} (ADR 1173), nominated by {}", self.tag, self.o.ack);
            self.promote(&format!("abandon {old} --reason {reason} --superseded-by {by}"), &promote::Sub::Abandon { record: old.clone(), reason, superseded_by: Some(by) })?;
        }
        Ok(())
    }

    /// Regenerate pins + intents in the worktree; re-sign ONLY the intents
    /// whose body changed (a fresh signature over an unchanged body is a diff
    /// that lands a PR of nothing but .sig files); then the two gates.
    fn regen_sign_check(&mut self) -> Result<()> {
        let changed: Vec<String> = if self.o.dry_run {
            self.note(format!("DRY: goatbuilder pins {}", self.wt.display()));
            // nothing is regenerated in a dry run, so every intent is named below
            let intents = self.root.join("ops/consistency/intent");
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(&intents).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
            dirs.sort();
            dirs.iter().map(|d| d.file_name().unwrap_or_default().to_string_lossy().to_string()).collect()
        } else {
            let r = pins::regenerate(&self.wt)?;
            if r.changed_intents.is_empty() {
                self.note("intents unchanged — no re-sign (a fresh signature over an unchanged body would land a PR of .sig files)".into());
            }
            r.changed_intents
        };
        for g in changed {
            let i = self.wt.join("ops/consistency/intent").join(&g).join("intent.json");
            let src = if i.is_file() { i.clone() } else { self.root.join("ops/consistency/intent").join(&g).join("intent.json") };
            let (key, _, _) = pins::ca_key_for(&src, &g)?;
            if !Path::new(&key).is_file() {
                // A dry run signs nothing: say what a real run needs and go on, so
                // the rehearsal of a step runs anywhere (rc.sh regen_sign_check).
                if self.o.dry_run {
                    self.note(format!("DRY: CA key for {g} not at {key} — a real run signs {g}'s intent with it and refuses without it"));
                    continue;
                }
                return Err(goat_errors::coded("GB-019", format!("CA key for {g} not at {key}")).into());
            }
            let home = std::env::var("HOME").unwrap_or_default();
            let audit = crate::settings::ca_audit_log_abs_global();
            let cmd = format!("cd '{}' && go run ./cmd/intent-sign --intent '{}' --root-key '{}' --age-identity '{home}/.config/age/keys.txt' --audit-log '{audit}' >/dev/null", self.wt.join("ops/enrollment").display(), i.display(), key);
            if self.o.dry_run {
                self.note(format!("DRY: bash -c {cmd}"));
            } else {
                pins::sign_intent(&self.wt, &i, &key)?;
            }
        }
        self.promote("check", &promote::Sub::Check)?;
        if self.o.dry_run {
            self.note(format!("DRY: goatbuilder pins --check {}", self.wt.display()));
        } else {
            let r = pins::run(&self.wt, true)?;
            if !r.ok {
                return Err(anyhow!("{}", r.checks.iter().filter(|c| !c.ok).map(|c| c.detail.clone()).collect::<Vec<_>>().join("\n")));
            }
        }
        Ok(())
    }

    fn land(&mut self, title: &str, body: &str) -> Result<()> {
        if let Some(ov) = &self.overlay {
            if self.o.dry_run {
                let line = format!("DRY: the step's edits in {} land as a PR on {} via {}/pr-land.sh --worktree {}: {title}", self.wt.display(), ov.slug, self.repo.join("ops/scripts").display(), ov.owt.display());
                self.note(line);
                return Ok(());
            }
            return self.land_overlay(title, body);
        }
        if !self.o.dry_run {
            // A re-run whose pin is already merged regenerates the same bytes;
            // only signatures (never re-signed now) or nothing would differ.
            let wt_s = self.wt.display().to_string();
            let mut paths: Vec<String> = sh("git", &["-C", &wt_s, "diff", "--name-only", "origin/main"]).unwrap_or_default().lines().map(String::from).collect();
            paths.extend(sh("git", &["-C", &wt_s, "ls-files", "--others", "--exclude-standard"]).unwrap_or_default().lines().map(String::from));
            if pins::nothing_to_land(&paths) {
                let _ = sh("git", &["-C", &wt_s, "checkout", "-q", "--", "."]);
                self.note("pins already current on main; nothing to land".into());
                return Ok(());
            }
        }
        let b = std::env::temp_dir().join(format!("goatbuilder-pr-body-{}", std::process::id()));
        std::fs::write(&b, format!("{body}\n"))?;
        let branch = sh("git", &["-C", self.wt.to_str().unwrap_or("."), "rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|_| "rc".into());
        let mut args: Vec<String> = vec![
            "--worktree".into(), self.wt.display().to_string(), "--branch".into(), branch, "--title".into(), title.into(), "--body-file".into(), b.display().to_string(),
        ];
        if self.o.dry_run {
            args.push("--dry-run".into());
        }
        let land = self.repo.join("ops/scripts/pr-land.sh").display().to_string();
        self.run(&land, &args)?;
        if !self.o.dry_run {
            // The pin PR just merged advanced the trunk; the deploy that follows runs
            // from THIS checkout, which ansible-preflight requires to be current.
            let rbr = sh("git", &["-C", self.repo.to_str().unwrap_or("."), "symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_else(|_| "main".into());
            let _ = sh("git", &["-C", self.repo.to_str().unwrap_or("."), "fetch", "-q", "origin", &rbr]);
            let ok = Command::new("git")
                .env("GOAT_ALLOW_MASTER_GIT_MUTATE", "1")
                .args(["-C", self.repo.to_str().unwrap_or("."), "merge", "--ff-only", &format!("origin/{rbr}")])
                .stdout(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err(goat_errors::coded("GB-015", format!("could not fast-forward {} to origin/{rbr} after landing the PR — {rbr} has diverged; resolve it, then re-run the same command (re-running is safe — the pin PR already landed and release resumes)", self.repo.display())).into());
            }
            let nb = format!("chore/rc-{}-{}", self.image, push_stamp().replace('T', "-").trim_end_matches('Z'));
            let _ = sh("git", &["-C", self.wt.to_str().unwrap_or("."), "checkout", "-q", "-B", &nb, "origin/main"]);
        }
        Ok(())
    }

    fn runner_var(&self, goatnet: &str, key: &str) -> Result<String> {
        let runners = read_yaml(&self.repo.join("ops/validation/goatnet-runners.yaml"))?;
        let v = runners.get("goatnets").and_then(|g| g.get(goatnet)).and_then(|g| g.get(key));
        Ok(match v {
            None => String::new(),
            Some(serde_json::Value::Null) => "None".into(),
            Some(x) => crate::pins::py_str(x),
        })
    }

    /// The live acceptance rows the change affects, run on a goatnet; the evidence path lands in `self.evid`.
    fn rows_on(&mut self, goatnet: &str) -> Result<bool> {
        let name = self.runner_var(goatnet, "name")?;
        if name.is_empty() {
            return Err(goat_errors::coded("GB-016", format!("no runner settings for {goatnet} in ops/validation/goatnet-runners.yaml")).into());
        }
        let rows = Command::new("python3")
            .args([self.repo.join("ops/validation/affected-flows.py").to_str().unwrap_or(""), "--component", self.o.component, "--head", &self.tag, "--json"])
            .output()
            .ok()
            .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
            .and_then(|v| v.get("live_rows").and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>()))
            .unwrap_or_default()
            .join(",");
        if rows.is_empty() {
            self.note(format!("no live rows are mapped for what {} changed (ops/validation/flow-map.yaml) — evidence is the cheap suites only", self.tag));
            self.evid.clear();
            return Ok(true);
        }
        let tests = read_yaml(&self.repo.join("ops/validation/acceptance-tests.yaml"))?;
        let wanted: Vec<&str> = rows.split(',').collect();
        let mut reqs: Vec<String> = tests
            .get("tests")
            .and_then(|t| t.as_array())
            .map(|a| a.iter().filter(|t| t.get("id").and_then(|i| i.as_str()).map(|i| wanted.contains(&i)).unwrap_or(false)).flat_map(|t| t.get("requires").and_then(|r| r.as_array()).cloned().unwrap_or_default()).map(|r| crate::pins::py_str(&r)).collect())
            .unwrap_or_default();
        reqs.sort();
        reqs.dedup();
        let req = reqs.join(",");
        let evid = self.repo.join("ops/validation/evidence").join(format!("{}-{}-{name}.jsonl", self.image, self.tag.split('/').nth(1).unwrap_or(&self.tag)));
        std::fs::create_dir_all(evid.parent().unwrap())?;
        self.evid = evid.display().to_string();
        self.note(format!("rows on {name}: {rows} (requires {req})"));
        // A browser row needs the console's Playwright toolchain on this machine;
        // without it the rows are UNTESTED, never FAILED (rc.sh browser_toolchain_check).
        if rows.split(',').any(|r| matches!(r, "intake-form-browser-submit" | "review-gate-accepts-open-field-invite")) {
            let e2e = self.repo.join("ops/goat-id-console/web/e2e");
            let mut missing: Vec<String> = Vec::new();
            for t in ["node", "npm"] {
                if !sh_ok("sh", &["-c", &format!("command -v {t}")]) {
                    missing.push(t.into());
                }
            }
            if !e2e.join("node_modules").is_dir() {
                missing.push(e2e.join("node_modules").display().to_string());
            }
            if missing.is_empty() {
                self.note(format!("browser row(s) in [{rows}]: node, npm and {}/node_modules present", e2e.display()));
            } else {
                self.note(format!("browser row(s) in [{rows}] need the console's Playwright toolchain here; missing: {}", missing.join(" ")));
                self.note(format!("  fix: (cd {} && npm ci && npx playwright install chromium)   — then re-run the same command", e2e.display()));
                if !self.o.dry_run {
                    return Err(goat_errors::coded("GB-042", format!("the live rows on {name} cannot run from this machine (see above) — the candidate is UNTESTED here, not dead; install the toolchain and re-run the same command")).into());
                }
            }
        }
        let mut via = self.runner_var(goatnet, "goat_via")?;
        let mut jump = String::new();
        // A rung this laptop is not meshed to runs its rows on its own management
        // host through the door (goat_via `door:<env>`; release-candidate-validation.md):
        // the rows' ssh goes to that host's mesh address and jumps through the
        // door (ops/validation/probes.py _ssh_jump). The door address itself is
        // never the destination: it is a tunnel port whose host key churns.
        if let Some(env) = via.strip_prefix("door:") {
            let env = env.to_string();
            let door_open = self.repo.join("ops/scripts/door-open.sh");
            if self.o.dry_run {
                // rc.sh prints this step in a dry run; the parity gate compares it.
                self.note(format!("DRY: {} {env}  (the rows run goat on {name}'s management host through the door)", door_open.display()));
            }
            if !self.o.dry_run && !sh_ok(door_open.to_str().unwrap_or(""), &[&env]) {
                return Err(goat_errors::coded("GB-043", format!("door to {env} did not open for the rows on {name}")).into());
            }
            let env_door = self.repo.join("ops/scripts/env-door.sh");
            jump = sh(env_door.to_str().unwrap_or(""), &[&env, "--ssh-target"]).unwrap_or_default();
            via = sh(env_door.to_str().unwrap_or(""), &[&env, "--host-target"]).unwrap_or_default();
            if (jump.is_empty() || via.is_empty()) && !self.o.dry_run {
                return Err(goat_errors::coded("GB-043", format!("env-door.sh {env} printed no door (--ssh-target) or host (--host-target)")).into());
            }
        }
        // The contexts the rows run as come from the values file (goat_sponsor_context,
        // probe_context): non-person identities, never an operator key (ADR 1207).
        let res = self.runner_var(goatnet, "intake_resolve")?;
        let values = self.repo.join(self.runner_var(goatnet, "values")?).display().to_string();
        let args: Vec<String> = vec![
            format!("GOAT_ROWS_SSH_JUMP={jump}"), format!("INTAKE_RESOLVE={res}"), "python3".into(), self.repo.join("ops/validation/run-acceptance.py").display().to_string(),
            "--env".into(), name.clone(), "--values".into(), values, "--deployed".into(), req, "--only".into(), rows,
            "--var".into(), format!("goat_via={via}"), "--var".into(), format!("env={name}"),
            "--json".into(), self.evid.clone(), "--require-pass".into(),
        ];
        if self.o.dry_run {
            self.note(format!("DRY: env {}", args.join(" ")));
            return Ok(true);
        }
        let st = Command::new("env").args(&args).status()?;
        // exit 2 from the runner is an environment problem (a tool missing, a
        // context not configured): the rows did not run. UNTESTED, never FAIL.
        if st.code() == Some(2) {
            return Err(goat_errors::coded("GB-042", format!("the live rows on {name} did not run (exit 2: a tool or context is missing on this machine — read the output above) — the candidate is UNTESTED here, not dead; fix the environment and re-run the same command")).into());
        }
        Ok(st.success())
    }

    fn deploy(&mut self, goatnet: &str, verify_only: bool) -> Result<()> {
        let r = push::deploy(&self.root, &push::Options { component: self.o.component, goatnet, only_group: None, dry_run: self.o.dry_run, verify_only, ack: self.o.ack })?;
        let ok = r.ok;
        self.checks.borrow_mut().extend(r.checks);
        if !ok && !self.o.dry_run {
            return Err(goat_errors::coded("GB-017", format!("{} on {goatnet}: NOT converged everywhere (see above)", self.o.component)).into());
        }
        Ok(())
    }
}

/// A refusal keeps the steps already printed, as the script's stderr `rc: …`
/// follows its stdout steps: the report carries them and ends with the refusal.
/// The open record for `reference` when `pre` already pins it on main, else
/// None: the --from preprod-rows lookup, made by every run before its preprod
/// step. Exactly one open record for the digest, or none.
/// A dry run has no digest (it does not read the registry): the image and tag
/// decide there, and the digest is the one preprod pins.
fn preprod_open_record(repo: &Path, component: &str, pre: &str, reference: &str, dry_run: bool) -> Result<Option<String>> {
    if reference.is_empty() {
        return Ok(None);
    }
    let pins = read_yaml(&repo.join("ops/pins/fleet-pins.yaml"))?;
    let pinned = pins.get("components").and_then(|c| c.get(component)).and_then(|c| c.get("deviations")).and_then(|d| d.get(pre)).and_then(|d| d.get("value")).and_then(|v| v.as_str()).unwrap_or("");
    let image_tag = |r: &str| r.rsplit_once('@').map(|(a, _)| a.to_string()).unwrap_or_else(|| r.to_string());
    let same = if dry_run { !pinned.is_empty() && image_tag(pinned) == image_tag(reference) } else { pinned == reference };
    if !same {
        return Ok(None);
    }
    let d12: String = pinned.rsplit(':').next().unwrap_or("").chars().take(12).collect();
    let hits = promote::open_records_by_digest(repo, component, &d12);
    Ok(if hits.len() == 1 { hits.into_iter().next() } else { None })
}

pub fn release(repo: &Path, o: &Options) -> Result<Report> {
    let checks: Rc<RefCell<Vec<Check>>> = Rc::new(RefCell::new(Vec::new()));
    match release_inner(repo, o, checks.clone()) {
        Ok(r) => Ok(r),
        Err(e) => {
            let mut v = checks.take();
            v.push(Check { name: "refused".into(), ok: false, detail: e.to_string(), fix: None });
            let mut r = Report::new("release", v, EXIT_REFUSED);
            r.ok = false;
            r.exit = EXIT_REFUSED;
            Ok(r)
        }
    }
}

fn release_inner(repo: &Path, o: &Options, checks: Rc<RefCell<Vec<Check>>>) -> Result<Report> {
    if o.ack.is_empty() {
        return Err(goat_errors::coded("GB-018", "--ack '<name>' is required — a candidate reaches preprod and is attributed").into());
    }
    if o.version.is_none() && o.tag.is_none() {
        return Err(goat_errors::coded("GB-018", "--version X.Y.Z (or --tag) is required").into());
    }
    if matches!(o.from, From::Prod | From::PreprodRows) && o.tag.is_none() {
        return Err(goat_errors::coded("GB-018", format!("--from {} needs --tag <the candidate tag on preprod>", if o.from == From::Prod { "prod" } else { "preprod-rows" })).into());
    }
    let repo_s = repo.to_str().unwrap_or(".").to_string();
    let url = sh("git", &["-C", &repo_s, "remote", "get-url", "origin"])?;
    let repo_slug = url.trim_end_matches(".git").rsplit([':', '/']).take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("/");

    // --- environment preflight
    if !sh_ok("python3", &["-c", "import yaml, jsonschema"]) {
        return Err(goat_errors::coded("GB-010", "python deps missing — this needs PyYAML + jsonschema. Install once:  pip install pyyaml jsonschema  (or pipx/venv), then re-run. Re-running the same command is safe (an existing tag is validated as-is).").into());
    }
    let mut rbr0 = sh("git", &["-C", &repo_s, "symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_else(|_| "DETACHED".into());
    // An assembled tree (rc.sh ASSEMBLED=1): the overlay the marker names owns
    // the pins and records this run changes.
    let mut overlay: Option<Overlay> = None;
    if let Some(m) = crate::fresh::marker(repo)? {
        if m.goatcore_ref == "local" {
            return Err(goat_errors::coded("GB-009", format!("{} was assembled from local directories (goatcore_ref=local), which no deploy accepts; assemble from the pinned refs", repo.display())).into());
        }
        let ovp = m.overlay_path.clone().unwrap_or_default();
        if ovp.is_empty() || !Path::new(&ovp).join("assemble.sh").is_file() {
            return Err(goat_errors::coded("GB-009", format!("{}/.goat-assembled names no overlay checkout with an assemble.sh (overlay_path={}); re-assemble from the overlay", repo.display(), if ovp.is_empty() { "<none>" } else { &ovp })).into());
        }
        // Detached by construction: its currency is the overlay commit it was
        // built from, which the deploy's fresh-source check judges.
        rbr0 = format!("assembled tree, overlay {}", m.overlay_commit.chars().take(12).collect::<String>());
        if o.from == From::Tag {
            return Err(goat_errors::coded("GB-018", format!("--from tag cuts the tag on this tree's origin ({repo_slug}); from an assembled tree, cut the tag where the component's image workflow builds it, then re-run here with --tag <that tag> --from dev")).into());
        }
        if o.tag.is_none() {
            return Err(goat_errors::coded("GB-018", "from an assembled tree, name the candidate with --tag <component>/vX.Y.Z-rc.N").into());
        }
        let ourl = sh("git", &["-C", &ovp, "remote", "get-url", "origin"])?;
        let slug = ourl.trim_end_matches(".git").rsplit([':', '/']).take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("/");
        overlay = Some(Overlay { path: PathBuf::from(&ovp), slug, owt: PathBuf::new(), snap: String::new() });
    } else if rbr0 != "main" && rbr0 != "master" {
        return Err(goat_errors::coded("GB-009", format!("run rc.sh from a checkout on main — this one ({}) is on '{rbr0}'. rc.sh deploys from the invoking checkout and only a main checkout is kept current with the pin it merges; a feature-branch run can deploy a stale pin and verify green. Switch to a current main checkout (git checkout main && git pull --ff-only origin main) and re-run.", repo.display())).into());
    }

    // --- facts
    let facts = facts(repo, o.component)?;
    let f = |k: &str| facts.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut run = Run {
        repo,
        root: repo.to_path_buf(),
        overlay,
        o,
        repo_slug,
        image: f("image"),
        fleet: f("fleet"),
        dev: f("dev"),
        pre: f("preprod"),
        prod: f("prod"),
        tag: o.tag.unwrap_or("").to_string(),
        rcn: o.rc.unwrap_or(0),
        reference: String::new(),
        wt: repo.parent().unwrap_or(repo).join(format!("{}-wt-rc-{}", repo.file_name().unwrap_or_default().to_string_lossy(), &push_stamp()[9..15])),
        rec: String::new(),
        evid: String::new(),
        dev_evid: String::new(),
        checks,
    };
    if let Some(ov) = run.overlay.as_mut() {
        ov.owt = PathBuf::from(format!("{}-overlay", run.wt.display()));
    }
    run.step(&format!("component {}", o.component));
    let fleet_shown = if run.fleet.is_empty() { "<none>".to_string() } else { render(&run.fleet) };
    run.note(format!("image {}; rungs dev={} preprod={} prod={}; fleet ref {fleet_shown}", run.image, run.dev, run.pre, run.prod));
    let _ = sh("git", &["-C", &repo_s, "fetch", "-q", "origin", "main"]);
    let main = sh("git", &["-C", &repo_s, "rev-parse", "--short", "origin/main"])?;

    // --- 1 preflight
    run.step("preflight");
    run.note(format!("deploying from {} on {rbr0}; deps ok (PyYAML, jsonschema)", repo.display()));
    // GOAT_RC_ROLE_CHECK=0 skips the nominator check where there is no person
    // (the parity gate in CI); the script honours the same variable.
    let role_check = std::env::var("GOAT_RC_ROLE_CHECK").map(|v| v != "0").unwrap_or(true);
    if role_check && run.overlay.is_some() {
        // Nothing is tagged here; what the run changes is the overlay
        // repository's pins and records, so its admins are the nominators.
        let slug = run.overlay.as_ref().map(|ov| ov.slug.clone()).unwrap_or_default();
        let me = sh("gh", &["api", "user", "-q", ".login"])?;
        let role = sh("gh", &["api", &format!("repos/{slug}/collaborators/{me}/permission"), "-q", ".role_name"]).unwrap_or_else(|_| "unknown".into());
        if role != "admin" {
            return Err(goat_errors::coded("GB-011", format!("{me} holds '{role}' on {slug}; only an admin of the overlay repository may put a release candidate on its goatnets")).into());
        }
        run.note(format!("{me} holds {role} on {slug} (the overlay this tree was assembled from)"));
    } else if role_check {
        let me = sh("gh", &["api", "user", "-q", ".login"])?;
        let role = sh("gh", &["api", &format!("repos/{}/collaborators/{me}/permission", run.repo_slug), "-q", ".role_name"]).unwrap_or_else(|_| "unknown".into());
        if role != "admin" {
            return Err(goat_errors::coded("GB-011", format!("{me} holds '{role}' on {}; only a repository admin may nominate a release candidate (the rule rc-validate.yml enforces)", run.repo_slug)).into());
        }
        run.note(format!("{me} holds {role} on {}", run.repo_slug));
        // The origin repository must run rc-validate.yml, or the tag is pushed and
        // the wait dies on a 404 with a tag already cut. The script makes the same check.
        if sh("gh", &["api", &format!("repos/{}/actions/workflows/rc-validate.yml", run.repo_slug), "-q", ".state"]).is_err() {
            return Err(goat_errors::coded("GB-077", format!("{} does not run rc-validate.yml, so it is not where release candidates are cut; run from a checkout whose origin is the primary repository (git remote get-url origin)", run.repo_slug)).into());
        }
    }
    if run.tag.is_empty() {
        let version = o.version.unwrap_or("");
        if run.rcn == 0 {
            let last = sh("git", &["-C", &repo_s, "ls-remote", "--tags", "--refs", "origin", &format!("refs/tags/{}/v{version}-rc.*", run.image)])
                .unwrap_or_default()
                .lines()
                .filter_map(|l| l.rsplit("-rc.").next().and_then(|n| n.parse::<u32>().ok()))
                .max()
                .unwrap_or(0);
            run.rcn = last + 1;
        }
        run.tag = format!("{}/v{version}-rc.{}", run.image, run.rcn);
    }
    run.note(format!("candidate tag {} on main {main}", run.tag));
    // Secrets, before the tag: every rung this run deploys to templates the
    // component's secrets from this environment. Refuse here, naming every
    // variable, rather than tag and pin first and refuse at the deploy (rc.sh
    // secrets_preflight; the panoscope run tagged before it looked).
    {
        let rungs: Vec<String> = match o.from {
            From::Prod => vec![run.prod.clone()],
            From::Preprod => vec![run.pre.clone()],
            _ => vec![run.dev.clone(), run.pre.clone()],
        };
        let mut missing_any = false;
        for g in rungs {
            let p = push::plan(repo, &push::Options { component: o.component, goatnet: &g, only_group: None, dry_run: true, verify_only: false, ack: "" });
            match p {
                Ok(p) if !p.secrets_missing.is_empty() => {
                    missing_any = true;
                    run.note(format!("secrets: {} on {g} templates {} variable(s) from your environment; NOT SET: {}", o.component, p.secrets.len(), p.secrets_missing.join(" ")));
                    // Named by this checkout, not by ours: `repo: source:` and
                    // `secrets: op_account:` in goatbuilder.yaml.
                    let owner = crate::config::owner(repo).unwrap_or_else(|| "<owner>".to_string());
                    let account = crate::config::op_account(repo).unwrap_or_else(|| "<your account>".to_string());
                    run.note(format!("         references (pointers, no values) live in {owner}/goat-credentials refs/goat.refs; put the lines for those names in .envrc.local.d/<file>.refs, then"));
                    run.note(format!("         OP_ACCOUNT={account} op run --env-file=.envrc.local.d/<file>.refs -- <this same command>   (under direnv: direnv exec . env OP_ACCOUNT=… op run …)"));
                }
                Ok(_) => run.note(format!("secrets: {} on {g} — every variable the play templates is set", o.component)),
                Err(e) => run.note(format!("secrets: could not derive what {} needs on {g} ({e})", o.component)),
            }
        }
        if missing_any {
            if o.dry_run {
                run.note("secrets: a real run refuses here, before the tag and the pin".to_string());
            } else {
                return Err(goat_errors::coded("GB-044", format!("refusing before the tag: {} needs the variables named above in the environment", o.component)).into());
            }
        }
    }

    // --- 2 tag + wait
    if o.from == From::Tag {
        run.step(&format!("tag {}", run.tag));
        let exists = sh("git", &["-C", &repo_s, "ls-remote", "--tags", "origin", &format!("refs/tags/{}", run.tag)]).map(|s| !s.is_empty()).unwrap_or(false);
        if exists {
            run.note("tag exists on origin — validating it as-is".into());
        } else {
            let msg = format!("{} release candidate {}-rc.{} (ADR 1173), nominated by {}", run.image, o.version.unwrap_or(""), run.rcn, o.ack);
            let tag = run.tag.clone();
            run.run("git", &["-C".into(), repo_s.clone(), "tag".into(), "-a".into(), tag.clone(), "origin/main".into(), "-m".into(), msg])?;
            run.run("git", &["-C".into(), repo_s.clone(), "push".into(), "-q".into(), "origin".into(), tag])?;
        }
        if !o.dry_run {
            run.note(format!("waiting for rc-validate and the image build on {}", run.tag));
            for wf in ["rc-validate.yml".to_string(), format!("{}-image.yml", run.image)] {
                let mut rid = String::new();
                for _ in 0..20 {
                    rid = sh("gh", &["run", "list", "-R", &run.repo_slug, "--workflow", &wf, "-L", "5", "--json", "databaseId,headBranch", "-q", &format!(".[] | select(.headBranch==\"{}\") | .databaseId", run.tag)])
                        .unwrap_or_default()
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    if !rid.is_empty() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(10));
                }
                if rid.is_empty() {
                    return Err(goat_errors::coded("GB-012", format!("no {wf} run appeared for {}", run.tag)).into());
                }
                let _ = sh_ok("gh", &["run", "watch", &rid, "-R", &run.repo_slug, "--exit-status"]);
                let c = sh("gh", &["run", "view", &rid, "-R", &run.repo_slug, "--json", "conclusion", "-q", ".conclusion"]).unwrap_or_default();
                run.note(format!("{wf}: {c}"));
                if wf == "rc-validate.yml" && c != "success" {
                    let still = sh("git", &["-C", &repo_s, "ls-remote", "--tags", "origin", &format!("refs/tags/{}", run.tag)]).map(|s| !s.is_empty()).unwrap_or(false);
                    if still {
                        run.note("rc-validate could not read the pusher's role — the tag was kept; the cheap suites did not run on the tag. Continuing on the live rows.".into());
                    } else {
                        return Err(goat_errors::coded("GB-011", format!("rc-validate deleted {}: the pusher does not hold maintain/admin, or the commit is not on main", run.tag)).into());
                    }
                }
                if wf.ends_with("-image.yml") && c != "success" {
                    return Err(goat_errors::coded("GB-012", format!("image build failed for {}", run.tag)).into());
                }
            }
        }
    }
    let short_tag = run.tag.split('/').nth(1).unwrap_or(&run.tag).to_string();
    let dig = if o.dry_run {
        format!("sha256:<digest of {}:{short_tag}>", run.image)
    } else {
        // Any of crane, skopeo or docker resolves it (ADR 1204); the tool's own error is kept.
        sh(&repo.join("ops/scripts/image-digest").to_string_lossy(), &[&format!("{}/{}:{short_tag}", crate::settings::image_namespace(repo), run.image)])
            .map_err(|e| goat_errors::coded("GB-013", format!("no digest for {}/{}:{short_tag}: {e:#}", crate::settings::image_namespace(repo), run.image)))?
    };
    run.reference = format!("ghcr.io/{{{{ ghcr_org }}}}/{}:{short_tag}@{dig}", run.image);
    run.note(format!("candidate ref {}", render(&run.reference)));

    // --- 3 dev pin + 4 dev deploy
    if matches!(o.from, From::Tag | From::Dev) {
        run.step(&format!("dev rung: {} runs {}", run.dev, run.tag));
        run.mkwt()?;
        let dev = run.dev.clone();
        let reason = format!("RC {} ({}, ADR 1173), nominated by {}: the dev-rung candidate. Validated by the live acceptance rows the change affects; their PASS is the dev->preprod validation evidence.", run.tag, utc_date(), o.ack);
        run.set_deviation(&dev, &reason)?;
        // after the pin: a record dev ran until now goes stale exactly here
        run.reconcile_superseded(&dev)?;
        run.regen_sign_check()?;
        let title = format!("chore(pins): {dev} runs release candidate {} (ADR 1173)", run.tag);
        let (line, changes) = run.changes();
        let body = format!("Release candidate {} on the dev rung ({dev}) as a declared deviation. Nominated by {}. Image built by CI from the tag; digest {dig} identical under both orgs. Validation follows on the goatnet.\n\nWhat {} carries ({line}):\n{changes}", run.tag, o.ack, run.tag);
        run.land(&title, &body)?;
        run.step(&format!("deploy {} to {dev}", run.tag));
        run.deploy(&dev, false)?;
    }

    // --- 5 rows on dev
    if !matches!(o.from, From::Preprod | From::PreprodRows | From::Prod) {
        let dev = run.dev.clone();
        run.step(&format!("live rows on {dev}"));
        if !run.rows_on(&dev)? {
            return Err(goat_errors::coded("GB-014", format!("a live row FAILED on {dev} — the candidate is dead: fix on main, cut rc.{} (delete {})", run.rcn + 1, run.tag)).into());
        }
        run.dev_evid = run.evid.clone();
    }

    // --- 6 preprod
    // A run that reaches this step with preprod already on the tag and its
    // record open (a re-run from dev after the candidate reached preprod)
    // reuses that record as --from preprod-rows does, rather than opening a
    // second record for one ref, which `promote check` then refuses (#2086).
    let mut from = o.from;
    let mut rec_found = None;
    if !matches!(from, From::Prod | From::PreprodRows) {
        rec_found = preprod_open_record(&run.root.clone(), o.component, &run.pre, &run.reference, o.dry_run)?;
        if let Some(r) = &rec_found {
            run.note(format!("{} already runs {} on main and its record {r} is open; reusing it, as --from preprod-rows does", run.pre, run.tag));
            from = From::PreprodRows;
        }
    }
    if from != From::Prod {
        let (dev, pre) = (run.dev.clone(), run.pre.clone());
        if from == From::PreprodRows {
            // Preprod already runs the tag on main (a later step failed, say a
            // live row). Find its record instead of reopening it: reopening
            // rewrites the dev evidence from a variable only the dev rows set,
            // and re-pinning lands a signature-only PR.
            run.step(&format!("preprod rung: {pre} already runs {}; resuming at its deploy check and live rows", run.tag));
            if let Some(r) = rec_found {
                run.rec = r;
            } else if o.dry_run {
                run.rec = format!("ops/release/records/{}/<date>-<ref12>.yaml", o.component);
            } else {
                let pins = read_yaml(&run.root.join("ops/pins/fleet-pins.yaml"))?;
                let pinned = pins.get("components").and_then(|c| c.get(o.component)).and_then(|c| c.get("deviations")).and_then(|d| d.get(&pre)).and_then(|d| d.get("value")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                if pinned != run.reference {
                    return Err(goat_errors::coded("GB-038", format!("{pre} does not run {} on main (it pins {}) — resume with --from preprod, which pins it", run.tag, if pinned.is_empty() { "the fleet value" } else { &pinned })).into());
                }
                let d12: String = run.reference.rsplit(':').next().unwrap_or("").chars().take(12).collect();
                let hits = promote::open_records_by_digest(&run.root, o.component, &d12);
                if hits.len() != 1 {
                    return Err(goat_errors::coded("GB-038", format!("no single open record for {} under ops/release/records/{}/ — resume with --from preprod", run.tag, o.component)).into());
                }
                run.rec = hits[0].clone();
            }
            run.note(format!("record {}", run.rec));
        } else {
        run.step(&format!("preprod rung: open the candidate and put {} on {pre}", run.tag));
        run.mkwt()?;
        let (line, changes) = run.changes();
        let reason = format!("RC {} (ADR 1173), nominated by {}; {line}; validated on {dev} by the affected live acceptance rows", run.tag, o.ack);
        let out = run.promote(&format!("open {} --ref {} --reason {reason}", o.component, run.reference), &promote::Sub::Open { component: o.component.into(), reference: run.reference.clone(), reason: reason.clone() })?;
        if o.dry_run {
            run.rec = format!("ops/release/records/{}/<date>-<ref12>.yaml", o.component);
        } else {
            run.rec = out.iter().find(|l| l.contains("record written")).and_then(|l| l.split("record written: ").nth(1)).unwrap_or("").trim().to_string();
            if run.rec.is_empty() {
                return Err(goat_errors::coded("GB-020", format!("promote open wrote no record: {}", out.join("\n"))).into());
            }
        }
        let rec_path = run.wt.join(&run.rec).display().to_string();
        let args = vec![rec_path.clone(), dev.clone(), run.tag.clone(), run.dev_evid.clone()];
        {
            let (rp, d, tag, evid) = (PathBuf::from(&rec_path), dev.clone(), run.tag.clone(), run.dev_evid.clone());
            run.edit("record-validation", &args, || {
                let ev = format!("{d}, {}: release candidate {tag} deployed as the dev-rung deviation; the affected live acceptance rows PASS ({}).", utc_local_date(), if evid.is_empty() { "no live rows mapped; cheap suites only".to_string() } else { relpath(&evid) });
                promote::record_set_evidence(&rp, "validation", &ev)
            })?;
        }
        let rec_base = Path::new(&run.rec).file_name().unwrap_or_default().to_string_lossy().to_string();
        // Once preprod runs the candidate, every other open ladder candidate for
        // the component is stale (check C6) and the landing refuses: close each
        // in this same PR, naming the new record (C8).
        let recs_base = if run.wt.join("ops/release").is_dir() { run.wt.clone() } else { run.root.clone() };
        for old in promote::stale_ladder_candidates(&recs_base, o.component, &rec_base) {
            let reason = format!("Superseded by release candidate {} on {pre} (ADR 1173), nominated by {}", run.tag, o.ack);
            run.promote(&format!("abandon {old} --reason {reason} --superseded-by {rec_base}"), &promote::Sub::Abandon { record: old.clone(), reason, superseded_by: Some(rec_base.clone()) })?;
        }
        let reason = format!("RC {} ({}, ADR 1173), validated on {dev} by the affected live acceptance rows — record {rec_base}", run.tag, utc_date());
        run.set_deviation(&pre, &reason)?;
        run.regen_sign_check()?;
        run.land(
            &format!("chore(pins): {pre} runs release candidate {}; record opened with dev evidence (ADR 1173)", run.tag),
            &format!("Preprod candidate record {rec_base} for {} with the {dev} validation evidence; {pre} deviation set. Nominated by {}.\n\nWhat {} carries ({line}):\n{changes}", run.tag, o.ack, run.tag),
        )?;
        }
        let rec_path = run.wt.join(&run.rec).display().to_string();
        let rec_base = Path::new(&run.rec).file_name().unwrap_or_default().to_string_lossy().to_string();
        run.step(&format!("deploy {} to {pre}", run.tag));
        run.deploy(&pre, false)?;
        run.step(&format!("live rows on {pre}"));
        if !run.rows_on(&pre)? {
            return Err(goat_errors::coded("GB-014", format!("a live row FAILED on {pre} after the dev PASS — leave the candidate open, investigate; graduate.py refuses without convergence evidence")).into());
        }
        run.mkwt()?;
        let args = vec![rec_path.clone(), pre.clone(), run.tag.clone(), run.evid.clone()];
        {
            let (rp, p, tag, evid) = (PathBuf::from(&rec_path), pre.clone(), run.tag.clone(), run.evid.clone());
            run.edit("record-convergence", &args, || {
                let ev = format!("{p}, {}: deploy-component.sh converged every host's ledger on {tag}; the affected live rows PASS on preprod too ({}).", utc_local_date(), if evid.is_empty() { "no live rows mapped".to_string() } else { relpath(&evid) });
                promote::record_set_evidence(&rp, "preprod_convergence", &ev)
            })?;
        }
        run.promote("check", &promote::Sub::Check)?;
        run.land(
            &format!("chore(release): {} converged on {pre} with the live rows PASS (ADR 1173)", run.tag),
            &format!("Preprod convergence and validation evidence for {} recorded on {rec_base}.", run.tag),
        )?;

        // --- 7 soak
        run.step("soak");
        let mind = promote::soak_min_days(&run.root);
        run.note(format!("{} is the preprod candidate. After {mind} day(s), with an operator ack:", run.tag));
        run.note(format!("  goatbuilder release {} --tag {} --from prod --ack '<name>'    # graduate, fold into fleet:, deploy {}, verify every rung", o.component, run.tag, run.prod));
    }

    // --- 8 prod
    if o.from == From::Prod {
        let (dev, pre, prod) = (run.dev.clone(), run.pre.clone(), run.prod.clone());
        run.step(&format!("prod rung: graduate {} and fold it into fleet:", run.tag));
        run.mkwt()?;
        let out = run.promote(&format!("graduate {} --ack '{}'  (refuses until the soak gate is met)", o.component, o.ack), &promote::Sub::Graduate { component: o.component.into(), ack: Some(o.ack.into()) })?;
        if o.dry_run {
            run.rec = format!("ops/release/records/{}/<the open candidate record>.yaml", o.component);
        } else {
            run.rec = out.iter().find_map(|l| l.strip_prefix("record graduated: ")).unwrap_or("").trim().to_string();
            if run.rec.is_empty() {
                return Err(anyhow!("{}", out.join("\n")));
            }
            run.note(format!("graduated: {}", run.rec));
            let text = std::fs::read_to_string(run.wt.join(&run.rec)).unwrap_or_default();
            if !text.lines().any(|l| l == format!("candidate_ref: {}", run.reference)) {
                return Err(goat_errors::coded("GB-020", format!("{} names a different candidate_ref than {} resolves to ({})", run.rec, run.tag, run.reference)).into());
            }
        }
        let rec_base = Path::new(&run.rec).file_name().unwrap_or_default().to_string_lossy().to_string();
        let pins_wt = run.wt.join("ops/pins/fleet-pins.yaml");
        // Fold only the pins that carried THIS candidate (rc.sh step 8): a value
        // deviation for any other ref is a newer candidate already on its rung.
        let pins_src = if pins_wt.is_file() { pins_wt.clone() } else { run.root.join("ops/pins/fleet-pins.yaml") };
        let recs_root = if run.wt.join("ops/release").is_dir() { run.wt.clone() } else { run.root.clone() };
        let hot = promote::open_hotfix_refs(&recs_root, o.component);
        let (gs, kept) = pins::fold_rungs(&pins_src, o.component, &run.reference, &prod, &hot, &[dev.clone(), pre.clone(), prod.clone()])?;
        for (g, v) in &kept {
            run.note(format!("keeps {g}'s deviation {v}: it carries a different candidate"));
        }
        let mut args = vec![pins_wt.display().to_string(), o.component.to_string(), run.reference.clone(), run.fleet.clone(), run.tag.clone(), rec_base.clone()];
        args.extend(gs.iter().cloned());
        {
            let (comp, reference, fleet, tag, rb) = (o.component.to_string(), run.reference.clone(), run.fleet.clone(), run.tag.clone(), rec_base.clone());
            run.edit("fold-fleet", &args, || pins::edit_fold_fleet(&pins_wt, &comp, &reference, &fleet, &tag, &rb, &gs))?;
        }
        let hot: Vec<String> = if o.dry_run { Vec::new() } else { promote::hotfix_candidates_with_ref(&run.wt, o.component, &run.fleet) };
        for h in hot {
            run.note(format!("hotfix record {h} carried the fleet value {} replaces; abandoning as superseded", run.tag));
            let reason = format!("superseded: its ref was the fleet value until {} graduated ({rec_base}); the backfill it owed is this graduation", run.tag);
            run.promote(&format!("abandon {h} --reason {reason} --superseded-by {rec_base}"), &promote::Sub::Abandon { record: h.clone(), reason, superseded_by: Some(rec_base.clone()) })?;
        }
        run.regen_sign_check()?;
        run.land(
            &format!("chore(release): {} graduates; fleet runs it and {prod} folds (ADR 1120/1173)", run.tag),
            &format!("Candidate record {rec_base} graduated with the operator ack of {} after the soak on {pre}. fleet: is now {}; the {dev}, {pre} and {prod} deviations that carried the candidate (or the hotfix it replaces) are deleted, so every rung reads one value. Pins and intents regenerated and re-signed; graduate.py check and generate-pins --check pass.", o.ack, run.reference),
        )?;
        run.step(&format!("deploy {} to {prod}", run.tag));
        run.deploy(&prod, false)?;
        run.step(&format!("every rung reads {}", run.tag));
        for g in [dev.clone(), pre.clone()] {
            run.deploy(&g, true)?;
        }
        if o.dry_run {
            run.note(format!("{}: DRY: a real run puts fleet {} on {dev}, {pre} and {prod} and reads every ledger", o.component, run.reference));
        } else {
            run.note(format!("{}: fleet {} on {dev}, {pre} and {prod} (ledger-verified)", o.component, run.reference));
        }
    }
    if let Some(ov) = &run.overlay {
        // The scratch tree is a plain directory; the overlay worktree is clean
        // once its PR merged, and its branch stays for the reflog.
        if !o.dry_run {
            let _ = std::fs::remove_dir_all(&run.wt);
            if ov.owt.is_dir() {
                let _ = sh("git", &["-C", &ov.path.display().to_string(), "worktree", "remove", &ov.owt.display().to_string()]);
            }
        }
    } else if run.wt.is_dir() && !o.dry_run {
        let _ = sh("git", &["-C", &repo_s, "worktree", "remove", run.wt.to_str().unwrap_or(".")]);
    }
    let mut report = Report::new("release", run.checks.take(), EXIT_REFUSED);
    report.exit = EXIT_OK;
    report.ok = true;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_finds_the_open_record_only_when_preprod_already_pins_the_ref() {
        let d = std::env::temp_dir().join(format!("goatbuilder-release-{}-reuse", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("ops/pins")).unwrap();
        std::fs::create_dir_all(d.join("ops/release/records/comp")).unwrap();
        let r = "ghcr.io/o/comp:v1-rc.1@sha256:cccccccccccc0000";
        std::fs::write(d.join("ops/pins/fleet-pins.yaml"), format!("components:\n  comp:\n    fleet: old\n    deviations:\n      pre:\n        value: '{r}'\n")).unwrap();
        let rec = |f: &str, state: &str| std::fs::write(d.join("ops/release/records/comp").join(f), format!("component: comp\nclass: ladder\ncandidate_ref: '{r}'\nstate: {state}\nopened: '2026-10-28'\n")).unwrap();
        rec("20261028-cccccccccccc.yaml", "candidate");
        // preprod pins the ref and one record is open: it is reused
        assert_eq!(preprod_open_record(&d, "comp", "pre", r, false).unwrap(), Some("ops/release/records/comp/20261028-cccccccccccc.yaml".to_string()));
        // another rung, another ref, or no ref: nothing to reuse, the step opens one
        assert_eq!(preprod_open_record(&d, "comp", "dev", r, false).unwrap(), None);
        assert_eq!(preprod_open_record(&d, "comp", "pre", "ghcr.io/o/comp:v1-rc.2@sha256:eeee", false).unwrap(), None);
        assert_eq!(preprod_open_record(&d, "comp", "pre", "", false).unwrap(), None);
        // a dry run has no digest: the image and tag decide, the pinned digest names the record
        assert_eq!(preprod_open_record(&d, "comp", "pre", "ghcr.io/o/comp:v1-rc.1@sha256:<digest of comp:v1-rc.1>", true).unwrap(), Some("ops/release/records/comp/20261028-cccccccccccc.yaml".to_string()));
        assert_eq!(preprod_open_record(&d, "comp", "pre", "ghcr.io/o/comp:v1-rc.1@sha256:<digest of comp:v1-rc.1>", false).unwrap(), None);
        // the record closed: nothing open to reuse
        rec("20261028-cccccccccccc.yaml", "abandoned");
        assert_eq!(preprod_open_record(&d, "comp", "pre", r, false).unwrap(), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_resume_step_parses_and_an_unknown_one_is_refused_naming_the_set() {
        for (s, want) in [("tag", From::Tag), ("dev", From::Dev), ("rows", From::Rows), ("preprod", From::Preprod), ("preprod-rows", From::PreprodRows), ("prod", From::Prod)] {
            assert!(From::parse(s).unwrap() == want, "{s}");
        }
        let e = From::parse("staging").unwrap_err().to_string();
        assert!(e.contains("tag|dev|rows|preprod|preprod-rows|prod"), "{e}");
    }

    #[test]
    fn every_step_parses_to_a_distinct_variant() {
        let all: Vec<From> = ["tag", "dev", "rows", "preprod", "preprod-rows", "prod"].iter().map(|s| From::parse(s).unwrap()).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(a != b);
            }
        }
    }

    #[test]
    fn render_never_leaves_the_org_template_in_a_printed_ref() {
        if !crate::config::checkout_declares_a_deployment() {
            return; // a public clone of goatcore ships no deployment
        }
        let r = render("ghcr.io/{{ ghcr_org }}/netvis:sha-1@sha256:aa");
        assert!(!r.contains("{{"), "{r}");
        assert!(r.starts_with("ghcr.io/") && r.ends_with("/netvis:sha-1@sha256:aa"), "{r}");
    }

    #[test]
    fn relpath_under_the_cwd_is_relative_and_outside_climbs() {
        let cwd = std::env::current_dir().unwrap();
        let inside = cwd.join("a/b.txt");
        assert_eq!(relpath(inside.to_str().unwrap()), "a/b.txt");
        let parent = cwd.parent().unwrap().join("sibling/x.json");
        assert_eq!(relpath(parent.to_str().unwrap()), "../sibling/x.json");
    }

    #[test]
    fn utc_date_is_iso_and_matches_the_push_stamp() {
        let d = utc_date();
        assert_eq!(d.len(), 10, "{d}");
        assert_eq!(&d[4..5], "-");
        assert_eq!(&d[7..8], "-");
        assert!(d[0..4].parse::<u32>().unwrap() >= 2026);
    }
}
