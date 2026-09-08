/*
  main.rs

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

//! The `spm` executable.
//!
//! Three jobs and no more: parse the arguments, hand off to a command, and turn
//! whatever comes back into an exit code. Everything a test would want to call
//! lives in the library beside this file.

use std::process::ExitCode;

use clap::Parser;

use spm::cli::{Cli, Command};
use spm::error::Result;
use spm::{ops, ui};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            ui::report(&error);
            ExitCode::from(error.exit_code())
        }
    }
}

/// Everything the process does, so that the only thing above it is the mapping
/// from a failure to an exit code.
fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Create(args) => {
            let created = ops::create::create(&args.root, &args.metadata, &args.output)?;
            ui::created(&created);
            Ok(())
        }
    }
}
