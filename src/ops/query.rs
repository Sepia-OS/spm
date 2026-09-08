/*
  query.rs

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

//! The commands that only read: `search`, `info`, `list`, `list-sources`,
//! `source-info`.
//!
//! None of them touches the network or takes the lock. They report what is on
//! the device, which is only ever as current as the last `update` — and saying
//! so is part of their job.

use crate::error::{Error, Result};
use crate::model::name::SourceName;
use crate::store::config::Sources;
use crate::store::db::Database;
use crate::store::{Store, index};

/// What is known about one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReport {
    /// Its name, which is what every other command refers to it by.
    pub name: SourceName,
    /// Where its index is published.
    pub url: String,
    /// Whether it is the source used when a command needs one and none was
    /// given.
    pub is_default: bool,
    /// What its local index says, or `None` if it has never been fetched.
    ///
    /// Not an empty index: a source nobody has updated and a source offering
    /// nothing look alike in a listing and mean opposite things.
    pub index: Option<IndexSummary>,
    /// How many installed packages came from it.
    pub installed: usize,
}

/// What a fetched index amounts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexSummary {
    /// When the source last rebuilt it, in seconds since the epoch.
    pub updated: u64,
    /// How many packages it offers.
    pub packages: usize,
}

/// Every configured source, by name, with what is known about each.
///
/// An empty list is not an error. It is what a freshly installed device looks
/// like, and the caller says so.
///
/// # Errors
///
/// [`Error::Io`] or [`Error::Parse`] if the configuration, an index or a
/// record cannot be read.
pub fn list_sources(store: &Store) -> Result<Vec<SourceReport>> {
    let sources = Sources::load(store)?;
    let installed = Database::new(store).all()?;

    let mut reports = Vec::new();
    for source in sources.iter() {
        let summary = index::read(store, &source.name)?.map(|index| IndexSummary {
            updated: index.updated,
            packages: index.packages.len(),
        });
        reports.push(SourceReport {
            name: source.name.clone(),
            url: source.url.clone(),
            is_default: source.is_default,
            index: summary,
            // A package keeps its source recorded even after that source is
            // removed, so this counts what came from here rather than what
            // this source currently offers.
            installed: installed
                .iter()
                .filter(|record| record.source == source.name)
                .count(),
        });
    }

    reports.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(reports)
}

/// What is known about the source published at this URL.
///
/// Addressed by URL, as `docs/dev/ARCHITECTURE.md` specifies — which is the
/// one place in the command set where a source is not named. `list-sources` is
/// where a user finds the URL, and the name.
///
/// # Errors
///
/// [`Error::SourceNotFound`] if no configured source has that URL, or whatever
/// reading the state gives.
pub fn source_info(store: &Store, url: &str) -> Result<SourceReport> {
    list_sources(store)?
        .into_iter()
        .find(|report| report.url == url)
        .ok_or_else(|| Error::SourceNotFound {
            name: url.to_owned(),
        })
}
