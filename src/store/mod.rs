/*
  mod.rs

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

//! Everything the device keeps, and the paths it keeps it under.
//!
//! Owns the root prefix that every path is built from. Mechanism only: nothing
//! here decides anything, prints anything, or touches the filesystem — these
//! are the names of files, not the files.
//!
//! The prefix exists so that the tests can run against a temporary directory
//! instead of the real `/`. It is not a user-facing option: a package manager
//! with a `--root` flag is one that can be pointed at the wrong system, and
//! nothing in `docs/dev/ARCHITECTURE.md` asks for that.

use std::path::{Path, PathBuf};

use crate::model::name::{PackageName, SourceName};

pub mod atomic;
pub mod cache;
pub mod config;
pub mod db;
pub mod index;
pub mod lock;

/// Where everything lives.
///
/// Every path in `spm` comes from here, so that "what does the device keep,
/// and where" has exactly one answer and the tests can move all of it at once.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The real one, rooted at `/`.
    #[must_use]
    pub fn new() -> Self {
        Store::at("/")
    }

    /// One rooted somewhere else — a temporary directory, in a test.
    ///
    /// The fragments below are all relative, which matters more than it looks:
    /// `Path::join` with an absolute argument throws the base away, so a
    /// single leading `/` in one of them would silently write to the real
    /// system while a test believed it was sandboxed.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    /// The prefix everything is under.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `/etc/spm/sources.json` — the configured sources.
    #[must_use]
    pub fn sources_file(&self) -> PathBuf {
        self.root.join("etc/spm/sources.json")
    }

    /// `/var/lib/spm` — what `spm` knows.
    #[must_use]
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("var/lib/spm")
    }

    /// `/var/lib/spm/index` — the local copy of each source's index.
    #[must_use]
    pub fn index_dir(&self) -> PathBuf {
        self.state_dir().join("index")
    }

    /// The local copy of one source's index.
    ///
    /// Safe to build from a name because a [`SourceName`] cannot contain `/`
    /// or be `..` — that is what the validation in [`crate::model::name`] is
    /// for, and it is why this can be a `join` and not a sanitising step.
    #[must_use]
    pub fn index_file(&self, source: &SourceName) -> PathBuf {
        self.index_dir().join(format!("{source}.json"))
    }

    /// `/var/lib/spm/installed` — one record per installed package.
    #[must_use]
    pub fn installed_dir(&self) -> PathBuf {
        self.state_dir().join("installed")
    }

    /// The record of one installed package.
    #[must_use]
    pub fn record_file(&self, package: &PackageName) -> PathBuf {
        self.installed_dir().join(format!("{package}.json"))
    }

    /// The journal an install writes before it writes any files.
    ///
    /// A record with this name is an install that did not finish, and finding
    /// one is what tells the next command to undo it.
    #[must_use]
    pub fn partial_record_file(&self, package: &PackageName) -> PathBuf {
        self.installed_dir().join(format!("{package}.json.partial"))
    }

    /// `/var/lib/spm/lock` — held by every command that writes.
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.state_dir().join("lock")
    }

    /// `/var/cache/spm` — downloaded packages, none of them needed twice.
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("var/cache/spm")
    }
}

impl Default for Store {
    fn default() -> Self {
        Store::new()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    fn package() -> PackageName {
        PackageName::parse("helix").unwrap()
    }

    fn source() -> SourceName {
        SourceName::parse("sepia").unwrap()
    }

    #[test]
    fn the_real_store_is_where_the_documents_say() {
        let store = Store::new();
        assert_eq!(store.sources_file(), Path::new("/etc/spm/sources.json"));
        assert_eq!(store.index_dir(), Path::new("/var/lib/spm/index"));
        assert_eq!(store.installed_dir(), Path::new("/var/lib/spm/installed"));
        assert_eq!(store.lock_file(), Path::new("/var/lib/spm/lock"));
        assert_eq!(store.cache_dir(), Path::new("/var/cache/spm"));
        assert_eq!(
            store.index_file(&source()),
            Path::new("/var/lib/spm/index/sepia.json")
        );
        assert_eq!(
            store.record_file(&package()),
            Path::new("/var/lib/spm/installed/helix.json")
        );
        assert_eq!(
            store.partial_record_file(&package()),
            Path::new("/var/lib/spm/installed/helix.json.partial")
        );
    }

    #[test]
    fn every_path_moves_with_the_prefix() {
        // The whole point of the prefix: a test can put all of it somewhere
        // harmless, and nothing is left behind pointing at the real system.
        let store = Store::at("/tmp/spm-test");
        for path in [
            store.sources_file(),
            store.state_dir(),
            store.index_dir(),
            store.installed_dir(),
            store.lock_file(),
            store.cache_dir(),
            store.index_file(&source()),
            store.record_file(&package()),
            store.partial_record_file(&package()),
        ] {
            assert!(
                path.starts_with("/tmp/spm-test"),
                "{} escaped the prefix",
                path.display()
            );
        }
    }

    #[test]
    fn a_relative_prefix_stays_relative() {
        // Nothing may assume the root is absolute; a test may use a relative
        // temporary directory.
        let store = Store::at("scratch");
        assert_eq!(
            store.sources_file(),
            Path::new("scratch/etc/spm/sources.json")
        );
    }

    #[test]
    fn a_record_cannot_be_written_outside_its_directory() {
        // Names are validated where they are made, so this cannot be
        // constructed at all - which is the point. The test is here so that
        // loosening the name rules fails something in the store as well.
        assert!(PackageName::parse("../../etc/passwd").is_err());
        let store = Store::at("/tmp/spm-test");
        let record = store.record_file(&package());
        assert!(record.starts_with(store.installed_dir()));
    }

    #[test]
    fn the_default_store_is_the_real_one() {
        assert_eq!(Store::default().root(), Path::new("/"));
    }
}
