/*
  conffile.rs

  Created on 2026-09-09 by Thomas Bonk <thomas@meandmymac.de>
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

//! Configuration files: the ones a package ships and an administrator edits.
//!
//! Everything under `usr/` belongs to the package that put it there, and `spm`
//! may replace or delete it without asking. `etc/` is the one place where that
//! is not true: a package ships a *default* there, and the whole point of the
//! file is that somebody may change it. So every rule the rest of the program
//! applies to a file - replace it on upgrade, delete it on remove - has to ask
//! one more question first, and this module is that question.
//!
//! **The answer is a digest, recorded at install time.** A record carries the
//! SHA-256 of each configuration file *as `spm` wrote it*. Later, comparing
//! that against what is on the card says whether anybody has touched it since,
//! which is the only thing that distinguishes a default nobody wanted from a
//! decision somebody made.
//!
//! Two consequences, and they are the whole of the policy:
//!
//! - **An untouched file is `spm`'s.** It is replaced on upgrade and deleted on
//!   remove, exactly like anything under `usr/`, because leaving it would only
//!   leave a stale default behind.
//! - **An edited file is the administrator's.** It is never overwritten and
//!   never deleted. On upgrade the new default is written beside it with
//!   [`SUFFIX`] appended, so the change is on the card to be looked at rather
//!   than lost; on remove it simply stays.
//!
//! **The recorded digest is of what was shipped, not of what is there now.**
//! Once a file has been diverted the record keeps the digest it already had, so
//! the file stays "edited" for every upgrade that follows. Recording the
//! administrator's own bytes instead would make the next upgrade think the file
//! was untouched and overwrite it, which is the one outcome this exists to
//! prevent.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256 as Hasher};

use crate::error::{Error, Result};
use crate::model::metadata::Sha256;

/// The top-level directory a package's configuration lives under.
pub const ETC: &str = "etc";

/// What is appended to a configuration file's name when it cannot be replaced.
///
/// Appended to the whole name rather than substituted for the extension:
/// `sshd.conf` becomes `sshd.conf.spmnew`, so the original name is still
/// legible and two files that differ only by extension cannot collide.
pub const SUFFIX: &str = ".spmnew";

/// How big a configuration file may be before its digest is not worth taking.
///
/// Nothing proportional to a package is held in memory anywhere else in this
/// program and nothing is here either - the file is streamed - but a bound
/// still says what this is for. Configuration is text somebody edits; a
/// hundred-megabyte one is a payload that has been put in the wrong place.
pub const LARGEST: u64 = 16 * 1024 * 1024;

/// Whether this path is a configuration file rather than a package's own.
///
/// A path under `etc/`, and only that. Everything else in a package is owned
/// outright by the package that shipped it.
#[must_use]
pub fn is_config(path: &Path) -> bool {
    matches!(path.components().next(), Some(Component::Normal(first)) if first == ETC)
}

/// Where the new default goes when the one on the card may not be replaced.
#[must_use]
pub fn diverted(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(SUFFIX);
    PathBuf::from(name)
}

/// The digest of the file at `path`, or `None` if there is nothing there.
///
/// Streamed rather than read, for the same reason every other digest in this
/// program is: a file's size is not this program's to assume.
///
/// # Errors
///
/// [`Error::Io`] if the file is there but cannot be read, or if it is larger
/// than [`LARGEST`] - which is not a configuration file having grown but a
/// package having shipped something else under `etc/`.
pub fn digest_of(path: &Path) -> Result<Option<Sha256>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source: error,
            });
        }
    };

    let size = file
        .metadata()
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if size > LARGEST {
        return Err(Error::Io {
            path: path.to_path_buf(),
            source: io::Error::other(format!(
                "a configuration file of {size} bytes is larger than the {LARGEST} this will read - whatever it is, it does not belong under {ETC}/"
            )),
        });
    }

    let mut reader = BufReader::new(file);
    let mut hasher = Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        // `read` cannot exceed the buffer, so the slice is in range.
        match buffer.get(..read) {
            Some(chunk) => hasher.update(chunk),
            None => break,
        }
    }

    Ok(Sha256::parse(&hex(&hasher.finalize())))
}

/// Whether what is on the card is still the bytes that were installed.
///
/// `None` for the recorded digest means the file is not a configuration file
/// this record knows about, and then it is not the administrator's - it is
/// treated as any other file the package owns.
///
/// # Errors
///
/// [`Error::Io`] if the file is there and cannot be read.
pub fn is_untouched(path: &Path, installed: Option<&Sha256>) -> Result<bool> {
    match installed {
        None => Ok(true),
        Some(installed) => Ok(digest_of(path)?.as_ref() == Some(installed)),
    }
}

/// Whether `spm` may delete this file on the package's behalf.
///
/// Asked at all three places a payload's files are deleted - `remove` taking a
/// package back, `upgrade` dropping what the new version no longer ships, and
/// the rollback of an install that did not finish - because the answer has to
/// be the same at all three. A configuration file somebody has edited survives
/// every one of them; everything else goes.
///
/// # Errors
///
/// [`Error::Io`] if the file is there and cannot be read.
pub fn may_delete(
    root: &Path,
    relative: &Path,
    digests: &BTreeMap<PathBuf, Sha256>,
) -> Result<bool> {
    // Only `etc/` is anybody else's. A file under `usr/` belongs to the package
    // that put it there whatever has happened to it since, and a record now
    // carries a digest for that too - so the question has to be asked of the
    // path rather than of whether a digest exists.
    if !is_config(relative) {
        return Ok(true);
    }
    match digests.get(relative) {
        None => Ok(true),
        Some(installed) => is_untouched(&root.join(relative), Some(installed)),
    }
}

/// A digest as the lower-case hexadecimal the rest of the program uses.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut text, byte| {
        // Writing to a String cannot fail, and the result is discarded rather
        // than unwrapped so that this stays free of the panicking helpers.
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything here is test code, and a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    use std::fs;

    #[test]
    fn a_path_under_etc_is_configuration_and_nothing_else_is() {
        assert!(is_config(Path::new("etc/spm/sources.json")));
        assert!(is_config(Path::new("etc/helix.conf")));
        assert!(!is_config(Path::new("usr/bin/hx")));
        assert!(!is_config(Path::new("usr/share/etc/x")));
        assert!(!is_config(Path::new("")));
    }

    #[test]
    fn the_suffix_is_appended_to_the_whole_name() {
        // Not `sshd.spmnew`: the extension is part of the name, and dropping it
        // would let `a.conf` and `a.ini` collide on one diverted file.
        assert_eq!(
            diverted(Path::new("etc/sshd.conf")),
            PathBuf::from("etc/sshd.conf.spmnew")
        );
        assert_eq!(
            diverted(Path::new("etc/hostname")),
            PathBuf::from("etc/hostname.spmnew")
        );
    }

    #[test]
    fn a_file_that_is_not_there_has_no_digest() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            digest_of(&directory.path().join("absent.conf"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_digest_is_of_the_bytes_on_the_card() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a.conf");
        fs::write(&path, b"one = 1\n").unwrap();

        let first = digest_of(&path).unwrap().unwrap();
        // The empty-file digest is the well-known one, so a hasher that was
        // never fed would be visible rather than merely different.
        assert_eq!(first.as_str().len(), Sha256::LENGTH);

        fs::write(&path, b"one = 2\n").unwrap();
        assert_ne!(digest_of(&path).unwrap().unwrap(), first);
    }

    #[test]
    fn an_empty_file_hashes_to_the_known_empty_digest() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty.conf");
        fs::write(&path, b"").unwrap();
        assert_eq!(
            digest_of(&path).unwrap().unwrap().as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn untouched_is_what_was_installed_and_edited_is_anything_else() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a.conf");
        fs::write(&path, b"shipped\n").unwrap();
        let shipped = digest_of(&path).unwrap().unwrap();

        assert!(is_untouched(&path, Some(&shipped)).unwrap());

        fs::write(&path, b"edited by hand\n").unwrap();
        assert!(!is_untouched(&path, Some(&shipped)).unwrap());

        // Deleted by hand is "edited" too: it is certainly not what was
        // installed, and writing a default back over somebody's deletion would
        // be as much of a surprise as overwriting their edit.
        fs::remove_file(&path).unwrap();
        assert!(!is_untouched(&path, Some(&shipped)).unwrap());
    }

    #[test]
    fn a_file_with_no_recorded_digest_is_not_the_administrators() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("a.conf");
        fs::write(&path, b"whatever\n").unwrap();
        assert!(is_untouched(&path, None).unwrap());
    }

    #[test]
    fn an_edited_configuration_file_survives_every_deletion_path() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("etc")).unwrap();
        let relative = PathBuf::from("etc/a.conf");
        fs::write(root.path().join(&relative), b"shipped\n").unwrap();
        let shipped = digest_of(&root.path().join(&relative)).unwrap().unwrap();

        let mut digests = BTreeMap::new();
        digests.insert(relative.clone(), shipped);

        // Untouched: it is still the package's to take away.
        assert!(may_delete(root.path(), &relative, &digests).unwrap());

        // Edited: it is the administrator's now, and nothing removes it.
        fs::write(root.path().join(&relative), b"mine\n").unwrap();
        assert!(!may_delete(root.path(), &relative, &digests).unwrap());

        // A file the record says nothing about is not configuration at all.
        assert!(may_delete(root.path(), Path::new("usr/bin/x"), &digests).unwrap());
    }

    #[test]
    fn a_changed_file_under_usr_is_still_the_packages_to_delete() {
        // The rule that changed when every file gained a digest: a record now
        // has one for `usr/bin/x` too, and a mismatch there means corruption
        // rather than somebody's work. It must not stop a removal.
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("usr/bin")).unwrap();
        let relative = PathBuf::from("usr/bin/x");
        fs::write(root.path().join(&relative), b"installed\n").unwrap();
        let installed = digest_of(&root.path().join(&relative)).unwrap().unwrap();

        let mut digests = BTreeMap::new();
        digests.insert(relative.clone(), installed);

        fs::write(root.path().join(&relative), b"something else\n").unwrap();
        assert!(
            may_delete(root.path(), &relative, &digests).unwrap(),
            "usr/ belongs to the package whatever has happened to it"
        );
    }

    #[test]
    fn something_too_large_to_be_configuration_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("payload.conf");
        let big = vec![0_u8; usize::try_from(LARGEST).unwrap() + 1];
        fs::write(&path, &big).unwrap();
        assert!(digest_of(&path).is_err());
    }
}
