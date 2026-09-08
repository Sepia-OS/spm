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

use std::collections::VecDeque;

use crate::error::{Error, Result};
use crate::model::index::{IndexPackage, IndexVersion};
use crate::model::installed::{Reason, Record};
use crate::model::name::{PackageName, PackageRef, SourceName, Target};
use crate::model::version::Version;
use crate::store::config::Sources;
use crate::store::db::Database;
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

/// One package in the set an install would put on the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needed {
    /// Which package, at which version, from which source.
    pub selected: Selected,
    /// Whether it is what was asked for or something that came in with it.
    pub reason: Reason,
}

/// A package in the working set, and what pulled it in.
///
/// The parent is not part of the answer; it is what makes a circle
/// distinguishable from a diamond, which are the same thing seen from the
/// package that closes them.
#[derive(Debug)]
struct Pending {
    needed: Needed,
    parent: Option<usize>,
}

/// The whole set to install: these packages, and everything they need.
///
/// Takes the roots with the reason each is to be recorded under, because that
/// is not always the same answer: `install` asks for a package by name and it
/// becomes explicit whatever it was before, while `upgrade` moves a package
/// that may have come in as a dependency and has to leave it one.
///
/// Breadth-first over the transitive dependencies. Three rules decide what
/// comes in, and all three exist so that installing one package does not
/// rearrange the device around it:
///
/// - **A dependency's version is a floor**, not an exact version, so the lowest
///   version that satisfies it is taken. The newest would upgrade half the card
///   because one package asked for something old.
/// - **Never older than what is installed.** A floor below the installed
///   version is already met by it, so nothing is downgraded to satisfy a
///   dependency.
/// - **A dependency already satisfied is not touched at all**, which is what
///   makes installing a second package that needs `llvm-runtime` cost nothing.
///
/// The result lists everything that came in as a dependency first, in the order
/// it was discovered, and the roots last, in the order they were given. That is
/// a reading order and not an installation order: a package is files and
/// nothing else — there are no maintainer scripts, and musl has no
/// `ld.so.cache` — so nothing depends on which of them is written first.
///
/// **A circle is only found below a root.** Two roots that need each other are
/// not reported as one, and should not be: both are in the set already, so the
/// set *can* be completed, which is the only thing a circle would have made
/// impossible.
///
/// # Errors
///
/// [`Error::DependencyCycle`] if the packages need each other in a circle,
/// [`Error::DependencyNotSatisfiable`] if a dependency exists but not new
/// enough, and whatever [`find`] gives for one that is missing or ambiguous.
pub fn with_dependencies(store: &Store, roots: &[Needed], target: &Target) -> Result<Vec<Needed>> {
    let installed = Database::new(store).all()?;

    let mut set: Vec<Pending> = roots
        .iter()
        .map(|root| Pending {
            needed: root.clone(),
            parent: None,
        })
        .collect();
    let mut queue: VecDeque<usize> = (0..set.len()).collect();

    while let Some(at) = queue.pop_front() {
        let wants = set[at].needed.selected.clone();
        for dependency in &wants.version.dependencies {
            // The floor is the dependency's, raised to whatever is already on
            // the device: nothing is downgraded to satisfy a dependency.
            let floor = match version_of(&installed, &dependency.name) {
                Some(have) if have > &dependency.version => have.clone(),
                _ => dependency.version.clone(),
            };

            if let Some(already) = position_of(&set, &dependency.name) {
                // Reaching a package that is already in the set is either a
                // diamond - two packages needing one third - or a circle. It
                // is a circle exactly when what we reached is what we came
                // from.
                if already == at || is_ancestor(&set, already, at) {
                    return Err(Error::DependencyCycle {
                        chain: circle(&set, already, at),
                    });
                }
                // A diamond whose two sides disagree about how new the shared
                // package has to be: the higher floor wins, and what it needs
                // in turn is worked out again from there.
                if set[already].needed.selected.version.version < floor {
                    let raised = pick(store, &wants, &dependency.name, target, &floor)?;
                    set[already].needed.selected = raised;
                    queue.push_back(already);
                }
                continue;
            }

            // Already on the device and new enough: not reinstalled, not
            // mentioned, not touched.
            if version_of(&installed, &dependency.name).is_some_and(|have| have >= &floor) {
                continue;
            }

            let selected = pick(store, &wants, &dependency.name, target, &floor)?;
            set.push(Pending {
                needed: Needed {
                    // A package the user once asked for by name stays theirs
                    // even when it is now also a dependency, or autoremove
                    // would eventually take away something they chose.
                    reason: reason_for(&installed, &selected.name),
                    selected,
                },
                parent: Some(at),
            });
            queue.push_back(set.len().saturating_sub(1));
        }
    }

    // Dependencies first, the roots last, which is the order somebody reading
    // the plan expects to see them in.
    let mut needed: Vec<Needed> = set.into_iter().map(|pending| pending.needed).collect();
    needed.rotate_left(roots.len());
    Ok(needed)
}

/// Choose the version of a dependency to bring in.
fn pick(
    store: &Store,
    wants: &Selected,
    dependency: &PackageName,
    target: &Target,
    floor: &Version,
) -> Result<Selected> {
    let found = for_dependency(store, dependency, &wants.source)?;

    let Some(chosen) = found.package.lowest_from(target, floor) else {
        // Three ways to have no version, and three different fixes: it is for
        // other machines, or it is here and too old.
        return Err(match found.package.newest(target) {
            Some(newest) => Error::DependencyNotSatisfiable {
                package: wants.name.as_str().to_owned(),
                dependency: dependency.as_str().to_owned(),
                needed: floor.as_str().to_owned(),
                available: newest.version.as_str().to_owned(),
            },
            None => target_not_available(&found, target),
        });
    };

    Ok(Selected {
        source: found.source,
        name: found.package.name.clone(),
        description: found.package.description.clone(),
        version: chosen.clone(),
    })
}

/// Find the package a dependency names, preferring the source it was named in.
///
/// A dependency is a bare name, and a name identifies a package only within a
/// source. `helix` from `sepia` asking for `llvm-runtime` means the one `sepia`
/// publishes — pulling a package of that name out of an unrelated source
/// because the name happened to match is how a device ends up with something
/// nobody chose. Only when the source that named it does not have it is the
/// question widened, and then the ordinary rules apply: one answer, or a
/// refusal listing them.
fn for_dependency(store: &Store, want: &PackageName, prefer: &SourceName) -> Result<Found> {
    let offering = candidates(store, want)?;
    if let Some(same) = offering.iter().find(|found| &found.source == prefer) {
        return Ok(same.clone());
    }
    find(store, &PackageRef::of(want.clone()))
}

/// The version of a package the device already has, if it has one.
fn version_of<'records>(
    installed: &'records [Record],
    name: &PackageName,
) -> Option<&'records Version> {
    installed
        .iter()
        .find(|record| &record.metadata.name == name)
        .map(|record| &record.metadata.version)
}

/// Why a package will be on the device once this install has run.
fn reason_for(installed: &[Record], name: &PackageName) -> Reason {
    let was_explicit = installed
        .iter()
        .any(|record| &record.metadata.name == name && record.is_explicit());
    if was_explicit {
        Reason::Explicit
    } else {
        Reason::Dependency
    }
}

/// Where a package sits in the working set, if it is in it.
fn position_of(set: &[Pending], name: &PackageName) -> Option<usize> {
    set.iter()
        .position(|pending| &pending.needed.selected.name == name)
}

/// Whether `ancestor` is on the path from the root down to `of`.
fn is_ancestor(set: &[Pending], ancestor: usize, of: usize) -> bool {
    let mut at = set.get(of).and_then(|pending| pending.parent);
    while let Some(index) = at {
        if index == ancestor {
            return true;
        }
        at = set.get(index).and_then(|pending| pending.parent);
    }
    false
}

/// The circle, written the way somebody has to read it: from the package that
/// is depended on, down to the one that depends on it, and back round.
fn circle(set: &[Pending], from: usize, to: usize) -> Vec<String> {
    let mut names = vec![name_at(set, to)];
    let mut at = set.get(to).and_then(|pending| pending.parent);
    while let Some(index) = at {
        names.push(name_at(set, index));
        if index == from {
            break;
        }
        at = set.get(index).and_then(|pending| pending.parent);
    }
    names.reverse();
    names.push(name_at(set, from));
    names
}

fn name_at(set: &[Pending], at: usize) -> String {
    set.get(at).map_or_else(String::new, |pending| {
        pending.needed.selected.name.as_str().to_owned()
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
