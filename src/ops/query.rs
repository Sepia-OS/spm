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
use crate::model::index::IndexVersion;
use crate::model::name::{PackageName, PackageRef, SourceName, SourceRef, Target};
use crate::model::version::Version;
use crate::ops::resolve;
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
/// Addressed by the name it is configured under or by the URL it publishes at,
/// whichever the user typed. `list-sources` shows both.
///
/// # Errors
///
/// [`Error::SourceNotFound`] if no configured source has that name or URL, or
/// whatever reading the state gives.
pub fn source_info(store: &Store, reference: &SourceRef) -> Result<SourceReport> {
    list_sources(store)?
        .into_iter()
        .find(|report| match reference {
            SourceRef::Name(name) => &report.name == name,
            SourceRef::Url(url) => &report.url == url,
        })
        .ok_or_else(|| Error::SourceNotFound {
            reference: reference.clone(),
        })
}

/// One package, as `search` and `list` show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The source offering it.
    pub source: SourceName,
    /// What it is called.
    pub name: PackageName,
    /// The newest version built for this device, if any is.
    ///
    /// `None` when the package exists but not for this machine. Shown rather
    /// than hidden: "it exists, just not for you" is worth knowing.
    pub newest: Option<Version>,
    /// A sentence or two about it.
    pub description: String,
    /// The version installed *from this source*, if one is.
    pub installed: Option<Version>,
    /// Whether another source offers this name too, so it has to be written
    /// `<source>/<package>` to be unambiguous.
    pub qualify: bool,
}

/// Everything known about one package from one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    /// The source offering it.
    pub source: SourceName,
    /// What it is called.
    pub name: PackageName,
    /// A sentence or two about it.
    pub description: String,
    /// The version being shown.
    pub version: IndexVersion,
    /// The version installed from this source, if one is.
    pub installed: Option<Version>,
    /// Every version this source offers for this device, newest first.
    pub versions: Vec<Version>,
    /// Whether another source offers this name too.
    pub qualify: bool,
}

/// Search every source's index by name.
///
/// Matches anywhere in a name and ignores case, so a partial name finds what
/// somebody half-remembers.
///
/// # Errors
///
/// Whatever reading the configuration, an index or a record gives.
pub fn search(store: &Store, target: &Target, needle: &str) -> Result<Vec<Line>> {
    let needle = needle.to_lowercase();
    lines(store, target, |name| {
        name.as_str().to_lowercase().contains(&needle)
    })
}

/// Every package, or the ones from one source, or only the installed ones.
///
/// # Errors
///
/// [`Error::SourceNotFound`] if a source is named and not configured, or
/// whatever reading the state gives.
pub fn list(
    store: &Store,
    target: &Target,
    only_installed: bool,
    from: Option<&SourceName>,
) -> Result<Vec<Line>> {
    if let Some(name) = from
        && Sources::load(store)?.by_name(name).is_none()
    {
        return Err(Error::SourceNotFound {
            reference: SourceRef::Name(name.clone()),
        });
    }

    let all = lines(store, target, |_| true)?;
    Ok(all
        .into_iter()
        .filter(|line| from.is_none_or(|name| &line.source == name))
        .filter(|line| !only_installed || line.installed.is_some())
        .collect())
}

/// Everything known about a package, from every source that offers it.
///
/// A package two sources offer is not ambiguous here: `info` shows both, since
/// telling somebody about all of them is the answer to the question they
/// asked. Only the commands that *do* something have to refuse.
///
/// # Errors
///
/// [`Error::PackageNotFound`] if nothing offers it, [`Error::SourceNotFound`]
/// if a named source is not configured, and — when every source that offers it
/// fails to produce the version asked for — whichever error the first of them
/// gave.
pub fn info(
    store: &Store,
    reference: &PackageRef,
    target: &Target,
    version: Option<&Version>,
) -> Result<Vec<Detail>> {
    let found = match reference.source() {
        // A named source is a question about that source.
        Some(_) => vec![resolve::find(store, reference)?],
        None => {
            let all = resolve::candidates(store, reference.package())?;
            if all.is_empty() {
                return Err(Error::PackageNotFound {
                    name: reference.package().as_str().to_owned(),
                });
            }
            all
        }
    };

    let qualify = found.len() > 1;
    let installed = Database::new(store).all()?;

    let mut details = Vec::new();
    let mut first_failure = None;
    for one in found {
        let qualified = one.qualified();
        match resolve::select(store, &qualified, target, version) {
            Ok(selected) => {
                let mut versions: Vec<Version> = one
                    .package
                    .versions
                    .iter()
                    .filter(|entry| &entry.target == target)
                    .map(|entry| entry.version.clone())
                    .collect();
                versions.sort_by(|left, right| right.cmp(left));
                versions.dedup();

                details.push(Detail {
                    installed: installed_version(&installed, &one.source, &selected.name),
                    source: selected.source,
                    name: selected.name,
                    description: selected.description,
                    version: selected.version,
                    versions,
                    qualify,
                });
            }
            // One source not having the version asked for is not the answer
            // while another might; if none of them do, this is what is said.
            Err(error) => first_failure = first_failure.or(Some(error)),
        }
    }

    if details.is_empty() {
        return Err(first_failure.unwrap_or(Error::PackageNotFound {
            name: reference.package().as_str().to_owned(),
        }));
    }

    Ok(details)
}

/// Every package every source offers, filtered by a predicate on the name.
fn lines(
    store: &Store,
    target: &Target,
    wanted: impl Fn(&PackageName) -> bool,
) -> Result<Vec<Line>> {
    let sources = Sources::load(store)?;
    let installed = Database::new(store).all()?;

    let mut lines = Vec::new();
    for source in sources.iter() {
        let Some(index) = index::read(store, &source.name)? else {
            continue;
        };
        for package in index.packages {
            if !wanted(&package.name) {
                continue;
            }
            lines.push(Line {
                newest: package.newest(target).map(|entry| entry.version.clone()),
                installed: installed_version(&installed, &source.name, &package.name),
                source: source.name.clone(),
                name: package.name,
                description: package.description,
                qualify: false,
            });
        }
    }

    // A name more than one source offers has to be written qualified.
    let mut shared: Vec<PackageName> = Vec::new();
    for line in &lines {
        if lines.iter().filter(|other| other.name == line.name).count() > 1
            && !shared.contains(&line.name)
        {
            shared.push(line.name.clone());
        }
    }
    for line in &mut lines {
        line.qualify = shared.contains(&line.name);
    }

    lines.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source.cmp(&right.source))
    });
    Ok(lines)
}

/// The version installed from this source, if one is.
///
/// From *this* source: the same name from two sources is two packages, and
/// only one of them is on the device.
fn installed_version(
    installed: &[crate::model::installed::Record],
    source: &SourceName,
    name: &PackageName,
) -> Option<Version> {
    installed
        .iter()
        .find(|record| &record.source == source && &record.metadata.name == name)
        .map(|record| record.metadata.version.clone())
}
