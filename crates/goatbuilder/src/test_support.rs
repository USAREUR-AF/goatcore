//! Helpers the tests share (unit tests through `mod test_support`, integration
//! tests through `#[path = "../src/test_support.rs"]`). Test code only.
//!
//! **Executables.** A test that writes a stand-in script and then runs it
//! raced the other test threads: while this process held the script open for
//! writing, another thread's fork inherited that descriptor, and until the
//! child called exec, running the script failed with ETXTBSY ("Text file
//! busy", os error 26). `executable` and `copied_executable` write the file
//! in a child process, so this process never holds a descriptor open for
//! writing that a fork could inherit. The child has exited, and its
//! descriptor is closed, before the path is handed to the code under test.
//!
//! A run's home directory is tests/common/test_home.rs, not this file.
#![allow(dead_code)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Write `text` to `path` as an executable (mode 0755), creating its parent.
// `cfg(test)` on each item, though the whole file is test code: the scan in
// tests/legibility_one_door.rs reads it item by item, and test code may start
// its own processes.
#[cfg(test)]
pub fn executable(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut child = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn sh to write an executable");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(text.as_bytes())
        .unwrap();
    let st = child.wait().unwrap();
    assert!(
        st.success(),
        "could not write the executable {}",
        path.display()
    );
}

/// Copy `from` to `to` keeping its mode, the way `executable` writes one.
#[cfg(test)]
pub fn copied_executable(from: &Path, to: &Path) {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let st = Command::new("cp")
        .arg("-p")
        .arg(from)
        .arg(to)
        .status()
        .expect("spawn cp");
    assert!(
        st.success(),
        "could not copy {} to {}",
        from.display(),
        to.display()
    );
}
