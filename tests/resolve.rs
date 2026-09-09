/*
  resolve.rs

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

//! Turning what somebody typed into one package, in one source, at one
//! version.
//!
//! The case worth being careful about is the ambiguous one. A package name
//! identifies a package *within a source*, so two sources may both offer
//! `helix` — and picking one would mean installing something other than what
//! was meant. Refusing and listing them is the only honest answer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use spm::error::Error;
use spm::model::name::{PackageRef, SourceName, Target};
use spm::model::version::Version;
use spm::ops::resolve::{candidates, find, select};
use spm::store::Store;
use spm::store::config::{Source, Sources};
use spm::store::index;
use support::{Built, Package, index_of};

fn store_with(device: &tempfile::TempDir) -> Store {
    Store::at(device.path())
}

fn target(text: &str) -> Target {
    Target::parse(text).unwrap()
}

fn reference(text: &str) -> PackageRef {
    PackageRef::parse(text).unwrap()
}

/// Configure a source and give it an index listing these packages.
fn source_offering(store: &Store, source: &str, packages: &[&Built]) {
    let mut sources = Sources::load(store).unwrap();
    sources.insert(Source {
        name: SourceName::parse(source).unwrap(),
        url: format!("https://{source}.test/index.json"),
        is_default: false,
        key: support::test_public_key(),
    });
    sources.save(store).unwrap();

    let entries: Vec<(&Built, String)> = packages
        .iter()
        .map(|built| (*built, format!("https://{source}.test/p.tar.gz")))
        .collect();
    let parsed = serde_json::from_str(&index_of(source, &entries)).unwrap();
    index::write(store, &SourceName::parse(source).unwrap(), &parsed).unwrap();
}

#[test]
fn a_name_one_source_offers_resolves_to_that_source() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix").build(work.path())],
    );

    let found = find(&store, &reference("helix")).unwrap();

    assert_eq!(found.source.as_str(), "sepia");
    assert_eq!(found.package.name.as_str(), "helix");
}

#[test]
fn a_name_two_sources_offer_is_refused_and_listed_qualified() {
    // The form the user has to type back is the form they are shown.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    let built = Package::named("helix").build(work.path());
    source_offering(&store, "sepia", &[&built]);
    source_offering(&store, "local", &[&built]);

    match find(&store, &reference("helix")) {
        Err(Error::Ambiguous { name, candidates }) => {
            assert_eq!(name, "helix");
            assert_eq!(candidates, vec!["local/helix", "sepia/helix"]);
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[test]
fn a_qualified_name_says_which_source_it_means() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    let built = Package::named("helix").build(work.path());
    source_offering(&store, "sepia", &[&built]);
    source_offering(&store, "local", &[&built]);

    let found = find(&store, &reference("local/helix")).unwrap();

    assert_eq!(found.source.as_str(), "local");
}

#[test]
fn a_name_nothing_offers_is_not_found() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix").build(work.path())],
    );

    match find(&store, &reference("nothing")) {
        Err(Error::PackageNotFound { name }) => assert_eq!(name, "nothing"),
        other => panic!("expected PackageNotFound, got {other:?}"),
    }
}

#[test]
fn naming_a_source_that_is_not_configured_is_a_different_answer() {
    // "no such source" and "that source does not have it" are different
    // problems with different fixes.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix").build(work.path())],
    );

    match find(&store, &reference("nowhere/helix")) {
        Err(Error::SourceNotFound { reference }) => assert_eq!(reference.as_str(), "nowhere"),
        other => panic!("expected SourceNotFound, got {other:?}"),
    }

    source_offering(
        &store,
        "local",
        &[&Package::named("grit").build(work.path())],
    );
    match find(&store, &reference("local/helix")) {
        Err(Error::PackageNotFound { name }) => assert_eq!(name, "local/helix"),
        other => panic!("expected PackageNotFound, got {other:?}"),
    }
}

#[test]
fn the_newest_version_for_this_device_is_the_one_chosen() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    let old = Package::named("helix").version("23.1.0").build(work.path());
    let new = Package::named("helix")
        .version("25.07.1")
        .build(work.path());
    let mid = Package::named("helix").version("4.4.1").build(work.path());
    source_offering(&store, "sepia", &[&old, &new, &mid]);

    let chosen = select(&store, &reference("helix"), &target("aarch64-musl"), None).unwrap();

    assert_eq!(chosen.version.version.as_str(), "25.07.1");
    assert_eq!(chosen.source.as_str(), "sepia");
}

#[test]
fn a_version_can_be_asked_for_by_name() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    let old = Package::named("helix").version("23.1.0").build(work.path());
    let new = Package::named("helix")
        .version("25.07.1")
        .build(work.path());
    source_offering(&store, "sepia", &[&old, &new]);

    let wanted = Version::parse("23.1.0").unwrap();
    let chosen = select(
        &store,
        &reference("helix"),
        &target("aarch64-musl"),
        Some(&wanted),
    )
    .unwrap();

    assert_eq!(chosen.version.version.as_str(), "23.1.0");
}

#[test]
fn a_version_that_is_not_offered_says_so() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix").build(work.path())],
    );

    let wanted = Version::parse("9.9.9").unwrap();
    match select(
        &store,
        &reference("helix"),
        &target("aarch64-musl"),
        Some(&wanted),
    ) {
        Err(Error::VersionNotFound { package, version }) => {
            assert_eq!(package, "helix");
            assert_eq!(version, "9.9.9");
        }
        other => panic!("expected VersionNotFound, got {other:?}"),
    }
}

#[test]
fn a_package_built_only_for_another_machine_says_which_machines_it_is_for() {
    // Not "no such package": it exists, and the fix is a different one.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    let elsewhere = Package::named("helix")
        .target("x86_64-musl")
        .build(work.path());
    source_offering(&store, "sepia", &[&elsewhere]);

    match select(&store, &reference("helix"), &target("aarch64-musl"), None) {
        Err(Error::TargetNotAvailable {
            package,
            target,
            available,
        }) => {
            assert_eq!(package, "helix");
            assert_eq!(target, "aarch64-musl");
            assert_eq!(available, "x86_64-musl");
        }
        other => panic!("expected TargetNotAvailable, got {other:?}"),
    }
}

#[test]
fn a_version_that_exists_for_another_machine_is_not_a_missing_version() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    let elsewhere = Package::named("helix")
        .version("25.07.1")
        .target("x86_64-musl")
        .build(work.path());
    source_offering(&store, "sepia", &[&elsewhere]);

    let wanted = Version::parse("25.07.1").unwrap();
    match select(
        &store,
        &reference("helix"),
        &target("aarch64-musl"),
        Some(&wanted),
    ) {
        Err(Error::TargetNotAvailable { available, .. }) => {
            assert_eq!(available, "x86_64-musl");
        }
        other => panic!("expected TargetNotAvailable, got {other:?}"),
    }
}

#[test]
fn a_source_that_has_never_been_updated_offers_nothing_yet() {
    // Not an error: it is an `update` waiting to happen.
    let device = tempfile::tempdir().unwrap();
    let store = store_with(&device);
    let mut sources = Sources::default();
    sources.insert(Source {
        name: SourceName::parse("sepia").unwrap(),
        url: "https://sepia.test/index.json".to_owned(),
        is_default: true,
        key: support::test_public_key(),
    });
    sources.save(&store).unwrap();

    assert!(
        candidates(
            &store,
            &spm::model::name::PackageName::parse("helix").unwrap()
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn this_device_knows_what_it_is() {
    // Whatever machine the tests run on, it has a name and it is a name.
    let here = Target::current();
    assert!(!here.as_str().is_empty());
    assert!(here.as_str().contains('-'));
}
