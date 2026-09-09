/*
  verify.rs

  Created on 2026-09-09 by Thomas Bonk <thomas@meandmymac.de>
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

//! `verify`, against devices that were really installed onto.
//!
//! Three findings and one distinction: a missing file is a fault, a directory
//! standing where a file should be is a fault, and an edited configuration file
//! is not - it is what administering a device looks like. A `verify` that failed
//! because somebody had configured their card would be one nobody ran twice.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use std::fs;
use std::path::Path;

use spm::model::index::Index;
use spm::model::name::{PackageRef, SourceName, Target};
use spm::ops::install::install;
use spm::ops::verify::{Finding, verify};
use spm::store::Store;
use spm::store::config::{Source, Sources};
use spm::store::index;
use support::net::Fake;
use support::{Built, Package, index_of};

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

fn publish(store: &Store, fake: &Fake, source: &str, packages: &[&Built]) {
    let mut sources = Sources::load(store).unwrap();
    sources.insert(Source {
        name: SourceName::parse(source).unwrap(),
        url: fake.url_for(&format!("{source}/index.json")),
        is_default: false,
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

/// A device with one package on it, installed for real.
fn installed(fixture: &Fixture, package: Package) -> Store {
    let store = fixture.store();
    let fake = fixture.transport();
    let built = fixture.build(package);
    publish(&store, &fake, "sepia", &[&built]);
    installing(&store, &fake, built.name.as_str());
    store
}

#[test]
fn a_device_with_nothing_installed_verifies() {
    // Nothing is wrong with a card nothing has been added to, and saying so is
    // better than an empty report somebody has to interpret.
    let fixture = Fixture::new();
    let report = verify(&fixture.store(), None).unwrap();
    assert!(report.packages.is_empty());
    assert!(report.is_sound());
    assert_eq!(report.faults(), 0);
}

#[test]
fn a_package_straight_out_of_the_box_is_sound() {
    let fixture = Fixture::new();
    let store = installed(&fixture, Package::named("helix"));

    let report = verify(&store, None).unwrap();

    assert_eq!(report.packages.len(), 1);
    assert!(report.is_sound());
    assert!(report.packages[0].findings.is_empty());
    assert!(report.packages[0].files > 0, "it should have checked files");
}

#[test]
fn a_deleted_file_is_a_fault_and_is_named() {
    let fixture = Fixture::new();
    let store = installed(&fixture, Package::named("helix"));
    let gone = Path::new("usr/bin/helix");
    fs::remove_file(fixture.root().join(gone)).unwrap();

    let report = verify(&store, None).unwrap();

    assert!(!report.is_sound());
    assert_eq!(report.faults(), 1);
    assert_eq!(report.broken().len(), 1);
    let found = &report.packages[0].findings;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, gone);
    assert_eq!(found[0].finding, Finding::Missing);
}

#[test]
fn a_directory_where_a_file_should_be_is_a_fault() {
    let fixture = Fixture::new();
    let store = installed(&fixture, Package::named("helix"));
    let path = fixture.root().join("usr/bin/helix");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();

    let report = verify(&store, None).unwrap();

    assert!(!report.is_sound());
    assert_eq!(report.packages[0].findings[0].finding, Finding::NotAFile);
}

#[test]
fn an_edited_configuration_file_is_reported_and_is_not_a_fault() {
    // The distinction the whole command turns on.
    let fixture = Fixture::new();
    let store = installed(
        &fixture,
        Package::named("helix").file("etc/helix.conf", b"theme = default\n"),
    );
    fs::write(fixture.root().join("etc/helix.conf"), b"theme = mine\n").unwrap();

    let report = verify(&store, None).unwrap();

    assert!(report.is_sound(), "an edit is not damage");
    assert_eq!(report.faults(), 0);
    assert_eq!(report.edited(), 1);
    assert_eq!(report.packages[0].findings[0].finding, Finding::Edited);
    assert_eq!(
        report.packages[0].findings[0].path,
        Path::new("etc/helix.conf")
    );
}

#[test]
fn an_untouched_configuration_file_says_nothing() {
    let fixture = Fixture::new();
    let store = installed(
        &fixture,
        Package::named("helix").file("etc/helix.conf", b"theme = default\n"),
    );

    let report = verify(&store, None).unwrap();

    assert!(report.is_sound());
    assert_eq!(report.edited(), 0);
    assert!(report.packages[0].findings.is_empty());
}

#[test]
fn one_package_can_be_checked_on_its_own() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let helix = fixture.build(Package::named("helix"));
    let other = fixture.build(Package::named("grit"));
    publish(&store, &fake, "sepia", &[&helix, &other]);
    installing(&store, &fake, "helix");
    installing(&store, &fake, "grit");

    // Break the one that is not being asked about.
    fs::remove_file(fixture.root().join("usr/bin/grit")).unwrap();

    let report = verify(&store, Some(&PackageRef::parse("helix").unwrap())).unwrap();

    assert_eq!(report.packages.len(), 1);
    assert_eq!(report.packages[0].name.as_str(), "helix");
    assert!(report.is_sound(), "grit's damage is not helix's");

    // And asking about everything does find it.
    let all = verify(&store, None).unwrap();
    assert!(!all.is_sound());
    assert_eq!(all.faults(), 1);
}

#[test]
fn asking_about_something_that_is_not_installed_says_so() {
    let fixture = Fixture::new();
    let store = installed(&fixture, Package::named("helix"));

    let outcome = verify(&store, Some(&PackageRef::parse("nothing").unwrap()));

    assert!(
        matches!(outcome, Err(spm::error::Error::NotInstalled { .. })),
        "expected NotInstalled, got {outcome:?}"
    );
}

#[test]
fn verifying_changes_nothing() {
    // It reads the card and the records and writes neither. Checked by running
    // it twice over a device with a fault and an edit, and finding the same
    // answer both times.
    let fixture = Fixture::new();
    let store = installed(
        &fixture,
        Package::named("helix").file("etc/helix.conf", b"theme = default\n"),
    );
    fs::write(fixture.root().join("etc/helix.conf"), b"theme = mine\n").unwrap();
    fs::remove_file(fixture.root().join("usr/bin/helix")).unwrap();

    let first = verify(&store, None).unwrap();
    let second = verify(&store, None).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.faults(), 1);
    assert_eq!(first.edited(), 1);

    // The edit is still there, and so is the missing file's absence.
    assert_eq!(
        fs::read(fixture.root().join("etc/helix.conf")).unwrap(),
        b"theme = mine\n"
    );
    assert!(!fixture.root().join("usr/bin/helix").exists());
}
#[test]
fn verify_failed_reads_as_english() {
    use spm::error::Error;
    assert_eq!(
        Error::VerifyFailed {
            files: 1,
            packages: 1
        }
        .to_string(),
        "1 file of 1 installed package is missing or no longer a file"
    );
    assert_eq!(
        Error::VerifyFailed {
            files: 3,
            packages: 2
        }
        .to_string(),
        "3 files of 2 installed packages are missing or no longer files"
    );
}
