/*
  metadata.rs

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

//! A package's `metadata.json`.
//!
//! What `create` writes into a package and what a source's scan reads out of a
//! release. It is the one format a person writes by hand, so it is read
//! strictly: an unknown field is a misspelling to report rather than a key to
//! drop, and every value is validated on the way in.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::name::{PackageName, Target};
use crate::model::version::Version;
use crate::sign::{PublicKey, Signature};

/// A SHA-256 digest, as 64 lower-case hexadecimal characters.
///
/// A type rather than a `String` because there are two digests in play — one
/// of a package and one of the payload inside it — and `docs/dev/DESIGN.md` is
/// blunt about what confusing them costs: "a verification that passes while
/// checking nothing". Two `String`s are interchangeable by accident. These are
/// at least the same shape as each other, which is as far as a type can help;
/// the naming does the rest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256(String);

impl Sha256 {
    /// The number of characters in a SHA-256 written as hexadecimal.
    pub const LENGTH: usize = 64;

    /// Read a digest, or `None` if the text is not one.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() != Self::LENGTH {
            return None;
        }
        // Hexadecimal, and lower case: upper case would be a second spelling of
        // the same digest, so two equal digests could fail to compare equal.
        if !text
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return None;
        }
        Some(Sha256(text.to_owned()))
    }

    /// The digest as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Sha256 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Sha256::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "'{text}' is not a SHA-256 digest: {} lower-case hexadecimal characters",
                Sha256::LENGTH
            ))
        })
    }
}

/// `""` in the file means "not filled in yet", which is what an author writes
/// and what `create` replaces.
mod unfilled_is_empty {
    use super::Sha256;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        digest: &Option<Sha256>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match digest {
            Some(digest) => serializer.serialize_str(digest.as_str()),
            None => serializer.serialize_str(""),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Sha256>, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.is_empty() {
            return Ok(None);
        }
        Sha256::parse(&text).map(Some).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "'{text}' is not a SHA-256 digest: {} lower-case hexadecimal characters, or \"\" if it has not been filled in yet",
                Sha256::LENGTH
            ))
        })
    }
}

/// A package this one needs, and the version of it that will do.
///
/// The version is a floor: *that version or a newer one*. The index accumulates
/// versions rather than replacing them, so a dependency naming an exact version
/// would stop being satisfiable the first time the package it names is
/// upgraded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    /// The package that has to be installed too.
    pub name: PackageName,
    /// The oldest version of it that will do.
    pub version: Version,
}

/// Everything a package says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    /// What the package is called.
    pub name: PackageName,
    /// The version of the software inside it.
    pub version: Version,
    /// What it was built for.
    pub target: Target,
    /// A sentence or two, shown by `search` and `info`.
    pub description: String,
    /// The packages that have to be installed alongside this one.
    pub dependencies: Vec<Dependency>,
    /// The digest of `data.tar.gz`, or `None` while it is unwritten.
    ///
    /// The only field `create` writes rather than reads: the digest cannot
    /// exist before the payload it describes does. An author leaves it empty
    /// and `create` fills it in on the copy it packs.
    #[serde(with = "unfilled_is_empty")]
    pub sha256: Option<Sha256>,
    /// The key the package was signed with, or `None` while it is unsigned.
    ///
    /// Written by `create --sign`, like the digest above and for the same
    /// reason: an author cannot know it, because it is a fact about the build
    /// rather than about the software. A device checks it against the key the
    /// index published for this package - a package that names a key of its own
    /// choosing proves nothing.
    #[serde(default, with = "unsigned_is_empty_key")]
    pub public_key: Option<PublicKey>,
    /// The signature over this package's identity and payload.
    ///
    /// Not over this file: `metadata.json` is JSON, and two serialisations of
    /// one document differ in whitespace while meaning the same thing, so a
    /// signature over it would break for reasons that have nothing to do with
    /// the package. What is signed is name, version, target and the payload
    /// digest, bound together - see [`crate::sign`].
    #[serde(default, with = "unsigned_is_empty_signature")]
    pub signature: Option<Signature>,
}

/// `""` in the file means "not signed yet", as `""` means "not hashed yet".
mod unsigned_is_empty_key {
    use super::PublicKey;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        key: &Option<PublicKey>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match key {
            Some(key) => serializer.serialize_str(key.as_str()),
            None => serializer.serialize_str(""),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<PublicKey>, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.is_empty() {
            return Ok(None);
        }
        PublicKey::parse(&text).map(Some).ok_or_else(|| {
            serde::de::Error::custom(format!("'{text}' is not an Ed25519 public key"))
        })
    }
}

/// The same, for the signature.
mod unsigned_is_empty_signature {
    use super::Signature;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(
        signature: &Option<Signature>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match signature {
            Some(signature) => serializer.serialize_str(signature.as_str()),
            None => serializer.serialize_str(""),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Signature>, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.is_empty() {
            return Ok(None);
        }
        Signature::parse(&text).map(Some).ok_or_else(|| {
            serde::de::Error::custom(format!("'{text}' is not an Ed25519 signature"))
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    /// The example from `docs/dev/ARCHITECTURE.md`, verbatim.
    ///
    /// It is the fixture on purpose: if the parser and the documentation ever
    /// disagree, this test is what says so.
    const ARCHITECTURE_EXAMPLE: &str = r#"{
  "name": "helix",
  "version": "25.07.1",
  "target": "aarch64-musl",
  "description": "The Helix editor, with its tree-sitter grammars.",
  "dependencies": [
    { "name": "llvm-runtime", "version": "23.1.0" }
  ],
  "sha256": ""
}"#;

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    #[test]
    fn the_documented_example_parses() {
        let metadata: Metadata = serde_json::from_str(ARCHITECTURE_EXAMPLE).unwrap();
        assert_eq!(metadata.name.as_str(), "helix");
        assert_eq!(metadata.version.as_str(), "25.07.1");
        assert_eq!(metadata.target.as_str(), "aarch64-musl");
        assert_eq!(metadata.dependencies.len(), 1);
        assert_eq!(metadata.dependencies[0].name.as_str(), "llvm-runtime");
        assert_eq!(metadata.dependencies[0].version.as_str(), "23.1.0");
        assert!(metadata.sha256.is_none(), "an author leaves it empty");
    }

    #[test]
    fn the_documented_example_round_trips() {
        let metadata: Metadata = serde_json::from_str(ARCHITECTURE_EXAMPLE).unwrap();
        let written = serde_json::to_string(&metadata).unwrap();
        let again: Metadata = serde_json::from_str(&written).unwrap();
        assert_eq!(metadata, again);
    }

    #[test]
    fn an_unfilled_digest_is_written_back_as_empty() {
        let metadata: Metadata = serde_json::from_str(ARCHITECTURE_EXAMPLE).unwrap();
        let written = serde_json::to_string(&metadata).unwrap();
        assert!(written.contains(r#""sha256":"""#), "{written}");
    }

    #[test]
    fn a_filled_digest_round_trips() {
        let mut metadata: Metadata = serde_json::from_str(ARCHITECTURE_EXAMPLE).unwrap();
        metadata.sha256 = Sha256::parse(&digest(0xab));
        assert!(metadata.sha256.is_some());
        let written = serde_json::to_string(&metadata).unwrap();
        let again: Metadata = serde_json::from_str(&written).unwrap();
        assert_eq!(again.sha256, metadata.sha256);
    }

    #[test]
    fn a_missing_name_is_an_error() {
        let without = ARCHITECTURE_EXAMPLE.replace(r#""name": "helix","#, "");
        let result: Result<Metadata, _> = serde_json::from_str(&without);
        let message = result.unwrap_err().to_string();
        assert!(message.contains("name"), "{message}");
    }

    #[test]
    fn a_misspelled_field_is_reported_rather_than_dropped() {
        // The whole reason for deny_unknown_fields: a hand-written file with
        // `dependancies` would otherwise install a package with no
        // dependencies and say nothing.
        let misspelled = ARCHITECTURE_EXAMPLE.replace(r#""dependencies""#, r#""dependancies""#);
        let result: Result<Metadata, _> = serde_json::from_str(&misspelled);
        let message = result.unwrap_err().to_string();
        assert!(message.contains("dependancies"), "{message}");
    }

    #[test]
    fn a_bad_value_is_refused_with_the_value_in_the_message() {
        let bad_version = ARCHITECTURE_EXAMPLE.replace(r#""25.07.1""#, r#""""#);
        let message = serde_json::from_str::<Metadata>(&bad_version)
            .unwrap_err()
            .to_string();
        assert!(message.contains("not a version"), "{message}");

        let bad_name = ARCHITECTURE_EXAMPLE.replace(r#""helix""#, r#""../../etc/passwd""#);
        let message = serde_json::from_str::<Metadata>(&bad_name)
            .unwrap_err()
            .to_string();
        assert!(message.contains("not allowed"), "{message}");
    }

    #[test]
    fn a_digest_has_to_be_sixty_four_lower_case_hex_characters() {
        assert!(Sha256::parse(&digest(0x00)).is_some());
        assert!(Sha256::parse("").is_none());
        assert!(Sha256::parse("abc").is_none());
        // Upper case is a different spelling of the same digest, and allowing
        // both would mean two strings that are equal and do not compare equal.
        assert!(Sha256::parse(&digest(0xab).to_uppercase()).is_none());
        // Right length, wrong alphabet.
        assert!(Sha256::parse(&"z".repeat(64)).is_none());
    }

    #[test]
    fn a_dependency_is_a_name_and_a_floor() {
        let dependency: Dependency =
            serde_json::from_str(r#"{ "name": "llvm-runtime", "version": "23.1.0" }"#).unwrap();
        assert_eq!(dependency.name.as_str(), "llvm-runtime");
        assert_eq!(dependency.version.as_str(), "23.1.0");
        let extra: Result<Dependency, _> =
            serde_json::from_str(r#"{ "name": "x", "version": "1", "optional": true }"#);
        assert!(extra.is_err(), "an unknown field is an error here too");
    }
}
