/*
  lib.rs

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

//! `spm`, the SepiaOS package manager.
//!
//! The crate is a library with a thin binary on top of it. That is not a
//! preference: an integration test in `tests/` cannot reach inside a binary
//! crate, and `docs/dev/DESIGN.md` asks for tests that drive whole commands.
//! Everything therefore lives here, and `main.rs` parses arguments and turns a
//! result into an exit code.

pub mod cli;
pub mod conffile;
pub mod error;
pub mod model;
pub mod net;
pub mod ops;
pub mod sign;
pub mod store;
pub mod ui;
pub mod unpack;
