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
/// Take the single-writer lock, saying what is being waited for.
///
/// `store` prints nothing, so the sequence lives here: try, and if somebody
/// has it, say who and then wait. Read-only commands never call this.
fn locked(store: &spm::store::Store) -> Result<spm::store::lock::Lock> {
    let path = store.lock_file();
    if let Some(lock) = spm::store::lock::Lock::try_acquire(&path)? {
        return Ok(lock);
    }
    ui::waiting_for_lock(spm::store::lock::Lock::holder(&path));
    spm::store::lock::Lock::wait(&path)
}

/// Read a package reference from the command line.
fn package_ref(text: &str) -> Result<spm::model::name::PackageRef> {
    spm::model::name::PackageRef::parse(text)
        .map_err(|reason| spm::error::Error::Usage(format!("'{text}': {reason}")))
}

/// Read a version from the command line.
fn version(text: &str) -> Result<spm::model::version::Version> {
    spm::model::version::Version::parse(text)
        .ok_or_else(|| spm::error::Error::Usage(format!("'{text}' is not a version")))
}

fn run() -> Result<()> {
    // The store is where everything the device keeps lives. Read-only commands
    // take no lock; the ones that write will.
    let store = spm::store::Store::new();

    match Cli::parse().command {
        Command::Update(args) => {
            let which = match args.source {
                Some(name) => spm::ops::update::Which::One(
                    spm::model::name::SourceName::parse(&name).map_err(|reason| {
                        spm::error::Error::Usage(format!("'{name}': {reason}"))
                    })?,
                ),
                // --all, or neither: the same thing.
                None => spm::ops::update::Which::All,
            };
            let _lock = locked(&store)?;
            let transport = spm::net::https::Https::new();
            let report = ops::update::update(&store, &transport, &which)?;
            ui::updated(&report);
            // Printed first, then the failure: a script should see both.
            report.outcome()
        }
        Command::Search(args) => {
            let target = spm::model::name::Target::current();
            let found = ops::query::search(&store, &target, &args.needle)?;
            if found.is_empty() {
                // Its own answer: the advice attached to "no such package" is
                // to run a search, which is what just happened.
                return Err(spm::error::Error::NothingMatched {
                    needle: args.needle,
                });
            }
            ui::packages(&found);
            Ok(())
        }
        Command::Info(args) => {
            let target = spm::model::name::Target::current();
            let wanted = args.version.as_deref().map(version).transpose()?;
            let details = ops::query::info(
                &store,
                &package_ref(&args.package)?,
                &target,
                wanted.as_ref(),
            )?;
            ui::package_detail(&details);
            Ok(())
        }
        Command::List(args) => {
            let target = spm::model::name::Target::current();
            let from = args
                .source
                .as_deref()
                .map(|name| {
                    spm::model::name::SourceName::parse(name)
                        .map_err(|reason| spm::error::Error::Usage(format!("'{name}': {reason}")))
                })
                .transpose()?;
            let found = ops::query::list(&store, &target, args.installed, from.as_ref())?;
            ui::packages(&found);
            Ok(())
        }
        Command::ListSources => {
            ui::list_sources(&ops::query::list_sources(&store)?);
            Ok(())
        }
        Command::SourceInfo(args) => {
            ui::source_info(&ops::query::source_info(&store, &args.url)?);
            Ok(())
        }
        Command::AddSource(args) => {
            let _lock = locked(&store)?;
            let transport = spm::net::https::Https::new();
            let added = ops::source::add_source(
                &store,
                &transport,
                &args.url,
                args.name.as_deref(),
                args.default,
            )?;
            ui::added_source(&added);
            Ok(())
        }
        Command::RemoveSource(args) => {
            let _lock = locked(&store)?;
            ui::removed_source(&ops::source::remove_source(&store, &args.url)?);
            Ok(())
        }
        Command::Create(args) => {
            let created = ops::create::create(&args.root, &args.metadata, &args.output)?;
            ui::created(&created);
            Ok(())
        }
    }
}
