/*
  download.rs

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

//! That a download streams, hashes as it goes, and leaves nothing behind when
//! it fails.
//!
//! The size here is not decoration. A package is 216 MiB and the smallest
//! supported board has 512 MiB of RAM, so "it works" and "it works without
//! holding the file" are different claims and only the second one matters.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

mod support;

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use spm::error::{Error, Result};
use spm::net::download;
use spm::net::transport::Transport;
use support::net::Fake;

/// Fifty megabytes, which is what the step asks for and about three times the
/// helix package compressed.
const LARGE: usize = 50 * 1024 * 1024;

/// A transport that watches how much is asked for at a time.
struct Metered<'a> {
    inner: &'a dyn Transport,
    largest: Arc<AtomicUsize>,
}

impl Transport for Metered<'_> {
    fn get(&self, url: &str) -> Result<Box<dyn Read>> {
        Ok(Box::new(MeteredReader {
            inner: self.inner.get(url)?,
            largest: Arc::clone(&self.largest),
        }))
    }
}

struct MeteredReader {
    inner: Box<dyn Read>,
    largest: Arc<AtomicUsize>,
}

impl Read for MeteredReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.largest.fetch_max(buffer.len(), Ordering::SeqCst);
        self.inner.read(buffer)
    }
}

/// A transport whose body dies partway through.
struct Flaky {
    good_bytes: usize,
}

impl Transport for Flaky {
    fn get(&self, _url: &str) -> Result<Box<dyn Read>> {
        Ok(Box::new(FlakyReader {
            left: self.good_bytes,
        }))
    }
}

struct FlakyReader {
    left: usize,
}

impl Read for FlakyReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "the network went",
            ));
        }
        let giving = self.left.min(buffer.len());
        buffer[..giving].fill(b'x');
        self.left -= giving;
        Ok(giving)
    }
}

/// Write a large file without holding it in memory, and say what it hashes to.
fn large_file(path: &std::path::Path) -> String {
    let mut file = File::create(path).unwrap();
    let mut hasher = Sha256::new();
    let block = vec![b'p'; 1024 * 1024];
    for _ in 0..(LARGE / block.len()) {
        file.write_all(&block).unwrap();
        hasher.update(&block);
    }
    file.flush().unwrap();
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn a_large_file_arrives_whole_and_hashes_to_what_it_should() {
    let served = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());

    let expected = large_file(&served.path().join("big.tar.gz"));
    let url = fake.url_for("big.tar.gz");
    let destination = into.path().join("big.tar.gz");

    let downloaded = download::to_file(&fake, &url, &destination).unwrap();

    assert_eq!(downloaded.sha256.as_str(), expected);
    assert_eq!(downloaded.bytes, LARGE as u64);
    assert_eq!(downloaded.path, destination);
    assert_eq!(fs::metadata(&destination).unwrap().len(), LARGE as u64);
}

#[test]
fn nothing_is_read_in_pieces_the_size_of_the_file() {
    // The claim that matters on a 512 MiB board: what is held at any moment
    // does not depend on how big the download is.
    let served = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    large_file(&served.path().join("big.tar.gz"));

    let largest = Arc::new(AtomicUsize::new(0));
    let metered = Metered {
        inner: &fake,
        largest: Arc::clone(&largest),
    };

    download::to_file(
        &metered,
        &fake.url_for("big.tar.gz"),
        &into.path().join("big.tar.gz"),
    )
    .unwrap();

    let largest = largest.load(Ordering::SeqCst);
    assert!(largest > 0, "nothing was read at all");
    assert!(
        largest <= 1024 * 1024,
        "read {largest} bytes at once from a {LARGE} byte file - the buffer is following the file size"
    );
}

#[test]
fn a_download_that_fails_partway_leaves_nothing_behind() {
    let into = tempfile::tempdir().unwrap();
    let destination = into.path().join("half.tar.gz");

    let outcome = download::to_file(
        &Flaky { good_bytes: 4096 },
        "https://example.test/half.tar.gz",
        &destination,
    );

    assert!(outcome.is_err());
    assert!(!destination.exists(), "half a download was left in place");
    let left: Vec<_> = fs::read_dir(into.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "something was left behind: {left:?}");
}

#[test]
fn a_failed_download_does_not_destroy_what_was_already_there() {
    // A retry of an upgrade must not lose the copy that worked.
    let into = tempfile::tempdir().unwrap();
    let destination = into.path().join("package.tar.gz");
    fs::write(&destination, b"the one that worked").unwrap();

    let outcome = download::to_file(
        &Flaky { good_bytes: 10 },
        "https://example.test/package.tar.gz",
        &destination,
    );

    assert!(outcome.is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"the one that worked");
}

#[test]
fn a_download_replaces_what_was_there() {
    let served = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    let url = fake.serve("package.tar.gz", b"the new one");
    let destination = into.path().join("package.tar.gz");
    fs::write(&destination, b"the old one").unwrap();

    download::to_file(&fake, &url, &destination).unwrap();

    assert_eq!(fs::read(&destination).unwrap(), b"the new one");
}

#[test]
fn an_index_can_be_read_as_text() {
    let served = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    let url = fake.serve("index.json", br#"{"name":"sepia"}"#);

    let text = download::to_string(&fake, &url, 1024).unwrap();

    assert_eq!(text, r#"{"name":"sepia"}"#);
}

#[test]
fn a_response_longer_than_the_limit_is_refused() {
    // The other end decides how much it sends; this decides how much is read.
    let served = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    let url = fake.serve("index.json", &vec![b'x'; 4096]);

    match download::to_string(&fake, &url, 1024) {
        Err(Error::Network { message, .. }) => assert!(message.contains("1024"), "{message}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_transport_failure_is_passed_through_untouched() {
    let served = tempfile::tempdir().unwrap();
    let into = tempfile::tempdir().unwrap();
    let fake = Fake::serving(served.path());
    let url = fake.serve("gone.json", b"{}");
    fake.break_url(&url);

    match download::to_file(&fake, &url, &into.path().join("gone.json")) {
        Err(Error::Network { url: named, .. }) => assert_eq!(named, url),
        other => panic!("expected the transport's error, got {other:?}"),
    }
}
