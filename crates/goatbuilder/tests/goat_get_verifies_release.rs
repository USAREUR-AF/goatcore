// Gate row E4 (docs/design/goatbuilder-v1-gate.md, "Upgrading is tested"): a
// surface that serves goatbuilder releases checks each release's signature
// before it serves it, with `goatbuilder update`'s own verifier.
//
// The surface is the goat-get deploy's goatbuilder family
// (ops/ansible/roles/goat-get/tasks/goatbuilder.yml), run by ansible-playbook
// against this machine, with no privilege, into a temporary directory. The
// release comes from a stand-in GitHub API served by this process on
// 127.0.0.1, and a stand-in Sigstore signs it (tests/support/sigstore.rs).
// The verifier the role runs is the build under test, with the stand-in's
// trusted root.
//
// A release whose bundle verifies is served. Each of these is refused, and
// the served directory is byte for byte as it was before: SHA256SUMS changed
// after signing, a signature by another workflow identity, a
// transparency-log entry naming another sha256, no bundle, a binary
// SHA256SUMS does not list at its digest, and a .sha256 that disagrees with
// SHA256SUMS.
//
// Without ansible-playbook on PATH the test says it did not run and passes,
// unless GOAT_GET_RELEASE_TEST_REQUIRED=1.

#[path = "support/sigstore.rs"]
mod sigstore;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

const REPO: &str = "example-org/goatcore";
const VERSION: &str = "9.9.9";
const BINS: [&str; 2] = ["goatbuilder-linux-amd64", "goatbuilder-darwin-arm64"];

fn sha256(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// A release as the release workflow publishes it: the binaries, each with
/// its `.sha256`, SHA256SUMS over the binaries, and its bundle. `edit` changes
/// it before it is served.
type Release = BTreeMap<String, Vec<u8>>;

fn release(build: &str, bundle: impl FnOnce(&[u8]) -> Option<String>) -> Release {
    let mut r = Release::new();
    let mut sums = String::new();
    for name in BINS {
        let bin = format!("the {name} build {build}").into_bytes();
        let d = sha256(&bin);
        r.insert(
            format!("{name}.sha256"),
            format!("{d}  {name}\n").into_bytes(),
        );
        r.insert(name.to_string(), bin);
        sums.push_str(&format!("{d}  {name}\n"));
    }
    if let Some(b) = bundle(sums.as_bytes()) {
        r.insert("SHA256SUMS.cosign-bundle".into(), b.into_bytes());
    }
    r.insert("SHA256SUMS".into(), sums.into_bytes());
    r
}

/// The asset names the role is asked to serve: every file of a whole release.
fn assets() -> Vec<String> {
    let mut v: Vec<String> = BINS
        .iter()
        .flat_map(|n| [n.to_string(), format!("{n}.sha256")])
        .collect();
    v.push("SHA256SUMS".into());
    v.push("SHA256SUMS.cosign-bundle".into());
    v
}

/// The stand-in GitHub: the release by tag, as the API answers it, and each
/// asset's bytes. The release it serves can be swapped between runs.
fn github(served: Arc<Mutex<Release>>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let api = base.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let served = served.clone();
            let base = api.clone();
            std::thread::spawn(move || {
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let req = String::from_utf8_lossy(&req);
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let release = served.lock().unwrap().clone();
                let is_api = path == format!("/repos/{REPO}/releases/tags/goatbuilder/v{VERSION}");
                let body = if is_api {
                    let list: Vec<serde_json::Value> = release
                        .keys()
                        .map(
                            |n| serde_json::json!({"name": n, "url": format!("{base}/assets/{n}")}),
                        )
                        .collect();
                    Some(serde_json::to_vec(&serde_json::json!({ "assets": list })).unwrap())
                } else {
                    path.strip_prefix("/assets/")
                        .and_then(|n| release.get(n).cloned())
                };
                match body {
                    Some(b) => {
                        let _ = write!(
                            s,
                            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            if is_api {
                                "application/json"
                            } else {
                                "application/octet-stream"
                            },
                            b.len()
                        );
                        let _ = s.write_all(&b);
                    }
                    None => {
                        let _ = write!(
                            s,
                            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                    }
                }
            });
        }
    });
    base
}

/// The goat-get role, run without its handlers: the goatbuilder family
/// notifies none, and they need a collection this test does not install.
fn role() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/ansible/roles/goat-get")
        .canonicalize()
        .expect("the goat-get role")
}

fn ansible_playbook() -> Option<PathBuf> {
    let required = std::env::var("GOAT_GET_RELEASE_TEST_REQUIRED").as_deref() == Ok("1");
    let found = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("ansible-playbook"))
            .find(|f| f.is_file())
    });
    if found.is_none() {
        assert!(
            !required,
            "GOAT_GET_RELEASE_TEST_REQUIRED=1 and ansible-playbook is not on PATH"
        );
        eprintln!("goat-get release test NOT RUN: ansible-playbook is not on PATH");
    }
    found
}

struct Surface {
    tmp: tempfile::TempDir,
    api: String,
    served: Arc<Mutex<Release>>,
    root: PathBuf,
    playbook: PathBuf,
}

impl Surface {
    fn new(s: &sigstore::Sigstore) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let served = Arc::new(Mutex::new(Release::new()));
        let api = github(served.clone());
        let root = tmp.path().join("trusted_root.json");
        std::fs::write(&root, s.trusted_root()).unwrap();
        let playbook = tmp.path().join("serve.yml");
        std::fs::write(
            &playbook,
            format!(
                "- name: serve the pinned goatbuilder release\n  hosts: localhost\n  connection: local\n  gather_facts: false\n  vars_files:\n    - {role}/defaults/main.yml\n  vars:\n    role_path: {role}\n  tasks:\n    - name: the goat-get goatbuilder family\n      ansible.builtin.include_tasks: {role}/tasks/goatbuilder.yml\n",
                role = role().display()
            ),
        )
        .unwrap();
        // The role's templates, where a play outside the role looks for them.
        std::os::unix::fs::symlink(role().join("templates"), tmp.path().join("templates")).unwrap();
        for d in ["www", "home", "ansible-tmp"] {
            std::fs::create_dir_all(tmp.path().join(d)).unwrap();
        }
        Surface {
            tmp,
            api,
            served,
            root,
            playbook,
        }
    }

    fn www(&self) -> PathBuf {
        self.tmp.path().join("www")
    }

    fn served_dir(&self) -> PathBuf {
        self.www().join("goatbuilder")
    }

    /// Run the role with `release` on the stand-in GitHub.
    fn deploy(&self, bin: &Path, release: Release) -> Output {
        *self.served.lock().unwrap() = release;
        let vars = serde_json::json!({
            "goat_get_www_dir": self.www(),
            "goat_get_goatbuilder_enabled": true,
            "goat_get_goatbuilder_release_repo": REPO,
            "goat_get_goatbuilder_release_private": false,
            "goat_get_goatbuilder_release_tag": format!("goatbuilder/v{VERSION}"),
            "goat_get_goatbuilder_assets": assets(),
            "goat_get_github_api": self.api,
            "goat_get_github_token": "",
            "goat_get_goatbuilder_verifier": env!("CARGO_BIN_EXE_goatbuilder"),
            "goat_get_goatbuilder_trusted_root": self.root,
            "goat_get_hostname": "get.zagros.example.org",
            "site_id": "zagros",
            "site_dns_zone": "zagros.example.org",
            "ansible_become": false,
            "ansible_python_interpreter": "auto_silent",
        });
        let vars_file = self.tmp.path().join("vars.json");
        std::fs::write(&vars_file, serde_json::to_vec(&vars).unwrap()).unwrap();
        Command::new(bin)
            .arg("-i")
            .arg("localhost,")
            .arg(&self.playbook)
            .arg("-e")
            .arg(format!("@{}", vars_file.display()))
            .env("ANSIBLE_LOCAL_TEMP", self.tmp.path().join("ansible-tmp"))
            .env("ANSIBLE_REMOTE_TEMP", self.tmp.path().join("ansible-tmp"))
            .env("ANSIBLE_NOCOLOR", "1")
            .env("ANSIBLE_RETRY_FILES_ENABLED", "0")
            .env("HOME", self.tmp.path().join("home"))
            .env("GOATBUILDER_NO_VERSION_CHECK", "1")
            .env_remove("ANSIBLE_CONFIG")
            .env_remove("GOAT_CLI_RELEASE_TOKEN")
            .env_remove("GH_TOKEN")
            .env_remove("GHCR_READ_TOKEN")
            .current_dir(self.tmp.path())
            .output()
            .unwrap()
    }

    /// Every file under the served directory, with its bytes.
    fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(d: &Path, base: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else {
                    out.insert(
                        p.strip_prefix(base).unwrap().to_path_buf(),
                        std::fs::read(&p).unwrap(),
                    );
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.served_dir(), &self.served_dir(), &mut out);
        out
    }

    /// Nothing but the served directory is left in the web root.
    fn leftovers(&self) -> Vec<String> {
        std::fs::read_dir(self.www())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "goatbuilder")
            .collect()
    }
}

fn text(o: &Output) -> String {
    format!(
        "exit {:?}\n{}{}",
        o.status.code(),
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn e4_goat_get_serves_a_release_only_once_its_signature_verifies() {
    let Some(bin) = ansible_playbook() else {
        return;
    };
    let s = sigstore::Sigstore::new();
    let surface = Surface::new(&s);
    let good = |build: &str| {
        release(build, |sums| {
            Some(s.bundle(sums, &sigstore::Workflow::release(REPO, VERSION)))
        })
    };

    // A release whose bundle verifies is served, every asset byte for byte.
    let first = good("one");
    let o = surface.deploy(&bin, first.clone());
    assert_eq!(o.status.code(), Some(0), "the good release: {}", text(&o));
    for (name, bytes) in &first {
        assert_eq!(
            std::fs::read(surface.served_dir().join(name)).ok().as_ref(),
            Some(bytes),
            "{name} is not served as released"
        );
    }
    assert!(surface.leftovers().is_empty(), "{:?}", surface.leftovers());

    let other_workflow = sigstore::Workflow {
        repo: REPO.into(),
        workflow: "goat-release.yml".into(),
        tag: format!("goatbuilder/v{VERSION}"),
    };
    let cases: Vec<(&str, Release, &str)> = vec![
        (
            "SHA256SUMS changed after signing",
            {
                let mut r = good("two");
                r.insert(
                    "SHA256SUMS".into(),
                    b"0000  goatbuilder-linux-amd64\n".to_vec(),
                );
                r
            },
            "GB-261",
        ),
        (
            "a signature by another workflow identity",
            release("two", |sums| Some(s.bundle(sums, &other_workflow))),
            "GB-261",
        ),
        (
            "a transparency-log entry naming another sha256",
            release("two", |_| {
                Some(s.bundle(
                    b"another SHA256SUMS",
                    &sigstore::Workflow::release(REPO, VERSION),
                ))
            }),
            "GB-261",
        ),
        (
            "no bundle",
            release("two", |_| None),
            "SHA256SUMS.cosign-bundle",
        ),
        (
            "a binary SHA256SUMS does not list at its digest",
            {
                let mut r = good("two");
                r.insert("goatbuilder-linux-amd64".into(), b"another build".to_vec());
                r
            },
            "goatbuilder-linux-amd64",
        ),
        (
            "a .sha256 that disagrees with SHA256SUMS",
            {
                let mut r = good("two");
                r.insert(
                    "goatbuilder-darwin-arm64.sha256".into(),
                    format!("{}  goatbuilder-darwin-arm64\n", sha256(b"x")).into_bytes(),
                );
                r
            },
            "goatbuilder-darwin-arm64.sha256",
        ),
    ];
    for (what, bad, names) in cases {
        let before = surface.snapshot();
        let o = surface.deploy(&bin, bad);
        assert_ne!(o.status.code(), Some(0), "{what}: served: {}", text(&o));
        assert!(
            text(&o).contains(names),
            "{what}: the refusal does not name {names}: {}",
            text(&o)
        );
        assert!(
            surface.snapshot() == before,
            "{what}: the served directory changed on a refusal"
        );
        assert!(
            surface.leftovers().is_empty(),
            "{what}: left behind {:?}",
            surface.leftovers()
        );
    }

    // And the next release that verifies replaces the served one.
    let next = good("three");
    let o = surface.deploy(&bin, next.clone());
    assert_eq!(
        o.status.code(),
        Some(0),
        "the next good release: {}",
        text(&o)
    );
    for (name, bytes) in &next {
        assert_eq!(
            std::fs::read(surface.served_dir().join(name)).ok().as_ref(),
            Some(bytes),
            "{name} is not the next release's"
        );
    }
}
