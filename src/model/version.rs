/*
  version.rs

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

//! Upstream version numbers, and how they compare.
//!
//! Deliberately not semver. The versions here are whatever upstream chose:
//! `25.07.1`, `1.2.6`, `23.1.0`. `25.07.1` has a leading zero that semver
//! forbids, and the helix repository publishes the tag `25.07.1` for what its
//! own crate calls `25.7.1` — so the two have to compare equal, which neither
//! semver nor a string comparison would give.
//!
//! A version is its dot-separated components, compared left to right:
//!
//! - two numeric components compare numerically, so `25.07.1` equals `25.7.1`
//!   and `1.10` is above `1.9`;
//! - a numeric component sorts above a non-numeric one, so `1.0` is above
//!   `1.0-rc1`;
//! - two non-numeric components compare as text;
//! - a missing component counts as zero, so `1.2` equals `1.2.0`.
//!
//! That is an ordering and nothing more. It does not know what a major version
//! means, and it is not asked to: `dependencies` needs "this version or newer"
//! and `upgrade` needs "is there a higher one", and both are answered by this.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One dot-separated part of a version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Component {
    /// A part that is all digits and fits in a `u64`.
    Number(u64),
    /// Anything else, compared as text. A run of digits too long for a `u64`
    /// lands here too, and therefore sorts *below* any number — 20 digits is
    /// not a version anybody has, and ordering it consistently at the bottom
    /// beats panicking or truncating it.
    Text(String),
}

impl Component {
    fn parse(part: &str) -> Self {
        match part.parse::<u64>() {
            Ok(number) => Component::Number(number),
            Err(_) => Component::Text(part.to_owned()),
        }
    }

    /// The value a component that is not there has.
    const fn absent() -> Self {
        Component::Number(0)
    }
}

impl Ord for Component {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Component::Number(a), Component::Number(b)) => a.cmp(b),
            (Component::Text(a), Component::Text(b)) => a.cmp(b),
            // A release sorts above its own pre-releases: 1.0 over 1.0-rc1.
            (Component::Number(_), Component::Text(_)) => Ordering::Greater,
            (Component::Text(_), Component::Number(_)) => Ordering::Less,
        }
    }
}

impl PartialOrd for Component {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// An upstream version number.
///
/// Holds the text it was written as, so that `info` and `list` show a package
/// the way its own index does, and the components it compares by. Two versions
/// that compare equal may therefore print differently — `25.07.1` and `25.7.1`
/// are the same version with two spellings.
#[derive(Debug, Clone)]
pub struct Version {
    /// As written, for display.
    text: String,
    /// As compared, with trailing zero components removed so that equality and
    /// ordering agree and `1.2` hashes like `1.2.0`.
    parts: Vec<Component>,
}

impl Version {
    /// Read a version, or `None` if the text is not one.
    ///
    /// The only text that is not a version is empty text. Everything else
    /// orders sensibly against everything else, which is the point: these
    /// strings come from other people's repositories and this crate does not
    /// get to tell them how to number their releases.
    ///
    /// There is no `FromStr` on purpose. A version that will not parse is an
    /// error whose right form depends on where it came from — a file wants
    /// [`crate::error::Error::Parse`] with the path, an argument wants
    /// [`crate::error::Error::Usage`] — and only the caller knows which.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }

        let mut parts: Vec<Component> = text.split('.').map(Component::parse).collect();
        // Trailing zeros carry no information, and dropping them is what makes
        // `1.2 == 1.2.0` hold for `Eq` and `Hash` as well as for `Ord`.
        while parts.last() == Some(&Component::absent()) {
            parts.pop();
        }

        Some(Version {
            text: text.to_owned(),
            parts,
        })
    }

    /// The version as it was written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let width = self.parts.len().max(other.parts.len());
        for index in 0..width {
            let ordering = match (self.parts.get(index), other.parts.get(index)) {
                (Some(mine), Some(theirs)) => mine.cmp(theirs),
                (Some(mine), None) => mine.cmp(&Component::absent()),
                (None, Some(theirs)) => Component::absent().cmp(theirs),
                (None, None) => Ordering::Equal,
            };
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Equality is the ordering's, not the text's, and hashing follows equality -
// otherwise `25.07.1` and `25.7.1` would be equal and hash differently, which
// is the kind of thing that makes a HashMap lose entries.
impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.parts == other.parts
    }
}

impl Eq for Version {}

impl Hash for Version {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parts.hash(state);
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

// A version is a string everywhere it is written down, and it is validated on
// the way in: a metadata file saying `"version": ""` is a file to reject, not
// one to carry an unusable value out of.
impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Version::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("'{text}' is not a version")))
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn hash_of(version: &Version) -> u64 {
        let mut hasher = DefaultHasher::new();
        version.hash(&mut hasher);
        hasher.finish()
    }

    // Rule one: numeric components compare numerically.
    #[test]
    fn leading_zeros_do_not_make_a_different_version() {
        assert_eq!(v("25.07.1"), v("25.7.1"));
        assert_eq!(v("1.02"), v("1.2"));
    }

    #[test]
    fn numbers_compare_as_numbers_and_not_as_text() {
        // The comparison a string sort gets wrong.
        assert!(v("1.10") > v("1.9"));
        assert!(v("1.10.0") > v("1.9.9"));
        assert!(v("100") > v("99"));
    }

    // Rule two: a number sorts above text.
    #[test]
    fn a_release_is_above_its_pre_releases() {
        assert!(v("1.0") > v("1.0-rc1"));
        assert!(v("2.0") > v("2.0-beta"));
    }

    // Rule three: text compares as text.
    #[test]
    fn text_components_compare_as_text() {
        assert!(v("1.0-rc2") > v("1.0-rc1"));
        assert!(v("1.0-beta") < v("1.0-rc1"));
    }

    // Rule four: a missing component is zero.
    #[test]
    fn trailing_zeros_are_not_a_difference() {
        assert_eq!(v("1.2"), v("1.2.0"));
        assert_eq!(v("1.2"), v("1.2.0.0"));
        assert_eq!(v("1"), v("1.0.0"));
        assert!(v("1.2.1") > v("1.2"));
    }

    #[test]
    fn a_missing_component_beats_a_pre_release() {
        // "1.2" is "1.2.0", and a number is above text.
        assert!(v("1.2") > v("1.2.0-rc1"));
    }

    #[test]
    fn real_sepiaos_versions_sort_the_way_they_should() {
        let mut versions = [v("25.07.1"), v("1.2.6"), v("4.4.1"), v("23.1.0")];
        versions.sort();
        let sorted: Vec<&str> = versions.iter().map(Version::as_str).collect();
        assert_eq!(sorted, vec!["1.2.6", "4.4.1", "23.1.0", "25.07.1"]);
    }

    #[test]
    fn equal_versions_hash_alike() {
        // If this fails, a HashMap keyed on Version loses entries.
        assert_eq!(hash_of(&v("25.07.1")), hash_of(&v("25.7.1")));
        assert_eq!(hash_of(&v("1.2")), hash_of(&v("1.2.0")));
    }

    #[test]
    fn a_version_prints_as_it_was_written() {
        assert_eq!(v("25.07.1").to_string(), "25.07.1");
        assert_eq!(v("1.2.0").to_string(), "1.2.0");
        // Equal, and still printed the way each was written.
        assert_eq!(v("1.2.0"), v("1.2"));
    }

    #[test]
    fn only_empty_text_is_not_a_version() {
        assert!(Version::parse("").is_none());
        assert!(Version::parse("   ").is_none());
        assert!(Version::parse("1.2.6").is_some());
        assert!(Version::parse("nightly").is_some());
        assert!(Version::parse("2026-09-08").is_some());
    }

    #[test]
    fn surrounding_space_is_not_part_of_the_version() {
        assert_eq!(v(" 1.2.6 "), v("1.2.6"));
        assert_eq!(v(" 1.2.6 ").as_str(), "1.2.6");
    }

    #[test]
    fn a_version_survives_a_round_trip_through_json() {
        let version = v("25.07.1");
        let json = serde_json::to_string(&version).unwrap();
        assert_eq!(json, "\"25.07.1\"");
        let back: Version = serde_json::from_str(&json).unwrap();
        assert_eq!(back, version);
        assert_eq!(back.as_str(), "25.07.1");
    }

    #[test]
    fn an_empty_version_is_refused_on_the_way_in() {
        let result: Result<Version, _> = serde_json::from_str("\"\"");
        assert!(result.is_err());
    }

    #[test]
    fn a_number_too_long_for_a_u64_is_still_ordered() {
        // 20 digits overflows a u64, so the component is text - which by the
        // second rule sorts below any number. No release has such a version;
        // what matters is that it neither panics nor truncates, and that it
        // compares the same way every time.
        let huge = v("99999999999999999999.1");
        assert_eq!(huge, v("99999999999999999999.1"));
        // Text sorts below any number, so it is below every ordinary version -
        // including 0.9, whose first component is the number 0.
        assert!(huge < v("1.0"));
        assert!(huge < v("0.9-rc1"));
        // Against other text it is ordered as text.
        assert!(huge > v("111111111111111111111.1"));
    }
}
