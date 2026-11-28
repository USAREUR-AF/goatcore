//! A small computer's end state, read on the machine (docs/design/
//! goatbuilder-build-targets.md sections 3.1 and 3.5): what its model's
//! golden path delivers, held to the machine whatever path built it.
//! `doctor --target <type> --site <site> --live` reads each machine that
//! declares the small-computer family. `ops/scripts/small-computer-endstate.py`
//! runs on the machine as root, reads, and changes nothing; the judging is
//! here. A fact that could not be read is unknown, which is not a pass.
//!
//! What a model delivers is data: its entry in its family's file
//! (`ops/goatbuilder/families/small-computer.yaml`), with its `end_state:`.
//! The FIPS-approved primitives are not a model's: they are the lists below,
//! which are the builder's own (`ops/rpi-base-image/helper-scripts/
//! encrypt-rootfs.sh` and `05-enable-fips.sh`).

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::Check;

/// The collector, under the checkout.
pub(crate) const SCRIPT: &str = "ops/scripts/small-computer-endstate.py";
/// The family whose machines these rows read.
pub(crate) const FAMILY: &str = "small-computer";

/// Data-at-rest primitives a LUKS2 root may use (FIPS 197, SP 800-38E, SP 800-132, FIPS 180-4).
const LUKS_CIPHERS: &[&str] = &["aes-xts-plain64"];
const LUKS_KEY_BITS: &[u64] = &[256, 512];
const LUKS_KDF: &[&str] = &["pbkdf2"];
const LUKS_HASHES: &[&str] = &["sha256", "sha384", "sha512"];
/// SSH, as `05-enable-fips.sh` restricts it.
const SSH_CIPHERS: &[&str] = &["aes256-gcm@openssh.com", "aes128-gcm@openssh.com", "aes256-ctr", "aes192-ctr", "aes128-ctr"];
const SSH_MACS: &[&str] = &["hmac-sha2-512", "hmac-sha2-256", "hmac-sha2-512-etm@openssh.com", "hmac-sha2-256-etm@openssh.com"];
const SSH_KEX: &[&str] = &["ecdh-sha2-nistp521", "ecdh-sha2-nistp384", "ecdh-sha2-nistp256", "diffie-hellman-group16-sha512", "diffie-hellman-group14-sha256"];
const SSH_KEYS: &[&str] = &["rsa-sha2-512", "rsa-sha2-256", "ecdsa-sha2-nistp521", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp256", "ssh-ed25519"];

/// What one model delivers, from its entry in its family's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Delivers {
    /// `uname -m` of the model.
    pub machine: String,
    /// What `/proc/device-tree/model` starts with on the board.
    pub board: String,
    /// Ubuntu versions the model's builder installs (`built` or `measured`).
    pub os_builder: Vec<String>,
    /// Ubuntu versions a machine of the model may run, however it was built:
    /// every one its entry lists but those `not supported`.
    pub os_accepted: Vec<String>,
    /// The kind of disk the root's partition is on: `nvme`.
    pub root_media: String,
    /// The ways the root may be unlocked at boot (the entry's `encryption.unlock`).
    pub unlock: Vec<String>,
    /// Where FIPS mode holds.
    pub fips: Fips,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fips {
    /// The OpenSSL FIPS provider alone: the model's kernel has no FIPS mode.
    Userspace,
    /// The kernel's FIPS mode as well.
    Kernel,
}

/// GB-410, with what could not be read.
fn unreadable(why: String) -> anyhow::Error {
    goat_errors::coded("GB-410", format!("what the small computer's model delivers cannot be read: {why}")).into()
}

/// The one model of the family, and what its entry says it delivers.
pub(crate) fn model(models: Option<&crate::targets::Models>) -> Result<(String, Delivers)> {
    let file = format!("ops/goatbuilder/families/{FAMILY}.yaml");
    let entries = match models {
        Some(crate::targets::Models::Described(e)) => e,
        Some(crate::targets::Models::Listed(_)) => return Err(unreadable(format!("{file} lists its models' names with no entry for each"))),
        None => return Err(unreadable(format!("there is no {file}"))),
    };
    let mut it = entries.iter();
    let (Some((name, m)), None) = (it.next(), it.next()) else {
        return Err(unreadable(format!("the family has {} models, and which one this machine is is not declared (not built)", entries.len())));
    };
    let Some(e) = &m.end_state else {
        return Err(unreadable(format!("{file}'s {name} has no end_state")));
    };
    if m.encryption.scope != "root" {
        return Err(unreadable(format!("{file}'s {name} delivers an encrypted {}, and these rows read an encrypted root", m.encryption.scope)));
    }
    let machine = match m.arch.as_str() {
        "arm64" => "aarch64",
        "amd64" => "x86_64",
        other => return Err(unreadable(format!("{file}'s {name} is of architecture {other}"))),
    };
    let fips = match e.fips.as_str() {
        "userspace" => Fips::Userspace,
        "kernel" => Fips::Kernel,
        other => return Err(unreadable(format!("{file}'s {name} holds FIPS mode in '{other}'"))),
    };
    Ok((
        name.clone(),
        Delivers {
            machine: machine.into(),
            board: e.board.clone(),
            os_builder: m.os.ubuntu.iter().filter(|(_, st)| matches!(st.as_str(), "built" | "measured")).map(|(v, _)| v.clone()).collect(),
            os_accepted: m.os.ubuntu.iter().filter(|(_, st)| st.as_str() != "not supported").map(|(v, _)| v.clone()).collect(),
            root_media: e.root_media.clone(),
            unlock: m.encryption.unlock.clone(),
            fips,
        },
    ))
}

/// One end-state check's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

fn error(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("not reported".into()),
        _ => v.get("error").and_then(|e| e.as_str()).map(String::from),
    }
}

/// What is wrong, and what could not be read, for one row.
#[derive(Default)]
struct Found {
    bad: Vec<String>,
    unknown: Vec<String>,
}

impl Found {
    fn known<'a>(&mut self, what: &str, v: &'a Value) -> Option<&'a Value> {
        match error(v) {
            Some(e) => {
                self.unknown.push(format!("{what}: {e}"));
                None
            }
            None => Some(v),
        }
    }

    fn row(self, name: &'static str, good: String) -> Row {
        if !self.bad.is_empty() {
            return Row { name, ok: false, detail: self.bad.into_iter().chain(self.unknown.into_iter().map(|u| format!("unknown: {u}"))).collect::<Vec<_>>().join("; ") };
        }
        if !self.unknown.is_empty() {
            return Row { name, ok: false, detail: format!("unknown: {}", self.unknown.join("; ")) };
        }
        Row { name, ok: true, detail: good }
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn list(v: &Value) -> Vec<String> {
    s(v).split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// Algorithms of an `sshd -T` list that are not in `allowed`. A certificate
/// form of an allowed key type is allowed.
fn outside(v: &Value, allowed: &[&str]) -> Vec<String> {
    list(v).into_iter().filter(|a| !allowed.contains(&a.as_str()) && !allowed.iter().any(|k| a == &format!("{}-cert-v01@openssh.com", k.trim_end_matches("@openssh.com")))).collect()
}

/// Every way the root can be unlocked at boot, from crypttab, the header's
/// tokens, the initramfs and the boot partition. Each is held to the model.
fn unlock_paths(f: &Value, found: &mut Found) -> Vec<String> {
    let mut paths = Vec::new();
    let Some(tab) = found.known("/etc/crypttab", &f["crypttab"]).map(|t| t.as_array().cloned().unwrap_or_default()) else { return paths };
    let Some(mapper) = found.known("the root's active mapping", &f["mapping"]).map(|m| s(&m["name"]).trim_start_matches("/dev/mapper/").to_string()) else { return paths };
    let init = found.known("the initramfs", &f["initramfs"]).cloned();
    let dev = s(&f["luks"]["device"]).to_string();
    let uuid = f["luks"]["uuid"].as_str().map(|u| format!("UUID={u}"));
    let ours = |e: &Value| {
        let d = s(&e["device"]);
        s(&e["name"]) == mapper || (!dev.is_empty() && d == dev) || uuid.as_deref() == Some(d) || uuid.as_deref().is_some_and(|u| d == u.replace("UUID=", "/dev/disk/by-uuid/"))
    };
    let entries: Vec<&Value> = tab.iter().filter(|e| ours(e)).collect();
    if entries.is_empty() && !tab.is_empty() {
        found.unknown.push(format!("crypttab has entries ({}), and none names this root's mapping {mapper} or its device", tab.iter().map(|e| s(&e["name"])).collect::<Vec<_>>().join(", ")));
        return paths;
    }
    let mut keyscript = false;
    for e in &entries {
        let opts: Vec<String> = e["options"].as_array().cloned().unwrap_or_default().iter().map(|o| s(o).to_string()).collect();
        for k in opts.iter().filter_map(|o| o.strip_prefix("keyscript=")) {
            keyscript = true;
            if k == "passdev" || k.ends_with("/passdev") {
                continue;
            }
            if k == "yubikey_unlock" || k.ends_with("/yubikey_unlock") {
                if let Some(i) = &init {
                    let lacks: Vec<&str> = [("ykchalresp", &i["ykchalresp"]), ("yubikey_unlock", &i["yubikey_unlock"])].iter().filter(|(_, v)| **v != true).map(|(n, _)| *n).collect();
                    if !lacks.is_empty() {
                        found.bad.push(format!("crypttab names the YubiKey keyscript, and {} lacks {}", s(&i["file"]), lacks.join(" and ")));
                    }
                }
                paths.push("yubikey-hmac".into());
            } else {
                paths.push(format!("keyscript {k}"));
            }
        }
        if !matches!(s(&e["key"]), "none" | "-" | "") {
            keyscript = true;
            // A key on removable media, as passdev reads it (<device>:<path>), or a file on the machine.
            let k = s(&e["key"]);
            if opts.iter().any(|o| o == "keyscript=passdev") || k.starts_with("/dev/") {
                paths.push("usb-key".into());
            } else {
                paths.push(format!("key file {k}"));
            }
        }
    }
    for (_, t) in f["luks"]["metadata"]["tokens"].as_object().cloned().unwrap_or_default() {
        paths.push(format!("token {}", s(&t["type"])));
    }
    if let Some(i) = &init {
        if i["clevis"] == true {
            paths.push("clevis in the initramfs".into());
        }
        for k in i["keyfiles"].as_array().cloned().unwrap_or_default() {
            paths.push(format!("key file {} in the initramfs", s(&k)));
        }
        if i["dropbear"] == true {
            paths.push("ssh-passphrase".into());
        }
        if !keyscript {
            if i["cryptsetup"] != true {
                found.bad.push(format!("{} has no cryptsetup, so nothing can unlock the root at boot", s(&i["file"])));
            }
            paths.push("console-passphrase".into());
        }
    }
    if let Some(k) = found.known("the boot partition's key files", &f["boot_keyfiles"]) {
        for k in k.as_array().cloned().unwrap_or_default() {
            found.bad.push(format!("the unencrypted boot partition holds the key file {}, which unlocks the root for anyone who has the disk", s(&k)));
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Hold one machine's facts to its model. Every row of section 3.5's small
/// computer that the machine itself shows.
pub(crate) fn judge(model: &str, d: &Delivers, f: &Value) -> Vec<Row> {
    let mut out = Vec::new();

    let mut o = Found::default();
    if let Some(v) = o.known("/etc/os-release", &f["os"]) {
        let (id, ver) = (s(&v["id"]), s(&v["version_id"]));
        if id != "ubuntu" || !d.os_accepted.iter().any(|a| a == ver) {
            o.bad.push(format!("the machine runs {id} {ver}; a {model} is accepted on Ubuntu {}", d.os_accepted.join(" or ")));
        }
    }
    let ver = s(&f["os"]["version_id"]).to_string();
    let note = if d.os_builder.contains(&ver) { format!("which the {model} builder installs") } else { format!("accepted; the {model} builder's steps install {} only, so these rows are what this machine shows, not what the builder measured", d.os_builder.join(", ")) };
    out.push(o.row("os", format!("Ubuntu {ver}, {note}")));

    let mut h = Found::default();
    if s(&f["arch"]) != d.machine {
        h.bad.push(format!("the machine is {}, and a {model} is {}", s(&f["arch"]), d.machine));
    }
    if let Some(b) = h.known("/proc/device-tree/model", &f["board"]) {
        if !s(b).starts_with(&d.board) {
            h.bad.push(format!("the board is '{}', not a {}", s(b), d.board));
        }
    }
    out.push(h.row("board", format!("{}, {}", s(&f["board"]), d.machine)));

    let mut r = Found::default();
    if let Some(root) = r.known("the root's devices", &f["root"]) {
        let chain = root["chain"].as_array().cloned().unwrap_or_default();
        let kinds: Vec<&str> = chain.iter().map(|x| s(&x["type"])).collect();
        if !kinds.contains(&"crypt") {
            r.bad.push(format!("the root ({}) is not on an encrypted device, and the {model} delivers an encrypted root", s(&root["source"])));
        } else if let Some(v) = r.known("the LUKS header", &f["luks"]) {
            if v["version"] != 2 {
                r.bad.push(format!("{} is LUKS{}, not LUKS2", s(&v["device"]), v["version"]));
            }
            let disk = chain.last().map(|x| s(&x["name"]).to_string()).unwrap_or_default();
            if !disk.contains(&format!("/{}", d.root_media)) {
                r.bad.push(format!("the root's disk is {disk}, and the {model} boots its root from {}", d.root_media));
            }
            // The mapping the kernel runs, not only the header on the disk.
            if let Some(m) = r.known("the root's active mapping", &f["mapping"]) {
                let header = f["luks"]["metadata"]["segments"].as_object().and_then(|x| x.values().find(|g| s(&g["type"]) == "crypt")).map(|g| s(&g["encryption"]).to_string()).unwrap_or_default();
                if s(&m["type"]) != "LUKS2" {
                    r.bad.push(format!("the root's mapping {} is {}, not the LUKS2 header's", s(&m["name"]), if m["type"].is_null() { "of no type cryptsetup reports" } else { s(&m["type"]) }));
                }
                // A header with no encrypted segment fails the primitives row.
                if !header.is_empty() && s(&m["cipher"]) != header {
                    r.bad.push(format!("the root's mapping runs {}, and its header says {header}", s(&m["cipher"])));
                }
                let bits: u64 = s(&m["keysize"]).split_whitespace().next().and_then(|b| b.parse().ok()).unwrap_or(0);
                if !LUKS_KEY_BITS.contains(&bits) {
                    r.bad.push(format!("the root's mapping has a {}-bit key", bits));
                }
            }
        }
    }
    out.push(r.row("root", format!("the root is on {} (LUKS2, {}) on {}; the root alone is read, not other mounts or swap", s(&f["mapping"]["name"]), s(&f["mapping"]["cipher"]), s(&f["luks"]["device"]))));

    let mut p = Found::default();
    let meta = if f["luks"]["version"] == 2 { p.known("the LUKS2 metadata", &f["luks"]["metadata"]) } else { None };
    let mut seen = String::new();
    if let Some(m) = meta {
        let approved = |what: &str, v: &str, allowed: &[&str], bad: &mut Vec<String>| {
            if !allowed.contains(&v) {
                bad.push(format!("{what} uses {v}, which is not FIPS-approved here ({})", allowed.join(", ")));
            }
        };
        let segments = m["segments"].as_object().cloned().unwrap_or_default();
        if !segments.values().any(|g| s(&g["type"]) == "crypt") {
            p.bad.push("the header has no encrypted segment".into());
        }
        if m["digests"].as_object().is_none_or(|g| g.is_empty()) {
            p.bad.push("the header has no digest".into());
        }
        for (id, seg) in segments {
            if s(&seg["type"]) == "crypt" {
                approved(&format!("segment {id}"), s(&seg["encryption"]), LUKS_CIPHERS, &mut p.bad);
            } else {
                p.bad.push(format!("segment {id} is {}, so part of the disk is not encrypted", s(&seg["type"])));
            }
        }
        let slots = m["keyslots"].as_object().cloned().unwrap_or_default();
        if slots.is_empty() {
            p.bad.push("the header has no keyslot".into());
        }
        for (id, k) in &slots {
            approved(&format!("keyslot {id}'s key derivation"), s(&k["kdf"]["type"]), LUKS_KDF, &mut p.bad);
            if s(&k["kdf"]["type"]) == "pbkdf2" {
                approved(&format!("keyslot {id}'s key derivation hash"), s(&k["kdf"]["hash"]), LUKS_HASHES, &mut p.bad);
            }
            approved(&format!("keyslot {id}'s area"), s(&k["area"]["encryption"]), LUKS_CIPHERS, &mut p.bad);
            if !k["af"].is_null() {
                approved(&format!("keyslot {id}'s anti-forensic split"), s(&k["af"]["hash"]), LUKS_HASHES, &mut p.bad);
            }
            let area = k["area"]["key_size"].as_u64().unwrap_or(0) * 8;
            if !LUKS_KEY_BITS.contains(&area) {
                p.bad.push(format!("keyslot {id}'s area is encrypted with a {area}-bit key"));
            }
            let bits = k["key_size"].as_u64().unwrap_or(0) * 8;
            if !LUKS_KEY_BITS.contains(&bits) {
                p.bad.push(format!("keyslot {id} holds a {bits}-bit volume key; AES-XTS takes {}", LUKS_KEY_BITS.iter().map(u64::to_string).collect::<Vec<_>>().join(" or ")));
            }
        }
        for (id, g) in m["digests"].as_object().cloned().unwrap_or_default() {
            approved(&format!("digest {id}"), s(&g["hash"]), LUKS_HASHES, &mut p.bad);
        }
        let reqs: Vec<String> = m["requirements"]["mandatory"].as_array().cloned().unwrap_or_default().iter().map(|x| s(x).to_string()).collect();
        if reqs.iter().any(|x| x.contains("reencrypt")) {
            p.bad.push("a reencryption is not finished (the header requires online-reencrypt), so part of the disk may still be plain".into());
        }
        let bits: Vec<String> = slots.values().map(|k| (k["key_size"].as_u64().unwrap_or(0) * 8).to_string()).collect();
        seen = format!(
            "{} with a {}-bit key; {} keyslot{}, every one {}",
            m["segments"].as_object().and_then(|x| x.values().next()).map(|x| s(&x["encryption"]).to_string()).unwrap_or_default(),
            bits.first().cloned().unwrap_or_default(),
            slots.len(),
            if slots.len() == 1 { "" } else { "s" },
            slots.values().map(|k| format!("{}-{}", s(&k["kdf"]["type"]), s(&k["kdf"]["hash"]))).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(", ")
        );
    } else if f["luks"]["version"].is_number() && f["luks"]["version"] != 2 {
        p.bad.push("the header is not LUKS2, so its primitives are not read".into());
    } else if p.unknown.is_empty() {
        p.unknown.push(format!("the LUKS header: {}", error(&f["luks"]).unwrap_or_else(|| "not read".into())));
    }
    out.push(p.row("primitives", seen));

    let mut u = Found::default();
    let paths = if error(&f["luks"]).is_none() { unlock_paths(f, &mut u) } else {
        u.unknown.push("the unlock: the root's LUKS device was not read".into());
        Vec::new()
    };
    for p in paths.iter().filter(|p| !d.unlock.contains(p)) {
        u.bad.push(format!("the root unlocks by {p}, which the {model} does not deliver ({})", d.unlock.join(", ")));
    }
    let slots = f["luks"]["metadata"]["keyslots"].as_object().map(|x| x.len()).unwrap_or(0);
    let mut recovery = String::new();
    if paths.iter().any(|p| p == "yubikey-hmac") {
        if slots < 2 {
            u.bad.push(format!("the YubiKey unlocks the root and the header has {slots} keyslot: there is no recovery passphrase beside it"));
        } else {
            recovery = format!("; {slots} keyslots, so a recovery passphrase can be beside it (which slot is which is not read)");
        }
    }
    if paths.is_empty() && u.bad.is_empty() && u.unknown.is_empty() {
        u.unknown.push("no way to unlock the root at boot was found".into());
    }
    out.push(u.row("unlock", format!("{}{recovery}", paths.join(", "))));

    let mut x = Found::default();
    if let Some(pv) = x.known("openssl list -providers", &f["openssl_providers"]) {
        match pv.as_object().and_then(|m| m.iter().find(|(k, v)| k.as_str() == "fips" || s(&v["name"]).contains("FIPS"))) {
            Some((_, v)) if s(&v["status"]) == "active" => {}
            Some((_, v)) => x.bad.push(format!("the OpenSSL FIPS provider is {}, not active", s(&v["status"]))),
            None => x.bad.push("OpenSSL has no FIPS provider loaded".into()),
        }
    }
    // Whether the default properties hold programs to the provider: reported,
    // not required. The builder activates the FIPS provider beside the default
    // one and sets no default properties, so a program that asks for no
    // provider may still use a non-approved algorithm.
    let defaults = match &f["openssl_md5"] {
        Value::String(v) if v == "refused" => "its default properties hold programs to it (MD5 is refused)".to_string(),
        Value::String(v) if v == "computes" => "its default properties do not hold programs to it (MD5 still computes; the builder sets none)".to_string(),
        v => format!("whether its default properties hold programs to it is not read ({})", error(v).unwrap_or_else(|| "no answer".into())),
    };
    if let Some(c) = x.known("/proc/cmdline", &f["cmdline"]) {
        if !c.as_array().is_some_and(|a| a.iter().any(|t| s(t) == "fips=1")) {
            x.bad.push("the kernel command line has no fips=1".into());
        }
    }
    let kernel = match x.known("the kernel's FIPS mode", &f["kernel_fips"]).map(s) {
        Some("1") => "the kernel's FIPS mode is on".to_string(),
        Some(k) if d.fips == Fips::Kernel => {
            x.bad.push(format!("the kernel's FIPS mode is {k}, and the {model} delivers it"));
            String::new()
        }
        Some("absent") => format!("this kernel has no FIPS mode (the {model} delivers FIPS mode in userspace only)"),
        Some(k) => format!("the kernel's FIPS mode reads {k} (the {model} delivers FIPS mode in userspace only)"),
        None => String::new(),
    };
    out.push(x.row("fips", format!("the OpenSSL FIPS provider is active and fips=1 is set; {defaults}; {kernel}")));

    let mut k = Found::default();
    if let Some(v) = k.known("sshd -T", &f["sshd"]) {
        for (key, allowed) in [("ciphers", SSH_CIPHERS), ("macs", SSH_MACS), ("kexalgorithms", SSH_KEX), ("hostkeyalgorithms", SSH_KEYS), ("pubkeyacceptedalgorithms", SSH_KEYS)] {
            if v[key].is_null() {
                k.unknown.push(format!("sshd -T printed no {key}"));
                continue;
            }
            let o = outside(&v[key], allowed);
            if !o.is_empty() {
                k.bad.push(format!("sshd offers {key} that are not FIPS-approved here: {}", o.join(", ")));
            }
        }
        for key in ["passwordauthentication", "kbdinteractiveauthentication"] {
            if s(&v[key]) != "no" {
                k.bad.push(format!("sshd {key} is {}, not no: SSH is not key-only", if v[key].is_null() { "not reported" } else { s(&v[key]) }));
            }
        }
        if v["hostbasedauthentication"].is_null() {
            k.unknown.push("sshd -T printed no hostbasedauthentication".into());
        } else if s(&v["hostbasedauthentication"]) != "no" {
            k.bad.push("sshd takes host-based logins (hostbasedauthentication is not no)".into());
        }
        if !v["gssapiauthentication"].is_null() && s(&v["gssapiauthentication"]) != "no" {
            k.bad.push("sshd takes GSSAPI logins (gssapiauthentication is not no)".into());
        }
        if s(&v["pubkeyauthentication"]) != "yes" {
            k.bad.push("sshd does not take keys (pubkeyauthentication is not yes)".into());
        }
        if s(&v["permitrootlogin"]) == "yes" {
            k.bad.push("sshd lets root log in with a password (permitrootlogin yes)".into());
        }
    }
    if let Some(m) = k.known("sshd's Match blocks", &f["sshd_match"]) {
        let at: Vec<&str> = m.as_array().map(|a| a.iter().map(s).collect()).unwrap_or_default();
        if !at.is_empty() {
            k.unknown.push(format!("sshd's configuration has Match blocks ({}), and sshd -T without a connection does not show what they change", at.join(", ")));
        }
    }
    out.push(k.row("ssh", format!("key-only, root login {}, every algorithm FIPS-approved, no Match block", s(&f["sshd"]["permitrootlogin"]))));

    let mut b = Found::default();
    let fb = &f["first_boot"];
    if let Some(st) = b.known("cloud-init status", &fb["status"]) {
        let finished = fb["finished"] == true;
        match s(st) {
            "done" => {}
            "disabled" if finished => {}
            other => b.bad.push(format!("cloud-init's first boot is {other}{}, not done", if finished { "" } else { " and never finished" })),
        }
    }
    if f["root_user"] != true {
        b.bad.push("the collector did not run as root: sudo did not work for the deploy user".into());
    }
    out.push(b.row("first boot", format!("cloud-init's first boot {}; Python {}, sudo works", s(&fb["status"]), s(&f["python"]))));

    out
}

/// Read one machine over Ansible, as root.
fn read(repo: &Path, inv: &[String], node: &str) -> Result<Value, String> {
    let mut args = inv.to_vec();
    // BatchMode: an unknown host key or a password prompt fails at once
    // (OpenSSH asks on the terminal, not on stdin); -T bounds the connection.
    args.extend(["-T".into(), CONNECT_SECONDS.to_string(), "--ssh-extra-args=-o BatchMode=yes".into()]);
    args.extend([node.to_string(), "-b".into(), "-m".into(), "script".into(), "-a".into(), format!("executable=/usr/bin/python3 {}", repo.join(SCRIPT).display())]);
    let mut c = crate::push::ansible_cmd(repo, "ansible", &args);
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let child = c.spawn().map_err(|e| format!("could not start ansible: {e}"))?;
    let out = with_deadline(child, std::time::Duration::from_secs(READ_SECONDS)).map_err(|e| format!("{e} (ansible did not answer within {READ_SECONDS} s)"))?;
    facts(&String::from_utf8_lossy(&out))
}

/// Seconds Ansible may take to connect, and to read the whole machine.
const CONNECT_SECONDS: u64 = 20;
const READ_SECONDS: u64 = 300;

/// A child's stdout, or an error once `limit` passes (the child is killed).
fn with_deadline(mut child: std::process::Child, limit: std::time::Duration) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout.read_to_end(&mut b);
        b
    });
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(format!("could not wait for ansible: {e}")),
        }
    }
    reader.join().map_err(|_| "its output could not be read".to_string())
}

/// The collector's object inside Ansible's `host | CHANGED => {...}` answer.
pub(crate) fn facts(answer: &str) -> Result<Value, String> {
    let first = answer.lines().find(|l| !l.trim().is_empty()).unwrap_or("ansible answered nothing").trim();
    let bad = || first.chars().take(160).collect::<String>();
    if !first.contains("| CHANGED") && !first.contains("| SUCCESS") {
        return Err(bad());
    }
    let at = answer.find('{').ok_or_else(bad)?;
    let v: Value = serde_json::from_str(answer[at..].trim()).map_err(|_| bad())?;
    let text = v.get("stdout").and_then(|s| s.as_str()).unwrap_or("");
    serde_json::from_str(text.trim()).map_err(|e| format!("{SCRIPT} did not print its facts: {e}"))
}

/// Turn rows into doctor checks named for the machine.
pub(crate) fn checks(node: &str, rows: Vec<Row>) -> Vec<Check> {
    rows.into_iter()
        .map(|r| Check {
            name: format!("{FAMILY} {node} {}", r.name),
            ok: r.ok,
            fix: (!r.ok).then(|| "correct it on the machine (ops/rpi-base-image/README.md says how the golden path does it), then run this again".to_string()),
            detail: r.detail,
        })
        .collect()
}

/// The `--live` rows of one small computer, and whether its root was read.
pub(crate) fn rows(repo: &Path, inv: &[String], node: &str, model: &Result<(String, Delivers)>) -> (Vec<Check>, bool) {
    let (name, d) = match model {
        Ok(m) => m,
        Err(e) => return (checks(node, vec![Row { name: "model", ok: false, detail: format!("unknown: {e:#}") }]), false),
    };
    match read(repo, inv, node) {
        Ok(f) => (checks(node, judge(name, d, &f)), true),
        Err(why) => (checks(node, vec![Row { name: "machine", ok: false, detail: format!("unknown: {node} was not read: {why}") }]), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shipped() -> crate::targets::Targets {
        crate::targets::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
    }

    fn rpi5() -> Delivers {
        model(shipped().models(FAMILY)).unwrap().1
    }

    /// A Pi as the builder leaves it, with the YubiKey enrolled.
    pub(crate) fn good() -> Value {
        let slot = json!({"type": "luks2", "key_size": 64, "af": {"type": "luks1", "stripes": 4000, "hash": "sha256"}, "area": {"encryption": "aes-xts-plain64", "key_size": 64}, "kdf": {"type": "pbkdf2", "hash": "sha256", "iterations": 600000}});
        json!({
            "os": {"id": "ubuntu", "version_id": "24.04"}, "arch": "aarch64", "board": "Raspberry Pi 5 Model B Rev 1.0",
            "root": {"source": "/dev/mapper/rootfs", "chain": [{"name": "/dev/mapper/rootfs", "type": "crypt", "fstype": "ext4"}, {"name": "/dev/nvme0n1p2", "type": "part", "fstype": "crypto_LUKS"}, {"name": "/dev/nvme0n1", "type": "disk", "fstype": null}]},
            "luks": {"device": "/dev/nvme0n1p2", "version": 2, "uuid": "e6b2f910-123a-40bf-a11f-f7b98b35b522", "metadata": {"keyslots": {"0": slot, "1": slot}, "segments": {"0": {"type": "crypt", "encryption": "aes-xts-plain64"}}, "digests": {"0": {"type": "pbkdf2", "hash": "sha256"}}, "tokens": {}, "requirements": {}}},
            "mapping": {"name": "rootfs", "type": "LUKS2", "cipher": "aes-xts-plain64", "keysize": "512 bits"},
            "crypttab": [{"name": "rootfs", "device": "/dev/nvme0n1p2", "key": "-", "options": ["luks", "initramfs", "keyscript=yubikey_unlock", "noauto"]}],
            "initramfs": {"file": "/boot/firmware/initrd.img", "cryptsetup": true, "ykchalresp": true, "yubikey_unlock": true, "dropbear": false, "clevis": false, "keyfiles": []},
            "cmdline": ["console=tty1", "root=/dev/mapper/rootfs", "cryptdevice=/dev/nvme0n1p2:rootfs", "fips=1"],
            "kernel_fips": "absent",
            "openssl_providers": {"default": {"name": "OpenSSL Default Provider", "status": "active"}, "fips": {"name": "OpenSSL FIPS Provider", "status": "active"}},
            "openssl_md5": "computes", "sshd_match": [], "boot_keyfiles": [],
            "sshd": {"ciphers": SSH_CIPHERS.join(","), "macs": SSH_MACS.join(","), "kexalgorithms": SSH_KEX.join(","), "hostkeyalgorithms": SSH_KEYS.join(","), "pubkeyacceptedalgorithms": SSH_KEYS.join(","),
                     "passwordauthentication": "no", "kbdinteractiveauthentication": "no", "permitrootlogin": "prohibit-password", "pubkeyauthentication": "yes",
                     "hostbasedauthentication": "no", "gssapiauthentication": "no"},
            "first_boot": {"finished": true, "status": "done"}, "boot_order": "0xf416", "python": "3.12.3", "root_user": true
        })
    }

    fn failing(f: &Value) -> Vec<(&'static str, String)> {
        judge("rpi5", &rpi5(), f).into_iter().filter(|r| !r.ok).map(|r| (r.name, r.detail)).collect()
    }

    fn set(f: &mut Value, ptr: &str, v: Value) {
        *f.pointer_mut(ptr).unwrap_or_else(|| panic!("no {ptr}")) = v;
    }

    #[test]
    fn a_pi_as_the_builder_leaves_it_passes_every_row() {
        let rows = judge("rpi5", &rpi5(), &good());
        assert_eq!(rows.iter().map(|r| r.name).collect::<Vec<_>>(), ["os", "board", "root", "primitives", "unlock", "fips", "ssh", "first boot"]);
        assert!(rows.iter().all(|r| r.ok), "{rows:#?}");
        let detail = |n: &str| rows.iter().find(|r| r.name == n).unwrap().detail.clone();
        assert_eq!(detail("primitives"), "aes-xts-plain64 with a 512-bit key; 2 keyslots, every one pbkdf2-sha256");
        assert!(detail("unlock").starts_with("yubikey-hmac; 2 keyslots"), "{}", detail("unlock"));
        assert!(detail("fips").contains("this kernel has no FIPS mode"), "{}", detail("fips"));
        assert!(detail("fips").contains("do not hold programs to it (MD5 still computes; the builder sets none)"), "{}", detail("fips"));
        // Reported, never required: enforced, or not read, the row still holds.
        for (v, says) in [(json!("refused"), "hold programs to it (MD5 is refused)"), (json!({"error": "openssl: cannot load config"}), "not read (openssl: cannot load config)")] {
            let mut f = good();
            set(&mut f, "/openssl_md5", v);
            let r = judge("rpi5", &rpi5(), &f).into_iter().find(|r| r.name == "fips").unwrap();
            assert!(r.ok && r.detail.contains(says), "{r:?}");
        }
        assert!(detail("os").contains("which the rpi5 builder installs"), "{}", detail("os"));
    }

    #[test]
    fn each_rule_broken_on_its_own_fails_its_own_row() {
        let argon = json!({"type": "luks2", "key_size": 64, "area": {"encryption": "aes-xts-plain64", "key_size": 64}, "kdf": {"type": "argon2id", "time": 4}});
        let cases: Vec<(&str, Value, &str, &str)> = vec![
            ("/os/version_id", json!("22.04"), "os", "the machine runs ubuntu 22.04"),
            ("/arch", json!("x86_64"), "board", "the machine is x86_64"),
            ("/board", json!("linux,dummy-virt"), "board", "not a Raspberry Pi 5"),
            ("/root/chain/0/type", json!("part"), "root", "is not on an encrypted device"),
            ("/root/chain/2/name", json!("/dev/mmcblk0"), "root", "boots its root from nvme"),
            ("/luks/metadata/keyslots/1", argon, "primitives", "keyslot 1's key derivation uses argon2id"),
            ("/luks/metadata/keyslots/0/kdf/hash", json!("sha1"), "primitives", "keyslot 0's key derivation hash uses sha1"),
            ("/luks/metadata/keyslots/0/key_size", json!(16), "primitives", "a 128-bit volume key"),
            ("/luks/metadata/digests/0/hash", json!("sha1"), "primitives", "digest 0 uses sha1"),
            ("/luks/metadata/requirements", json!({"mandatory": ["online-reencrypt-v2"]}), "primitives", "a reencryption is not finished"),
            ("/initramfs/ykchalresp", json!(false), "unlock", "lacks ykchalresp"),
            ("/crypttab/0/options", json!(["luks", "keyscript=decrypt_keyctl"]), "unlock", "keyscript decrypt_keyctl, which the rpi5 does not deliver"),
            ("/openssl_providers/fips/status", json!("inactive"), "fips", "is inactive, not active"),
            ("/openssl_providers", json!({"default": {"name": "OpenSSL Default Provider", "status": "active"}}), "fips", "no FIPS provider"),
            ("/cmdline", json!(["root=/dev/mapper/rootfs"]), "fips", "no fips=1"),
            ("/sshd/ciphers", json!("chacha20-poly1305@openssh.com,aes256-ctr"), "ssh", "ciphers that are not FIPS-approved here: chacha20-poly1305@openssh.com"),
            ("/sshd/kexalgorithms", json!("sntrup761x25519-sha512,curve25519-sha256"), "ssh", "sntrup761x25519-sha512, curve25519-sha256"),
            ("/sshd/passwordauthentication", json!("yes"), "ssh", "SSH is not key-only"),
            ("/sshd/permitrootlogin", json!("yes"), "ssh", "permitrootlogin yes"),
            ("/first_boot", json!({"finished": false, "status": "running"}), "first boot", "first boot is running and never finished"),
            ("/board", json!("Raspberry Pi 500 Rev 1.0"), "board", "not a Raspberry Pi 5 Model B"),
            ("/mapping/cipher", json!("cipher_null-ecb"), "root", "mapping runs cipher_null-ecb, and its header says aes-xts-plain64"),
            ("/mapping/type", json!("PLAIN"), "root", "mapping rootfs is PLAIN"),
            ("/luks/metadata/segments/0/type", json!("linear"), "primitives", "no encrypted segment"),
            ("/luks/metadata/digests", json!({}), "primitives", "no digest"),
            ("/luks/metadata/keyslots/0/af", json!({"type": "luks1", "stripes": 4000, "hash": "sha1"}), "primitives", "anti-forensic split uses sha1"),
            ("/luks/metadata/keyslots/0/area/key_size", json!(16), "primitives", "area is encrypted with a 128-bit key"),
            ("/luks/metadata/tokens", json!({"0": {"type": "clevis"}}), "unlock", "unlocks by token clevis"),
            ("/initramfs/clevis", json!(true), "unlock", "unlocks by clevis in the initramfs"),
            ("/initramfs/keyfiles", json!(["cryptroot/keyfiles/rootfs.key"]), "unlock", "key file cryptroot/keyfiles/rootfs.key in the initramfs"),
            ("/boot_keyfiles", json!(["disk.lek"]), "unlock", "boot partition holds the key file disk.lek"),
            ("/crypttab/0/key", json!("/etc/keys/root.key"), "unlock", "key file /etc/keys/root.key"),
            ("/crypttab/0/options", json!(["luks", "keyscript=decrypt_derived"]), "unlock", "keyscript decrypt_derived"),
            ("/sshd/hostbasedauthentication", json!("yes"), "ssh", "host-based logins"),
            ("/sshd/gssapiauthentication", json!("yes"), "ssh", "GSSAPI logins"),
            ("/sshd_match", json!(["/etc/ssh/sshd_config:120"]), "ssh", "Match blocks (/etc/ssh/sshd_config:120)"),
            ("/root_user", json!(false), "first boot", "did not run as root"),
        ];
        for (ptr, v, row, says) in cases {
            let mut f = good();
            set(&mut f, ptr, v);
            let bad = failing(&f);
            assert!(bad.len() == 1 && bad[0].0 == row && bad[0].1.contains(says), "{ptr}: {bad:#?}");
        }
    }

    #[test]
    fn a_cipher_that_is_not_approved_fails_the_primitives_and_a_mapping_unlike_its_header_fails_the_root() {
        let mut f = good();
        set(&mut f, "/luks/metadata/segments/0/encryption", json!("serpent-xts-plain64"));
        set(&mut f, "/mapping/cipher", json!("serpent-xts-plain64"));
        let bad = failing(&f);
        assert!(bad.len() == 1 && bad[0].0 == "primitives" && bad[0].1.contains("segment 0 uses serpent-xts-plain64"), "{bad:#?}");
        set(&mut f, "/mapping/cipher", json!("aes-xts-plain64"));
        assert!(failing(&f).iter().any(|(n, d)| *n == "root" && d.contains("runs aes-xts-plain64, and its header says serpent-xts-plain64")));
    }

    #[test]
    fn a_luks1_root_fails_the_root_and_its_primitives_are_not_read() {
        let mut f = good();
        set(&mut f, "/luks/version", json!(1));
        let bad = failing(&f);
        assert!(bad.iter().any(|(n, d)| *n == "root" && d.contains("is LUKS1, not LUKS2")), "{bad:#?}");
        assert!(bad.iter().any(|(n, d)| *n == "primitives" && d.contains("not LUKS2")), "{bad:#?}");
    }

    #[test]
    fn the_passphrase_path_and_its_one_keyslot_pass_and_a_yubikey_needs_a_recovery_slot() {
        let mut f = good();
        set(&mut f, "/crypttab", json!([]));
        assert!(failing(&f).is_empty(), "{:#?}", failing(&f));
        let unlock = judge("rpi5", &rpi5(), &f).into_iter().find(|r| r.name == "unlock").unwrap();
        assert_eq!(unlock.detail, "console-passphrase");
        // A key file on removable media, read by passdev: the model's usb-key.
        let mut k = good();
        set(&mut k, "/crypttab/0/key", json!("/dev/disk/by-label/KEYS:/rootfs.lek"));
        set(&mut k, "/crypttab/0/options", json!(["luks", "initramfs", "keyscript=passdev"]));
        let unlock = judge("rpi5", &rpi5(), &k).into_iter().find(|r| r.name == "unlock").unwrap();
        assert!(unlock.ok && unlock.detail == "usb-key", "{unlock:?}");
        let mut f = good();
        f["luks"]["metadata"]["keyslots"].as_object_mut().unwrap().remove("1");
        assert!(failing(&f).iter().any(|(n, d)| *n == "unlock" && d.contains("no recovery passphrase")), "{:#?}", failing(&f));
        let mut f = good();
        set(&mut f, "/crypttab", json!([]));
        set(&mut f, "/initramfs/cryptsetup", json!(false));
        assert!(failing(&f).iter().any(|(n, d)| *n == "unlock" && d.contains("has no cryptsetup")), "{:#?}", failing(&f));
    }

    #[test]
    fn crypttab_is_matched_by_name_device_or_uuid_and_an_entry_for_another_device_is_unknown() {
        let mut f = good();
        set(&mut f, "/crypttab/0/name", json!("root_crypt"));
        set(&mut f, "/crypttab/0/device", json!("UUID=e6b2f910-123a-40bf-a11f-f7b98b35b522"));
        let unlock = judge("rpi5", &rpi5(), &f).into_iter().find(|r| r.name == "unlock").unwrap();
        assert!(unlock.ok && unlock.detail.starts_with("yubikey-hmac"), "{unlock:?}");
        set(&mut f, "/crypttab/0/device", json!("UUID=00000000-0000-0000-0000-000000000000"));
        let bad = failing(&f);
        assert!(bad.len() == 1 && bad[0].0 == "unlock" && bad[0].1.contains("unknown: crypttab has entries (root_crypt), and none names this root's mapping rootfs"), "{bad:#?}");
        let mut f = good();
        set(&mut f, "/mapping", json!({"error": "cryptsetup: no such device"}));
        assert!(failing(&f).iter().any(|(n, d)| *n == "unlock" && d.contains("unknown: the root's active mapping")), "{:#?}", failing(&f));
    }

    #[test]
    fn the_initramfs_ssh_passphrase_is_delivered_and_a_second_way_in_beside_the_yubikey_is_held_to_the_model() {
        let mut f = good();
        set(&mut f, "/crypttab", json!([]));
        set(&mut f, "/initramfs/dropbear", json!(true));
        let unlock = judge("rpi5", &rpi5(), &f).into_iter().find(|r| r.name == "unlock").unwrap();
        assert!(unlock.ok && unlock.detail == "console-passphrase, ssh-passphrase", "{unlock:?}");
        let mut d = rpi5();
        d.unlock.retain(|u| u != "ssh-passphrase");
        let bad: Vec<Row> = judge("rpi5", &d, &f).into_iter().filter(|r| !r.ok).collect();
        assert!(bad.len() == 1 && bad[0].detail.contains("unlocks by ssh-passphrase, which the rpi5 does not deliver"), "{bad:#?}");
        // A Tang binding beside the YubiKey is a way in the model does not deliver.
        let mut f = good();
        set(&mut f, "/luks/metadata/tokens", json!({"0": {"type": "clevis"}}));
        let bad = failing(&f);
        assert!(bad.len() == 1 && bad[0].1.contains("token clevis"), "{bad:#?}");
    }

    #[test]
    fn a_model_entry_this_check_cannot_hold_a_machine_to_is_refused_with_its_code() {
        let real = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ops/goatbuilder/families/small-computer.yaml")).unwrap();
        let code = |e: &anyhow::Error| e.downcast_ref::<goat_errors::Coded>().map(|c| c.code.clone());
        for (from, to, says) in [
            ("    end_state:\n      board: Raspberry Pi 5 Model B\n      root_media: nvme\n      fips: userspace\n", "", "rpi5 has no end_state"),
            ("      scope: root\n", "      scope: data\n", "delivers an encrypted data"),
            ("    arch: arm64\n", "    arch: unknown\n", "of architecture unknown"),
        ] {
            assert!(real.contains(from), "{from}");
            let t = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(t.path().join("ops/goatbuilder/families")).unwrap();
            std::fs::write(t.path().join("ops/goatbuilder/families/small-computer.yaml"), real.replacen(from, to, 1)).unwrap();
            let targets = crate::targets::load(t.path());
            // A scope no type accepts is refused when the files are read, before this check.
            let Ok(targets) = targets else { continue };
            let e = model(targets.models(FAMILY)).unwrap_err();
            assert_eq!(code(&e).as_deref(), Some("GB-410"), "{e:#}");
            assert!(format!("{e:#}").contains(says) && format!("{e:#}").contains("what the small computer's model delivers cannot be read"), "{from}: {e:#}");
        }
        let e = model(None).unwrap_err();
        assert_eq!(code(&e).as_deref(), Some("GB-410"));
    }

    #[test]
    fn ubuntu_26_04_is_accepted_and_said_to_be_outside_what_the_builder_measured() {
        let mut f = good();
        set(&mut f, "/os/version_id", json!("26.04"));
        let os = judge("rpi5", &rpi5(), &f).into_iter().find(|r| r.name == "os").unwrap();
        assert!(os.ok && os.detail.contains("install 24.04 only"), "{os:?}");
    }

    #[test]
    fn a_fact_that_was_not_read_is_unknown_and_never_a_pass() {
        for (ptr, row) in [("/os", "os"), ("/board", "board"), ("/root", "root"), ("/luks", "primitives"), ("/luks/metadata", "primitives"), ("/crypttab", "unlock"), ("/openssl_providers", "fips"), ("/kernel_fips", "fips"), ("/sshd", "ssh"), ("/first_boot/status", "first boot")] {
            let mut f = good();
            set(&mut f, ptr, json!({"error": "cannot read"}));
            let bad = failing(&f);
            let r = bad.iter().find(|(n, _)| *n == row).unwrap_or_else(|| panic!("{ptr}: {bad:#?}"));
            assert!(r.1.contains("unknown"), "{ptr}: {r:?}");
        }
        let mut f = good();
        f.as_object_mut().unwrap().remove("sshd");
        assert!(failing(&f).iter().any(|(n, d)| *n == "ssh" && d.contains("not reported")));
    }

    #[test]
    fn a_kernel_that_must_hold_fips_mode_is_held_to_it() {
        let mut d = rpi5();
        d.fips = Fips::Kernel;
        let bad: Vec<Row> = judge("rpi5", &d, &good()).into_iter().filter(|r| !r.ok).collect();
        assert!(bad.len() == 1 && bad[0].detail.contains("kernel's FIPS mode is absent"), "{bad:#?}");
    }

    #[test]
    fn the_rpi5_entry_is_read_and_a_family_of_two_models_is_not_guessed() {
        let d = rpi5();
        assert_eq!((d.machine.as_str(), d.fips, d.root_media.as_str()), ("aarch64", Fips::Userspace, "nvme"));
        assert_eq!((d.os_builder.clone(), d.os_accepted.clone()), (vec!["24.04".to_string()], vec!["24.04".to_string(), "26.04".to_string()]));
        assert_eq!(d.unlock, ["yubikey-hmac", "usb-key", "console-passphrase", "ssh-passphrase"]);
        let mut two = match shipped().models(FAMILY) {
            Some(crate::targets::Models::Described(m)) => m.clone(),
            _ => unreachable!(),
        };
        let one = two.values().next().unwrap().clone();
        two.insert("other".into(), one);
        let e = model(Some(&crate::targets::Models::Described(two))).unwrap_err();
        assert!(format!("{e}").contains("2 models"), "{e}");
        assert!(model(shipped().models("host-class")).is_err(), "a Karbon has no end_state");
    }

    #[test]
    fn a_read_that_does_not_answer_in_time_is_stopped() {
        let child = std::process::Command::new("sh").args(["-c", "echo started; sleep 30"]).stdout(std::process::Stdio::piped()).spawn().unwrap();
        let t = std::time::Instant::now();
        assert_eq!(with_deadline(child, std::time::Duration::from_millis(500)).unwrap_err(), "timed out");
        assert!(t.elapsed() < std::time::Duration::from_secs(10));
        let child = std::process::Command::new("sh").args(["-c", "echo done"]).stdout(std::process::Stdio::piped()).spawn().unwrap();
        assert_eq!(with_deadline(child, std::time::Duration::from_secs(10)).unwrap(), b"done\n");
    }

    #[test]
    fn ansibles_answer_is_unwrapped_and_a_refusal_is_not_facts() {
        let ok = "pi-1 | CHANGED => {\n    \"rc\": 0,\n    \"stdout\": \"{\\\"arch\\\": \\\"aarch64\\\"}\"\n}\n";
        assert_eq!(facts(ok).unwrap()["arch"], "aarch64");
        assert!(facts("pi-1 | UNREACHABLE! => {\n \"msg\": \"no route\"\n}").unwrap_err().contains("UNREACHABLE"));
        assert!(facts("").is_err());
    }
}
