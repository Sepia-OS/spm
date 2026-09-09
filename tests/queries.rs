/*
  queries.rs

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

//! `search`, `info` and `list`.
//!
//! All three read the copy of the index that is on the device and nothing
//! else, so what they say is as current as the last `update` and no more.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use spm::error::Error;
use spm::model::installed::{Reason, Record};
use spm::model::name::{PackageRef, SourceName, Target};
use spm::model::version::Version;
use spm::ops::query::{info, list, search};
use spm::store::Store;
use spm::store::config::{Source, Sources};
use spm::store::db::Database;
use spm::store::index;
use std::collections::BTreeMap;
use support::{Built, Package, index_of};

fn target() -> Target {
    Target::parse("aarch64-musl").unwrap()
}

fn reference(text: &str) -> PackageRef {
    PackageRef::parse(text).unwrap()
}

fn source_offering(store: &Store, source: &str, packages: &[&Built]) {
    let mut sources = Sources::load(store).unwrap();
    sources.insert(Source {
        name: SourceName::parse(source).unwrap(),
        url: format!("https://{source}.test/index.json"),
        is_default: false,
    });
    sources.save(store).unwrap();

    let entries: Vec<(&Built, String)> = packages
        .iter()
        .map(|built| (*built, format!("https://{source}.test/p.tar.gz")))
        .collect();
    let parsed = serde_json::from_str(&index_of(source, &entries)).unwrap();
    index::write(store, &SourceName::parse(source).unwrap(), &parsed).unwrap();
}

fn mark_installed(store: &Store, built: &Built, source: &str) {
    let record = Record {
        metadata: built.packed_metadata(),
        source: SourceName::parse(source).unwrap(),
        reason: Reason::Explicit,
        installed_at: 1,
        files: Vec::new(),
        digests: BTreeMap::new(),
    };
    let db = Database::new(store);
    db.begin(&record).unwrap();
    db.commit(&record.metadata.name).unwrap();
}

#[test]
fn a_search_finds_a_package_by_part_of_its_name() {
    // What somebody half-remembers is enough.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    source_offering(
        &store,
        "sepia",
        &[
            &Package::named("helix").build(work.path()),
            &Package::named("grit").build(work.path()),
        ],
    );

    let found = search(&store, &target(), "hel").unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name.as_str(), "helix");
    assert_eq!(found[0].newest.as_ref().unwrap().as_str(), "1.0.0");
}

#[test]
fn a_search_ignores_case() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix").build(work.path())],
    );

    assert_eq!(search(&store, &target(), "HEL").unwrap().len(), 1);
    assert_eq!(search(&store, &target(), "LiX").unwrap().len(), 1);
}

#[test]
fn a_search_that_matches_nothing_comes_back_empty() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix").build(work.path())],
    );

    assert!(search(&store, &target(), "zzz").unwrap().is_empty());
}

#[test]
fn a_package_two_sources_offer_is_shown_qualified() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let built = Package::named("helix").build(work.path());
    source_offering(&store, "sepia", &[&built]);
    source_offering(&store, "local", &[&built]);

    let found = search(&store, &target(), "helix").unwrap();

    assert_eq!(found.len(), 2);
    assert!(
        found.iter().all(|line| line.qualify),
        "both need qualifying"
    );
}

#[test]
fn a_package_built_for_another_machine_is_shown_without_a_version() {
    // It exists, just not for this device - which is worth saying rather than
    // hiding.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix")
            .target("x86_64-musl")
            .build(work.path())],
    );

    let found = search(&store, &target(), "helix").unwrap();

    assert_eq!(found.len(), 1);
    assert!(found[0].newest.is_none());
}

#[test]
fn an_installed_package_is_marked_and_only_for_the_source_it_came_from() {
    // The same name from two sources is two packages, and only one of them is
    // on the device.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let built = Package::named("helix").build(work.path());
    source_offering(&store, "sepia", &[&built]);
    source_offering(&store, "local", &[&built]);
    mark_installed(&store, &built, "sepia");

    let found = search(&store, &target(), "helix").unwrap();
    let sepia = found.iter().find(|l| l.source.as_str() == "sepia").unwrap();
    let local = found.iter().find(|l| l.source.as_str() == "local").unwrap();

    assert_eq!(sepia.installed.as_ref().unwrap().as_str(), "1.0.0");
    assert!(local.installed.is_none());
}

#[test]
fn a_listing_can_be_narrowed_to_what_is_installed() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let one = Package::named("helix").build(work.path());
    let two = Package::named("grit").build(work.path());
    source_offering(&store, "sepia", &[&one, &two]);
    mark_installed(&store, &one, "sepia");

    assert_eq!(list(&store, &target(), false, None).unwrap().len(), 2);
    let installed = list(&store, &target(), true, None).unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].name.as_str(), "helix");
}

#[test]
fn a_listing_can_be_narrowed_to_one_source() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix").build(work.path())],
    );
    source_offering(
        &store,
        "local",
        &[&Package::named("grit").build(work.path())],
    );

    let only = SourceName::parse("local").unwrap();
    let found = list(&store, &target(), false, Some(&only)).unwrap();

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name.as_str(), "grit");
}

#[test]
fn listing_from_a_source_that_is_not_configured_says_so() {
    let device = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let nowhere = SourceName::parse("nowhere").unwrap();

    match list(&store, &target(), false, Some(&nowhere)) {
        Err(Error::SourceNotFound { reference }) => assert_eq!(reference.as_str(), "nowhere"),
        other => panic!("expected SourceNotFound, got {other:?}"),
    }
}

#[test]
fn a_listing_is_ordered_by_name_and_then_by_source() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let helix = Package::named("helix").build(work.path());
    let grit = Package::named("grit").build(work.path());
    source_offering(&store, "sepia", &[&helix, &grit]);
    source_offering(&store, "local", &[&helix]);

    let found = list(&store, &target(), false, None).unwrap();
    let seen: Vec<String> = found
        .iter()
        .map(|line| format!("{}/{}", line.source, line.name))
        .collect();

    assert_eq!(
        seen,
        vec!["sepia/grit", "local/helix", "sepia/helix"],
        "not ordered by name then source"
    );
}

#[test]
fn info_shows_the_newest_version_and_the_others_on_offer() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    source_offering(
        &store,
        "sepia",
        &[
            &Package::named("helix").version("23.1.0").build(work.path()),
            &Package::named("helix")
                .version("25.07.1")
                .build(work.path()),
            &Package::named("helix").version("4.4.1").build(work.path()),
        ],
    );

    let details = info(&store, &reference("helix"), &target(), None).unwrap();

    assert_eq!(details.len(), 1);
    assert_eq!(details[0].version.version.as_str(), "25.07.1");
    let versions: Vec<&str> = details[0].versions.iter().map(Version::as_str).collect();
    // Newest first: 25 above 23 above 4, which a string sort would get wrong.
    assert_eq!(versions, vec!["25.07.1", "23.1.0", "4.4.1"]);
}

#[test]
fn info_can_be_asked_for_an_older_version() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    source_offering(
        &store,
        "sepia",
        &[
            &Package::named("helix").version("23.1.0").build(work.path()),
            &Package::named("helix")
                .version("25.07.1")
                .build(work.path()),
        ],
    );

    let wanted = Version::parse("23.1.0").unwrap();
    let details = info(&store, &reference("helix"), &target(), Some(&wanted)).unwrap();

    assert_eq!(details[0].version.version.as_str(), "23.1.0");
}

#[test]
fn info_shows_every_source_that_offers_a_package() {
    // Unlike `install`, this is not ambiguous: telling somebody about all of
    // them is the answer to the question they asked.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let built = Package::named("helix").build(work.path());
    source_offering(&store, "sepia", &[&built]);
    source_offering(&store, "local", &[&built]);

    let details = info(&store, &reference("helix"), &target(), None).unwrap();

    assert_eq!(details.len(), 2);
    assert!(details.iter().all(|detail| detail.qualify));
}

#[test]
fn info_about_a_package_nothing_offers_says_so() {
    let device = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());

    match info(&store, &reference("helix"), &target(), None) {
        Err(Error::PackageNotFound { name }) => assert_eq!(name, "helix"),
        other => panic!("expected PackageNotFound, got {other:?}"),
    }
}

#[test]
fn info_shows_what_a_package_depends_on() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    source_offering(
        &store,
        "sepia",
        &[&Package::named("helix")
            .depends_on("llvm-runtime", "23.1.0")
            .build(work.path())],
    );

    let details = info(&store, &reference("helix"), &target(), None).unwrap();

    assert_eq!(details[0].version.dependencies.len(), 1);
    assert_eq!(
        details[0].version.dependencies[0].name.as_str(),
        "llvm-runtime"
    );
}
