/*
  cli.rs

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

//! The command-line surface: one type per command, and nothing else.
//!
//! Parsing only. A command's behaviour lives in `ops`, so that the tests can
//! call it without going through argument parsing.
//!
//! `clap` handles a malformed command line itself: it prints the usage and
//! exits 2, which is the code `docs/USER-GUIDE.md` documents for wrong usage.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// `spm`.
#[derive(Debug, Parser)]
#[command(name = "spm", version, about = "The SepiaOS package manager")]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The commands. One variant per command in `docs/dev/ARCHITECTURE.md`; the
/// rest arrive with the steps that implement them.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Fetch the package indexes from the configured sources
    Update(UpdateArgs),

    /// Install a package and everything it needs
    Install(InstallArgs),

    /// Remove a package, and anything that came in with it and is now unneeded
    Remove(RemoveArgs),

    /// Move installed packages onto the newest versions the indexes offer
    Upgrade(UpgradeArgs),

    /// Search the indexes for a package
    Search(SearchArgs),

    /// Show everything known about a package
    Info(InfoArgs),

    /// List packages
    List(ListArgs),

    /// List the configured package sources
    ListSources,

    /// Show what is known about one package source
    SourceInfo(SourceInfoArgs),

    /// Add a package source
    AddSource(AddSourceArgs),

    /// Remove a package source, without uninstalling anything
    RemoveSource(RemoveSourceArgs),

    /// Turn a staged tree into an installable package
    Create(CreateArgs),
}

/// `spm update`.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Update every source. This is what happens when neither option is given
    #[arg(long, conflicts_with = "source")]
    pub all: bool,

    /// Update only this source
    #[arg(long, value_name = "NAME")]
    pub source: Option<String>,
}

/// `spm install`.
#[derive(Debug, Args)]
pub struct InstallArgs {
    /// The package, as `<package>` or `<source>/<package>`
    #[arg(value_name = "PACKAGE")]
    pub package: String,

    /// Install this version rather than the newest
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,

    /// Work out what would be installed and change nothing
    #[arg(long)]
    pub dry_run: bool,
}

/// `spm remove`.
#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// The package, as `<package>` or `<source>/<package>`
    #[arg(value_name = "PACKAGE")]
    pub package: String,

    /// Work out what would be removed and change nothing
    #[arg(long)]
    pub dry_run: bool,
}

/// `spm upgrade`.
#[derive(Debug, Args)]
pub struct UpgradeArgs {
    /// Upgrade only this package rather than everything installed
    #[arg(value_name = "PACKAGE")]
    pub package: Option<String>,

    /// Work out what would be upgraded and change nothing
    #[arg(long)]
    pub dry_run: bool,
}

/// `spm search`.
#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Part of a package name
    #[arg(value_name = "PACKAGE")]
    pub needle: String,
}

/// `spm info`.
#[derive(Debug, Args)]
pub struct InfoArgs {
    /// The package, as `<package>` or `<source>/<package>`
    #[arg(value_name = "PACKAGE")]
    pub package: String,

    /// Show this version rather than the newest
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,
}

/// `spm list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Only the packages that are installed
    #[arg(long)]
    pub installed: bool,

    /// Only the packages from this source
    #[arg(long, value_name = "NAME")]
    pub source: Option<String>,
}

/// `spm source-info`.
#[derive(Debug, Args)]
pub struct SourceInfoArgs {
    /// The source, by the name it is configured under or by its URL
    #[arg(value_name = "NAME|URL")]
    pub source: String,
}

/// `spm add-source`.
#[derive(Debug, Args)]
pub struct AddSourceArgs {
    /// Where the source publishes its index
    #[arg(value_name = "URL")]
    pub url: String,

    /// Make this the source used when a command needs one and none is given
    #[arg(long)]
    pub default: bool,

    /// Use this name rather than the one the index declares
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
}

/// `spm remove-source`.
#[derive(Debug, Args)]
pub struct RemoveSourceArgs {
    /// The source, by the name it is configured under or by its URL
    #[arg(value_name = "NAME|URL")]
    pub source: String,
}

/// `spm create`.
#[derive(Debug, Args)]
pub struct CreateArgs {
    /// The staged tree to pack, laid out as it will appear on the device
    #[arg(long, value_name = "DIRECTORY")]
    pub root: PathBuf,

    /// The package's metadata
    #[arg(long, value_name = "FILE", default_value = "metadata.json")]
    pub metadata: PathBuf,

    /// Where to write the package, its metadata and SHA256SUMS
    #[arg(long, value_name = "DIRECTORY", default_value = ".")]
    pub output: PathBuf,
}
