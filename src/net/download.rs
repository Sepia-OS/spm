/*
  download.rs

  Created on 2026-09-07 by Thomas Bonk <thomas@meandmymac.de>
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

//! Downloading to disk, hashing on the way past.
//!
//! Never into memory. A package is 216 MiB — the Helix one is, and most of it
//! is tree-sitter grammars — and the smallest supported board has 512 MiB of
//! RAM. So the bytes go from the transport to the disk through a fixed buffer,
//! and the digest is taken in the same pass rather than by reading the file
//! back afterwards.
//!
//! A download that fails partway leaves nothing behind. There is no resuming:
//! resuming needs range support and a way to know the partial file belongs to
//! the same object, and the digest is what says we got it right — which a
//! fresh start always satisfies.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256 as Hasher};

use crate::error::{Error, Result};
use crate::model::metadata::Sha256;
use crate::net::transport::Transport;
use crate::store::atomic;

/// What a download produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    /// Where it was written.
    pub path: PathBuf,
    /// What it hashes to. Whether that is the *right* digest is somebody
    /// else's question — this only says what arrived.
    pub sha256: Sha256,
    /// How much of it there was.
    pub bytes: u64,
}

/// Fetch `url` into `into`, hashing as it goes.
///
/// The file appears whole or not at all: it is written beside its destination
/// and renamed into place, so a download interrupted by a power cut cannot be
/// mistaken for a complete one.
///
/// # Errors
///
/// Whatever the transport gives — [`Error::Network`] or [`Error::ClockBehind`]
/// — or [`Error::Io`] if the bytes cannot be written.
pub fn to_file(transport: &dyn Transport, url: &str, into: &Path) -> Result<Downloaded> {
    let mut source = transport.get(url)?;

    let mut hasher = Hasher::new();
    let mut bytes = 0_u64;

    atomic::write_with(into, |file| {
        let mut sink = Hashing {
            inner: file,
            hasher: &mut hasher,
            written: &mut bytes,
        };
        // Bounded by construction: `io::copy` uses a fixed buffer, so what is
        // held at any moment does not depend on how big the file is.
        io::copy(&mut source, &mut sink)?;
        sink.flush()
    })?;

    let digest = hex(&hasher.finalize());
    let sha256 = Sha256::parse(&digest).ok_or_else(|| Error::Parse {
        path: into.to_path_buf(),
        message: format!("the digest of the download came out as '{digest}'"),
    })?;

    Ok(Downloaded {
        path: into.to_path_buf(),
        sha256,
        bytes,
    })
}

/// A writer that hashes and counts on the way through.
struct Hashing<'a, W: Write> {
    inner: &'a mut W,
    hasher: &'a mut Hasher,
    written: &'a mut u64,
}

impl<W: Write> Write for Hashing<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        // Only what was accepted, or the digest describes bytes the file does
        // not contain.
        self.hasher.update(&buffer[..written]);
        *self.written = self.written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Bytes as lower-case hexadecimal.
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Read a whole response as text.
///
/// For an index, which is small and has to be parsed in one go. Not for a
/// package: this is the function that would run a board out of memory, so it
/// takes a limit and refuses beyond it rather than trusting whoever is on the
/// other end.
///
/// # Errors
///
/// Whatever the transport gives, or [`Error::Network`] if the response is
/// longer than `limit`.
pub fn to_string(transport: &dyn Transport, url: &str, limit: u64) -> Result<String> {
    let source = transport.get(url)?;
    let mut text = String::new();
    source
        .take(limit.saturating_add(1))
        .read_to_string(&mut text)
        .map_err(|source| Error::Network {
            url: url.to_owned(),
            message: format!("the response could not be read: {source}"),
        })?;

    if text.len() as u64 > limit {
        return Err(Error::Network {
            url: url.to_owned(),
            message: format!(
                "the response is longer than {limit} bytes, which is more than an index should be"
            ),
        });
    }

    Ok(text)
}
