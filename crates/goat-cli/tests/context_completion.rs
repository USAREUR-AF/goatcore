use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn goat(profile: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_goat"))
        .arg("--profile-dir")
        .arg(profile)
        .args(args)
        .env_remove("GOAT_CONTEXT")
        .env_remove("GOAT_PROFILE_DIR")
        .env("GOAT_LANG", "en")
        .env("NO_COLOR", "1")
        .output()
        .expect("run compiled goat binary")
}

fn stdout_lines(output: &Output) -> Vec<String> {
    String::from_utf8(output.stdout.clone())
        .expect("stdout is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn completion(profile: &Path, partial: &str) -> Vec<String> {
    let output = goat(profile, &["_complete", "context-name", partial]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    stdout_lines(&output)
}

fn listed_contexts(profile: &Path) -> Vec<String> {
    let output = goat(profile, &["context", "ls", "--json"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    stdout_lines(&output)
        .into_iter()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(&line).expect("context ls emits JSON")["name"]
                .as_str()
                .expect("context entry has a name")
                .to_owned()
        })
        .collect()
}

#[test]
fn missing_and_empty_context_directories_complete_to_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("missing-profile");
    fs::create_dir(&missing).unwrap();

    assert!(listed_contexts(&missing).is_empty());
    assert!(completion(&missing, "").is_empty());

    let empty = tmp.path().join("empty-profile");
    fs::create_dir_all(empty.join("contexts")).unwrap();

    assert!(listed_contexts(&empty).is_empty());
    assert!(completion(&empty, "").is_empty());
}

#[test]
fn one_context_has_listing_and_completion_parity() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("contexts/zagros")).unwrap();

    assert_eq!(listed_contexts(tmp.path()), ["zagros"]);
    assert_eq!(completion(tmp.path(), ""), ["zagros"]);
}

#[test]
fn completion_uses_the_listed_context_set_before_prefix_filtering() {
    let tmp = tempfile::tempdir().unwrap();
    let contexts = tmp.path().join("contexts");
    for name in ["zagros", "alpha.bak", "alpine", "alpha"] {
        fs::create_dir_all(contexts.join(name)).unwrap();
    }
    fs::write(contexts.join("alfile"), []).unwrap();
    fs::write(contexts.join(".DS_Store"), []).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        symlink(contexts.join("alpha"), contexts.join("alpha-link")).unwrap();
    }

    #[cfg(target_os = "linux")]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        fs::create_dir(contexts.join(OsString::from_vec(vec![b'a', b'l', 0xff]))).unwrap();
    }

    let listed = listed_contexts(tmp.path());
    assert_eq!(listed, ["alpha", "alpine", "zagros"]);
    assert_eq!(completion(tmp.path(), ""), listed);
    assert_eq!(completion(tmp.path(), "al"), ["alpha", "alpine"]);
    assert!(completion(tmp.path(), "nomatch").is_empty());

    let use_file = goat(tmp.path(), &["context", "use", "alfile"]);
    assert!(!use_file.status.success(), "{use_file:?}");
    assert!(use_file.stdout.is_empty(), "{use_file:?}");
    assert!(String::from_utf8_lossy(&use_file.stderr).contains("does not exist"));
}

#[test]
fn completion_suppresses_enumeration_errors_that_listing_reports() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("contexts"), "not a directory").unwrap();

    let complete = goat(tmp.path(), &["_complete", "context-name", ""]);
    assert!(complete.status.success(), "{complete:?}");
    assert!(complete.stdout.is_empty(), "{complete:?}");
    assert!(complete.stderr.is_empty(), "{complete:?}");

    let list = goat(tmp.path(), &["context", "ls", "--json"]);
    assert!(!list.status.success(), "{list:?}");
    assert!(list.stdout.is_empty(), "{list:?}");
    assert!(!list.stderr.is_empty(), "{list:?}");
}
