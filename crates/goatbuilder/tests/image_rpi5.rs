//! `goatbuilder image rpi5`, through the built binary. `ssh` is a stand-in on
//! PATH that plays the Pi: it runs the real collector
//! (ops/scripts/small-computer-endstate.py) against the stand-in machine of
//! tests/fixtures/small-computer/machine.sh, as root (the stand-in for `sudo -n`
//! sets that fact, and the board's architecture), answers `sha256sum` for its
//! card's copies of the builder's scripts, and records each script it is asked
//! to run. Where cryptsetup is not installed the machine cannot be made, and
//! those tests say so and do not run.

#[path = "../src/test_support.rs"]
mod test_support;

use std::path::{Path, PathBuf};
use std::process::Command;

fn real() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), &t).unwrap();
        }
    }
}

struct World {
    root: tempfile::TempDir,
    machine: Option<tempfile::TempDir>,
}

impl World {
    /// A checkout with the builder, the model's entry and the collector; a card
    /// holding the builder's scripts; the stand-in Pi, when it can be made.
    fn new() -> World {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        copy_dir(&real().join("ops/rpi-base-image"), &r.join("ops/rpi-base-image"));
        // The family files, whose rpi5 entry pins the golden image.
        copy_dir(&real().join("ops/goatbuilder/families"), &r.join("ops/goatbuilder/families"));
        test_support::copied_executable(&real().join("ops/scripts/small-computer-endstate.py"), &r.join("ops/scripts/small-computer-endstate.py"));
        // The card's boot partition, as 01-flash.sh fills it: the helper scripts at the top as well.
        copy_dir(&real().join("ops/rpi-base-image"), &r.join("card"));
        for f in ["encrypt-rootfs.sh", "encrypt-rootfs-unlock.sh"] {
            std::fs::copy(real().join("ops/rpi-base-image/helper-scripts").join(f), r.join("card").join(f)).unwrap();
        }
        test_support::executable(
            &r.join("bin/ssh"),
            r#"#!/bin/sh
echo "ssh $*" >> "$GB_CALLS"
while [ $# -gt 0 ]; do case "$1" in -o) shift 2 ;; -t) shift ;; *) break ;; esac; done
host="$1"; shift
[ "$host" = "pi@down" ] && { echo "ssh: connect to host down port 22: No route to host" >&2; exit 255; }
case "$*" in
  "sudo -n python3 -")
    SMALL_COMPUTER_ROOT="$GB_MACHINE/root" SMALL_COMPUTER_PATH="$GB_MACHINE/bin:/usr/sbin:/usr/bin:/sbin:/bin" python3 - \
      | python3 -c 'import json, sys; f = json.load(sys.stdin); f["root_user"] = True; f["arch"] = "aarch64"; print(json.dumps(f))' ;;
  "sha256sum /boot/firmware/"*) sha256sum "$GB_CARD/${2#/boot/firmware/}" ;;
  "sudo bash /boot/firmware/"*) [ -z "${GB_ON_RUN:-}" ] || sh -c "$GB_ON_RUN"; exit "${GB_SCRIPT_EXIT:-0}" ;;
  *) echo "stand-in ssh: unexpected: $*" >&2; exit 99 ;;
esac
"#,
        );
        let machine = if Path::new("/usr/sbin/cryptsetup").exists() {
            let m = tempfile::tempdir().unwrap();
            let o = Command::new("bash").arg(real().join("tests/fixtures/small-computer/machine.sh")).arg(m.path()).output().unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            Some(m)
        } else {
            assert!(std::env::var_os("GOAT_TEST_REQUIRE_CRYPTSETUP").is_none(), "GOAT_TEST_REQUIRE_CRYPTSETUP is set and there is no /usr/sbin/cryptsetup: the stand-in Pi cannot be made");
            eprintln!("SKIP (not run, not a pass): no /usr/sbin/cryptsetup here, so the stand-in Pi cannot be made");
            None
        };
        World { root, machine }
    }

    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> (i32, String) {
        let r = self.root.path();
        let mut c = Command::new(env!("CARGO_BIN_EXE_goatbuilder"));
        c.args(["-o", "json", "-C", r.to_str().unwrap(), "image", "rpi5"])
            .args(args)
            .env_clear()
            .env("PATH", format!("{}:/usr/sbin:/usr/bin:/sbin:/bin", r.join("bin").display()))
            .env("HOME", r.join("home"))
            .env("GB_CALLS", r.join("calls"))
            .env("GB_CARD", r.join("card"))
            .env("GB_MACHINE", self.machine.as_ref().map(|m| m.path().to_path_buf()).unwrap_or_default())
            .env("PYTHONDONTWRITEBYTECODE", "1");
        for (k, v) in env {
            c.env(k, v);
        }
        let o = c.output().unwrap();
        (o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    }

    fn calls(&self) -> String {
        std::fs::read_to_string(self.root.path().join("calls")).unwrap_or_default()
    }

    fn machine(&self) -> Option<&Path> {
        self.machine.as_ref().map(|m| m.path())
    }
}

type Row = (String, bool, String);

fn rows(out: &str) -> Vec<Row> {
    let at = out.find('{').unwrap_or_else(|| panic!("no JSON: {out}"));
    let v: serde_json::Value = serde_json::Deserializer::from_str(&out[at..]).into_iter().next().unwrap().unwrap_or_else(|e| panic!("{e}: {out}"));
    v["checks"].as_array().unwrap().iter().map(|c| (c["name"].as_str().unwrap().to_string(), c["ok"] == true, c["detail"].as_str().unwrap().to_string())).collect()
}

fn row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
    rows.iter().find(|r| r.0 == name).unwrap_or_else(|| panic!("no row {name}: {rows:#?}"))
}

#[test]
fn the_plan_names_every_step_what_runs_it_and_what_proves_it_and_reads_nothing() {
    let w = World::new();
    let (code, out) = w.run(&["--dry-run"], &[]);
    assert_eq!(code, 0, "{out}");
    let rs = rows(&out);
    let names: Vec<&str> = rs.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(names, ["step 00 prerequisites", "step 01 flash", "step 01b verify-image", "step - boot", "step 02 clone", "step 03 prepare-encryption", "step 03 encrypt", "step 04 yubikey", "step 05 fips", "step 06 verify-box"]);
    assert!(row(&rs, "step 02 clone").2.contains("runs: ssh -t <--host> sudo bash /boot/firmware/02-clone-to-nvme.sh; then a person: on its first pass 02 applies the NVMe power fix"));
    assert!(row(&rs, "step 03 encrypt").2.contains("directions: sudo reboot; at the HDMI console"));
    let (_, out) = w.run(&["--dry-run", "--unlock", "passphrase", "--headless"], &[]);
    let rs = rows(&out);
    assert!(!rs.iter().any(|r| r.0.contains("yubikey")), "{rs:#?}");
    assert!(row(&rs, "step 03b headless-unlock").2.contains("03b-headless-dropbear-unlock.sh"));
    assert!(row(&rs, "step 03 encrypt").2.contains("ssh root@<its address>"));
    assert_eq!(w.calls(), "", "a plan reaches nothing");
}

#[test]
fn a_model_this_builder_does_not_build_and_a_step_it_does_not_have_are_refused() {
    let w = World::new();
    let r = w.root.path().to_str().unwrap().to_string();
    let o = Command::new(env!("CARGO_BIN_EXE_goatbuilder")).args(["-C", &r, "image", "karbon-801"]).env("HOME", w.root.path()).output().unwrap();
    let out = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(1), "{out}");
    assert!(out.contains("GB-411") && out.contains("build-image-karbon.sh"), "{out}");
    let (code, out) = w.run(&["--step", "yubikey", "--unlock", "passphrase"], &[]);
    assert!(code == 1 && out.contains("GB-411") && out.contains("has no step 'yubikey'"), "{out}");
}

#[test]
fn a_step_without_what_it_needs_is_refused_before_anything_runs() {
    let w = World::new();
    let (code, out) = w.run(&["--step", "flash"], &[]);
    assert!(code == 1 && out.contains("GB-412") && out.contains("--device"), "{out}");
    let (code, out) = w.run(&["--step", "clone"], &[]);
    assert!(code == 1 && out.contains("GB-412") && out.contains("--host"), "{out}");
    let (code, out) = w.run(&["--step", "clone", "--host", "pi@down"], &[]);
    assert!(code == 1 && out.contains("the step before it, boot") && out.contains("No route to host"), "{out}");
    assert!(!w.calls().contains("sudo bash"), "{}", w.calls());
}

#[test]
fn the_golden_image_is_held_to_its_pin_and_user_data_to_a_key() {
    let w = World::new();
    let img = w.root.path().join("fake.img.xz");
    std::fs::write(&img, b"not the image").unwrap();
    let (code, out) = w.run(&["--step", "prerequisites", "--check", "--image", img.to_str().unwrap()], &[]);
    assert_eq!(code, 1, "{out}");
    let r = row(&rows(&out), "step 00 prerequisites").clone();
    assert!(!r.1 && r.2.contains("and the model pins 790652fa"), "{r:?}");
    // The checkout's pin moved to this file, and a key in user-data: it holds.
    let classes = w.root.path().join("ops/goatbuilder/families/small-computer.yaml");
    let o = Command::new("sha256sum").arg(&img).output().unwrap();
    let got = String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap().to_string();
    let text = std::fs::read_to_string(&classes).unwrap().replace("790652faeb4f61ce7bb12f5cb61734595c61d3cd882915b8b5f9918106c80d37", &got);
    std::fs::write(&classes, text).unwrap();
    let (_, out) = w.run(&["--step", "prerequisites", "--check", "--image", img.to_str().unwrap()], &[]);
    assert!(row(&rows(&out), "step 00 prerequisites").2.contains("SSH key placeholder"), "{out}");
    let ud = w.root.path().join("ops/rpi-base-image/cloud-init-configs/user-data");
    std::fs::write(&ud, std::fs::read_to_string(&ud).unwrap().replace("REPLACE_WITH_YOUR_PUBLIC_KEY", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5TestKeyOnly test")).unwrap();
    let (code, out) = w.run(&["--step", "prerequisites", "--check", "--image", img.to_str().unwrap()], &[]);
    let r = row(&rows(&out), "step 00 prerequisites").clone();
    assert!(code == 0 && r.1 && r.2.starts_with("holds:"), "{out}");
}

#[test]
fn a_pi_as_the_builder_leaves_it_is_verified_and_nothing_runs() {
    let w = World::new();
    let Some(_) = w.machine() else { return };
    let (code, out) = w.run(&["--host", "pi@zagros-bg-1"], &[]);
    let rs = rows(&out);
    for s in ["step - boot", "step 02 clone", "step 03 prepare-encryption", "step 03 encrypt", "step 04 yubikey", "step 05 fips", "step 06 verify-box"] {
        let r = row(&rs, s);
        assert!(r.1 && r.2.starts_with("holds:"), "{r:?}");
    }
    // The card is in the Pi: the workstation's steps stand on the later proofs.
    let r = row(&rs, "step 01b verify-image");
    assert!(r.1 && r.2.starts_with("not checked: the machine has booted from the card since"), "{r:?}");
    assert!(row(&rs, "next").2.starts_with("none: the rpi5 is verified; next is the host baseline"), "{rs:#?}");
    assert_eq!(code, 0, "{out}");
    assert!(!w.calls().contains("sudo bash"), "{}", w.calls());
}

#[test]
fn a_step_runs_the_cards_own_copy_after_the_step_before_is_proven_and_ends_at_its_reboot() {
    let w = World::new();
    let Some(m) = w.machine() else { return };
    // A Pi on its first boot, from the card: the root is plain, on the SD card.
    std::fs::write(m.join("bin/lsblk"), "#!/bin/sh\necho '{\"blockdevices\": [{\"name\": \"/dev/mmcblk0p2\", \"type\": \"part\", \"fstype\": \"ext4\", \"children\": [{\"name\": \"/dev/mmcblk0\", \"type\": \"disk\", \"fstype\": null}]}]}'\n").unwrap();
    let (code, out) = w.run(&["--host", "pi@zagros-bg-1"], &[]);
    assert_eq!(code, 3, "{out}");
    let rs = rows(&out);
    assert!(row(&rs, "step - boot").1, "{rs:#?}");
    assert!(!row(&rs, "step 02 clone").1 && row(&rs, "step 02 clone").2.contains("the root's disk is /dev/mmcblk0, not nvme"), "{rs:#?}");
    assert_eq!(row(&rs, "next").2, "goatbuilder image rpi5 --step clone --host pi@zagros-bg-1");
    let (code, out) = w.run(&["--step", "clone", "--host", "pi@zagros-bg-1"], &[]);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("take the SD card out") && out.contains("--step clone --check --host pi@zagros-bg-1"), "{out}");
    assert!(w.calls().lines().any(|l| l == "ssh -t pi@zagros-bg-1 sudo bash /boot/firmware/02-clone-to-nvme.sh"), "{}", w.calls());
    // The script failed: GB-413, in its own words' place.
    let (code, out) = w.run(&["--step", "clone", "--host", "pi@zagros-bg-1"], &[("GB_SCRIPT_EXIT", "1")]);
    assert!(code == 1 && out.contains("GB-413") && out.contains("02-clone-to-nvme.sh exited 1"), "{out}");
    // The card's copy is not this checkout's: refused, and nothing runs.
    let before = w.calls().lines().filter(|l| l.contains("sudo bash")).count();
    std::fs::write(w.root.path().join("card/02-clone-to-nvme.sh"), "#!/bin/sh\necho edited on the Pi\n").unwrap();
    let (code, out) = w.run(&["--step", "clone", "--host", "pi@zagros-bg-1"], &[]);
    assert!(code == 1 && out.contains("GB-412") && out.contains("flash the card again from this checkout"), "{out}");
    assert_eq!(w.calls().lines().filter(|l| l.contains("sudo bash")).count(), before);
}

#[test]
fn a_step_whose_step_before_does_not_hold_is_refused_naming_it() {
    let w = World::new();
    let Some(m) = w.machine() else { return };
    // The YubiKey was never enrolled (crypttab has no keyscript), and FIPS mode is not on.
    std::fs::write(m.join("root/etc/crypttab"), "").unwrap();
    std::fs::write(m.join("root/proc/cmdline"), "console=tty1 root=/dev/mapper/rootfs cryptdevice=/dev/nvme0n1p2:rootfs\n").unwrap();
    let (code, out) = w.run(&["--step", "fips", "--host", "pi@zagros-bg-1"], &[]);
    assert!(code == 1 && out.contains("GB-412") && out.contains("the step before it, yubikey (04), is not proven") && out.contains("--step yubikey --check"), "{out}");
    assert!(!w.calls().contains("05-enable-fips.sh"), "{}", w.calls());
    // With the passphrase as the unlock, the step before is the encryption, which holds.
    let (code, out) = w.run(&["--step", "fips", "--host", "pi@zagros-bg-1", "--unlock", "passphrase"], &[]);
    assert_eq!(code, 3, "{out}");
    assert!(w.calls().contains("sudo bash /boot/firmware/05-enable-fips.sh"), "{}", w.calls());
    // The proof read alone: an SSH that takes passwords does not hold.
    std::fs::write(m.join("bin/sshd"), std::fs::read_to_string(m.join("bin/sshd")).unwrap().replace("passwordauthentication no", "passwordauthentication yes")).unwrap();
    let (code, out) = w.run(&["--step", "fips", "--check", "--host", "pi@zagros-bg-1", "--unlock", "passphrase"], &[]);
    let r = row(&rows(&out), "step 05 fips").clone();
    assert!(code == 1 && !r.1 && r.2.contains("SSH is not key-only"), "{out}");
}

/// A small Pi-shaped image and a card "flashed" from it, made without root:
/// an MBR, a FAT boot partition with this checkout's copy list on the card,
/// a root partition of random bytes. `None` where sfdisk, mtools or xz is missing.
fn card(w: &World) -> Option<(PathBuf, PathBuf)> {
    let have = |t: &str| Command::new("sh").args(["-c", &format!("command -v {t}")]).env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").output().is_ok_and(|o| o.status.success());
    if !["sfdisk", "mformat", "mcopy", "mmd", "xz"].iter().all(|t| have(t)) {
        assert!(std::env::var_os("GOAT_TEST_REQUIRE_CARD_TOOLS").is_none(), "GOAT_TEST_REQUIRE_CARD_TOOLS is set and sfdisk, mtools or xz is missing: no card can be made");
        eprintln!("SKIP (not run, not a pass): no sfdisk, mtools or xz here, so no card can be made");
        return None;
    }
    let r = w.root.path();
    let ud = r.join("ops/rpi-base-image/cloud-init-configs/user-data");
    std::fs::write(&ud, std::fs::read_to_string(&ud).unwrap().replace("REPLACE_WITH_YOUR_PUBLIC_KEY", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5TestKeyOnly test")).unwrap();
    let script = r#"set -eu
cd "$1"; b=ops/rpi-base-image; export MTOOLS_SKIP_CHECK=1
truncate -s 64M img
printf 'label: dos\nstart=2048, size=32768, type=c\nstart=34816, type=83\n' | sfdisk -q img
mformat -i img@@1048576 -T 32768 -v system-boot ::
head -c 2000000 /dev/urandom | dd of=img bs=512 seek=34816 conv=notrunc status=none
xz -k -c img > img.xz; cp img sd.img; mmd -i sd.img@@1048576 ::/helper-scripts
for p in "cloud-init-configs/user-data user-data" "cloud-init-configs/network-config network-config" "02-clone-to-nvme.sh 02-clone-to-nvme.sh" \
  "03-prepare-encryption.sh 03-prepare-encryption.sh" "03b-headless-dropbear-unlock.sh 03b-headless-dropbear-unlock.sh" "04-enroll-yubikey.sh 04-enroll-yubikey.sh" \
  "05-enable-fips.sh 05-enable-fips.sh" "helper-scripts/encrypt-rootfs.sh encrypt-rootfs.sh" "helper-scripts/encrypt-rootfs-unlock.sh encrypt-rootfs-unlock.sh"; do
  set -- $p; if [ -f "$b/$1" ]; then mcopy -o -i sd.img@@1048576 "$b/$1" "::/$2"; fi
done
for f in "$b"/helper-scripts/*; do mcopy -o -i sd.img@@1048576 "$f" "::/helper-scripts/$(basename "$f")"; done
sha256sum img.xz | cut -d' ' -f1 > img.sha
"#;
    let o = Command::new("bash").args(["-c", script, "card", r.to_str().unwrap()]).env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let classes = r.join("ops/goatbuilder/families/small-computer.yaml");
    let sha = std::fs::read_to_string(r.join("img.sha")).unwrap();
    std::fs::write(&classes, std::fs::read_to_string(&classes).unwrap().replace("790652faeb4f61ce7bb12f5cb61734595c61d3cd882915b8b5f9918106c80d37", sha.trim())).unwrap();
    Some((r.join("sd.img"), r.join("img.xz")))
}

#[test]
fn the_card_is_read_back_by_the_builders_own_script_and_a_changed_byte_is_named() {
    let w = World::new();
    let Some((card, img)) = card(&w) else { return };
    let (d, i) = (card.to_str().unwrap(), img.to_str().unwrap());
    let (code, out) = w.run(&["--step", "verify-image", "--device", d, "--image", i], &[]);
    assert_eq!(code, 0, "{out}");
    let r = row(&rows(&out), "step 01b verify-image").clone();
    assert!(r.1 && r.2.starts_with("holds:"), "{r:?}");
    assert!(row(&rows(&out), "next").2.contains("--step boot"), "{out}");
    let mut b = std::fs::read(&card).unwrap();
    b[34816 * 512 + 777] ^= 1;
    std::fs::write(&card, b).unwrap();
    let (code, out) = w.run(&["--step", "verify-image", "--check", "--device", d, "--image", i], &[]);
    let r = row(&rows(&out), "step 01b verify-image").clone();
    assert!(code == 1 && !r.1 && r.2.starts_with("does not hold: root partition: sha256") && !r.2.contains('\u{1b}'), "{r:?}");
    let (code, out) = w.run(&["--step", "verify-image", "--device", d, "--image", i], &[]);
    assert!(code == 1 && out.contains("GB-413") && out.contains("root partition"), "{out}");
}

#[test]
fn a_host_or_device_that_reads_as_an_option_is_refused() {
    let w = World::new();
    for (flag, v) in [("--host", "-oProxyCommand=touch pwned"), ("--host", "pi@zagros bg"), ("--device", "--dry-run")] {
        let (code, out) = w.run(&[&format!("{flag}={v}")], &[]);
        assert!(code == 1 && out.contains("GB-412") && out.contains("may not start with '-'"), "{v}: {out}");
    }
    assert_eq!(w.calls(), "", "nothing reached ssh");
}

#[test]
fn a_step_whose_proof_already_holds_runs_nothing_and_a_failing_card_row_stays_red() {
    let w = World::new();
    let Some(_) = w.machine() else { return };
    let (code, out) = w.run(&["--step", "fips", "--host", "pi@zagros-bg-1"], &[]);
    assert!(code == 0 && out.contains("it already held, so nothing ran"), "{out}");
    assert!(!w.calls().contains("sudo bash"), "{}", w.calls());
    // An image that is not the pinned one fails, and a verified Pi does not hide it.
    let img = w.root.path().join("other.img.xz");
    std::fs::write(&img, b"another image").unwrap();
    let (code, out) = w.run(&["--host", "pi@zagros-bg-1", "--image", img.to_str().unwrap()], &[]);
    let rs = rows(&out);
    assert!(code == 3 && !row(&rs, "step 00 prerequisites").1 && row(&rs, "step 00 prerequisites").2.contains("the model pins"), "{rs:#?}");
}

#[test]
fn prepare_encryption_reads_its_proof_after_the_script_and_from_the_next_boots_command_line() {
    let w = World::new();
    let Some(m) = w.machine() else { return };
    // Cloned, not yet prepared: the next boot's command line is the stock one.
    let stock = "console=serial0,115200 console=tty1 root=PARTUUID=00000000-02 rootfstype=ext4 rootwait\n";
    std::fs::write(m.join("root/boot/firmware/cmdline.txt"), stock).unwrap();
    let after = format!("cp {} {}", m.join("root/proc/cmdline").display(), m.join("root/boot/firmware/cmdline.txt").display());
    let (code, out) = w.run(&["--step", "prepare-encryption", "--host", "pi@zagros-bg-1"], &[("GB_ON_RUN", &after)]);
    let r = row(&rows(&out), "step 03 prepare-encryption").clone();
    assert!(code == 0 && r.1 && r.2.contains("root=/dev/mapper/rootfs cryptdevice=/dev/nvme0n1p2:rootfs"), "{out}");
    assert!(w.calls().contains("sudo bash /boot/firmware/03-prepare-encryption.sh"), "{}", w.calls());
    // The script ran and changed nothing: its proof does not hold.
    std::fs::write(m.join("root/boot/firmware/cmdline.txt"), stock).unwrap();
    let (code, out) = w.run(&["--step", "prepare-encryption", "--host", "pi@zagros-bg-1"], &[]);
    assert!(code == 1 && out.contains("GB-413") && out.contains("no root=/dev/mapper/<name>"), "{out}");
}

#[test]
fn a_persons_step_prints_its_directions_only_once_the_step_before_holds() {
    let w = World::new();
    let Some(m) = w.machine() else { return };
    // Not prepared: no directions for the console.
    std::fs::write(m.join("root/boot/firmware/cmdline.txt"), "console=tty1 root=PARTUUID=00000000-02 rootwait\n").unwrap();
    std::fs::write(m.join("bin/lsblk"), "#!/bin/sh\necho '{\"blockdevices\": [{\"name\": \"/dev/nvme0n1p2\", \"type\": \"part\", \"fstype\": \"ext4\", \"children\": [{\"name\": \"/dev/nvme0n1\", \"type\": \"disk\", \"fstype\": null}]}]}'\n").unwrap();
    let (code, out) = w.run(&["--step", "encrypt", "--host", "pi@zagros-bg-1"], &[]);
    assert!(code == 1 && out.contains("the step before it, prepare-encryption (03), is not proven"), "{out}");
    let (code, out) = w.run(&["--step", "encrypt"], &[]);
    assert!(code == 1 && out.contains("--host"), "{out}");
    // Prepared: the directions, after the card's encryption scripts are checked.
    std::fs::copy(m.join("root/proc/cmdline"), m.join("root/boot/firmware/cmdline.txt")).unwrap();
    let (code, out) = w.run(&["--step", "encrypt", "--host", "pi@zagros-bg-1"], &[]);
    assert!(code == 3 && out.contains("a person, now: sudo reboot; at the HDMI console") && out.contains("--step encrypt --check"), "{out}");
    assert!(w.calls().contains("sha256sum /boot/firmware/encrypt-rootfs.sh") && w.calls().contains("sha256sum /boot/firmware/helper-scripts/encrypt-rootfs.sh"), "{}", w.calls());
    std::fs::write(w.root.path().join("card/encrypt-rootfs.sh"), "#!/bin/sh\n").unwrap();
    let (code, out) = w.run(&["--step", "encrypt", "--host", "pi@zagros-bg-1"], &[]);
    assert!(code == 1 && out.contains("GB-412") && out.contains("/boot/firmware/encrypt-rootfs.sh"), "{out}");
    // A Pi still on its first boot: the boot's directions need the card read back first.
    std::fs::write(m.join("bin/cloud-init"), "#!/bin/sh\nprintf 'status: running\\n'\n").unwrap();
    std::fs::remove_file(m.join("root/var/lib/cloud/instance/boot-finished")).unwrap();
    let (code, out) = w.run(&["--step", "boot", "--host", "pi@zagros-bg-1"], &[]);
    assert!(code == 1 && out.contains("verify-image") && out.contains("--device"), "{out}");
}
