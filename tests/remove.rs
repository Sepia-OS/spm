/*
  remove.rs

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

//! `remove`, against packages that were really installed.
//!
//! Every device here is built by running the real `install` over packages the
//! real `create` built, so what is taken back is what was actually put on —
//! rather than records a test wrote to describe an install that never happened.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use spm::error::Error;
use spm::model::index::Index;
use spm::model::installed::{Reason, Record};
use spm::model::metadata::Metadata;
use spm::model::name::{PackageName, PackageRef, SourceName, Target};
use spm::model::version::Version;
use spm::ops::install::install;
use spm::ops::remove::{Outcome, remove};
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

    fn has(&self, path: &str) -> bool {
        self.root().join(path).exists()
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

fn removing(store: &Store, package: &str) -> spm::error::Result<Outcome> {
    remove(store, &PackageRef::parse(package).unwrap(), false)
}

/// The names a removal took, in the order it took them.
fn taken(outcome: &Outcome) -> Vec<String> {
    outcome
        .removal
        .packages
        .iter()
        .map(|going| going.name.as_str().to_owned())
        .collect()
}

// ------------------------------------------------------- Step 34: the removal

#[test]
fn exactly_what_was_installed_is_taken_back() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain").file("usr/share/plain/data", b"data"));
    publish(&store, &fake, "sepia", &[&plain]);
    installing(&store, &fake, "plain");
    assert!(fixture.has("usr/bin/plain"));

    let outcome = removing(&store, "plain").unwrap();

    assert!(outcome.changed);
    assert_eq!(taken(&outcome), vec!["plain"]);
    assert!(!fixture.has("usr/bin/plain"));
    assert!(!fixture.has("usr/share/plain/data"));
    assert!(!fixture.has("usr/share/licenses/plain/LICENSE"));
    assert!(!Database::new(&store).is_installed(&name("plain")).unwrap());
}

#[test]
fn directories_go_when_the_last_thing_in_them_does() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain =
        fixture.build(Package::named("plain").file("usr/lib/plain/deep/down/data", b"data"));
    publish(&store, &fake, "sepia", &[&plain]);
    installing(&store, &fake, "plain");
    assert!(fixture.has("usr/lib/plain/deep/down"));

    removing(&store, "plain").unwrap();

    for emptied in [
        "usr/lib/plain/deep/down",
        "usr/lib/plain/deep",
        "usr/lib/plain",
        "usr/lib",
        "usr/bin",
        "usr/share/licenses/plain",
    ] {
        assert!(!fixture.has(emptied), "{emptied} was left behind, empty");
    }
    // The root the package was unpacked into is not the package's to remove.
    assert!(fixture.root().exists());
}

#[test]
fn a_directory_that_still_holds_something_stays() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);
    installing(&store, &fake, "plain");
    // Something the image put in the same directory.
    fs::write(fixture.root().join("usr/bin/from-the-image"), b"not ours").unwrap();

    removing(&store, "plain").unwrap();

    assert!(!fixture.has("usr/bin/plain"));
    assert!(
        fixture.has("usr/bin/from-the-image"),
        "a file spm did not install was removed"
    );
    assert!(fixture.has("usr/bin"), "a directory that is not empty went");
}

#[test]
fn nothing_outside_the_record_is_touched() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let other = fixture.build(Package::named("other"));
    publish(&store, &fake, "sepia", &[&plain, &other]);
    installing(&store, &fake, "plain");
    installing(&store, &fake, "other");
    fs::create_dir_all(fixture.root().join("usr/local")).unwrap();
    fs::write(fixture.root().join("usr/local/hand-made"), b"by hand").unwrap();

    removing(&store, "plain").unwrap();

    assert!(!fixture.has("usr/bin/plain"));
    assert!(fixture.has("usr/bin/other"), "another package lost a file");
    assert!(fixture.has("usr/share/licenses/other/LICENSE"));
    assert!(fixture.has("usr/local/hand-made"));
    assert!(Database::new(&store).is_installed(&name("other")).unwrap());
}

#[test]
fn a_package_something_else_needs_is_refused_and_the_dependents_are_named() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let dependent = fixture.build(Package::named("dependent").depends_on("plain", "1.0.0"));
    publish(&store, &fake, "sepia", &[&plain, &dependent]);
    installing(&store, &fake, "dependent");

    match removing(&store, "plain") {
        Err(error @ Error::HasDependents { .. }) => {
            assert_eq!(error.exit_code(), 7);
            let text = error.to_string();
            assert!(text.contains("plain"), "{text}");
            assert!(text.contains("dependent 1.0.0"), "{text}");
        }
        other => panic!("expected HasDependents, got {other:?}"),
    }

    // Refused means nothing happened, not "most of it happened".
    assert!(fixture.has("usr/bin/plain"));
    assert!(Database::new(&store).is_installed(&name("plain")).unwrap());
}

#[test]
fn a_file_two_records_claim_survives_the_first_of_them_going() {
    // `install` refuses to let two records claim one file, so this state cannot
    // be reached through it — the records are written directly. The rule is a
    // net rather than a mechanism: a record is a file somebody can edit, and
    // taking a file another package is using is not a mistake worth being able
    // to make.
    let fixture = Fixture::new();
    let store = fixture.store();
    let database = Database::new(&store);

    let shared = PathBuf::from("usr/lib/shared.so");
    let mine = PathBuf::from("usr/bin/mine");
    for path in [&shared, &mine] {
        let full = fixture.root().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, b"contents").unwrap();
    }

    for (package, files) in [
        ("mine", vec![shared.clone(), mine.clone()]),
        ("theirs", vec![shared.clone()]),
    ] {
        let record = Record {
            metadata: Metadata {
                name: name(package),
                version: Version::parse("1.0.0").unwrap(),
                target: target(),
                description: String::new(),
                dependencies: Vec::new(),
                sha256: None,
            },
            source: SourceName::parse("sepia").unwrap(),
            reason: Reason::Explicit,
            installed_at: 1,
            files,
        };
        database.begin(&record).unwrap();
        database.commit(&name(package)).unwrap();
    }

    let outcome = removing(&store, "mine").unwrap();

    assert_eq!(
        outcome.removal.packages[0].files,
        vec![mine.clone()],
        "the shared file should not even be planned for removal"
    );
    assert!(
        fixture.has("usr/lib/shared.so"),
        "a file the other record also claims was taken"
    );
    assert!(!fixture.has("usr/bin/mine"));
    assert!(database.is_installed(&name("theirs")).unwrap());
}

#[test]
fn a_package_that_is_not_installed_says_so() {
    let fixture = Fixture::new();
    let store = fixture.store();

    match removing(&store, "nothing") {
        Err(error @ Error::NotInstalled { .. }) => {
            assert_eq!(error.exit_code(), 3);
            let text = error.to_string();
            assert!(text.contains("nothing"), "{text}");
            assert!(text.contains("--installed"), "it should say how to look");
        }
        other => panic!("expected NotInstalled, got {other:?}"),
    }
}

#[test]
fn a_source_that_did_not_supply_it_is_not_the_package_that_is_installed() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);
    installing(&store, &fake, "plain");

    // Installed, but not from there.
    assert!(matches!(
        removing(&store, "local/plain"),
        Err(Error::NotInstalled { .. })
    ));
    // And the qualified form that is right works.
    removing(&store, "sepia/plain").unwrap();
    assert!(!fixture.has("usr/bin/plain"));
}

#[test]
fn a_dry_run_says_what_would_go_and_takes_nothing() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let dependent = fixture.build(Package::named("dependent").depends_on("plain", "1.0.0"));
    publish(&store, &fake, "sepia", &[&plain, &dependent]);
    installing(&store, &fake, "dependent");

    let outcome = remove(&store, &PackageRef::parse("dependent").unwrap(), true).unwrap();

    assert!(!outcome.changed);
    assert_eq!(taken(&outcome), vec!["dependent", "plain"]);
    assert!(outcome.removal.files() > 0);
    assert!(fixture.has("usr/bin/dependent"));
    assert!(fixture.has("usr/bin/plain"));
    assert!(
        Database::new(&store)
            .is_installed(&name("dependent"))
            .unwrap()
    );
}

// ----------------------------------------------------- Step 35: the autoremove

#[test]
fn what_came_in_as_a_dependency_goes_when_nothing_needs_it() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let dependent = fixture.build(Package::named("dependent").depends_on("plain", "1.0.0"));
    publish(&store, &fake, "sepia", &[&plain, &dependent]);
    installing(&store, &fake, "dependent");

    let outcome = removing(&store, "dependent").unwrap();

    // The one that was asked for first, then what followed from it.
    assert_eq!(taken(&outcome), vec!["dependent", "plain"]);
    assert!(
        !outcome.removal.packages[0].unneeded,
        "the package that was asked for is not an unneeded one"
    );
    assert!(outcome.removal.packages[1].unneeded);
    assert!(!fixture.has("usr/bin/plain"));
    assert!(!fixture.has("usr/bin/dependent"));
    assert!(Database::new(&store).all().unwrap().is_empty());
}

#[test]
fn removing_the_head_of_a_chain_three_deep_removes_all_three() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let bottom = fixture.build(Package::named("bottom"));
    let middle = fixture.build(Package::named("middle").depends_on("bottom", "1.0.0"));
    let top = fixture.build(Package::named("top").depends_on("middle", "1.0.0"));
    publish(&store, &fake, "sepia", &[&bottom, &middle, &top]);
    installing(&store, &fake, "top");

    let outcome = removing(&store, "top").unwrap();

    let mut names = taken(&outcome);
    assert_eq!(names.first().map(String::as_str), Some("top"));
    names.sort();
    assert_eq!(names, vec!["bottom", "middle", "top"]);
    assert!(Database::new(&store).all().unwrap().is_empty());
    assert!(!fixture.has("usr/bin"), "nothing should be left under usr");
}

#[test]
fn an_explicitly_installed_package_stops_the_cascade() {
    // The whole point of recording *why* a package is there: somebody asked
    // for the middle one, and nothing about what else goes changes that.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let bottom = fixture.build(Package::named("bottom"));
    let middle = fixture.build(Package::named("middle").depends_on("bottom", "1.0.0"));
    let top = fixture.build(Package::named("top").depends_on("middle", "1.0.0"));
    publish(&store, &fake, "sepia", &[&bottom, &middle, &top]);
    installing(&store, &fake, "middle");
    installing(&store, &fake, "top");

    let database = Database::new(&store);
    assert!(
        database
            .get(&name("middle"))
            .unwrap()
            .unwrap()
            .is_explicit(),
        "installing it by name should have made it explicit"
    );

    let outcome = removing(&store, "top").unwrap();

    assert_eq!(taken(&outcome), vec!["top"]);
    assert!(database.is_installed(&name("middle")).unwrap());
    assert!(
        database.is_installed(&name("bottom")).unwrap(),
        "middle still needs it"
    );
    assert!(fixture.has("usr/bin/middle"));
    assert!(fixture.has("usr/bin/bottom"));
}

#[test]
fn a_dependency_two_packages_share_stays_until_both_are_gone() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let shared = fixture.build(Package::named("shared"));
    let left = fixture.build(Package::named("left").depends_on("shared", "1.0.0"));
    let right = fixture.build(Package::named("right").depends_on("shared", "1.0.0"));
    publish(&store, &fake, "sepia", &[&shared, &left, &right]);
    installing(&store, &fake, "left");
    installing(&store, &fake, "right");

    let first = removing(&store, "left").unwrap();
    assert_eq!(taken(&first), vec!["left"], "right still needs shared");
    assert!(fixture.has("usr/bin/shared"));

    let second = removing(&store, "right").unwrap();
    assert_eq!(taken(&second), vec!["right", "shared"]);
    assert!(!fixture.has("usr/bin/shared"));
}

#[test]
fn an_install_that_did_not_finish_is_taken_back_before_a_removal_runs() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);
    installing(&store, &fake, "plain");

    // What a power cut during an install of something else leaves behind.
    let database = Database::new(&store);
    let stray = fixture.root().join("usr/bin/half-installed");
    fs::write(&stray, b"half").unwrap();
    let record = Record {
        metadata: Metadata {
            name: name("interrupted"),
            version: Version::parse("1.0.0").unwrap(),
            target: target(),
            description: String::new(),
            dependencies: Vec::new(),
            sha256: None,
        },
        source: SourceName::parse("sepia").unwrap(),
        reason: Reason::Explicit,
        installed_at: 1,
        files: vec![PathBuf::from("usr/bin/half-installed")],
    };
    database.begin(&record).unwrap();

    let outcome = removing(&store, "plain").unwrap();

    assert_eq!(outcome.rolled_back, vec![name("interrupted")]);
    assert!(
        !stray.exists(),
        "the unfinished install was left on the card"
    );
    // And the removal it was actually asked for still happened.
    assert!(!fixture.has("usr/bin/plain"));
}
