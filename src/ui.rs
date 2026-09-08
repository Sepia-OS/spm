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
    fn a_date_comes_out_of_a_number_of_seconds() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_735_689_600), "2025-01-01");
        assert_eq!(date(1_757_260_800), "2025-09-07");
        // A leap day, which is where a hand-written calendar goes wrong.
        assert_eq!(date(1_709_164_800), "2024-02-29");
    }
}
