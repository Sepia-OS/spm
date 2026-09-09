/*
  atomic.rs

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

//! Write-then-rename, for every file that must survive a power cut.
//!
//! A device loses power mid-write, and what it must never find afterwards is
//! half a file. So nothing is written in place: the bytes go to a temporary
//! file, that file is flushed to the disk, and only then is it renamed over the
//! destination. `rename` within one directory is atomic, so a reader sees
//! either all of the old file or all of the new one and never a mixture.
//!
//! The temporary file goes in the **destination directory**, not `/tmp`. A
//! rename across filesystems is not a rename at all — it is a copy and a
//! delete, which is exactly the non-atomic write this module exists to avoid —
//! and on a SepiaOS card `/tmp` may well be a different filesystem.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use tempfile::NamedTempFile;

use crate::error::{Error, Result};

/// Write `contents` to `path`, atomically.
///
/// # Errors
///
/// [`Error::Io`] if the directory cannot be made, the bytes cannot be written,
/// or the rename fails. On any failure the destination is left exactly as it
/// was and no temporary file is left behind.
pub fn write(path: &Path, contents: &[u8]) -> Result<()> {
    write_with(path, |file| file.write_all(contents))
}

/// Write to `path` atomically, readable by nobody but its owner.
///
/// For a private key, which is the only thing this program writes that is a
/// secret. The mode is set on the temporary file *before* the rename, so the
/// key is never briefly world-readable under its final name - a window that
/// would be short and entirely sufficient.
///
/// # Errors
///
/// [`Error::Io`] if anything fails. The destination is untouched in that case.
pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    write_with(path, |file| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents)
    })
}

/// Write to `path` atomically, by handing an open file to `produce`.
///
/// The form to use when the content is not already a slice — serialising JSON
/// straight into the file, or copying a stream through it, without building the
/// whole thing in memory first. That matters here: a package is 216 MiB and the
/// smallest supported board has 512 MiB of RAM.
///
/// # Errors
///
/// [`Error::Io`] if anything fails, including a failure inside `produce`. The
/// destination is untouched in every one of those cases.
pub fn write_with<F>(path: &Path, produce: F) -> Result<()>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let directory = path.parent().unwrap_or_else(|| Path::new("."));

    // Idempotent, and it means every caller does not have to remember. The
    // paths all come from `Store`, so there is no wrong directory to create.
    fs::create_dir_all(directory).map_err(|source| Error::Io {
        path: directory.to_path_buf(),
        source,
    })?;

    // In `directory`, not the system temporary directory - see above.
    let mut temporary = NamedTempFile::new_in(directory).map_err(|source| Error::Io {
        path: directory.to_path_buf(),
        source,
    })?;

    // A failure here drops the temporary file, which deletes it.
    produce(temporary.as_file_mut()).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;

    // The bytes have to be on the disk before the rename makes them visible.
    // Without this, a power cut can leave a renamed file full of nothing.
    temporary.as_file().sync_all().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;

    temporary.persist(path).map_err(|error| Error::Io {
        path: path.to_path_buf(),
        source: error.error,
    })?;

    // And the rename itself has to be on the disk, or a power cut can lose the
    // directory entry that points at the bytes we just flushed. A filesystem
    // that will not sync a directory is not a reason to fail the write.
    if let Ok(handle) = File::open(directory) {
        drop(handle.sync_all());
    }

    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn it_writes_a_new_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.json");

        write(&path, b"{}").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"{}");
    }

    #[test]
    fn it_replaces_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.json");
        fs::write(&path, b"old").unwrap();

        write(&path, b"new").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn a_failure_partway_leaves_the_old_file_untouched() {
        // The reason this module exists: an `update` that dies halfway through
        // must leave the previous index, not half of the next one.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.json");
        fs::write(&path, b"the previous index").unwrap();

        let outcome = write_with(&path, |file| {
            file.write_all(b"half of the ne")?;
            Err(io::Error::new(
                ErrorKind::ConnectionReset,
                "the network went",
            ))
        });

        assert!(outcome.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"the previous index");
    }

    #[test]
    fn a_failure_leaves_no_temporary_file_behind() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.json");
        fs::write(&path, b"the previous index").unwrap();

        let _ = write_with(&path, |_| {
            Err(io::Error::new(
                ErrorKind::ConnectionReset,
                "the network went",
            ))
        });

        let left: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, vec!["index.json"], "a temporary file was left behind");
    }

    #[test]
    fn the_temporary_file_is_in_the_destination_directory() {
        // Not `/tmp`: a rename across filesystems is a copy and a delete, which
        // is the non-atomic write this module exists to avoid.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("index.json");
        let watched = directory.path().to_path_buf();

        write_with(&path, move |file| {
            let alongside: Vec<_> = fs::read_dir(&watched)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(
                alongside.len(),
                1,
                "the temporary file should be here, and it is not: {alongside:?}"
            );
            file.write_all(b"{}")
        })
        .unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"{}");
    }

    #[test]
    fn it_makes_the_directory_if_it_has_to() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("var/lib/spm/index/sepia.json");

        write(&path, b"{}").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"{}");
    }

    #[test]
    fn a_write_that_cannot_happen_says_which_path() {
        let directory = tempfile::tempdir().unwrap();
        // A file where a directory would have to be.
        let blocker = directory.path().join("blocked");
        fs::write(&blocker, b"in the way").unwrap();

        let outcome = write(&blocker.join("index.json"), b"{}");

        match outcome {
            Err(Error::Io { path, .. }) => {
                assert!(path.starts_with(directory.path()), "{}", path.display());
            }
            other => panic!("expected an Io error naming the path, got {other:?}"),
        }
    }
}
