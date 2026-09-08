/*
  lock.rs

  Created on 2026-09-07 by Thomas Bonk <thomas@meandmymac.de>
  Copyright 2026 SepiaOS Development Team

  Licensed under the Apache License, Version 2.0 (the "License");
  you may not use this file except in compliance with the License.
  You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

  Unless required by applicable law or agreed to in writing, software
  distributed under the License is distributed on an "AS IS" BASIS,
  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
  See the License for the specific language governing permissions and
  limitations under the License.
*/

//! The single-writer lock.
//!
//! Held on `/var/lib/spm/lock` by every command that writes and by none that
//! only reads. Two `spm`s installing at once would each decide what to do from
//! a picture the other was already changing, and the loser would record files
//! it did not write.
//!
//! The lock is the kernel's, taken on an open file, so it goes when the process
//! goes — including when the process is killed, or the device loses power.
//! There is no stale lock to detect, no timeout to tune, and nothing to clean
//! up on the way back.
//!
//! The holder writes its process id into the file after taking the lock, so
//! that a waiter can say *what* it is waiting for rather than only that it is
//! waiting.
//!
//! This is `std::fs::File::lock`, stable since Rust 1.89 — which is why this
//! module contains no `unsafe` and the crate needs no `libc`. The design was
//! written expecting `flock` through `libc`; std having grown the same thing
//! is strictly better and the documents have been corrected.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// A held lock. Releasing it is dropping it.
#[derive(Debug)]
pub struct Lock {
    /// Kept because the lock lives on the open file: closing it releases.
    file: File,
    path: PathBuf,
}

impl Lock {
    /// Take the lock if it is free, or return `Ok(None)` if somebody has it.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the lock file cannot be made or opened. Being held by
    /// somebody else is not an error — it is `Ok(None)`, because the caller
    /// usually wants to say so and then wait.
    pub fn try_acquire(path: &Path) -> Result<Option<Self>> {
        let file = open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self::record(file, path)?)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(source)) => Err(Error::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Take the lock, waiting for as long as it takes.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the lock file cannot be made, opened or locked.
    pub fn wait(path: &Path) -> Result<Self> {
        let file = open(path)?;
        file.lock().map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::record(file, path)
    }

    /// The process id recorded in the lock file, if there is one to read.
    ///
    /// Only ever a diagnostic. It is read without holding anything, so it can
    /// be out of date by the time it is printed — which is harmless, because
    /// the thing it is used for is a sentence telling somebody why they are
    /// waiting.
    #[must_use]
    pub fn holder(path: &Path) -> Option<u32> {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    /// The lock file this lock is on.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write our process id into the file, now that we hold it.
    fn record(mut file: File, path: &Path) -> Result<Self> {
        let note = |source| Error::Io {
            path: path.to_path_buf(),
            source,
        };
        file.set_len(0).map_err(note)?;
        write!(file, "{}", std::process::id()).map_err(note)?;
        file.flush().map_err(note)?;
        Ok(Lock {
            file,
            path: path.to_path_buf(),
        })
    }
}

impl Drop for Lock {
    /// Release it.
    ///
    /// Closing the file would do this on its own, and so would the process
    /// ending — the kernel owns the lock, which is what makes a killed `spm`
    /// leave nothing stale behind. Doing it explicitly says so, and a failure
    /// here is not something a caller could act on.
    fn drop(&mut self) {
        drop(self.file.unlock());
    }
}

/// Open the lock file, making its directory if it is not there yet.
fn open(path: &Path) -> Result<File> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|source| Error::Io {
            path: directory.to_path_buf(),
            source,
        })?;
    }
    // Not truncating: truncation would destroy the holder's recorded id
    // before we know whether we are allowed to have the lock at all.
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// Two handles on one file contend exactly as two processes do: the lock
    /// belongs to the open file, not to the process that opened it. So the
    /// contention below is the real thing, without a helper binary to run.
    #[test]
    fn a_second_taker_is_told_it_is_held() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");

        let held = Lock::wait(&path).unwrap();
        assert!(Lock::try_acquire(&path).unwrap().is_none());

        drop(held);
        assert!(Lock::try_acquire(&path).unwrap().is_some());
    }

    #[test]
    fn a_waiter_blocks_until_the_holder_lets_go() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let held = Lock::wait(&path).unwrap();

        let (sender, receiver) = mpsc::channel();
        let waiting_on = path.clone();
        let waiter = thread::spawn(move || {
            let lock = Lock::wait(&waiting_on).unwrap();
            sender.send(()).unwrap();
            lock
        });

        // Still held, so the waiter must not have got through.
        assert!(
            receiver.recv_timeout(Duration::from_millis(150)).is_err(),
            "the second taker did not wait for the first"
        );

        drop(held);

        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("the waiter should have been let through");
        drop(waiter.join().unwrap());
    }

    #[test]
    fn the_holder_says_who_it_is() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");

        assert_eq!(Lock::holder(&path), None, "nothing has held it yet");

        let held = Lock::wait(&path).unwrap();
        assert_eq!(Lock::holder(&path), Some(std::process::id()));
        assert_eq!(held.path(), path);
    }

    #[test]
    fn taking_the_lock_makes_the_directory() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("var/lib/spm/lock");

        let held = Lock::wait(&path).unwrap();

        assert!(path.exists());
        drop(held);
    }

    #[test]
    fn a_waiter_can_read_who_holds_it_before_waiting() {
        // The sequence a command actually uses: try, find it held, read the
        // holder to say what it is waiting for, then wait.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let held = Lock::wait(&path).unwrap();

        assert!(Lock::try_acquire(&path).unwrap().is_none());
        assert_eq!(Lock::holder(&path), Some(std::process::id()));

        drop(held);
    }
}
