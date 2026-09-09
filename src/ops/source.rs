/*
  source.rs

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

//! The commands that change which sources are configured:
//! `add-source` and `remove-source`.
//!
//! Split from `query` because these write, and writing is the part that needs
//! care.

use crate::error::{Error, Result};
use crate::model::index::Index;
use crate::model::name::{SourceName, SourceRef};
use crate::net::download;
use crate::net::transport::Transport;
use crate::sign::{self, PublicKey, Signature};
use crate::store::config::{Source, Sources};
use crate::store::db::Database;
use crate::store::{Store, index};

/// The most an index may be.
///
/// The other end decides how much it sends; this decides how much is read. A
/// few thousand packages with a few versions each is a small fraction of this,
/// and a board with 512 MiB of RAM has to parse it in one go.
pub const INDEX_LIMIT: u64 = 32 * 1024 * 1024;

/// What adding a source did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// The name the device now knows it by.
    pub name: SourceName,
    /// Where its index is published.
    pub url: String,
    /// Whether it is now the default.
    pub is_default: bool,
    /// How many packages its index offers.
    pub packages: usize,
    /// Whether this updated an entry that was already there.
    pub replaced: bool,
}

/// Fetch and parse a source's index.
///
/// Shared with `update`, so that "what is an index and how is one read" has
/// one answer.
///
/// # Errors
///
/// [`Error::Network`] or [`Error::ClockBehind`] if it cannot be fetched,
/// [`Error::Parse`] if it cannot be understood.
pub fn fetch_index(transport: &dyn Transport, url: &str, key: &PublicKey) -> Result<Index> {
    let text = download::to_string(transport, url, INDEX_LIMIT)?;

    // The signature is fetched from beside the index and checked **before the
    // index is parsed**. Parsing first would mean deciding what the document
    // says before knowing whether to believe any of it, and every field in it -
    // which packages exist, where they are downloaded from, which keys signed
    // them - is a thing an unsigned index could lie about.
    let signature_url = format!("{url}{SIGNATURE_SUFFIX}");
    let signature_text = download::to_string(transport, &signature_url, SIGNATURE_LIMIT)?;
    let signature = Signature::parse(signature_text.trim()).ok_or_else(|| Error::Parse {
        path: std::path::PathBuf::from(&signature_url),
        message: format!(
            "it is not an Ed25519 signature: {} lower-case hexadecimal characters",
            Signature::BYTES * 2
        ),
    })?;
    sign::verify_index(key, &signature, text.as_bytes(), url)?;

    serde_json::from_str(&text).map_err(|error| Error::Parse {
        path: std::path::PathBuf::from(url),
        message: error.to_string(),
    })
}

/// What is appended to an index's URL to reach its signature.
const SIGNATURE_SUFFIX: &str = ".sig";

/// The largest a signature file may be: 128 characters and some whitespace.
const SIGNATURE_LIMIT: u64 = 1024;

/// Add a source, or update the one already at this URL.
///
/// The index is fetched **before anything is written**. That establishes the
/// URL is a source at all rather than a typo that would surface at the next
/// `update`; it is where the name comes from; and it leaves the source usable
/// straight away, without an `update` first.
///
/// # Errors
///
/// [`Error::Network`] if the index cannot be fetched, [`Error::Parse`] if it
/// cannot be understood, [`Error::Usage`] if the URL is not `https` or the
/// name is already held by a different URL. Nothing is written in any of those
/// cases.
pub fn add_source(
    store: &Store,
    transport: &dyn Transport,
    url: &str,
    key: &PublicKey,
    name: Option<&str>,
    make_default: bool,
) -> Result<Added> {
    // Before the fetch, so that a refusal costs nothing and tells nobody what
    // this device was about to ask for.
    if !url.starts_with("https://") {
        return Err(Error::Usage(format!(
            "'{url}' is not an https URL - an index decides which packages a device installs and where it fetches them from, so it is not read over a channel that anyone in the way can rewrite"
        )));
    }

    // Fetched *and verified* against the key being pinned, before anything is
    // written. So adding a source is also the first proof that the key is the
    // right one: a mistyped key fails here, where nothing has been kept, rather
    // than at the next `update` on a device that already trusts it.
    let index = fetch_index(transport, url, key)?;

    // The name the index declares, unless the user gave one. Two sources are
    // free to call themselves the same thing, which is what `--name` is for.
    let name = match name {
        Some(given) => SourceName::parse(given)
            .map_err(|reason| Error::Usage(format!("'{given}': {reason}")))?,
        None => index.name.clone(),
    };

    let mut sources = Sources::load(store)?;

    // A name has to identify one source, because it is how every other command
    // refers to one and how its index is filed.
    if let Some(holder) = sources.by_name(&name)
        && holder.url != url
    {
        return Err(Error::Usage(format!(
            "the name '{name}' is already held by {} - give this one another name with --name",
            holder.url
        )));
    }

    let existing = sources.by_url(url).cloned();
    let replaced = existing.is_some();
    // The first source added is the default whether or not it was asked for,
    // since a lone source is the only one it could be. Re-adding a source that
    // already holds the flag does not take it away.
    let is_default =
        make_default || sources.is_empty() || existing.is_some_and(|source| source.is_default);

    // The index first: a configured source whose index is missing is a source
    // that needs an `update`, which is recoverable. The other way round leaves
    // an index nothing refers to.
    index::write(store, &name, &index)?;

    sources.insert(Source {
        name: name.clone(),
        url: url.to_owned(),
        is_default,
        key: key.clone(),
    });
    sources.save(store)?;

    Ok(Added {
        name,
        url: url.to_owned(),
        is_default,
        packages: index.packages.len(),
        replaced,
    })
}

/// What removing a source did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// The name it was known by, which installed packages still record.
    pub name: SourceName,
    /// Where its index was published.
    pub url: String,
    /// How many installed packages came from it.
    ///
    /// They are not touched. What they lose is upgrades: nothing is left to
    /// say a newer version exists.
    pub losing_upgrades: usize,
    /// The source that became the default because this one held it.
    pub new_default: Option<SourceName>,
    /// Whether the device is now left with no default at all.
    pub without_default: bool,
}

/// Remove a source: its entry, and its local index.
///
/// **Packages installed from it are not touched.** Removing a source is not a
/// way of uninstalling things: they stay installed and keep the source name
/// recorded against them, which is why a record can name a source that is no
/// longer configured.
///
/// If the removed source held the default flag, the flag moves rather than
/// disappearing quietly: to the last remaining source if there is exactly one,
/// for the same reason the first source added is the default. With several
/// left there is no default until one is named, and the caller says so.
///
/// # Errors
///
/// [`Error::SourceNotFound`] if no configured source has that name or URL, or
/// whatever reading or writing the state gives.
pub fn remove_source(store: &Store, reference: &SourceRef) -> Result<Removed> {
    let mut sources = Sources::load(store)?;

    let Some(going) = sources.by_ref(reference).cloned() else {
        return Err(Error::SourceNotFound {
            reference: reference.clone(),
        });
    };

    // Counted before anything changes, and reported: somebody removing a
    // source should be told what it costs, even though it costs no files.
    let losing_upgrades = Database::new(store)
        .all()?
        .iter()
        .filter(|record| record.source == going.name)
        .count();

    // The index first, as in `add_source`: if the write below fails, nothing
    // has really been removed and an `update` puts the index back.
    index::forget(store, &going.name)?;

    // The entry it gives back is the one already in hand, above.
    drop(sources.remove(&going.name));

    let mut new_default = None;
    let mut without_default = false;
    if going.is_default {
        let remaining: Vec<SourceName> = sources.iter().map(|source| source.name.clone()).collect();
        match remaining.as_slice() {
            [] => {}
            [only] => {
                sources.set_default(only);
                new_default = Some(only.clone());
            }
            // More than one, and nothing here can guess which. Saying so beats
            // choosing for somebody.
            _ => without_default = true,
        }
    }

    sources.save(store)?;

    Ok(Removed {
        name: going.name,
        url: going.url,
        losing_upgrades,
        new_default,
        without_default,
    })
}
