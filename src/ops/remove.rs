/*
  remove.rs

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

//! `remove`: take back exactly what was installed, and what it pulled in.
//!
//! Nothing outside a package's own record is ever touched.
//!
//! This is the reverse of `install` and it is much the simpler of the two,
//! because the record already says what to do: it lists the files, in the order
//! they were written, so undoing an install is walking that list backwards.
//! There is nothing to fetch, nothing to verify and nothing to work out — every
//! decision was made when the package went on.
//!
//! Three rules decide what actually goes, and all three exist so that removing
//! one package cannot break the device around it:
//!
//! - **A file another record also claims stays.** It is not this package's to
//!   take away while something else is still using it.
//! - **A file no record claims is never touched at all.** It came from the
//!   system image or from somebody's hand, and `spm` does not remove what it
//!   did not install — the same rule that made `install` refuse to adopt it.
//! - **A package something else still needs is refused**, and the packages that
//!   need it are named, so that what would have to go first is on the screen
//!   rather than left to be worked out.
//!
//! Then the autoremove pass, which is the other half of `install` recording
//! *why* a package is on the device: anything that came in as a dependency and
//! that nothing remaining needs goes too. A package somebody asked for by name
//! never does, however unreferenced it looks.

use std::fs;
use std::path::PathBuf;

use crate::conffile;
use crate::error::{Error, Result};
use crate::model::installed::{Reason, Record};
use crate::model::name::{PackageName, PackageRef, SourceName};
use crate::model::version::Version;
use crate::store::Store;
use crate::store::db::{self, Database};

/// One package a removal would take off the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Going {
    /// What it is called.
    pub name: PackageName,
    /// The version that is installed.
    pub version: Version,
    /// The source it came from, which may no longer be configured.
    pub source: SourceName,
    /// Whether it is going because nothing needs it any more, rather than
    /// because it was asked for.
    pub unneeded: bool,
    /// The files that will actually be deleted.
    ///
    /// Not the record's whole list: a path some record that stays also claims
    /// is not one of them, and neither is a configuration file somebody has
    /// edited - those are in `kept`.
    pub files: Vec<PathBuf>,
    /// The configuration files that stay behind, and why they are not in
    /// `files`: somebody edited them, so they are the administrator's work
    /// rather than the package's, and removing the package does not remove it.
    pub kept: Vec<PathBuf>,
}

/// What a removal would do, worked out before it does any of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// The packages to remove: the one that was asked for first, then whatever
    /// is going with it.
    pub packages: Vec<Going>,
}

impl Removal {
    /// How many files it comes to.
    #[must_use]
    pub fn files(&self) -> usize {
        self.packages.iter().map(|going| going.files.len()).sum()
    }

    /// How many configuration files it will leave behind.
    #[must_use]
    pub fn kept(&self) -> usize {
        self.packages.iter().map(|going| going.kept.len()).sum()
    }
}

/// What a removal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Installs that had not finished and have now been taken back.
    pub rolled_back: Vec<PackageName>,
    /// What was to be done.
    pub removal: Removal,
    /// Whether it was done, as opposed to only described.
    pub changed: bool,
}

/// Work out what removing this package would take away, and change nothing.
///
/// # Errors
///
/// [`Error::NotInstalled`] if nothing on the device goes by that name,
/// [`Error::HasDependents`] if something that would stay still needs it, and
/// [`Error::Io`] or [`Error::Parse`] if the records cannot be read.
pub fn plan(store: &Store, reference: &PackageRef) -> Result<Removal> {
    let installed = Database::new(store).all()?;

    let named = find(&installed, reference)?;
    let going = with_unneeded(&installed, named);

    // Everything that is not going has to keep working, so anything among them
    // that needs the package being removed is a reason to stop. The packages
    // taken by the autoremove pass cannot be in this list: the pass only takes
    // one when nothing remaining needs it.
    let dependents: Vec<String> = installed
        .iter()
        .filter(|record| !going.contains(&record.metadata.name))
        .filter(|record| record.depends_on(&named.metadata.name))
        .map(|record| format!("{} {}", record.metadata.name, record.metadata.version))
        .collect();
    if !dependents.is_empty() {
        return Err(Error::HasDependents {
            package: named.metadata.name.as_str().to_owned(),
            dependents,
        });
    }

    // The package that was asked for first, then what goes with it, in the
    // order the pass found them.
    let mut packages = Vec::new();
    for name in &going {
        let Some(record) = installed
            .iter()
            .find(|record| &record.metadata.name == name)
        else {
            continue;
        };
        let keeping = unshared(store, &installed, &going, record)?;
        packages.push(Going {
            name: record.metadata.name.clone(),
            version: record.metadata.version.clone(),
            source: record.source.clone(),
            unneeded: record.metadata.name != named.metadata.name,
            files: keeping.0,
            kept: keeping.1,
        });
    }

    Ok(Removal { packages })
}

/// Remove a package and everything that came in with it and is now unneeded.
///
/// With `dry_run` the set is worked out and returned and **nothing is
/// touched**.
///
/// # Errors
///
/// As [`plan`], plus [`Error::Io`] if a file cannot be deleted or a record
/// cannot be forgotten.
pub fn remove(store: &Store, reference: &PackageRef, dry_run: bool) -> Result<Outcome> {
    // Before anything else, as every command that writes must.
    let rolled_back = Database::new(store).recover()?;

    let removal = plan(store, reference)?;

    if dry_run {
        return Ok(Outcome {
            rolled_back,
            removal,
            changed: false,
        });
    }

    let database = Database::new(store);
    for going in &removal.packages {
        take_back(store, going)?;
        database.forget(&going.name)?;
    }

    Ok(Outcome {
        rolled_back,
        removal,
        changed: true,
    })
}

/// The record for what somebody typed.
///
/// A qualified name is not a different package here — there is one record per
/// name, so a device has at most one `helix` — but it is a different question:
/// `sepia/helix` asks about the one that came from `sepia`, and if the one on
/// the device came from somewhere else then the thing asked about is not
/// installed.
fn find<'records>(
    installed: &'records [Record],
    reference: &PackageRef,
) -> Result<&'records Record> {
    installed
        .iter()
        .find(|record| {
            &record.metadata.name == reference.package()
                && reference
                    .source()
                    .is_none_or(|source| &record.source == source)
        })
        .ok_or_else(|| Error::NotInstalled {
            name: reference.to_string(),
        })
}

/// The named package, and every dependency that nothing left would need.
///
/// Repeated until a pass changes nothing, because taking one package out can be
/// what makes the next one unneeded — a chain three deep goes in three passes,
/// each one uncovering the next.
///
/// **A package installed explicitly is never taken**, however unreferenced it
/// looks. That is the whole point of recording why a package is there: somebody
/// asked for it, and nothing about what else is on the device changes that.
fn with_unneeded(installed: &[Record], named: &Record) -> Vec<PackageName> {
    // A list rather than a set, because the order is part of the answer: what
    // somebody asked for is the first line of what they are shown, and what
    // followed from it comes after, in the order it followed.
    let mut going = vec![named.metadata.name.clone()];

    loop {
        let orphan = installed.iter().find(|record| {
            record.reason == Reason::Dependency
                && !going.contains(&record.metadata.name)
                && !installed.iter().any(|other| {
                    !going.contains(&other.metadata.name) && other.depends_on(&record.metadata.name)
                })
        });

        match orphan {
            Some(record) => going.push(record.metadata.name.clone()),
            None => break,
        }
    }

    going
}

/// The files of a record that no package left standing also claims.
///
/// `install` refuses to let two records claim one file, so on a device this
/// tool built this takes nothing away. It is a net rather than a mechanism: a
/// record is a file somebody can edit, and deleting a file another package is
/// using because two records disagreed is not a mistake worth being able to
/// make.
fn unshared(
    store: &Store,
    installed: &[Record],
    going: &[PackageName],
    record: &Record,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut files = Vec::new();
    let mut kept = Vec::new();

    for file in &record.files {
        let claimed = installed
            .iter()
            .any(|other| !going.contains(&other.metadata.name) && other.files.contains(file));
        if claimed {
            continue;
        }
        // The third place the same question is asked. A configuration file
        // nobody touched is a stale default and goes with the package; one
        // somebody edited is theirs, and outlives the package that brought it.
        if conffile::may_delete(store.root(), file, &record.digests)? {
            files.push(file.clone());
        } else {
            kept.push(file.clone());
        }
    }

    Ok((files, kept))
}

/// Delete one package's files, and the directories they leave empty.
fn take_back(store: &Store, going: &Going) -> Result<()> {
    let record = store.record_file(&going.name);
    let mut emptied: Vec<PathBuf> = Vec::new();

    // Backwards, which is the order a record lists them in reverse: they were
    // written parents-first, so this takes the deepest thing first.
    for file in going.files.iter().rev() {
        let Some(path) = under(store, file) else {
            // Nothing this crate writes can name such a path. Refusing beats
            // deleting whatever it points at.
            return Err(Error::Parse {
                path: record,
                message: format!(
                    "it claims the file {}, which is not inside the device's root",
                    file.display()
                ),
            });
        };

        match fs::remove_file(&path) {
            Ok(()) => {}
            // Already gone: somebody deleted it by hand, which is not a reason
            // to leave the rest of the package on the device.
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::Io { path, source }),
        }

        if let Some(parent) = file.parent()
            && parent.components().next().is_some()
            && !emptied.contains(&parent.to_path_buf())
        {
            emptied.push(parent.to_path_buf());
        }
    }

    // Deepest first, so a directory holding nothing but other emptied ones goes
    // in the same pass.
    emptied.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in &emptied {
        db::prune(store.root(), directory, &record)?;
    }

    Ok(())
}

/// Resolve a recorded file against the root, or `None` if it would escape.
fn under(store: &Store, file: &std::path::Path) -> Option<PathBuf> {
    for part in file.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return None;
        }
    }
    Some(store.root().join(file))
}
