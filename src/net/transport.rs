/*
  transport.rs

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

//! The seam between `spm` and the network.
//!
//! One method: fetch a URL, and give back something to read. Everything above
//! takes a `&dyn Transport`, so a test drives the real code path — the real
//! `update`, the real `install`, the real verification — with fixtures behind
//! it and no network anywhere.
//!
//! That is why the trait returns a boxed reader rather than `impl Read`: a
//! trait with a return-position `impl Trait` is not object-safe, and `&dyn
//! Transport` is the whole point.
//!
//! It streams. A package is 216 MiB and the smallest supported board has
//! 512 MiB of RAM, so nothing above this may be handed a `Vec<u8>` of a
//! download.

use std::io::Read;

use crate::error::Result;

/// Somewhere bytes come from.
pub trait Transport {
    /// Fetch a URL.
    ///
    /// The reader is the response body, streamed. What it is read into is the
    /// caller's business: an index is small enough to hold, a package is not.
    ///
    /// # Errors
    ///
    /// [`crate::error::Error::Network`] if it cannot be fetched,
    /// [`crate::error::Error::ClockBehind`] if a certificate was rejected and
    /// the device's clock is the likely reason.
    fn get(&self, url: &str) -> Result<Box<dyn Read>>;
}
