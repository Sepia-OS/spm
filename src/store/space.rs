/*
  space.rs

  Created on 2026-09-08 by Thomas Bonk <thomas@meandmymac.de>
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

//! How much room is left.
//!
//! Mechanism, like the rest of `store`: it reports a number and decides
//! nothing. What that number has to be is `install`'s to say, because a package
//! is on the card **twice** while it installs — the archive in
//! `/var/cache/spm/` and the unpacked tree under `/usr` — and on a 2 GiB card
//! with Rust and Helix already on it that is not a hypothetical.
//!
//! There is no free-space call in `std`, so this is `statvfs`. Reaching it
//! through `libc` would mean the crate's only `unsafe` block; `rustix` wraps
//! the same call safely and is already in the dependency tree underneath
//! `tempfile`, so the crate still contains no `unsafe` at all.

use std::path::Path;

use crate::error::{Error, Result};

/// How many bytes are free on the filesystem holding `path`.
///
/// # Errors
///
/// [`Error::Io`] if the filesystem cannot be interrogated — a path that is not
/// there, most likely.
#[cfg(unix)]
pub fn available(path: &Path) -> Result<u64> {
    let statistics = rustix::fs::statvfs(path).map_err(|errno| Error::Io {
        path: path.to_path_buf(),
        source: std::io::Error::from(errno),
    })?;

    // f_bavail, not f_bfree: the blocks a non-privileged process may use.
    // `spm` runs as root and could dip into the reserve, but filling the
    // reserve on the filesystem the device boots from is not something a
    // package install should be allowed to do.
    Ok(statistics.f_bavail.saturating_mul(block_size(&statistics)))
}

/// The size of the blocks `f_bavail` counts.
///
/// `f_frsize` is the fragment size and the one the count is in; some systems
/// leave it zero, and there `f_bsize` is what it means.
#[cfg(unix)]
fn block_size(statistics: &rustix::fs::StatVfs) -> u64 {
    if statistics.f_frsize == 0 {
        statistics.f_bsize
    } else {
        statistics.f_frsize
    }
}

/// How many bytes are free on the filesystem holding `path`.
///
/// # Errors
///
/// Never here: a platform without `statvfs` is not a platform SepiaOS runs on,
/// and refusing every install on one would be worse than not checking.
#[cfg(not(unix))]
pub fn available(_path: &Path) -> Result<u64> {
    Ok(u64::MAX)
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

    #[test]
    fn a_real_directory_has_a_real_amount_of_room_left() {
        let directory = tempfile::tempdir().unwrap();
        let free = available(directory.path()).unwrap();
        // Anything running this test has room for something.
        assert!(free > 0, "no room at all on the temporary directory");
    }

    #[test]
    fn writing_a_file_does_not_make_more_room() {
        // Weak on purpose: the number moves for reasons that have nothing to
        // do with this test. What it does establish is that the reading comes
        // from a filesystem rather than from a constant.
        let directory = tempfile::tempdir().unwrap();
        let before = available(directory.path()).unwrap();
        std::fs::write(directory.path().join("big"), vec![0_u8; 1 << 20]).unwrap();
        let after = available(directory.path()).unwrap();
        assert!(
            after <= before,
            "{after} > {before} after writing a megabyte"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_there_says_so() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("nowhere");
        match available(&missing) {
            Err(Error::Io { path, .. }) => assert_eq!(path, missing),
            other => panic!("expected an Io error naming the path, got {other:?}"),
        }
    }
}
