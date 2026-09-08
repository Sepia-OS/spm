/*
  transport.rs

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

//! That the seam works, and that a test can serve an index through it.
//!
//! Nothing here touches a network, which is the point of the trait: every
//! layer above takes a `&dyn Transport`, so the code a test drives is the code
//! that runs on a device.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use std::io::Read;

use spm::error::Error;
use spm::model::index::Index;
use spm::model::name::{PackageName, Target};
use spm::net::transport::Transport;
use support::net::Fake;
use support::{dependent, index_of, plain};

fn read(transport: &dyn Transport, url: &str) -> String {
    let mut body = String::new();
    transport
        .get(url)
        .expect("the fake should serve this")
        .read_to_string(&mut body)
        .expect("the body is text");
    body
}

#[test]
fn a_fixture_index_can_be_read_through_the_transport() {
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());

    // Two packages, published where the index will say they are.
    let one = plain(work.path());
    let two = dependent(work.path());
    let one_url = fake.serve_file("packages/plain.tar.gz", &one.package);
    let two_url = fake.serve_file("packages/dependent.tar.gz", &two.package);
    let index_url = fake.serve(
        "index.json",
        index_of("sepia", &[(&one, one_url.clone()), (&two, two_url)]).as_bytes(),
    );

    // The whole point: this is how `update` will read an index.
    let index: Index = serde_json::from_str(&read(&fake, &index_url)).unwrap();

    assert_eq!(index.name.as_str(), "sepia");
    assert_eq!(index.packages.len(), 2);

    let target = Target::parse("aarch64-musl").unwrap();
    let entry = index
        .newest(&PackageName::parse("plain").unwrap(), &target)
        .expect("the index offers plain for this target");
    assert_eq!(entry.version.as_str(), "1.0.0");
    assert_eq!(entry.url, one_url);
    assert_eq!(entry.sha256.as_str(), one.sha256);
}

#[test]
fn what_is_served_is_what_comes_back() {
    let served = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    let url = fake.serve("hello.txt", b"the bytes");

    assert_eq!(read(&fake, &url), "the bytes");
}

#[test]
fn a_package_comes_back_whole() {
    // Not text: the transport streams bytes, and a package is bytes.
    let work = tempfile::tempdir().unwrap();
    let served = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    let built = plain(work.path());
    let url = fake.serve_file("plain.tar.gz", &built.package);

    let mut bytes = Vec::new();
    fake.get(&url).unwrap().read_to_end(&mut bytes).unwrap();

    assert_eq!(bytes, std::fs::read(&built.package).unwrap());
    assert_eq!(support::digest(&bytes), built.sha256);
}

#[test]
fn a_url_that_is_not_served_is_a_network_error() {
    let served = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());

    // The Ok side is a `Box<dyn Read>` and cannot be printed, so the arms are
    // split rather than formatted.
    match fake.get("https://example.test/nothing.json") {
        Err(Error::Network { url, .. }) => assert_eq!(url, "https://example.test/nothing.json"),
        Err(other) => panic!("expected a network error, got {other:?}"),
        Ok(_) => panic!("expected a failure, got a body"),
    }
}

#[test]
fn a_url_can_be_made_to_fail() {
    // What Step 24 needs: one source of several unreachable, the rest fine.
    let served = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    let good = fake.serve("good.json", b"{}");
    let bad = fake.serve("bad.json", b"{}");
    fake.break_url(&bad);

    assert_eq!(read(&fake, &good), "{}");
    assert!(fake.get(&bad).is_err());
    // And it stays broken.
    assert!(fake.get(&bad).is_err());
}

#[test]
fn the_fake_remembers_what_was_asked_for() {
    let served = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    let url = fake.serve("index.json", b"{}");

    assert!(fake.asked().is_empty());
    let _ = fake.get(&url);
    let _ = fake.get("https://example.test/missing");

    assert_eq!(
        fake.asked(),
        vec![url, "https://example.test/missing".to_owned()]
    );
}
