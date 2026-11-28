// Gate rows H3, H4 and H5 (docs/design/goatbuilder-v1-gate.md), on the phone
// site `delta` of the fixture `ops/goatbuilder/fixtures/join-node-third-site`:
//
//   H3  `--via usb`: the tool reaches the node through the Android debug
//       bridge, with no SSH configuration written by hand.
//   H4  For a phone, phase 2 prepares the phone's Linux VM (kernel, container
//       engine, deploy user) and refuses to continue unless the integrity
//       checks pass.
//   H5  The machine class `phone` carries the declared differences once: the
//       site passes the config preflight with none of them written by hand.
//
// Nothing here reaches a device or a network. The fixture is copied with the
// real phone scripts, the real config preflight and the real class file.
// `adb` is a stand-in that answers for a phone. `ssh` is a stand-in that does
// what ssh does with `-F <file>`: it runs the proxy command the file names,
// which is the real `phone-node-ssh.sh --proxy` calling the stand-in `adb`,
// and then runs what is piped to the VM (`bash -s`) against a directory that
// stands for the VM's filesystem, with stand-ins for the VM's own tools. A VM
// start boots what the Terminal app's boot folder then holds, so the kernel
// the real guest script stages is the kernel the next integrity check sees.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const NODE: &str = "delta-phone-1";
const DEBIAN_KERNEL: &str = "6.12.0-cloud-arm64";
// Two words, as a public key file holds them; not a key.
const PUBKEY: &str = "stand-in-key-type stand-in-key-material";

/// The file a shell would run for `tool` on `path`: the first one of that name
/// in the path's directories that is a file with an execute bit set. A file of
/// the name that is not executable is passed over, as the shell passes it.
fn host_tool(path: &std::ffi::OsStr, tool: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path).map(|d| d.join(tool)).find(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), &dest).unwrap();
        }
    }
}

fn executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Site {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    checkout: PathBuf,
    /// Every argument list a stand-in was started with, one per line.
    log: PathBuf,
    /// Flags the stand-ins read: the VM's booted kernel, a check made to fail.
    state: PathBuf,
    /// The directory that stands for the VM's filesystem.
    vm: PathBuf,
    /// The node's mesh address once enrolled, in the fixture's synthetic range.
    mesh: String,
}

impl Site {
    fn usb_base(&self) -> String {
        format!(
            "{}/.goat/state/join-node-delta-{NODE}-usb",
            self.root.display()
        )
    }
    fn flag(&self, name: &str) {
        std::fs::write(self.state.join(name), "").unwrap();
    }
    fn unflag(&self, name: &str) {
        std::fs::remove_file(self.state.join(name)).unwrap();
    }
    /// What the stand-ins recorded. Nothing of this machine's own was started
    /// in place of a tool of the VM.
    fn log(&self) -> String {
        let log = std::fs::read_to_string(&self.log).unwrap_or_default();
        assert!(
            !log.lines().any(|l| l.starts_with("host:")),
            "a program of this machine ran in place of the VM's:\n{log}"
        );
        log
    }
}

/// `prepared`: the VM already runs Debian's kernel with the boot settings,
/// Docker Engine and the deploy user. Otherwise it is as the Terminal app
/// installs it: the kernel Android supplies, and its first user `droid`.
fn site(prepared: bool) -> Site {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let checkout = root.join("checkout");
    let r = repo_root();
    copy_tree(
        &r.join("ops/goatbuilder/fixtures/join-node-third-site"),
        &checkout,
    );
    std::fs::remove_file(checkout.join("README.md")).unwrap();
    copy_tree(
        &r.join("ops/goatbuilder/endstate"),
        &checkout.join("ops/goatbuilder/endstate"),
    );
    std::fs::create_dir_all(checkout.join("ops/scripts")).unwrap();
    // The data router's play: a run is refused without it (GB-377). ansible is
    // a stand-in, so an empty one is enough.
    std::fs::write(checkout.join("ops/ansible/zenoh-deploy.yml"), "---\n").unwrap();
    std::fs::create_dir_all(checkout.join("ops/secrets")).unwrap();
    std::fs::create_dir_all(checkout.join("ops/federation/standup")).unwrap();
    for s in [
        // The real gates, mesh record and secrets injector, as for H1 and H2.
        "ops/scripts/standup-config-preflight.py",
        // Phase 3's clock check and the federation clock code it shares.
        "ops/scripts/join-node-clock-check.py",
        "ops/federation/standup/preflight_live.py",
        "ops/scripts/signal-federation-check.py",
        "ops/scripts/standup-mesh-record.py",
        "ops/scripts/inventory-hosts.py",
        "ops/scripts/with-secrets",
        "ops/secrets/posture.py",
        // The gate before a joiner's mesh DNS step (phase 13).
        "ops/scripts/require-secret.sh",
        "ops/ansible/fabric-health-deploy.yml",
        "ops/ansible/mesh-health-deploy.yml",
        "ops/ansible/presence-health-deploy.yml",
        "ops/ansible/time-health-deploy.yml",
        "ops/ansible/consistency-health-deploy.yml",
        "ops/ansible/fabric-crawler-deploy.yml",
        // The core type's site services (phase 13).
        "ops/ansible/decoy-deploy.yml",
        "ops/ansible/deadman-watch-deploy.yml",
        "ops/ansible/traefik-compression-metrics-deploy.yml",
        "ops/ansible/budget-notify-deploy.yml",
        "ops/ansible/names-collector-deploy.yml",
        "ops/ansible/oci-substrate-deploy.yml",
        "ops/ansible/host-net-publish-deploy.yml",
        // The real phone scripts and the class's declarations.
        "ops/scripts/phone-node-ssh.sh",
        // Phase 1: the phone answers over USB.
        "ops/scripts/join-node-reach.sh",
        "ops/scripts/phone-node-provision.sh",
        "ops/scripts/phone-node-guest-prepare.sh",
        "ops/scripts/phone-node-integrity.sh",
        "ops/ansible/group_vars/class_phone.yml",
    ] {
        std::fs::copy(r.join(s), checkout.join(s)).unwrap();
    }
    // The node's mesh address once enrolled: the next one after the founder's
    // in the fixture's own synthetic range.
    let alpha =
        std::fs::read_to_string(checkout.join("ops/ansible/group_vars/alpha_hosts/vars.yml"))
            .unwrap();
    let founder = alpha
        .lines()
        .find_map(|l| l.strip_prefix("mesh_ip_a: "))
        .unwrap()
        .trim();
    let mesh = format!("{}.4", founder.rsplit_once('.').unwrap().0);

    let log = root.join("argv.log");
    let state = root.join("state");
    let vm = root.join("vm");
    let vmbin = root.join("vmbin");
    std::fs::create_dir_all(&state).unwrap();
    let (l, st, v, vb) = (
        log.display(),
        state.display(),
        vm.display(),
        vmbin.display(),
    );

    // ---- the VM's filesystem, as the Terminal app's Debian image has it ----
    let boot = vm.join("mnt/internal/linux");
    write(&boot.join("vmlinuz"), "android 6.12.60-android16\n");
    write(&boot.join("initrd.img"), "android initrd\n");
    write(&boot.join("build_id"), "stand-in\n");
    write(
        &boot.join("vm_config.json"),
        "{\n    \"name\": \"debian\",\n    \"params\": \"root=/dev/vda1 console=hvc0\",\n    \"memory_mib\": 3923,\n    \"auto_memory_balloon\": true\n}\n",
    );
    write(&vm.join("proc/cmdline"), "root=/dev/vda1 console=hvc0\n");
    write(&vm.join("proc/sys/vm/drop_caches"), "");
    for ns in ["pid", "ipc", "user", "net", "mnt"] {
        write(&vm.join("proc/self/ns").join(ns), "");
    }
    write(&vm.join("usr/lib/aarch64-linux-gnu/libc.so.6"), "libc\n");
    write(
        &vm.join("usr/lib/aarch64-linux-gnu/libcrypto.so.3"),
        "libcrypto\n",
    );
    write(&vm.join("usr/bin/ssh"), "ssh\n");
    write(
        &vm.join("etc/fstab"),
        "/dev/vda1 / ext4 rw,discard,errors=remount-ro 0 1\n",
    );
    write(&vm.join("etc/os-release"), "VERSION_CODENAME=trixie\n");
    write(&vm.join("etc/hosts"), "127.0.0.1 localhost\n");
    for d in ["var/tmp", "var/lib/dpkg", "boot", "home/droid"] {
        std::fs::create_dir_all(vm.join(d)).unwrap();
    }
    std::fs::write(state.join("kernel"), "6.12.60-android16\n").unwrap();

    // ---- the VM's command path ----
    // What runs "in the VM" finds two directories and nothing of this
    // machine's own path: the stand-ins below, and `vmhost`, which names the
    // general tools the guest scripts use and passes each to this machine's.
    // A program this machine happens to have (dockerd, sudo, apt-get, dpkg)
    // is therefore never found in its place; a tool with no stand-in and no
    // entry here is "command not found", which fails the run.
    let vmhost = root.join("vmhost");
    let host_path = std::env::var("PATH").unwrap();
    for tool in [
        "sh", "bash", "env", "cat", "cp", "mkdir", "chmod", "rm", "tee", "grep", "sed", "awk",
        "find", "sort", "head", "tail", "seq", "sleep", "sync", "wc", "tr", "df", "cksum",
        "python3", "test", "true",
    ] {
        let real = host_tool(std::ffi::OsStr::new(&host_path), tool)
            .unwrap_or_else(|| panic!("this machine has no {tool}"));
        executable(
            &vmhost.join(tool),
            &format!("#!/bin/sh\nexec '{}' \"$@\"\n", real.display()),
        );
    }
    let vh = vmhost.display();

    // ---- the VM's own tools ----
    let vmtool = |name: &str, body: &str| {
        executable(
            &vmbin.join(name),
            &format!("#!/bin/sh\nprintf 'vm:{name} %s\\n' \"$*\" >> '{l}'\n{body}\n"),
        )
    };
    vmtool(
        "uname",
        &format!("if [ \"$1\" = -r ]; then cat '{st}/kernel'; else echo Linux; fi"),
    );
    // sudo runs the command as it is: the stand-in VM has one user.
    vmtool(
        "sudo",
        "while [ \"$1\" = -n ] || [ \"$1\" = -S ]; do shift; done\nexec env \"$@\"",
    );
    // A different answer on every read when reads are made to differ.
    vmtool(
        "sha256sum",
        &format!(
            "if [ -f '{st}/fail-reads' ]; then cat > /dev/null; echo \"$$ -\"; else cksum; fi"
        ),
    );
    vmtool(
        "dpkg",
        &format!("case \"$*\" in *--verify*) if [ -f '{st}/fail-dpkg' ]; then echo '??5?????? /usr/bin/thing'; fi ;; esac\nexit 0"),
    );
    vmtool(
        "dmesg",
        &format!("if [ -f '{st}/fail-dmesg' ]; then echo 'EXT4-fs error (device vda1): stand-in'; fi\nexit 0"),
    );
    vmtool("docker", "echo 'Hello from Docker!'");
    vmtool("nproc", "echo 8");
    vmtool("free", "echo 'Mem: 3923 1134 2789'");
    for quiet in [
        "modprobe",
        "systemctl",
        "hostnamectl",
        "chown",
        "curl",
        "useradd",
    ] {
        vmtool(quiet, "exit 0");
    }
    vmtool("fuser", "exit 1");
    vmtool(
        "gpg",
        "out=''; prev=''\nfor a; do [ \"$prev\" = -o ] && out=$a; prev=$a; done\ncat > /dev/null\n[ -n \"$out\" ] && : > \"$out\"\nexit 0",
    );
    vmtool("id", &format!("[ -d \"{v}/home/$1\" ]"));
    // apt-get installs Debian's kernel under /boot, and Docker Engine.
    vmtool(
        "apt-get",
        &format!(
            "case \"$*\" in\n\
             *linux-image-cloud-arm64*) echo 'cloud {DEBIAN_KERNEL}' > '{v}/boot/vmlinuz-{DEBIAN_KERNEL}'; echo 'cloud initrd' > '{v}/boot/initrd.img-{DEBIAN_KERNEL}' ;;\n\
             *docker-ce*) printf '#!/bin/sh\\nexit 0\\n' > '{vb}/dockerd'; chmod 755 '{vb}/dockerd' ;;\n\
             esac\nexit 0"
        ),
    );

    if prepared {
        std::fs::write(state.join("kernel"), format!("{DEBIAN_KERNEL}\n")).unwrap();
        write(&boot.join("vmlinuz"), &format!("cloud {DEBIAN_KERNEL}\n"));
        write(
            &vm.join("proc/cmdline"),
            "root=/dev/vda1 console=hvc0 arm64.nosve arm64.nosme modprobe.blacklist=virtio_balloon\n",
        );
        executable(&vmbin.join("dockerd"), "#!/bin/sh\nexit 0\n");
        write(
            &vm.join("home/operator/.ssh/authorized_keys"),
            &format!("{PUBKEY}\n"),
        );
    }

    // ---- this machine's own copies of the VM's tools ----
    // A machine that runs this test may have Docker, sudo, apt and dpkg of
    // its own. These stand for them, first on the path the run is given: one
    // that is ever started writes a `host:` line, and every run asserts none.
    for own in [
        "dockerd",
        "docker",
        "sudo",
        "apt-get",
        "dpkg",
        "systemctl",
        "useradd",
        "modprobe",
    ] {
        executable(
            &root.join("bin").join(own),
            &format!("#!/bin/sh\nprintf 'host:{own} %s\\n' \"$*\" >> '{l}'\nexit 0\n"),
        );
    }

    // ---- adb: the phone ----
    // `am start` starts the VM on what the Terminal app's boot folder holds:
    // Debian's kernel when the guest script staged it there, with the
    // parameters of vm_config.json on its command line.
    executable(
        &root.join("bin/adb"),
        &format!(
            "#!/bin/sh\nprintf 'adb %s\\n' \"$*\" >> '{l}'\n\
             if [ -f '{st}/no-phone' ]; then echo 'error: no devices/emulators found' >&2; exit 1; fi\n\
             case \"$*\" in\n\
             get-state) echo device ;;\n\
             'shell nc '*) [ ! -f '{st}/vm-off' ] || exit 1 ;;\n\
             *'ip neigh show dev avf_tap_fixed'*) [ -f '{st}/vm-off' ] || echo '10.0.2.15 lladdr stand-in REACHABLE' ;;\n\
             *'getprop ro.product.model'*) echo 'Stand-in Phone' ;;\n\
             *'getprop ro.build.version.release'*) echo 16 ;;\n\
             *'getprop ro.boot.hypervisor.vm.supported'*) echo 1 ;;\n\
             *'pm list packages'*) echo 'package:com.android.virtualization.terminal' ;;\n\
             *'dumpsys window'*) echo 'isKeyguardShowing=false'; echo 'mCurrentFocus=Window{{1 u0 com.android.virtualization.terminal/.MainActivity}}' ;;\n\
             *'vm list'*) [ -f '{st}/vm-off' ] || echo 'name: debian' ;;\n\
             *'am start'*)\n\
               rm -f '{st}/vm-off'\n\
               k=$(sed -n 's/^cloud //p' '{v}/mnt/internal/linux/vmlinuz')\n\
               if [ -n \"$k\" ] && [ ! -f '{st}/new-kernel-does-not-boot' ]; then\n\
                 echo \"$k\" > '{st}/kernel'\n\
                 python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[\"params\"])' '{v}/mnt/internal/linux/vm_config.json' > '{v}/proc/cmdline'\n\
               fi ;;\n\
             esac\nexit 0\n"
        ),
    );

    // ---- ssh ----
    // With -F it carries the connection as ssh does: through the file's proxy
    // command. The deploy user answers only once the guest script made it.
    // What is piped to `bash -s` runs against the VM's filesystem.
    executable(
        &root.join("bin/ssh"),
        &format!(
            "#!/bin/sh\nprintf 'ssh %s\\n' \"$*\" >> '{l}'\n\
             cfg=''; prev=''; target=''; cmd=''\n\
             for a; do\n\
               [ \"$prev\" = -F ] && cfg=$a\n\
               case \"$a\" in *@*) [ -n \"$target\" ] || target=$a ;; esac\n\
               prev=$a; cmd=$a\n\
             done\n\
             if [ -n \"$cfg\" ]; then\n\
               proxy=$(sed -n 's/^ *ProxyCommand //p' \"$cfg\")\n\
               [ -n \"$proxy\" ] || {{ echo \"ssh stand-in: $cfg names no ProxyCommand\" >&2; exit 255; }}\n\
               sh -c \"$proxy\" < /dev/null || exit 255\n\
               user=${{target%@*}}\n\
               [ \"$user\" = droid ] || [ -f \"{v}/home/$user/.ssh/authorized_keys\" ] || exit 255\n\
             fi\n\
             case \"$cmd\" in\n\
             *'bash -s'*) PATH='{vb}:{vh}' HOME='{v}/home/droid' PHONE_NODE_GUEST_ROOT='{v}' PHONE_NODE_GATE_MIB=1 /bin/sh -c \"$cmd\" ;;\n\
             *poweroff*) touch '{st}/vm-off' ;;\n\
             esac\n"
        ),
    );

    // ---- the programs that reach the goatnet's other hosts ----
    // with-aws starts every play. The enrolment play stands in for the mint
    // and writes the node's mesh address to the file the run names.
    executable(
        &checkout.join("ops/scripts/with-aws"),
        &format!(
            "#!/bin/sh\nprintf 'play %s\\n' \"$*\" >> '{l}'\n\
             case \"$*\" in\n\
             *ory-deploy.yml*) touch '{st}/ory-up' ;;\n\
             *mesh-enroll-host.yml*)\n\
               out=''; for a; do case \"$a\" in mesh_ips_out=*) out=\"${{a#mesh_ips_out=}}\" ;; esac; done\n\
               [ -n \"$out\" ] || {{ echo 'mesh-enroll-host.yml: no mesh_ips_out'; exit 2; }}\n\
               printf '{{\"{NODE}\": \"{mesh}\"}}\\n' > \"$out\" ;;\n\
             *bootstrap*) case \"$*\" in *'--skip-tags after-ory'*) ;; *)\n\
               [ -f '{st}/ory-up' ] || {{ echo 'TASK [check the management service is up] fatal: netbird-mgmt is not up'; exit 2; }} ;; esac ;;\n\
             esac\nexit 0\n"
        ),
    );
    executable(
        &checkout.join("ops/scripts/mint-site-certs.sh"),
        &format!("#!/bin/sh\nprintf 'mint %s\\n' \"$*\" >> '{l}'\n"),
    );
    // join-site-secrets.py takes the joiner's shared secrets from its
    // founder in phase 3; the stand-in records it.
    executable(
        &checkout.join("ops/scripts/join-site-secrets.py"),
        &format!("import sys\nopen('{l}', 'a').write('secrets ' + ' '.join(sys.argv[1:]) + '\\n')\n"),
    );
    executable(
        &checkout.join("ops/scripts/ansible-preflight.sh"),
        "#!/bin/sh\nexit 0\n",
    );
    executable(
        &checkout.join("ops/validation/generate-values.py"),
        &format!(
            "import sys\nopen('{l}', 'a').write('values ' + ' '.join(sys.argv[1:]) + '\\n')\n"
        ),
    );
    executable(
        &checkout.join("ops/validation/run-acceptance.py"),
        &format!("import sys\nopen('{l}', 'a').write('acceptance ' + ' '.join(sys.argv[1:]) + '\\n')\nprint('every applicable row PASS')\n"),
    );
    Site {
        _tmp: tmp,
        root,
        checkout,
        log,
        state,
        vm,
        mesh,
    }
}

fn goatbuilder(s: &Site, args: &[&str]) -> (Output, serde_json::Value) {
    let mut all = vec![
        "-C".to_string(),
        s.checkout.display().to_string(),
        "--yes".into(),
        "-o".into(),
        "json".into(),
        "join-node".into(),
        "zagros".into(),
        "--site".into(),
        "delta".into(),
        "--node".into(),
        NODE.into(),
    ];
    all.extend(args.iter().map(|a| a.to_string()));
    let path = format!(
        "{}:{}",
        s.root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let out = Command::new(env!("CARGO_BIN_EXE_goatbuilder"))
        .args(&all)
        .env_clear()
        .env("PATH", path)
        .env("HOME", &s.root)
        .env("GOAT_SECRET_STORE", "env")
        .env("DELTA_PG_PASS", "pg-stand-in")
        // The founder's management API token, which the mesh DNS step reads.
        .env("ALPHA_NB_API_TOKEN", "api-token-stand-in")
        .env("GOAT_NO_FUN", "1")
        // The phone script's waits, which are for a real phone.
        .env("PHONE_NODE_NAP", "0")
        .output()
        .unwrap();
    let v = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    (out, v)
}

/// `join-node ... --class phone --via usb`, from a phase.
fn join_phone(s: &Site, extra: &[&str]) -> (Output, serde_json::Value) {
    let mut args = vec!["--class", "phone", "--via", "usb"];
    args.extend_from_slice(extra);
    goatbuilder(s, &args)
}

fn says(o: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn phases(v: &serde_json::Value) -> Vec<u64> {
    let mut seen: Vec<u64> = v["checks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|c| c["name"] == "phase")
                .filter_map(|c| {
                    c["detail"]
                        .as_str()?
                        .strip_prefix("=== Phase ")?
                        .split_whitespace()
                        .next()?
                        .parse()
                        .ok()
                })
                .collect()
        })
        .unwrap_or_default();
    seen.dedup();
    seen
}

/// The operator's public key, which the deploy user of a VM not yet prepared
/// is given. Every class reads the operator's key pair; a prepared VM needs
/// none of it from this script.
fn operator_key(s: &Site) {
    write(
        &s.root.join(".ssh/id_ed25519.pub"),
        &format!("{PUBKEY} operator\n"),
    );
}

/// The real config preflight, on the checkout, for the phone site.
fn preflight(s: &Site, args: &[&str]) -> Output {
    Command::new("python3")
        .arg(s.checkout.join("ops/scripts/standup-config-preflight.py"))
        .arg("delta_hosts")
        .args(args)
        .output()
        .unwrap()
}

/// Gate row H5. The class file declares the five differences; the site's own
/// group_vars and inventory line state none of them; and the config preflight
/// passes for the site with `--class phone`.
#[test]
fn a_phone_site_passes_the_config_preflight_with_no_class_value_written_by_hand() {
    let s = site(false);
    let class: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(s.checkout.join("ops/ansible/group_vars/class_phone.yml"))
            .unwrap(),
    )
    .unwrap();
    // The five declared differences, each as the class file carries it.
    let meta = &class["machine_class"];
    assert_eq!(meta["lan_address"], "none", "no LAN address");
    assert_eq!(meta["public_ingress"], "none", "no public ingress");
    assert_eq!(class["signal_federation_ingress_enabled"], false);
    assert_eq!(
        class["luks_volumes_mgmt_host"],
        serde_yaml::Value::Sequence(vec![]),
        "no disk encryption inside the VM"
    );
    for limit in [
        "mgmt_traefik_memory",
        "mgmt_management_memory",
        "mgmt_signal_memory",
        "goat_zenoh_memory",
        "mgmt_postgres_memory",
        "fabric_crawler_memory_limit",
    ] {
        assert!(class[limit].is_string(), "a memory limit for 4 GB: {limit}");
    }
    assert_eq!(class["has_obs_host"], false, "no observability host");

    // The site states none of them.
    let vars_path = s
        .checkout
        .join("ops/ansible/group_vars/delta_hosts/vars.yml");
    let vars_text = std::fs::read_to_string(&vars_path).unwrap();
    let vars: serde_yaml::Value = serde_yaml::from_str(&vars_text).unwrap();
    for key in class.as_mapping().unwrap().keys() {
        assert!(
            vars.get(key).is_none(),
            "delta's group_vars repeat {key:?}, which the class declares"
        );
    }
    let inv_path = s.checkout.join("ops/ansible/inventory/mgmt-hosts.ini");
    let inv = std::fs::read_to_string(&inv_path).unwrap();
    assert!(
        inv.lines().any(|l| l == NODE) && !inv.contains(&format!("{NODE} ansible_host")),
        "the node's line carries no LAN address:\n{inv}"
    );

    let out = preflight(&s, &["--class", "phone", "--before-enrolment"]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("OK: all standup-config checks pass"),
        "{}",
        says(&out)
    );

    // Each declared difference, contradicted in the site, is refused (GB-301).
    let refused = |what: &str, needle: &str| {
        let out = preflight(&s, &["--class", "phone", "--before-enrolment"]);
        let all = says(&out);
        assert!(!out.status.success(), "{what} passed:\n{all}");
        assert!(
            all.contains("GB-301: delta_hosts does not hold what the machine class phone declares")
                && all.contains(needle),
            "{what}: {all}"
        );
    };
    for (line, needle) in [
        ("has_obs_host: true\n", "sets has_obs_host: True"),
        (
            "mgmt_postgres_memory: 2G\n",
            "sets mgmt_postgres_memory: '2G'",
        ),
        (
            "luks_volumes_mgmt_host: [{name: data}]\n",
            "sets luks_volumes_mgmt_host",
        ),
        (
            "rathole_client_service_name: delta\n",
            "takes none of rathole_client_*",
        ),
    ] {
        std::fs::write(&vars_path, format!("{vars_text}{line}")).unwrap();
        refused(line.trim(), needle);
    }
    std::fs::write(&vars_path, &vars_text).unwrap();
    std::fs::write(
        &inv_path,
        inv.replace(
            &format!("[delta_hosts]\n{NODE}\n"),
            &format!("[delta_hosts]\n{NODE} ansible_host=192.0.2.40\n"),
        ),
    )
    .unwrap();
    refused("a LAN address", "has no LAN address");
    std::fs::write(
        &inv_path,
        inv.replace(&format!("[class_phone]\n{NODE}\n"), "[class_phone]\n"),
    )
    .unwrap();
    refused(
        "a node outside the class's group",
        "is not under [class_phone]",
    );
    std::fs::write(&inv_path, &inv).unwrap();

    // A value the class declares, repeated as it is, is a warning.
    std::fs::write(&vars_path, format!("{vars_text}has_obs_host: false\n")).unwrap();
    let out = preflight(&s, &["--class", "phone", "--before-enrolment"]);
    assert!(out.status.success(), "{}", says(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("repeats has_obs_host"),
        "{}",
        says(&out)
    );
}

/// Gate rows H3 and H4, the whole run. A phone whose VM is as the Terminal
/// app installs it joins from phase 1 to 15 in one run with no operator step:
/// phase 1 checks the phone answers over USB, phase 2
/// prepares the VM and passes the integrity checks, the node is reached over
/// the Android debug bridge until phase 12 and at its mesh address after it,
/// and the plays reach the goatnet's other hosts over the mesh.
#[test]
fn a_phone_joins_over_usb_from_phase_1_to_15() {
    let s = site(false);
    operator_key(&s);
    let (out, v) = join_phone(&s, &[]);
    let log = s.log();
    assert!(out.status.success(), "{}\nargv:\n{log}", says(&out));
    assert_eq!(phases(&v), (1..=15).collect::<Vec<u64>>(), "{}", says(&out));
    assert!(
        log.lines().any(|l| l == "adb get-state"),
        "phase 1 asked adb:\n{log}"
    );
    assert!(
        !says(&out).contains("OPERATOR STEP"),
        "no operator step after phase 1: {}",
        says(&out)
    );
    the_joiners_gaps_are_the_admin_surfaces("delta", &log);

    // H4: the VM was prepared by the real guest script.
    let config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(s.vm.join("mnt/internal/linux/vm_config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        config["params"],
        "root=/dev/vda1 console=hvc0 arm64.nosve arm64.nosme modprobe.blacklist=virtio_balloon"
    );
    assert_eq!(config["auto_memory_balloon"], false);
    assert_eq!(config["memory_mib"], 3923, "memory is left as it was");
    assert_eq!(
        std::fs::read_to_string(s.vm.join("mnt/internal/linux/vmlinuz")).unwrap(),
        format!("cloud {DEBIAN_KERNEL}\n"),
        "Debian's kernel is what the Terminal app boots"
    );
    assert_eq!(
        std::fs::read_to_string(s.vm.join("home/droid/orig-boot/vmlinuz")).unwrap(),
        "android 6.12.60-android16\n",
        "the kernel Android shipped is kept"
    );
    assert!(
        !std::fs::read_to_string(s.vm.join("etc/fstab"))
            .unwrap()
            .contains("discard"),
        "no disk discard"
    );
    assert!(s.vm.join("etc/kernel/postinst.d/zz-phone-node").is_file());
    assert_eq!(
        std::fs::read_to_string(s.vm.join("home/operator/.ssh/authorized_keys")).unwrap(),
        format!("{PUBKEY}\n"),
        "the deploy user accepts the operator's key"
    );
    assert_eq!(
        std::fs::read_to_string(s.vm.join("etc/sudoers.d/90-operator")).unwrap(),
        "operator ALL=(ALL) NOPASSWD:ALL\n"
    );
    assert!(
        log.contains("vm:hostnamectl set-hostname delta-phone-1")
            && log.contains("vm:docker run --rm hello-world"),
        "{log}"
    );

    // H3: every connection to the node before phase 12 names the SSH
    // configuration the run wrote, and adb carried it.
    let cfg = format!("{}.ssh", s.usb_base());
    let ini = format!("{}.ini", s.usb_base());
    let cfg_text = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        cfg_text.contains(&format!(
            "ProxyCommand \"{}/ops/scripts/phone-node-ssh.sh\" --proxy",
            s.checkout.display()
        )),
        "{cfg_text}"
    );
    let lines: Vec<&str> = log.lines().collect();
    let to_node: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("ssh ") && l.contains(&format!("@{NODE}")))
        .collect();
    assert!(to_node.len() >= 6, "{to_node:#?}");
    for l in &to_node {
        assert!(l.starts_with(&format!("ssh -F {cfg} ")), "{l}");
    }
    assert!(
        to_node.iter().any(|l| l.contains("chronyc -c tracking")),
        "phase 3 reads the phone's clock through the cable: {to_node:#?}"
    );
    assert!(
        lines
            .iter()
            .filter(|l| **l == "adb shell nc 10.0.2.15 22")
            .count()
            >= to_node.len(),
        "adb carries every connection to the node:\n{log}"
    );
    let plays: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| l.starts_with("play "))
        .collect();
    let mesh_inventories = "-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini";
    for play in [
        "mgmt-host-baseline.yml",
        "wireguard-kmod.yml",
        "mesh-enroll-host.yml",
    ] {
        let p = plays.iter().find(|l| l.contains(play)).unwrap();
        assert!(
            p.contains(&format!("{mesh_inventories} -i {ini} {play}")),
            "{play} reaches the node over USB and the founder over the mesh: {p}"
        );
    }
    let ini_text = std::fs::read_to_string(&ini).unwrap();
    assert!(
        ini_text.contains(&format!(
            "{NODE} ansible_host={NODE} ansible_ssh_common_args='-F \"{cfg}\"'"
        )),
        "{ini_text}"
    );
    // From phase 12 the node is reached at its mesh address, as every node.
    let bootstrap = plays
        .iter()
        .position(|l| l.contains("participates_in_spock=false"))
        .expect("pass 1 ran");
    for p in &plays[bootstrap..] {
        assert!(!p.contains("-usb.ini"), "after enrolment, the mesh: {p}");
    }
    assert!(
        lines.contains(
            &format!(
                "ssh operator@{} sudo systemctl unmask apt-daily.timer apt-daily-upgrade.timer && sudo systemctl enable --now apt-daily.timer apt-daily-upgrade.timer",
                s.mesh
            )
            .as_str()
        ),
        "phase 14 reaches the node at its mesh address:\n{log}"
    );
    let gv = std::fs::read_to_string(
        s.checkout
            .join("ops/ansible/group_vars/delta_hosts/vars.yml"),
    )
    .unwrap();
    assert!(
        gv.contains(&s.mesh) && !gv.contains("TBD_DELTA_MESH_IP_A"),
        "{gv}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("acceptance ")),
        "phase 15 ran acceptance:\n{log}"
    );
    // No first-login password is read, and no LAN address is used.
    assert!(
        !log.contains("sshpass") && !log.contains("seed-ssh-key"),
        "{log}"
    );

    // Idempotent: phase 2 again on the prepared VM goes straight to the checks.
    let installs = |log: &str| log.lines().filter(|l| l.starts_with("vm:apt-get ")).count();
    let before = installs(&log);
    assert!(
        before >= 2,
        "the first run installed the kernel and Docker: {log}"
    );
    let (out, _) = join_phone(&s, &["--from", "2", "--to", "2"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(
        installs(&s.log()),
        before,
        "a prepared VM is not prepared again"
    );
    assert!(
        s.log()
            .lines()
            .filter(|l| l.contains("vm:docker run"))
            .count()
            == 2
    );
}

/// Gate row H3, with nothing of the operator's: a prepared phone joins from
/// phase 2 to 15 on a machine whose home directory has no `.ssh` at all. No
/// SSH configuration, alias or known-hosts file was written by hand, and the
/// run leaves none there: what it wrote is in the project's state directory.
#[test]
fn a_prepared_phone_is_reached_with_no_ssh_configuration_written_by_hand() {
    let s = site(true);
    assert!(!s.root.join(".ssh").exists());
    let (out, v) = join_phone(&s, &["--from", "2"]);
    assert!(out.status.success(), "{}\nargv:\n{}", says(&out), s.log());
    assert_eq!(phases(&v), (2..=15).collect::<Vec<u64>>());
    assert!(
        !s.root.join(".ssh").exists(),
        "the run wrote nothing under ~/.ssh"
    );
    let log = s.log();
    assert!(
        !log.lines().any(|l| l.starts_with("vm:apt-get ")),
        "a prepared VM is not prepared again:\n{log}"
    );
    for f in ["ssh", "ini"] {
        assert!(Path::new(&format!("{}.{f}", s.usb_base())).is_file());
    }
    // A dry run lists where each of them comes from.
    let (out, _) = join_phone(&s, &["--dry-run"]);
    let all = says(&out);
    assert!(out.status.success(), "{all}");
    assert!(
        all.contains("phone-node-ssh.sh' --write") && all.contains("usb_ssh_config"),
        "{all}"
    );
}

/// Gate row H4, the refusal. A check made to fail stops the run at phase 2
/// with GB-300, naming the check, before any play runs; once the check
/// passes, the same command goes on.
#[test]
fn phase_2_refuses_when_an_integrity_check_fails() {
    let s = site(true);
    for (flag, names) in [
        ("fail-dpkg", "1 installed files fail dpkg --verify"),
        (
            "fail-reads",
            "file reads differ between cache drops (6 distinct hashes)",
        ),
        (
            "fail-dmesg",
            "the kernel log reports filesystem or I/O errors",
        ),
    ] {
        s.flag(flag);
        let (out, v) = join_phone(&s, &["--from", "2"]);
        let all = says(&out);
        assert!(!out.status.success(), "{flag} passed: {all}");
        assert_eq!(phases(&v), vec![2], "{flag}: the run stops at phase 2");
        assert!(
            all.contains("GB-300") && all.contains(names),
            "{flag}: {all}"
        );
        s.unflag(flag);
    }
    assert!(
        !s.log().lines().any(|l| l.starts_with("play ")),
        "no play ran on a VM that failed a check:\n{}",
        s.log()
    );
    let (out, v) = join_phone(&s, &["--from", "2", "--to", "3"]);
    assert!(out.status.success(), "{}", says(&out));
    assert_eq!(phases(&v), vec![2, 3]);
}

/// Gate row H4: a VM that does not come back on Debian's kernel after the
/// restart is refused with GB-300, and nothing is built on it.
#[test]
fn a_vm_that_does_not_come_back_on_the_new_kernel_is_refused() {
    let s = site(false);
    operator_key(&s);
    s.flag("new-kernel-does-not-boot");
    let (out, v) = join_phone(&s, &["--from", "2"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert_eq!(phases(&v), vec![2]);
    assert!(
        all.contains("GB-300")
            && all.contains("still on the kernel Android supplies: 6.12.60-android16"),
        "{all}"
    );
    assert!(!s.log().lines().any(|l| l.starts_with("play ")));
}

/// Gate row H3: no phone on the cable stops the run at phase 1 (GB-314), with
/// the instruction for a person; from phase 2 it is refused with GB-299.
#[test]
fn no_phone_on_the_cable_is_refused() {
    let s = site(true);
    s.flag("no-phone");
    let (out, v) = join_phone(&s, &[]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert_eq!(phases(&v), vec![1], "{all}");
    assert!(
        all.contains("GB-314") && all.contains("allow USB debugging") && all.contains("--from 1"),
        "{all}"
    );
    let (out, v) = join_phone(&s, &["--from", "2"]);
    let all = says(&out);
    assert!(!out.status.success(), "{all}");
    assert_eq!(phases(&v), vec![2]);
    assert!(
        all.contains("GB-299") && all.contains("no phone on adb"),
        "{all}"
    );
}

/// A phone node and the USB transport go together, and a node on a USB cable
/// has no LAN address: each other combination is refused before any phase.
#[test]
fn the_phone_class_and_the_usb_transport_go_together() {
    let s = site(true);
    for (args, names) in [
        (vec!["--class", "phone"], "--class phone with --via mesh"),
        (
            vec!["--class", "phone", "--via", "door"],
            "--class phone with --via door",
        ),
        (
            vec![
                "--class",
                "phone",
                "--via",
                "usb",
                "--address",
                "192.0.2.40",
            ],
            "a phone node has no LAN address",
        ),
    ] {
        let (out, _) = goatbuilder(&s, &args);
        let all = says(&out);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {all}");
        assert!(all.contains(names), "{args:?}: {all}");
        assert!(s.log().is_empty(), "{args:?} ran something:\n{}", s.log());
    }
}

/// A shim passes each general tool to the file a shell would run, so a
/// non-executable file of the same name earlier on PATH (a 0644 `env` that an
/// installer left in a home directory) is passed over, as the shell does, and
/// the executable one after it is taken.
#[test]
fn a_shim_takes_the_executable_tool_and_not_an_earlier_file_of_its_name() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let (plain, real) = (t.path().join("first"), t.path().join("second"));
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(plain.join("env"), "# a shell snippet, not a program\n").unwrap();
    std::fs::set_permissions(plain.join("env"), std::fs::Permissions::from_mode(0o644)).unwrap();
    executable(&real.join("env"), "#!/bin/sh\nexit 0\n");
    let path = std::env::join_paths([&plain, &real]).unwrap();
    assert_eq!(host_tool(&path, "env"), Some(real.join("env")));
    assert_eq!(host_tool(&path, "no-such-tool"), None);
}

/// The joiner's end state, as phase 15 hands it to the rows: the four admin
/// surfaces are declared and not yet built at a second site (gate rows I1,
/// I4), nor is the site's own SPIRE server and agent (ADR 1164), and the
/// real runner's plan against that model reads their rows as gaps (SKIPPED
/// in a run, with the gap's words) and exits 0.
fn the_joiners_gaps_are_the_admin_surfaces(env: &str, log: &str) {
    let run = log
        .lines()
        .find(|l| l.starts_with("acceptance "))
        .unwrap_or_else(|| panic!("phase 15 ran no acceptance:\n{log}"));
    let words: Vec<&str> = run.split(' ').collect();
    let at = words.iter().position(|w| *w == "--endstate").unwrap_or_else(|| panic!("{run}"));
    let model = words[at + 1];
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(model).unwrap()).unwrap();
    let gaps: std::collections::BTreeSet<&str> = doc["services"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, s)| !s["gap"].is_null())
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(
        gaps,
        ["id-admin-console", "netbird-dashboard", "onboard-runner", "portal", "spire", "spire-agent"]
            .into_iter()
            .collect(),
        "{doc}"
    );
    let yaml = std::process::Command::new("python3").args(["-c", "import yaml"]).status().map(|s| s.success()).unwrap_or(false);
    if !yaml {
        eprintln!("skipped: the acceptance runner needs python3 with PyYAML");
        return;
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let values = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(values.path(), "{}\n").unwrap();
    let out = std::process::Command::new("python3")
        .arg(repo.join("ops/validation/run-acceptance.py"))
        .args(["--env", env, "--values"])
        .arg(values.path())
        .args(["--endstate", model, "--plan"])
        .output()
        .unwrap();
    let plan = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{plan}{}", String::from_utf8_lossy(&out.stderr));
    let line = plan
        .lines()
        .find(|l| l.contains("gaps here, declared and not yet built at this site:"))
        .unwrap_or_else(|| panic!("no gaps in the plan:\n{plan}"));
    for row in ["portal-", "onboard-runner-", "console-audit-path-is-a-file"] {
        assert!(line.contains(row), "{row}: {line}");
    }
}
