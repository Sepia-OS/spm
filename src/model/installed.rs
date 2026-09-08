/*
  installed.rs

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

//! The record of an installed package.
//!
//! Its metadata, the files it put on the device, the source it came from, and
//! whether it was asked for or pulled in as a dependency. A package counts as
//! installed if and only if one of these records says so, and a file that no
//! record claims is one `spm` never removes or overwrites.
//!
//! `files` are relative to `/` and in the order they were written, so undoing
//! an install is walking the list backwards. Directories are not listed: they
//! go when they empty out.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::metadata::Metadata;
use crate::model::name::{PackageName, SourceName};
use crate::model::version::Version;

/// Why a package is on the device.
///
/// The whole basis of `remove`'s autoremove: a package pulled in as a
/// dependency goes when nothing needs it any more, and one the user asked for
/// by name never does, however unreferenced it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    /// The user asked for this package by name.
    Explicit,
    /// It came in because something else needed it.
    Dependency,
}

/// What `/var/lib/spm/installed/<name>.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// The package's own `metadata.json`, as it arrived.
    pub metadata: Metadata,
    /// The source it came from, which stays recorded even if that source is
    /// later removed.
    pub source: SourceName,
    /// Whether it was asked for or pulled in.
    pub reason: Reason,
    /// When it was installed, in seconds since the epoch.
    pub installed_at: u64,
    /// Every file it put on the device, relative to `/`, in the order they
    /// were written.
    ///
    /// A `PathBuf` because that is what a path is, but note the consequence:
    /// this record is JSON, and a path that is not valid UTF-8 cannot be
    /// written into it. A package carrying such a path therefore cannot be
    /// recorded, so `install` has to refuse one — see the extraction rules.
    pub files: Vec<PathBuf>,
}

impl Record {
    /// Whether this package needs the named one.
    ///
    /// What `remove` asks of every other record before taking a package away,
    /// and what autoremove asks to decide whether a dependency is still
    /// wanted.
    #[must_use]
    pub fn depends_on(&self, name: &PackageName) -> bool {
        self.requirement(name).is_some()
    }

    /// The version of the named package this one needs, if it needs it.
    ///
    /// A floor, not an exact version: *that version or a newer one*.
    #[must_use]
    pub fn requirement(&self, name: &PackageName) -> Option<&Version> {
        self.metadata
            .dependencies
            .iter()
            .find(|dependency| &dependency.name == name)
            .map(|dependency| &dependency.version)
    }

    /// Whether the user asked for this package by name.
    #[must_use]
    pub fn is_explicit(&self) -> bool {
        self.reason == Reason::Explicit
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    /// The example from `docs/dev/DESIGN.md`, with the metadata written out —
    /// the document elides it as "the package's own metadata.json, verbatim",
    /// and that is the example from the architecture document.
    const DESIGN_EXAMPLE: &str = r#"{
  "metadata": {
    "name": "helix",
    "version": "25.07.1",
    "target": "aarch64-musl",
    "description": "The Helix editor, with its tree-sitter grammars.",
    "dependencies": [
      { "name": "llvm-runtime", "version": "23.1.0" }
    ],
    "sha256": ""
  },
  "source": "sepia",
  "reason": "explicit",
  "installed_at": 1757260800,
  "files": [ "usr/bin/hx", "usr/lib/helix/runtime/grammars/rust.so" ]
}"#;

    fn parsed() -> Record {
        serde_json::from_str(DESIGN_EXAMPLE).unwrap()
    }

    #[test]
    fn the_documented_example_parses() {
        let record = parsed();
        assert_eq!(record.metadata.name.as_str(), "helix");
        assert_eq!(record.source.as_str(), "sepia");
        assert_eq!(record.reason, Reason::Explicit);
        assert_eq!(record.installed_at, 1_757_260_800);
        assert_eq!(record.files.len(), 2);
        assert_eq!(record.files[0], PathBuf::from("usr/bin/hx"));
    }

    #[test]
    fn the_documented_example_round_trips() {
        let record = parsed();
        let written = serde_json::to_string(&record).unwrap();
        let again: Record = serde_json::from_str(&written).unwrap();
        assert_eq!(record, again);
    }

    #[test]
    fn a_reason_is_spelled_the_way_the_file_spells_it() {
        let record = parsed();
        let written = serde_json::to_string(&record).unwrap();
        assert!(written.contains(r#""reason":"explicit""#), "{written}");

        let pulled_in = DESIGN_EXAMPLE.replace(r#""explicit""#, r#""dependency""#);
        let record: Record = serde_json::from_str(&pulled_in).unwrap();
        assert_eq!(record.reason, Reason::Dependency);
        assert!(!record.is_explicit());
    }

    #[test]
    fn an_unknown_reason_is_an_error_rather_than_a_guess() {
        let odd = DESIGN_EXAMPLE.replace(r#""explicit""#, r#""maybe""#);
        assert!(serde_json::from_str::<Record>(&odd).is_err());
    }

    #[test]
    fn a_misspelled_field_is_reported() {
        let misspelled = DESIGN_EXAMPLE.replace(r#""installed_at""#, r#""installed""#);
        let message = serde_json::from_str::<Record>(&misspelled)
            .unwrap_err()
            .to_string();
        assert!(message.contains("installed"), "{message}");
    }

    #[test]
    fn a_record_knows_what_it_needs() {
        let record = parsed();
        let needed = PackageName::parse("llvm-runtime").unwrap();
        let unrelated = PackageName::parse("grit").unwrap();

        assert!(record.depends_on(&needed));
        assert!(!record.depends_on(&unrelated));
        assert_eq!(record.requirement(&needed).unwrap().as_str(), "23.1.0");
        assert!(record.requirement(&unrelated).is_none());
    }

    #[test]
    fn the_file_list_keeps_the_order_it_was_written_in() {
        // remove walks it backwards, so the order is part of the format.
        let record = parsed();
        assert_eq!(
            record.files,
            vec![
                PathBuf::from("usr/bin/hx"),
                PathBuf::from("usr/lib/helix/runtime/grammars/rust.so"),
            ]
        );
    }
}
