/*
  update.rs

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

//! `update`: fetch each source's index and put it in place.
//!
//! Atomically, and without letting one unreachable source stop the others.
//!
//! An index is parsed **in full** before anything is written, and then renamed
//! into place — so an `update` interrupted by a power cut or by nonsense on
//! the wire leaves the previous index rather than half of the next one. A
//! device with a stale index can still install; a device with half an index
//! can do nothing at all.
//!
//! Failures are collected rather than thrown. Every source is attempted, each
//! failure is reported with the name of the source it belongs to, and the
//! command finishes non-zero: the picture it leaves is incomplete, and a
//! script that carries on regardless should have to say so deliberately.

use crate::error::{Error, Result};
use crate::model::name::{PackageName, SourceName};
use crate::net::transport::Transport;
use crate::ops::source::fetch_index;
use crate::store::config::Sources;
use crate::store::{Store, index};

/// Which sources to update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Which {
    /// All of them, which is what happens when neither option is given.
    All,
    /// One, by name.
    One(SourceName),
}

/// What updating one source did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    /// The source.
    pub name: SourceName,
    /// How many packages its index now offers.
    pub packages: usize,
    /// How many of those were not in the index this replaced.
    ///
    /// Zero on a first fetch as well as on a fetch that changed nothing —
    /// there was no previous index to have been missing from.
    pub new_packages: usize,
}

/// A source that could not be updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The source.
    pub name: SourceName,
    /// Where its index is published.
    pub url: String,
    /// What went wrong, in the words of whatever went wrong.
    pub message: String,
}

/// What the whole command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The sources that were updated.
    pub updated: Vec<Updated>,
    /// The sources that were not.
    pub failed: Vec<Failure>,
}

impl Report {
    /// Whether the command succeeded.
    ///
    /// Separate from doing the work, so the caller can print everything that
    /// happened and *then* fail. A failure that hid the successes would make
    /// the next run's behaviour hard to predict.
    ///
    /// # Errors
    ///
    /// [`Error::Incomplete`] naming the sources that failed, if any did.
    pub fn outcome(&self) -> Result<()> {
        if self.failed.is_empty() {
            return Ok(());
        }
        Err(Error::Incomplete {
            failed: self
                .failed
                .iter()
                .map(|failure| failure.name.as_str().to_owned())
                .collect(),
            total: self.updated.len().saturating_add(self.failed.len()),
        })
    }
}

/// Fetch and replace the indexes.
///
/// # Errors
///
/// [`Error::SourceNotFound`] if a named source is not configured, or whatever
/// reading the configuration gives. A source that cannot be *fetched* is not
/// an error here — it is a [`Failure`] in the report, so that the other
/// sources are still attempted.
pub fn update(store: &Store, transport: &dyn Transport, which: &Which) -> Result<Report> {
    let sources = Sources::load(store)?;

    let selected: Vec<_> = match which {
        Which::All => sources.iter().cloned().collect(),
        Which::One(name) => {
            let found = sources
                .by_name(name)
                .cloned()
                .ok_or_else(|| Error::SourceNotFound {
                    name: name.as_str().to_owned(),
                })?;
            vec![found]
        }
    };

    let mut report = Report {
        updated: Vec::new(),
        failed: Vec::new(),
    };

    for source in selected {
        // What it offered before, so that "3 new" means something. Read
        // before the fetch, because the fetch is what replaces it.
        let before = index::read(store, &source.name)
            .ok()
            .flatten()
            .map(|index| {
                index
                    .packages
                    .into_iter()
                    .map(|package| package.name)
                    .collect::<Vec<PackageName>>()
            })
            .unwrap_or_default();

        match fetch_index(transport, &source.url) {
            Ok(index) => {
                let packages = index.packages.len();
                let new_packages = if before.is_empty() {
                    0
                } else {
                    index
                        .packages
                        .iter()
                        .filter(|package| !before.contains(&package.name))
                        .count()
                };

                // Written whole or not at all - see the note at the top.
                if let Err(error) = index::write(store, &source.name, &index) {
                    report.failed.push(Failure {
                        name: source.name.clone(),
                        url: source.url.clone(),
                        message: error.to_string(),
                    });
                    continue;
                }

                report.updated.push(Updated {
                    name: source.name.clone(),
                    packages,
                    new_packages,
                });
            }
            Err(error) => report.failed.push(Failure {
                name: source.name.clone(),
                url: source.url.clone(),
                message: error.to_string(),
            }),
        }
    }

    Ok(report)
}
