/*
  resolve.rs

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

//! Names to packages, and packages to the set they drag in.
//!
//! A package name identifies a package **within a source**. Two sources are
//! free to offer the same name, so a bare `helix` is only an answer when one
//! source offers it; when several do, this refuses and lists them qualified
//! rather than picking one. Picking one would mean a device installing
//! something other than what the person meant, and being consistent about
//! which is worse than being obvious about neither.

use crate::error::{Error, Result};
use crate::model::index::{IndexPackage, IndexVersion};
use crate::model::name::{PackageName, PackageRef, SourceName, Target};
use crate::model::version::Version;
use crate::store::config::Sources;
use crate::store::{Store, index};

/// A package, and the source that offers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The source it came from.
    pub source: SourceName,
    /// Everything that source says about it.
    pub package: IndexPackage,
}

impl Found {
    /// How this package has to be written when more than one source offers it.
    #[must_use]
    pub fn qualified(&self) -> PackageRef {
        PackageRef::qualified(self.package.name.clone(), self.source.clone())
    }
}

/// One version of one package in one source: what `install` needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    /// The source it came from.
    pub source: SourceName,
    /// What the package is called.
    pub name: PackageName,
    /// What the source says it is.
    pub description: String,
    /// The version chosen, and everything needed to fetch and check it.
    pub version: IndexVersion,
}

/// Every source that offers this package, by source name.
///
/// Ordered, so that what a refusal lists does not depend on the order the
/// configuration happens to be in.
///
/// # Errors
///
/// Whatever reading the configuration or an index gives.
pub fn candidates(store: &Store, name: &PackageName) -> Result<Vec<Found>> {
    let sources = Sources::load(store)?;

    let mut found = Vec::new();
    for source in sources.iter() {
        // A source whose index has never been fetched offers nothing yet, and
        // that is not an error - it is an `update` waiting to happen.
        if let Some(index) = index::read(store, &source.name)?
            && let Some(package) = index.package(name)
        {
            found.push(Found {
                source: source.name.clone(),
                package: package.clone(),
            });
        }
    }

    found.sort_by(|left, right| left.source.cmp(&right.source));
    Ok(found)
}

/// Resolve what somebody typed to one package in one source.
///
/// # Errors
///
/// [`Error::SourceNotFound`] if a named source is not configured,
/// [`Error::PackageNotFound`] if nothing offers the package, and
/// [`Error::Ambiguous`] if more than one source does and no source was named.
pub fn find(store: &Store, reference: &PackageRef) -> Result<Found> {
    let mut offering = candidates(store, reference.package())?;

    let Some(wanted) = reference.source() else {
        return match offering.len() {
            0 => Err(Error::PackageNotFound {
                name: reference.package().as_str().to_owned(),
            }),
            1 => Ok(offering.remove(0)),
            // Listed qualified, in the form the user has to type back.
            _ => Err(Error::Ambiguous {
                name: reference.package().as_str().to_owned(),
                candidates: offering
                    .iter()
                    .map(|found| found.qualified().to_string())
                    .collect(),
            }),
        };
    };

    // A source was named, so the question is only whether it has it - and
    // whether it is a source at all, which is a different answer.
    if Sources::load(store)?.by_name(wanted).is_none() {
        return Err(Error::SourceNotFound {
            name: wanted.as_str().to_owned(),
        });
    }

    offering
        .into_iter()
        .find(|found| &found.source == wanted)
        .ok_or_else(|| Error::PackageNotFound {
            name: reference.to_string(),
        })
}

/// Resolve a name to one version of one package.
///
/// The version asked for, or the highest built for this device. A package that
/// exists only for other machines is reported as exactly that, listing the
/// targets it does come in — which is a different thing from not existing.
///
/// # Errors
///
/// As [`find`], plus [`Error::VersionNotFound`] if the version asked for is
/// not offered and [`Error::TargetNotAvailable`] if none of them was built for
/// this device.
pub fn select(
    store: &Store,
    reference: &PackageRef,
    target: &Target,
    version: Option<&Version>,
) -> Result<Selected> {
    let found = find(store, reference)?;

    let chosen = match version {
        Some(wanted) => found
            .package
            .versions
            .iter()
            .find(|entry| &entry.version == wanted && &entry.target == target)
            .cloned()
            .ok_or_else(|| {
                // Offered, but not for this machine, is not the same as not
                // offered at all.
                let for_another = found
                    .package
                    .versions
                    .iter()
                    .any(|entry| &entry.version == wanted);
                if for_another {
                    target_not_available(&found, target)
                } else {
                    Error::VersionNotFound {
                        package: found.package.name.as_str().to_owned(),
                        version: wanted.as_str().to_owned(),
                    }
                }
            })?,
        None => found
            .package
            .newest(target)
            .cloned()
            .ok_or_else(|| target_not_available(&found, target))?,
    };

    Ok(Selected {
        source: found.source,
        name: found.package.name,
        description: found.package.description,
        version: chosen,
    })
}

/// "Not for this machine", listing the machines it is for.
fn target_not_available(found: &Found, target: &Target) -> Error {
    let available: Vec<&str> = found
        .package
        .targets()
        .into_iter()
        .map(Target::as_str)
        .collect();
    Error::TargetNotAvailable {
        package: found.package.name.as_str().to_owned(),
        target: target.as_str().to_owned(),
        available: available.join(", "),
    }
}
