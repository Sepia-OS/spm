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

/// A date, from seconds since the epoch.
///
/// Absolute rather than "2 hours ago": a relative time needs to know what the
/// time is now, and on a device that has just booted that is exactly the thing
/// that cannot be relied on. It is also what makes this testable.
///
/// Howard Hinnant's civil-from-days, written out rather than taken from a
/// crate: one date in one format against a dozen lines of arithmetic.
#[must_use]
pub fn date(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

/// A number of bytes, the way a person reads one.
///
/// Binary multiples, because that is what a card's free space is measured in
/// and the two numbers appear in the same sentence when an install will not
/// fit. Here rather than in `error` so that there is one of these: an error
/// carries the text and `ops` composes it, the same way the clock message is
/// built out of [`date`].
#[must_use]
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut amount = bytes as f64;
    let mut unit = 0;
    while amount >= 1024.0 && unit + 1 < UNITS.len() {
        amount /= 1024.0;
        unit = unit.saturating_add(1);
    }

    let name = UNITS.get(unit).copied().unwrap_or("B");
    if unit == 0 {
        format!("{bytes} {name}")
    } else {
        format!("{amount:.1} {name}")
    }
}

/// Print every configured source, or say that there are none.
pub fn list_sources(reports: &[crate::ops::query::SourceReport]) {
    if reports.is_empty() {
        // Not an error: it is what a freshly installed device looks like.
        println!("No package sources are configured.");
        println!("Add one with: spm add-source <url>");
        return;
    }

    let width = reports
        .iter()
        .map(|report| report.name.as_str().len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!(
        "{:<width$}  {:<8} {:<12} {:>8}  {:>9}",
        "NAME",
        "DEFAULT",
        "UPDATED",
        "PACKAGES",
        "INSTALLED",
        width = width
    );
    for report in reports {
        let (updated, packages) = match report.index {
            Some(summary) => (date(summary.updated), summary.packages.to_string()),
            // Never fetched, which is not the same as offering nothing.
            None => ("never".to_owned(), "-".to_owned()),
        };
        println!(
            "{:<width$}  {:<8} {:<12} {:>8}  {:>9}",
            report.name.as_str(),
            if report.is_default { "yes" } else { "no" },
            updated,
            packages,
            report.installed,
            width = width
        );
        println!("{:width$}  {}", "", report.url, width = width);
    }
}

/// Print what is known about one source.
pub fn source_info(report: &crate::ops::query::SourceReport) {
    println!("Name       {}", report.name);
    println!("URL        {}", report.url);
    println!(
        "Default    {}",
        if report.is_default { "yes" } else { "no" }
    );
    match report.index {
        Some(summary) => {
            println!("Updated    {}", date(summary.updated));
            println!("Packages   {}", summary.packages);
        }
        None => {
            println!("Updated    never - run 'spm update' to fetch its index");
            println!("Packages   unknown until it has been updated");
        }
    }
    println!("Installed  {}", report.installed);
}

/// Say what adding a source did.
pub fn added_source(added: &crate::ops::source::Added) {
    println!(
        "Fetched the index for '{}': {} package{}.",
        added.name,
        added.packages,
        if added.packages == 1 { "" } else { "s" }
    );
    let what = if added.replaced { "Updated" } else { "Added" };
    let default = if added.is_default { " (default)" } else { "" };
    println!("{what} source '{}'{default}.", added.name);
}

/// Say what removing a source did.
pub fn removed_source(removed: &crate::ops::source::Removed) {
    println!(
        "Removed source '{}'. {} installed package{} came from it.",
        removed.name,
        removed.losing_upgrades,
        if removed.losing_upgrades == 1 {
            ""
        } else {
            "s"
        }
    );
    if removed.losing_upgrades > 0 {
        // They are not touched; what they lose is worth saying plainly.
        println!("They stay installed, but nothing is left to offer them newer versions.");
    }
    if let Some(name) = &removed.new_default {
        println!("'{name}' is now the default source.");
    }
    if removed.without_default {
        println!("There is now no default source.");
        println!("Name one with: spm add-source <url> --default");
    }
}

/// Say what `update` did, including what it could not do.
pub fn updated(report: &crate::ops::update::Report) {
    for updated in &report.updated {
        let new = if updated.new_packages > 0 {
            format!(" ({} new)", updated.new_packages)
        } else {
            String::new()
        };
        println!(
            "{}: {} package{}{new}.",
            updated.name,
            updated.packages,
            if updated.packages == 1 { "" } else { "s" }
        );
    }
    for failure in &report.failed {
        // To stderr: a script reading the listing should not have to sift
        // failures out of it.
        eprintln!(
            "{}: could not be updated - {}",
            failure.name, failure.message
        );
        eprintln!("    {}", failure.url);
    }
}

/// Say what an install did, or what it would have done.
///
/// The set first, dependencies before the package that was asked for, then the
/// download. A dry run prints the same thing and says that it changed nothing,
/// so that the two are comparable line for line.
pub fn installed(outcome: &crate::ops::install::Outcome) {
    recovered(&outcome.rolled_back);

    let plan = &outcome.plan;
    if plan.is_empty() {
        for already in &plan.satisfied {
            println!(
                "{} {} is already installed.",
                already.name, already.version.version
            );
        }
        if plan.satisfied.is_empty() {
            println!("There is nothing to install.");
        }
        return;
    }

    println!(
        "{}",
        if outcome.changed {
            "Installed:"
        } else {
            "The following will be installed:"
        }
    );

    let width = steps(plan);

    for already in &plan.satisfied {
        println!(
            "  {:<width$}  {:<10}(already installed)",
            already.name.as_str(),
            already.version.version.to_string(),
            width = width
        );
    }

    println!("Download: {}.", size(plan.download));
    diverted(&outcome.diverted);
    if !outcome.changed {
        // The point of --dry-run, said plainly rather than left to be inferred
        // from the absence of anything else.
        println!("Nothing was changed.");
    }
}

/// Name the configuration files whose new default was written beside the one on
/// the card.
///
/// Named rather than counted, and to stdout rather than stderr: a `.spmnew`
/// nobody is told about is a correction nobody will ever look at, and the whole
/// reason it was written instead of installed is that somebody has to decide.
fn diverted(paths: &[std::path::PathBuf]) {
    if paths.is_empty() {
        return;
    }
    if paths.len() == 1 {
        println!(
            "1 configuration file you had edited was left as it is; the new default is beside it:"
        );
    } else {
        println!(
            "{} configuration files you had edited were left as they are; the new defaults are beside them:",
            paths.len()
        );
    }
    for path in paths {
        println!("  /{}", path.display());
    }
}

/// Print the packages a plan would put on the device, and say how wide the
/// name column came out so that anything printed after them lines up.
///
/// Shared with `upgrade`, whose plan is an install plan like any other.
fn steps(plan: &crate::ops::install::Plan) -> usize {
    let width = plan
        .steps
        .iter()
        .map(|step| step.selected.name.as_str().len())
        .max()
        .unwrap_or(4);

    for step in &plan.steps {
        // Trimmed, because a package with nothing to say about it would
        // otherwise be a line ending in the padding of an empty column.
        let line = format!(
            "  {:<width$}  {:<10}{}",
            step.selected.name.as_str(),
            step.selected.version.version.to_string(),
            note(step),
            width = width
        );
        println!("{}", line.trim_end());
    }

    width
}

/// Say what `upgrade` did, including what it could not do.
pub fn upgraded(report: &crate::ops::upgrade::Report) {
    recovered(&report.rolled_back);

    if report.is_empty() {
        // "Everything is current" and "nothing could be moved" are opposite
        // things, and a package that is being held back is not a package that
        // is up to date.
        if report.held.is_empty() {
            println!("Everything is already at the newest version the indexes offer.");
        } else {
            println!("Nothing could be upgraded.");
        }
    } else {
        println!(
            "{}",
            if report.changed {
                "Upgraded:"
            } else {
                "The following will be upgraded:"
            }
        );
        steps(&report.plan);
        println!("Download: {}.", size(report.plan.download));
        diverted(&report.diverted);
        if !report.changed {
            println!("Nothing was changed.");
        }
    }

    for held in &report.held {
        // To stderr, like `update`'s failures: a script reading the listing
        // should not have to sift these out of it.
        eprintln!(
            "{} stays at {} - {} is offered and cannot be installed: {}",
            held.name, held.installed, held.offered, held.why
        );
    }
}

/// What a step does to what is already on the device.
fn note(step: &crate::ops::install::Step) -> String {
    use crate::ops::install::Change;

    let what = match &step.change {
        Change::New => String::new(),
        Change::Replaces(replaced) if replaced.source != step.selected.source => format!(
            "(replaces {}/{} {})",
            replaced.source, step.selected.name, replaced.version
        ),
        Change::Replaces(replaced) if replaced.version < step.selected.version.version => {
            format!("(upgrade from {})", replaced.version)
        }
        // Lower, or equal from the same source — which cannot reach here, so
        // this is the downgrade `install --version` allows and names.
        Change::Replaces(replaced) => format!("(downgrade from {})", replaced.version),
    };

    match (step.is_dependency(), what.is_empty()) {
        (true, true) => "(dependency)".to_owned(),
        (true, false) => format!("{what} (dependency)"),
        (false, _) => what,
    }
}

/// Say what `verify` found, package by package.
///
/// A sound package gets one line. A package with something wrong gets its files
/// listed underneath, because a count of faults is not something anybody can
/// act on and a path is.
///
/// Edited configuration is listed too and marked as what it is, so that it is
/// not read as damage: it is the expected result of administering a device, and
/// this is the only command that will tell you which files they are.
pub fn verified(report: &crate::ops::verify::Report) {
    use crate::ops::verify::Finding;

    if report.packages.is_empty() {
        println!("Nothing is installed, so there is nothing to check.");
        return;
    }

    let width = report
        .packages
        .iter()
        .map(|package| package.name.as_str().len())
        .max()
        .unwrap_or(4);

    for package in &report.packages {
        let summary = if package.findings.is_empty() {
            format!("{} files, all present", package.files)
        } else {
            let mut parts = Vec::new();
            if package.faults() > 0 {
                parts.push(format!("{} wrong", package.faults()));
            }
            if package.edited() > 0 {
                parts.push(format!("{} edited", package.edited()));
            }
            format!("{} files, {}", package.files, parts.join(", "))
        };
        println!(
            "  {:<width$}  {:<10}{}",
            package.name.as_str(),
            package.version.to_string(),
            summary,
            width = width
        );

        for checked in &package.findings {
            let what = match checked.finding {
                Finding::Missing => "missing",
                Finding::NotAFile => "not a file any more",
                Finding::Modified => "contents are not what was installed",
                Finding::Edited => "edited since it was installed",
            };
            println!("      /{}  - {what}", checked.path.display());
        }
    }

    if report.is_sound() {
        // Said plainly, because "no output means fine" is a thing people have
        // to learn and a sentence is not.
        if report.edited() > 0 {
            println!(
                "Everything installed is present. {} configuration file{} edited, which is not a fault.",
                report.edited(),
                if report.edited() == 1 { " is" } else { "s are" }
            );
        } else {
            println!("Everything installed is present and is what the records say.");
        }
    }
}

/// Say what a removal took away, or what it would take away.
///
/// The package that was asked for first, then whatever went with it, marked so
/// that a package somebody did not name is not mistaken for one they did.
pub fn removed(outcome: &crate::ops::remove::Outcome) {
    recovered(&outcome.rolled_back);

    let removal = &outcome.removal;
    println!(
        "{}",
        if outcome.changed {
            "Removed:"
        } else {
            "The following will be removed:"
        }
    );

    let width = removal
        .packages
        .iter()
        .map(|going| going.name.as_str().len())
        .max()
        .unwrap_or(4);
    for going in &removal.packages {
        let line = format!(
            "  {:<width$}  {:<10}{}",
            going.name.as_str(),
            going.version.to_string(),
            if going.unneeded {
                "(no longer needed)"
            } else {
                ""
            },
            width = width
        );
        println!("{}", line.trim_end());
    }

    let files = removal.files();
    println!("{files} file{} removed.", if files == 1 { "" } else { "s" });

    // Named rather than counted. A configuration file that outlives its package
    // is something the administrator has to decide about later, and a number
    // tells them nothing about which file or where.
    let kept: Vec<&std::path::PathBuf> = removal
        .packages
        .iter()
        .flat_map(|going| going.kept.iter())
        .collect();
    if !kept.is_empty() {
        if kept.len() == 1 {
            println!(
                "1 configuration file was edited since it was installed, and is left in place:"
            );
        } else {
            println!(
                "{} configuration files were edited since they were installed, and are left in place:",
                kept.len()
            );
        }
        for path in kept {
            println!("  /{}", path.display());
        }
    }

    if !outcome.changed {
        println!("Nothing was changed.");
    }
}

/// Say what an unfinished install left behind and what became of it.
pub fn recovered(names: &[crate::model::name::PackageName]) {
    for name in names {
        // To stderr: it is not part of the answer to what was asked, and a
        // script reading a listing should not have to sift it out.
        eprintln!(
            "An earlier install of '{name}' had not finished; the files it had written have been removed."
        );
    }
}

/// Say that another `spm` holds the lock, and what is being waited for.
pub fn waiting_for_lock(holder: Option<u32>) {
    match holder {
        Some(pid) => eprintln!("Waiting for another spm to finish (process {pid})..."),
        None => eprintln!("Waiting for another spm to finish..."),
    }
}

/// How a package has to be written: qualified when two sources offer the name.
fn written(line: &crate::ops::query::Line) -> String {
    if line.qualify {
        format!("{}/{}", line.source, line.name)
    } else {
        line.name.to_string()
    }
}

/// Print the results of a search or a listing.
pub fn packages(lines: &[crate::ops::query::Line]) {
    let width = lines
        .iter()
        .map(|line| written(line).len())
        .max()
        .unwrap_or(4);
    for line in lines {
        let version = match &line.newest {
            Some(version) => version.to_string(),
            // It exists, but not for this machine.
            None => "-".to_owned(),
        };
        let installed = match &line.installed {
            Some(have) if Some(have) == line.newest.as_ref() => "  [installed]".to_owned(),
            Some(have) => format!("  [installed {have}]"),
            None => String::new(),
        };
        println!(
            "{:<width$}  {:<10} {:<8}{installed}",
            written(line),
            version,
            line.source.as_str(),
            width = width
        );
        if !line.description.is_empty() {
            // The first line only: a description may be a paragraph.
            let first = line.description.lines().next().unwrap_or_default();
            println!("{:width$}  {first}", "", width = width);
        }
    }
}

/// Print everything known about a package, once per source that offers it.
pub fn package_detail(details: &[crate::ops::query::Detail]) {
    for (at, detail) in details.iter().enumerate() {
        if at > 0 {
            println!();
        }
        let name = if detail.qualify {
            format!("{}/{}", detail.source, detail.name)
        } else {
            detail.name.to_string()
        };
        println!("Name          {name}");
        println!("Version       {}", detail.version.version);
        println!("Target        {}", detail.version.target);
        println!("Source        {}", detail.source);
        println!("Description   {}", detail.description);
        if detail.version.dependencies.is_empty() {
            println!("Dependencies  none");
        } else {
            let needs: Vec<String> = detail
                .version
                .dependencies
                .iter()
                .map(|dependency| format!("{} >= {}", dependency.name, dependency.version))
                .collect();
            println!("Dependencies  {}", needs.join(", "));
        }
        match &detail.installed {
            Some(version) => println!("Installed     {version}"),
            None => println!("Installed     no"),
        }
        let versions: Vec<String> = detail
            .versions
            .iter()
            .map(std::string::ToString::to_string)
            .collect();
        println!("Versions      {}", versions.join(", "));
    }
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

    #[test]
    fn a_size_is_written_the_way_a_person_reads_one() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1024), "1.0 KiB");
        // The number the user guide shows for a helix-sized package.
        assert_eq!(size(16_148_070), "15.4 MiB");
        assert_eq!(size(2 * 1024 * 1024 * 1024), "2.0 GiB");
        // Nothing overflows off the end of the table.
        assert!(size(u64::MAX).ends_with("TiB"));
    }

    #[test]
    fn a_date_comes_out_of_a_number_of_seconds() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_735_689_600), "2025-01-01");
        assert_eq!(date(1_757_260_800), "2025-09-07");
        // A leap day, which is where a hand-written calendar goes wrong.
        assert_eq!(date(1_709_164_800), "2024-02-29");
    }
}
