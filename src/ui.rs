/*
  ui.rs

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

//! The only module that writes to the terminal.
//!
//! Output is a public interface: keeping it in one place is what stops it
//! drifting between commands, and what makes it possible to test. Errors go to
//! standard error, everything else to standard output.

use crate::error::Error;

/// Print an error the way every command prints one.
///
/// Standard error, prefixed, and nothing else — the exit code carries the
/// machine-readable half of the same message.
pub fn report(error: &Error) {
    eprintln!("spm: {error}");
}

/// Say what `create` wrote.
///
/// One path per line, in the order `docs/USER-GUIDE.md` shows them: the
/// package, its metadata, and the digest. A release pipeline reads this.
pub fn created(created: &crate::ops::create::Created) {
    println!("{}", created.package.display());
    println!("{}", created.metadata.display());
    println!("{}", created.sums.display());
}
