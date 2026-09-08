/*
  index.rs

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

//! The index a source publishes.
//!
//! Carries the metadata of every package it lists, so that the client never
//! downloads a package to find out what it is — searching, comparing versions
//! and resolving dependencies are all answered from here.
//!
//! It carries **two digests per version**, and they are not interchangeable:
//! `sha256` is of the package as published, checked before the archive is
//! opened at all; `payload_sha256` is of the `data.tar.gz` inside it, and is
//! the digest that package's own `metadata.json` carries. Neither name is
//! shortened. Confusing them is a verification that passes while checking
//! nothing.

use serde::{Deserialize, Serialize};

use crate::model::metadata::{Dependency, Sha256};
use crate::model::name::{PackageName, SourceName, Target};
use crate::model::version::Version;

/// What a source publishes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    /// The source's own name, which is where `add-source` learns it.
    pub name: SourceName,
    /// When the source last rebuilt this index, in seconds since the epoch.
    pub updated: u64,
    /// Every package the source offers.
    pub packages: Vec<IndexPackage>,
}

/// One package in an index, with every version of it the source knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexPackage {
    /// What the package is called.
    pub name: PackageName,
    /// A sentence or two, shown by `search` and `list`.
    pub description: String,
    /// Every version, in no particular order — the index accumulates them.
    pub versions: Vec<IndexVersion>,
}

/// One version of one package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexVersion {
    /// The version of the software.
    pub version: Version,
    /// What it was built for.
    pub target: Target,
    /// Where the package can be downloaded.
    ///
    /// Text rather than a checked type on purpose. A URL that is not `https`
    /// is refused by the transport, where refusing costs one package; refusing
    /// it here would throw away a whole index over one bad entry.
    pub url: String,
    /// How big the package is, in bytes.
    ///
    /// Here rather than discovered by asking the server, because both things
    /// that need it happen **before** anything is fetched: the plan `install`
    /// shows says what it is about to download, and the check that the card has
    /// room refuses before filling the root filesystem rather than after. A
    /// scan can read it off a release listing without downloading the package,
    /// which is the property the whole index format is built around.
    pub bytes: u64,
    /// The digest of the package as published, checked before it is opened.
    pub sha256: Sha256,
    /// The digest of the `data.tar.gz` inside it, checked before it is
    /// unpacked.
    pub payload_sha256: Sha256,
    /// What has to be installed alongside it.
    pub dependencies: Vec<Dependency>,
}

impl Index {
    /// The entry for a package, if this source offers it.
    #[must_use]
    pub fn package(&self, name: &PackageName) -> Option<&IndexPackage> {
        self.packages.iter().find(|package| &package.name == name)
    }

    /// The newest version of a package that was built for this target.
    ///
    /// `None` if the source does not offer the package at all, or offers it
    /// only for other targets — which are different answers and the caller has
    /// to tell them apart, so it asks [`Index::package`] first.
    #[must_use]
    pub fn newest(&self, name: &PackageName, target: &Target) -> Option<&IndexVersion> {
        self.package(name)?.newest(target)
    }
}

impl IndexPackage {
    /// The newest version built for this target.
    #[must_use]
    pub fn newest(&self, target: &Target) -> Option<&IndexVersion> {
        self.versions
            .iter()
            .filter(|entry| &entry.target == target)
            .max_by(|left, right| left.version.cmp(&right.version))
    }

    /// The oldest version built for this target that is not older than `floor`.
    ///
    /// The **oldest**, where [`IndexPackage::newest`] takes the newest, and the
    /// difference is the whole of what a dependency means. `dependencies` names
    /// a floor — that version or a newer one — so the least that satisfies it is
    /// what an install should bring in. Taking the newest instead would drag
    /// every dependency to its latest release on the strength of one package
    /// asking for an old one.
    #[must_use]
    pub fn lowest_from(&self, target: &Target, floor: &Version) -> Option<&IndexVersion> {
        self.versions
            .iter()
            .filter(|entry| &entry.target == target && &entry.version >= floor)
            .min_by(|left, right| left.version.cmp(&right.version))
    }

    /// The targets this package is offered for, in the order the index lists
    /// them, for the error a user sees when theirs is not among them.
    #[must_use]
    pub fn targets(&self) -> Vec<&Target> {
        let mut seen: Vec<&Target> = Vec::new();
        for entry in &self.versions {
            if !seen.contains(&&entry.target) {
                seen.push(&entry.target);
            }
        }
        seen
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    /// The example from `docs/dev/DESIGN.md`, with the digests filled in — the
    /// document elides them as `…`, and everything else is verbatim.
    fn design_example() -> String {
        format!(
            r#"{{
  "name": "sepia",
  "updated": 1757260800,
  "packages": [
    {{
      "name": "helix",
      "description": "The Helix editor, with its tree-sitter grammars.",
      "versions": [
        {{
          "version": "25.07.1",
          "target": "aarch64-musl",
          "url": "https://example.test/helix-25.07.1-aarch64-musl.tar.gz",
          "bytes": 16148070,
          "sha256": "{package}",
          "payload_sha256": "{payload}",
          "dependencies": [ {{ "name": "llvm-runtime", "version": "23.1.0" }} ]
        }}
      ]
    }}
  ]
}}"#,
            package = digest(0xaa),
            payload = digest(0xbb)
        )
    }

    fn parsed() -> Index {
        serde_json::from_str(&design_example()).unwrap()
    }

    #[test]
    fn the_documented_example_parses() {
        let index = parsed();
        assert_eq!(index.name.as_str(), "sepia");
        assert_eq!(index.updated, 1_757_260_800);
        assert_eq!(index.packages.len(), 1);
        assert_eq!(index.packages[0].name.as_str(), "helix");
        assert_eq!(index.packages[0].versions.len(), 1);
        // What the plan shows and what the space check works from, both of
        // which happen before a byte is fetched.
        assert_eq!(index.packages[0].versions[0].bytes, 16_148_070);
    }

    #[test]
    fn the_documented_example_round_trips() {
        let index = parsed();
        let written = serde_json::to_string(&index).unwrap();
        let again: Index = serde_json::from_str(&written).unwrap();
        assert_eq!(index, again);
    }

    #[test]
    fn the_two_digests_are_kept_apart() {
        // The one thing this format must not get wrong: the package's digest
        // and its payload's are different values with different jobs, and a
        // reader that swapped them would verify nothing while appearing to.
        let index = parsed();
        let entry = &index.packages[0].versions[0];
        assert_eq!(entry.sha256.as_str(), digest(0xaa));
        assert_eq!(entry.payload_sha256.as_str(), digest(0xbb));
        assert_ne!(entry.sha256, entry.payload_sha256);
    }

    #[test]
    fn a_misspelled_field_is_reported() {
        let misspelled = design_example().replace(r#""payload_sha256""#, r#""payload_sha""#);
        let message = serde_json::from_str::<Index>(&misspelled)
            .unwrap_err()
            .to_string();
        assert!(message.contains("payload_sha"), "{message}");
    }

    /// An index with several versions and two targets, which is what a real
    /// one looks like after a few releases.
    fn several_versions() -> Index {
        let entry = |version: &str, target: &str| {
            format!(
                r#"{{
                    "version": "{version}",
                    "target": "{target}",
                    "url": "https://example.test/p.tar.gz",
                    "bytes": 1024,
                    "sha256": "{d}",
                    "payload_sha256": "{d}",
                    "dependencies": []
                }}"#,
                d = digest(0x11)
            )
        };
        let text = format!(
            r#"{{
              "name": "sepia",
              "updated": 1,
              "packages": [
                {{
                  "name": "helix",
                  "description": "",
                  "versions": [{a}, {b}, {c}, {d}]
                }}
              ]
            }}"#,
            // Deliberately not in order, and not all for one target.
            a = entry("23.1.0", "aarch64-musl"),
            b = entry("25.07.1", "aarch64-musl"),
            c = entry("26.0.0", "x86_64-musl"),
            d = entry("4.4.1", "aarch64-musl"),
        );
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn the_newest_version_for_a_target_is_the_one_chosen() {
        let index = several_versions();
        let name = PackageName::parse("helix").unwrap();
        let target = Target::parse("aarch64-musl").unwrap();
        let newest = index.newest(&name, &target).unwrap();
        // 25.07.1 beats 23.1.0 and 4.4.1, and 26.0.0 is for another machine.
        assert_eq!(newest.version.as_str(), "25.07.1");
        assert_eq!(newest.target.as_str(), "aarch64-musl");
    }

    #[test]
    fn the_oldest_version_that_satisfies_a_floor_is_the_one_a_dependency_gets() {
        // 23.1.0 and 25.07.1 both satisfy ">= 5.0"; a dependency takes the
        // lower, or one package asking for an old version would drag every
        // other package to its newest release.
        let index = several_versions();
        let package = index
            .package(&PackageName::parse("helix").unwrap())
            .unwrap();
        let target = Target::parse("aarch64-musl").unwrap();

        let chosen = package
            .lowest_from(&target, &Version::parse("5.0").unwrap())
            .unwrap();
        assert_eq!(chosen.version.as_str(), "23.1.0");

        // A floor below everything takes the oldest there is.
        let oldest = package
            .lowest_from(&target, &Version::parse("0").unwrap())
            .unwrap();
        assert_eq!(oldest.version.as_str(), "4.4.1");

        // And a floor above everything is not satisfiable at all.
        assert!(
            package
                .lowest_from(&target, &Version::parse("99.0").unwrap())
                .is_none()
        );
        // 26.0.0 would satisfy it, and is for another machine.
        assert!(
            package
                .lowest_from(
                    &Target::parse("x86_64-musl").unwrap(),
                    &Version::parse("26.0.0").unwrap()
                )
                .is_some()
        );
    }

    #[test]
    fn a_package_for_another_target_only_is_not_a_package_for_this_one() {
        let index = several_versions();
        let name = PackageName::parse("helix").unwrap();
        let elsewhere = Target::parse("riscv64-musl").unwrap();
        assert!(index.newest(&name, &elsewhere).is_none());
        // But the package does exist, which is a different message to the user.
        assert!(index.package(&name).is_some());
    }

    #[test]
    fn a_package_that_is_not_there_is_not_there() {
        let index = several_versions();
        let missing = PackageName::parse("nothing").unwrap();
        assert!(index.package(&missing).is_none());
    }

    #[test]
    fn the_targets_on_offer_can_be_listed_for_an_error_message() {
        let index = several_versions();
        let name = PackageName::parse("helix").unwrap();
        let targets: Vec<&str> = index
            .package(&name)
            .unwrap()
            .targets()
            .into_iter()
            .map(Target::as_str)
            .collect();
        assert_eq!(targets, vec!["aarch64-musl", "x86_64-musl"]);
    }
}
