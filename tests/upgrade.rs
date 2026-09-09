/*
  upgrade.rs

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

//! `upgrade`, against devices that were really installed onto.
//!
//! An upgrade is an install of a set somebody did not type, so everything from
//! the plan onwards is `install`'s and is tested there. What is tested here is
//! the part that is this command's: which packages move, which are left where
//! they are, and what the device is told about the difference.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use spm::error::Error;
use spm::model::index::Index;
use spm::model::installed::Reason;
use spm::model::name::{PackageName, PackageRef, SourceName, Target};
use spm::ops::install::install;
use spm::ops::upgrade::{Report, upgrade};
use spm::store::Store;
use spm::store::config::{Source, Sources};
use spm::store::db::Database;
use spm::store::index;
use support::net::Fake;
use support::{Built, Package, index_of};

/// A device, a place to build packages, and a place to serve them from.
struct Fixture {
    device: tempfile::TempDir,
    work: tempfile::TempDir,
    served: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Fixture {
            device: tempfile::tempdir().unwrap(),
            work: tempfile::tempdir().unwrap(),
            served: tempfile::tempdir().unwrap(),
        }
    }

    fn store(&self) -> Store {
        Store::at(self.device.path())
    }

    fn transport(&self) -> Fake {
        Fake::serving(self.served.path())
    }

    fn build(&self, package: Package) -> Built {
        package.build(self.work.path())
    }

    fn root(&self) -> &Path {
        self.device.path()
    }
}

fn target() -> Target {
    Target::parse("aarch64-musl").unwrap()
}

fn name(text: &str) -> PackageName {
    PackageName::parse(text).unwrap()
}

/// Configure a source, serve its packages, and write the index describing them.
fn publish(store: &Store, fake: &Fake, source: &str, packages: &[&Built]) {
    let mut sources = Sources::load(store).unwrap();
    sources.insert(Source {
        name: SourceName::parse(source).unwrap(),
        url: fake.url_for(&format!("{source}/index.json")),
        is_default: false,
        key: support::test_public_key(),
    });
    sources.save(store).unwrap();

    let entries: Vec<(&Built, String)> = packages
        .iter()
        .map(|built| {
            let served = format!("{source}/{}", built.package.file_name().unwrap().display());
            let url = fake.serve_file(&served, &built.package);
            (*built, url)
        })
        .collect();

    let parsed: Index = serde_json::from_str(&index_of(source, &entries)).unwrap();
    index::write(store, &SourceName::parse(source).unwrap(), &parsed).unwrap();
}

fn installing(store: &Store, fake: &Fake, package: &str) {
    install(
        store,
        fake,
        &PackageRef::parse(package).unwrap(),
        &target(),
        None,
        false,
    )
    .unwrap();
}

fn upgrading(store: &Store, fake: &Fake) -> spm::error::Result<Report> {
    upgrade(store, fake, &target(), None, false)
}

/// The version of each installed package, so a whole device can be compared in
/// one assertion.
fn versions(store: &Store) -> BTreeMap<String, String> {
    Database::new(store)
        .all()
        .unwrap()
        .into_iter()
        .map(|record| {
            (
                record.metadata.name.as_str().to_owned(),
                record.metadata.version.as_str().to_owned(),
            )
        })
        .collect()
}

/// Every file under a directory, with its contents.
fn snapshot(at: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut found = BTreeMap::new();
    walk(at, at, &mut found);
    found
}

fn walk(root: &Path, at: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
    let Ok(listing) = fs::read_dir(at) else {
        return;
    };
    for entry in listing {
        let path = entry.unwrap().path();
        let metadata = fs::symlink_metadata(&path).unwrap();
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        if metadata.is_dir() {
            into.insert(relative, Vec::new());
            walk(root, &path, into);
        } else {
            into.insert(relative, fs::read(&path).unwrap_or_default());
        }
    }
}

#[test]
fn an_installed_package_moves_to_the_newest_version_its_source_offers() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(Package::named("plain").version("1.0.0"));
    publish(&store, &fake, "sepia", &[&old]);
    installing(&store, &fake, "plain");

    // The source publishes a newer one, and an `update` brings it in - which
    // here is the index being rewritten.
    let new = fixture.build(Package::named("plain").version("2.0.0"));
    publish(&store, &fake, "sepia", &[&old, &new]);

    let report = upgrading(&store, &fake).unwrap();

    assert!(report.changed);
    report.outcome().unwrap();
    assert_eq!(
        versions(&store).get("plain").map(String::as_str),
        Some("2.0.0")
    );
    assert!(fixture.root().join("usr/bin/plain").exists());
}

#[test]
fn a_device_that_is_already_current_is_left_alone() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);
    installing(&store, &fake, "plain");
    let before = snapshot(fixture.root());

    let report = upgrading(&store, &fake).unwrap();

    assert!(!report.changed);
    assert!(report.is_empty());
    report.outcome().unwrap();
    assert_eq!(snapshot(fixture.root()), before);
}

#[test]
fn one_package_that_cannot_move_leaves_the_others_free_to() {
    // The rule the command exists to keep: one unsatisfiable package is a
    // reason to leave that package alone, not to leave the device unpatched.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();

    let one = fixture.build(Package::named("one").version("1.0.0"));
    let two = fixture.build(Package::named("two").version("1.0.0"));
    let three = fixture.build(Package::named("three").version("1.0.0"));
    publish(&store, &fake, "sepia", &[&one, &two, &three]);
    for each in ["one", "two", "three"] {
        installing(&store, &fake, each);
    }

    // Newer versions of all three, but `two` now needs something no source has.
    let one_new = fixture.build(Package::named("one").version("2.0.0"));
    let two_new = fixture.build(
        Package::named("two")
            .version("2.0.0")
            .depends_on("nowhere", "1.0.0"),
    );
    let three_new = fixture.build(Package::named("three").version("2.0.0"));
    publish(
        &store,
        &fake,
        "sepia",
        &[&one, &two, &three, &one_new, &two_new, &three_new],
    );

    let report = upgrading(&store, &fake).unwrap();

    let moved = versions(&store);
    assert_eq!(moved.get("one").map(String::as_str), Some("2.0.0"));
    assert_eq!(moved.get("three").map(String::as_str), Some("2.0.0"));
    assert_eq!(
        moved.get("two").map(String::as_str),
        Some("1.0.0"),
        "the one that could not be upgraded should not have been"
    );

    assert_eq!(report.held.len(), 1);
    let held = &report.held[0];
    assert_eq!(held.name.as_str(), "two");
    assert_eq!(held.installed.as_str(), "1.0.0");
    assert_eq!(held.offered.as_str(), "2.0.0");
    assert!(held.why.contains("nowhere"), "{}", held.why);

    // And the exit says the picture is not the whole one.
    match report.outcome() {
        Err(error @ Error::UpgradeIncomplete { .. }) => {
            assert_eq!(error.exit_code(), 8);
            let text = error.to_string();
            assert!(text.contains("two 1.0.0"), "{text}");
            assert!(text.contains("of 3 packages"), "{text}");
        }
        other => panic!("expected UpgradeIncomplete, got {other:?}"),
    }
}

#[test]
fn upgrading_one_package_touches_only_that_package() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let one = fixture.build(Package::named("one").version("1.0.0"));
    let two = fixture.build(Package::named("two").version("1.0.0"));
    publish(&store, &fake, "sepia", &[&one, &two]);
    installing(&store, &fake, "one");
    installing(&store, &fake, "two");

    let one_new = fixture.build(Package::named("one").version("2.0.0"));
    let two_new = fixture.build(Package::named("two").version("2.0.0"));
    publish(&store, &fake, "sepia", &[&one, &two, &one_new, &two_new]);

    let report = upgrade(&store, &fake, &target(), Some(&name("one")), false).unwrap();

    assert_eq!(report.considered, 1);
    let moved = versions(&store);
    assert_eq!(moved.get("one").map(String::as_str), Some("2.0.0"));
    assert_eq!(
        moved.get("two").map(String::as_str),
        Some("1.0.0"),
        "a package that was not named was upgraded"
    );
}

#[test]
fn a_dry_run_says_what_would_move_and_moves_nothing() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(Package::named("plain").version("1.0.0"));
    publish(&store, &fake, "sepia", &[&old]);
    installing(&store, &fake, "plain");
    let new = fixture.build(Package::named("plain").version("2.0.0"));
    publish(&store, &fake, "sepia", &[&old, &new]);
    let before = snapshot(fixture.root());
    let asked_before = fake.asked().len();

    let report = upgrade(&store, &fake, &target(), None, true).unwrap();

    assert!(!report.changed);
    assert_eq!(report.plan.steps.len(), 1);
    assert_eq!(
        report.plan.steps[0].selected.version.version.as_str(),
        "2.0.0"
    );
    assert_eq!(snapshot(fixture.root()), before);
    assert_eq!(
        fake.asked().len(),
        asked_before,
        "a dry run fetched something"
    );
}

#[test]
fn a_new_version_brings_in_a_dependency_it_did_not_used_to_need() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(Package::named("helix").version("1.0.0"));
    publish(&store, &fake, "sepia", &[&old]);
    installing(&store, &fake, "helix");

    let runtime = fixture.build(Package::named("llvm-runtime").version("1.0.0"));
    let new = fixture.build(
        Package::named("helix")
            .version("2.0.0")
            .depends_on("llvm-runtime", "1.0.0"),
    );
    publish(&store, &fake, "sepia", &[&old, &runtime, &new]);

    let report = upgrading(&store, &fake).unwrap();

    report.outcome().unwrap();
    let moved = versions(&store);
    assert_eq!(moved.get("helix").map(String::as_str), Some("2.0.0"));
    assert_eq!(
        moved.get("llvm-runtime").map(String::as_str),
        Some("1.0.0"),
        "the new version's dependency did not come in with it"
    );
    let database = Database::new(&store);
    assert_eq!(
        database.get(&name("llvm-runtime")).unwrap().unwrap().reason,
        Reason::Dependency
    );
}

#[test]
fn a_package_that_came_in_as_a_dependency_stays_one_when_it_moves() {
    // Making it explicit because it moved would quietly take it out of
    // autoremove's reach for ever.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let runtime = fixture.build(Package::named("llvm-runtime").version("1.0.0"));
    let helix = fixture.build(
        Package::named("helix")
            .version("1.0.0")
            .depends_on("llvm-runtime", "1.0.0"),
    );
    publish(&store, &fake, "sepia", &[&runtime, &helix]);
    installing(&store, &fake, "helix");

    let runtime_new = fixture.build(Package::named("llvm-runtime").version("2.0.0"));
    publish(&store, &fake, "sepia", &[&runtime, &helix, &runtime_new]);

    upgrading(&store, &fake).unwrap();

    let record = Database::new(&store)
        .get(&name("llvm-runtime"))
        .unwrap()
        .unwrap();
    assert_eq!(record.metadata.version.as_str(), "2.0.0");
    assert_eq!(record.reason, Reason::Dependency, "it became explicit");
}

#[test]
fn a_package_whose_source_is_gone_has_nowhere_to_be_upgraded_from() {
    // `remove-source` says this will happen; this is it happening.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(Package::named("plain").version("1.0.0"));
    publish(&store, &fake, "sepia", &[&old]);
    installing(&store, &fake, "plain");

    // Another source offers a newer one of the same name, and the one it came
    // from is gone.
    let new = fixture.build(Package::named("plain").version("2.0.0"));
    publish(&store, &fake, "local", &[&old, &new]);
    index::forget(&store, &SourceName::parse("sepia").unwrap()).unwrap();

    let report = upgrading(&store, &fake).unwrap();

    assert!(report.is_empty(), "it took a version from another source");
    assert_eq!(
        versions(&store).get("plain").map(String::as_str),
        Some("1.0.0")
    );
    report.outcome().unwrap();
}

#[test]
fn nothing_is_found_that_the_last_update_did_not() {
    // It reads the local indexes and nothing else, so a source publishing
    // something makes no difference until an `update` fetches it.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(Package::named("plain").version("1.0.0"));
    publish(&store, &fake, "sepia", &[&old]);
    installing(&store, &fake, "plain");

    // Served, but not indexed on the device.
    let new = fixture.build(Package::named("plain").version("2.0.0"));
    fake.serve_file(
        &format!("sepia/{}", new.package.file_name().unwrap().display()),
        &new.package,
    );

    let report = upgrading(&store, &fake).unwrap();

    assert!(report.is_empty());
    assert_eq!(
        versions(&store).get("plain").map(String::as_str),
        Some("1.0.0")
    );
}

#[test]
fn upgrading_something_that_is_not_installed_says_so() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();

    match upgrade(&store, &fake, &target(), Some(&name("nothing")), false) {
        Err(error @ Error::NotInstalled { .. }) => {
            assert_eq!(error.exit_code(), 3);
            assert!(error.to_string().contains("nothing"), "{error}");
        }
        other => panic!("expected NotInstalled, got {other:?}"),
    }
}

#[test]
fn a_device_with_nothing_installed_succeeds_at_doing_nothing() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();

    let report = upgrading(&store, &fake).unwrap();

    assert!(report.is_empty());
    assert_eq!(report.considered, 0);
    report.outcome().unwrap();
}

#[test]
fn an_upgrade_takes_away_what_the_old_version_no_longer_ships() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(
        Package::named("plain")
            .version("1.0.0")
            .file("usr/share/plain/dropped", b"only in 1.0.0"),
    );
    publish(&store, &fake, "sepia", &[&old]);
    installing(&store, &fake, "plain");
    assert!(fixture.root().join("usr/share/plain/dropped").exists());

    let new = fixture.build(Package::named("plain").version("2.0.0"));
    publish(&store, &fake, "sepia", &[&old, &new]);

    upgrading(&store, &fake).unwrap();

    assert!(
        !fixture.root().join("usr/share/plain/dropped").exists(),
        "a file the new version does not ship was left behind"
    );
    assert!(fixture.root().join("usr/bin/plain").exists());
}
