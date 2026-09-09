/*
  update.rs

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

//! Fetching indexes, and what happens when one of them cannot be fetched.
//!
//! The two properties that matter: a bad fetch never damages the index a
//! device already has, and one unreachable source does not stop the others.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use clap::Parser;
use spm::cli::{Cli, Command};
use spm::error::Error;
use spm::model::name::SourceName;
use spm::ops::source::add_source;
use spm::ops::update::{Which, update};
use spm::store::Store;
use spm::store::index;
use support::net::Fake;
use support::{Built, dependent, index_of, plain};

fn name(text: &str) -> SourceName {
    SourceName::parse(text).unwrap()
}

/// Publish an index for `source` at `at`, listing the given packages.
fn publish(fake: &Fake, source: &str, at: &str, packages: &[&Built]) -> String {
    let entries: Vec<(&Built, String)> = packages
        .iter()
        .map(|built| (*built, fake.url_for("p.tar.gz")))
        .collect();
    support::serve_index(fake, at, &index_of(source, &entries), support::test_key())
}

#[test]
fn an_index_that_cannot_be_understood_leaves_the_previous_one_alone() {
    // A device with a stale index can still install. A device with half an
    // index can do nothing at all.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let fake = Fake::serving(served.path());

    let one = plain(work.path());
    let url = publish(&fake, "sepia", "index.json", &[&one]);
    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();
    let before = std::fs::read(store.index_file(&name("sepia"))).unwrap();

    // The same URL now serves nonsense.
    let body = "<html>the server is having a day</html>";
    fake.serve("index.json", body.as_bytes());
    fake.serve(
        "index.json.sig",
        format!("{}\n", support::test_key().sign_index(body.as_bytes())).as_bytes(),
    );
    let report = update(&store, &fake, &Which::All).unwrap();

    assert_eq!(report.failed.len(), 1);
    assert!(report.updated.is_empty());
    assert_eq!(
        std::fs::read(store.index_file(&name("sepia"))).unwrap(),
        before,
        "the previous index was damaged"
    );
    assert!(report.outcome().is_err());
}

#[test]
fn one_source_that_cannot_be_reached_does_not_stop_the_others() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let fake = Fake::serving(served.path());
    let built = plain(work.path());

    let first = publish(&fake, "one", "one.json", &[&built]);
    let second = publish(&fake, "two", "two.json", &[&built]);
    let third = publish(&fake, "three", "three.json", &[&built]);
    for url in [&first, &second, &third] {
        add_source(&store, &fake, url, &support::test_public_key(), None, false).unwrap();
    }

    fake.break_url(&second);
    let report = update(&store, &fake, &Which::All).unwrap();

    assert_eq!(
        report.updated.len(),
        2,
        "the other two should still be done"
    );
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].name.as_str(), "two");
    assert_eq!(report.failed[0].url, second);

    // And the command as a whole fails, naming what was missed.
    match report.outcome() {
        Err(Error::Incomplete { failed, total }) => {
            assert_eq!(failed, vec!["two".to_owned()]);
            assert_eq!(total, 3);
        }
        other => panic!("expected Incomplete, got {other:?}"),
    }
}

#[test]
fn both_options_together_is_a_usage_error() {
    // clap enforces it, which is why this checks the definition rather than
    // the command: a wrong command line never reaches `ops`.
    let outcome = Cli::try_parse_from(["spm", "update", "--all", "--source", "sepia"]);
    assert!(
        outcome.is_err(),
        "--all and --source were accepted together"
    );

    // And each on its own is fine.
    assert!(Cli::try_parse_from(["spm", "update", "--all"]).is_ok());
    assert!(Cli::try_parse_from(["spm", "update", "--source", "sepia"]).is_ok());
}

#[test]
fn neither_option_means_all_of_them() {
    let parsed = Cli::try_parse_from(["spm", "update"]).unwrap();
    match parsed.command {
        Command::Update(args) => {
            assert!(!args.all);
            assert!(args.source.is_none());
        }
        other => panic!("expected update, got {other:?}"),
    }
}

#[test]
fn a_named_source_is_the_only_one_fetched() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let fake = Fake::serving(served.path());
    let built = plain(work.path());
    let first = publish(&fake, "one", "one.json", &[&built]);
    let second = publish(&fake, "two", "two.json", &[&built]);
    add_source(
        &store,
        &fake,
        &first,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();
    add_source(
        &store,
        &fake,
        &second,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    let before = fake.asked().len();
    let report = update(&store, &fake, &Which::One(name("two"))).unwrap();

    assert_eq!(report.updated.len(), 1);
    assert_eq!(report.updated[0].name.as_str(), "two");
    // The index and its signature, and nothing belonging to the other source.
    // The signature is fetched every time an index is: an index is not read
    // until it has been checked, so the two are one request as far as this is
    // concerned.
    let asked: Vec<String> = fake.asked().into_iter().skip(before).collect();
    assert_eq!(
        asked,
        vec![second.clone(), format!("{second}.sig")],
        "it fetched something else as well"
    );
}

#[test]
fn naming_a_source_that_is_not_configured_says_so() {
    let device = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let fake = Fake::serving(served.path());

    match update(&store, &fake, &Which::One(name("nothing"))) {
        Err(Error::SourceNotFound { reference }) => assert_eq!(reference.as_str(), "nothing"),
        other => panic!("expected SourceNotFound, got {other:?}"),
    }
}

#[test]
fn what_is_new_since_the_last_time_is_counted() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let fake = Fake::serving(served.path());

    let one = plain(work.path());
    let url = publish(&fake, "sepia", "index.json", &[&one]);
    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    // The source publishes a second package.
    let two = dependent(work.path());
    publish(&fake, "sepia", "index.json", &[&one, &two]);
    let report = update(&store, &fake, &Which::All).unwrap();

    assert_eq!(report.updated[0].packages, 2);
    assert_eq!(report.updated[0].new_packages, 1);
}

#[test]
fn a_source_with_nothing_to_update_is_not_a_failure() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let fake = Fake::serving(served.path());
    let one = plain(work.path());
    let url = publish(&fake, "sepia", "index.json", &[&one]);
    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    let report = update(&store, &fake, &Which::All).unwrap();

    assert_eq!(report.updated.len(), 1);
    assert_eq!(report.updated[0].new_packages, 0);
    assert!(report.outcome().is_ok());
}

#[test]
fn a_device_with_no_sources_updates_nothing_and_succeeds() {
    let device = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let fake = Fake::serving(served.path());

    let report = update(&store, &fake, &Which::All).unwrap();

    assert!(report.updated.is_empty());
    assert!(report.failed.is_empty());
    assert!(report.outcome().is_ok());
}

/// The index a source publishes is stored as it was fetched.
#[test]
fn the_index_is_stored_as_it_arrived() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = Store::at(device.path());
    let fake = Fake::serving(served.path());
    let one = plain(work.path());
    let url = publish(&fake, "sepia", "index.json", &[&one]);
    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    update(&store, &fake, &Which::All).unwrap();

    let stored = index::read(&store, &name("sepia")).unwrap().unwrap();
    assert_eq!(stored.name.as_str(), "sepia");
    assert_eq!(stored.packages.len(), 1);
    assert_eq!(stored.packages[0].name.as_str(), "plain");
}
