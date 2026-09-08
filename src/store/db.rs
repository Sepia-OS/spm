/*
  db.rs

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

//! `/var/lib/spm/installed/` — what is installed, and why.
//!
//! One record per package. A package counts as installed if and only if a
//! record here says so, and a file that no record claims is one `spm` never
//! removes or overwrites.
//!
//! It is also the journal that makes an interrupted install recoverable in
//! exactly one direction. An install writes `<name>.json.partial` with the
//! full file list **before** it writes a single file, and renames it to
//! `<name>.json` only once every file is on the device. So a marker left behind
//! means an install that did not finish, and the only safe reading of it is
//! "undo this" — never "finish it", because nothing here knows how far it got.
//! [`Database::recover`] does that undoing, and every command that writes runs
//! it before anything else.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::installed::Record;
use crate::model::name::PackageName;
use crate::store::{Store, atomic};

/// The records of what is installed.
#[derive(Debug)]
pub struct Database<'store> {
    store: &'store Store,
}

impl<'store> Database<'store> {
    /// Open the database under a store.
    #[must_use]
    pub fn new(store: &'store Store) -> Self {
        Database { store }
    }

    /// The record for a package, or `None` if it is not installed.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the record is there but unreadable, [`Error::Parse`] if
    /// it cannot be understood.
    pub fn get(&self, name: &PackageName) -> Result<Option<Record>> {
        read_record(&self.store.record_file(name))
    }

    /// Whether a package is installed.
    ///
    /// # Errors
    ///
    /// As [`Database::get`].
    pub fn is_installed(&self, name: &PackageName) -> Result<bool> {
        Ok(self.get(name)?.is_some())
    }

    /// Every installed package, ordered by name.
    ///
    /// Ordered so that what `list` prints does not depend on the order a
    /// directory happens to be read in. Unfinished installs are skipped: a
    /// `.partial` is not an installed package.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the directory cannot be read, [`Error::Parse`] if a
    /// record cannot be understood.
    pub fn all(&self) -> Result<Vec<Record>> {
        let directory = self.store.installed_dir();
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            // Nothing installed yet is not a problem to report.
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::Io {
                    path: directory,
                    source,
                });
            }
        };

        let mut records = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|source| Error::Io {
                    path: directory.clone(),
                    source,
                })?
                .path();
            // `.json` and nothing else - in particular not `.json.partial`.
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
                && let Some(record) = read_record(&path)?
            {
                records.push(record);
            }
        }

        records.sort_by(|left, right| left.metadata.name.cmp(&right.metadata.name));
        Ok(records)
    }

    /// Write the journal for an install that is about to start.
    ///
    /// The record names every file the install will write, so that undoing it
    /// needs nothing but this file.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if it cannot be written.
    pub fn begin(&self, record: &Record) -> Result<()> {
        let path = self.store.partial_record_file(&record.metadata.name);
        let text = serde_json::to_vec_pretty(record).map_err(|error| Error::Parse {
            path: path.clone(),
            message: error.to_string(),
        })?;
        atomic::write(&path, &text)
    }

    /// Turn the journal into a record: the install finished.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if there is no journal to commit, or the rename fails.
    pub fn commit(&self, name: &PackageName) -> Result<()> {
        let from = self.store.partial_record_file(name);
        let to = self.store.record_file(name);
        fs::rename(&from, &to).map_err(|source| Error::Io { path: from, source })
    }

    /// Forget a package: it has been removed.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the record is there and cannot be deleted. A record
    /// that is already gone is not an error.
    pub fn forget(&self, name: &PackageName) -> Result<()> {
        let path = self.store.record_file(name);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io { path, source }),
        }
    }

    /// Undo every install that did not finish, and say which they were.
    ///
    /// Run before anything else by every command that writes. Most of the files
    /// a journal lists will not exist — it is written before any of them are —
    /// so a missing file is the ordinary case and not a failure.
    ///
    /// A file that exists and cannot be deleted *is* a failure: the journal is
    /// left in place, so the next command tries again rather than leaving a
    /// half-installed package that nothing remembers.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the directory cannot be read or a file cannot be
    /// deleted, [`Error::Parse`] if a journal cannot be understood.
    pub fn recover(&self) -> Result<Vec<PackageName>> {
        let directory = self.store.installed_dir();
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::Io {
                    path: directory,
                    source,
                });
            }
        };

        let mut undone = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|source| Error::Io {
                    path: directory.clone(),
                    source,
                })?
                .path();
            if path
                .extension()
                .is_some_and(|extension| extension == "partial")
            {
                if let Some(record) = read_record(&path)? {
                    self.undo(&record)?;
                    undone.push(record.metadata.name.clone());
                }
                // Only once every file it named is gone.
                fs::remove_file(&path).map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })?;
            }
        }

        undone.sort();
        Ok(undone)
    }

    /// Delete the files a journal claims, and nothing else.
    fn undo(&self, record: &Record) -> Result<()> {
        for file in &record.files {
            let Some(path) = under(self.store.root(), file) else {
                // A record naming an absolute path or one with `..` in it did
                // not come from this crate. Refusing beats deleting whatever
                // it points at.
                return Err(Error::Parse {
                    path: self.store.partial_record_file(&record.metadata.name),
                    message: format!(
                        "it claims the file {}, which is not inside the device's root",
                        file.display()
                    ),
                });
            };
            match fs::remove_file(&path) {
                // The ordinary case: the journal is written before the files.
                Ok(()) => {}
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(Error::Io { path, source }),
            }
        }
        Ok(())
    }
}

/// Read one record, or `None` if the file is not there.
fn read_record(path: &Path) -> Result<Option<Record>> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| Error::Parse {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

/// Resolve a recorded file against the root, or `None` if it would escape.
///
/// A record's paths are relative to `/`. An absolute one would make
/// [`Path::join`] throw the root away — the same trap the store documents —
/// and a `..` would walk out of it. Neither can be produced by this crate, and
/// both are refused rather than trusted. The extraction in Step 30 needs the
/// same rule and will share it.
fn under(root: &Path, file: &Path) -> Option<PathBuf> {
    for part in file.components() {
        match part {
            Component::Normal(_) => {}
            _ => return None,
        }
    }
    Some(root.join(file))
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
    use crate::model::installed::Reason;
    use crate::model::metadata::Metadata;
    use crate::model::name::Target;
    use crate::model::version::Version;

    fn package(name: &str) -> PackageName {
        PackageName::parse(name).unwrap()
    }

    fn record(name: &str, files: &[&str]) -> Record {
        Record {
            metadata: Metadata {
                name: package(name),
                version: Version::parse("1.0.0").unwrap(),
                target: Target::parse("aarch64-musl").unwrap(),
                description: String::new(),
                dependencies: Vec::new(),
                sha256: None,
            },
            source: crate::model::name::SourceName::parse("sepia").unwrap(),
            reason: Reason::Explicit,
            installed_at: 1,
            files: files.iter().map(PathBuf::from).collect(),
        }
    }

    /// A store on a temporary root, with the files a record claims actually
    /// written, as an install would have left them.
    fn with_files(root: &Path, files: &[&str]) {
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"installed").unwrap();
        }
    }

    #[test]
    fn a_record_survives_being_written_and_read() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);
        let written = record("helix", &["usr/bin/hx"]);

        db.begin(&written).unwrap();
        db.commit(&package("helix")).unwrap();

        assert_eq!(db.get(&package("helix")).unwrap().unwrap(), written);
        assert!(db.is_installed(&package("helix")).unwrap());
    }

    #[test]
    fn a_package_that_is_not_installed_is_not_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);

        assert!(db.get(&package("helix")).unwrap().is_none());
        assert!(!db.is_installed(&package("helix")).unwrap());
        assert!(db.all().unwrap().is_empty());
    }

    #[test]
    fn a_journal_is_not_an_installed_package() {
        // Written, not committed: the package is not installed yet.
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);

        db.begin(&record("helix", &["usr/bin/hx"])).unwrap();

        assert!(!db.is_installed(&package("helix")).unwrap());
        assert!(db.all().unwrap().is_empty(), "a .partial is not a record");
    }

    #[test]
    fn everything_installed_comes_back_in_order() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);

        for name in ["helix", "grit", "musl"] {
            db.begin(&record(name, &[])).unwrap();
            db.commit(&package(name)).unwrap();
        }
        // And one that never finished.
        db.begin(&record("zlib", &[])).unwrap();

        let installed = db.all().unwrap();
        let names: Vec<&str> = installed
            .iter()
            .map(|record| record.metadata.name.as_str())
            .collect();
        assert_eq!(names, vec!["grit", "helix", "musl"]);
    }

    #[test]
    fn forgetting_a_package_removes_its_record() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);
        db.begin(&record("helix", &[])).unwrap();
        db.commit(&package("helix")).unwrap();

        db.forget(&package("helix")).unwrap();

        assert!(!db.is_installed(&package("helix")).unwrap());
        // Twice is not an error.
        db.forget(&package("helix")).unwrap();
    }

    #[test]
    fn an_unfinished_install_is_undone() {
        // The done-when: a journal, the files it claims, and nothing left of
        // either afterwards.
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);
        let files = ["usr/bin/hx", "usr/lib/helix/runtime/grammars/rust.so"];

        db.begin(&record("helix", &files)).unwrap();
        with_files(directory.path(), &files);
        for file in files {
            assert!(directory.path().join(file).exists());
        }

        let undone = db.recover().unwrap();

        assert_eq!(undone, vec![package("helix")]);
        for file in files {
            assert!(
                !directory.path().join(file).exists(),
                "{file} survived the rollback"
            );
        }
        assert!(!store.partial_record_file(&package("helix")).exists());
        assert!(!db.is_installed(&package("helix")).unwrap());
    }

    #[test]
    fn a_rollback_tolerates_files_that_were_never_written() {
        // The ordinary case: the journal is written first, so an install that
        // died early claims files that never existed.
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);

        db.begin(&record("helix", &["usr/bin/hx", "usr/share/doc/hx.1"]))
            .unwrap();

        assert_eq!(db.recover().unwrap(), vec![package("helix")]);
        assert!(!store.partial_record_file(&package("helix")).exists());
    }

    #[test]
    fn a_rollback_leaves_finished_installs_alone() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);
        with_files(directory.path(), &["usr/bin/grit"]);
        db.begin(&record("grit", &["usr/bin/grit"])).unwrap();
        db.commit(&package("grit")).unwrap();

        assert!(db.recover().unwrap().is_empty());

        assert!(directory.path().join("usr/bin/grit").exists());
        assert!(db.is_installed(&package("grit")).unwrap());
    }

    #[test]
    fn nothing_to_recover_is_not_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        assert!(Database::new(&store).recover().unwrap().is_empty());
    }

    #[test]
    fn a_journal_claiming_a_file_outside_the_root_is_refused() {
        // Nothing in this crate can write such a record. If one exists, the
        // answer is to refuse, not to delete whatever it points at.
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path());
        let db = Database::new(&store);

        for claim in ["/etc/passwd", "../../etc/passwd"] {
            db.begin(&record("evil", &[claim])).unwrap();
            match db.recover() {
                Err(Error::Parse { message, .. }) => {
                    assert!(message.contains("root"), "{message}");
                }
                other => panic!("expected a refusal for {claim}, got {other:?}"),
            }
            // Left in place, so it is not silently forgotten.
            assert!(store.partial_record_file(&package("evil")).exists());
            fs::remove_file(store.partial_record_file(&package("evil"))).unwrap();
        }
    }
}
