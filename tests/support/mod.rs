/*
  mod.rs

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

//! Building packages for tests to install.
//!
//! The packages are **built by `spm create`**, not checked in. There is one
//! implementation of the package format and these use it, so a change to the
//! format cannot leave the fixtures describing the old one — and a fixture
//! that `create` could not have produced is not a fixture worth testing
//! against.
//!
//! Three of them are named, because they are the three shapes every later step
//! needs: one that stands alone, one that needs another, and two that disagree
//! over a file.

#![allow(dead_code, reason = "each test crate uses the part of this it needs")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use spm::model::metadata::Metadata;
use spm::ops::create::{METADATA, PAYLOAD, create};

/// A package to build.
pub struct Package {
    name: String,
    version: String,
    target: String,
    description: String,
    dependencies: Vec<(String, String)>,
    /// Path under the staged root, contents, and whether it is executable.
    files: Vec<(String, Vec<u8>, bool)>,
    licence: bool,
}

/// A package that has been built, and where its three files are.
#[derive(Debug, Clone)]
pub struct Built {
    pub name: String,
    pub version: String,
    /// The package itself.
    pub package: PathBuf,
    /// Its metadata, as published beside it.
    pub metadata: PathBuf,
    /// Its digest, as published beside it.
    pub sums: PathBuf,
    /// The digest of the package.
    pub sha256: String,
    /// How big the package is.
    pub bytes: u64,
}

impl Package {
    /// A package with a licence and one executable of its own name, which is
    /// the smallest thing `create` will agree to build.
    pub fn named(name: &str) -> Self {
        Package {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            target: "aarch64-musl".to_owned(),
            description: format!("The {name} package."),
            dependencies: Vec::new(),
            files: vec![(
                format!("usr/bin/{name}"),
                format!("#!/bin/sh\necho {name}\n").into_bytes(),
                true,
            )],
            licence: true,
        }
    }

    #[must_use]
    pub fn version(mut self, version: &str) -> Self {
        self.version = version.to_owned();
        self
    }

    #[must_use]
    pub fn target(mut self, target: &str) -> Self {
        self.target = target.to_owned();
        self
    }

    /// Need another package, at that version or newer.
    #[must_use]
    pub fn depends_on(mut self, name: &str, version: &str) -> Self {
        self.dependencies
            .push((name.to_owned(), version.to_owned()));
        self
    }

    /// Ship a file at this path under the staged root.
    #[must_use]
    pub fn file(mut self, path: &str, contents: &[u8]) -> Self {
        self.files.push((path.to_owned(), contents.to_vec(), false));
        self
    }

    /// Ship an executable.
    #[must_use]
    pub fn executable(mut self, path: &str, contents: &[u8]) -> Self {
        self.files.push((path.to_owned(), contents.to_vec(), true));
        self
    }

    /// Leave the licence out, for the tests about refusing that.
    #[must_use]
    pub fn without_licence(mut self) -> Self {
        self.licence = false;
        self
    }

    /// Stage the tree and run `create` on it.
    ///
    /// # Panics
    ///
    /// If the tree cannot be staged or `create` refuses it — in a test, either
    /// is a failure and there is nothing to recover to.
    pub fn build(self, into: &Path) -> Built {
        let work = into.join(format!("build-{}-{}", self.name, self.version));
        let stage = work.join("stage");
        let output = work.join("dist");

        for (path, contents, executable) in &self.files {
            let full = stage.join(path);
            fs::create_dir_all(full.parent().expect("a file has a parent")).unwrap();
            fs::write(&full, contents).unwrap();
            #[cfg(unix)]
            if *executable {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&full, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        if self.licence {
            let licence = stage.join(format!("usr/share/licenses/{}/LICENSE", self.name));
            fs::create_dir_all(licence.parent().expect("a file has a parent")).unwrap();
            fs::write(&licence, b"Apache-2.0\n").unwrap();
        }

        let dependencies: Vec<String> = self
            .dependencies
            .iter()
            .map(|(name, version)| format!(r#"{{ "name": "{name}", "version": "{version}" }}"#))
            .collect();
        let metadata_file = work.join("metadata.json");
        fs::create_dir_all(&work).unwrap();
        fs::write(
            &metadata_file,
            format!(
                r#"{{
  "name": "{name}",
  "version": "{version}",
  "target": "{target}",
  "description": "{description}",
  "dependencies": [{dependencies}],
  "sha256": ""
}}"#,
                name = self.name,
                version = self.version,
                target = self.target,
                description = self.description,
                dependencies = dependencies.join(", "),
            ),
        )
        .unwrap();

        let created = create(&stage, &metadata_file, &output)
            .unwrap_or_else(|error| panic!("create refused the {} fixture: {error}", self.name));

        Built {
            name: self.name,
            version: self.version,
            package: created.package,
            metadata: created.metadata,
            sums: created.sums,
            sha256: created.sha256.as_str().to_owned(),
            bytes: created.bytes,
        }
    }
}

impl Built {
    /// One member of the package.
    ///
    /// # Panics
    ///
    /// If the package cannot be read or has no such member.
    pub fn member(&self, want: &str) -> Vec<u8> {
        let file = fs::File::open(&self.package).unwrap();
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            if entry.path().unwrap().display().to_string() == want {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                return bytes;
            }
        }
        panic!("{want} is not in {}", self.package.display());
    }

    /// The metadata packed inside the package.
    ///
    /// # Panics
    ///
    /// If it is not there or does not parse.
    pub fn packed_metadata(&self) -> Metadata {
        serde_json::from_slice(&self.member(METADATA)).unwrap()
    }

    /// Check the package against everything published beside it.
    ///
    /// # Panics
    ///
    /// If any of it disagrees — which is the test.
    pub fn verify(&self) {
        let bytes = fs::read(&self.package).unwrap();
        assert_eq!(digest(&bytes), self.sha256, "the package is not its digest");

        let sums = fs::read_to_string(&self.sums).unwrap();
        let name = self.package.file_name().unwrap().display().to_string();
        assert_eq!(sums, format!("{}  {name}\n", self.sha256));

        let metadata = self.packed_metadata();
        assert_eq!(
            metadata
                .sha256
                .expect("a packed metadata has its digest")
                .as_str(),
            digest(&self.member(PAYLOAD)),
            "the packed metadata does not describe the payload beside it"
        );

        assert_eq!(fs::read(&self.metadata).unwrap(), self.member(METADATA));
    }
}

/// A digest, as `create` writes them.
pub fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A package that stands alone.
pub fn plain(into: &Path) -> Built {
    Package::named("plain").build(into)
}

/// A package that needs `plain`.
pub fn dependent(into: &Path) -> Built {
    Package::named("dependent")
        .depends_on("plain", "1.0.0")
        .build(into)
}

/// A package that ships the file `plain` ships, so the two cannot both be
/// installed.
pub fn rival(into: &Path) -> Built {
    Package::named("rival")
        .executable("usr/bin/plain", b"#!/bin/sh\necho not plain\n")
        .build(into)
}
