//! `goatbuilder image <model>`: a model's golden path, one numbered step at a
//! time (docs/design/goatbuilder-build-targets.md sections 3.1 and 3.5). It
//! wraps the model's own builder, here `ops/rpi-base-image/` for `rpi5`, and
//! does not replace it: each step runs the builder's script for that step, or,
//! where the step needs a person (the card, the console during encryption, the
//! YubiKey), prints the builder's directions. Every step's result is then read
//! and proven before the next step runs.
//!
//! - With no `--step`, every proof that can be read is read, and the next
//!   step is named. Nothing is changed.
//! - `--step <name>` checks that the step before it is proven, runs the step,
//!   and reads its proof. A step that ends in a reboot prints what to do, and
//!   its proof is read with `--step <name> --check` once the machine is back.
//! - `--dry-run` prints the steps, what each runs and what proves it.
//!
//! The machine's proofs are what `doctor --target --live` reads on a small
//! computer (`small_computer.rs`): the same collector, run over SSH as root
//! with `sudo -n`, and the same judge.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Result;
use serde_json::Value;

use crate::runner::Cmd;
use crate::small_computer;
use crate::{Check, Report};

const BUILDER: &str = "ops/rpi-base-image";
/// The EEPROM boot order 02-clone-to-nvme.sh sets.
const BOOT_ORDER: &str = "0xf416";

/// Where a step runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// This machine, the operator's workstation.
    Workstation,
    /// The Pi, over SSH, as root.
    Machine,
    /// A person, by the builder's directions.
    Person,
    /// goatbuilder itself: it reads, and runs nothing.
    Reads,
}

impl Place {
    fn word(self) -> &'static str {
        match self {
            Place::Workstation => "workstation",
            Place::Machine => "machine",
            Place::Person => "a person",
            Place::Reads => "goatbuilder reads",
        }
    }
}

/// What proves a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proof {
    /// The golden image is here with its pinned SHA-256, and user-data names a key.
    Image,
    /// `01b-verify-image.sh` reads the card back and it holds.
    Card,
    /// The machine answers over SSH, and its first boot finished.
    Reached,
    /// The root's disk is the model's (NVMe).
    Cloned,
    /// The boot is set to unlock a LUKS root: `root=/dev/mapper/...`, `cryptdevice=`, cryptsetup in the initramfs.
    Prepared,
    /// The initramfs runs dropbear (the headless way in).
    Headless,
    /// These rows of the model's end state hold.
    Rows(&'static [&'static str]),
    /// The unlock row holds, with the YubiKey among its ways in.
    Yubikey,
    /// Every row of the model's end state holds.
    All,
}

impl Proof {
    fn says(self) -> String {
        match self {
            Proof::Image => "the golden image is here with its pinned SHA-256, and user-data carries an SSH key".into(),
            Proof::Card => "01b-verify-image.sh reads the card back: the image's partition table and root partition, and this checkout's boot files".into(),
            Proof::Reached => "the machine answers over SSH as the key's user, sudo works, and cloud-init's first boot finished".into(),
            Proof::Cloned => format!("the root's disk is the model's, and the boot order is {BOOT_ORDER}, which 02 sets once it has cloned"),
            Proof::Prepared => "the next boot's command line (/boot/firmware/cmdline.txt) boots root=/dev/mapper/<name> with cryptdevice=, and the initramfs has cryptsetup".into(),
            Proof::Headless => "the initramfs the board boots runs dropbear".into(),
            Proof::Rows(r) => format!("the machine's {} rows hold", r.join(" and ")),
            Proof::Yubikey => "the unlock row holds with the YubiKey's keyscript, its tools in the initramfs and a second keyslot; whether the YubiKey unlocked a boot is not read: the person's reboot shows it".into(),
            Proof::All => "every row of the model's end state holds (doctor --target --live's small-computer rows)".into(),
        }
    }

    fn needs_machine(self) -> bool {
        !matches!(self, Proof::Image | Proof::Card)
    }
}

/// One numbered step of the builder.
#[derive(Debug, Clone)]
struct Step {
    /// The builder's number.
    number: &'static str,
    name: &'static str,
    place: Place,
    /// The builder's script, under the builder's directory.
    script: Option<&'static str>,
    /// What a person does, in the builder's words (ops/rpi-base-image/README.md).
    directions: &'static str,
    /// The step ends in a reboot a person makes (and unlocks).
    reboot: Option<&'static str>,
    proof: Proof,
}

/// How the root is to unlock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Unlock {
    /// A YubiKey challenge-response (04-enroll-yubikey.sh), the passphrase kept for recovery.
    Yubikey,
    /// The passphrase alone, typed at boot.
    Passphrase,
}

pub struct Options<'a> {
    pub model: &'a str,
    pub step: Option<&'a str>,
    pub check: bool,
    pub host: Option<&'a str>,
    pub device: Option<&'a str>,
    pub image: Option<&'a Path>,
    pub unlock: Unlock,
    pub headless: bool,
    pub dry_run: bool,
}

/// The rpi5 builder's steps, in order, for this unlock and console.
fn steps(o: &Options) -> Vec<Step> {
    let mut s = vec![
        Step { number: "00", name: "prerequisites", place: Place::Workstation, script: Some("00-prerequisites.sh"), directions: "", reboot: None, proof: Proof::Image },
        Step { number: "01", name: "flash", place: Place::Workstation, script: Some("01-flash.sh"), directions: "", reboot: None, proof: Proof::Card },
        // Its script is its proof: goatbuilder runs it to read the card, once.
        Step { number: "01b", name: "verify-image", place: Place::Reads, script: None, directions: "", reboot: None, proof: Proof::Card },
        Step {
            number: "-",
            name: "boot",
            place: Place::Person,
            script: None,
            directions: "put the card in the Pi and the NVMe drive in its HAT, power it on on a wired network, wait 3 to 5 minutes for cloud-init, and find its address (your router, or nmap -sn on the subnet)",
            reboot: None,
            proof: Proof::Reached,
        },
        Step { number: "02", name: "clone", place: Place::Machine, script: Some("02-clone-to-nvme.sh"), directions: "", reboot: Some("on its first pass 02 applies the NVMe power fix and reboots the Pi itself: once it is back, run this step again. After the pass that clones, shut the Pi down, take the SD card out, and power it on: it boots from NVMe"), proof: Proof::Cloned },
        Step { number: "03", name: "prepare-encryption", place: Place::Machine, script: Some("03-prepare-encryption.sh"), directions: "", reboot: None, proof: Proof::Prepared },
    ];
    if o.headless {
        s.push(Step {
            number: "03b",
            name: "headless-unlock",
            place: Place::Person,
            script: Some("03b-headless-dropbear-unlock.sh"),
            directions: "on the Pi: sudo CIP=<its address> GW=<gateway> MASK=<netmask> NIC=eth0 PUBKEY=\"$(cat ~/.ssh/id_ed25519.pub)\" bash /boot/firmware/03b-headless-dropbear-unlock.sh, and do not reboot until it prints \"Safe to reboot\" (its values are this site's, so goatbuilder prints the command and does not run it)",
            reboot: None,
            proof: Proof::Headless,
        });
    }
    s.push(Step {
        number: "03",
        name: "encrypt",
        place: Place::Person,
        script: None,
        directions: if o.headless {
            "sudo reboot; once the Pi waits in its initramfs: ssh root@<its address>, mount -t vfat /dev/nvme0n1p1 /mnt/boot, sh /mnt/boot/helper-scripts/encrypt-rootfs.sh (passphrase mode; it takes about 40 minutes for a small drive, hours for a large one: do not power off), then unlock with the passphrase after its reboot"
        } else {
            "sudo reboot; at the HDMI console and USB keyboard, in busybox: mkdir -p /mnt/boot && mount -t vfat /dev/nvme0n1p1 /mnt/boot, sh /mnt/boot/encrypt-rootfs.sh, set the passphrase (it stays as the recovery slot), wait (about 40 minutes for a small drive: do not power off), and after its reboot: cryptsetup luksOpen /dev/nvme0n1p2 rootfs, exit"
        },
        reboot: None,
        proof: Proof::Rows(&["root", "primitives"]),
    });
    if o.unlock == Unlock::Yubikey {
        s.push(Step {
            number: "04",
            name: "yubikey",
            place: Place::Machine,
            script: Some("04-enroll-yubikey.sh"),
            directions: "the YubiKey is plugged into the Pi; touch it when it blinks",
            reboot: Some("reboot the Pi and touch the YubiKey when it blinks: it unlocks without the passphrase"),
            proof: Proof::Yubikey,
        });
    }
    s.push(Step { number: "05", name: "fips", place: Place::Machine, script: Some("05-enable-fips.sh"), directions: "", reboot: Some("reboot the Pi and unlock it"), proof: Proof::Rows(&["fips", "ssh"]) });
    s.push(Step { number: "06", name: "verify-box", place: Place::Reads, script: None, directions: "", reboot: None, proof: Proof::All });
    s
}

/// The model's golden image, from its entry's `os.golden_image`.
#[derive(Debug, Clone)]
struct Golden {
    file: String,
    sha256: String,
    url: String,
}

/// GB-411: no builder goatbuilder wraps for this model.
fn no_builder(why: String) -> anyhow::Error {
    goat_errors::coded("GB-411", format!("goatbuilder image cannot build this model: {why}")).into()
}

/// GB-412: a step refused before it ran.
fn refused(step: &str, why: String) -> anyhow::Error {
    goat_errors::coded("GB-412", format!("image step {step} did not run: {why}")).into()
}

/// The model's entry in its family's file: its builder must be the one
/// wrapped here, and its golden image pinned. With the family's name.
fn golden(repo: &Path, model: &str) -> Result<(String, Golden)> {
    let targets = crate::targets::load(repo)?;
    let mut known = Vec::new();
    let mut wrapped = Vec::new();
    let mut found = None;
    for f in targets.family_names() {
        if let Some(crate::targets::Models::Described(entries)) = targets.models(f) {
            for (name, m) in entries {
                known.push(name.clone());
                if m.builder.trim_end_matches('/') == BUILDER {
                    wrapped.push(name.clone());
                }
                if name == model {
                    found = Some((f.to_string(), m.clone()));
                }
            }
        }
    }
    let Some((family, m)) = found else {
        return Err(no_builder(format!("'{model}' is not a model with an entry in ops/goatbuilder/families/; known: {}", known.join(", "))));
    };
    if m.builder.trim_end_matches('/') != BUILDER {
        return Err(no_builder(format!(
            "its builder is '{}', which goatbuilder does not wrap yet; build it by that builder's own steps, then check it with doctor --target. Wrapped: {}",
            m.builder,
            wrapped.join(", ")
        )));
    }
    let Some(g) = m.os.golden_image else {
        return Err(no_builder(format!("ops/goatbuilder/families/{family}.yaml's {model} has no os.golden_image")));
    };
    Ok((family, Golden { file: g.file, sha256: g.sha256, url: g.url }))
}

/// Where the golden image is: `--image`, else where 00-prerequisites.sh
/// keeps it: `$IMAGES_DIR`, `ops/artifacts/images`, `ops/images`, `~/Downloads`.
fn image_path(repo: &Path, o: &Options, g: &Golden) -> Option<PathBuf> {
    if let Some(p) = o.image {
        return Some(p.to_path_buf());
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("IMAGES_DIR").map(PathBuf::from).into_iter().collect();
    dirs.push(repo.join("ops/artifacts/images"));
    dirs.push(repo.join("ops/images"));
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(h).join("Downloads"));
    }
    dirs.into_iter().map(|d| d.join(&g.file)).find(|p| p.is_file())
}

/// Text without its terminal colour codes.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' && it.peek() == Some(&'[') {
            for d in it.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}

/// The machine's facts, read over SSH as root: `ssh <host> sudo -n python3 -`.
fn machine_facts(repo: &Path, host: &str) -> std::result::Result<Value, String> {
    let script = std::fs::File::open(repo.join(small_computer::SCRIPT)).map_err(|e| format!("{}: {e}", small_computer::SCRIPT))?;
    let out = Cmd::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", host, "sudo", "-n", "python3", "-"])
        .stdin(Stdio::from(script))
        .output()
        .map_err(|e| format!("could not start ssh: {e}"))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        return Err(format!("ssh {host} answered {}: {}", out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into()), said.lines().last().unwrap_or("").trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("{} did not print its facts: {e}", small_computer::SCRIPT))
}

/// A proof's verdict: holds, does not hold (why), or cannot be read now (why).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    Holds(String),
    Fails(String),
    Unread(String),
}

/// The machine's rows, by name.
fn rows(model: &str, d: &small_computer::Delivers, f: &Value) -> Vec<small_computer::Row> {
    small_computer::judge(model, d, f)
}

fn of_rows(model: &str, d: &small_computer::Delivers, f: &Value, want: &[&str]) -> Verdict {
    let all = rows(model, d, f);
    let picked: Vec<&small_computer::Row> = all.iter().filter(|r| want.is_empty() || want.contains(&r.name)).collect();
    if let Some(w) = want.iter().find(|w| !all.iter().any(|r| r.name == **w)) {
        return Verdict::Unread(format!("the end state has no row '{w}'"));
    }
    let bad: Vec<String> = picked.iter().filter(|r| !r.ok).map(|r| format!("{}: {}", r.name, r.detail)).collect();
    if bad.is_empty() {
        Verdict::Holds(picked.iter().map(|r| format!("{}: {}", r.name, r.detail)).collect::<Vec<_>>().join("; "))
    } else {
        Verdict::Fails(bad.join("; "))
    }
}

/// The golden image, held to its pin: its path, or the verdict that it is not.
fn pinned(repo: &Path, o: &Options, g: &Golden) -> std::result::Result<PathBuf, Verdict> {
    let Some(p) = image_path(repo, o, g) else {
        return Err(Verdict::Unread(format!("no {} here (--image, $IMAGES_DIR, ops/artifacts/images, ops/images or ~/Downloads): 00-prerequisites.sh downloads it from {}", g.file, g.url)));
    };
    match sha256_file(&p) {
        Ok(h) if h == g.sha256 => Ok(p),
        Ok(h) => Err(Verdict::Fails(format!("{} is sha256 {h}, and the model pins {}: remove it, and 00-prerequisites.sh downloads it again", p.display(), g.sha256))),
        Err(e) => Err(Verdict::Unread(format!("{}: {e}", p.display()))),
    }
}

/// Read one step's proof, with what is at hand.
fn prove(repo: &Path, o: &Options, g: &Golden, model: &str, d: &small_computer::Delivers, step: &Step, facts: &mut Option<std::result::Result<Value, String>>) -> Verdict {
    match step.proof {
        Proof::Image => {
            let p = match pinned(repo, o, g) {
                Ok(p) => p,
                Err(v) => return v,
            };
            let ud = repo.join(BUILDER).join("cloud-init-configs/user-data");
            match std::fs::read_to_string(&ud) {
                Ok(t) if t.contains("REPLACE_WITH_YOUR_PUBLIC_KEY") => Verdict::Fails(format!("{} still has the SSH key placeholder: 00-prerequisites.sh puts your key there", ud.display())),
                Ok(_) => Verdict::Holds(format!("{} (sha256 {}), and user-data carries a key", p.display(), &g.sha256[..16])),
                Err(e) => Verdict::Unread(format!("{}: {e}", ud.display())),
            }
        }
        Proof::Card => {
            let Some(dev) = o.device else {
                return Verdict::Unread("needs --device <the card>".into());
            };
            // The card is held to the pinned image, never to whatever image is named.
            let img = match pinned(repo, o, g) {
                Ok(p) => p,
                Err(v) => return v,
            };
            let out = Cmd::new("bash").arg(repo.join(BUILDER).join("01b-verify-image.sh")).arg(dev).arg(&img).stdin(Stdio::inherit()).output();
            match out {
                Ok(x) if x.status.code() == Some(0) => Verdict::Holds(format!("{dev} holds {} and this checkout's boot files", g.file)),
                Ok(x) if x.status.code() == Some(1) => Verdict::Fails(plain(&String::from_utf8_lossy(&x.stderr)).lines().map(|l| l.trim().trim_start_matches("[FAIL] ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("; ")),
                Ok(x) => Verdict::Unread(plain(&String::from_utf8_lossy(&x.stderr)).lines().last().unwrap_or("01b-verify-image.sh could not run").trim_start_matches("[error] ").to_string()),
                Err(e) => Verdict::Unread(format!("could not start bash: {e}")),
            }
        }
        _ if o.host.is_none() => Verdict::Unread("needs --host <user@address>, as ssh reaches the Pi".into()),
        p => {
            let f = facts.get_or_insert_with(|| machine_facts(repo, o.host.unwrap_or_default()));
            let f = match f {
                Ok(f) => f.clone(),
                Err(e) => return Verdict::Unread(e.clone()),
            };
            match p {
                Proof::Reached => of_rows(model, d, &f, &["first boot"]),
                Proof::Cloned => {
                    // 02 sets the boot order at the end of the pass that clones
                    // (its first pass applies the NVMe power fix and reboots).
                    let disk = f["root"]["chain"].as_array().and_then(|c| c.last()).and_then(|x| x["name"].as_str()).unwrap_or("").to_string();
                    let order = f["boot_order"].as_str().map(String::from);
                    if !disk.contains(&format!("/{}", d.root_media)) {
                        Verdict::Fails(format!("the root's disk is {}, not {}", if disk.is_empty() { "not read" } else { &disk }, d.root_media))
                    } else if order.as_deref() != Some(BOOT_ORDER) {
                        match order {
                            Some(o) => Verdict::Fails(format!("the root's disk is {disk}, and the boot order is {o}, not {BOOT_ORDER}, which 02 sets once it has cloned")),
                            None => Verdict::Unread(format!("the root's disk is {disk}, and the boot order: {}", f["boot_order"]["error"].as_str().unwrap_or("not read"))),
                        }
                    } else {
                        Verdict::Holds(format!("the root's disk is {disk}, and the boot order {BOOT_ORDER} (02's) tries the SD card first, then NVMe; whether the NVMe held an install before 02 is not read"))
                    }
                }
                Proof::Prepared => {
                    // The next boot's: 03 changes cmdline.txt, and the running boot is the old one.
                    let words: Vec<&str> = f["cmdline_next"].as_array().map(|a| a.iter().filter_map(|w| w.as_str()).collect()).unwrap_or_default();
                    let mut missing = Vec::new();
                    if !words.iter().any(|w| w.starts_with("root=/dev/mapper/")) {
                        missing.push("root=/dev/mapper/<name>".to_string());
                    }
                    if !words.iter().any(|w| w.starts_with("cryptdevice=")) {
                        missing.push("cryptdevice=".to_string());
                    }
                    if f["initramfs"]["cryptsetup"] != true {
                        missing.push("cryptsetup".to_string());
                    }
                    if missing.is_empty() {
                        Verdict::Holds(format!("{}; cryptsetup in {}", words.join(" "), f["initramfs"]["file"].as_str().unwrap_or("the initramfs")))
                    } else {
                        Verdict::Fails(format!("no {} in /boot/firmware/cmdline.txt and the initramfs", missing.join(", no ")))
                    }
                }
                Proof::Headless => {
                    if f["initramfs"]["dropbear"] == true {
                        Verdict::Holds("the initramfs runs dropbear".into())
                    } else {
                        Verdict::Fails("the initramfs has no dropbear".into())
                    }
                }
                Proof::Rows(want) => of_rows(model, d, &f, want),
                Proof::Yubikey => match of_rows(model, d, &f, &["unlock"]) {
                    Verdict::Holds(h) if h.contains("yubikey-hmac") => Verdict::Holds(h),
                    Verdict::Holds(h) => Verdict::Fails(format!("the YubiKey is not one of the ways in: {h}")),
                    v => v,
                },
                Proof::All => of_rows(model, d, &f, &[]),
                Proof::Image | Proof::Card => unreachable!(),
            }
        }
    }
}

fn row(step: &Step, v: &Verdict) -> Check {
    let name = format!("step {} {}", step.number, step.name);
    match v {
        Verdict::Holds(d) => Check { name, ok: true, detail: format!("holds: {d}"), fix: None },
        Verdict::Fails(d) => Check { name, ok: false, detail: format!("does not hold: {d}"), fix: None },
        Verdict::Unread(d) => Check { name, ok: false, detail: format!("unknown: {d}"), fix: None },
    }
}

/// The command a step runs, as it is run.
fn command(repo: &Path, o: &Options, g: &Golden, step: &Step) -> Option<String> {
    let script = step.script?;
    let img = image_path(repo, o, g).map(|p| p.display().to_string()).unwrap_or_else(|| format!("<{}>", g.file));
    let dev = o.device.unwrap_or("<--device>");
    let host = o.host.unwrap_or("<--host>");
    let local = repo.join(BUILDER).join(script).display().to_string();
    Some(match (step.place, step.name) {
        (Place::Workstation, "prerequisites") => local,
        (Place::Workstation, _) => format!("{local} {dev} {img}"),
        (Place::Machine, _) => format!("ssh -t {host} sudo bash /boot/firmware/{script}"),
        _ => return None,
    })
}

fn plan(repo: &Path, o: &Options, g: &Golden, all: &[Step]) -> Vec<Check> {
    all.iter()
        .map(|s| {
            let does = match (s.place, command(repo, o, g, s)) {
                (Place::Person, _) => format!("directions: {}", if s.directions.is_empty() { "the builder's" } else { s.directions }),
                (Place::Reads, _) if s.proof == Proof::Card => "reads the card back with 01b-verify-image.sh <--device> <image>, and writes nothing".into(),
                (Place::Reads, _) => "reads the machine, and changes nothing".into(),
                (_, Some(c)) => format!("runs: {c}{}", if s.directions.is_empty() { String::new() } else { format!(" ({})", s.directions) }),
                (_, None) => String::new(),
            };
            let reboot = s.reboot.map(|r| format!("; then a person: {r}")).unwrap_or_default();
            Check { name: format!("step {} {}", s.number, s.name), ok: true, detail: format!("not checked: plan, {}: {does}{reboot}; proven by: {}", s.place.word(), s.proof.says()), fix: None }
        })
        .collect()
}

/// The step before `i` whose proof stands before step `i` may run.
fn before(all: &[Step], i: usize) -> Option<&Step> {
    let me = &all[i];
    // flash and verify-image both stand on the prerequisites.
    all[..i].iter().rev().find(|s| !(me.proof == Proof::Card && s.proof == Proof::Card))
}

fn the_command(o: &Options, step: &str, check: bool) -> String {
    let mut c = format!("goatbuilder image {} --step {step}", o.model);
    if check {
        c.push_str(" --check");
    }
    if let Some(h) = o.host {
        c.push_str(&format!(" --host {}", quoted(h)));
    }
    if let Some(d) = o.device {
        c.push_str(&format!(" --device {}", quoted(d)));
    }
    if let Some(i) = o.image {
        c.push_str(&format!(" --image {}", quoted(&i.display().to_string())));
    }
    if o.unlock == Unlock::Passphrase {
        c.push_str(" --unlock passphrase");
    }
    if o.headless {
        c.push_str(" --headless");
    }
    c
}

/// A value that is passed on to ssh or a script as an argument: never one
/// that reads as an option.
fn checked(step: &str, what: &str, v: Option<&str>, allowed: impl Fn(char) -> bool) -> Result<()> {
    match v {
        Some(x) if x.is_empty() || x.starts_with('-') || !x.chars().all(&allowed) => Err(refused(
            step,
            format!("--{what} '{x}' is not a {what}: it may not start with '-', and holds only letters, digits and {}", if what == "host" { "@ . _ - : [ ]" } else { "/ . _ - and +" }),
        )),
        _ => Ok(()),
    }
}

/// A word as a shell reads it.
fn quoted(v: &str) -> String {
    if v.chars().all(|c| c.is_ascii_alphanumeric() || "@%_+=:,./-".contains(c)) {
        v.to_string()
    } else {
        format!("'{}'", v.replace('\'', "'\\''"))
    }
}

pub fn run(repo: &Path, o: &Options) -> Result<Report> {
    let step = o.step.unwrap_or("(status)");
    checked(step, "host", o.host, |c| c.is_ascii_alphanumeric() || "@._-:[]".contains(c))?;
    checked(step, "device", o.device, |c| c.is_ascii_alphanumeric() || "/._-+".contains(c))?;
    let (family, g) = golden(repo, o.model)?;
    let (model, d) = small_computer::model(crate::targets::load(repo)?.models(&family))?;
    let all = steps(o);
    let mut facts: Option<std::result::Result<Value, String>> = None;
    if o.dry_run && o.step.is_none() {
        return Ok(Report::new("image", plan(repo, o, &g, &all), crate::EXIT_REFUSED));
    }
    let Some(name) = o.step else {
        // Every proof that can be read, and the next step.
        let mut checks = Vec::new();
        let mut next = None;
        for s in &all {
            if s.proof.needs_machine() && o.host.is_none() && next.is_some() {
                checks.push(Check { name: format!("step {} {}", s.number, s.name), ok: true, detail: "not checked: needs --host <user@address>".into(), fix: None });
                continue;
            }
            let v = prove(repo, o, &g, &model, &d, s, &mut facts);
            if next.is_none() && !matches!(v, Verdict::Holds(_)) {
                next = Some(s.name);
            }
            checks.push(row(s, &v));
        }
        // Once the machine has booted from the card, the card is in the Pi and
        // the image may be gone: the workstation's proofs are not read, and the
        // next step is the first of the machine's that does not hold.
        let machine_read = all.iter().zip(&checks).any(|(s, c)| s.proof.needs_machine() && c.detail.starts_with("holds"));
        if machine_read {
            for (s, c) in all.iter().zip(checks.iter_mut()) {
                // Only what cannot be read now: a proof that fails stays red.
                if !s.proof.needs_machine() && c.detail.starts_with("unknown") {
                    c.ok = true;
                    let why = c.detail.split_once(": ").map(|(_, w)| w).unwrap_or(&c.detail).to_string();
                    c.detail = format!("not checked: the machine has booted from the card since ({why})");
                }
            }
            next = all.iter().zip(&checks).find(|(s, c)| s.proof.needs_machine() && !c.detail.starts_with("holds")).map(|(s, _)| s.name);
        }
        let done = checks.last().is_some_and(|c| c.ok && c.detail.starts_with("holds"));
        checks.push(Check {
            name: "next".into(),
            ok: done,
            detail: match next {
                _ if done => {
                    let ver = facts.as_ref().and_then(|f| f.as_ref().ok()).and_then(|f| f["os"]["version_id"].as_str()).unwrap_or("").to_string();
                    let built = if d.os_builder.contains(&ver) { String::new() } else { format!(" (Ubuntu {ver}: these steps install {} only, so this machine was not built by them)", d.os_builder.join(", ")) };
                    format!("none: the {model} is verified{built}; next is the host baseline (join-node, build-targets section 3.3 step 3)")
                }
                Some(n) => the_command(o, n, false),
                None => "none".into(),
            },
            fix: None,
        });
        return Ok(Report::new("image", checks, crate::EXIT_NOT_CONVERGED));
    };
    let names: Vec<&str> = all.iter().map(|s| s.name).collect();
    let Some(i) = all.iter().position(|s| s.name == name) else {
        return Err(no_builder(format!("the {model} builder has no step '{name}' here; its steps: {}", names.join(", "))));
    };
    let step = &all[i];
    if o.dry_run {
        return Ok(Report::new("image", plan(repo, o, &g, std::slice::from_ref(step)), crate::EXIT_REFUSED));
    }
    let mut ran = false;
    if !o.check {
        // Its inputs.
        if (step.proof == Proof::Card || step.name == "flash") && o.device.is_none() {
            return Err(refused(name, "it writes or reads the card: name it with --device <the whole disk> (01-flash.sh --dry-run shows what it would do)".into()));
        }
        let needs_host = step.proof.needs_machine() || before(&all, i).is_some_and(|b| b.proof.needs_machine());
        if needs_host && o.host.is_none() {
            return Err(refused(name, "it is read on the Pi: name it with --host <user@address>, as ssh reaches it".into()));
        }
        // A step whose own proof holds already has nothing to do: running a
        // builder script twice is not always safe (03 appends to fstab).
        if let Verdict::Holds(h) = prove(repo, o, &g, &model, &d, step, &mut facts) {
            let mut checks = vec![row(step, &Verdict::Holds(h))];
            checks[0].detail = format!("{} (it already held, so nothing ran)", checks[0].detail);
            if let Some(n) = all.get(i + 1) {
                checks.push(Check { name: "next".into(), ok: true, detail: the_command(o, n.name, false), fix: None });
            }
            return Ok(Report::new("image", checks, crate::EXIT_REFUSED));
        }
        // The step before it, proven: for a person's step too, so its directions
        // are printed only when the machine is ready for them.
        if let Some(b) = before(&all, i) {
            match prove(repo, o, &g, &model, &d, b, &mut facts) {
                Verdict::Holds(_) => {}
                Verdict::Fails(why) | Verdict::Unread(why) => {
                    return Err(refused(name, format!("the step before it, {} ({}), is not proven: {why}. Run: {}", b.name, b.number, the_command(o, b.name, true))));
                }
            }
        }
        // What the step runs on the Pi is the card's own copy: it must be this checkout's.
        if let Some(host) = o.host {
            for (on_card, ours) in copies(step) {
                let want = sha256_file(&repo.join(BUILDER).join(ours)).map_err(|e| refused(name, format!("{BUILDER}/{ours}: {e}")))?;
                let out = Cmd::new("ssh").args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", host, "sha256sum", &format!("/boot/firmware/{on_card}")]).output();
                let have = out.ok().filter(|x| x.status.success()).map(|x| String::from_utf8_lossy(&x.stdout).split_whitespace().next().unwrap_or("").to_string());
                match have {
                    Some(h) if h == want => {}
                    Some(h) => return Err(refused(name, format!("the Pi's /boot/firmware/{on_card} is sha256 {h}, and this checkout's {ours} is {want}: flash the card again from this checkout, or copy the script there"))),
                    None => return Err(refused(name, format!("ssh {host} could not read /boot/firmware/{on_card}"))),
                }
            }
        }
        match (step.place, command(repo, o, &g, step)) {
            (Place::Person, _) => {
                return Ok(Report::new(
                    "image",
                    vec![Check { name: format!("step {} {}", step.number, step.name), ok: false, detail: format!("a person, now: {}. Then read its proof: {}", step.directions, the_command(o, name, true)), fix: None }],
                    crate::EXIT_NOT_CONVERGED,
                ));
            }
            (Place::Reads, _) => {}
            (_, Some(line)) => {
                if !step.directions.is_empty() {
                    eprintln!("{}", step.directions);
                }
                let mut c = match step.place {
                    Place::Machine => {
                        let mut c = Cmd::new("ssh");
                        c.args(["-t", o.host.unwrap_or_default(), "sudo", "bash", &format!("/boot/firmware/{}", step.script.unwrap_or_default())]);
                        c
                    }
                    _ => {
                        let mut c = Cmd::new("bash");
                        c.arg(repo.join(BUILDER).join(step.script.unwrap_or_default()));
                        if step.name != "prerequisites" {
                            c.arg(o.device.unwrap_or_default()).arg(image_path(repo, o, &g).unwrap_or_default());
                        }
                        c
                    }
                };
                let st = c.stdin(Stdio::inherit()).status().map_err(|e| refused(name, format!("could not start {line}: {e}")))?;
                if !st.success() {
                    return Err(goat_errors::coded("GB-413", format!("image step {name} failed: {line} exited {}; its own words above say why", st.code().map(|c| c.to_string()).unwrap_or_else(|| "on a signal".into()))).into());
                }
                ran = true;
            }
            (_, None) => {}
        }
        // What was read before the step ran is not its result.
        facts = None;
        if let Some(r) = step.reboot {
            return Ok(Report::new(
                "image",
                vec![Check { name: format!("step {} {}", step.number, step.name), ok: false, detail: format!("ran; a person now: {r}. Then read its proof: {}", the_command(o, name, true)), fix: None }],
                crate::EXIT_NOT_CONVERGED,
            ));
        }
    }
    let v = prove(repo, o, &g, &model, &d, step, &mut facts);
    match v {
        // A system that ejects the card after writing it (macOS does) has nothing to read back.
        Verdict::Unread(why) if ran && step.proof == Proof::Card => Ok(Report::new(
            "image",
            vec![Check { name: format!("step {} {}", step.number, step.name), ok: false, detail: format!("ran; the card cannot be read back now ({why}): put it in the reader again if it was ejected, then: {}", the_command(o, "verify-image", false)), fix: None }],
            crate::EXIT_NOT_CONVERGED,
        )),
        Verdict::Fails(why) if !o.check => Err(goat_errors::coded("GB-413", format!("image step {name} ran and its proof does not hold: {why}")).into()),
        v => {
            let mut checks = vec![row(step, &v)];
            if matches!(v, Verdict::Holds(_)) {
                if let Some(n) = all.get(i + 1) {
                    checks.push(Check { name: "next".into(), ok: true, detail: the_command(o, n.name, false), fix: None });
                }
            }
            Ok(Report::new("image", checks, crate::EXIT_REFUSED))
        }
    }
}

/// The builder's files a step runs on the Pi, as the card has them
/// (/boot/firmware/<path>) and as this checkout has them (under the builder).
fn copies(step: &Step) -> Vec<(&'static str, &'static str)> {
    match (step.place, step.name, step.script) {
        (Place::Machine, _, Some(s)) => vec![(s, s)],
        (_, "headless-unlock", Some(s)) => vec![(s, s)],
        // The initramfs runs these from the boot partition: the top-level copy
        // at the console, helper-scripts/ over SSH (01-flash.sh puts both).
        (_, "encrypt", _) => vec![
            ("encrypt-rootfs.sh", "helper-scripts/encrypt-rootfs.sh"),
            ("encrypt-rootfs-unlock.sh", "helper-scripts/encrypt-rootfs-unlock.sh"),
            ("helper-scripts/encrypt-rootfs.sh", "helper-scripts/encrypt-rootfs.sh"),
            ("helper-scripts/encrypt-rootfs-unlock.sh", "helper-scripts/encrypt-rootfs-unlock.sh"),
        ],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn opts(step: Option<&'static str>) -> Options<'static> {
        Options { model: "rpi5", step, check: false, host: None, device: None, image: None, unlock: Unlock::Yubikey, headless: false, dry_run: false }
    }

    #[test]
    fn the_rpi5_steps_are_the_builders_and_each_one_has_its_script() {
        let names: Vec<&str> = steps(&opts(None)).iter().map(|s| s.name).collect();
        assert_eq!(names, ["prerequisites", "flash", "verify-image", "boot", "clone", "prepare-encryption", "encrypt", "yubikey", "fips", "verify-box"]);
        let o = Options { headless: true, unlock: Unlock::Passphrase, ..opts(None) };
        let names: Vec<&str> = steps(&o).iter().map(|s| s.name).collect();
        assert_eq!(names, ["prerequisites", "flash", "verify-image", "boot", "clone", "prepare-encryption", "headless-unlock", "encrypt", "fips", "verify-box"]);
        for s in steps(&o) {
            if let Some(script) = s.script {
                assert!(repo().join(BUILDER).join(script).is_file(), "{script}");
            }
        }
    }

    #[test]
    fn the_golden_image_is_pinned_and_a_model_without_this_builder_is_refused() {
        let (family, g) = golden(&repo(), "rpi5").unwrap();
        assert_eq!(family, "small-computer");
        assert_eq!(g.file, "ubuntu-24.04.4-preinstalled-server-arm64+raspi.img.xz");
        assert_eq!(g.sha256.len(), 64);
        for (model, says) in [("karbon-801", "goatbuilder does not wrap yet"), ("vax", "not a model")] {
            let e = golden(&repo(), model).unwrap_err();
            assert_eq!(e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.as_str()), Some("GB-411"), "{e:#}");
            assert!(format!("{e:#}").contains(says), "{e:#}");
        }
    }

    #[test]
    fn flash_and_verify_image_both_stand_on_the_prerequisites() {
        let all = steps(&opts(None));
        let at = |n: &str| all.iter().position(|s| s.name == n).unwrap();
        assert_eq!(before(&all, at("flash")).unwrap().name, "prerequisites");
        assert_eq!(before(&all, at("verify-image")).unwrap().name, "prerequisites");
        assert_eq!(before(&all, at("clone")).unwrap().name, "boot");
        assert_eq!(before(&all, at("verify-box")).unwrap().name, "fips");
        assert!(before(&all, 0).is_none());
    }
}
