/*
  cache.rs

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

//! `/var/cache/spm/` - downloaded packages.
//!
//! Nothing in it is needed twice, so anything here may be deleted at any
//! time without consequence.
//!
//! The one thing this module has to be careful about is the **version**.
//! A [`PackageName`] and a [`Target`] are validated where they are made and
//! cannot escape a directory; a [`Version`] is deliberately whatever upstream
//! chose, and this is one of the two places in the program that turns one into
//! a filename — `ops::create` is the other, and refuses for the same reason.

use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};
use crate::model::name::{PackageName, SourceName, Target};
use crate::model::version::Version;
use crate::store::Store;

/// Where a downloaded package is put while it is being installed.
///
/// `source` is not part of the name — it is there so that a version which
/// cannot be a filename is reported against the index it arrived in, which is
/// the file somebody would have to look at.
///
/// # Errors
///
/// [`Error::Parse`] naming the source's index if the version would not be one
/// file in one directory.
pub fn package_file(
    store: &Store,
    source: &SourceName,
    package: &PackageName,
    version: &Version,
    target: &Target,
) -> Result<PathBuf> {
    let name = format!("{package}-{version}-{target}.tar.gz");

    let mut parts = Path::new(&name).components();
    let single = matches!(parts.next(), Some(Component::Normal(part)) if part == name.as_str());
    if !single || parts.next().is_some() {
        return Err(Error::Parse {
            path: store.index_file(source),
            message: format!(
                "the version '{version}' of '{package}' cannot be part of a filename - a package is downloaded as <name>-<version>-<target>.tar.gz, and this would not be one file in one directory"
            ),
        });
    }

    Ok(store.cache_dir().join(name))
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

    fn parts() -> (SourceName, PackageName, Target) {
        (
            SourceName::parse("sepia").unwrap(),
            PackageName::parse("helix").unwrap(),
            Target::parse("aarch64-musl").unwrap(),
        )
    }

    #[test]
    fn a_package_is_cached_under_the_name_it_was_published_as() {
        let store = Store::at("/tmp/spm-test");
        let (source, package, target) = parts();
        let version = Version::parse("25.07.1").unwrap();

        let path = package_file(&store, &source, &package, &version, &target).unwrap();

        assert_eq!(
            path,
            Path::new("/tmp/spm-test/var/cache/spm/helix-25.07.1-aarch64-musl.tar.gz")
        );
    }

    #[test]
    fn a_version_that_cannot_be_a_filename_is_refused() {
        // The hazard this module exists to close: a name and a target are
        // validated where they are made, a version is whatever upstream chose.
        let store = Store::at("/tmp/spm-test");
        let (source, package, target) = parts();
        let version = Version::parse("../../evil").unwrap();

        match package_file(&store, &source, &package, &version, &target) {
            Err(Error::Parse { path, message }) => {
                assert_eq!(path, store.index_file(&source));
                assert!(message.contains("../../evil"), "{message}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn nothing_cached_escapes_the_cache_directory() {
        let store = Store::at("/tmp/spm-test");
        let (source, package, target) = parts();
        for text in ["25.07.1", "1.0", "0"] {
            let version = Version::parse(text).unwrap();
            let path = package_file(&store, &source, &package, &version, &target).unwrap();
            assert!(path.starts_with(store.cache_dir()), "{}", path.display());
        }
    }
}
