/*
  unpack.rs

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

//! Extraction of a package payload, and the rules that make it safe.
//!
//! The most dangerous code in the program - it writes into `/` as root - so
//! it is one module with its own tests, and every rule it enforces is
//! listed in `docs/dev/DESIGN.md`:
//!
//! - **Relative, and inside the root.** No leading `/`, no `..`. An entry that
//!   escapes is refused rather than clamped: a package that meant to write
//!   outside the tree has not asked for something this can helpfully correct.
//! - **Under `usr/` or `etc/`.** `ops::create` enforces this when packing, and
//!   this enforces it again, because a package can reach a device without
//!   having passed through this `create`. `etc/` is where a package ships
//!   configuration; what happens to such a file once somebody edits it is
//!   `crate::conffile`'s business rather than this module's, which writes what
//!   it is told to write.
//! - **Regular files, directories and symlinks, and nothing else.** No devices,
//!   no FIFOs, no sockets, and in particular no hard links - a hard link to
//!   `/etc/shadow` is a way to hand out its contents.
//! - **A symlink's target is a path too**, and checked the same way, so a
//!   package cannot drop a link pointing at `/etc` and then write "through" it
//!   with a later entry.
//! - **Permissions come from the archive, ownership does not.** Everything is
//!   left owned by whoever is running, which on a device is root. The uid a
//!   package was built under is an accident of the build machine, and restoring
//!   it is the mistake that broke helix's CI: GNU tar as root recreated
//!   `runner:docker` out of the archive and git then refused the tree.
//! - **Nothing is followed.** An existing symlink at a destination - or at any
//!   directory on the way to one - is refused, never written through.
//!
//! There is one further rule the design does not state, and it comes from the
//! record format rather than from safety: **a path has to be valid UTF-8.** A
//! record is JSON, so a path that is not could not be written into one, and a
//! package whose files cannot be recorded is a package that could never be
//! removed again.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read};
use std::path::{Component, Path, PathBuf};

use std::collections::BTreeSet;

use crate::conffile;
use crate::error::{Error, Result};

/// What one entry in a payload is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// A directory, which is made but never recorded — directories go when
    /// they empty out rather than being owned by anybody.
    Directory {
        /// The permission bits, already reduced to what will be set.
        mode: u32,
    },
    /// A regular file, with the permissions the archive asks for.
    File {
        /// The permission bits, already reduced to what will be set.
        mode: u32,
    },
    /// A symbolic link, and where it points.
    Symlink {
        /// The link's target, as the archive spells it.
        target: PathBuf,
    },
}

/// One entry of a payload that broke none of the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Where it goes, relative to the device's root.
    pub path: PathBuf,
    /// What it is.
    pub kind: Kind,
}

impl Entry {
    /// Whether this entry is one the record has to list.
    ///
    /// Everything but a directory: a record's `files` is what `remove` walks
    /// backwards, and directories are removed when they empty rather than by
    /// being claimed.
    /// Whether this entry is a configuration file rather than the package's own.
    ///
    /// A regular file under `etc/`. A symlink there is not one: there is no
    /// content of its own to compare, and following it to decide would be
    /// asking about whatever it points at instead.
    #[must_use]
    pub fn is_config_file(&self) -> bool {
        matches!(self.kind, Kind::File { .. }) && conffile::is_config(&self.path)
    }

    #[must_use]
    pub fn is_recorded(&self) -> bool {
        !matches!(self.kind, Kind::Directory { .. })
    }
}

/// Read a payload and say what it would write, **writing nothing**.
///
/// Every rule is applied here, so a package that breaks one is refused before
/// a single file has been created. `install` uses this to build the file list
/// for the journal and to look for conflicts, and the extraction that follows
/// applies the same rules again over the same entries.
///
/// # Errors
///
/// [`Error::UnsafeEntry`] naming the entry and the rule it breaks,
/// [`Error::Io`] if the payload cannot be read, [`Error::Parse`] if it is not
/// a gzipped tar at all.
pub fn inspect(payload: impl Read, from: &Path) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for_each(payload, from, |entry, _contents| {
        entries.push(entry);
        Ok(())
    })?;
    Ok(entries)
}

/// Extract a payload under `root`, and say what was written.
///
/// The paths come back relative to `root` and in the order they were written,
/// which is the order a record keeps them in so that undoing an install is
/// walking the list backwards.
///
/// # Errors
///
/// As [`inspect`], plus [`Error::FileUnowned`] if something that is not a
/// directory is sitting where a directory has to go, and [`Error::Io`] if a
/// file cannot be written.
pub fn extract(
    payload: impl Read,
    from: &Path,
    root: &Path,
    keep: &BTreeSet<PathBuf>,
) -> Result<Vec<PathBuf>> {
    // On a device this is `/` and has been there since the card was written.
    // Making it is for the tests, which unpack into a directory that does not
    // exist yet, and costs nothing when it already does.
    fs::create_dir_all(root).map_err(|source| Error::Io {
        path: root.to_path_buf(),
        source,
    })?;

    let mut written = Vec::new();
    for_each(payload, from, |entry, contents| {
        // A configuration file the caller asked to keep is not written over;
        // the new default lands beside it instead, so the change is on the card
        // to be looked at rather than lost. Everything else is written as it
        // comes.
        let mut entry = entry;
        if keep.contains(&entry.path) {
            entry.path = conffile::diverted(&entry.path);
        }
        write_entry(root, &entry, contents)?;
        if entry.is_recorded() {
            written.push(entry.path);
        }
        Ok(())
    })?;
    Ok(written)
}

/// Walk a payload, handing every entry over once it has passed every rule.
///
/// The payload arrives as something to read rather than as a path because
/// `install` streams it straight out of the package it is inside: writing it
/// out first would put the package on the card a third time, and a card with
/// Rust and Helix already on it does not have the room. `from` is the file
/// that stream came from, so that a failure names something a person can look
/// at.
///
/// The contents of each entry come the same way, for the same reason: a package
/// is 216 MiB and the smallest supported board has 512 MiB of RAM.
fn for_each<F>(payload: impl Read, from: &Path, mut each: F) -> Result<()>
where
    F: FnMut(Entry, &mut dyn Read) -> Result<()>,
{
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(payload));

    let members = archive.entries().map_err(|source| Error::Parse {
        path: from.to_path_buf(),
        message: format!("its payload is not a readable tar archive: {source}"),
    })?;

    for member in members {
        let mut member = member.map_err(|source| Error::Parse {
            path: from.to_path_buf(),
            message: format!("an entry in its payload could not be read: {source}"),
        })?;
        let entry = checked(&member)?;
        each(entry, &mut member)?;
    }

    Ok(())
}

/// Turn one archive member into an [`Entry`], or refuse it.
fn checked<R: Read>(member: &tar::Entry<'_, R>) -> Result<Entry> {
    let header = member.header();
    let raw = member.path().map_err(|source| Error::UnsafeEntry {
        path: PathBuf::from("(a name that could not be read)"),
        reason: format!("its name could not be read out of the archive: {source}"),
    })?;
    let path = safe_path(&raw)?;

    let kind = match header.entry_type() {
        // Forced readable, writable and traversable by its owner, whatever the
        // archive says: a directory nothing may enter is a directory the rest
        // of this package cannot be unpacked into.
        tar::EntryType::Directory => Kind::Directory {
            mode: file_mode(header, &path)? | 0o700,
        },
        tar::EntryType::Regular => Kind::File {
            mode: file_mode(header, &path)?,
        },
        tar::EntryType::Symlink => {
            let target = header
                .link_name()
                .map_err(|source| Error::UnsafeEntry {
                    path: path.clone(),
                    reason: format!("its target could not be read: {source}"),
                })?
                .ok_or_else(|| Error::UnsafeEntry {
                    path: path.clone(),
                    reason: "it is a symbolic link with no target".to_owned(),
                })?
                .into_owned();
            safe_target(&path, &target)?;
            Kind::Symlink { target }
        }
        // Everything else, named so the refusal says what was in the package.
        other => {
            return Err(Error::UnsafeEntry {
                path,
                reason: format!(
                    "it is {}, and a package holds regular files, directories and symbolic links and nothing else - a hard link to /etc/shadow is a way of handing out its contents, and a device node is a way of reaching a disk",
                    describe(other)
                ),
            });
        }
    };

    Ok(Entry { path, kind })
}

/// Check a path from an archive, and return it.
fn safe_path(path: &Path) -> Result<PathBuf> {
    // A record is JSON, so a path that is not valid UTF-8 could not be written
    // into one - and a package whose files cannot be recorded is one that could
    // never be removed again.
    if path.to_str().is_none() {
        return Err(Error::UnsafeEntry {
            path: path.to_path_buf(),
            reason: "its name is not valid UTF-8, and a record of what a package installed is JSON - a file that cannot be recorded is one that could never be removed again".to_owned(),
        });
    }

    for part in path.components() {
        match part {
            Component::Normal(_) => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::UnsafeEntry {
                    path: path.to_path_buf(),
                    reason: "it is an absolute path, and a package says where it goes relative to the root of the device rather than choosing one".to_owned(),
                });
            }
            Component::ParentDir => {
                return Err(Error::UnsafeEntry {
                    path: path.to_path_buf(),
                    reason: "it walks up out of the directory it is unpacked into with '..'"
                        .to_owned(),
                });
            }
            Component::CurDir => {
                return Err(Error::UnsafeEntry {
                    path: path.to_path_buf(),
                    reason:
                        "it contains a '.' component, and a package's paths are written plainly"
                            .to_owned(),
                });
            }
        }
    }

    if !starts_at_a_writable_top(path) {
        return Err(Error::UnsafeEntry {
            path: path.to_path_buf(),
            reason: "a package writes under usr/ or etc/ and nowhere else - anything outside them belongs to the system image, and a package that writes there is altering the system rather than adding to it".to_owned(),
        });
    }

    Ok(path.to_path_buf())
}

/// Whether a checked path begins with a directory a package may write in.
///
/// Two of them: `usr/`, which the package owns outright, and `etc/`, where it
/// ships defaults an administrator may then edit. Nothing else.
fn starts_at_a_writable_top(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Normal(first)) if first == "usr" || first == conffile::ETC
    )
}

/// Check where a symlink points.
///
/// A link is a path the package writes, so its target is checked the same way
/// one is: it has to be relative, and once resolved against the directory the
/// link sits in it has to land under `usr/` or `etc/` like everything else.
/// That is what stops a package shipping `usr/lib/x -> /var` and then writing
/// `usr/lib/x/spool` in the next entry.
fn safe_target(link: &Path, target: &Path) -> Result<()> {
    let refuse = |reason: &str| Error::UnsafeEntry {
        path: link.to_path_buf(),
        reason: format!(
            "it is a symbolic link pointing at {}, and {reason}",
            target.display()
        ),
    };

    let mut resolved: Vec<Component<'_>> = link
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect();

    for part in target.components() {
        match part {
            Component::Normal(_) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if resolved.pop().is_none() {
                    return Err(refuse("that walks up out of the root of the device"));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(refuse(
                    "a link in a package points somewhere inside the package rather than at an absolute path on the device it happens to land on",
                ));
            }
        }
    }

    let landed: PathBuf = resolved.iter().collect();
    if !starts_at_a_writable_top(&landed) {
        return Err(refuse(&format!(
            "that is {}, which is outside usr/ and etc/",
            if landed.as_os_str().is_empty() {
                PathBuf::from("the root of the device")
            } else {
                landed
            }
            .display()
        )));
    }

    Ok(())
}

/// The permissions to set on a file, from the archive.
///
/// The design says permissions come from the archive; the set-user-id,
/// set-group-id and sticky bits do not. `ops::create` normalises every mode to
/// 644 or 755 and so cannot produce one, which means a package carrying one did
/// not come from this tool, and quietly handing out a set-user-id root binary is
/// not something an extraction should do on its own.
fn file_mode(header: &tar::Header, path: &Path) -> Result<u32> {
    let mode = header.mode().map_err(|source| Error::UnsafeEntry {
        path: path.to_path_buf(),
        reason: format!("its permissions could not be read: {source}"),
    })?;
    Ok(mode & 0o777)
}

/// A name for an entry type, for the message that refuses it.
fn describe(kind: tar::EntryType) -> &'static str {
    match kind {
        tar::EntryType::Link => "a hard link",
        tar::EntryType::Char => "a character device",
        tar::EntryType::Block => "a block device",
        tar::EntryType::Fifo => "a named pipe",
        tar::EntryType::Continuous => "a contiguous file",
        _ => "not a kind of file a package may hold",
    }
}

/// Write one entry, following nothing on the way.
fn write_entry(root: &Path, entry: &Entry, contents: &mut dyn Read) -> Result<()> {
    let destination = root.join(&entry.path);

    match &entry.kind {
        Kind::Directory { mode } => {
            make_directory(root, &entry.path)?;
            set_directory_mode(&destination, *mode)?;
        }
        Kind::File { mode } => {
            make_directory(root, parent_of(&entry.path))?;
            clear(&destination)?;
            write_file(&destination, *mode, contents)?;
        }
        Kind::Symlink { target } => {
            make_directory(root, parent_of(&entry.path))?;
            clear(&destination)?;
            make_symlink(target, &destination)?;
        }
    }

    Ok(())
}

/// Write one file, with the permissions the archive asked for.
///
/// `create_new`, after [`clear`] has unlinked whatever was there: together they
/// mean the file that ends up at the destination is one this call made, and
/// never one an existing symbolic link redirected the write into.
///
/// Buffered, because a package is 11,000 files on an SD card and unbuffered
/// per-entry writes are the difference between seconds and minutes.
fn write_file(destination: &Path, mode: u32, contents: &mut dyn Read) -> Result<()> {
    let file = File::create_new(destination).map_err(|source| Error::Io {
        path: destination.to_path_buf(),
        source,
    })?;

    let mut sink = BufWriter::new(file);
    io::copy(contents, &mut sink).map_err(|source| Error::Io {
        path: destination.to_path_buf(),
        source,
    })?;
    let file = sink.into_inner().map_err(|error| Error::Io {
        path: destination.to_path_buf(),
        source: error.into_error(),
    })?;

    set_mode(&file, destination, mode)
}

/// Ownership is deliberately not touched; permissions are.
#[cfg(unix)]
fn set_mode(file: &File, destination: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|source| Error::Io {
            path: destination.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
fn set_mode(_file: &File, _destination: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// The same for a directory, which has no open handle to set it through.
#[cfg(unix)]
fn set_directory_mode(destination: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(destination, fs::Permissions::from_mode(mode)).map_err(|source| Error::Io {
        path: destination.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn set_directory_mode(_destination: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// The directory an entry sits in, relative to the root.
fn parent_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}

/// Make a directory and everything above it, refusing to follow a link.
///
/// Not `create_dir_all`: that would happily walk through a symlink standing in
/// for one of the directories, which is the "nothing is followed" rule, and it
/// is the one an attacker gets for free from a package installed earlier.
fn make_directory(root: &Path, relative: &Path) -> Result<()> {
    let mut at = root.to_path_buf();
    for part in relative.components() {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(existing) if existing.is_dir() => {}
            Ok(existing) if existing.is_symlink() => {
                return Err(Error::UnsafeEntry {
                    path: relative.to_path_buf(),
                    reason: format!(
                        "{} is a symbolic link, and writing through one would put this package's files wherever that link happens to point",
                        at.display()
                    ),
                });
            }
            // Something that is not a directory, in the way. Not this
            // package's to move: `spm` never removes what no record claims.
            Ok(_) => {
                return Err(Error::FileUnowned { path: at });
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&at).map_err(|source| Error::Io {
                    path: at.clone(),
                    source,
                })?;
            }
            Err(source) => {
                return Err(Error::Io {
                    path: at.clone(),
                    source,
                });
            }
        }
    }
    Ok(())
}

/// Unlink whatever is at a destination, without following it.
///
/// `remove_file` on a symbolic link removes the link and not what it points at,
/// which is exactly what is wanted: an upgrade replaces its own files, and a
/// link left by anybody must not become a way of redirecting the write.
fn clear(destination: &Path) -> Result<()> {
    match fs::remove_file(destination) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: destination.to_path_buf(),
            source,
        }),
    }
}

#[cfg(unix)]
fn make_symlink(target: &Path, destination: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, destination).map_err(|source| Error::Io {
        path: destination.to_path_buf(),
        source,
    })
}

#[cfg(not(unix))]
fn make_symlink(_target: &Path, destination: &Path) -> Result<()> {
    Err(Error::Io {
        path: destination.to_path_buf(),
        source: io::Error::other("this platform has no symbolic links"),
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
    use flate2::Compression;
    use flate2::write::GzEncoder;

    /// One thing to put in a hand-built payload.
    struct Member {
        path: PathBuf,
        kind: tar::EntryType,
        contents: Vec<u8>,
        link: Option<PathBuf>,
        mode: u32,
        uid: u64,
        /// Write the name straight into the header rather than through
        /// `append_data`, which refuses `..`, an absolute path and a `.`
        /// component — the very entries the rules below exist to refuse.
        raw: bool,
    }

    fn member(path: impl Into<PathBuf>, kind: tar::EntryType) -> Member {
        Member {
            path: path.into(),
            kind,
            contents: Vec::new(),
            link: None,
            mode: 0o644,
            uid: 0,
            raw: false,
        }
    }

    /// A file at a path no well-formed archive would carry.
    fn hostile(path: &str) -> Member {
        Member {
            contents: b"not yours".to_vec(),
            raw: true,
            ..member(path, tar::EntryType::Regular)
        }
    }

    fn file(path: &str, contents: &[u8]) -> Member {
        Member {
            contents: contents.to_vec(),
            ..member(path, tar::EntryType::Regular)
        }
    }

    fn directory(path: &str) -> Member {
        Member {
            mode: 0o755,
            ..member(path, tar::EntryType::Directory)
        }
    }

    fn link(path: &str, target: &str) -> Member {
        Member {
            link: Some(PathBuf::from(target)),
            mode: 0o777,
            ..member(path, tar::EntryType::Symlink)
        }
    }

    /// Write a `data.tar.gz` holding exactly these members.
    ///
    /// By hand rather than through `ops::create`, because every rule below is
    /// about something `create` would never produce - which is the point of
    /// checking them again here.
    fn payload(into: &Path, members: Vec<Member>) -> PathBuf {
        let path = into.join("data.tar.gz");
        let file = File::create(&path).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = tar::Builder::new(encoder);

        for member in members {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(member.kind);
            header.set_mode(member.mode);
            header.set_uid(member.uid);
            header.set_gid(member.uid);
            header.set_mtime(0);
            header.set_size(member.contents.len() as u64);
            if let Some(target) = &member.link {
                header.set_link_name(target).unwrap();
                header.set_size(0);
            }

            if member.raw {
                let name = member.path.to_string_lossy();
                let slot = &mut header.as_old_mut().name;
                slot.fill(0);
                slot[..name.len()].copy_from_slice(name.as_bytes());
                header.set_cksum();
                builder.append(&header, member.contents.as_slice()).unwrap();
            } else {
                builder
                    .append_data(&mut header, &member.path, member.contents.as_slice())
                    .unwrap();
            }
        }

        builder.into_inner().unwrap().finish().unwrap();
        path
    }

    /// A payload shaped like a real package: directories, a file, an
    /// executable, and the `git -> grit` kind of symlink.
    fn good(into: &Path) -> PathBuf {
        payload(
            into,
            vec![
                directory("usr"),
                directory("usr/bin"),
                Member {
                    mode: 0o755,
                    ..file("usr/bin/grit", b"#!/bin/sh\necho grit\n")
                },
                link("usr/bin/git", "grit"),
                directory("usr/share"),
                directory("usr/share/licenses"),
                file("usr/share/licenses/grit/LICENSE", b"Apache-2.0\n"),
            ],
        )
    }

    /// Read a payload file through the streaming interface `install` uses.
    fn inspecting(archive: &Path) -> Result<Vec<Entry>> {
        inspect(File::open(archive).unwrap(), archive)
    }

    fn extracting_into(archive: &Path, root: &Path) -> Result<Vec<PathBuf>> {
        extract(
            File::open(archive).unwrap(),
            archive,
            root,
            &BTreeSet::new(),
        )
    }

    /// What refusing an entry said, or a failure naming what happened instead.
    fn refusal(outcome: Result<Vec<PathBuf>>) -> (PathBuf, String) {
        match outcome {
            Err(Error::UnsafeEntry { path, reason }) => (path, reason),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn extracting(members: Vec<Member>) -> (tempfile::TempDir, Result<Vec<PathBuf>>) {
        let directory = tempfile::tempdir().unwrap();
        let archive = payload(directory.path(), members);
        let root = directory.path().join("device");
        fs::create_dir_all(&root).unwrap();
        let outcome = extracting_into(&archive, &root);
        (directory, outcome)
    }

    #[test]
    fn a_payload_says_what_it_would_write_without_writing_it() {
        let directory = tempfile::tempdir().unwrap();
        let archive = good(directory.path());

        let entries = inspecting(&archive).unwrap();

        let recorded: Vec<String> = entries
            .iter()
            .filter(|entry| entry.is_recorded())
            .map(|entry| entry.path.display().to_string())
            .collect();
        assert_eq!(
            recorded,
            vec![
                "usr/bin/grit",
                "usr/bin/git",
                "usr/share/licenses/grit/LICENSE"
            ],
            "directories are not recorded; they go when they empty out"
        );
        // And nothing was created on the way to finding that out.
        assert!(!directory.path().join("usr").exists());
    }

    #[test]
    fn what_a_package_holds_is_what_lands_on_the_device() {
        let directory = tempfile::tempdir().unwrap();
        let archive = good(directory.path());
        let root = directory.path().join("device");

        let written = extracting_into(&archive, &root).unwrap();

        assert_eq!(written.len(), 3);
        assert_eq!(
            fs::read_to_string(root.join("usr/bin/grit")).unwrap(),
            "#!/bin/sh\necho grit\n"
        );
        assert!(root.join("usr/share/licenses/grit/LICENSE").exists());
        #[cfg(unix)]
        {
            let link = fs::symlink_metadata(root.join("usr/bin/git")).unwrap();
            assert!(link.is_symlink(), "a link was written as a copy");
            assert_eq!(
                fs::read_link(root.join("usr/bin/git")).unwrap(),
                Path::new("grit")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn permissions_come_from_the_archive_and_set_user_id_does_not() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let archive = payload(
            directory.path(),
            vec![
                Member {
                    mode: 0o755,
                    ..file("usr/bin/grit", b"binary")
                },
                Member {
                    mode: 0o644,
                    ..file("usr/share/doc/grit", b"text")
                },
                // A package built by `ops::create` cannot carry one of these,
                // so one that does did not come from this tool.
                Member {
                    mode: 0o4755,
                    ..file("usr/bin/sneaky", b"binary")
                },
            ],
        );
        let root = directory.path().join("device");

        extracting_into(&archive, &root).unwrap();

        let mode = |at: &str| fs::metadata(root.join(at)).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode("usr/bin/grit"), 0o755);
        assert_eq!(mode("usr/share/doc/grit"), 0o644);
        assert_eq!(mode("usr/bin/sneaky"), 0o755, "set-user-id survived");
    }

    #[cfg(unix)]
    #[test]
    fn ownership_is_not_restored_from_the_archive() {
        // The mistake that broke helix's CI: GNU tar as root recreated
        // `runner:docker` out of the archive and git then refused the tree.
        // On a device this call runs as root and everything lands root-owned;
        // here it runs as whoever runs the tests, and the claim is the same
        // one either way - the archive did not get a say.
        use std::os::unix::fs::MetadataExt;
        let directory = tempfile::tempdir().unwrap();

        // Whoever is running, learnt from a file they wrote themselves rather
        // than assumed. The uid the archive claims is then one step away from
        // it: a fixed number would eventually *be* the current user, and a test
        // that passes because the archive named whoever ran it asserts nothing.
        // GitHub's runner is uid 1001, which is how this was found out.
        let reference = directory.path().join("ours");
        fs::write(&reference, b"ours").unwrap();
        let ours = fs::metadata(&reference).unwrap().uid();
        let claimed = u64::from(ours).saturating_add(1);

        let archive = payload(
            directory.path(),
            vec![Member {
                uid: claimed,
                ..file("usr/bin/grit", b"binary")
            }],
        );
        let root = directory.path().join("device");

        extracting_into(&archive, &root).unwrap();

        let unpacked = fs::metadata(root.join("usr/bin/grit")).unwrap().uid();
        assert_eq!(
            unpacked, ours,
            "the archive claimed uid {claimed} and got it"
        );
        assert_ne!(u64::from(unpacked), claimed);
    }

    #[test]
    fn an_entry_that_walks_up_out_of_the_tree_is_refused() {
        let (_directory, outcome) = extracting(vec![hostile("../../etc/passwd")]);
        let (path, reason) = refusal(outcome);
        assert_eq!(path, PathBuf::from("../../etc/passwd"));
        assert!(reason.contains(".."), "{reason}");
    }

    #[test]
    fn an_absolute_entry_is_refused() {
        let (_directory, outcome) = extracting(vec![hostile("/etc/passwd")]);
        let (_path, reason) = refusal(outcome);
        assert!(reason.contains("absolute"), "{reason}");
    }

    #[test]
    fn an_entry_outside_usr_and_etc_is_refused() {
        // `ops::create` refuses this when packing; a package can reach a device
        // without having passed through that `create`.
        let (_directory, outcome) = extracting(vec![file("var/lib/x/state", b"s")]);
        let (path, reason) = refusal(outcome);
        assert_eq!(path, PathBuf::from("var/lib/x/state"));
        assert!(reason.contains("usr/ or etc/"), "{reason}");
    }

    #[test]
    fn an_entry_under_etc_is_written() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let archive = payload(
            temp.path(),
            vec![directory("etc"), file("etc/helix.conf", b"theme\n")],
        );

        let written = extract(
            File::open(&archive).unwrap(),
            &archive,
            &root,
            &BTreeSet::new(),
        )
        .expect("a package may ship configuration under etc/");
        assert_eq!(written, vec![PathBuf::from("etc/helix.conf")]);
        assert_eq!(fs::read(root.join("etc/helix.conf")).unwrap(), b"theme\n");
    }

    #[test]
    fn a_kept_configuration_file_is_diverted_beside_the_one_on_the_card() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("etc")).unwrap();
        fs::write(root.join("etc/helix.conf"), b"mine\n").unwrap();

        let archive = payload(
            temp.path(),
            vec![directory("etc"), file("etc/helix.conf", b"theirs\n")],
        );

        let mut keep = BTreeSet::new();
        keep.insert(PathBuf::from("etc/helix.conf"));
        let written = extract(File::open(&archive).unwrap(), &archive, &root, &keep).unwrap();

        // The edit is untouched and the new default is beside it, under the
        // whole name plus the suffix.
        assert_eq!(fs::read(root.join("etc/helix.conf")).unwrap(), b"mine\n");
        assert_eq!(
            fs::read(root.join("etc/helix.conf.spmnew")).unwrap(),
            b"theirs\n"
        );
        assert_eq!(written, vec![PathBuf::from("etc/helix.conf.spmnew")]);
    }

    #[test]
    fn a_dot_component_is_refused() {
        let (_directory, outcome) = extracting(vec![hostile("./usr/bin/grit")]);
        let (_path, reason) = refusal(outcome);
        assert!(reason.contains("'.'"), "{reason}");
    }

    #[test]
    fn a_hard_link_is_refused() {
        // A hard link to /etc/shadow is a way of handing out its contents.
        let mut hard = member("usr/bin/shadow", tar::EntryType::Link);
        hard.link = Some(PathBuf::from("/etc/shadow"));
        let (_directory, outcome) = extracting(vec![hard]);
        let (path, reason) = refusal(outcome);
        assert_eq!(path, PathBuf::from("usr/bin/shadow"));
        assert!(reason.contains("hard link"), "{reason}");
    }

    #[test]
    fn a_device_node_is_refused() {
        for (kind, expected) in [
            (tar::EntryType::Char, "character device"),
            (tar::EntryType::Block, "block device"),
            (tar::EntryType::Fifo, "named pipe"),
        ] {
            let (_directory, outcome) = extracting(vec![member("usr/lib/thing", kind)]);
            let (_path, reason) = refusal(outcome);
            assert!(reason.contains(expected), "{reason}");
        }
    }

    #[test]
    fn a_symlink_pointing_out_of_usr_is_refused() {
        // A link is a path the package writes, so where it points is checked
        // the same way. `usr/lib/x` sits in `usr/lib`, so it takes two steps up
        // to leave `usr/` and three to leave the device's root altogether.
        // `var/` rather than `etc/`, which a package may now write.
        for target in ["/var", "../../var", "../../../../var/lib/passwd"] {
            let (_directory, outcome) = extracting(vec![link("usr/lib/x", target)]);
            let (path, reason) = refusal(outcome);
            assert_eq!(path, PathBuf::from("usr/lib/x"), "{target}");
            assert!(reason.contains(target), "{reason}");
        }
    }

    #[test]
    fn a_symlink_staying_inside_usr_is_allowed() {
        let directory = tempfile::tempdir().unwrap();
        let archive = payload(
            directory.path(),
            vec![
                file("usr/lib/helix/hx", b"binary"),
                link("usr/bin/hx", "../lib/helix/hx"),
            ],
        );
        let root = directory.path().join("device");

        extracting_into(&archive, &root).unwrap();

        #[cfg(unix)]
        assert!(
            fs::symlink_metadata(root.join("usr/bin/hx"))
                .unwrap()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn nothing_is_written_through_a_link_that_is_already_there() {
        // The other half of the rule above: the package is well behaved, and
        // the device already carries a link somebody else left. Following it
        // would put this package's files wherever it points.
        let directory = tempfile::tempdir().unwrap();
        let archive = payload(directory.path(), vec![file("usr/lib/grit/x", b"mine")]);
        let root = directory.path().join("device");
        let elsewhere = directory.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::create_dir_all(root.join("usr")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("usr/lib")).unwrap();

        let (_path, reason) = refusal(extracting_into(&archive, &root));

        assert!(reason.contains("symbolic link"), "{reason}");
        assert!(
            !elsewhere.join("grit").exists(),
            "the write went through the link"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_at_a_destination_is_replaced_rather_than_written_through() {
        let directory = tempfile::tempdir().unwrap();
        let archive = payload(directory.path(), vec![file("usr/bin/grit", b"mine")]);
        let root = directory.path().join("device");
        let bait = directory.path().join("bait");
        fs::write(&bait, b"not mine").unwrap();
        fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::os::unix::fs::symlink(&bait, root.join("usr/bin/grit")).unwrap();

        extracting_into(&archive, &root).unwrap();

        assert_eq!(
            fs::read(&bait).unwrap(),
            b"not mine",
            "the bait was written"
        );
        assert_eq!(fs::read(root.join("usr/bin/grit")).unwrap(), b"mine");
    }

    #[cfg(unix)]
    #[test]
    fn a_name_that_is_not_utf8_is_refused() {
        // Not a safety rule but a record one: a record is JSON, so a file whose
        // name cannot be written into one could never be removed again.
        use std::os::unix::ffi::OsStrExt;
        let name = PathBuf::from(std::ffi::OsStr::from_bytes(b"usr/bin/gr\xffit"));
        let (_directory, outcome) = extracting(vec![Member {
            contents: b"binary".to_vec(),
            ..member(name, tar::EntryType::Regular)
        }]);
        let (_path, reason) = refusal(outcome);
        assert!(reason.contains("UTF-8"), "{reason}");
    }

    #[test]
    fn a_name_too_long_for_a_tar_header_still_arrives_whole() {
        // Over 100 bytes, so the archive carries it in a GNU long-name header.
        // Helix ships tree-sitter grammars nested far deeper than this, and a
        // reader that saw the extension header instead of the entry would
        // refuse every one of them.
        let deep = format!(
            "usr/lib/helix/runtime/grammars/{}/parser.so",
            "nested/".repeat(15)
        );
        assert!(deep.len() > 100);
        let directory = tempfile::tempdir().unwrap();
        let archive = payload(directory.path(), vec![file(&deep, b"grammar")]);

        let entries = inspecting(&archive).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from(&deep));
    }

    #[test]
    fn a_payload_that_is_not_an_archive_says_so() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data.tar.gz");
        fs::write(&path, b"not an archive").unwrap();

        match inspect(File::open(&path).unwrap(), &path) {
            Err(Error::Parse { path: named, .. }) => assert_eq!(named, path),
            other => panic!("expected a Parse error naming the payload, got {other:?}"),
        }
    }

    #[test]
    fn a_file_where_a_directory_has_to_go_is_not_moved_aside() {
        // `spm` never removes what no record claims, so it stops instead.
        let directory = tempfile::tempdir().unwrap();
        let archive = payload(directory.path(), vec![file("usr/lib/grit/x", b"mine")]);
        let root = directory.path().join("device");
        fs::create_dir_all(root.join("usr")).unwrap();
        fs::write(root.join("usr/lib"), b"put here by the image").unwrap();

        match extracting_into(&archive, &root) {
            Err(Error::FileUnowned { path }) => assert_eq!(path, root.join("usr/lib")),
            other => panic!("expected FileUnowned, got {other:?}"),
        }
        assert_eq!(
            fs::read(root.join("usr/lib")).unwrap(),
            b"put here by the image"
        );
    }
}
