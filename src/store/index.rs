/*
  index.rs

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

//! `/var/lib/spm/index/` — the local copy of each source's index.
//!
//! Written by `update` and read by everything else. Nothing here fetches: a
//! command is only ever as current as the last `update`, which is the rule
//! `docs/dev/ARCHITECTURE.md` sets and this module is where it is kept.
//!
//! **A source with no local index is not a source with no packages.** The two
//! look alike in a listing and mean opposite things — one has never been
//! fetched, the other offers nothing — so this returns `None` for the first
//! rather than an empty index, and every caller has to say which it means.

use crate::error::{Error, Result};
use crate::model::index::Index;
use crate::model::name::SourceName;
use crate::store::{Store, atomic};

/// Read a source's index, or `None` if it has never been fetched.
///
/// # Errors
///
/// [`Error::Io`] if it is there and unreadable, [`Error::Parse`] if it cannot
/// be understood.
pub fn read(store: &Store, source: &SourceName) -> Result<Option<Index>> {
    let path = store.index_file(source);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::Io { path, source }),
    };

    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| Error::Parse {
            path,
            message: error.to_string(),
        })
}

/// Put a source's index in place, filed under the name this device knows it by.
///
/// That name is not always the one the index declares: `add-source --name`
/// exists because two sources are free to call themselves the same thing. The
/// index is stored exactly as it was fetched — what it calls itself is its
/// business — and the file it goes in is the device's.
///
/// Atomically, so an interrupted `update` leaves the previous index rather
/// than half of the next one.
///
/// # Errors
///
/// [`Error::Io`] if it cannot be written.
pub fn write(store: &Store, source: &SourceName, index: &Index) -> Result<()> {
    let path = store.index_file(source);
    let text = serde_json::to_vec(index).map_err(|error| Error::Parse {
        path: path.clone(),
        message: error.to_string(),
    })?;
    atomic::write(&path, &text)
}

/// Forget a source's index, because the source is gone.
///
/// # Errors
///
/// [`Error::Io`] if it is there and cannot be deleted. One that is already
/// gone is not an error.
pub fn forget(store: &Store, source: &SourceName) -> Result<()> {
    let path = store.index_file(source);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io { path, source }),
    }
}
