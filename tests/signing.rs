/*
  signing.rs

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

//! Signatures: what they stop, which is the only reason they are here.
//!
//! The digests were always enough to catch a download that went wrong. None of
//! these tests is about a download going wrong - every one of them is about a
//! source or a package that is exactly as its publisher meant it to be, and
//! published by the wrong publisher.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use std::fs;

use spm::error::Error;
use spm::model::index::Index;
use spm::model::name::{PackageRef, SourceName, Target};
use spm::ops::install::install;
use spm::ops::source::add_source;
use spm::sign::PrivateKey;
use spm::store::Store;
use spm::store::config::Sources;
use spm::store::index;
use support::net::Fake;
use support::{Built, Package, another_key, index_of, serve_index, test_key, test_public_key};

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
}

fn target() -> Target {
    Target::parse("aarch64-musl").unwrap()
}

/// One package, served, with an index describing it.
fn published(fixture: &Fixture, fake: &Fake, package: Package) -> (Built, String) {
    let built = package.build(fixture.work.path());
    let url = fake.serve_file(
        &format!("{}", built.package.file_name().unwrap().display()),
        &built.package,
    );
    (built, url)
}

#[test]
fn an_index_signed_by_the_pinned_key_is_accepted() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let (built, url) = published(&fixture, &fake, Package::named("helix"));
    let text = index_of("sepia", &[(&built, url)]);
    let index_url = serve_index(&fake, "index.json", &text, test_key());

    let added = add_source(&store, &fake, &index_url, &test_public_key(), None, false).unwrap();

    assert_eq!(added.name.as_str(), "sepia");
    assert_eq!(added.packages, 1);
}

#[test]
fn an_index_signed_by_another_key_is_refused_and_nothing_is_written() {
    // The whole threat: the source is reachable, the index parses, the digests
    // in it are perfectly consistent with the packages it points at. It is
    // simply not the source this device agreed to trust.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let (built, url) = published(&fixture, &fake, Package::named("helix"));
    let text = index_of("sepia", &[(&built, url)]);
    let index_url = serve_index(&fake, "index.json", &text, &another_key());

    let outcome = add_source(&store, &fake, &index_url, &test_public_key(), None, false);

    assert!(
        matches!(outcome, Err(Error::BadSignature { .. })),
        "expected BadSignature, got {outcome:?}"
    );
    // And it kept nothing: no source, no index.
    assert!(Sources::load(&store).unwrap().is_empty());
    assert!(
        index::read(&store, &SourceName::parse("sepia").unwrap())
            .unwrap()
            .is_none()
    );
}

#[test]
fn an_index_changed_after_it_was_signed_is_refused() {
    // A signature over the bytes, not over what they parse to: one byte of
    // difference and it is a different document.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let (built, url) = published(&fixture, &fake, Package::named("helix"));
    let text = index_of("sepia", &[(&built, url)]);

    // Signed honestly, then edited - which is what a taken-over host can do and
    // a taken-over key cannot.
    let signature = test_key().sign_index(text.as_bytes());
    let tampered = text.replace("\"bytes\":", "\"bytes\" :");
    assert_ne!(tampered, text, "the fixture has to actually change");
    let index_url = fake.serve("index.json", tampered.as_bytes());
    fake.serve("index.json.sig", format!("{signature}\n").as_bytes());

    let outcome = add_source(&store, &fake, &index_url, &test_public_key(), None, false);

    assert!(
        matches!(outcome, Err(Error::BadSignature { .. })),
        "expected BadSignature, got {outcome:?}"
    );
}

#[test]
fn an_index_with_no_signature_beside_it_is_refused() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let (built, url) = published(&fixture, &fake, Package::named("helix"));
    let text = index_of("sepia", &[(&built, url)]);
    let index_url = fake.serve("index.json", text.as_bytes());

    let outcome = add_source(&store, &fake, &index_url, &test_public_key(), None, false);

    // A missing signature is a missing file, and the message says which.
    assert!(
        matches!(outcome, Err(Error::Network { .. })),
        "expected the signature to be missing, got {outcome:?}"
    );
}

#[test]
fn a_package_signed_by_a_key_the_index_did_not_name_is_refused() {
    // The second layer. The index verified, so the device believes what it says
    // - including which key may sign this package. A package signed by anybody
    // else is refused even though its own signature is perfectly valid.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();

    let built = Package::named("helix").build(fixture.work.path());
    let url = fake.serve_file("helix.tar.gz", &built.package);

    // An index that names a different key for it.
    let mut parsed: Index = serde_json::from_str(&index_of("sepia", &[(&built, url)])).unwrap();
    parsed.packages[0].versions[0].public_key = another_key().public();
    let text = serde_json::to_string_pretty(&parsed).unwrap();
    let index_url = serve_index(&fake, "index.json", &text, test_key());
    add_source(&store, &fake, &index_url, &test_public_key(), None, false).unwrap();

    let outcome = install(
        &store,
        &fake,
        &PackageRef::parse("helix").unwrap(),
        &target(),
        None,
        false,
    );

    assert!(
        matches!(outcome, Err(Error::BadSignature { .. })),
        "expected BadSignature, got {outcome:?}"
    );
}

#[test]
fn an_unsigned_package_is_refused_however_well_its_digests_check_out() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();

    // Built without a signature, and an index describing it honestly - both
    // digests correct, because nothing about the bytes is wrong.
    let unsigned = support::unsigned_package(fixture.work.path(), "helix");
    let url = fake.serve_file("helix.tar.gz", &unsigned.package);
    let mut parsed: Index = serde_json::from_str(&index_of("sepia", &[(&unsigned, url)])).unwrap();
    parsed.packages[0].versions[0].public_key = test_public_key();
    let text = serde_json::to_string_pretty(&parsed).unwrap();
    let index_url = serve_index(&fake, "index.json", &text, test_key());
    add_source(&store, &fake, &index_url, &test_public_key(), None, false).unwrap();

    let outcome = install(
        &store,
        &fake,
        &PackageRef::parse("helix").unwrap(),
        &target(),
        None,
        false,
    );

    assert!(
        matches!(outcome, Err(Error::BadSignature { .. })),
        "expected BadSignature, got {outcome:?}"
    );
}

#[test]
fn a_properly_signed_package_installs() {
    // The control: everything above fails for a reason, and this is what it
    // looks like when nothing is wrong.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let (built, url) = published(&fixture, &fake, Package::named("helix"));
    let text = index_of("sepia", &[(&built, url)]);
    let index_url = serve_index(&fake, "index.json", &text, test_key());
    add_source(&store, &fake, &index_url, &test_public_key(), None, false).unwrap();

    let outcome = install(
        &store,
        &fake,
        &PackageRef::parse("helix").unwrap(),
        &target(),
        None,
        false,
    )
    .unwrap();

    assert!(outcome.changed);
}

#[test]
fn a_key_written_by_keygen_signs_an_index_that_verifies() {
    // The round trip somebody running a source actually does: make a key, sign
    // the index with it, and hand the public half to a device.
    let work = tempfile::tempdir().unwrap();
    let (private, public) = PrivateKey::generate().unwrap();
    let key_file = work.path().join("source.key");
    fs::write(&key_file, format!("{private}\n")).unwrap();

    let index_file = work.path().join("index.json");
    let text = r#"{"name":"sepia","updated":1,"packages":[]}"#;
    fs::write(&index_file, text).unwrap();

    let signed = spm::ops::create::sign_index(&index_file, &key_file).unwrap();

    assert_eq!(signed.public_key, public);
    let written = fs::read_to_string(&signed.signature).unwrap();
    let signature = spm::sign::Signature::parse(written.trim()).unwrap();
    spm::sign::verify_index(&public, &signature, text.as_bytes(), "sepia").unwrap();
}

#[test]
fn signing_something_that_is_not_an_index_is_refused() {
    // Better here, once, than on every device that fetches it.
    let work = tempfile::tempdir().unwrap();
    let (private, _) = PrivateKey::generate().unwrap();
    let key_file = work.path().join("source.key");
    fs::write(&key_file, private).unwrap();
    let not_an_index = work.path().join("index.json");
    fs::write(&not_an_index, b"<html>not an index</html>").unwrap();

    let outcome = spm::ops::create::sign_index(&not_an_index, &key_file);

    assert!(
        matches!(outcome, Err(Error::Parse { .. })),
        "expected Parse, got {outcome:?}"
    );
    assert!(!work.path().join("index.json.sig").exists());
}

#[test]
fn a_key_file_that_is_not_a_key_says_so_rather_than_signing_nothing() {
    let work = tempfile::tempdir().unwrap();
    let key_file = work.path().join("source.key");
    fs::write(&key_file, b"this is not a key\n").unwrap();
    let index_file = work.path().join("index.json");
    fs::write(
        &index_file,
        br#"{"name":"sepia","updated":1,"packages":[]}"#,
    )
    .unwrap();

    let outcome = spm::ops::create::sign_index(&index_file, &key_file);

    assert!(
        matches!(outcome, Err(Error::Signing(_))),
        "expected Signing, got {outcome:?}"
    );
}

#[test]
fn re_adding_a_source_moves_the_pinned_key() {
    // How a key is rotated, and the user guide says so: the source signs its
    // index with the new key, and every device re-adds the same URL with the
    // new public half. Nothing else has to be taken apart first.
    let fixture = Fixture::new();
    let store = fixture.store();
    let fake = fixture.transport();
    let (built, url) = published(&fixture, &fake, Package::named("helix"));
    let text = index_of("sepia", &[(&built, url)]);

    // Added against the first key.
    let index_url = serve_index(&fake, "index.json", &text, test_key());
    add_source(&store, &fake, &index_url, &test_public_key(), None, false).unwrap();

    // The source rotates: the same index, signed by a new key.
    let rotated = another_key();
    fake.serve(
        "index.json.sig",
        format!("{}\n", rotated.sign_index(text.as_bytes())).as_bytes(),
    );

    // The old pin now refuses it, which is the point of pinning.
    let stale = add_source(&store, &fake, &index_url, &test_public_key(), None, false);
    assert!(
        matches!(stale, Err(Error::BadSignature { .. })),
        "the old key must stop working, got {stale:?}"
    );

    // Re-adding with the new key updates the pin rather than duplicating it.
    add_source(&store, &fake, &index_url, &rotated.public(), None, false).unwrap();

    let sources = Sources::load(&store).unwrap();
    assert_eq!(sources.iter().count(), 1, "one source, not two");
    let pinned = sources
        .by_name(&SourceName::parse("sepia").unwrap())
        .expect("the source is still configured");
    assert_eq!(pinned.key, rotated.public());
}
