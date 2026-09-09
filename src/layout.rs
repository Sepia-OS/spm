/*
  layout.rs

  Created on 2026-09-09 by Thomas Bonk <thomas@meandmymac.de>
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

//! Where on a device a package may put things.
//!
//! One list, consulted twice: [`crate::ops::create`] refuses a staged tree that
//! steps outside it, and [`crate::unpack`] refuses an entry that does, because a
//! package can reach a device without having passed through this program's
//! `create`. Two enforcement points and one definition, so they cannot drift.
//!
//! **This used to be `usr/` and `etc/` and nothing else**, on the reasoning that
//! a package writing anywhere else was altering the system rather than adding to
//! it. That reasoning held only as long as everything below the applications was
//! baked into the image. It made two packages the operating system actually
//! needs impossible to express:
//!
//! - **busybox** ships `/bin/sh`, `/sbin/init` and several hundred applet
//!   symlinks. Its applets declare where they belong, and a card whose
//!   `/bin/sh` does not exist cannot run a script.
//! - **musl** ships the dynamic loader at `/lib/ld-musl-aarch64.so.1`. That path
//!   is compiled into every dynamically linked binary on the card, so no other
//!   directory will do.
//!
//! ## An allowlist, not a denylist
//!
//! A denylist would silently permit every directory somebody invents later,
//! which is the wrong default for the one part of this program that writes into
//! `/` as root. So the roots are named, and each is named for a reason:
//!
//! | | |
//! |---|---|
//! | `bin`, `sbin` | the commands the system boots into, busybox's among them |
//! | `lib` | the dynamic loader, whose path is compiled into every binary |
//! | `usr` | everything above the base system - the package's own, outright |
//! | `etc` | defaults an administrator may then edit; see [`crate::conffile`] |
//!
//! What is left out is left out deliberately:
//!
//! - **`var`** holds `/var/lib/spm`, which is this program's own database. A
//!   package able to write there could forge an install record, or corrupt the
//!   one that says what it is allowed to remove. State a package needs at
//!   runtime is state it makes at runtime.
//! - **`boot`** belongs to `Sepia-OS/boot` and is read by the firmware before
//!   anything here exists.
//! - **`dev`, `proc`, `sys`, `run`, `tmp`** are kernel or volatile filesystems.
//!   Nothing installed belongs in one, and what is written there does not
//!   survive to be removed again.
//! - **`home`, `root`, `mnt`, `media`, `opt`, `srv`** are people's files and
//!   mount points. A package that wants one of these wants a decision made by
//!   whoever runs the device.
//!
//! ## What this is not
//!
//! It is not what stops two packages fighting over one file, and it never was.
//! `ops::install` refuses to write over a file another record claims
//! ([`crate::error::Error::FileConflict`]) or a file no record claims at all
//! ([`crate::error::Error::FileUnowned`]), and `ops::remove` refuses to take
//! away a package something else depends on
//! ([`crate::error::Error::HasDependents`]). Those are the checks that keep a
//! card with a libc in its image from acquiring a second one; this list only
//! says which directories are addressable at all.

use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use crate::conffile;

/// The top-level directories a package may write in.
///
/// Sorted, because it is rendered into error messages and a list whose order
/// depended on how it happened to be typed would make two otherwise identical
/// failures read differently.
pub const ROOTS: [&str; 5] = ["bin", conffile::ETC, "lib", "sbin", "usr"];

/// Whether a relative path begins with a directory a package may write in.
///
/// Asked of a path that has already been checked for `..`, a leading `/` and
/// the rest of what [`crate::unpack`] refuses; this asks only where it lands.
/// An empty path is not one of them - it names the root of the device itself.
#[must_use]
pub fn is_writable_root(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Normal(first)) if ROOTS.iter().any(|root| first == *root)
    )
}

/// Where a symbolic link lands, relative to the root of the device.
///
/// A link is a path a package writes, so where it points is checked the same way
/// the path itself is. This resolves the target against the directory the link
/// sits in and hands back the landing place for [`is_writable_root`] to judge;
/// it does not touch the filesystem, and says nothing about whether anything is
/// there.
///
/// **An absolute target starts again from the root of the device**, which is
/// what `/` means to the card that ends up with the link. musl's loader is
/// exactly this and could not be packaged otherwise: `lib/ld-musl-aarch64.so.1`
/// points at `/usr/lib/libc.so`, and the absolute spelling is upstream's, not a
/// mistake — the file is both the shared libc and the program interpreter, and
/// every binary on the card names that path in its `PT_INTERP`.
///
/// This grants nothing a relative target did not already have. `usr/bin/x`
/// pointing at `../../etc/shadow` resolves to `etc/shadow` and always has; the
/// absolute spelling of the same place now resolves to the same answer instead
/// of being refused for its punctuation. What stops a package writing *through*
/// a link is [`crate::unpack`] refusing to follow one, not this.
///
/// `None` if the target walks up past the root of the device, which is a link
/// pointing at nothing this program can reason about.
#[must_use]
pub fn resolve_link(link: &Path, target: &Path) -> Option<PathBuf> {
    let mut resolved: Vec<Component<'_>> = link
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect();

    for part in target.components() {
        match part {
            // Absolute: whatever the link's own directory was, `/` restarts.
            Component::RootDir => resolved.clear(),
            Component::Normal(_) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop()?;
            }
            // A drive letter, which cannot occur on the device and is not
            // something to guess the meaning of.
            Component::Prefix(_) => return None,
        }
    }

    Some(resolved.iter().collect())
}

/// The roots as a sentence: `bin/, etc/, lib/, sbin/ or usr/`.
///
/// Built rather than written out, so that adding a root cannot leave an error
/// message describing the list as it used to be.
#[must_use]
pub fn listed() -> String {
    let last = ROOTS.len().saturating_sub(1);
    ROOTS
        .iter()
        .enumerate()
        .fold(String::new(), |mut text, (index, root)| {
            if index == last && index > 0 {
                text.push_str(" or ");
            } else if index > 0 {
                text.push_str(", ");
            }
            // Writing to a String cannot fail, and the result is discarded
            // rather than unwrapped so that this stays free of the panicking
            // helpers.
            let _ = write!(text, "{root}/");
            text
        })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything here is test code, and a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    #[test]
    fn the_roots_the_operating_system_needs_are_writable() {
        // One case per reason the list has the entry, so that removing an entry
        // fails with the reason rather than with a bare count.
        assert!(is_writable_root(Path::new("usr/bin/hx")));
        assert!(is_writable_root(Path::new("etc/helix.conf")));
        assert!(
            is_writable_root(Path::new("bin/sh")),
            "busybox ships /bin/sh, and a card without one cannot run a script"
        );
        assert!(
            is_writable_root(Path::new("sbin/init")),
            "busybox's applets declare where they belong"
        );
        assert!(
            is_writable_root(Path::new("lib/ld-musl-aarch64.so.1")),
            "the loader's path is compiled into every dynamically linked binary"
        );
    }

    #[test]
    fn a_bare_root_directory_is_itself_writable() {
        // The directory entries of an archive arrive too, and `bin` is one.
        assert!(is_writable_root(Path::new("bin")));
        assert!(is_writable_root(Path::new("usr")));
    }

    #[test]
    fn the_directories_left_out_stay_out() {
        // var/ is the one that matters most: /var/lib/spm is this program's own
        // database, and a package that could write there could forge a record
        // saying it owned a file it did not install.
        assert!(!is_writable_root(Path::new("var/lib/spm/installed/x.json")));
        assert!(!is_writable_root(Path::new("boot/config.txt")));
        assert!(!is_writable_root(Path::new("dev/sda")));
        assert!(!is_writable_root(Path::new("proc/1/mem")));
        assert!(!is_writable_root(Path::new("sys/kernel")));
        assert!(!is_writable_root(Path::new("run/spm.pid")));
        assert!(!is_writable_root(Path::new("tmp/x")));
        assert!(!is_writable_root(Path::new("home/pi/.profile")));
        assert!(!is_writable_root(Path::new("root/.ssh/authorized_keys")));
        assert!(!is_writable_root(Path::new("opt/thing")));
        assert!(!is_writable_root(Path::new("srv/www")));
    }

    #[test]
    fn the_root_of_the_device_is_not_a_root_a_package_may_write() {
        assert!(!is_writable_root(Path::new("")));
    }

    #[test]
    fn a_root_is_matched_whole_rather_than_as_a_prefix() {
        // `binary/` is not `bin/`, and `libexec/` is not `lib/`. Matching on a
        // string prefix rather than on the path component would let both in.
        assert!(!is_writable_root(Path::new("binary/thing")));
        assert!(!is_writable_root(Path::new("libexec/thing")));
        assert!(!is_writable_root(Path::new("usrlocal/thing")));
        assert!(!is_writable_root(Path::new("etcetera/thing")));
    }

    #[test]
    fn a_root_only_counts_at_the_top() {
        // `usr/share/etc/x` is the package's own file, not configuration, and
        // `usr/lib` is not the loader's `lib`. Only the first component decides.
        assert!(is_writable_root(Path::new("usr/share/etc/x")));
        assert!(!is_writable_root(Path::new("var/usr/x")));
        assert!(!is_writable_root(Path::new("opt/bin/x")));
    }

    #[test]
    fn the_listed_roots_read_as_a_sentence() {
        assert_eq!(listed(), "bin/, etc/, lib/, sbin/ or usr/");
    }

    #[test]
    fn every_root_appears_in_the_sentence() {
        // The point of building the string is that it cannot fall behind the
        // array, so that is what is asserted rather than the exact wording.
        let listed = listed();
        for root in ROOTS {
            assert!(listed.contains(&format!("{root}/")), "{root} is missing");
        }
    }

    #[test]
    fn a_relative_target_resolves_against_the_links_own_directory() {
        // busybox's applets, which is the common case: several hundred links in
        // one directory pointing at one file in it.
        assert_eq!(
            resolve_link(Path::new("bin/sh"), Path::new("busybox")),
            Some(PathBuf::from("bin/busybox"))
        );
        assert_eq!(
            resolve_link(Path::new("sbin/init"), Path::new("../bin/busybox")),
            Some(PathBuf::from("bin/busybox"))
        );
        assert_eq!(
            resolve_link(Path::new("usr/bin/awk"), Path::new("../../bin/busybox")),
            Some(PathBuf::from("bin/busybox"))
        );
    }

    #[test]
    fn an_absolute_target_starts_again_from_the_root_of_the_device() {
        // musl's loader, and the reason this function exists. The absolute
        // spelling is upstream's: every binary on the card names that path in
        // its PT_INTERP.
        assert_eq!(
            resolve_link(
                Path::new("lib/ld-musl-aarch64.so.1"),
                Path::new("/usr/lib/libc.so")
            ),
            Some(PathBuf::from("usr/lib/libc.so"))
        );
        // The link's own directory is discarded rather than prepended - the
        // answer must not depend on where the link happens to sit.
        assert_eq!(
            resolve_link(Path::new("usr/lib/x"), Path::new("/lib/y")),
            resolve_link(Path::new("bin/x"), Path::new("/lib/y"))
        );
    }

    #[test]
    fn a_target_that_walks_above_the_root_has_nowhere_to_land() {
        assert_eq!(
            resolve_link(Path::new("bin/sh"), Path::new("../../..")),
            None
        );
        assert_eq!(
            resolve_link(Path::new("usr/bin/x"), Path::new("../../../../etc/passwd")),
            None
        );
    }

    #[test]
    fn a_target_outside_the_roots_is_visible_as_such() {
        // resolve_link only says where it lands; is_writable_root judges it.
        // Splitting the two is what lets create and unpack phrase their own
        // refusals while agreeing on the answer.
        let landed = resolve_link(Path::new("usr/bin/x"), Path::new("/var/lib/spm"))
            .expect("it lands somewhere");
        assert_eq!(landed, PathBuf::from("var/lib/spm"));
        assert!(!is_writable_root(&landed));
    }

    #[test]
    fn an_absolute_target_grants_nothing_a_relative_one_did_not() {
        // The two spellings of one place resolve alike, which is the argument
        // for accepting the absolute one at all.
        assert_eq!(
            resolve_link(Path::new("usr/bin/x"), Path::new("/etc/helix.conf")),
            resolve_link(Path::new("usr/bin/x"), Path::new("../../etc/helix.conf"))
        );
    }

    #[test]
    fn a_curdir_component_changes_nothing() {
        assert_eq!(
            resolve_link(Path::new("bin/sh"), Path::new("./busybox")),
            Some(PathBuf::from("bin/busybox"))
        );
    }

    #[test]
    fn etc_is_spelled_once() {
        // conffile owns what `etc/` *means*; this module only says it is
        // addressable. Two spellings of the name would let one drift.
        assert!(ROOTS.contains(&conffile::ETC));
    }
}
