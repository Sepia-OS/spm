/*
  upgrade.rs

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

//! `upgrade`: move installed packages onto newer versions.
//!
//! **It reads the local indexes and nothing else**, so it finds nothing the
//! last `update` did not. That is the rule `docs/dev/ARCHITECTURE.md` sets for
//! every command that reads an index, and it is what makes this one
//! predictable: what it will do can be seen with `--dry-run` and will not have
//! changed by the time it is run for real.
//!
//! **A newer version comes from the source the package came from.** A record
//! says where it was installed from, and that is where its upgrades come from;
//! taking one from another source because the name matched would swap a package
//! for a different package of the same name. A source that has since been
//! removed therefore offers nothing, which is exactly what `remove-source` says
//! will happen.
//!
//! **One package that cannot be upgraded is not a reason to leave the device
//! unpatched.** A package whose new version needs something that cannot be
//! satisfied is left at the version it has and reported; everything else still
//! moves; and the command finishes non-zero so that a script can tell a
//! complete job from a partial one. That is the same shape as `update`, and for
//! the same reason.
//!
//! Everything from the plan onwards is `install`'s — the plan, the room check,
//! the two digests, the extraction rules, the journal. There is no second path
//! here that fetches or unpacks anything.

use crate::error::{Error, Result};
use crate::model::installed::Record;
use crate::model::name::{PackageName, Target};
use crate::model::version::Version;
use crate::net::transport::Transport;
use crate::ops::install::{self, Plan};
use crate::ops::resolve::{Needed, Selected};
use crate::store::db::Database;
use crate::store::{Store, index};

/// A package that could have moved and did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// The package.
    pub name: PackageName,
    /// The version it stays at.
    pub installed: Version,
    /// The version it could have been.
    pub offered: Version,
    /// Why it could not be, in the words of the failure itself.
    pub why: String,
}

/// What an upgrade did, and what it could not do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Installs that had not finished and have now been taken back.
    pub rolled_back: Vec<PackageName>,
    /// What was to be done, which is an `install` plan like any other.
    pub plan: Plan,
    /// The packages left where they were, and why.
    pub held: Vec<Held>,
    /// How many packages were considered.
    pub considered: usize,
    /// Whether it was done, as opposed to only described.
    pub changed: bool,
}

impl Report {
    /// Whether anything would move.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plan.is_empty()
    }

    /// The exit this leaves behind, once everything has been printed.
    ///
    /// Separate from the work, like `update`'s: the caller prints what happened
    /// and *then* fails, so that a script sees the packages that were upgraded
    /// as well as the one that was not. A command that failed at the first
    /// problem would hide the successes it had already had.
    ///
    /// # Errors
    ///
    /// [`Error::UpgradeIncomplete`] if any package was left where it was.
    pub fn outcome(&self) -> Result<()> {
        if self.held.is_empty() {
            return Ok(());
        }
        Err(Error::UpgradeIncomplete {
            held: self
                .held
                .iter()
                .map(|held| format!("{} {}", held.name, held.installed))
                .collect(),
            total: self.considered,
        })
    }
}

/// Move everything that can move, or just one package.
///
/// With `dry_run` the set is worked out and returned and **nothing is
/// touched**.
///
/// # Errors
///
/// [`Error::NotInstalled`] if a package is named and is not on the device, and
/// whatever planning or carrying out the set gives. A package that simply
/// cannot be upgraded is **not** an error here — it is in [`Report::held`], and
/// [`Report::outcome`] is what turns that into one.
pub fn upgrade(
    store: &Store,
    transport: &dyn Transport,
    target: &Target,
    only: Option<&PackageName>,
    dry_run: bool,
) -> Result<Report> {
    // Before anything else, as every command that writes must.
    let rolled_back = Database::new(store).recover()?;

    let installed = Database::new(store).all()?;
    let considering: Vec<&Record> = match only {
        Some(name) => {
            let record = installed
                .iter()
                .find(|record| &record.metadata.name == name)
                .ok_or_else(|| Error::NotInstalled {
                    name: name.as_str().to_owned(),
                })?;
            vec![record]
        }
        None => installed.iter().collect(),
    };
    let considered = considering.len();

    // Every package with something newer to move to, from the source it came
    // from.
    let mut candidates = Vec::new();
    for record in considering {
        if let Some(newer) = newer_than(store, record, target)? {
            candidates.push((record, newer));
        }
    }

    // Then, one at a time, whether that new version's dependencies can be
    // satisfied. One at a time on purpose: resolving them all together would
    // mean one unsatisfiable package taking the whole upgrade down with it,
    // and the point is to upgrade what can be upgraded.
    let mut roots = Vec::new();
    let mut held = Vec::new();
    for (record, newer) in candidates {
        let root = Needed {
            selected: newer,
            // A package that came in as a dependency is still one. Making it
            // explicit because it moved would quietly take it out of
            // autoremove's reach for ever.
            reason: record.reason,
        };
        match crate::ops::resolve::with_dependencies(store, std::slice::from_ref(&root), target) {
            Ok(_) => roots.push(root),
            Err(why) => held.push(Held {
                name: record.metadata.name.clone(),
                installed: record.metadata.version.clone(),
                offered: root.selected.version.version.clone(),
                why: why.to_string(),
            }),
        }
    }

    // The survivors resolved together, so that a dependency two of them share
    // is worked out once and at one version.
    let needed = crate::ops::resolve::with_dependencies(store, &roots, target)?;
    let plan = install::plan_for(store, needed)?;

    if dry_run || plan.is_empty() {
        return Ok(Report {
            rolled_back,
            plan,
            held,
            considered,
            changed: false,
        });
    }

    install::carry_out(store, transport, &plan)?;

    Ok(Report {
        rolled_back,
        plan,
        held,
        considered,
        changed: true,
    })
}

/// The version of a record's package that its own source now offers, if that
/// is newer than the one installed.
///
/// `None` covers every way there is nothing to do, and they are deliberately
/// not told apart: the source is gone, its index has never been fetched, it no
/// longer lists the package, it has nothing built for this machine, or what it
/// has is not newer. None of them is a failure, and none of them is something
/// a person can act on differently.
fn newer_than(store: &Store, record: &Record, target: &Target) -> Result<Option<Selected>> {
    let Some(index) = index::read(store, &record.source)? else {
        return Ok(None);
    };
    let Some(package) = index.package(&record.metadata.name) else {
        return Ok(None);
    };
    let Some(newest) = package.newest(target) else {
        return Ok(None);
    };
    if newest.version <= record.metadata.version {
        return Ok(None);
    }

    Ok(Some(Selected {
        source: record.source.clone(),
        name: package.name.clone(),
        description: package.description.clone(),
        version: newest.clone(),
    }))
}
