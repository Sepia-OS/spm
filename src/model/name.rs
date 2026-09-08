/*
  name.rs

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

//! Package and source names, and the `<source>/<package>` form.
//!
//! Parsed once, at the edge, so that everything inside works with a name
//! already known to be well-formed. The rules are stricter than they look like
//! they need to be, and the reason is that **a name becomes a filename**:
//! an installed package is recorded at `/var/lib/spm/installed/<name>.json`
//! and a source's index at `/var/lib/spm/index/<source>.json`. A package
//! called `../../etc/passwd` would otherwise be a package that writes wherever
//! it likes, so the traversal is refused here rather than guarded for at every
//! place a path is built.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The longest a name may be.
///
/// Well under any filesystem's limit, with room for the `.json` a record adds.
/// Nothing real comes close: the longest name in SepiaOS is `rust-toolchain`.
const MAX_LENGTH: usize = 128;

/// Why a name was refused.
///
/// Not an application error and not a variant of [`crate::error::Error`]: what
/// a bad name *means* depends on where it came from. In `metadata.json` it is
/// a `Parse` with the path; on the command line it is a `Usage`. So this says
/// what is wrong with the name and the caller says what that amounts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidName {
    /// There was nothing there.
    Empty,
    /// Longer than [`MAX_LENGTH`].
    TooLong {
        /// How long it actually was.
        length: usize,
    },
    /// Did not begin with a letter or a digit.
    BadStart {
        /// The character it began with.
        first: char,
    },
    /// Contained something that is not allowed in a name.
    BadCharacter {
        /// The offending character.
        character: char,
    },
    /// Contained an upper-case letter.
    Uppercase,
    /// More than one `/`, so it is not `<source>/<package>`.
    TooManyParts,
}

impl fmt::Display for InvalidName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidName::Empty => formatter.write_str("a name cannot be empty"),
            InvalidName::TooLong { length } => write!(
                formatter,
                "a name cannot be longer than {MAX_LENGTH} characters, and this one is {length}"
            ),
            InvalidName::BadStart { first } => write!(
                formatter,
                "a name has to start with a letter or a digit, and this one starts with '{first}'"
            ),
            InvalidName::BadCharacter { character } => write!(
                formatter,
                "'{character}' is not allowed in a name - letters, digits, and - _ . + are"
            ),
            InvalidName::Uppercase => formatter.write_str(
                "a name has to be lower case: it becomes a filename, and on a filesystem that ignores case two spellings would be one file",
            ),
            InvalidName::TooManyParts => formatter.write_str(
                "a name is either <package> or <source>/<package>, and this one has more than one '/'",
            ),
        }
    }
}

/// Check a single name and return it.
///
/// Shared by both name types because they are used the same way and stored the
/// same way; a rule that held for one and not the other would be a trap.
fn validate(text: &str) -> Result<String, InvalidName> {
    if text.is_empty() {
        return Err(InvalidName::Empty);
    }
    if text.len() > MAX_LENGTH {
        return Err(InvalidName::TooLong { length: text.len() });
    }

    for character in text.chars() {
        if character.is_ascii_uppercase() {
            return Err(InvalidName::Uppercase);
        }
        let allowed = character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_' | '.' | '+');
        if !allowed {
            return Err(InvalidName::BadCharacter { character });
        }
    }

    // Must begin with a letter or a digit. That is what rules out `.`, `..`
    // and anything else that would be a hidden file or a step up a directory,
    // and it also keeps a name from looking like a command-line option.
    let Some(first) = text.chars().next() else {
        return Err(InvalidName::Empty);
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(InvalidName::BadStart { first });
    }

    Ok(text.to_owned())
}

/// The name of a package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageName(String);

/// The name of a source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceName(String);

/// What a package was built for: `aarch64-musl`.
///
/// Validated like a name, and for the same reason — it is part of the filename
/// `create` writes, `<name>-<version>-<target>.tar.gz`, so a target that walks
/// up a directory would be a package written outside the output directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Target(String);

macro_rules! name_type {
    ($type:ty, $what:literal) => {
        impl $type {
            #[doc = concat!("Read a ", $what, " name, or say what is wrong with it.")]
            ///
            /// # Errors
            ///
            /// Returns [`InvalidName`] describing the first rule the text
            /// breaks.
            pub fn parse(text: &str) -> Result<Self, InvalidName> {
                validate(text).map(Self)
            }

            #[doc = concat!("The ", $what, " name as text.")]
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl Serialize for $type {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        // Validated on the way in, so a name that reached a file by some other
        // route than this crate is refused when it is read rather than used.
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Self::parse(&text)
                    .map_err(|reason| serde::de::Error::custom(format!("'{text}': {reason}")))
            }
        }
    };
}

impl Target {
    /// What this device is.
    ///
    /// Built from the compiler's own idea of the machine, so a binary knows
    /// what it can install without being told: `aarch64-musl` on a SepiaOS
    /// card. On anything else it is that machine's honest answer — a
    /// workstation says so, and is then told that a package built for a card
    /// is not built for it, which is the truth.
    ///
    /// # Panics
    ///
    /// Never in practice: the value is built from `std::env::consts`, which
    /// are always names this module accepts.
    #[must_use]
    pub fn current() -> Self {
        let flavour = if cfg!(target_env = "musl") {
            "musl"
        } else if cfg!(target_env = "gnu") {
            "gnu"
        } else {
            std::env::consts::OS
        };
        Target::parse(&format!("{}-{flavour}", std::env::consts::ARCH))
            .unwrap_or_else(|_| Target(format!("{}-unknown", std::env::consts::ARCH)))
    }
}

name_type!(PackageName, "package");
name_type!(SourceName, "source");
name_type!(Target, "target");

/// A package, and optionally the source it is to come from.
///
/// This is what a user types: `helix`, or `sepia/helix` when more than one
/// source offers the name. One type parses both spellings so that no command
/// re-implements the split — and every command needs it, because `install`,
/// `remove`, `info` and `search` all take a name that may be qualified.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageRef {
    source: Option<SourceName>,
    package: PackageName,
}

impl PackageRef {
    /// Read `<package>` or `<source>/<package>`.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidName`] if either part breaks a rule, or if there is
    /// more than one `/`.
    pub fn parse(text: &str) -> Result<Self, InvalidName> {
        let mut parts = text.split('/');
        let Some(first) = parts.next() else {
            return Err(InvalidName::Empty);
        };

        match parts.next() {
            None => Ok(PackageRef {
                source: None,
                package: PackageName::parse(first)?,
            }),
            Some(second) => {
                if parts.next().is_some() {
                    return Err(InvalidName::TooManyParts);
                }
                Ok(PackageRef {
                    source: Some(SourceName::parse(first)?),
                    package: PackageName::parse(second)?,
                })
            }
        }
    }

    /// The source the user named, if they named one.
    #[must_use]
    pub fn source(&self) -> Option<&SourceName> {
        self.source.as_ref()
    }

    /// The package.
    #[must_use]
    pub fn package(&self) -> &PackageName {
        &self.package
    }

    /// The reference with a source attached, as `search` and `list` print a
    /// package that more than one source offers.
    #[must_use]
    pub fn qualified(package: PackageName, source: SourceName) -> Self {
        PackageRef {
            source: Some(source),
            package,
        }
    }
}

impl fmt::Display for PackageRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Some(source) => write!(formatter, "{source}/{}", self.package),
            None => write!(formatter, "{}", self.package),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_round_trips() {
        let reference = PackageRef::parse("helix").unwrap();
        assert_eq!(reference.package().as_str(), "helix");
        assert!(reference.source().is_none());
        assert_eq!(reference.to_string(), "helix");
    }

    #[test]
    fn a_qualified_name_round_trips() {
        let reference = PackageRef::parse("sepia/helix").unwrap();
        assert_eq!(reference.package().as_str(), "helix");
        assert_eq!(reference.source().unwrap().as_str(), "sepia");
        assert_eq!(reference.to_string(), "sepia/helix");
    }

    #[test]
    fn real_sepiaos_package_names_are_accepted() {
        for name in [
            "helix",
            "grit",
            "llvm-runtime",
            "e2fsprogs",
            "rust-toolchain",
            "wpa_supplicant",
            "libnl-3",
            "spm",
            "musl",
        ] {
            assert!(PackageName::parse(name).is_ok(), "{name} should be a name");
        }
    }

    #[test]
    fn a_name_cannot_be_empty() {
        assert_eq!(PackageName::parse(""), Err(InvalidName::Empty));
        assert_eq!(PackageRef::parse(""), Err(InvalidName::Empty));
        // Both halves of a qualified name have to be there.
        assert_eq!(PackageRef::parse("sepia/"), Err(InvalidName::Empty));
        assert_eq!(PackageRef::parse("/helix"), Err(InvalidName::Empty));
    }

    #[test]
    fn a_name_cannot_hold_whitespace() {
        assert_eq!(
            PackageName::parse("two words"),
            Err(InvalidName::BadCharacter { character: ' ' })
        );
        assert!(PackageName::parse("helix\n").is_err());
        assert!(PackageName::parse("\thelix").is_err());
    }

    #[test]
    fn a_name_cannot_be_a_path() {
        // The rule this module exists for: a record is a file named after the
        // package, so a name that walks up a directory must not be a name.
        assert!(PackageName::parse("..").is_err());
        assert!(PackageName::parse(".").is_err());
        assert!(PackageName::parse("../../etc/passwd").is_err());
        assert_eq!(
            PackageName::parse("/etc/passwd"),
            Err(InvalidName::BadCharacter { character: '/' })
        );
    }

    #[test]
    fn a_reference_has_at_most_one_slash() {
        assert_eq!(
            PackageRef::parse("sepia/extra/helix"),
            Err(InvalidName::TooManyParts)
        );
    }

    #[test]
    fn a_name_is_lower_case() {
        assert_eq!(PackageName::parse("Helix"), Err(InvalidName::Uppercase));
        assert_eq!(PackageName::parse("HELIX"), Err(InvalidName::Uppercase));
        assert!(PackageName::parse("helix").is_ok());
    }

    #[test]
    fn a_name_starts_with_a_letter_or_a_digit() {
        assert_eq!(
            PackageName::parse("-helix"),
            Err(InvalidName::BadStart { first: '-' })
        );
        assert_eq!(
            PackageName::parse(".hidden"),
            Err(InvalidName::BadStart { first: '.' })
        );
        assert!(PackageName::parse("7zip").is_ok());
    }

    #[test]
    fn a_name_has_a_length_limit() {
        let long = "a".repeat(MAX_LENGTH);
        assert!(PackageName::parse(&long).is_ok());
        let longer = "a".repeat(MAX_LENGTH + 1);
        assert_eq!(
            PackageName::parse(&longer),
            Err(InvalidName::TooLong {
                length: MAX_LENGTH + 1
            })
        );
    }

    #[test]
    fn a_source_name_follows_the_same_rules() {
        assert!(SourceName::parse("sepia").is_ok());
        assert!(SourceName::parse("..").is_err());
        assert_eq!(SourceName::parse("Sepia"), Err(InvalidName::Uppercase));
    }

    #[test]
    fn a_qualified_reference_can_be_built_for_display() {
        let reference = PackageRef::qualified(
            PackageName::parse("helix").unwrap(),
            SourceName::parse("sepia").unwrap(),
        );
        assert_eq!(reference.to_string(), "sepia/helix");
    }

    #[test]
    fn a_target_is_a_name_too() {
        assert!(Target::parse("aarch64-musl").is_ok());
        assert!(Target::parse("x86_64-unknown-linux-gnu").is_ok());
        // The reason it is validated at all.
        assert!(Target::parse("../../evil").is_err());
    }

    #[test]
    fn names_round_trip_through_json() {
        let name = PackageName::parse("llvm-runtime").unwrap();
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, "\"llvm-runtime\"");
        assert_eq!(serde_json::from_str::<PackageName>(&json).unwrap(), name);
    }

    #[test]
    fn a_name_that_reached_a_file_by_another_route_is_still_refused() {
        // Nothing in this crate can write it, but a file is a file.
        let result: Result<PackageName, _> = serde_json::from_str("\"../../etc/passwd\"");
        let message = result.unwrap_err().to_string();
        assert!(message.contains("../../etc/passwd"), "{message}");
        assert!(message.contains("not allowed"), "{message}");
    }

    #[test]
    fn every_refusal_says_what_is_wrong() {
        for reason in [
            InvalidName::Empty,
            InvalidName::TooLong { length: 200 },
            InvalidName::BadStart { first: '-' },
            InvalidName::BadCharacter { character: '/' },
            InvalidName::Uppercase,
            InvalidName::TooManyParts,
        ] {
            let text = reason.to_string();
            assert!(text.len() > 20, "too terse to act on: {text}");
        }
    }
}
