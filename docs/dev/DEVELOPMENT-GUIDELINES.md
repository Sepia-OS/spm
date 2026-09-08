# Development Guidelines

## File Headers

Each source file shall have the following file header:

```rust
/*
  <filename>

  Created on <yyyy-mm-dd> by <author> <<email address>>
  Copyright <yyyy> SepiaOS Development Team

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
```

**The indentation is not a matter of taste.** `rustfmt` normalises a leading
block comment, so a header indented any other way is silently rewritten the
first time somebody runs `cargo fmt`, and fails `cargo fmt --check` until they
do. Two spaces and a `*/` in column one is what it leaves alone.

Neither `rustfmt` nor `clippy` knows the header has to be *there*, so nothing
will tell you when it is missing. CI checks that with a grep over
`src/**/*.rs` and fails the build if a file has none.

## Toolchain

- **Edition 2024**, as `Cargo.toml` already says.
- **Rust 1.98.1 is the floor.** That is the version `Sepia-OS/rust-toolchain`
  publishes and therefore the newest one a SepiaOS device has. A package
  manager that its own operating system cannot compile would be an odd thing,
  so nothing goes in that needs a later compiler. Set it explicitly:

  ```toml
  [package]
  rust-version = "1.98.1"
  ```

  With that in place `cargo` refuses a too-new feature at build time rather
  than leaving it to be discovered on a device.
- **No nightly.** Not for features, not for `fmt`, not for a lint.

## Formatting and lints

`rustfmt` with its defaults — no `rustfmt.toml`. The default style is not
better than a considered one, but it is the one everybody else's tools already
agree with, and a formatting discussion costs more than it settles.

Lints are configured in `Cargo.toml` so that they apply to `cargo build` in a
worktree exactly as they do in CI:

```toml
[lints.rust]
unsafe_code = "deny"
missing_debug_implementations = "warn"

[lints.clippy]
all = { level = "deny", priority = -1 }
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
```

CI runs `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
`--all-targets` matters: without it the tests are not linted, and the tests are
where the sloppiness accumulates.

## Errors

- **One error enum**, in `error.rs`, built with `thiserror`. Every variant maps
  to a documented exit code, and the mapping has a test so that reordering the
  enum cannot silently change a number a script depends on.
- **No `anyhow` outside `main.rs`.** A library module that returns
  `anyhow::Error` has decided its caller will never want to distinguish one
  failure from another, which is exactly what the exit codes need to do.
- **An error message says what happened, to what, and what the user can do.**
  `"failed to open file"` fails all three tests. `"cannot read
  /var/lib/spm/index/sepia.json: no such file — run 'spm update'"` passes.
- **Context belongs in the variant, not in a formatted string.** Carry the path
  or the package name as a field; `Display` composes the sentence.

## Panics

`spm` runs as root and writes into `/`. A panic halfway through an extraction
is a device left in a state nobody designed.

- **`unwrap` and `expect` are denied outside tests.** Inside `#[cfg(test)]`
  they are fine and expected — a test that cannot fail loudly is worse.
- **No slice indexing where the index is not provably in range.** Prefer
  `get()` and handle the `None`.
- **Arithmetic on sizes and counts uses checked or saturating operations.** A
  release build wraps silently, and the numbers here come from a file that a
  package published.
- **A panic is a bug, not an error path.** If a condition can happen because of
  bad input, bad network, or a bad package, it is an `Error` variant.

## `unsafe`

Denied at crate level, and **the crate contains none**. The one use the design
expected — `flock` through `libc` — turned out to be unnecessary:
`std::fs::File::lock` has been stable since Rust 1.89.

If a case ever does arise, it is isolated to the smallest possible module with
`#[allow(unsafe_code)]`, and every block carries a `// SAFETY:` comment saying
what invariant makes it sound:

```rust
// SAFETY: fd is owned by the File we hold for the duration of the call, and
// the call does not retain it.
```

A block without one does not pass review. Before writing the first one, check
whether std has grown the thing you need — it did here.

## Types

- **Paths are `Path` and `PathBuf`, never `String`.** A package's file list
  arrives from a tar archive, and a path that is not valid UTF-8 is a path, not
  an error to paper over with `to_string_lossy`.
- **Wrap the identifiers.** `PackageName`, `SourceName` and `Version` are
  types, not `String`s. They parse once, at the edge, and everything inside
  works with something already known to be well-formed.
- **`u64` for sizes**, and no casts to `usize` without a check — a 32-bit build
  is not the target, but the code should not quietly depend on that.

## I/O

- **Nothing proportional to a package is held in memory.** No `read_to_end` on
  a download, no `Vec<u8>` of an archive. Packages reach 216 MiB and the
  smallest supported board has 512 MiB of RAM. Stream, and hash as part of the
  same pass.
- **Buffer explicitly.** `BufReader`/`BufWriter` around files; unbuffered
  per-entry writes during an extraction of 11,000 files are the difference
  between seconds and minutes on an SD card.
- **Every write that must survive a power cut goes through `store::atomic`** —
  temporary file in the destination directory, `fsync`, `rename`. Not `/tmp`: a
  rename across filesystems is not atomic.

## Serde and on-disk formats

- **`#[serde(deny_unknown_fields)]` on everything read from disk or the
  network.** A misspelled key in a hand-written `metadata.json` is a mistake to
  report, not to ignore. The cost is that adding a field is a format change,
  which is the right cost.
- **Round-trip tests for every format**, using the example from
  [ARCHITECTURE.md](ARCHITECTURE.md) as the fixture, so the documentation and
  the parser cannot drift apart.
- **Names in the file are the names in the document.** `payload_sha256` is not
  shortened to `psha` because a struct field looked long; the index carries two
  digests and confusing them is a verification that passes while checking
  nothing.

## Module boundaries

The layout in [DESIGN.md](DESIGN.md) is a rule, not a suggestion:

- `store/` and `net/` are mechanism. They know nothing about commands, take no
  policy decisions, and print nothing.
- `ops/` is policy. It decides, and returns what happened.
- `ui.rs` is the only module that writes to stdout. A `println!` anywhere else
  is how output stops being consistent and starts being untestable.
- **`ops` functions take a `&dyn Transport`.** That is what lets every test run
  the real code path with no network.

## Dependencies

Adding a crate is a decision to be defended in the pull request, against:

- **It must cross-compile statically to `aarch64-unknown-linux-musl`.**
- **No C dependencies.** `zlib`, `openssl` and their relatives are the usual
  reason a musl cross-build stops working; prefer the pure-Rust backend even
  when it is slower (`flate2` with `rust_backend`, `rustls` over `native-tls`).
- **No async runtime.** This program makes a handful of sequential requests;
  `tokio` would be the largest dependency in the tree and buy nothing.
- **Check what it drags in**, not just what it is. `cargo tree -d` before, not
  after.

`Cargo.lock` is committed and updated deliberately, in its own commit.

## Testing

- **Unit tests live beside the code** in `#[cfg(test)] mod tests`; integration
  tests live in `tests/` and use the public entry points.
- **Every test module and every file under `tests/` carries the same allow**:

  ```rust
  #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic,
           reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse")]
  ```

  The lint table denies those crate-wide, and clippy cannot tell that a file in
  `tests/` is a test — each one is its own crate, so a per-module allow does not
  reach it. Inside `src/`, the same three go on the `#[cfg(test)] mod tests`.
- **No test touches the network.** The `Transport` trait exists for this;
  fixtures are served from a temporary directory.
- **No test writes outside a temporary root.** Everything is built from the
  configurable prefix, so a test that installs does so into `tempfile::tempdir()`.
- **Fixture packages are built by `create`**, not checked in. One
  implementation of the format, and a test that would notice if it changed.
- **Tests are deterministic.** No wall-clock, no randomness, no ordering that
  depends on a filesystem's iteration order — sort before asserting.
- **A bug fix comes with the test that would have caught it.** This is where
  the extraction rules and the version ordering earn their keep.

## Comments and documentation

- **`//!` at the top of each module** saying what it owns and what it does not.
- **`///` on every public item**, saying what it does, and what it returns on
  the interesting failure.
- **Comments explain why.** What the code does is legible from the code; why it
  was done that way, and what happens if it is changed back, is not. The
  reasons worth writing down are usually the ones that were expensive to learn:
  ownership is not restored from an archive because doing so broke a CI run;
  the digest is checked before the archive is opened because opening it is
  already trusting it.

## Cross-compilation

Three things that will cost an afternoon each if they are forgotten:

- **Build scripts and proc macros run on the *host*.** A cross-build still
  needs a host C compiler; a container with only the cross toolchain fails with
  ``linker `cc` not found`` in the middle of a dependency.
- **`ring`, under `rustls`, needs a C compiler for the target.** That is the
  cross toolchain the sibling repositories already use.
- **The build host is not the target.** Nothing may depend on the host's libc,
  page size, or path conventions. If a test only passes on the machine that
  built it, it is testing the machine.

## Branches

**Work on a module starts by creating a branch for it**, named after the module
in [the implementation plan](IMPLEMENTATION-PLAN.md):

```sh
git switch -c feat/M1-the-model        # M1 - The model
```

Every step in that module is committed on that branch, and the branch merges
back to `main` when the module is finished — which is also when the module gets
its ✅ in the plan. So a branch is one module's worth of work, and `main` moves
a module at a time rather than a step at a time.

Naming is `feat/<module>-<title>`, where `<module>` is the identifier the plan
uses and `<title>` is the title of the module, with spaces replaced by `-` and
all characters lowercase. The plan is the index of what a branch is for, and a
name that has to be translated to find it is worth nothing.

CI runs on every branch, so a module's branch is checked from its first commit
rather than at the point it is merged.

## Changes and the changelog

Every change to this repository gets an entry in [CHANGELOG.md](../../CHANGELOG.md),
under `## [Unreleased]`, **in the same commit as the change**. The format is
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).

An entry says what changed and, when it is not obvious, why it mattered. The
changelog is read by people deciding whether to upgrade a device; "refactored
`install`" tells them nothing, and "an interrupted install no longer leaves
files behind" tells them everything.
