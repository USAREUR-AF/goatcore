//! The play a release component carries in its own archive (`release.play`;
//! docs/design/goatbuilder-build-targets.md section 3.8). The archive is
//! release_file.rs's: verified against the pin, its signature and its checksums
//! file. The play's directory is unpacked from those bytes, not from a file that
//! could have changed since, and what is on disk is held to them on every run
//! (release_play.py). ops/ansible/release-archive-play.yml imports it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use sha2::{Digest, Sha256};

use crate::release_file::Fetched;
use crate::runner::Cmd;

const SCRIPT: &str = include_str!("release_play.py");
/// The directory under the state directory the plays are unpacked in, which the
/// wrapper play knows.
const DIR: &str = "release-plays";

/// This checkout, in twelve characters.
pub(crate) fn here(repo: &Path) -> String {
    let repo = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    let h: String = Sha256::digest(repo.display().to_string().as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    h[..12].to_string()
}

/// Where the play of one verified archive is, for this checkout.
fn dest(state: &Path, component: &str, repo: &Path, sha256: &str) -> PathBuf {
    state.join(DIR).join(component).join(format!("{sha256}-{}", here(repo)))
}

/// Unpack `play` of `component`'s verified archive and return its path; the
/// reason, otherwise.
pub(crate) fn unpack(repo: &Path, state: &Path, component: &str, f: &Fetched, play: &str) -> Result<PathBuf, String> {
    let repo = repo.canonicalize().map_err(|e| format!("{}: {e}", repo.display()))?;
    // The bytes that were verified: read again and held to their SHA-256, then
    // handed to the script.
    let bytes = std::fs::read(&f.path).map_err(|e| format!("{} cannot be read: {e}", f.path.display()))?;
    if goat_selfupdate::sha256_hex(&bytes) != f.sha256 {
        return Err(format!("{} changed while it was read", f.path.display()));
    }
    let mut c = Cmd::new("python3");
    c.args(["-I", "-B", "-c", SCRIPT]).arg(dest(state, component, &repo, &f.sha256)).arg(play).arg(repo.join("ops/ansible"));
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("could not start python3: {e}"))?;
    let mut pipe = child.stdin.take().ok_or("python3 takes no input")?;
    // It reads all of its input before it prints; a write that fails is its refusal.
    let _ = pipe.write_all(&bytes);
    drop(pipe);
    let out = child.wait_with_output().map_err(|e| format!("python3: {e}"))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        return Err(said.lines().last().unwrap_or("the play was not unpacked").trim().chars().take(300).collect());
    }
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    hosts_are_the_target(&text).map_err(|why| format!("its play {play}: {why}"))?;
    Ok(p)
}

/// The hosts every play of the file is run on, as Ansible reads the file: each
/// play says exactly `hosts: "{{ target | mandatory }}"`, the group the driver
/// names, and none imports another file. A line that looks right is not enough:
/// a second play, a list item or a longer template would reach other hosts.
fn hosts_are_the_target(text: &str) -> Result<(), String> {
    const HOSTS: &str = "{{ target | mandatory }}";
    let want = format!("each play says hosts: \"{HOSTS}\" and imports no other");
    let doc: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| format!("it is not YAML that can be read ({}); {want}", e.to_string().chars().take(120).collect::<String>()))?;
    let Some(plays) = doc.as_sequence().filter(|s| !s.is_empty()) else { return Err(format!("it is not a list of plays; {want}")) };
    for (i, play) in plays.iter().enumerate() {
        let Some(m) = play.as_mapping() else { return Err(format!("play {} is not a mapping; {want}", i + 1)) };
        if m.keys().any(|k| k.as_str().is_some_and(|k| k.ends_with("import_playbook") || k == "<<")) {
            return Err(format!("play {} imports another or merges keys; {want}", i + 1));
        }
        // A plain string: a tagged one (`!unsafe`) is another value to Ansible.
        if !matches!(m.get("hosts"), Some(serde_yaml::Value::String(h)) if h == HOSTS) {
            return Err(format!("play {} has another hosts:; {want}", i + 1));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAY: &str = "- name: t\n  hosts: \"{{ target | mandatory }}\"\n  tasks: []\n";

    /// An archive from [type, name, content or target] rows, by python's own tarfile.
    fn archive(dir: &Path, rows: &[(&str, &str, &str)]) -> Fetched {
        let path = dir.join("tool.tar.gz");
        let make = "import io,json,sys,tarfile\nt=tarfile.open(sys.argv[1],'w:gz')\nfor k,n,c in json.loads(sys.argv[2]):\n i=tarfile.TarInfo(n); i.type={'f':tarfile.REGTYPE,'d':tarfile.DIRTYPE,'l':tarfile.SYMTYPE,'h':tarfile.LNKTYPE,'p':tarfile.FIFOTYPE}[k]\n if k in 'lh': i.linkname=c\n b=c.encode() if k=='f' else b''\n i.size=len(b); t.addfile(i,io.BytesIO(b))\nt.close()";
        let rows = serde_json::to_string(rows).unwrap();
        assert!(std::process::Command::new("python3").args(["-I", "-c", make]).arg(&path).arg(rows).status().unwrap().success());
        fetched(path)
    }

    fn fetched(path: PathBuf) -> Fetched {
        let sha256 = goat_selfupdate::sha256_hex(&std::fs::read(&path).unwrap());
        Fetched { path, name: "tool.tar.gz".into(), sha256, signed_by: String::new(), log_index: 0 }
    }

    fn checkout(dir: &Path) -> PathBuf {
        let r = dir.join("repo");
        for d in ["group_vars/g", "host_vars", "roles/release-file/tasks", "roles/rollback-ledger"] {
            std::fs::create_dir_all(r.join("ops/ansible").join(d)).unwrap();
        }
        std::fs::write(r.join("ops/ansible/group_vars/g/v.yml"), "seen: true\n").unwrap();
        r
    }

    const GOOD: &[(&str, &str, &str)] = &[
        ("d", "tool-1", ""),
        ("f", "tool-1/README", "x"),
        ("f", "tool-1/deploy/play.yml", PLAY),
        ("f", "tool-1/deploy/roles/tool/tasks/main.yml", "[]\n"),
    ];

    #[test]
    fn every_play_of_the_file_runs_on_the_target_group_only() {
        assert_eq!(hosts_are_the_target(PLAY), Ok(()));
        assert_eq!(hosts_are_the_target(&format!("{PLAY}- name: u\n  hosts: '{{{{ target | mandatory }}}}'\n  roles: [tool]\n")), Ok(()));
        for (text, words) in [
            ("- hosts: all\n  vars:\n    x:\n      hosts: \"{{ target | mandatory }}\"\n", "play 1 has another hosts:"),
            ("- hosts: \"{{ target ~ ',all' }}\"\n", "play 1 has another hosts:"),
            (&format!("{PLAY}- hosts: all\n  tasks: []\n"), "play 2 has another hosts:"),
            ("- name: t\n  tasks: []\n", "play 1 has another hosts:"),
            ("- hosts: !unsafe \"{{ target | mandatory }}\"\n", "play 1 has another hosts:"),
            ("- hosts: \"{{ target | mandatory }}\"\n  hosts: all\n", "not YAML that can be read"),
            ("- import_playbook: other.yml\n", "imports another"),
            ("- ansible.builtin.import_playbook: other.yml\n  hosts: \"{{ target | mandatory }}\"\n", "imports another"),
            ("- x: &a {hosts: all}\n  hosts: \"{{ target | mandatory }}\"\n- <<: *a\n", "merges keys"),
            ("hosts: \"{{ target | mandatory }}\"\n", "not a list of plays"),
            ("[]\n", "not a list of plays"),
            ("- just a string\n", "play 1 is not a mapping"),
        ] {
            let e = hosts_are_the_target(text).expect_err(text);
            assert!(e.contains(words), "{text:?}: {e}");
        }
    }

    #[test]
    fn the_plays_directory_is_unpacked_with_this_checkouts_variables_and_roles_beside_it() {
        let d = tempfile::tempdir().unwrap();
        let (repo, state) = (checkout(d.path()), d.path().join("state"));
        let f = archive(d.path(), GOOD);
        let p = unpack(&repo, &state, "tool", &f, "deploy/play.yml").unwrap();
        let at = p.parent().unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), PLAY);
        assert!(at.join("roles/tool/tasks/main.yml").is_file() && at.join("roles/release-file/tasks").is_dir());
        assert!(at.join("group_vars/g/v.yml").is_file() && !at.join("README").exists());
        let wrapper = regex::Regex::new(r"^/.+/release-plays/[a-z0-9][a-z0-9._-]*/[0-9a-f]{64}-[0-9a-f]{12}/.+\.ya?ml$").unwrap();
        assert!(wrapper.is_match(p.to_str().unwrap()), "{}", p.display());
        // The same again, and after this checkout gained a role.
        std::fs::create_dir_all(repo.join("ops/ansible/roles/newer")).unwrap();
        assert_eq!(unpack(&repo, &state, "tool", &f, "deploy/play.yml").unwrap(), p);
        assert!(at.join("roles/newer").is_dir());
        // Another checkout has a directory of its own.
        let other = checkout(&d.path().join("o"));
        assert_ne!(here(&repo), here(&other));
        assert_ne!(unpack(&other, &state, "tool", &f, "deploy/play.yml").unwrap(), p);
    }

    #[test]
    fn a_play_that_is_not_the_archives_is_never_run() {
        let d = tempfile::tempdir().unwrap();
        let (repo, state) = (checkout(d.path()), d.path().join("state"));
        let f = archive(d.path(), GOOD);
        let p = unpack(&repo, &state, "tool", &f, "deploy/play.yml").unwrap();
        let at = p.parent().unwrap().to_path_buf();
        let refused = || unpack(&repo, &state, "tool", &f, "deploy/play.yml").unwrap_err();
        std::fs::write(&p, "changed\n").unwrap();
        assert!(refused().contains("is not the archive's (play.yml)"));
        std::fs::write(&p, PLAY).unwrap();
        std::fs::write(at.join("roles/tool/tasks/extra.yml"), "[]\n").unwrap();
        assert!(refused().contains("extra.yml"));
        std::fs::remove_file(at.join("roles/tool/tasks/extra.yml")).unwrap();
        // The play itself replaced by a link to the same bytes.
        std::fs::rename(&p, d.path().join("elsewhere.yml")).unwrap();
        std::os::unix::fs::symlink(d.path().join("elsewhere.yml"), &p).unwrap();
        assert!(refused().contains("is not the archive's"));
        std::fs::remove_file(&p).unwrap();
        std::fs::write(&p, PLAY).unwrap();
        // A link that is not this checkout's is put back, and a role no one has is not left.
        std::fs::remove_file(at.join("group_vars")).unwrap();
        std::os::unix::fs::symlink(d.path(), at.join("group_vars")).unwrap();
        std::os::unix::fs::symlink(d.path(), at.join("roles/planted")).unwrap();
        unpack(&repo, &state, "tool", &f, "deploy/play.yml").unwrap();
        assert_eq!(std::fs::read_link(at.join("group_vars")).unwrap(), repo.canonicalize().unwrap().join("ops/ansible/group_vars"));
        assert!(std::fs::symlink_metadata(at.join("roles/planted")).is_err());
        // The directory itself a link.
        std::fs::rename(&at, d.path().join("moved")).unwrap();
        std::os::unix::fs::symlink(d.path().join("moved"), &at).unwrap();
        assert!(refused().contains("is not a directory of this user's"));
        // The archive changed after it was verified.
        std::fs::remove_file(&at).unwrap();
        std::fs::write(&f.path, "other").unwrap();
        assert!(refused().contains("changed while it was read"));
    }

    #[test]
    fn an_archive_that_would_write_elsewhere_or_bring_what_is_not_its_own_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let (repo, state) = (checkout(d.path()), d.path().join("state"));
        for (row, words) in [
            (("f", "tool-1/../../x", "x"), "not a plain one"),
            (("f", "/abs/x", "x"), "not a plain one"),
            (("f", "tool-1/./x", "x"), "not a plain one"),
            (("f", "tool-1/a\u{1b}b", "x"), "not a plain one"),
            (("l", "tool-1/deploy/link", "/etc/passwd"), "not a plain file or directory"),
            (("h", "tool-1/deploy/hard", "tool-1/README"), "not a plain file or directory"),
            (("p", "tool-1/deploy/fifo", ""), "not a plain file or directory"),
            (("f", "other/x", "x"), "more than one top directory"),
            (("f", "tool-1/deploy/play.yml", "again"), "twice"),
            (("f", "tool-1/deploy/play.yml/x", "x"), "a file and a directory"),
            (("f", "tool-1/deploy/group_vars/all.yml", "x"), "its own group_vars"),
            (("f", "tool-1/deploy/host_vars/h.yml", "x"), "its own host_vars"),
            (("f", "tool-1/deploy/roles/release-file/tasks/main.yml", "x"), "a role release-file, which goatcore has"),
        ] {
            let rows: Vec<_> = GOOD.iter().copied().chain([row]).collect();
            let e = unpack(&repo, &state, "tool", &archive(d.path(), &rows), "deploy/play.yml").unwrap_err();
            assert!(e.contains(words), "{row:?}: {e}");
            assert!(!e.contains('\u{1b}'));
        }
        assert!(unpack(&repo, &state, "tool", &archive(d.path(), GOOD), "deploy/other.yml").unwrap_err().contains("has no file"));
        let not = d.path().join("not.tar.gz");
        std::fs::write(&not, "not an archive").unwrap();
        assert!(unpack(&repo, &state, "tool", &fetched(not), "deploy/play.yml").unwrap_err().contains("not a gzipped tar"));
        let hosts = [("f", "tool-1/deploy/play.yml", "- name: t\n  hosts: all\n  tasks: []\n")];
        let e = unpack(&repo, &state, "tool", &archive(d.path(), &hosts), "deploy/play.yml").unwrap_err();
        assert!(e.contains("target | mandatory"), "{e}");
        // Ansible code of its own, beside the play or in its role; a role of
        // goatcore's name as a file; two names that are one where case is ignored.
        for (row, words) in [
            (("f", "tool-1/deploy/library/copy.py", "x"), "Ansible code of its own"),
            (("f", "tool-1/deploy/module_utils/u.py", "x"), "Ansible code of its own"),
            (("f", "tool-1/deploy/filter_plugins/f.py", "x"), "Ansible code of its own"),
            (("f", "tool-1/deploy/vars_plugins/v.py", "x"), "Ansible code of its own"),
            (("f", "tool-1/deploy/collections/ansible_collections/a/b/galaxy.yml", "x"), "Ansible code of its own"),
            (("f", "tool-1/deploy/roles/tool/library/m.py", "x"), "Ansible code of its own"),
            (("f", "tool-1/deploy/roles/tool/action_plugins/a.py", "x"), "Ansible code of its own"),
            (("f", "tool-1/deploy/roles/release-file", "x"), "a role release-file, which goatcore has"),
            (("f", "tool-1/deploy/Play.yml", "x"), "one name where case is ignored"),
        ] {
            let rows: Vec<_> = GOOD.iter().copied().chain([row]).collect();
            let e = unpack(&repo, &state, "tool", &archive(d.path(), &rows), "deploy/play.yml").unwrap_err();
            assert!(e.contains(words), "{row:?}: {e}");
        }
        assert!(std::fs::read_dir(state.join(DIR).join("tool")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with('.')));
    }
}
