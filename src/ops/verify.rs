/*
  verify.rs

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

//! `verify`: re-check what is installed against the records.
//!
//! The only command that reads the card and the records and nothing else - no
//! network, no index, and nothing written. It answers one question: is what the
//! records say is installed still there?
//!
//! Three findings, and the difference between them is the whole design:
//!
//! - **Missing.** A recorded file is gone. Something took it - a hand, an
//!   operation that did not finish, a card that lost a block - and the package
//!   is no longer what it claims to be. A fault.
//! - **Not a file any more.** Records list files and symlinks and never
//!   directories, so a directory at a recorded path is something else standing
//!   where the package's file should be. A fault.
//! - **Modified.** The file is there and its contents are not what was
//!   installed. A fault, and the one this command exists for: a record carries
//!   the digest of every file as it was written, so a binary that lost a block
//!   to a tired card is found here rather than when somebody runs it.
//! - **Edited configuration.** A file under `etc/` whose contents differ in
//!   exactly the same way - and **not** a fault, because it is the expected
//!   result of somebody administering the device and the reason
//!   `crate::conffile` exists. The mismatch is identical; where the file lives
//!   is what says which of the two it means.
//!
//! **A symlink is checked for being there and for still being a link**, and no
//! further: it has no contents of its own, and hashing what it points at would
//! report on somebody else's file.

use std::fs;
use std::path::PathBuf;

use crate::conffile;
use crate::error::{Error, Result};
use crate::model::installed::Record;
use crate::model::name::{PackageName, PackageRef};
use crate::model::version::Version;
use crate::store::Store;
use crate::store::db::Database;

/// What is wrong with one file, or merely worth saying about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// The record lists it and the card does not have it.
    Missing,
    /// Something that is not a file stands where the file should.
    NotAFile,
    /// The file is there and its contents are not what was installed.
    ///
    /// Under `usr/` that is a fault: the package owns the file, so something
    /// changed it underneath - a card that lost a block, or a hand. It is
    /// [`Finding::Edited`] rather than this when the file is configuration,
    /// where the same mismatch is somebody doing their job.
    Modified,
    /// Configuration that no longer matches what was installed.
    ///
    /// Not a fault. It is what an administrator editing a default looks like.
    Edited,
}

impl Finding {
    /// Whether this finding means the installation is broken.
    ///
    /// Edited configuration is not: it is the documented outcome of the
    /// configuration rules, and a `verify` that failed because somebody had
    /// configured their device would be a `verify` nobody ran twice.
    #[must_use]
    pub fn is_fault(&self) -> bool {
        matches!(
            self,
            Finding::Missing | Finding::NotAFile | Finding::Modified
        )
    }
}

/// One file, and what was found about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// Where it is, relative to the device's root.
    pub path: PathBuf,
    /// What was found.
    pub finding: Finding,
}

/// One package, and what checking it came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// What it is called.
    pub name: PackageName,
    /// The version installed.
    pub version: Version,
    /// How many files the record lists.
    pub files: usize,
    /// Everything worth saying, in the order the record lists the files.
    ///
    /// Empty when the package is exactly as it was installed.
    pub findings: Vec<Checked>,
}

impl Verified {
    /// Whether anything about this package is broken.
    #[must_use]
    pub fn is_sound(&self) -> bool {
        !self
            .findings
            .iter()
            .any(|checked| checked.finding.is_fault())
    }

    /// How many of its files are missing or are no longer files.
    #[must_use]
    pub fn faults(&self) -> usize {
        self.findings
            .iter()
            .filter(|checked| checked.finding.is_fault())
            .count()
    }

    /// How many of its configuration files have been edited.
    #[must_use]
    pub fn edited(&self) -> usize {
        self.findings
            .iter()
            .filter(|checked| checked.finding == Finding::Edited)
            .count()
    }
}

/// What checking a device came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Every package that was checked, in the order the database lists them.
    pub packages: Vec<Verified>,
}

impl Report {
    /// Whether every package checked is exactly as it was installed.
    #[must_use]
    pub fn is_sound(&self) -> bool {
        self.packages.iter().all(Verified::is_sound)
    }

    /// How many files are missing or are no longer files, across all of them.
    #[must_use]
    pub fn faults(&self) -> usize {
        self.packages.iter().map(Verified::faults).sum()
    }

    /// How many configuration files have been edited, across all of them.
    #[must_use]
    pub fn edited(&self) -> usize {
        self.packages.iter().map(Verified::edited).sum()
    }

    /// The packages that are not as they were installed.
    #[must_use]
    pub fn broken(&self) -> Vec<&Verified> {
        self.packages
            .iter()
            .filter(|package| !package.is_sound())
            .collect()
    }
}

/// Check one installed package, or every one of them.
///
/// `reference` is `None` to check the whole device. A device with nothing
/// installed verifies successfully and says so: there is nothing wrong with a
/// card that has had nothing added to it.
///
/// Nothing is written and nothing is fetched, so this is safe to run at any
/// time and says nothing about whether a newer version exists.
///
/// # Errors
///
/// [`Error::NotInstalled`] if a package was named and is not installed,
/// [`Error::Io`] if a configuration file is there and cannot be read, and
/// whatever reading the records gives.
pub fn verify(store: &Store, reference: Option<&PackageRef>) -> Result<Report> {
    let installed = Database::new(store).all()?;

    let records: Vec<Record> = match reference {
        None => installed,
        Some(reference) => {
            let found = installed
                .into_iter()
                .find(|record| {
                    &record.metadata.name == reference.package()
                        && reference
                            .source()
                            .is_none_or(|source| &record.source == source)
                })
                .ok_or_else(|| Error::NotInstalled {
                    name: reference.to_string(),
                })?;
            vec![found]
        }
    };

    let mut packages = Vec::with_capacity(records.len());
    for record in &records {
        packages.push(one(store, record)?);
    }

    Ok(Report { packages })
}

/// Check every file one record lists.
fn one(store: &Store, record: &Record) -> Result<Verified> {
    let mut findings = Vec::new();

    for path in &record.files {
        let full = store.root().join(path);

        // `symlink_metadata`, not `metadata`: a recorded symlink is a file the
        // package put there, and following it would report on whatever it
        // points at instead - including saying "missing" for a link whose
        // target has gone, which is a different fault about a different file.
        let found = match fs::symlink_metadata(&full) {
            Ok(found) => found,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                findings.push(Checked {
                    path: path.clone(),
                    finding: Finding::Missing,
                });
                continue;
            }
            Err(source) => return Err(Error::Io { path: full, source }),
        };

        // A record holds a digest for every regular file it wrote, so having
        // one means the thing installed here was a file. Anything else standing
        // in its place now - a directory, or a symlink where a file was - is
        // something other than the package's file, whatever else is true.
        let installed = record.digests.get(path);
        if found.is_dir() || (installed.is_some() && !found.is_file()) {
            findings.push(Checked {
                path: path.clone(),
                finding: Finding::NotAFile,
            });
            continue;
        }

        // The contents. A symlink has no digest and nothing to compare; for
        // everything else a mismatch means one of two different things, and
        // where the file lives is what says which.
        if let Some(installed) = installed
            && !conffile::is_untouched(&full, Some(installed))?
        {
            findings.push(Checked {
                path: path.clone(),
                finding: if conffile::is_config(path) {
                    Finding::Edited
                } else {
                    Finding::Modified
                },
            });
        }
    }

    Ok(Verified {
        name: record.metadata.name.clone(),
        version: record.metadata.version.clone(),
        files: record.files.len(),
        findings,
    })
}
