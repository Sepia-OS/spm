/*
  install.rs

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

//! `install`: plan, verify, unpack, record.
//!
//! The order matters and is the design's, not this module's to change. Per
//! package it is: download while hashing; check the archive against the digest
//! the index carries **before the archive is opened at all**; open it and check
//! `data.tar.gz` against the digest its own `metadata.json` carries; work out
//! every file it would write **without writing one**; refuse if any of them is
//! already spoken for; write the journal; extract; and only then turn the
//! journal into a record.
//!
//! Two of those steps are the ones worth being careful about, and they are
//! careful for different reasons.
//!
//! **The digests are checked at two different moments, against two different
//! things.** The first is of the package as published and is what makes opening
//! it safe at all — checking it afterwards would mean the archive had already
//! been trusted enough to parse. The second binds the metadata to the payload
//! it describes, and catches a package rebuilt around a different payload even
//! when the outer archive is perfectly well formed.
//!
//! **The journal can only ever mean "undo".** It is written before a single
//! file is, and it records nothing about how far the install got, so the only
//! safe reading of one left behind is to take back what it claims. That
//! asymmetry is what makes an interrupted install recoverable at all.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256 as Hasher};

use crate::conffile;
use crate::error::{Error, Result};
use crate::model::installed::{Reason, Record};
use crate::model::metadata::{Metadata, Sha256};
use crate::model::name::{PackageName, PackageRef, SourceName, Target};
use crate::model::version::Version;
use crate::net::download;
use crate::net::transport::Transport;
use crate::ops::create::{METADATA, PAYLOAD};
use crate::ops::resolve::{self, Needed, Selected};
use crate::store::db::Database;
use crate::store::{Store, cache, space};
use crate::{ui, unpack};

/// How much room an install wants, as a multiple of what it downloads.
///
/// A package is on the card twice while it installs — the archive in
/// `/var/cache/spm/` and the unpacked files under `/usr` — so the room it needs
/// is about twice what it fetches. On a 2 GiB card with Rust and Helix already
/// on it, that is not a hypothetical.
const ROOM_FOR: u64 = 2;

/// The largest a package's `metadata.json` may be.
///
/// It is a few hundred bytes of JSON and it is the one part of a package read
/// into memory, so there is a number here rather than trust in whoever built
/// the archive.
const METADATA_LIMIT: u64 = 1 << 20;

/// What installing one package amounts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Not on the device at all.
    New,
    /// On the device already, at another version or from another source.
    Replaces(Replaced),
}

/// The package a step would replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replaced {
    /// The version that is installed now.
    pub version: Version,
    /// The source it came from, which is not always this step's.
    pub source: SourceName,
    /// The files it put on the device, so that the ones the new version does
    /// not ship can be taken away rather than left behind owned by nobody.
    pub files: Vec<PathBuf>,
    /// The digests of its configuration files, as that version wrote them.
    ///
    /// What decides whether an upgrade may replace a configuration file: if
    /// what is on the card still hashes to this, nobody has touched it and the
    /// new default goes in; if it does not, the file is somebody's work and the
    /// new default is written beside it instead.
    pub config: BTreeMap<PathBuf, Sha256>,
}

/// One package an install would put on the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Which package, at which version, from which source.
    pub selected: Selected,
    /// Whether it is what was asked for or something that came in with it.
    pub reason: Reason,
    /// What it does to what is already there.
    pub change: Change,
}

impl Step {
    /// Whether this package came in because something else needed it.
    #[must_use]
    pub fn is_dependency(&self) -> bool {
        self.reason == Reason::Dependency
    }
}

/// What an install would do, worked out before it does any of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The packages to install, dependencies first.
    pub steps: Vec<Step>,
    /// How many bytes all of them come to.
    pub download: u64,
    /// Packages in the set the device already has at exactly this version,
    /// from exactly this source.
    ///
    /// In practice that is the package that was asked for: a *dependency*
    /// already satisfied never reaches here, because resolution drops one
    /// before it becomes part of the set. So this is what turns
    /// `spm install <something already installed>` into a sentence rather than
    /// a command that appears to do nothing.
    pub satisfied: Vec<Selected>,
}

impl Plan {
    /// Whether there is anything to do at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// What an install did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Installs that had not finished and have now been taken back.
    pub rolled_back: Vec<PackageName>,
    /// What was to be done.
    pub plan: Plan,
    /// The configuration files whose new default was written beside the one on
    /// the card, because somebody had edited it.
    ///
    /// The `.spmnew` paths themselves, so the caller can name them. Empty on a
    /// dry run, which writes nothing.
    pub diverted: Vec<PathBuf>,
    /// Whether it was done, as opposed to only described.
    pub changed: bool,
}

/// Work out what installing this package would mean, and change nothing.
///
/// # Errors
///
/// Whatever resolving the name and its dependencies gives, or [`Error::Io`] and
/// [`Error::Parse`] if the device's own state cannot be read.
pub fn plan(
    store: &Store,
    reference: &PackageRef,
    target: &Target,
    version: Option<&Version>,
) -> Result<Plan> {
    let root = resolve::select(store, reference, target, version)?;
    let roots = [Needed {
        selected: root,
        // Whatever it was here for before, it is asked for now.
        reason: Reason::Explicit,
    }];
    plan_for(store, resolve::with_dependencies(store, &roots, target)?)
}

/// What a set that has already been resolved amounts to.
///
/// The planning step, from a set rather than from a name. `upgrade` works out
/// its own set — every installed package with a newer version, minus the ones
/// whose dependencies cannot be satisfied — and then joins `install` here,
/// rather than there being a second path that fetches, checks and unpacks.
///
/// # Errors
///
/// [`Error::Io`] or [`Error::Parse`] if the device's own records cannot be read.
pub fn plan_for(store: &Store, needed: Vec<Needed>) -> Result<Plan> {
    let database = Database::new(store);

    let mut steps = Vec::new();
    let mut satisfied = Vec::new();
    let mut download = 0_u64;

    for one in needed {
        let installed = database.get(&one.selected.name)?;
        let change = match installed {
            // Already exactly this, from exactly here: left alone, and said so.
            Some(record)
                if record.metadata.version == one.selected.version.version
                    && record.source == one.selected.source =>
            {
                satisfied.push(one.selected);
                continue;
            }
            Some(record) => Change::Replaces(Replaced {
                version: record.metadata.version.clone(),
                source: record.source.clone(),
                files: record.files.clone(),
                config: record.config.clone(),
            }),
            None => Change::New,
        };

        download = download.saturating_add(one.selected.version.bytes);
        steps.push(Step {
            selected: one.selected,
            reason: one.reason,
            change,
        });
    }

    Ok(Plan {
        steps,
        download,
        satisfied,
    })
}

/// Install a package and everything it needs.
///
/// With `dry_run` the plan is worked out and returned and **nothing is
/// touched** — not a file, not a record, not the cache.
///
/// # Errors
///
/// Whatever planning gives, plus [`Error::NotEnoughSpace`] before anything is
/// fetched, [`Error::Verification`] if either digest does not match,
/// [`Error::UnsafeEntry`] if the package holds something extraction will not
/// write, [`Error::FileConflict`] or [`Error::FileUnowned`] if a file is
/// already spoken for, and [`Error::Io`] for the rest.
pub fn install(
    store: &Store,
    transport: &dyn Transport,
    reference: &PackageRef,
    target: &Target,
    version: Option<&Version>,
    dry_run: bool,
) -> Result<Outcome> {
    // Before anything else, as every command that writes must: an install that
    // did not finish is taken back, so that what is planned next is planned
    // against a device in a state somebody designed.
    let rolled_back = Database::new(store).recover()?;

    let plan = plan(store, reference, target, version)?;

    if dry_run || plan.is_empty() {
        return Ok(Outcome {
            rolled_back,
            plan,
            diverted: Vec::new(),
            changed: false,
        });
    }

    let diverted = carry_out(store, transport, &plan)?;

    Ok(Outcome {
        rolled_back,
        plan,
        diverted,
        changed: true,
    })
}

/// Do what a plan says: check the room, then fetch, check and unpack each of it.
///
/// The other half of what `upgrade` reuses. Everything that can be refused
/// without touching the device has been refused by the time this runs.
///
/// # Errors
///
/// [`Error::NotEnoughSpace`] before anything is fetched,
/// [`Error::Verification`] if either digest does not match,
/// [`Error::UnsafeEntry`] if the package holds something extraction will not
/// write, [`Error::FileConflict`] or [`Error::FileUnowned`] if a file is
/// already spoken for, and [`Error::Io`] for the rest.
pub fn carry_out(store: &Store, transport: &dyn Transport, plan: &Plan) -> Result<Vec<PathBuf>> {
    enough_room(store, plan.download)?;

    let mut diverted = Vec::new();
    for step in &plan.steps {
        diverted.extend(one(store, transport, step)?);
    }

    Ok(diverted)
}

/// Refuse before filling the root filesystem, rather than after.
///
/// # Errors
///
/// [`Error::NotEnoughSpace`], naming what is needed and what there is.
fn enough_room(store: &Store, download: u64) -> Result<()> {
    let root = store.root();
    room_for(download, space::available(root)?, root)
}

/// Whether that much may be downloaded with that much free, and the refusal.
///
/// Split from the reading rather than folded into it, because the reading is
/// the one part of this that cannot be pinned down: the free space of a real
/// filesystem moves under a test while it runs. The decision is what is worth
/// asserting, and it is worth asserting exactly.
///
/// # Errors
///
/// [`Error::NotEnoughSpace`], naming what is needed and what there is.
fn room_for(download: u64, free: u64, root: &Path) -> Result<()> {
    let needed = download.saturating_mul(ROOM_FOR);

    if needed > free {
        return Err(Error::NotEnoughSpace {
            path: root.to_path_buf(),
            needed: ui::size(needed),
            free: ui::size(free),
        });
    }

    Ok(())
}

/// Fetch, check, unpack and record one package.
///
/// Returns the configuration files whose new default had to be written beside
/// the one on the card, so that the caller can say so: a `.spmnew` nobody is
/// told about is a change nobody will ever look at.
fn one(store: &Store, transport: &dyn Transport, step: &Step) -> Result<Vec<PathBuf>> {
    let selected = &step.selected;
    let archive = cache::package_file(
        store,
        &selected.source,
        &selected.name,
        &selected.version.version,
        &selected.version.target,
    )?;

    let downloaded = download::to_file(transport, &selected.version.url, &archive)?;

    // The archive against the index's digest, **before it is opened at all**.
    // Checking it afterwards would mean it had already been trusted enough to
    // parse, which is the trust this check exists to withhold.
    if downloaded.sha256 != selected.version.sha256 {
        return Err(Error::Verification {
            what: format!("the package {}", name_of(&archive)),
            expected: selected.version.sha256.to_string(),
            found: downloaded.sha256.to_string(),
        });
    }

    // Now it may be opened. `data.tar.gz` against the digest in the
    // `metadata.json` packed beside it: that is what binds the metadata to the
    // payload it claims to describe.
    let inside = opened(&archive)?;
    let promised = inside.metadata.sha256.as_ref().ok_or_else(|| Error::Parse {
        path: archive.clone(),
        message: format!(
            "the {METADATA} packed inside it carries no digest for its {PAYLOAD}, so there is nothing to check the payload against"
        ),
    })?;
    if &inside.payload_sha256 != promised {
        return Err(Error::Verification {
            what: format!("{PAYLOAD} inside the package {}", name_of(&archive)),
            expected: promised.to_string(),
            found: inside.payload_sha256.to_string(),
        });
    }

    // Everything it would write, worked out without writing any of it, so that
    // a package breaking one of the extraction rules is refused before a file
    // has been created.
    let entries = with_payload(&archive, |payload| unpack::inspect(payload, &archive))?;
    let recorded: Vec<unpack::Entry> = entries
        .into_iter()
        .filter(unpack::Entry::is_recorded)
        .collect();

    // What the version being replaced, if any, recorded about its own
    // configuration. An install onto a card that has none of this package leaves
    // it empty, and then nothing below diverts anything.
    let previously = match &step.change {
        Change::Replaces(replaced) => replaced.config.clone(),
        Change::New => BTreeMap::new(),
    };

    // The configuration files somebody has edited since they were installed.
    // Those are not this program's to overwrite, so the new default is diverted
    // beside them; everything else is written where it says.
    let mut keep = BTreeSet::new();
    for entry in &recorded {
        if !entry.is_config_file() {
            continue;
        }
        if let Some(installed) = previously.get(&entry.path)
            && !conffile::is_untouched(&store.root().join(&entry.path), Some(installed))?
        {
            keep.insert(entry.path.clone());
        }
    }

    // The record lists what extraction will actually write - so a diverted
    // default is listed under its `.spmnew` name - **and** the file it was
    // diverted around, which this package still owns. Leaving the latter out
    // would hand the administrator's file to nobody: `remove` would never reach
    // it and a later install would refuse to overwrite it.
    let mut files: Vec<PathBuf> = Vec::with_capacity(recorded.len());
    for entry in &recorded {
        if keep.contains(&entry.path) {
            files.push(conffile::diverted(&entry.path));
            files.push(entry.path.clone());
        } else {
            files.push(entry.path.clone());
        }
    }

    unclaimed(store, step, &files)?;

    // The package's own metadata, as it arrived, which is what the record
    // format asks for.
    let mut record = Record {
        metadata: inside.metadata,
        source: selected.source.clone(),
        reason: step.reason,
        installed_at: now(),
        files,
        // Filled in below, once the files exist: the digest of a configuration
        // file is of what was written, and until the extraction has run there
        // is nothing to hash.
        config: BTreeMap::new(),
    };

    // The journal, then the files, then the record. In that order an install
    // that stops anywhere leaves something that says what to take back — and
    // the journal names what the extraction *would* write rather than what it
    // did, which is what makes it a list to undo rather than a tally to finish.
    let database = Database::new(store);
    database.begin(&record)?;
    with_payload(&archive, |payload| {
        unpack::extract(payload, &archive, store.root(), &keep)
    })?;

    // Now the configuration digests, because now there are files to take them
    // of. A file that was written gets the digest of what was written; a file
    // that was diverted around keeps the digest it already had, so that it
    // stays "edited" for every upgrade after this one. Recording the
    // administrator's own bytes instead would make the next upgrade believe
    // nobody had touched it and overwrite it, which is the single outcome this
    // whole mechanism exists to prevent.
    for entry in &recorded {
        if !entry.is_config_file() {
            continue;
        }
        let digest = if keep.contains(&entry.path) {
            previously.get(&entry.path).cloned()
        } else {
            conffile::digest_of(&store.root().join(&entry.path))?
        };
        if let Some(digest) = digest {
            record.config.insert(entry.path.clone(), digest);
        }
    }
    // The journal is rewritten with the digests in it before it is committed.
    // Safe to do between the extraction and the commit because the amendment
    // adds only `config`: the file list an undo walks is exactly what it was, so
    // a crash on either side of this leaves the same recoverable state.
    database.begin(&record)?;
    database.commit(&selected.name)?;

    // The old version's files that the new one does not ship. Only now that the
    // new record is in place: a crash before this leaves files that are merely
    // stale, and a crash after it would have left files that nothing remembers.
    if let Change::Replaces(replaced) = &step.change {
        superseded(store, &record, replaced)?;
    }

    Ok(keep.iter().map(|path| conffile::diverted(path)).collect())
}

/// What was found inside a package.
struct Opened {
    /// The digest of the `data.tar.gz` it holds.
    payload_sha256: Sha256,
    /// The `metadata.json` it holds.
    metadata: Metadata,
}

/// Read a package's payload digest and its metadata, in one pass.
///
/// The payload is hashed where it lies rather than written out first: a package
/// is 216 MiB, and staging it would put it on the card a third time on top of
/// the archive and the unpacked tree.
fn opened(archive: &Path) -> Result<Opened> {
    let file = File::open(archive).map_err(|source| Error::Io {
        path: archive.to_path_buf(),
        source,
    })?;
    let mut package = tar::Archive::new(flate2::read::GzDecoder::new(file));

    let mut payload_sha256 = None;
    let mut metadata = None;

    for member in package
        .entries()
        .map_err(|source| unreadable(archive, &source))?
    {
        let mut member = member.map_err(|source| unreadable(archive, &source))?;
        let path = member
            .path()
            .map_err(|source| unreadable(archive, &source))?
            .display()
            .to_string();

        if path == PAYLOAD {
            let mut hasher = Hashing(Hasher::new());
            io::copy(&mut member, &mut hasher).map_err(|source| Error::Io {
                path: archive.to_path_buf(),
                source,
            })?;
            let digest = hex(&hasher.0.finalize());
            payload_sha256 = Sha256::parse(&digest);
        } else if path == METADATA {
            let mut text = String::new();
            member
                .take(METADATA_LIMIT)
                .read_to_string(&mut text)
                .map_err(|source| Error::Io {
                    path: archive.to_path_buf(),
                    source,
                })?;
            metadata = Some(serde_json::from_str(&text).map_err(|error| Error::Parse {
                path: archive.to_path_buf(),
                message: format!("the {METADATA} inside it could not be read: {error}"),
            })?);
        }
    }

    match (payload_sha256, metadata) {
        (Some(payload_sha256), Some(metadata)) => Ok(Opened {
            payload_sha256,
            metadata,
        }),
        (payload, _) => Err(Error::Parse {
            path: archive.to_path_buf(),
            message: format!(
                "a package holds a {PAYLOAD} and a {METADATA}, and this one has no {}",
                if payload.is_none() { PAYLOAD } else { METADATA }
            ),
        }),
    }
}

/// A sink that only hashes, so the payload can be digested where it lies.
///
/// `io::copy` wants somewhere to write and the hasher is not one; this makes it
/// one without the bytes going anywhere. Bounded by `io::copy`'s own buffer,
/// which is what keeps a 216 MiB payload out of memory.
struct Hashing(Hasher);

impl std::fmt::Debug for Hashing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Hashing")
    }
}

impl io::Write for Hashing {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Hand the `data.tar.gz` inside a package over as something to read.
///
/// Rather than writing it out first, which would put the package on the card a
/// third time. The member is the first thing in the archive, so reaching it
/// costs a header.
fn with_payload<T, F>(archive: &Path, use_it: F) -> Result<T>
where
    F: FnOnce(&mut dyn Read) -> Result<T>,
{
    let file = File::open(archive).map_err(|source| Error::Io {
        path: archive.to_path_buf(),
        source,
    })?;
    let mut package = tar::Archive::new(flate2::read::GzDecoder::new(file));

    for member in package
        .entries()
        .map_err(|source| unreadable(archive, &source))?
    {
        let mut member = member.map_err(|source| unreadable(archive, &source))?;
        let is_payload = member
            .path()
            .map_err(|source| unreadable(archive, &source))?
            .display()
            .to_string()
            == PAYLOAD;
        if is_payload {
            return use_it(&mut member);
        }
    }

    Err(Error::Parse {
        path: archive.to_path_buf(),
        message: format!("it holds no {PAYLOAD}"),
    })
}

/// Refuse rather than overwrite.
///
/// Two refusals, because they are two different situations with two different
/// fixes:
///
/// - **Another package owns it.** Two packages disagreeing about one file is
///   something a person has to settle, not something to decide by whoever ran
///   last.
/// - **Nothing owns it.** It came from the system image or was put there by
///   hand, and adopting it would mean `remove` later deleting something `spm`
///   never installed.
///
/// A package's own files are neither: replacing them is what an upgrade is.
fn unclaimed(store: &Store, step: &Step, files: &[PathBuf]) -> Result<()> {
    let mine = &step.selected.name;
    let records = Database::new(store).all()?;

    for file in files {
        if let Some(owner) = records
            .iter()
            .find(|record| &record.metadata.name != mine && record.files.contains(file))
        {
            return Err(Error::FileConflict {
                path: file.clone(),
                owner: owner.metadata.name.as_str().to_owned(),
            });
        }

        // Owned by this package already: an upgrade replacing its own file.
        let ours =
            matches!(&step.change, Change::Replaces(replaced) if replaced.files.contains(file));
        if ours {
            continue;
        }

        let on_disk = store.root().join(file);
        if std::fs::symlink_metadata(&on_disk).is_ok() {
            return Err(Error::FileUnowned { path: on_disk });
        }
    }

    Ok(())
}

/// Take away what the version being replaced left behind.
///
/// Only the files the new version does not ship and no other record claims.
/// Without this an upgrade to a version that dropped a file would leave it on
/// the card owned by nobody, where `remove` would never reach it and a later
/// install would refuse to overwrite it.
fn superseded(store: &Store, record: &Record, replaced: &Replaced) -> Result<()> {
    let others = Database::new(store).all()?;

    for file in &replaced.files {
        if record.files.contains(file) {
            continue;
        }
        let claimed = others
            .iter()
            .any(|other| other.metadata.name != record.metadata.name && other.files.contains(file));
        if claimed {
            continue;
        }

        // A configuration file the administrator has edited is not the old
        // version's to take away, even though the new version stopped shipping
        // it. The same question `remove` and the rollback ask.
        if !conffile::may_delete(store.root(), file, &replaced.config)? {
            continue;
        }

        let path = store.root().join(file);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::Io { path, source }),
        }
    }

    Ok(())
}

/// When this happened, in seconds since the epoch.
///
/// A device that has not run `sepia-time` yet believes it is 1970, and that is
/// what gets recorded. A wrong timestamp on a record is worth having; refusing
/// to install until the clock is set would not be.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The file name of a package, for a message about it.
fn name_of(archive: &Path) -> String {
    archive
        .file_name()
        .unwrap_or(archive.as_os_str())
        .display()
        .to_string()
}

fn unreadable(archive: &Path, source: &io::Error) -> Error {
    Error::Parse {
        path: archive.to_path_buf(),
        message: format!("it is not a readable package: {source}"),
    }
}

/// Bytes as lower-case hexadecimal.
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        use std::fmt::Write as _;
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

    /// A megabyte, so the numbers below read as sizes rather than as counts.
    const MIB: u64 = 1 << 20;

    #[test]
    fn an_install_asks_for_twice_what_it_downloads() {
        // A package is on the card twice while it installs. Against fixed
        // numbers rather than a real filesystem: the free space of one moves
        // while a test runs, and a test whose answer depends on what another
        // test happened to write is not testing this.
        let root = Path::new("/");

        // Exactly twice fits.
        room_for(50 * MIB, 100 * MIB, root).unwrap();
        // A byte more does not.
        assert!(room_for(50 * MIB + 1, 100 * MIB, root).is_err());
        // And once is never enough on its own.
        assert!(room_for(100 * MIB, 100 * MIB, root).is_err());
    }

    #[test]
    fn the_refusal_says_what_is_needed_and_what_there_is() {
        let root = Path::new("/");
        let error = room_for(50 * MIB, 12 * MIB, root).unwrap_err();

        match &error {
            Error::NotEnoughSpace { path, .. } => assert_eq!(path, root),
            other => panic!("expected NotEnoughSpace, got {other:?}"),
        }

        let message = error.to_string();
        assert!(message.contains("100.0 MiB"), "what is needed: {message}");
        assert!(message.contains("12.0 MiB"), "what there is: {message}");
        assert!(message.contains("twice"), "{message}");
    }

    #[test]
    fn asking_for_nothing_needs_nothing() {
        // A set that is entirely already installed downloads nothing, and must
        // not be refused by a card with nothing free either.
        room_for(0, 0, Path::new("/")).unwrap();
    }

    #[test]
    fn a_package_is_named_by_its_file_name_in_a_message() {
        assert_eq!(
            name_of(Path::new(
                "/var/cache/spm/helix-25.07.1-aarch64-musl.tar.gz"
            )),
            "helix-25.07.1-aarch64-musl.tar.gz"
        );
    }
}
