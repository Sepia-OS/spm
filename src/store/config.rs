/*
  config.rs

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

//! `/etc/spm/sources.json` — the configured package sources.
//!
//! The one file here a person may edit by hand, so it is written pretty-printed
//! and read strictly: a misspelled key is a mistake to report rather than a
//! setting to drop.
//!
//! **An absent file is not an error.** It means no sources are configured,
//! which is what a freshly installed device looks like, and every command that
//! reads it has to say so rather than fail.
//!
//! Two invariants hold over the whole file: names are unique, because a name is
//! how every other command refers to a source and how its index is filed; and
//! at most one source is the default. Reading enforces both, so a hand-edited
//! file that breaks one is reported. Writing cannot break either, because the
//! only ways to change the list maintain them.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::name::{SourceName, SourceRef};
use crate::sign::PublicKey;
use crate::store::{Store, atomic};

/// One configured source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// What this source is called, and what `--source` takes.
    pub name: SourceName,
    /// Where its index is published.
    pub url: String,
    /// Whether this is the source used when a command needs one and none was
    /// given.
    ///
    /// `default` in the file; `default` is a keyword in Rust.
    #[serde(rename = "default", default)]
    pub is_default: bool,
    /// The key this source's index must be signed with.
    ///
    /// Pinned when the source was added, and the root of everything the device
    /// trusts afterwards: the index verifies against this, and the publisher
    /// keys the index carries are believed because the index did. A source that
    /// is later taken over cannot sign an index the device will accept, because
    /// what it lost was the server and not this key.
    ///
    /// Required. A source with no key is a source nothing can be checked
    /// against, and the digests alone were never the point.
    pub key: PublicKey,
}

/// Everything in `sources.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    sources: Vec<Source>,
}

impl Sources {
    /// Read the configuration.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the file is there but cannot be read, and
    /// [`Error::Parse`] if it cannot be understood or breaks one of the two
    /// invariants. A file that is *not there* is not an error: it is a device
    /// with no sources configured, and the answer is an empty list.
    pub fn load(store: &Store) -> Result<Self> {
        let path = store.sources_file();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Sources::default());
            }
            Err(source) => return Err(Error::Io { path, source }),
        };

        let sources: Sources = serde_json::from_str(&text).map_err(|error| Error::Parse {
            path: path.clone(),
            message: error.to_string(),
        })?;

        sources
            .check()
            .map_err(|message| Error::Parse { path, message })?;
        Ok(sources)
    }

    /// Write the configuration, atomically.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if it cannot be written.
    pub fn save(&self, store: &Store) -> Result<()> {
        let path = store.sources_file();
        // Pretty, because somebody may open it in an editor on the device.
        let mut text = serde_json::to_vec_pretty(self).map_err(|error| Error::Parse {
            path: path.clone(),
            message: error.to_string(),
        })?;
        text.push(b'\n');
        atomic::write(&path, &text)
    }

    /// Whether any source is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// How many are configured.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// Every source, in the order the file lists them.
    pub fn iter(&self) -> impl Iterator<Item = &Source> {
        self.sources.iter()
    }

    /// The source with this name.
    #[must_use]
    pub fn by_name(&self, name: &SourceName) -> Option<&Source> {
        self.sources.iter().find(|source| &source.name == name)
    }

    /// The source published at this URL.
    ///
    /// What `add-source` asks before adding one: a URL that is already
    /// configured updates its entry rather than being added twice.
    #[must_use]
    pub fn by_url(&self, url: &str) -> Option<&Source> {
        self.sources.iter().find(|source| source.url == url)
    }

    /// The source addressed by name or by URL, whichever was given.
    ///
    /// What `remove-source` and `source-info` ask, so that neither has to know
    /// which of the two spellings it was handed.
    #[must_use]
    pub fn by_ref(&self, reference: &SourceRef) -> Option<&Source> {
        match reference {
            SourceRef::Name(name) => self.by_name(name),
            SourceRef::Url(url) => self.by_url(url),
        }
    }

    /// The default source, if one is set.
    #[must_use]
    pub fn default_source(&self) -> Option<&Source> {
        self.sources.iter().find(|source| source.is_default)
    }

    /// Add a source, or replace the one that has its name or its URL.
    ///
    /// Replacing rather than appending is what keeps names unique without a
    /// check that could be forgotten. Refusing a name that another URL already
    /// holds is a decision for `add-source`, which can say so properly; this
    /// keeps the file consistent whatever it decides.
    pub fn insert(&mut self, source: Source) {
        self.sources
            .retain(|existing| existing.name != source.name && existing.url != source.url);
        if source.is_default {
            for existing in &mut self.sources {
                existing.is_default = false;
            }
        }
        self.sources.push(source);
    }

    /// Remove a source, returning it if it was there.
    #[must_use]
    pub fn remove(&mut self, name: &SourceName) -> Option<Source> {
        let at = self
            .sources
            .iter()
            .position(|source| &source.name == name)?;
        Some(self.sources.remove(at))
    }

    /// Make this source the default, taking the flag from whichever held it.
    ///
    /// `false` if there is no source by that name, in which case nothing
    /// changed.
    pub fn set_default(&mut self, name: &SourceName) -> bool {
        if self.by_name(name).is_none() {
            return false;
        }
        for source in &mut self.sources {
            source.is_default = &source.name == name;
        }
        true
    }

    /// The two invariants, checked when the file is read.
    fn check(&self) -> std::result::Result<(), String> {
        for (at, source) in self.sources.iter().enumerate() {
            if let Some(earlier) = self.sources.iter().take(at).find(|e| e.name == source.name) {
                return Err(format!(
                    "two sources are called '{}' - one at {} and one at {}; a name is how every other command refers to a source, so they have to differ",
                    source.name, earlier.url, source.url
                ));
            }
        }

        let defaults: Vec<&SourceName> = self
            .sources
            .iter()
            .filter(|source| source.is_default)
            .map(|source| &source.name)
            .collect();
        if defaults.len() > 1 {
            let names: Vec<&str> = defaults.iter().map(|name| name.as_str()).collect();
            return Err(format!(
                "more than one source is marked as the default: {} - only one can be",
                names.join(", ")
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    /// The example from `docs/dev/DESIGN.md`, with the URL written out.
    const DESIGN_EXAMPLE: &str = r#"{
  "sources": [
    { "name": "sepia", "url": "https://example.test/index.json", "default": true,
      "key": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" }
  ]
}"#;

    fn name(text: &str) -> SourceName {
        SourceName::parse(text).unwrap()
    }

    /// A key for the fixtures. Never verified against anything here - these
    /// tests are about `sources.json` as a file, not about signatures - so one
    /// constant is enough and is clearer than generating a keypair per source.
    fn key() -> PublicKey {
        PublicKey::parse(&"11".repeat(32)).unwrap()
    }

    fn source(text: &str, url: &str, is_default: bool) -> Source {
        Source {
            name: name(text),
            url: url.to_owned(),
            is_default,
            key: key(),
        }
    }

    fn store_in(directory: &tempfile::TempDir) -> Store {
        Store::at(directory.path())
    }

    #[test]
    fn the_documented_example_parses() {
        let sources: Sources = serde_json::from_str(DESIGN_EXAMPLE).unwrap();
        assert_eq!(sources.len(), 1);
        let only = sources.by_name(&name("sepia")).unwrap();
        assert_eq!(only.url, "https://example.test/index.json");
        assert!(only.is_default);
    }

    #[test]
    fn it_round_trips_through_a_real_file() {
        let directory = tempfile::tempdir().unwrap();
        let store = store_in(&directory);

        let mut sources = Sources::default();
        sources.insert(source("sepia", "https://example.test/index.json", true));
        sources.insert(source("local", "https://example.invalid/i.json", false));
        sources.save(&store).unwrap();

        let read = Sources::load(&store).unwrap();
        assert_eq!(read, sources);
        assert_eq!(read.default_source().unwrap().name.as_str(), "sepia");
    }

    #[test]
    fn a_file_that_is_not_there_is_a_device_with_no_sources() {
        // Not an error: it is what a freshly installed card looks like.
        let directory = tempfile::tempdir().unwrap();
        let sources = Sources::load(&store_in(&directory)).unwrap();
        assert!(sources.is_empty());
        assert_eq!(sources.len(), 0);
        assert!(sources.default_source().is_none());
    }

    #[test]
    fn two_sources_with_one_name_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let store = store_in(&directory);
        let text = r#"{
          "sources": [
            { "name": "sepia", "url": "https://one.test/i.json", "key": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" },
            { "name": "sepia", "url": "https://two.test/i.json", "key": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" }
          ]
        }"#;
        std::fs::create_dir_all(store.sources_file().parent().unwrap()).unwrap();
        std::fs::write(store.sources_file(), text).unwrap();

        match Sources::load(&store) {
            Err(Error::Parse { message, .. }) => {
                assert!(message.contains("sepia"), "{message}");
                assert!(message.contains("one.test"), "{message}");
                assert!(message.contains("two.test"), "{message}");
            }
            other => panic!("expected a Parse error, got {other:?}"),
        }
    }

    #[test]
    fn two_defaults_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        let store = store_in(&directory);
        let text = r#"{
          "sources": [
            { "name": "one", "url": "https://one.test/i.json", "default": true, "key": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" },
            { "name": "two", "url": "https://two.test/i.json", "default": true, "key": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" }
          ]
        }"#;
        std::fs::create_dir_all(store.sources_file().parent().unwrap()).unwrap();
        std::fs::write(store.sources_file(), text).unwrap();

        match Sources::load(&store) {
            Err(Error::Parse { message, .. }) => {
                assert!(message.contains("default"), "{message}");
                assert!(
                    message.contains("one") && message.contains("two"),
                    "{message}"
                );
            }
            other => panic!("expected a Parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_misspelled_key_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let store = store_in(&directory);
        std::fs::create_dir_all(store.sources_file().parent().unwrap()).unwrap();
        std::fs::write(
            store.sources_file(),
            r#"{ "sources": [ { "name": "sepia", "url": "https://a.test/i.json", "defualt": true } ] }"#,
        )
        .unwrap();

        let message = match Sources::load(&store) {
            Err(Error::Parse { message, .. }) => message,
            other => panic!("expected a Parse error, got {other:?}"),
        };
        assert!(message.contains("defualt"), "{message}");
    }

    #[test]
    fn a_url_that_is_already_configured_updates_its_entry() {
        // add-source re-run on the same URL: updated, not duplicated.
        let mut sources = Sources::default();
        sources.insert(source("sepia", "https://example.test/i.json", false));
        sources.insert(source("renamed", "https://example.test/i.json", true));

        assert_eq!(sources.len(), 1);
        assert_eq!(
            sources
                .by_url("https://example.test/i.json")
                .unwrap()
                .name
                .as_str(),
            "renamed"
        );
    }

    #[test]
    fn a_name_that_is_reused_replaces_rather_than_duplicating() {
        let mut sources = Sources::default();
        sources.insert(source("sepia", "https://one.test/i.json", false));
        sources.insert(source("sepia", "https://two.test/i.json", false));

        assert_eq!(sources.len(), 1);
        assert_eq!(
            sources.by_name(&name("sepia")).unwrap().url,
            "https://two.test/i.json"
        );
    }

    #[test]
    fn only_one_source_is_ever_the_default() {
        let mut sources = Sources::default();
        sources.insert(source("one", "https://one.test/i.json", true));
        sources.insert(source("two", "https://two.test/i.json", true));

        assert_eq!(sources.iter().filter(|s| s.is_default).count(), 1);
        assert_eq!(sources.default_source().unwrap().name.as_str(), "two");

        assert!(sources.set_default(&name("one")));
        assert_eq!(sources.default_source().unwrap().name.as_str(), "one");
        assert_eq!(sources.iter().filter(|s| s.is_default).count(), 1);
    }

    #[test]
    fn making_a_source_that_is_not_there_the_default_changes_nothing() {
        let mut sources = Sources::default();
        sources.insert(source("one", "https://one.test/i.json", true));
        assert!(!sources.set_default(&name("nothing")));
        assert_eq!(sources.default_source().unwrap().name.as_str(), "one");
    }

    #[test]
    fn removing_gives_back_what_was_removed() {
        let mut sources = Sources::default();
        sources.insert(source("one", "https://one.test/i.json", true));
        sources.insert(source("two", "https://two.test/i.json", false));

        let removed = sources.remove(&name("one")).unwrap();
        assert_eq!(removed.url, "https://one.test/i.json");
        assert_eq!(sources.len(), 1);
        assert!(sources.remove(&name("one")).is_none());
    }

    #[test]
    fn what_is_written_is_readable_by_a_person() {
        // It is the one file somebody may open in an editor on the device.
        let directory = tempfile::tempdir().unwrap();
        let store = store_in(&directory);
        let mut sources = Sources::default();
        sources.insert(source("sepia", "https://example.test/i.json", true));
        sources.save(&store).unwrap();

        let text = std::fs::read_to_string(store.sources_file()).unwrap();
        assert!(text.contains('\n'), "written on one line: {text}");
        assert!(text.ends_with('\n'), "no trailing newline");
        assert!(text.contains(r#""default": true"#), "{text}");
    }
}
