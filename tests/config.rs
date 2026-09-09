/*
  config.rs

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

//! Configuration files: what happens to `etc/` when a package moves or goes.
//!
//! The rest of a package is `spm`'s to replace and delete. A file under `etc/`
//! is a default somebody may have edited since, and every one of these tests is
//! about telling those two apart - because the digest recorded at install time
//! is the only thing that can.

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
use spm::model::name::{PackageName, PackageRef, SourceName, Target};
use spm::ops::install::install;
use spm::ops::remove::remove;
use spm::ops::upgrade::upgrade;
use spm::store::Store;
use spm::store::config::{Source, Sources};
use spm::store::db::Database;
use spm::store::index;
use support::net::Fake;
use support::{Built, Package, index_of};

const CONF: &str = "etc/helix.conf";

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

/// A package that ships one configuration file, at the given version.
fn with_config(version: &str, contents: &[u8]) -> Package {
    Package::named("helix")
        .version(version)
        .file(CONF, contents)
}

#[test]
fn a_package_may_ship_a_default_under_etc() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let built = fixture.build(with_config("1.0.0", b"theme = default\n"));
    publish(&store, &fake, "sepia", &[&built]);

    installing(&store, &fake, "helix");

    assert_eq!(
        fs::read(fixture.root().join(CONF)).unwrap(),
        b"theme = default\n"
    );

    // And the record carries its digest, which is the whole basis of every
    // decision below.
    let record = Database::new(&store).get(&name("helix")).unwrap().unwrap();
    assert!(record.files.iter().any(|file| file == Path::new(CONF)));
    assert!(record.config.contains_key(Path::new(CONF)));
}

#[test]
fn an_untouched_default_is_replaced_on_upgrade() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(with_config("1.0.0", b"theme = default\n"));
    let new = fixture.build(with_config("2.0.0", b"theme = corrected\n"));
    publish(&store, &fake, "sepia", &[&old]);
    installing(&store, &fake, "helix");
    publish(&store, &fake, "sepia", &[&old, &new]);

    upgrade(&store, &fake, &target(), None, false).unwrap();

    // Nobody touched it, so it was only ever a default and the new one wins.
    assert_eq!(
        fs::read(fixture.root().join(CONF)).unwrap(),
        b"theme = corrected\n"
    );
    assert!(!fixture.root().join("etc/helix.conf.spmnew").exists());
}

#[test]
fn an_edited_file_is_kept_and_the_new_default_lands_beside_it() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let old = fixture.build(with_config("1.0.0", b"theme = default\n"));
    let new = fixture.build(with_config("2.0.0", b"theme = corrected\n"));
    publish(&store, &fake, "sepia", &[&old]);
    installing(&store, &fake, "helix");

    // Somebody edits it.
    fs::write(fixture.root().join(CONF), b"theme = mine\n").unwrap();

    publish(&store, &fake, "sepia", &[&old, &new]);
    upgrade(&store, &fake, &target(), None, false).unwrap();

    // The edit survives, and the new default is beside it under the whole name
    // plus the suffix - not `helix.spmnew`.
    assert_eq!(
        fs::read(fixture.root().join(CONF)).unwrap(),
        b"theme = mine\n"
    );
    assert_eq!(
        fs::read(fixture.root().join("etc/helix.conf.spmnew")).unwrap(),
        b"theme = corrected\n"
    );
}

#[test]
fn an_edited_file_stays_edited_for_every_upgrade_after_the_first() {
    // The digest recorded is of what was *shipped*, never of the edit. Record
    // the edit and the next upgrade would think nobody had touched it.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let one = fixture.build(with_config("1.0.0", b"a\n"));
    let two = fixture.build(with_config("2.0.0", b"b\n"));
    let three = fixture.build(with_config("3.0.0", b"c\n"));
    publish(&store, &fake, "sepia", &[&one]);
    installing(&store, &fake, "helix");
    fs::write(fixture.root().join(CONF), b"mine\n").unwrap();

    publish(&store, &fake, "sepia", &[&one, &two]);
    upgrade(&store, &fake, &target(), None, false).unwrap();
    publish(&store, &fake, "sepia", &[&one, &two, &three]);
    upgrade(&store, &fake, &target(), None, false).unwrap();

    assert_eq!(fs::read(fixture.root().join(CONF)).unwrap(), b"mine\n");
    assert_eq!(
        fs::read(fixture.root().join("etc/helix.conf.spmnew")).unwrap(),
        b"c\n"
    );
}

#[test]
fn removing_takes_an_untouched_default_with_it() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let built = fixture.build(with_config("1.0.0", b"theme = default\n"));
    publish(&store, &fake, "sepia", &[&built]);
    installing(&store, &fake, "helix");

    let outcome = remove(&store, &PackageRef::parse("helix").unwrap(), false).unwrap();

    assert!(!fixture.root().join(CONF).exists());
    assert_eq!(outcome.removal.kept(), 0);
}

#[test]
fn removing_leaves_an_edited_file_behind_and_says_so() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let built = fixture.build(with_config("1.0.0", b"theme = default\n"));
    publish(&store, &fake, "sepia", &[&built]);
    installing(&store, &fake, "helix");
    fs::write(fixture.root().join(CONF), b"theme = mine\n").unwrap();

    let outcome = remove(&store, &PackageRef::parse("helix").unwrap(), false).unwrap();

    // The work outlives the package that brought it, and the removal names it
    // rather than leaving it to be discovered.
    assert_eq!(
        fs::read(fixture.root().join(CONF)).unwrap(),
        b"theme = mine\n"
    );
    assert_eq!(outcome.removal.kept(), 1);
    assert_eq!(
        outcome.removal.packages[0].kept,
        vec![std::path::PathBuf::from(CONF)]
    );
    // And it is not counted among the files that were deleted.
    assert!(
        !outcome.removal.packages[0]
            .files
            .contains(&std::path::PathBuf::from(CONF))
    );
}

#[test]
fn a_kept_file_belongs_to_nobody_and_a_reinstall_refuses_it() {
    // The consequence of keeping it, stated in the user guide: the file outlives
    // the record that claimed it, so installing the package again walks into
    // install's "a file no package owns" rule. Better a refusal that names it
    // than a silent overwrite of the work that was deliberately preserved.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let built = fixture.build(with_config("1.0.0", b"theme = default\n"));
    publish(&store, &fake, "sepia", &[&built]);
    installing(&store, &fake, "helix");
    fs::write(fixture.root().join(CONF), b"theme = mine\n").unwrap();
    remove(&store, &PackageRef::parse("helix").unwrap(), false).unwrap();

    let again = install(
        &store,
        &fake,
        &PackageRef::parse("helix").unwrap(),
        &target(),
        None,
        false,
    );
    assert!(
        matches!(again, Err(spm::error::Error::FileUnowned { .. })),
        "expected a refusal naming the file, got {again:?}"
    );
    // And the edit is still there, unharmed by the attempt.
    assert_eq!(
        fs::read(fixture.root().join(CONF)).unwrap(),
        b"theme = mine\n"
    );
}

#[test]
fn a_package_that_writes_outside_usr_and_etc_is_still_refused() {
    // The rule was widened by exactly one directory, not abandoned.
    let fixture = Fixture::new();
    let built = Package::named("helix").file("var/lib/helix/state", b"s");
    let outcome = std::panic::catch_unwind(|| built.build(fixture.work.path()));
    assert!(
        outcome.is_err(),
        "a package writing outside usr/ and etc/ must not build"
    );
}
