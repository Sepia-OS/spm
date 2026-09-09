/*
  sources.rs

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

//! The read-only source commands.
//!
//! These are what makes everything after them inspectable by hand, so they
//! come first — and the distinction they have to keep is between a source
//! whose index has never been fetched and one that offers nothing. The two
//! look alike in a listing and mean opposite things.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use spm::error::Error;
use spm::model::installed::{Reason, Record};
use spm::model::name::SourceName;
use spm::model::name::SourceRef;
use spm::ops::query::{list_sources, source_info};
use spm::ops::source::{add_source, remove_source};
use spm::store::config::{Source, Sources};
use spm::store::db::Database;
use spm::store::{Store, index};
use support::net::Fake;
use support::{Built, dependent, index_of, plain};

fn name(text: &str) -> SourceName {
    SourceName::parse(text).unwrap()
}

fn configure(store: &Store, entries: &[(&str, &str, bool)]) {
    let mut sources = Sources::default();
    for (source, url, is_default) in entries {
        sources.insert(Source {
            name: name(source),
            url: (*url).to_owned(),
            is_default: *is_default,
            key: support::test_public_key(),
        });
    }
    sources.save(store).unwrap();
}

/// Put an index for a source in place, as `update` will.
fn fetched(store: &Store, source: &str, packages: &[(&Built, String)]) {
    let text = index_of(source, packages);
    let parsed = serde_json::from_str(&text).unwrap();
    index::write(store, &name(source), &parsed).unwrap();
}

/// Record a package as installed from a source.
fn installed_from(store: &Store, built: &Built, source: &str) {
    let record = Record {
        metadata: built.packed_metadata(),
        source: name(source),
        reason: Reason::Explicit,
        installed_at: 1,
        files: Vec::new(),
        digests: BTreeMap::new(),
    };
    let db = Database::new(store);
    db.begin(&record).unwrap();
    db.commit(&record.metadata.name).unwrap();
}

fn store_at(directory: &Path) -> Store {
    Store::at(directory)
}

#[test]
fn a_device_with_no_sources_is_not_an_error() {
    // What a freshly installed card looks like. The command has to say so
    // rather than fail, because there is nothing wrong.
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());

    let reports = list_sources(&store).unwrap();

    assert!(reports.is_empty());
}

#[test]
fn a_source_that_has_never_been_updated_says_so() {
    // Not "no packages": nobody has looked yet, and the difference is the
    // whole reason this is an Option.
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[("sepia", "https://example.test/index.json", true)],
    );

    let reports = list_sources(&store).unwrap();

    assert_eq!(reports.len(), 1);
    assert!(
        reports[0].index.is_none(),
        "a never-fetched index should not look like an empty one"
    );
    assert_eq!(reports[0].installed, 0);
}

#[test]
fn a_source_that_has_been_updated_says_what_it_offers() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[("sepia", "https://example.test/index.json", true)],
    );

    let one = plain(work.path());
    let two = dependent(work.path());
    fetched(
        &store,
        "sepia",
        &[
            (&one, "https://example.test/plain.tar.gz".to_owned()),
            (&two, "https://example.test/dependent.tar.gz".to_owned()),
        ],
    );

    let reports = list_sources(&store).unwrap();
    let summary = reports[0].index.expect("the index has been fetched");

    assert_eq!(summary.packages, 2);
    assert_eq!(summary.updated, 1_757_260_800);
}

#[test]
fn two_sources_are_listed_by_name_with_one_default() {
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[
            ("sepia", "https://example.test/index.json", true),
            ("local", "https://example.invalid/index.json", false),
        ],
    );

    let reports = list_sources(&store).unwrap();

    let names: Vec<&str> = reports.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["local", "sepia"], "listed by name");
    assert_eq!(reports.iter().filter(|r| r.is_default).count(), 1);
    assert!(
        reports
            .iter()
            .find(|r| r.name.as_str() == "sepia")
            .unwrap()
            .is_default
    );
}

#[test]
fn a_source_says_how_many_installed_packages_came_from_it() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[
            ("sepia", "https://example.test/index.json", true),
            ("local", "https://example.invalid/index.json", false),
        ],
    );

    installed_from(&store, &plain(work.path()), "sepia");
    installed_from(&store, &dependent(work.path()), "sepia");

    let reports = list_sources(&store).unwrap();
    let sepia = reports.iter().find(|r| r.name.as_str() == "sepia").unwrap();
    let local = reports.iter().find(|r| r.name.as_str() == "local").unwrap();

    assert_eq!(sepia.installed, 2);
    assert_eq!(local.installed, 0);
}

#[test]
fn one_source_can_be_asked_about_by_its_url() {
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[
            ("sepia", "https://example.test/index.json", true),
            ("local", "https://example.invalid/index.json", false),
        ],
    );

    let report = source_info(
        &store,
        &SourceRef::parse("https://example.invalid/index.json"),
    )
    .unwrap();

    assert_eq!(report.name.as_str(), "local");
    assert!(!report.is_default);
}

#[test]
fn one_source_can_be_asked_about_by_its_name() {
    // The point of the change: every other command takes a name, and now these
    // do too. The URL still works, which is what the test above holds down.
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[
            ("sepia", "https://example.test/index.json", true),
            ("local", "https://example.invalid/index.json", false),
        ],
    );

    let report = source_info(&store, &SourceRef::parse("local")).unwrap();

    assert_eq!(report.name.as_str(), "local");
    assert_eq!(report.url, "https://example.invalid/index.json");
    assert!(!report.is_default);
}

#[test]
fn a_name_and_its_url_are_two_ways_to_the_same_source() {
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[("sepia", "https://example.test/index.json", true)],
    );

    let by_name = source_info(&store, &SourceRef::parse("sepia")).unwrap();
    let by_url = source_info(&store, &SourceRef::parse("https://example.test/index.json")).unwrap();
    assert_eq!(by_name, by_url);
}

#[test]
fn asking_about_a_name_that_is_not_configured_says_named_rather_than_at() {
    // The two failures are different sentences. A mistyped name is not a
    // malformed URL, and saying so is the whole reason the reference is carried
    // into the error rather than a bare string.
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[("sepia", "https://example.test/index.json", true)],
    );

    match source_info(&store, &SourceRef::parse("sepiaa")) {
        Err(error @ Error::SourceNotFound { .. }) => {
            let said = error.to_string();
            assert!(said.contains("no source named 'sepiaa'"), "{said}");
        }
        other => panic!("expected SourceNotFound, got {other:?}"),
    }

    match source_info(&store, &SourceRef::parse("https://nowhere.test/index.json")) {
        Err(error @ Error::SourceNotFound { .. }) => {
            let said = error.to_string();
            assert!(
                said.contains("no source at 'https://nowhere.test/index.json'"),
                "{said}"
            );
        }
        other => panic!("expected SourceNotFound, got {other:?}"),
    }
}

#[test]
fn a_source_can_be_removed_by_its_name() {
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[
            ("sepia", "https://example.test/index.json", true),
            ("local", "https://example.invalid/index.json", false),
        ],
    );

    let removed = remove_source(&store, &SourceRef::parse("local")).unwrap();

    assert_eq!(removed.name.as_str(), "local");
    assert!(source_info(&store, &SourceRef::parse("local")).is_err());
    // And it took only the one it was asked for.
    assert!(source_info(&store, &SourceRef::parse("sepia")).is_ok());
}

#[test]
fn asking_about_a_url_that_is_not_configured_says_so() {
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[("sepia", "https://example.test/index.json", true)],
    );

    match source_info(&store, &SourceRef::parse("https://nowhere.test/index.json")) {
        Err(Error::SourceNotFound { reference }) => {
            assert_eq!(reference.as_str(), "https://nowhere.test/index.json");
        }
        other => panic!("expected SourceNotFound, got {other:?}"),
    }
}

#[test]
fn a_package_keeps_its_source_after_that_source_is_removed() {
    // remove-source does not uninstall anything, so a record can name a
    // source that is no longer configured. Counting must not fall over.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    configure(
        &store,
        &[("sepia", "https://example.test/index.json", true)],
    );
    installed_from(&store, &plain(work.path()), "gone");

    let reports = list_sources(&store).unwrap();

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].installed, 0, "that package came from elsewhere");
}

/// A source serving an index that names itself, with two packages in it.
fn published(fake: &Fake, work: &Path, source: &str, at: &str) -> String {
    let one = plain(work);
    let two = dependent(work);
    let text = index_of(
        source,
        &[
            (&one, fake.url_for("plain.tar.gz")),
            (&two, fake.url_for("dependent.tar.gz")),
        ],
    );
    support::serve_index(fake, at, &text, support::test_key())
}

#[test]
fn the_first_source_added_becomes_the_default() {
    // A lone source is the only one it could be, so it does not have to be
    // asked for.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let url = published(&fake, work.path(), "sepia", "index.json");

    let added = add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    assert_eq!(
        added.name.as_str(),
        "sepia",
        "the name comes from the index"
    );
    assert!(added.is_default);
    assert_eq!(added.packages, 2);
    assert!(!added.replaced);
}

#[test]
fn the_index_is_fetched_so_the_source_is_usable_at_once() {
    // No `update` first: that is the point of fetching before writing.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let url = published(&fake, work.path(), "sepia", "index.json");

    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    let reports = list_sources(&store).unwrap();
    let summary = reports[0].index.expect("the index should already be here");
    assert_eq!(summary.packages, 2);
}

#[test]
fn re_adding_a_url_updates_its_entry_rather_than_duplicating_it() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let url = published(&fake, work.path(), "sepia", "index.json");

    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();
    let again = add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    assert!(again.replaced);
    assert_eq!(list_sources(&store).unwrap().len(), 1);
    assert!(again.is_default, "re-adding must not quietly drop the flag");
}

#[test]
fn the_default_flag_moves_to_the_source_that_asks_for_it() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let first = published(&fake, work.path(), "sepia", "one.json");
    let second = published(&fake, work.path(), "local", "two.json");

    add_source(
        &store,
        &fake,
        &first,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();
    let moved = add_source(
        &store,
        &fake,
        &second,
        &support::test_public_key(),
        None,
        true,
    )
    .unwrap();

    assert!(moved.is_default);
    let reports = list_sources(&store).unwrap();
    assert_eq!(reports.iter().filter(|r| r.is_default).count(), 1);
    assert!(
        reports
            .iter()
            .find(|r| r.name.as_str() == "local")
            .unwrap()
            .is_default
    );
}

#[test]
fn a_name_another_url_already_holds_is_refused_and_says_who_holds_it() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let first = published(&fake, work.path(), "sepia", "one.json");
    // A second source that calls itself the same thing, which it is free to.
    let second = published(&fake, work.path(), "sepia", "two.json");

    add_source(
        &store,
        &fake,
        &first,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    match add_source(
        &store,
        &fake,
        &second,
        &support::test_public_key(),
        None,
        false,
    ) {
        Err(Error::Usage(message)) => {
            assert!(message.contains("sepia"), "{message}");
            assert!(
                message.contains(&first),
                "should name the holder: {message}"
            );
            assert!(
                message.contains("--name"),
                "should say the way past: {message}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    assert_eq!(list_sources(&store).unwrap().len(), 1, "nothing was added");
}

#[test]
fn a_name_can_be_given_when_two_sources_call_themselves_the_same_thing() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let first = published(&fake, work.path(), "sepia", "one.json");
    let second = published(&fake, work.path(), "sepia", "two.json");

    add_source(
        &store,
        &fake,
        &first,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();
    let renamed = add_source(
        &store,
        &fake,
        &second,
        &support::test_public_key(),
        Some("mirror"),
        false,
    )
    .unwrap();

    assert_eq!(renamed.name.as_str(), "mirror");
    let reports = list_sources(&store).unwrap();
    assert_eq!(reports.len(), 2);
    // And its index is filed under the name this device knows it by.
    assert!(
        reports
            .iter()
            .find(|r| r.name.as_str() == "mirror")
            .unwrap()
            .index
            .is_some()
    );
}

#[test]
fn a_url_that_is_not_https_is_refused_without_being_fetched() {
    let device = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());

    match add_source(
        &store,
        &fake,
        "http://example.test/index.json",
        &support::test_public_key(),
        None,
        false,
    ) {
        Err(Error::Usage(message)) => assert!(message.contains("https"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }

    assert!(fake.asked().is_empty(), "it was fetched anyway");
    assert!(list_sources(&store).unwrap().is_empty());
}

#[test]
fn a_fetch_that_fails_leaves_the_configuration_as_it_was() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let good = published(&fake, work.path(), "sepia", "one.json");
    add_source(
        &store,
        &fake,
        &good,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    let broken = published(&fake, work.path(), "local", "two.json");
    fake.break_url(&broken);

    assert!(
        add_source(
            &store,
            &fake,
            &broken,
            &support::test_public_key(),
            None,
            false
        )
        .is_err()
    );

    let reports = list_sources(&store).unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].name.as_str(), "sepia");
}

#[test]
fn an_index_that_is_not_an_index_is_refused() {
    let device = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    // Signed, so that what fails is the parse rather than the signature: this
    // test is about an index that is not one, not about a source that cannot
    // sign.
    let body = "<html>not an index at all</html>";
    let url = fake.serve("index.json", body.as_bytes());
    fake.serve(
        "index.json.sig",
        format!("{}\n", support::test_key().sign_index(body.as_bytes())).as_bytes(),
    );

    match add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    ) {
        Err(Error::Parse { .. }) => {}
        other => panic!("expected a parse failure, got {other:?}"),
    }
    assert!(list_sources(&store).unwrap().is_empty());
}

#[test]
fn removing_a_source_does_not_uninstall_anything() {
    // The rule this command exists to keep: removing a source is not a way of
    // uninstalling things.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let url = published(&fake, work.path(), "sepia", "index.json");
    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    let built = plain(work.path());
    installed_from(&store, &built, "sepia");

    let removed = remove_source(&store, &SourceRef::parse(&url)).unwrap();

    assert_eq!(removed.name.as_str(), "sepia");
    assert_eq!(removed.losing_upgrades, 1, "it should say what it costs");

    // Still installed, and still remembering where it came from.
    let record = Database::new(&store)
        .get(&built.packed_metadata().name)
        .unwrap()
        .expect("the package is still installed");
    assert_eq!(record.source.as_str(), "sepia");
}

#[test]
fn removing_a_source_takes_its_index_with_it() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let url = published(&fake, work.path(), "sepia", "index.json");
    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();
    assert!(store.index_file(&name("sepia")).exists());

    remove_source(&store, &SourceRef::parse(&url)).unwrap();

    assert!(!store.index_file(&name("sepia")).exists());
    assert!(list_sources(&store).unwrap().is_empty());
}

#[test]
fn the_default_moves_to_the_last_source_standing() {
    // For the same reason the first source added is the default: a lone
    // source is the only one it could be.
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let first = published(&fake, work.path(), "sepia", "one.json");
    let second = published(&fake, work.path(), "local", "two.json");
    add_source(
        &store,
        &fake,
        &first,
        &support::test_public_key(),
        None,
        true,
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

    let removed = remove_source(&store, &SourceRef::parse(&first)).unwrap();

    assert_eq!(
        removed
            .new_default
            .as_ref()
            .map(spm::model::name::SourceName::as_str),
        Some("local")
    );
    assert!(!removed.without_default);
    let reports = list_sources(&store).unwrap();
    assert!(reports[0].is_default);
}

#[test]
fn with_several_left_the_default_is_not_guessed() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let held = published(&fake, work.path(), "sepia", "one.json");
    let other = published(&fake, work.path(), "local", "two.json");
    let third = published(&fake, work.path(), "mirror", "three.json");
    add_source(
        &store,
        &fake,
        &held,
        &support::test_public_key(),
        None,
        true,
    )
    .unwrap();
    add_source(
        &store,
        &fake,
        &other,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();
    add_source(
        &store,
        &fake,
        &third,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    let removed = remove_source(&store, &SourceRef::parse(&held)).unwrap();

    assert!(removed.new_default.is_none());
    assert!(removed.without_default, "it should say there is now none");
    let reports = list_sources(&store).unwrap();
    assert_eq!(reports.iter().filter(|r| r.is_default).count(), 0);
}

#[test]
fn removing_the_only_source_leaves_nothing_to_promote() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let url = published(&fake, work.path(), "sepia", "index.json");
    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    let removed = remove_source(&store, &SourceRef::parse(&url)).unwrap();

    assert!(removed.new_default.is_none());
    assert!(!removed.without_default, "there is nothing to be without");
    assert!(list_sources(&store).unwrap().is_empty());
}

#[test]
fn removing_a_source_that_is_not_configured_says_so() {
    let device = tempfile::tempdir().unwrap();
    let store = store_at(device.path());

    match remove_source(&store, &SourceRef::parse("https://nowhere.test/index.json")) {
        Err(Error::SourceNotFound { reference }) => {
            assert_eq!(reference.as_str(), "https://nowhere.test/index.json");
        }
        other => panic!("expected SourceNotFound, got {other:?}"),
    }
}

#[test]
fn removing_a_source_nothing_came_from_says_zero() {
    let device = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let store = store_at(device.path());
    let fake = Fake::serving(served.path());
    let url = published(&fake, work.path(), "sepia", "index.json");
    add_source(
        &store,
        &fake,
        &url,
        &support::test_public_key(),
        None,
        false,
    )
    .unwrap();

    assert_eq!(
        remove_source(&store, &SourceRef::parse(&url))
            .unwrap()
            .losing_upgrades,
        0
    );
}
