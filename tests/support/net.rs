/*
  net.rs

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

//! A transport that serves a directory, so no test needs a network.
//!
//! A URL is mapped to a file by its path: `https://example.test/index.json`
//! is `<directory>/index.json`. Which host it names does not matter — nothing
//! resolves it — so a test can use whatever reads clearly.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use spm::error::{Error, Result};
use spm::net::transport::Transport;

/// The host the fixtures pretend to be published on.
pub const HOST: &str = "https://example.test";

/// A transport that reads from a directory.
#[derive(Debug)]
pub struct Fake {
    directory: PathBuf,
    /// URLs that fail however often they are asked for, so that a test can
    /// have one source of three be unreachable.
    broken: RefCell<HashSet<String>>,
    /// What has been asked for, in order.
    asked: RefCell<Vec<String>>,
}

impl Fake {
    /// Serve this directory.
    pub fn serving(directory: &Path) -> Self {
        Fake {
            directory: directory.to_path_buf(),
            broken: RefCell::new(HashSet::new()),
            asked: RefCell::new(Vec::new()),
        }
    }

    /// Make one URL fail.
    pub fn break_url(&self, url: &str) {
        self.broken.borrow_mut().insert(url.to_owned());
    }

    /// Every URL fetched so far, in order.
    pub fn asked(&self) -> Vec<String> {
        self.asked.borrow().clone()
    }

    /// The URL a file in the served directory has.
    pub fn url_for(&self, name: &str) -> String {
        format!("{HOST}/{name}")
    }

    /// Put a file where a URL will find it, and give back that URL.
    pub fn serve(&self, name: &str, contents: &[u8]) -> String {
        let path = self.directory.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, contents).unwrap();
        self.url_for(name)
    }

    /// Serve a file that is already somewhere else, by copying it in.
    pub fn serve_file(&self, name: &str, from: &Path) -> String {
        self.serve(name, &fs::read(from).unwrap())
    }
}

impl Transport for Fake {
    fn get(&self, url: &str) -> Result<Box<dyn Read>> {
        self.asked.borrow_mut().push(url.to_owned());

        if self.broken.borrow().contains(url) {
            return Err(Error::Network {
                url: url.to_owned(),
                message: "the fake transport was told to fail this one".to_owned(),
            });
        }

        // Everything after the host is the path under the directory.
        let relative = url
            .strip_prefix(HOST)
            .unwrap_or(url)
            .trim_start_matches('/');
        assert!(
            !relative.contains(".."),
            "a fixture URL should not walk out of the directory: {url}"
        );

        // Opened, not read: a real transport hands back a stream, and a fake
        // that read the file into memory first would hide a caller that does
        // the same. A package is 216 MiB.
        let path = self.directory.join(relative);
        match fs::File::open(&path) {
            Ok(file) => Ok(Box::new(file)),
            Err(source) => Err(Error::Network {
                url: url.to_owned(),
                message: format!("nothing is served at {}: {source}", path.display()),
            }),
        }
    }
}
