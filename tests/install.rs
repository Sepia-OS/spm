/*
  install.rs

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

//! `install`, end to end, against real packages and a real filesystem.
//!
//! Every package here is built by `spm create` and served out of a directory
//! through the fake transport, so what these drive is the whole command —
//! resolution, the two digest checks, the extraction rules, the conflict
//! refusals and the journal — with nothing stubbed but the network.

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
use spm::model::version::Version;
use spm::ops::install::{Change, Outcome, install, plan};
use spm::store::Store;
use spm::store::config::{Source, Sources};
use spm::store::db::Database;
use spm::store::index;
use support::net::Fake;
use support::{Built, Package, index_of, swap_payload};

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

/// The path a package is served under, which is also where it is fetched from.
fn served_as(source: &str, built: &Built) -> String {
    format!("{source}/{}", built.package.file_name().unwrap().display())
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
            let url = fake.serve_file(&served_as(source, built), &built.package);
            (*built, url)
        })
        .collect();

    write_index(store, source, &index_of(source, &entries));
}

fn write_index(store: &Store, source: &str, text: &str) {
    let index: Index = serde_json::from_str(text).unwrap();
    index::write(store, &SourceName::parse(source).unwrap(), &index).unwrap();
}

/// The index as it was published, so a test can bend one entry of it.
fn published_index(store: &Store, source: &str) -> Index {
    index::read(store, &SourceName::parse(source).unwrap())
        .unwrap()
        .unwrap()
}

fn installing(store: &Store, fake: &Fake, package: &str) -> spm::error::Result<Outcome> {
    install(
        store,
        fake,
        &PackageRef::parse(package).unwrap(),
        &target(),
        None,
        false,
    )
}

fn installing_version(
    store: &Store,
    fake: &Fake,
    package: &str,
    version: &str,
) -> spm::error::Result<Outcome> {
    install(
        store,
        fake,
        &PackageRef::parse(package).unwrap(),
        &target(),
        Some(&Version::parse(version).unwrap()),
        false,
    )
}

/// Every file under a directory, with its contents, so two states can be
/// compared rather than spot-checked.
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
            // The directory itself counts: one appearing or vanishing is a
            // change to the prefix too.
            into.insert(relative, Vec::new());
            walk(root, &path, into);
        } else if metadata.is_symlink() {
            into.insert(
                relative,
                fs::read_link(&path)
                    .unwrap()
                    .into_os_string()
                    .into_encoded_bytes(),
            );
        } else {
            into.insert(relative, fs::read(&path).unwrap());
        }
    }
}

/// The names in a plan, in the order it lists them.
fn planned(outcome: &Outcome) -> Vec<String> {
    outcome
        .plan
        .steps
        .iter()
        .map(|step| step.selected.name.as_str().to_owned())
        .collect()
}

// ---------------------------------------------------------------- installing

#[test]
fn a_package_lands_on_the_device_and_is_recorded() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);

    let outcome = installing(&store, &fake, "plain").unwrap();

    assert!(outcome.changed);
    assert_eq!(planned(&outcome), vec!["plain"]);
    assert_eq!(
        fs::read_to_string(fixture.root().join("usr/bin/plain")).unwrap(),
        "#!/bin/sh\necho plain\n"
    );
    assert!(
        fixture
            .root()
            .join("usr/share/licenses/plain/LICENSE")
            .exists()
    );

    let record = Database::new(&store).get(&name("plain")).unwrap().unwrap();
    assert_eq!(record.reason, Reason::Explicit);
    assert_eq!(record.source.as_str(), "sepia");
    assert_eq!(record.metadata.version.as_str(), "1.0.0");
    assert!(record.files.contains(&PathBuf::from("usr/bin/plain")));
    // Directories are not claimed: they go when they empty out.
    assert!(!record.files.contains(&PathBuf::from("usr/bin")));
}

#[test]
fn a_dependency_comes_in_with_the_package_and_is_marked_as_one() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let dependent = fixture.build(Package::named("dependent").depends_on("plain", "1.0.0"));
    publish(&store, &fake, "sepia", &[&plain, &dependent]);

    let outcome = installing(&store, &fake, "dependent").unwrap();

    // Dependencies first, the package that was asked for last.
    assert_eq!(planned(&outcome), vec!["plain", "dependent"]);

    let database = Database::new(&store);
    assert_eq!(
        database.get(&name("plain")).unwrap().unwrap().reason,
        Reason::Dependency,
        "what lets remove clean up after itself"
    );
    assert_eq!(
        database.get(&name("dependent")).unwrap().unwrap().reason,
        Reason::Explicit
    );
    assert!(fixture.root().join("usr/bin/plain").exists());
    assert!(fixture.root().join("usr/bin/dependent").exists());
}

// ------------------------------------------------------- Step 27: resolution

#[test]
fn a_chain_three_deep_is_followed_to_the_end() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let bottom = fixture.build(Package::named("bottom"));
    let middle = fixture.build(Package::named("middle").depends_on("bottom", "1.0.0"));
    let top = fixture.build(Package::named("top").depends_on("middle", "1.0.0"));
    publish(&store, &fake, "sepia", &[&bottom, &middle, &top]);

    let outcome = installing(&store, &fake, "top").unwrap();

    let mut names = planned(&outcome);
    names.sort();
    assert_eq!(names, vec!["bottom", "middle", "top"]);
    for each in ["bottom", "middle", "top"] {
        assert!(
            Database::new(&store).is_installed(&name(each)).unwrap(),
            "{each} was not installed"
        );
    }
}

#[test]
fn a_diamond_brings_the_shared_package_in_once() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let shared = fixture.build(Package::named("shared"));
    let left = fixture.build(Package::named("left").depends_on("shared", "1.0.0"));
    let right = fixture.build(Package::named("right").depends_on("shared", "1.0.0"));
    let top = fixture.build(
        Package::named("top")
            .depends_on("left", "1.0.0")
            .depends_on("right", "1.0.0"),
    );
    publish(&store, &fake, "sepia", &[&shared, &left, &right, &top]);

    let outcome = installing(&store, &fake, "top").unwrap();

    let names = planned(&outcome);
    assert_eq!(
        names.iter().filter(|each| *each == "shared").count(),
        1,
        "the shared package appears more than once in {names:?}"
    );
    assert_eq!(names.len(), 4);
}

#[test]
fn packages_that_need_each_other_in_a_circle_are_reported() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let snake = fixture.build(Package::named("snake").depends_on("tail", "1.0.0"));
    let tail = fixture.build(Package::named("tail").depends_on("snake", "1.0.0"));
    publish(&store, &fake, "sepia", &[&snake, &tail]);

    match installing(&store, &fake, "snake") {
        Err(Error::DependencyCycle { chain }) => {
            assert_eq!(chain, vec!["snake", "tail", "snake"]);
        }
        other => panic!("expected DependencyCycle, got {other:?}"),
    }

    // And it stopped rather than looping: nothing was fetched or written.
    assert!(fake.asked().is_empty());
    assert!(!fixture.root().join("usr").exists());
}

#[test]
fn a_dependency_that_cannot_be_satisfied_names_what_wanted_it() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let needy = fixture.build(Package::named("needy").depends_on("plain", "9.9.9"));
    publish(&store, &fake, "sepia", &[&plain, &needy]);

    match installing(&store, &fake, "needy") {
        Err(Error::DependencyNotSatisfiable {
            package,
            dependency,
            needed,
            available,
        }) => {
            assert_eq!(package, "needy");
            assert_eq!(dependency, "plain");
            assert_eq!(needed, "9.9.9");
            assert_eq!(available, "1.0.0");
        }
        other => panic!("expected DependencyNotSatisfiable, got {other:?}"),
    }
}

#[test]
fn a_dependency_takes_the_oldest_version_that_satisfies_it() {
    // A floor is a floor. Taking the newest would upgrade half the card on the
    // strength of one package asking for something old.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(Package::named("plain").version("1.0.0"));
    let new = fixture.build(Package::named("plain").version("2.0.0"));
    let dependent = fixture.build(Package::named("dependent").depends_on("plain", "1.0.0"));
    publish(&store, &fake, "sepia", &[&old, &new, &dependent]);

    let outcome = installing(&store, &fake, "dependent").unwrap();

    let chosen = outcome
        .plan
        .steps
        .iter()
        .find(|step| step.selected.name.as_str() == "plain")
        .unwrap();
    assert_eq!(chosen.selected.version.version.as_str(), "1.0.0");
}

#[test]
fn a_dependency_already_satisfied_is_left_exactly_as_it_was() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let dependent = fixture.build(Package::named("dependent").depends_on("plain", "1.0.0"));
    publish(&store, &fake, "sepia", &[&plain, &dependent]);
    installing(&store, &fake, "plain").unwrap();
    let before = Database::new(&store).get(&name("plain")).unwrap().unwrap();

    let outcome = installing(&store, &fake, "dependent").unwrap();

    // It is not in the set at all: resolution drops a dependency the device
    // already satisfies rather than carrying it through to be skipped later.
    assert_eq!(planned(&outcome), vec!["dependent"]);
    // Still the user's own package, still at the version and the moment it was
    // installed, still owning the files it owned.
    let after = Database::new(&store).get(&name("plain")).unwrap().unwrap();
    assert_eq!(after, before);
    assert_eq!(after.reason, Reason::Explicit);
}

// ------------------------------------------------- Step 28: the plan and dry run

#[test]
fn a_dry_run_leaves_the_prefix_exactly_as_it_found_it() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let dependent = fixture.build(Package::named("dependent").depends_on("plain", "1.0.0"));
    publish(&store, &fake, "sepia", &[&plain, &dependent]);
    let before = snapshot(fixture.root());

    let outcome = install(
        &store,
        &fake,
        &PackageRef::parse("dependent").unwrap(),
        &target(),
        None,
        true,
    )
    .unwrap();

    // It said what it would do...
    assert!(!outcome.changed);
    assert_eq!(planned(&outcome), vec!["plain", "dependent"]);
    assert!(outcome.plan.download > 0);
    // ...and did none of it. Not a file, not a record, not the cache.
    assert_eq!(snapshot(fixture.root()), before);
    assert!(fake.asked().is_empty(), "a dry run fetched something");
}

#[test]
fn a_plan_says_what_is_new_what_is_an_upgrade_and_what_it_comes_to() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(Package::named("plain").version("1.0.0"));
    let new = fixture.build(Package::named("plain").version("2.0.0"));
    let dependent = fixture.build(Package::named("dependent").depends_on("plain", "2.0.0"));
    publish(&store, &fake, "sepia", &[&old, &new, &dependent]);
    installing_version(&store, &fake, "plain", "1.0.0").unwrap();

    let plan = plan(
        &store,
        &PackageRef::parse("dependent").unwrap(),
        &target(),
        None,
    )
    .unwrap();

    let upgrade = plan
        .steps
        .iter()
        .find(|step| step.selected.name.as_str() == "plain")
        .unwrap();
    match &upgrade.change {
        Change::Replaces(replaced) => assert_eq!(replaced.version.as_str(), "1.0.0"),
        other => panic!("expected an upgrade, got {other:?}"),
    }

    let fresh = plan
        .steps
        .iter()
        .find(|step| step.selected.name.as_str() == "dependent")
        .unwrap();
    assert_eq!(fresh.change, Change::New);

    // The download is what the index says those two packages weigh.
    assert_eq!(plan.download, new.bytes.saturating_add(dependent.bytes));
}

#[test]
fn a_package_already_installed_at_that_version_is_left_alone_and_said_so() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);
    installing(&store, &fake, "plain").unwrap();
    let before = snapshot(fixture.root());

    let outcome = installing(&store, &fake, "plain").unwrap();

    assert!(!outcome.changed);
    assert!(outcome.plan.is_empty());
    assert_eq!(outcome.plan.satisfied.len(), 1);
    assert_eq!(snapshot(fixture.root()), before);
}

// -------------------------------------------------- Step 29: the two digests

#[test]
fn a_package_that_is_not_what_the_index_describes_is_refused_before_it_is_opened() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);
    // Not merely corrupted: not an archive at all. Anything that opened it
    // would fail to parse it, and this must fail before that.
    fake.serve(&served_as("sepia", &plain), b"this is not a package");

    match installing(&store, &fake, "plain") {
        Err(error @ Error::Verification { .. }) => {
            assert_eq!(error.exit_code(), 6);
            let text = error.to_string();
            assert!(text.contains("plain-1.0.0-aarch64-musl.tar.gz"), "{text}");
            assert!(
                !text.contains("data.tar.gz"),
                "this is the package's own digest, not its payload's: {text}"
            );
        }
        other => panic!("expected the package's own digest to fail, got {other:?}"),
    }

    assert!(!fixture.root().join("usr").exists());
    assert!(!Database::new(&store).is_installed(&name("plain")).unwrap());
}

#[test]
fn a_package_rebuilt_around_another_payload_is_refused_at_the_second_check() {
    // A perfectly well-formed archive whose own digest is right, holding a
    // payload its metadata does not describe. Only the second check catches it,
    // and a test that could not tell the two apart would not be testing this.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let other = fixture.build(Package::named("other"));
    publish(&store, &fake, "sepia", &[&plain]);

    let (swapped, digest) = swap_payload(&other, &plain, fixture.work.path());
    fake.serve_file(&served_as("sepia", &plain), &swapped);
    // The index carries the swapped archive's own digest, so the first check
    // passes and the second is the only one left to fail.
    let mut index = published_index(&store, "sepia");
    index.packages[0].versions[0].sha256 = spm::model::metadata::Sha256::parse(&digest).unwrap();
    index::write(&store, &SourceName::parse("sepia").unwrap(), &index).unwrap();

    match installing(&store, &fake, "plain") {
        Err(error @ Error::Verification { .. }) => {
            assert_eq!(error.exit_code(), 6);
            assert!(error.to_string().contains("data.tar.gz"), "{error}");
        }
        other => panic!("expected the payload digest to fail, got {other:?}"),
    }

    assert!(!fixture.root().join("usr").exists());
}

// ---------------------------------------------------- Step 31: the conflicts

#[test]
fn two_packages_that_claim_one_file_do_not_both_get_it() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    let rival = fixture
        .build(Package::named("rival").executable("usr/bin/plain", b"#!/bin/sh\necho not plain\n"));
    publish(&store, &fake, "sepia", &[&plain, &rival]);
    installing(&store, &fake, "plain").unwrap();

    match installing(&store, &fake, "rival") {
        Err(error @ Error::FileConflict { .. }) => {
            assert_eq!(error.exit_code(), 7);
            let text = error.to_string();
            assert!(text.contains("usr/bin/plain"), "{text}");
            assert!(text.contains("plain"), "{text}");
        }
        other => panic!("expected FileConflict, got {other:?}"),
    }

    // Before a single file was written: not even the ones rival alone owns.
    assert!(!fixture.root().join("usr/bin/rival").exists());
    assert!(!Database::new(&store).is_installed(&name("rival")).unwrap());
    assert_eq!(
        fs::read_to_string(fixture.root().join("usr/bin/plain")).unwrap(),
        "#!/bin/sh\necho plain\n",
        "the file that was already there was overwritten"
    );
}

#[test]
fn a_file_the_image_put_there_is_not_taken_over() {
    // Adopting it would mean `remove` later deleting something `spm` never
    // installed.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);
    fs::create_dir_all(fixture.root().join("usr/bin")).unwrap();
    fs::write(fixture.root().join("usr/bin/plain"), b"from the image").unwrap();

    match installing(&store, &fake, "plain") {
        Err(error @ Error::FileUnowned { .. }) => {
            assert_eq!(error.exit_code(), 7);
            assert!(error.to_string().contains("usr/bin/plain"), "{error}");
        }
        other => panic!("expected FileUnowned, got {other:?}"),
    }

    assert_eq!(
        fs::read_to_string(fixture.root().join("usr/bin/plain")).unwrap(),
        "from the image"
    );
    assert!(!Database::new(&store).is_installed(&name("plain")).unwrap());
}

// ------------------------------------------- Step 32: the journal and recovery

#[cfg(unix)]
#[test]
fn an_install_that_does_not_finish_is_taken_back_by_the_next_command() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    // A package whose files straddle an obstruction: `usr/bin/broken` is
    // written, and then `usr/lib` turns out to be a symbolic link, which is
    // never written through. That is an install stopping partway with a
    // journal on disk and a file already on the card.
    let broken = fixture.build(Package::named("broken").file("usr/lib/broken/data", b"payload"));
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&broken, &plain]);

    let elsewhere = fixture.work.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(fixture.root().join("usr")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, fixture.root().join("usr/lib")).unwrap();

    assert!(installing(&store, &fake, "broken").is_err());

    // What an interrupted install leaves: a journal, and a file it had got to.
    let journal = store.partial_record_file(&name("broken"));
    assert!(journal.exists(), "no journal was left");
    assert!(fixture.root().join("usr/bin/broken").exists());
    assert!(!Database::new(&store).is_installed(&name("broken")).unwrap());

    // The next command runs the rollback before anything else it does.
    let outcome = installing(&store, &fake, "plain").unwrap();

    assert_eq!(outcome.rolled_back, vec![name("broken")]);
    assert!(!journal.exists(), "the journal survived the rollback");
    assert!(
        !fixture.root().join("usr/bin/broken").exists(),
        "the file the unfinished install wrote is still there"
    );
    assert!(!elsewhere.join("broken").exists(), "the link was followed");
    // And the command it was actually asked to run still ran.
    assert!(Database::new(&store).is_installed(&name("plain")).unwrap());
}

#[test]
fn an_upgrade_takes_away_what_the_old_version_no_longer_ships() {
    // Otherwise the file stays on the card owned by nobody, where `remove` will
    // never reach it and the next install will refuse to overwrite it.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(
        Package::named("plain")
            .version("1.0.0")
            .file("usr/share/plain/dropped", b"only in 1.0.0"),
    );
    let new = fixture.build(Package::named("plain").version("2.0.0"));
    publish(&store, &fake, "sepia", &[&old, &new]);
    installing_version(&store, &fake, "plain", "1.0.0").unwrap();
    assert!(fixture.root().join("usr/share/plain/dropped").exists());

    let outcome = installing(&store, &fake, "plain").unwrap();

    assert!(outcome.changed);
    let record = Database::new(&store).get(&name("plain")).unwrap().unwrap();
    assert_eq!(record.metadata.version.as_str(), "2.0.0");
    assert!(
        !fixture.root().join("usr/share/plain/dropped").exists(),
        "a file the new version does not ship was left behind"
    );
    assert!(fixture.root().join("usr/bin/plain").exists());
}

// ------------------------------------------------------ Step 33: the disk space

#[test]
fn an_install_that_would_not_fit_is_refused_before_anything_is_fetched() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);

    // The index is where a device learns how big a download is, before it has
    // fetched a byte of it. This one claims a package no card could hold.
    let mut index = published_index(&store, "sepia");
    index.packages[0].versions[0].bytes = u64::MAX / 4;
    index::write(&store, &SourceName::parse("sepia").unwrap(), &index).unwrap();

    match installing(&store, &fake, "plain") {
        Err(error @ Error::NotEnoughSpace { .. }) => {
            let text = error.to_string();
            assert!(text.contains("TiB"), "it should say what is needed: {text}");
            assert!(text.contains("free"), "and what there is: {text}");
        }
        other => panic!("expected NotEnoughSpace, got {other:?}"),
    }

    assert!(
        fake.asked().is_empty(),
        "it went to the network before checking there was room"
    );
    assert!(!fixture.root().join("usr").exists());
}

#[test]
fn a_package_that_does_fit_is_not_refused() {
    // The other half of the check: the real reading of the real filesystem has
    // to let an ordinary install through.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let plain = fixture.build(Package::named("plain"));
    publish(&store, &fake, "sepia", &[&plain]);

    installing(&store, &fake, "plain").unwrap();

    assert!(fixture.root().join("usr/bin/plain").exists());
}
