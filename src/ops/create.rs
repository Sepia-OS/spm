/*
  create.rs

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

//! `create`: turn a staged tree into a package.
//!
//! The one command that does not run on a device. It packages what a build has
//! already staged; it builds nothing itself.
//!
//! This step is the payload: the tree becomes `data.tar.gz`, hashed as it is
//! written, because the digest has to go into the metadata that is packed
//! *inside the same archive* and cannot exist until the payload does.
//!
//! **The same tree gives the same archive, byte for byte.** That is not a
//! nicety: a package is identified by its digest, and a build that produced a
//! different one each time would make "did this change?" unanswerable. So the
//! entries are sorted, timestamps are fixed at zero, and everything is owned by
//! `root:root` — none of which is a property of the machine that packed it.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Component, Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256 as Hasher};

use crate::error::{Error, Result};
use crate::layout;
use crate::model::index::Index;
use crate::model::metadata::{Metadata, Sha256};
use crate::sign::{self, PrivateKey, PublicKey, Signature};
use crate::store::atomic;

/// What packing produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    /// The digest of the file that was written.
    pub sha256: Sha256,
    /// How big it is, for the download size a plan shows.
    pub bytes: u64,
}

/// What the payload is called inside a package.
pub const PAYLOAD: &str = "data.tar.gz";

/// What the metadata is called inside a package, and beside it.
pub const METADATA: &str = "metadata.json";

/// The file listing the package's own digest, beside it in the release.
pub const SUMS: &str = "SHA256SUMS";

/// Where a package's licence has to be, under its own name.
const LICENSES: &str = "usr/share/licenses";

/// Refuse a staged tree that could not be installed safely.
///
/// The three rules are `docs/dev/ARCHITECTURE.md`'s, and two of them are
/// checked here. The third — that the metadata names a package — is not
/// checked at all, because it cannot be broken: a [`Metadata`] without a name,
/// a version or a target does not parse, so one cannot be handed to this
/// function.
///
/// **There used to be a fourth**, refusing any tree containing a `libc.so*` or
/// an `ld-musl-*`, on the reasoning that both belonged to the image and a
/// second copy of either was a device that stopped booting. The reasoning was
/// right and the rule was in the wrong place: it made the libc unpackageable
/// rather than making a *second* libc unpackageable, and what actually prevents
/// the second one is `ops::install`, which refuses to write over a file another
/// record claims ([`Error::FileConflict`]) or a file no record claims at all
/// ([`Error::FileUnowned`]) — so a musl package cannot land on a card whose
/// image already carries one, and two of them cannot both install. See
/// [`crate::layout`].
///
/// # Errors
///
/// [`Error::NotPackageable`] naming the file that broke the rule, or where the
/// missing thing should have been. [`Error::Io`] if the tree cannot be read.
pub fn check_tree(root: &Path, metadata: &Metadata) -> Result<()> {
    let entries = collect(root)?;

    for entry in &entries {
        // Everything under one of the roots `layout` names. That module carries
        // the reasoning for each of them, and for each of the ones left out.
        if !layout::is_writable_root(&entry.relative) {
            return Err(Error::NotPackageable {
                path: entry.relative.clone(),
                reason: format!(
                    "everything in a package has to be under {} - the rest of the device is the system image's, the kernel's or somebody's own, and a package that writes there is altering the system rather than adding to it",
                    layout::listed()
                ),
            });
        }

        // A link is a path the package writes, so where it lands is checked
        // too. `unpack` asks this of every entry it extracts, and until it was
        // asked here as well `create` would happily pack a link that no device
        // would then install - a package built by this program that this
        // program refuses. musl's loader is how that came to light.
        if let Kind::Symlink(target) = &entry.kind {
            let landed = layout::resolve_link(&entry.relative, target);
            let allowed = landed.as_deref().is_some_and(layout::is_writable_root);
            if !allowed {
                return Err(Error::NotPackageable {
                    path: entry.relative.clone(),
                    reason: format!(
                        "it is a symbolic link pointing at {}, which lands {} - a link has to land under {} like anything else a package writes",
                        target.display(),
                        landed.map_or_else(
                            || "above the root of the device".to_owned(),
                            |landed| format!("at {}", landed.display())
                        ),
                        layout::listed()
                    ),
                });
            }
        }
    }

    // A licence, under the package's own name.
    let licenses = PathBuf::from(LICENSES).join(metadata.name.as_str());
    let has_licence = entries.iter().any(|entry| {
        entry.kind == Kind::File
            && entry.relative.starts_with(&licenses)
            && fs::metadata(root.join(&entry.relative)).is_ok_and(|file| file.len() > 0)
    });
    if !has_licence {
        return Err(Error::NotPackageable {
            path: licenses,
            reason: "a package carries somebody else's work, and shipping it without its licence is not something this tool should make easy".to_owned(),
        });
    }

    Ok(())
}

/// Pack `root` into `into` as a gzipped tar, hashing it on the way past.
///
/// The digest is of the compressed file — `data.tar.gz` as it will sit inside
/// the package — because that is what `metadata.json` carries and what
/// `install` checks before unpacking.
///
/// # Errors
///
/// [`Error::Io`] if the tree cannot be read or the archive cannot be written.
pub fn pack_payload(root: &Path, into: &Path) -> Result<Payload> {
    let entries = collect(root)?;

    let file = File::create(into).map_err(|source| Error::Io {
        path: into.to_path_buf(),
        source,
    })?;

    // The digest is of the compressed bytes, so the hasher sits between the
    // encoder and the file: tar -> gzip -> hash -> disk.
    let counting = Counting::new(Hashing::new(BufWriter::new(file)));
    // A fixed compression level, like everything else here: the same tree has
    // to give the same bytes.
    let encoder = GzEncoder::new(counting, Compression::default());
    let mut builder = tar::Builder::new(encoder);

    for entry in &entries {
        append(&mut builder, root, entry).map_err(|source| Error::Io {
            path: root.join(&entry.relative),
            source,
        })?;
    }

    let counting = builder
        .into_inner()
        .and_then(GzEncoder::finish)
        .map_err(|source| Error::Io {
            path: into.to_path_buf(),
            source,
        })?;

    let bytes = counting.written;
    let Hashing { inner, hasher } = counting.inner;
    // The hasher has seen every byte on its way to the buffer; this is what
    // gets those bytes from the buffer to the disk.
    inner.into_inner().map_err(|error| Error::Io {
        path: into.to_path_buf(),
        source: error.into_error(),
    })?;

    let digest = hex(&hasher.finalize());
    let sha256 = Sha256::parse(&digest).ok_or_else(|| Error::Parse {
        path: into.to_path_buf(),
        message: format!("the digest of the payload came out as '{digest}'"),
    })?;

    Ok(Payload { sha256, bytes })
}

/// One thing to pack.
#[derive(Debug)]
struct Entry {
    /// Where it goes in the archive, relative to the root.
    relative: PathBuf,
    /// Its type, since that decides what the header says.
    kind: Kind,
}

#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Directory,
    File,
    /// A link, and where it points. `grit` ships `git -> grit`, so this is not
    /// hypothetical.
    Symlink(PathBuf),
}

/// Everything under `root`, sorted, so the archive does not depend on the
/// order a directory happened to be read in.
fn collect(root: &Path) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    walk(root, Path::new(""), &mut entries)?;
    entries.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(entries)
}

fn walk(root: &Path, relative: &Path, into: &mut Vec<Entry>) -> Result<()> {
    let directory = root.join(relative);
    let listing = fs::read_dir(&directory).map_err(|source| Error::Io {
        path: directory.clone(),
        source,
    })?;

    for entry in listing {
        let entry = entry.map_err(|source| Error::Io {
            path: directory.clone(),
            source,
        })?;
        let path = entry.path();
        let here = relative.join(entry.file_name());

        // symlink_metadata: a link is packed as a link, not as a copy of what
        // it points at - which is what keeps `git -> grit` one binary with two
        // names rather than two binaries.
        let metadata = fs::symlink_metadata(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;

        if metadata.is_symlink() {
            let target = fs::read_link(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            into.push(Entry {
                relative: here,
                kind: Kind::Symlink(target),
            });
        } else if metadata.is_dir() {
            into.push(Entry {
                relative: here.clone(),
                kind: Kind::Directory,
            });
            walk(root, &here, into)?;
        } else {
            into.push(Entry {
                relative: here,
                kind: Kind::File,
            });
        }
    }

    Ok(())
}

/// Write one entry, with a header that says nothing about this machine.
fn append<W: Write>(builder: &mut tar::Builder<W>, root: &Path, entry: &Entry) -> io::Result<()> {
    let source = root.join(&entry.relative);
    let mut header = tar::Header::new_gnu();

    // Nothing about who packed it or when.
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_username("root")?;
    header.set_groupname("root")?;

    match &entry.kind {
        Kind::Directory => {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mode(0o755);
            header.set_size(0);
            builder.append_data(&mut header, &entry.relative, io::empty())
        }
        Kind::Symlink(target) => {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_mode(0o777);
            header.set_size(0);
            header.set_link_name(target)?;
            builder.append_data(&mut header, &entry.relative, io::empty())
        }
        Kind::File => {
            let metadata = fs::symlink_metadata(&source)?;
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(metadata.len());
            header.set_mode(mode_of(&metadata));
            let file = File::open(&source)?;
            builder.append_data(&mut header, &entry.relative, file)
        }
    }
}

/// The permissions to record: the executable bit, and nothing else.
///
/// A staged tree comes from a build, and a build's umask is a property of the
/// machine it ran on. Recording 755 or 644 keeps the one distinction that
/// matters on the device — can this be run — without carrying the packer's
/// umask into every card.
#[cfg(unix)]
fn mode_of(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o111 == 0 {
        0o644
    } else {
        0o755
    }
}

#[cfg(not(unix))]
fn mode_of(_metadata: &fs::Metadata) -> u32 {
    0o644
}

/// A writer that hashes everything on its way through.
#[derive(Debug)]
struct Hashing<W: Write> {
    inner: W,
    hasher: Hasher,
}

impl<W: Write> Hashing<W> {
    fn new(inner: W) -> Self {
        Hashing {
            inner,
            hasher: Hasher::new(),
        }
    }
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        // Only what was actually written, or the digest describes bytes the
        // file does not contain.
        self.hasher.update(&buffer[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A writer that counts, so the size comes from the same pass as the digest.
#[derive(Debug)]
struct Counting<W: Write> {
    inner: W,
    written: u64,
}

impl<W: Write> Counting<W> {
    fn new(inner: W) -> Self {
        Counting { inner, written: 0 }
    }
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Bytes as lower-case hexadecimal.
///
/// By hand rather than with a crate: it is three lines, and `Sha256::parse`
/// refuses upper case, so the one thing that could go wrong is checked.
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        use std::fmt::Write as _;
        // Writing to a String cannot fail.
        let _ = write!(text, "{byte:02x}");
    }
    text
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
    use std::io::Read;

    /// A staged tree shaped like a real package: a binary, a library under a
    /// few directories, a licence, and the `git -> grit` symlink that made
    /// links worth supporting.
    fn staged(root: &Path) {
        fs::create_dir_all(root.join("usr/bin")).unwrap();
        fs::create_dir_all(root.join("usr/lib/helix/runtime")).unwrap();
        fs::create_dir_all(root.join("usr/share/licenses/helix")).unwrap();
        fs::write(root.join("usr/bin/hx"), b"#!/bin/sh\necho hx\n").unwrap();
        fs::write(root.join("usr/lib/helix/runtime/rust.so"), b"grammar").unwrap();
        fs::write(root.join("usr/share/licenses/helix/LICENSE"), b"Apache-2.0").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            fs::set_permissions(root.join("usr/bin/hx"), fs::Permissions::from_mode(0o755))
                .unwrap();
            symlink("hx", root.join("usr/bin/helix")).unwrap();
        }
    }

    fn pack_into(tree: &Path, name: &str, directory: &Path) -> (PathBuf, Payload) {
        let archive = directory.join(name);
        let payload = pack_payload(tree, &archive).unwrap();
        (archive, payload)
    }

    /// Every entry in the archive, as (path, type, mode, uid, mtime, link).
    fn entries(archive: &Path) -> Vec<(String, tar::EntryType, u32, u64, u64, Option<String>)> {
        let file = File::open(archive).unwrap();
        let decoder = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(decoder);
        tar.entries()
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let header = entry.header();
                (
                    entry.path().unwrap().display().to_string(),
                    header.entry_type(),
                    header.mode().unwrap(),
                    header.uid().unwrap(),
                    header.mtime().unwrap(),
                    entry
                        .link_name()
                        .unwrap()
                        .map(|link| link.display().to_string()),
                )
            })
            .collect()
    }

    /// The metadata the good tree above is packaged with.
    fn metadata_for(name: &str) -> Metadata {
        use crate::model::name::{PackageName, Target};
        use crate::model::version::Version;
        Metadata {
            name: PackageName::parse(name).unwrap(),
            version: Version::parse("25.07.1").unwrap(),
            target: Target::parse("aarch64-musl").unwrap(),
            description: "The Helix editor.".to_owned(),
            dependencies: Vec::new(),
            sha256: None,
            public_key: None,
            signature: None,
        }
    }

    fn refusal(root: &Path, name: &str) -> (PathBuf, String) {
        match check_tree(root, &metadata_for(name)) {
            Err(Error::NotPackageable { path, reason }) => (path, reason),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// Write the author's metadata file beside a staged tree.
    fn author_metadata(directory: &Path, version: &str) -> PathBuf {
        let path = directory.join("metadata.json");
        fs::write(
            &path,
            format!(
                r#"{{
  "name": "helix",
  "version": "{version}",
  "target": "aarch64-musl",
  "description": "The Helix editor.",
  "dependencies": [],
  "sha256": ""
}}"#
            ),
        )
        .unwrap();
        path
    }

    /// Read one member out of a package.
    fn member(package: &Path, want: &str) -> Vec<u8> {
        let file = File::open(package).unwrap();
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in tar.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path().unwrap().display().to_string() == want {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                return bytes;
            }
        }
        panic!("{want} is not in the package");
    }

    fn built(directory: &Path) -> Created {
        let tree = directory.join("stage");
        staged(&tree);
        let metadata = author_metadata(directory, "25.07.1");
        let output = directory.join("dist");
        create(&tree, &metadata, &output, None).unwrap()
    }

    #[test]
    fn it_writes_the_three_files_a_release_publishes_and_nothing_else() {
        let directory = tempfile::tempdir().unwrap();
        let created = built(directory.path());

        assert!(created.package.exists());
        assert!(created.metadata.exists());
        assert!(created.sums.exists());
        assert_eq!(
            created.package.file_name().unwrap(),
            "helix-25.07.1-aarch64-musl.tar.gz"
        );

        let mut left: Vec<String> = fs::read_dir(directory.path().join("dist"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().display().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "SHA256SUMS".to_owned(),
                "helix-25.07.1-aarch64-musl.tar.gz".to_owned(),
                "metadata.json".to_owned(),
            ],
            "something was left behind"
        );
    }

    #[test]
    fn the_package_holds_exactly_the_payload_and_the_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let created = built(directory.path());

        let names: Vec<String> = entries(&created.package)
            .into_iter()
            .map(|entry| entry.0)
            .collect();
        assert_eq!(names, vec![PAYLOAD.to_owned(), METADATA.to_owned()]);
    }

    #[test]
    fn the_metadata_inside_describes_the_payload_inside() {
        // The chain install depends on: the digest in the metadata is of the
        // data.tar.gz sitting beside it in the same archive.
        let directory = tempfile::tempdir().unwrap();
        let created = built(directory.path());

        let payload = member(&created.package, PAYLOAD);
        let mut hasher = Hasher::new();
        hasher.update(&payload);
        let really = hex(&hasher.finalize());

        let inside: Metadata = serde_json::from_slice(&member(&created.package, METADATA)).unwrap();
        assert_eq!(inside.sha256.unwrap().as_str(), really);
    }

    #[test]
    fn the_metadata_beside_the_package_is_the_metadata_inside_it() {
        let directory = tempfile::tempdir().unwrap();
        let created = built(directory.path());

        let beside = fs::read(&created.metadata).unwrap();
        assert_eq!(beside, member(&created.package, METADATA));
    }

    #[test]
    fn the_author_s_own_file_is_not_written_to() {
        // The digest goes into the copy that is packed, not into the file in
        // the package repository.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let metadata = author_metadata(directory.path(), "25.07.1");
        let before = fs::read(&metadata).unwrap();

        create(&tree, &metadata, &directory.path().join("dist"), None).unwrap();

        assert_eq!(fs::read(&metadata).unwrap(), before);
    }

    #[test]
    fn the_digest_file_is_what_sha256sum_reads() {
        let directory = tempfile::tempdir().unwrap();
        let created = built(directory.path());

        let sums = fs::read_to_string(&created.sums).unwrap();
        // digest, two spaces, name, newline - and nothing else.
        assert_eq!(
            sums,
            format!("{}  helix-25.07.1-aarch64-musl.tar.gz\n", created.sha256)
        );

        let mut hasher = Hasher::new();
        hasher.update(fs::read(&created.package).unwrap());
        assert_eq!(created.sha256.as_str(), hex(&hasher.finalize()));
        assert_eq!(created.bytes, fs::metadata(&created.package).unwrap().len());
    }

    #[test]
    fn the_same_tree_and_metadata_give_the_same_package() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let metadata = author_metadata(directory.path(), "25.07.1");

        let one = create(&tree, &metadata, &directory.path().join("a"), None).unwrap();
        let two = create(&tree, &metadata, &directory.path().join("b"), None).unwrap();

        assert_eq!(one.sha256, two.sha256);
        assert_eq!(
            fs::read(&one.package).unwrap(),
            fs::read(&two.package).unwrap()
        );
    }

    #[test]
    fn a_version_that_cannot_be_a_filename_is_refused() {
        // A name and a target are validated where they are made; a version is
        // whatever upstream chose, and this is where it becomes a filename.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let metadata = author_metadata(directory.path(), "../../evil");

        match create(&tree, &metadata, &directory.path().join("dist"), None) {
            Err(Error::Parse { path, message }) => {
                assert_eq!(path, metadata);
                assert!(message.contains("../../evil"), "{message}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_tree_that_breaks_a_rule_never_gets_packed() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        fs::remove_dir_all(tree.join("usr/share/licenses")).unwrap();
        let metadata = author_metadata(directory.path(), "25.07.1");
        let output = directory.path().join("dist");

        assert!(create(&tree, &metadata, &output, None).is_err());
        // And nothing was written on the way to finding out.
        assert!(!output.join("SHA256SUMS").exists());
    }

    #[test]
    fn a_metadata_file_that_is_not_there_says_so() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let missing = directory.path().join("nowhere.json");

        match create(&tree, &missing, &directory.path().join("dist"), None) {
            Err(Error::Io { path, .. }) => assert_eq!(path, missing),
            other => panic!("expected an Io error naming the file, got {other:?}"),
        }
    }

    #[test]
    fn a_staged_tree_that_is_a_package_passes() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        check_tree(&tree, &metadata_for("helix")).unwrap();
    }

    #[test]
    fn a_file_outside_the_writable_roots_is_refused() {
        // var/ is the one that matters most: /var/lib/spm is this program's own
        // database, and a package able to write there could forge a record.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        fs::create_dir_all(tree.join("var/lib/helix")).unwrap();
        fs::write(tree.join("var/lib/helix/state"), b"state").unwrap();

        let (path, reason) = refusal(&tree, "helix");
        assert_eq!(path, PathBuf::from("var"));
        assert!(reason.contains(&layout::listed()), "{reason}");
    }

    #[test]
    fn configuration_under_etc_is_allowed() {
        // Where a default somebody may then edit belongs - the one root whose
        // files this program does not own outright.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        fs::create_dir_all(tree.join("etc")).unwrap();
        fs::write(tree.join("etc/helix.conf"), b"theme = default\n").unwrap();

        let metadata = author_metadata(directory.path(), "25.07.1");
        let output = directory.path().join("dist");
        create(&tree, &metadata, &output, None)
            .expect("a package may ship configuration under etc/");
    }

    #[test]
    fn a_busybox_shaped_tree_packages() {
        // The tree that could not be expressed before: applets in /bin and
        // /sbin, symlinked into the one binary. busybox's applets declare where
        // they belong, and a card whose /bin/sh does not exist cannot run a
        // script - so packaging it as a usr/-only tree was never an option.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        fs::create_dir_all(tree.join("bin")).unwrap();
        fs::create_dir_all(tree.join("sbin")).unwrap();
        fs::create_dir_all(tree.join("usr/bin")).unwrap();
        fs::create_dir_all(tree.join("usr/share/licenses/busybox")).unwrap();
        fs::write(tree.join("bin/busybox"), b"\x7fELF").unwrap();
        fs::write(
            tree.join("usr/share/licenses/busybox/LICENSE"),
            b"GPL-2.0-only",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("busybox", tree.join("bin/sh")).unwrap();
            symlink("../bin/busybox", tree.join("sbin/init")).unwrap();
            symlink("../../bin/busybox", tree.join("usr/bin/awk")).unwrap();
        }

        check_tree(&tree, &metadata_for("busybox")).expect("busybox ships /bin/sh and /sbin/init");
    }

    #[test]
    fn a_libc_and_a_loader_now_package() {
        // The rule that used to refuse this was right about the danger and
        // wrong about where to stop it: it made the libc unpackageable rather
        // than a *second* libc unpackageable. `ops::install` is what refuses
        // the second one, by never writing over a file it does not own.
        //
        // The loader is staged exactly as musl's own `make install` leaves it -
        // a symlink to the *absolute* /usr/lib/libc.so - because that is the
        // path every binary on the card names in its PT_INTERP.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        fs::create_dir_all(tree.join("lib")).unwrap();
        fs::create_dir_all(tree.join("usr/lib")).unwrap();
        fs::create_dir_all(tree.join("usr/share/licenses/musl")).unwrap();
        fs::write(tree.join("usr/lib/libc.so"), b"\x7fELF").unwrap();
        fs::write(tree.join("usr/share/licenses/musl/COPYRIGHT"), b"MIT").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/usr/lib/libc.so", tree.join("lib/ld-musl-aarch64.so.1"))
            .unwrap();

        check_tree(&tree, &metadata_for("musl"))
            .expect("the loader's path is compiled into every binary on the card");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_landing_outside_the_roots_is_refused_when_packing_too() {
        // The half that was missing: `unpack` asked this of every entry and
        // `create` asked it of none, so `create` would build a package that no
        // device would install - one this program made and this program then
        // refused. Both spellings of the same escape, since accepting absolute
        // targets is what made the second one reachable.
        for target in ["/var/lib/spm", "../../var/lib/spm"] {
            let directory = tempfile::tempdir().unwrap();
            let tree = directory.path().join("stage");
            staged(&tree);
            std::os::unix::fs::symlink(target, tree.join("usr/bin/escape")).unwrap();

            let (path, reason) = refusal(&tree, "helix");
            assert_eq!(path, PathBuf::from("usr/bin/escape"), "{target}");
            assert!(reason.contains("var/lib/spm"), "{reason}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_walking_above_the_root_is_refused_when_packing() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        std::os::unix::fs::symlink("../../../../..", tree.join("usr/bin/up")).unwrap();

        let (_path, reason) = refusal(&tree, "helix");
        assert!(reason.contains("above the root of the device"), "{reason}");
    }

    #[test]
    fn a_package_without_a_licence_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        fs::remove_dir_all(tree.join("usr/share/licenses")).unwrap();

        let (path, reason) = refusal(&tree, "helix");
        assert_eq!(path, PathBuf::from("usr/share/licenses/helix"));
        assert!(reason.contains("licence"), "{reason}");
    }

    #[test]
    fn a_licence_under_another_package_s_name_does_not_count() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);

        // The tree carries a licence for helix; this is the grit package.
        let (path, _) = refusal(&tree, "grit");
        assert_eq!(path, PathBuf::from("usr/share/licenses/grit"));
    }

    #[test]
    fn an_empty_licence_file_does_not_count() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        fs::write(tree.join("usr/share/licenses/helix/LICENSE"), b"").unwrap();

        let (path, _) = refusal(&tree, "helix");
        assert_eq!(path, PathBuf::from("usr/share/licenses/helix"));
    }

    #[test]
    fn a_licence_by_any_name_counts() {
        // COPYING, LICENSE-MIT, NOTICE - the rule is that one is there.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        fs::remove_file(tree.join("usr/share/licenses/helix/LICENSE")).unwrap();
        fs::write(tree.join("usr/share/licenses/helix/COPYING"), b"Apache-2.0").unwrap();

        check_tree(&tree, &metadata_for("helix")).unwrap();
    }

    #[test]
    fn a_libc_or_a_loader_is_no_longer_refused_by_its_name() {
        // These three were refused outright until the libc became something a
        // package could be. Keeping the cases rather than deleting them: the
        // danger they named is real, and this records where it is now handled -
        // `ops::install::unclaimed`, which refuses to write over a file no
        // record claims, so a card whose image carries a libc cannot acquire a
        // second one however the package is named.
        for previously_refused in [
            "usr/lib/libc.so",
            "usr/lib/libc.so.6",
            "usr/lib/ld-musl-aarch64.so.1",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let tree = directory.path().join("stage");
            staged(&tree);
            fs::write(tree.join(previously_refused), b"a libc").unwrap();

            check_tree(&tree, &metadata_for("helix"))
                .unwrap_or_else(|error| panic!("{previously_refused}: {error}"));
        }
    }

    #[test]
    fn a_metadata_that_does_not_name_a_package_cannot_be_built_at_all() {
        // The fourth rule, which is not checked because it cannot be broken:
        // a metadata file without a name does not parse, so no Metadata
        // exists to hand to check_tree.
        let without_a_name = r#"{
            "version": "1.0.0",
            "target": "aarch64-musl",
            "description": "",
            "dependencies": [],
            "sha256": ""
        }"#;
        let outcome: std::result::Result<Metadata, _> = serde_json::from_str(without_a_name);
        assert!(outcome.is_err());
        assert!(outcome.unwrap_err().to_string().contains("name"));
    }

    #[test]
    fn the_same_tree_gives_the_same_archive() {
        // The reason all of this is fixed: a package is identified by its
        // digest, so packing twice has to give one answer.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);

        let (first, one) = pack_into(&tree, "one.tar.gz", directory.path());
        let (second, two) = pack_into(&tree, "two.tar.gz", directory.path());

        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
        assert_eq!(one, two);
    }

    #[test]
    fn when_the_files_were_touched_makes_no_difference() {
        // The stronger form of the same claim: two builds of one tree differ
        // in their timestamps, and must not differ in their packages.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let (first, _) = pack_into(&tree, "one.tar.gz", directory.path());

        let touched = File::options()
            .write(true)
            .open(tree.join("usr/bin/hx"))
            .unwrap();
        let later =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        touched
            .set_times(fs::FileTimes::new().set_modified(later))
            .unwrap();

        let (second, _) = pack_into(&tree, "two.tar.gz", directory.path());
        assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    }

    #[test]
    fn nothing_in_it_says_who_packed_it_or_when() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let (archive, _) = pack_into(&tree, "p.tar.gz", directory.path());

        for (path, _, _, uid, mtime, _) in entries(&archive) {
            assert_eq!(uid, 0, "{path} is owned by somebody");
            assert_eq!(mtime, 0, "{path} carries a timestamp");
        }
    }

    #[test]
    fn the_entries_are_sorted() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let (archive, _) = pack_into(&tree, "p.tar.gz", directory.path());

        let paths: Vec<String> = entries(&archive).into_iter().map(|e| e.0).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(
            paths, sorted,
            "a directory's read order leaked into the archive"
        );
    }

    #[test]
    fn what_went_in_comes_out() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let (archive, _) = pack_into(&tree, "p.tar.gz", directory.path());

        let file = File::open(&archive).unwrap();
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        let mut found = None;
        for entry in tar.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path().unwrap().display().to_string() == "usr/bin/hx" {
                let mut contents = String::new();
                entry.read_to_string(&mut contents).unwrap();
                found = Some(contents);
            }
        }
        assert_eq!(found.as_deref(), Some("#!/bin/sh\necho hx\n"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_packed_as_a_link_and_not_as_a_copy() {
        // `grit` ships `git -> grit`; following it here would put a second
        // copy of the binary on the card under a different name.
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let (archive, _) = pack_into(&tree, "p.tar.gz", directory.path());

        let link = entries(&archive)
            .into_iter()
            .find(|entry| entry.0 == "usr/bin/helix")
            .expect("the symlink should be in the archive");
        assert_eq!(link.1, tar::EntryType::Symlink);
        assert_eq!(link.5.as_deref(), Some("hx"));
    }

    #[cfg(unix)]
    #[test]
    fn the_executable_bit_survives_and_the_umask_does_not() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let (archive, _) = pack_into(&tree, "p.tar.gz", directory.path());

        let modes: Vec<(String, u32)> = entries(&archive)
            .into_iter()
            .filter(|entry| entry.1 == tar::EntryType::Regular)
            .map(|entry| (entry.0, entry.2))
            .collect();

        for (path, mode) in modes {
            let expected = if path == "usr/bin/hx" { 0o755 } else { 0o644 };
            assert_eq!(mode, expected, "{path} has mode {mode:o}");
        }
    }

    #[test]
    fn the_digest_describes_the_file_that_was_written() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        staged(&tree);
        let (archive, payload) = pack_into(&tree, "p.tar.gz", directory.path());

        let written = fs::read(&archive).unwrap();
        let mut hasher = Hasher::new();
        hasher.update(&written);
        assert_eq!(payload.sha256.as_str(), hex(&hasher.finalize()));
        assert_eq!(payload.bytes, written.len() as u64);
    }

    #[test]
    fn an_empty_tree_is_an_empty_archive_and_not_a_failure() {
        let directory = tempfile::tempdir().unwrap();
        let tree = directory.path().join("stage");
        fs::create_dir_all(&tree).unwrap();

        let (archive, payload) = pack_into(&tree, "p.tar.gz", directory.path());

        assert!(entries(&archive).is_empty());
        assert!(payload.bytes > 0, "even an empty archive has a header");
    }

    #[test]
    fn a_tree_that_is_not_there_says_so() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("nothing");
        match pack_payload(&missing, &directory.path().join("p.tar.gz")) {
            Err(Error::Io { path, .. }) => assert_eq!(path, missing),
            other => panic!("expected an Io error naming the tree, got {other:?}"),
        }
    }
}

/// The three files a release publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// The package itself.
    pub package: PathBuf,
    /// Its metadata, with the payload digest filled in, so that a source's
    /// scan can read it without downloading the package.
    pub metadata: PathBuf,
    /// The digest of the package, so the index has one for the archive itself.
    pub sums: PathBuf,
    /// That digest.
    pub sha256: Sha256,
    /// How big the package is.
    pub bytes: u64,
}

/// Turn a staged tree into a package, and write the three files a release
/// publishes.
///
/// # Errors
///
/// [`Error::Parse`] if the metadata cannot be read or names something that
/// cannot be a filename, [`Error::NotPackageable`] if the tree breaks one of
/// the rules, [`Error::Io`] if anything cannot be read or written.
pub fn create(
    root: &Path,
    metadata_file: &Path,
    output: &Path,
    signer: Option<&PrivateKey>,
) -> Result<Created> {
    let text = fs::read_to_string(metadata_file).map_err(|source| Error::Io {
        path: metadata_file.to_path_buf(),
        source,
    })?;
    let mut metadata: Metadata = serde_json::from_str(&text).map_err(|error| Error::Parse {
        path: metadata_file.to_path_buf(),
        message: error.to_string(),
    })?;

    check_tree(root, &metadata)?;

    let file_name = package_file_name(&metadata, metadata_file)?;

    fs::create_dir_all(output).map_err(|source| Error::Io {
        path: output.to_path_buf(),
        source,
    })?;

    // The payload first, because its digest has to go into the metadata that
    // is packed beside it - which is the whole reason this is two passes.
    let payload = tempfile::Builder::new()
        .prefix(".spm-payload-")
        .tempfile_in(output)
        .map_err(|source| Error::Io {
            path: output.to_path_buf(),
            source,
        })?;
    let packed = pack_payload(root, payload.path())?;
    metadata.sha256 = Some(packed.sha256.clone());

    // Signed here, between the payload's digest existing and the metadata being
    // packed: the signature covers the package's identity and that digest, so
    // it cannot be made before the payload and must be inside what is packed.
    // Both halves are written, the key as well as the signature, so that a
    // package says which key to check it with - and a device believes that only
    // when the index it already trusts names the same one.
    if let Some(signer) = signer {
        metadata.signature = Some(signer.sign_package(
            &metadata.name,
            &metadata.version,
            &metadata.target,
            &packed.sha256,
        ));
        metadata.public_key = Some(signer.public());
    }

    // The metadata that goes *inside* is the author's with the digest filled
    // in, not the file they wrote.
    let mut inner = serde_json::to_vec_pretty(&metadata).map_err(|error| Error::Parse {
        path: metadata_file.to_path_buf(),
        message: error.to_string(),
    })?;
    inner.push(b'\n');

    let package = output.join(&file_name);
    let sealed = seal(&package, payload.path(), &inner)?;

    // And beside it: the same metadata, and the package's own digest.
    let metadata_out = output.join(METADATA);
    atomic::write(&metadata_out, &inner)?;

    let sums_out = output.join(SUMS);
    // The format `sha256sum -c` reads: digest, two spaces, name.
    let sums = format!("{}  {}\n", sealed.sha256, file_name);
    atomic::write(&sums_out, sums.as_bytes())?;

    Ok(Created {
        package,
        metadata: metadata_out,
        sums: sums_out,
        sha256: sealed.sha256,
        bytes: sealed.bytes,
    })
}

/// Read a private key from the file `keygen` wrote.
///
/// # Errors
///
/// [`Error::Io`] if the file cannot be read, [`Error::Signing`] if what is in
/// it is not a key.
pub fn read_key(path: &Path) -> Result<PrivateKey> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    PrivateKey::parse(&text)
}

/// What signing a file produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedFile {
    /// The file that was signed.
    pub file: PathBuf,
    /// Where the signature was written.
    pub signature: PathBuf,
    /// The key it was signed with, which is what a reader checks it against.
    pub public_key: PublicKey,
}

/// Sign any file, writing the signature beside it as `<file>.sig`.
///
/// The counterpart to [`sign_index`], for everything that is not an index.
/// A source publishes more than its index - `sepiaos-package-index` publishes a
/// `source.json` saying what it is called and which key to pin - and until this
/// existed there was nothing to sign those with, because `sign_index` parses
/// its input and refuses anything that is not an index.
///
/// **The signature is under its own context**, so it can never be mistaken for
/// an index signature by anything that verifies. That is not a formality: a
/// shared context would let a source be made to publish an index it never
/// signed, by handing over a file it did.
///
/// Nothing is parsed here, because nothing is assumed about the file. The bytes
/// on disk are signed exactly as they are, for the same reason an index's are -
/// whoever checks it has the bytes they were given.
///
/// # Errors
///
/// [`Error::Io`] if the file or the key cannot be read, or the signature cannot
/// be written; [`Error::Signing`] if the key is not one.
pub fn sign_file(file: &Path, key: &Path) -> Result<SignedFile> {
    let signer = read_key(key)?;

    let bytes = fs::read(file).map_err(|source| Error::Io {
        path: file.to_path_buf(),
        source,
    })?;

    let signature = signer.sign_file(&bytes);
    let out = signature_path(file);
    atomic::write(&out, format!("{signature}\n").as_bytes())?;

    Ok(SignedFile {
        file: file.to_path_buf(),
        signature: out,
        public_key: signer.public(),
    })
}

/// Check a file against the signature written beside it.
///
/// Reads `<file>.sig`, so it checks what was published rather than what a
/// caller thought was published.
///
/// # Errors
///
/// [`Error::Io`] if the file or its signature cannot be read,
/// [`Error::Signing`] if the key given is not a public key or the signature
/// file does not hold one, and [`Error::BadSignature`] if it does not verify.
pub fn verify_file_signature(file: &Path, key: &str) -> Result<PublicKey> {
    let key = PublicKey::parse(key).ok_or_else(|| {
        Error::Signing(format!(
            "'{key}' is not a public key: {} lower-case hexadecimal characters",
            PublicKey::BYTES * 2
        ))
    })?;

    let bytes = fs::read(file).map_err(|source| Error::Io {
        path: file.to_path_buf(),
        source,
    })?;

    let signature_file = signature_path(file);
    let text = fs::read_to_string(&signature_file).map_err(|source| Error::Io {
        path: signature_file.clone(),
        source,
    })?;
    let signature = Signature::parse(text.trim()).ok_or_else(|| {
        Error::Signing(format!(
            "{} does not hold an Ed25519 signature",
            signature_file.display()
        ))
    })?;

    sign::verify_file(&key, &signature, &bytes, &file.display().to_string())?;
    Ok(key)
}

/// Where a signature goes: beside what it covers, with `.sig` appended.
///
/// Appended to the whole name rather than substituted for the extension, so
/// that the URL a device fetches is the index's URL with `.sig` on the end -
/// which is exactly how `ops::source::fetch_index` looks for it.
fn signature_path(file: &Path) -> PathBuf {
    let mut out = file.as_os_str().to_owned();
    out.push(".sig");
    PathBuf::from(out)
}

/// What signing an index produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIndex {
    /// The index that was signed.
    pub index: PathBuf,
    /// Where the signature was written.
    pub signature: PathBuf,
    /// The key it was signed with, which is what a device pins.
    pub public_key: PublicKey,
}

/// Sign an index file, writing the signature beside it.
///
/// The bytes on disk are signed exactly as they are - not a re-serialisation of
/// what they parse to - because that is what a device will check. It is parsed
/// first all the same, so that signing something that is not an index fails
/// here rather than on every device that fetches it.
///
/// # Errors
///
/// [`Error::Io`] if either file cannot be read or written, [`Error::Parse`] if
/// the index is not one, [`Error::Signing`] if the key is not a key.
pub fn sign_index(index: &Path, key: &Path) -> Result<SignedIndex> {
    let signer = read_key(key)?;

    let bytes = fs::read(index).map_err(|source| Error::Io {
        path: index.to_path_buf(),
        source,
    })?;

    // Parsed only to refuse the obvious mistake; the signature is over `bytes`.
    serde_json::from_slice::<Index>(&bytes).map_err(|error| Error::Parse {
        path: index.to_path_buf(),
        message: error.to_string(),
    })?;

    let signature = signer.sign_index(&bytes);
    let out = signature_path(index);
    atomic::write(&out, format!("{signature}\n").as_bytes())?;

    Ok(SignedIndex {
        index: index.to_path_buf(),
        signature: out,
        public_key: signer.public(),
    })
}

/// `<name>-<version>-<target>.tar.gz`, or a refusal.
///
/// `name` and `target` are validated where they are made and cannot escape a
/// directory. **A version is not**: it is whatever upstream chose, and this is
/// the one place that turns one into a filename. A version containing a path
/// separator would put the package somewhere other than the output directory,
/// so it is refused here rather than trusted.
fn package_file_name(metadata: &Metadata, metadata_file: &Path) -> Result<String> {
    let name = format!(
        "{}-{}-{}.tar.gz",
        metadata.name, metadata.version, metadata.target
    );

    let mut parts = Path::new(&name).components();
    let single = matches!(parts.next(), Some(Component::Normal(part)) if part == name.as_str());
    if !single || parts.next().is_some() {
        return Err(Error::Parse {
            path: metadata_file.to_path_buf(),
            message: format!(
                "the version '{}' cannot be part of a filename - a package is written as <name>-<version>-<target>.tar.gz, and this would not be one file in one directory",
                metadata.version
            ),
        });
    }

    Ok(name)
}

/// Write the package: the payload and the metadata, and nothing else.
fn seal(package: &Path, payload: &Path, metadata: &[u8]) -> Result<Payload> {
    let file = File::create(package).map_err(|source| Error::Io {
        path: package.to_path_buf(),
        source,
    })?;

    let counting = Counting::new(Hashing::new(BufWriter::new(file)));
    let encoder = GzEncoder::new(counting, Compression::default());
    let mut builder = tar::Builder::new(encoder);

    // Sorted, like everything else: data.tar.gz before metadata.json.
    append_member(&mut builder, PAYLOAD, payload, metadata_len(payload)?).map_err(|source| {
        Error::Io {
            path: payload.to_path_buf(),
            source,
        }
    })?;
    append_bytes(&mut builder, METADATA, metadata).map_err(|source| Error::Io {
        path: package.to_path_buf(),
        source,
    })?;

    let counting = builder
        .into_inner()
        .and_then(GzEncoder::finish)
        .map_err(|source| Error::Io {
            path: package.to_path_buf(),
            source,
        })?;

    let bytes = counting.written;
    let Hashing { inner, hasher } = counting.inner;
    inner.into_inner().map_err(|error| Error::Io {
        path: package.to_path_buf(),
        source: error.into_error(),
    })?;

    let digest = hex(&hasher.finalize());
    let sha256 = Sha256::parse(&digest).ok_or_else(|| Error::Parse {
        path: package.to_path_buf(),
        message: format!("the digest of the package came out as '{digest}'"),
    })?;

    Ok(Payload { sha256, bytes })
}

fn metadata_len(path: &Path) -> Result<u64> {
    fs::metadata(path)
        .map(|file| file.len())
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
}

/// One member of the package, read from a file.
fn append_member<W: Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    from: &Path,
    size: u64,
) -> io::Result<()> {
    let mut header = fixed_header(size);
    let file = File::open(from)?;
    builder.append_data(&mut header, name, file)
}

/// One member of the package, held in memory - the metadata, which is small.
fn append_bytes<W: Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    bytes: &[u8],
) -> io::Result<()> {
    let mut header = fixed_header(bytes.len() as u64);
    builder.append_data(&mut header, name, bytes)
}

/// A header that says nothing about this machine, as in the payload.
fn fixed_header(size: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_size(size);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    let _ = header.set_username("root");
    let _ = header.set_groupname("root");
    header
}
