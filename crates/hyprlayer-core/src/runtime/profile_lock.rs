use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use fs4::FileExt;

use crate::error::{CoreError, Result};

fn held() -> &'static Mutex<HashSet<PathBuf>> {
    static HELD: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(HashSet::new()))
}

struct Claim(PathBuf);

impl Drop for Claim {
    fn drop(&mut self) {
        if let Ok(mut set) = held().lock() {
            set.remove(&self.0);
        }
    }
}

/// An exclusive claim on one profile, held for the lifetime of the runtime that
/// opened it.
///
/// Two layers, because one cannot cover both cases. `flock` answers across
/// processes but is owned by the open file description, so a second `open` in
/// *this* process would be refused only by luck of path spelling; the in-process
/// registry answers that case directly and deterministically. The registry alone
/// would say nothing about other processes.
pub(crate) struct ProfileLock {
    file: std::fs::File,
    _claim: Claim,
}

impl ProfileLock {
    pub(crate) fn acquire(app_data_dir: &Path) -> Result<Self> {
        let busy = || CoreError::RuntimeInUse {
            path: app_data_dir.to_path_buf(),
        };

        if let Some(parent) = app_data_dir.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::create_dir_all(app_data_dir)?;
        let lock_path = app_data_dir.join(".runtime.lock");

        // Deliberately a `std::fs::File`: `tokio::fs::File` defers its close to the
        // blocking pool, so the lock would outlive the drop.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;

        let key = std::fs::canonicalize(&lock_path).unwrap_or_else(|_| lock_path.clone());
        {
            let mut set = held().lock().map_err(|_| busy())?;
            if !set.insert(key.clone()) {
                return Err(busy());
            }
        }
        let claim = Claim(key);

        if FileExt::try_lock(&file).is_err() {
            return Err(busy());
        }

        Ok(Self {
            file,
            _claim: claim,
        })
    }
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        // Release explicitly rather than letting `close` do it. An `flock` lives on
        // the open file description, so a child forked anywhere in this process
        // holds a duplicate that keeps the lock alive past our close until it
        // execs; `LOCK_UN` frees the description itself, so the profile is free the
        // moment the runtime is. On Windows, release-on-close is documented as not
        // synchronous, which this avoids for the same reason.
        //
        // Safe here only because `flock` has no unlock-any-fd hazard: this handle
        // is the one that took the lock, and the guard is constructed only on the
        // path that acquired it. Under POSIX record locks the same call would be a
        // silent lock-loss bug.
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_acquire_in_this_process_is_refused() {
        let dir = tempfile::tempdir().unwrap();

        let first = ProfileLock::acquire(dir.path()).expect("first acquire");
        let second = ProfileLock::acquire(dir.path());
        assert!(matches!(second, Err(CoreError::RuntimeInUse { .. })));
        drop(first);
    }

    #[test]
    fn dropping_the_guard_frees_the_profile() {
        let dir = tempfile::tempdir().unwrap();

        drop(ProfileLock::acquire(dir.path()).expect("first"));
        drop(ProfileLock::acquire(dir.path()).expect("second after drop"));
    }

    #[test]
    fn a_refused_acquire_leaves_the_holders_claim_intact() {
        let dir = tempfile::tempdir().unwrap();

        let held_lock = ProfileLock::acquire(dir.path()).unwrap();
        for _ in 0..5 {
            assert!(ProfileLock::acquire(dir.path()).is_err());
        }
        drop(held_lock);
        ProfileLock::acquire(dir.path()).expect("free once the holder drops");
    }

    #[test]
    fn distinct_profiles_do_not_contend() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();

        let _first = ProfileLock::acquire(a.path()).expect("a");
        ProfileLock::acquire(b.path()).expect("b is a different profile");
    }
}
