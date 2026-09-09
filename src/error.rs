/*
  error.rs

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

//! The one error type, and the exit code each variant maps to.
//!
//! Every fallible path in `spm` ends here. The mapping to exit codes is part of
//! the documented interface — `docs/USER-GUIDE.md` prints the table for people
//! writing scripts — so it is tested rather than assumed, and [`Error::exit_code`]
//! matches exhaustively so that a new variant without a code will not compile.
//!
//! | code | meaning |
//! |---|---|
//! | 0 | success |
//! | 1 | a general failure |
//! | 2 | wrong usage |
//! | 3 | not found |
//! | 4 | ambiguous |
//! | 5 | a network or TLS failure |
//! | 6 | a checksum or verification failure |
//! | 7 | a conflict |
//! | 8 | incomplete |
//!
//! Variants carry plain strings for package and source names rather than the
//! types in [`crate::model`]. It keeps this module dependent on nothing, so
//! every other module — `model` included — can return from it.

use std::path::PathBuf;

use crate::model::name::SourceRef;
use thiserror::Error;

/// Everything that can go wrong, and enough context with it to act.
#[derive(Debug, Error)]
pub enum Error {
    /// The command line did not make sense.
    #[error("{0}")]
    Usage(String),

    /// A file could not be read or written.
    #[error("cannot access {path}: {source}")]
    Io {
        /// What was being accessed.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },

    /// A file was read but could not be understood.
    #[error("cannot read {path}: {message}")]
    Parse {
        /// The file that could not be parsed.
        path: PathBuf,
        /// What was wrong with it.
        message: String,
    },

    /// Another `spm` holds the lock.
    #[error("another spm is running - it holds {path}; wait for it to finish")]
    Locked {
        /// The lock file being held.
        path: PathBuf,
    },

    /// No configured source offers this package.
    #[error(
        "no package named '{name}' in any configured source - run 'spm update' first, or 'spm search {name}' to look for a similar one"
    )]
    PackageNotFound {
        /// The name that was asked for.
        name: String,
    },

    /// A search found nothing.
    ///
    /// Its own answer rather than [`Error::PackageNotFound`], whose advice is
    /// to run `spm search` — which is what has just happened.
    #[error(
        "nothing matched '{needle}' - 'spm update' fetches whatever the sources have published since the last time"
    )]
    NothingMatched {
        /// What was searched for.
        needle: String,
    },

    /// Nothing on the device goes by that name.
    ///
    /// Distinct from [`Error::PackageNotFound`], whose advice is to update or
    /// search — a package that exists everywhere except on this device is not
    /// a package anybody needs to go looking for.
    #[error("'{name}' is not installed - 'spm list --installed' shows what is")]
    NotInstalled {
        /// The name that was asked for, as it was typed.
        name: String,
    },

    /// No source is configured under this name, or at this URL.
    #[error("no source {} - 'spm list-sources' shows the configured ones", .reference.describe())]
    SourceNotFound {
        /// How the source was addressed, so the message can say which of the
        /// two was looked for rather than guessing at one wording for both.
        reference: SourceRef,
    },

    /// The package exists, but not at that version.
    #[error(
        "'{package}' has no version {version} - 'spm info {package}' lists the versions the index offers"
    )]
    VersionNotFound {
        /// The package.
        package: String,
        /// The version that was asked for.
        version: String,
    },

    /// The package exists in this source but not for this device.
    #[error("'{package}' is not built for {target} - the index offers it for {available}")]
    TargetNotAvailable {
        /// The package.
        package: String,
        /// The target this device needs.
        target: String,
        /// The targets the index does offer, comma separated.
        available: String,
    },

    /// More than one source offers the name, so it does not identify a package.
    #[error("'{name}' is offered by more than one source: {} - install it by its full name, for example '{}'", .candidates.join(", "), .candidates.first().map(String::as_str).unwrap_or(""))]
    Ambiguous {
        /// The name that was asked for.
        name: String,
        /// The qualified names it could mean.
        candidates: Vec<String>,
    },

    /// Something could not be fetched.
    #[error("cannot reach {url}: {message}")]
    Network {
        /// What was being fetched.
        url: String,
        /// What went wrong.
        message: String,
    },

    /// A certificate was rejected, and the clock is the likely reason.
    #[error(
        "cannot verify the certificate for {url}: this device's clock reads {reading}, which is before every certificate's start date - set the time with 'sepia-time sync' and try again"
    )]
    ClockBehind {
        /// What was being fetched.
        url: String,
        /// What the device believes the date is.
        reading: String,
    },

    /// A package needs a version of something that nothing offers.
    ///
    /// Distinct from [`Error::PackageNotFound`], which is the dependency not
    /// existing at all, and from [`Error::TargetNotAvailable`], which is it
    /// existing for other machines. This one is "it is here, and not new
    /// enough", and the fix is a different one again.
    #[error(
        "'{package}' needs '{dependency}' at version {needed} or newer, and the newest one offered for this machine is {available} - 'spm update' fetches whatever the sources have published since the last time"
    )]
    DependencyNotSatisfiable {
        /// The package that wants it.
        package: String,
        /// The package it wants.
        dependency: String,
        /// The oldest version that would do.
        needed: String,
        /// The newest version there actually is.
        available: String,
    },

    /// Packages that need each other, so no set containing them is complete.
    #[error(
        "these packages need each other in a circle: {} - each of them is waiting for the next, so the set can never be completed; it is a mistake in the packages themselves, and whoever publishes them has to break the circle", .chain.join(" -> ")
    )]
    DependencyCycle {
        /// The circle, from the package that closes it back round to itself.
        chain: Vec<String>,
    },

    /// A package holds something that will not be unpacked into `/`.
    ///
    /// Its own variant rather than an [`Error::Io`] because nothing went
    /// wrong with the device: the package asked for something extraction does
    /// not do, and the refusal is the whole point.
    #[error(
        "refusing to unpack {path}: {reason}; nothing from this package has been written, and it is not one this tool would have built"
    )]
    UnsafeEntry {
        /// The path inside the package, exactly as the package spells it.
        path: PathBuf,
        /// Which rule it breaks.
        reason: String,
    },

    /// The card would fill up.
    #[error(
        "there is not enough room on {path}: {needed} is needed and {free} is free - a package is on the card twice while it installs, the download and the unpacked files, so an install needs about twice what it downloads; 'rm -rf /var/cache/spm/*' is safe at any time"
    )]
    NotEnoughSpace {
        /// The filesystem that would fill up.
        path: PathBuf,
        /// How much is needed, ready to read.
        needed: String,
        /// How much there is, ready to read.
        free: String,
    },

    /// Something did not match the digest that was published for it.
    #[error(
        "{what} does not match its checksum - expected {expected}, found {found}; the download was corrupted, or the source published something that does not match its own index"
    )]
    Verification {
        /// What was being checked.
        what: String,
        /// The digest that was expected.
        expected: String,
        /// The digest that was computed.
        found: String,
    },

    /// Two packages claim one file.
    #[error(
        "{path} is owned by the package '{owner}' - two packages cannot own one file, so remove '{owner}' first or take it up with whoever publishes them"
    )]
    FileConflict {
        /// The file both want.
        path: PathBuf,
        /// The package that already owns it.
        owner: String,
    },

    /// A file is in the way and belongs to nobody.
    #[error(
        "{path} already exists and no package owns it - it came from the system image or was put there by hand; move it aside to let this package own it"
    )]
    FileUnowned {
        /// The file that is in the way.
        path: PathBuf,
    },

    /// Removing this package would break others.
    #[error("'{package}' is required by {} - remove those first, or leave it in place", .dependents.join(", "))]
    HasDependents {
        /// The package that was to be removed.
        package: String,
        /// The installed packages that need it.
        dependents: Vec<String>,
    },

    /// A staged tree cannot be made into a package.
    #[error("{path} cannot be packaged: {reason}")]
    NotPackageable {
        /// What is wrong, or where what is missing should have been.
        path: PathBuf,
        /// Why it cannot be packaged.
        reason: String,
    },

    /// Some packages moved and some could not.
    ///
    /// Its own variant rather than [`Error::Incomplete`], which is about
    /// sources and says so. Both mean the same thing to a script — the picture
    /// you were left with is not the whole one — and both leave the same code.
    #[error(
        "{} of {total} packages could not be upgraded: {} - they are still installed at the version they had, and everything else was upgraded", .held.len(), .held.join(", ")
    )]
    UpgradeIncomplete {
        /// The packages left where they were, with the version they stay at.
        held: Vec<String>,
        /// How many were considered.
        total: usize,
    },

    /// Some of the work succeeded and some did not.
    #[error("{} of {total} sources could not be updated: {}", .failed.len(), .failed.join(", "))]
    Incomplete {
        /// The sources that failed.
        failed: Vec<String>,
        /// How many were attempted.
        total: usize,
    },
}

impl Error {
    /// The exit code this error leaves the process with.
    ///
    /// The match is exhaustive on purpose: there is no catch-all arm, so a new
    /// variant added without a code is a compile error rather than a silent 1.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            // A circle in the dependencies and a full card are neither of them
            // a thing that is missing, ambiguous, or in conflict on disk, and
            // neither has a code of its own in the documented table.
            Error::Io { .. }
            | Error::Parse { .. }
            | Error::Locked { .. }
            | Error::DependencyCycle { .. }
            | Error::NotEnoughSpace { .. } => 1,
            Error::Usage(_) | Error::NotPackageable { .. } => 2,
            Error::PackageNotFound { .. }
            | Error::NothingMatched { .. }
            | Error::SourceNotFound { .. }
            | Error::VersionNotFound { .. }
            | Error::NotInstalled { .. }
            | Error::TargetNotAvailable { .. }
            // The version that would satisfy it is the thing that is not there.
            | Error::DependencyNotSatisfiable { .. } => 3,
            Error::Ambiguous { .. } => 4,
            Error::Network { .. } | Error::ClockBehind { .. } => 5,
            // A package carrying `../../etc/passwd` fails a check about itself,
            // which is the same class as failing its digest - and the same
            // class of thing that may mean something other than bad luck.
            Error::Verification { .. } | Error::UnsafeEntry { .. } => 6,
            Error::FileConflict { .. }
            | Error::FileUnowned { .. }
            | Error::HasDependents { .. } => 7,
            Error::Incomplete { .. } | Error::UpgradeIncomplete { .. } => 8,
        }
    }
}

/// The result every fallible function in this crate returns.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// One of every variant, so that the table below cannot go stale by
    /// somebody adding a variant and forgetting this test.
    fn one_of_each() -> Vec<(Error, u8)> {
        vec![
            (
                Error::Io {
                    path: PathBuf::from("/var/lib/spm/lock"),
                    source: std::io::Error::other("nope"),
                },
                1,
            ),
            (
                Error::Parse {
                    path: PathBuf::from("/etc/spm/sources.json"),
                    message: "expected a value".to_owned(),
                },
                1,
            ),
            (
                Error::Locked {
                    path: PathBuf::from("/var/lib/spm/lock"),
                },
                1,
            ),
            (
                Error::Usage("--all and --source cannot be used together".to_owned()),
                2,
            ),
            (
                Error::NotPackageable {
                    path: PathBuf::from("etc/motd"),
                    reason: "everything in a package has to be under usr/".to_owned(),
                },
                2,
            ),
            (
                Error::PackageNotFound {
                    name: "helix".to_owned(),
                },
                3,
            ),
            (
                Error::SourceNotFound {
                    reference: SourceRef::parse("sepia"),
                },
                3,
            ),
            (
                Error::NothingMatched {
                    needle: "hel".to_owned(),
                },
                3,
            ),
            (
                Error::NotInstalled {
                    name: "helix".to_owned(),
                },
                3,
            ),
            (
                Error::VersionNotFound {
                    package: "helix".to_owned(),
                    version: "9.9.9".to_owned(),
                },
                3,
            ),
            (
                Error::TargetNotAvailable {
                    package: "helix".to_owned(),
                    target: "aarch64-musl".to_owned(),
                    available: "x86_64-musl".to_owned(),
                },
                3,
            ),
            (
                Error::DependencyNotSatisfiable {
                    package: "helix".to_owned(),
                    dependency: "llvm-runtime".to_owned(),
                    needed: "23.1.0".to_owned(),
                    available: "21.0.0".to_owned(),
                },
                3,
            ),
            (
                Error::DependencyCycle {
                    chain: vec![
                        "helix".to_owned(),
                        "llvm-runtime".to_owned(),
                        "helix".to_owned(),
                    ],
                },
                1,
            ),
            (
                Error::UnsafeEntry {
                    path: PathBuf::from("../../etc/passwd"),
                    reason: "a package may only write under usr/".to_owned(),
                },
                6,
            ),
            (
                Error::NotEnoughSpace {
                    path: PathBuf::from("/"),
                    needed: "431.0 MiB".to_owned(),
                    free: "12.4 MiB".to_owned(),
                },
                1,
            ),
            (
                Error::Ambiguous {
                    name: "helix".to_owned(),
                    candidates: vec!["sepia/helix".to_owned(), "local/helix".to_owned()],
                },
                4,
            ),
            (
                Error::Network {
                    url: "https://example.invalid/index.json".to_owned(),
                    message: "connection refused".to_owned(),
                },
                5,
            ),
            (
                Error::ClockBehind {
                    url: "https://example.invalid/index.json".to_owned(),
                    reading: "1970-01-01".to_owned(),
                },
                5,
            ),
            (
                Error::Verification {
                    what: "data.tar.gz".to_owned(),
                    expected: "aaaa".to_owned(),
                    found: "bbbb".to_owned(),
                },
                6,
            ),
            (
                Error::FileConflict {
                    path: PathBuf::from("/usr/bin/hx"),
                    owner: "helix".to_owned(),
                },
                7,
            ),
            (
                Error::FileUnowned {
                    path: PathBuf::from("/usr/bin/hx"),
                },
                7,
            ),
            (
                Error::HasDependents {
                    package: "llvm-runtime".to_owned(),
                    dependents: vec!["helix".to_owned()],
                },
                7,
            ),
            (
                Error::Incomplete {
                    failed: vec!["local".to_owned()],
                    total: 2,
                },
                8,
            ),
            (
                Error::UpgradeIncomplete {
                    held: vec!["helix 25.07.1".to_owned()],
                    total: 3,
                },
                8,
            ),
        ]
    }

    #[test]
    fn every_variant_has_its_documented_code() {
        for (error, code) in one_of_each() {
            assert_eq!(error.exit_code(), code, "wrong exit code for {error:?}");
        }
    }

    #[test]
    fn codes_stay_inside_the_documented_range() {
        for (error, _) in one_of_each() {
            let code = error.exit_code();
            assert!(
                (1..=8).contains(&code),
                "{error:?} uses undocumented code {code}"
            );
        }
    }

    #[test]
    fn success_is_not_an_error() {
        // 0 belongs to ExitCode::SUCCESS and to nothing in this enum.
        for (error, _) in one_of_each() {
            assert_ne!(error.exit_code(), 0);
        }
    }

    #[test]
    fn messages_name_the_thing_that_went_wrong() {
        let error = Error::PackageNotFound {
            name: "helix".to_owned(),
        };
        let text = error.to_string();
        assert!(text.contains("helix"), "{text}");
        assert!(
            text.contains("spm update"),
            "an error should say what to do: {text}"
        );
    }

    #[test]
    fn an_ambiguous_name_lists_what_it_could_mean() {
        let error = Error::Ambiguous {
            name: "helix".to_owned(),
            candidates: vec!["sepia/helix".to_owned(), "local/helix".to_owned()],
        };
        let text = error.to_string();
        assert!(text.contains("sepia/helix"), "{text}");
        assert!(text.contains("local/helix"), "{text}");
    }
}
