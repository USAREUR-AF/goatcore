//! Test fixtures own their directory. Each one is a `tempfile::TempDir` under
//! the system temp directory, with a prefix that names the test, and it is
//! removed when the fixture is dropped, a panicking test included.

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

/// A fixture value held together with the temporary directory it lives in.
/// It derefs to the value; the directory goes when it drops.
pub struct Held<T> {
    value: T,
    _dir: tempfile::TempDir,
}

impl<T> Held<T> {
    /// Keep the directory of `dir` alive for as long as `value` is used.
    pub fn with<U>(dir: Held<U>, value: T) -> Held<T> {
        Held {
            value,
            _dir: dir._dir,
        }
    }
}

impl<T> Deref for Held<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for Held<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T: AsRef<Path>> AsRef<Path> for Held<T> {
    fn as_ref(&self) -> &Path {
        self.value.as_ref()
    }
}

/// A new, empty directory named `<prefix><random>`, removed on drop.
pub fn path(prefix: &str) -> Held<PathBuf> {
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("a temporary directory for the test");
    Held {
        value: dir.path().to_path_buf(),
        _dir: dir,
    }
}
