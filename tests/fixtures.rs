/*
  fixtures.rs

  Created on 2026-09-08 by Thomas Bonk <thomas@meandmymac.de>
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

//! That the fixtures build, and are what they claim to be.
//!
//! Nothing is installed here — there is nothing to install with yet. This is
//! the check that the three shapes every later step needs exist, are packages
//! `create` actually produced, and hold together: the digest beside a package
//! describes the package, and the metadata inside it describes the payload
//! inside it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use spm::ops::create::{METADATA, PAYLOAD};
use support::{Package, dependent, plain, rival};

fn workshop() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

#[test]
fn the_three_fixtures_build() {
    let work = workshop();
    for built in [
        plain(work.path()),
        dependent(work.path()),
        rival(work.path()),
    ] {
        assert!(built.package.exists(), "{} has no package", built.name);
        assert!(built.metadata.exists(), "{} has no metadata", built.name);
        assert!(built.sums.exists(), "{} has no SHA256SUMS", built.name);
        assert!(built.bytes > 0);
    }
}

#[test]
fn every_fixture_verifies() {
    let work = workshop();
    plain(work.path()).verify();
    dependent(work.path()).verify();
    rival(work.path()).verify();
}

#[test]
fn a_fixture_is_a_package_and_not_something_shaped_like_one() {
    let work = workshop();
    let built = plain(work.path());

    // Exactly the two members, and the payload holds the staged tree.
    assert!(!built.member(PAYLOAD).is_empty());
    assert!(!built.member(METADATA).is_empty());

    let metadata = built.packed_metadata();
    assert_eq!(metadata.name.as_str(), "plain");
    assert_eq!(metadata.version.as_str(), "1.0.0");
    assert_eq!(metadata.target.as_str(), "aarch64-musl");
}

#[test]
fn the_dependent_fixture_says_what_it_needs() {
    let work = workshop();
    let metadata = dependent(work.path()).packed_metadata();

    assert_eq!(metadata.dependencies.len(), 1);
    assert_eq!(metadata.dependencies[0].name.as_str(), "plain");
    assert_eq!(metadata.dependencies[0].version.as_str(), "1.0.0");
}

#[test]
fn the_rival_fixture_really_does_clash_with_plain() {
    // The conflict later steps test against: two packages, one file.
    let work = workshop();
    let one = plain(work.path());
    let other = rival(work.path());

    let contested = "usr/bin/plain";
    assert!(
        payload_holds(&one, contested),
        "plain does not ship {contested}"
    );
    assert!(
        payload_holds(&other, contested),
        "rival does not ship {contested}"
    );
    assert_ne!(one.sha256, other.sha256);
}

#[test]
fn a_fixture_can_be_asked_for_a_version_and_a_target() {
    let work = workshop();
    let built = Package::named("plain")
        .version("2.5.0")
        .target("x86_64-musl")
        .build(work.path());

    let metadata = built.packed_metadata();
    assert_eq!(metadata.version.as_str(), "2.5.0");
    assert_eq!(metadata.target.as_str(), "x86_64-musl");
    assert!(
        built
            .package
            .file_name()
            .expect("a package has a name")
            .display()
            .to_string()
            .ends_with("plain-2.5.0-x86_64-musl.tar.gz")
    );
    built.verify();
}

#[test]
fn two_builds_of_one_fixture_are_the_same_package() {
    // The fixtures inherit create's determinism, which is what lets a test
    // compare digests rather than contents.
    let one = workshop();
    let two = workshop();
    assert_eq!(plain(one.path()).sha256, plain(two.path()).sha256);
}

/// Whether a built package's payload holds this path.
fn payload_holds(built: &support::Built, path: &str) -> bool {
    use std::io::Cursor;
    let payload = built.member(PAYLOAD);
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(payload)));
    archive
        .entries()
        .expect("a payload is a tar")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .path()
                .map(|found| found.display().to_string() == path)
                .unwrap_or(false)
        })
}
