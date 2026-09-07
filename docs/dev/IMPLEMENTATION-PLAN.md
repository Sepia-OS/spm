# `spm` Implementation Plan

The order in which [DESIGN.md](DESIGN.md) gets built. Each step is small enough
to finish in one sitting, leaves the tree compiling and its tests passing, and
says what has to be true before it counts as done.

Two rules of sequencing shape the order:

- **Nothing is built before what it depends on**, so no step is written against
  an interface that does not exist yet.
- **`create` comes early, before anything that installs.** It is the one
  command that needs neither the network nor the installed database, and it is
  what produces the fixture packages every later test installs. Building it
  fourth rather than last saves writing throwaway fixtures by hand.

Each step is written as:

> **Goal** — what exists afterwards that did not before.
> **Files** — what is added or changed.
> **Notes** — the decisions and the traps.
> **Done when** — what has to pass.

**Finished work is marked here.** A step that is done gets a ✅ on its heading,
and a milestone gets one when every step under it has. This document is
therefore the status as well as the plan, so there is no second place to keep
in step with it.

---

## M0 — Groundwork ✅

### Step 1 — Fix the lockfile ✅

**Goal.** Reproducible builds.
**Files.** `.gitignore`, `Cargo.lock`.
**Notes.** The `.gitignore` came from a generic template that ignores
`Cargo.lock`, with a comment in it saying to remove that line when the crate is
an executable. It is one. The same class of mistake broke `Sepia-OS/rust-toolchain`'s
CI, where a template `.gitignore` swallowed the `Makefile` and the build ran
against a tree that did not contain it.
**Done when.** `git check-ignore Cargo.lock` says nothing, and `Cargo.lock` is
tracked.
**Done.** The template's three lines were replaced with a comment saying why the
lockfile is kept. `git check-ignore` is silent, `Cargo.lock` is tracked, and
`cargo build` is clean.

### Step 2 — Crate skeleton ✅

**Goal.** The module tree from DESIGN.md exists, empty but compiling.
**Files.** `src/main.rs`, and a stub for every module the design names.
**Notes.** Declare the modules and nothing else. A tree of empty modules that
compiles is a scaffold; a tree of half-written ones is a merge conflict with
yourself.
**Done when.** `cargo build` and `cargo clippy -- -D warnings` are clean.
**Done.** 30 files: every module in the design's tree, each with the licence
header and a `//!` saying what it owns. `build`, `fmt --check` and
`clippy --all-targets -- -D warnings` are all clean. Three things came with it
that the step did not ask for, and the plan had nowhere else to put them: the
`[lints]` table from the guidelines went into `Cargo.toml`, because a later step
written without those lints would only have to meet them afterwards;
`ops/source.rs` was created because Steps 22 and 23 name it while the design's
tree did not list it, so the tree was corrected rather than the plan; and the
header template in the guidelines was re-indented to what `rustfmt` leaves
alone, since as written it failed `cargo fmt --check` in every file.

### Step 3 — Errors and exit codes ✅

**Goal.** One error type, and the exit code table from DESIGN.md.
**Files.** `src/error.rs`, `src/main.rs`.
**Notes.** Write the whole table now, including the variants nothing returns
yet. Retro-fitting exit codes means every call site is revisited.
**Done when.** `main` maps every variant to its code, and a unit test asserts
the mapping so a reordering cannot change a documented number.
**Done.** Sixteen variants covering all eight codes, with `thiserror` for the
messages and five tests over them. The guard turned out to be stronger than a
test: `exit_code` matches exhaustively with no catch-all arm, so a variant added
without a code does not compile — verified by adding one and reading the
`non-exhaustive patterns` error. The crate also gained `src/lib.rs`: `tests/`
cannot reach inside a binary crate, and the design asks for tests that drive
whole commands, so the modules live in a library and `main.rs` is now only the
mapping from a result to an exit code.

---

## M1 — The model

### Step 4 — `Version` and its ordering

**Goal.** Upstream versions compare correctly.
**Files.** `src/model/version.rs`.
**Notes.** Not semver. Numeric components compare numerically, so `25.07.1`
equals `25.7.1`; a numeric component sorts above a non-numeric one, so `1.0` is
above `1.0-rc1`; a missing component is zero, so `1.2` equals `1.2.0`.
**Done when.** Tests cover all four rules, `25.07.1 == 25.7.1`, `1.10 > 1.9`
(the comparison a string sort gets wrong), and sorting a shuffled list of real
SepiaOS versions — `1.2.6`, `23.1.0`, `25.07.1`, `4.4.1` — gives the expected
order.

### Step 5 — Names

**Goal.** `PackageName`, `SourceName`, and the `<source>/<package>` form.
**Files.** `src/model/name.rs`.
**Notes.** One type that parses both the qualified and unqualified spellings,
so no command re-implements the split. Reject empty parts, whitespace, and a
name with more than one `/`.
**Done when.** Round-trip tests for both forms, and rejection tests for the
malformed ones.

### Step 6 — `Metadata`

**Goal.** A package's `metadata.json` as a type.
**Files.** `src/model/metadata.rs`.
**Notes.** `name`, `version`, `target`, `description`, `dependencies`,
`sha256`. Deserialising must reject an unknown field rather than ignore it: a
misspelled key in a hand-written metadata file is a mistake to report, not to
drop silently.
**Done when.** The example from ARCHITECTURE.md round-trips, an empty `sha256`
is accepted (that is what an author writes), and a missing `name` is an error.

### Step 7 — `Index` and `Record`

**Goal.** The other two formats as types.
**Files.** `src/model/index.rs`, `src/model/installed.rs`.
**Notes.** The index carries both digests under distinct names — `sha256` for
the package and `payload_sha256` for `data.tar.gz`. Do not shorten either;
confusing them is a verification that passes while checking nothing.
**Done when.** Both round-trip, and a test asserts an `Index` selects the
highest version for a given target.

---

## M2 — Local state

### Step 8 — Root prefix

**Goal.** Every path is built from one configurable prefix.
**Files.** `src/store/mod.rs`.
**Notes.** Defaults to `/`. Not a user-facing option; it exists so the tests
can install into a temporary directory. Introduce it before anything computes a
path, or every later step has to be revisited.
**Done when.** `/etc/spm/sources.json`, `/var/lib/spm/…` and `/var/cache/spm/`
all come from it, and a test with a temporary prefix sees them relocate.

### Step 9 — Atomic writes

**Goal.** `write_atomic(path, bytes)`.
**Files.** `src/store/atomic.rs`.
**Notes.** Temporary file in the *destination directory* — a rename across
filesystems is not atomic, and `/tmp` may be a different one — then `fsync`,
then `rename`. This is what makes an interrupted `update` leave the old index
rather than half a file.
**Done when.** A test writes over an existing file and asserts the old content
is intact when the write fails partway.

### Step 10 — The lock

**Goal.** One writer at a time.
**Files.** `src/store/lock.rs`.
**Notes.** `flock(LOCK_EX)` on `/var/lib/spm/lock` via `libc`. Read-only
commands do not take it. The kernel releases it on death, so there is no stale
lock to reason about.
**Done when.** A test takes the lock in a child process and asserts the parent
blocks and then reports what holds it.

### Step 11 — `sources.json`

**Goal.** Read and write the configuration.
**Files.** `src/store/config.rs`.
**Notes.** Absent file means no sources — that is a fresh device, not an error.
Enforce on write: names unique, at most one default.
**Done when.** Round-trip tests, a missing-file test that yields an empty list,
and a test that rejects two sources with one name.

### Step 12 — The installed database

**Goal.** Read, write and enumerate `installed/<name>.json`.
**Files.** `src/store/db.rs`.
**Notes.** Also the `.partial` journal: `begin(record)` writes the `.partial`,
`commit()` renames it, `recover()` finds any left over, removes the files they
list and deletes them. `recover()` runs before any other work in every mutating
command.
**Done when.** A test writes a `.partial`, calls `recover()`, and asserts the
files listed in it are gone and the record with them.

---

## M3 — `create`

### Step 13 — Packing

**Goal.** A staged tree becomes `data.tar.gz`, hashed as it is written.
**Files.** `src/ops/create.rs`.
**Notes.** Write the payload to a temporary file through the hasher, because
the digest is not known until it is finished and it must go into the metadata
that is packed. Store paths relative, no leading `./`, sorted, with a fixed
mtime and `root:root` ownership so the same tree gives the same archive.
**Done when.** Packing a fixture tree twice byte-for-byte matches.

### Step 14 — The refusals

**Goal.** The four rules from ARCHITECTURE.md.
**Files.** `src/ops/create.rs`.
**Notes.** Everything under `usr/`; a licence under
`usr/share/licenses/<name>/`; no libc and no dynamic loader; `metadata.json`
names the package.
**Done when.** A test tree for each refusal fails with its own message, and the
good tree passes.

### Step 15 — The three outputs

**Goal.** `create` writes the package, the metadata and `SHA256SUMS`.
**Files.** `src/ops/create.rs`, `src/cli.rs`.
**Notes.** The metadata packed *inside* the archive is the author's with the
payload digest filled in — not the file they wrote. `SHA256SUMS` covers the
outer archive and is what the index's package checksum comes from.
**Done when.** `spm create --root … --metadata …` produces all three,
`sha256sum -c SHA256SUMS` passes, and the metadata inside the archive has a
digest that matches its own `data.tar.gz`.

### Step 16 — Fixture packages

**Goal.** Test fixtures built by the tool itself.
**Files.** `tests/fixtures/`, `tests/support/mod.rs`.
**Notes.** A helper that builds small packages on demand — one plain, one with
a dependency, one that conflicts with another over a file. Every later test
uses these, and they are produced by `create` rather than checked in, so the
format has exactly one implementation.
**Done when.** The helper builds all three and a test installs nothing yet but
asserts they exist and verify.

---

## M4 — Network

### Step 17 — `trait Transport`

**Goal.** The seam the tests replace.
**Files.** `src/net/transport.rs`.
**Notes.** `get(url) -> Result<impl Read>` and nothing more. Every layer above
takes a `&dyn Transport`, so no test ever touches the network.
**Done when.** A fake transport serving a temporary directory exists in
`tests/support/`, and a test reads a fixture index through it.

### Step 18 — HTTPS

**Goal.** The real transport.
**Files.** `src/net/https.rs`.
**Notes.** `ureq` + `rustls` + `webpki-roots` compiled in — the card has no
trust store to read. Refuse a non-`https` URL. Retry three times on connection
failures and 5xx with a growing delay, never on 4xx. Send `GITHUB_TOKEN` when
it is set.
**Done when.** A `http://` URL is refused without a request being made, and a
test against a local TLS server with a known-bad certificate fails closed.

### Step 19 — The clock error

**Goal.** A certificate that is not yet valid says why.
**Files.** `src/net/https.rs`, `src/error.rs`.
**Notes.** A device with an unset clock sits in 1970, so every certificate is
"not valid before" some later date and TLS fails. The message names the clock
and points at `sepia-time`, because "certificate error" sends the user looking
in the wrong place entirely. Read `/etc/sepia-build-date` to say how far behind
the clock is.
**Done when.** A test with the validator's clock set to 1970 produces the clock
message rather than the generic TLS one.

### Step 20 — Downloads

**Goal.** Stream to disk, hashing as it goes.
**Files.** `src/net/download.rs`.
**Notes.** Never buffer a package in memory: 216 MiB on a 512 MiB board. The
hash comes free from the same pass. A partial download is discarded, not
resumed.
**Done when.** A test downloads a 50 MiB fixture through the fake transport,
asserts the digest, and asserts peak memory does not scale with the file (by
construction — the reader is bounded).

---

## M5 — Sources

### Step 21 — `list-sources` and `source-info`

**Goal.** The two read-only source commands.
**Files.** `src/ops/query.rs`, `src/ui.rs`, `src/cli.rs`.
**Notes.** Build them first: they need no network, and they are what makes
every later step inspectable by hand. A source whose index was never fetched
must read as exactly that, not as one offering no packages. No sources at all
is a fresh device, so say so and name `add-source`.
**Done when.** Tests for: no sources, one never-updated source, two sources
with one default.

### Step 22 — `add-source`

**Goal.** Add a source, learning its name from its index.
**Files.** `src/ops/source.rs`.
**Notes.** Fetch and parse before writing anything. The name comes from the
index unless `--name` overrides it. A URL already configured updates its entry
rather than duplicating it — that is also how an existing source is made
default. The first source added becomes the default whichever way.
**Done when.** Tests for: first source becomes default; re-adding a URL updates
in place; `--default` moves the flag; a name collision is refused and names the
holder; a non-`https` URL is refused; a failed fetch leaves the config
untouched.

### Step 23 — `remove-source`

**Goal.** Remove a source without touching what it installed.
**Files.** `src/ops/source.rs`.
**Notes.** Delete the entry and the local index. Installed packages stay and
keep their recorded source. Removing the default promotes the last remaining
source, or reports that there is now none.
**Done when.** Tests for: installed packages survive; the count of affected
packages is reported; the default is promoted when one source remains and not
when two do.

---

## M6 — `update` and the read-only queries

### Step 24 — `update`

**Goal.** Fetch indexes and put them in place atomically.
**Files.** `src/ops/update.rs`.
**Notes.** Parse fully, then rename. Collect failures rather than stopping at
the first; report each with its source name; exit non-zero if any failed.
`--all` and `--source` are mutually exclusive, and `--all` is what happens when
neither is given.
**Done when.** Tests for: a broken index leaves the previous one intact; one
failing source of three still updates the other two and exits non-zero; both
options together is a usage error.

### Step 25 — Name resolution

**Goal.** A typed name becomes a package in a source.
**Files.** `src/ops/resolve.rs`.
**Notes.** Unqualified and unique: that one. Unqualified and offered by
several: refuse, listing them qualified. Qualified: that source or an error.
Version selection: the one asked for, else the highest whose `target` matches
the device.
**Done when.** Tests for all four outcomes, plus a package present only for
another target being reported as not available for this one.

### Step 26 — `search`, `info`, `list`

**Goal.** The three read-only package queries.
**Files.** `src/ops/query.rs`.
**Notes.** `search` matches anywhere in a name, case-insensitively, and exits
non-zero on no matches so a script can tell the two apart. `info` shows
metadata plus what only the client knows — source, installed state, the other
versions. `list` is one line per package with an installed marker.
**Done when.** Tests for each, including `search` finding a package by a
substring of its name and `info --version` selecting an older one.

---

## M7 — `install`

### Step 27 — Dependency resolution

**Goal.** A package name becomes the full set to install.
**Files.** `src/ops/resolve.rs`.
**Notes.** Breadth-first over the transitive dependencies. A version in
`dependencies` means *that version or newer*; take the lowest that satisfies it
and is not older than what is installed. Skip a dependency already satisfied.
Detect cycles with the visited set and report rather than loop.
**Done when.** Tests for: a chain three deep; a diamond resolved once; a cycle
reported; an unsatisfiable dependency named with what wanted it.

### Step 28 — The plan and `--dry-run`

**Goal.** Say what would happen before anything happens.
**Files.** `src/ops/install.rs`, `src/ui.rs`.
**Notes.** New packages, upgrades, and the total download. `--dry-run` stops
here and touches nothing — assert *that* in the test, not just the output.
**Done when.** A dry run over a fixture source leaves the prefix
byte-for-byte unchanged.

### Step 29 — Verification

**Goal.** Both digests, at the right moments.
**Files.** `src/ops/install.rs`.
**Notes.** The archive against the index's `sha256` **before it is opened at
all**; then `data.tar.gz` against the `sha256` inside the archive's own
`metadata.json`. Two checks, two failure messages — a test that cannot tell
them apart is not testing this.
**Done when.** A fixture with a corrupted outer archive fails before it is
opened, and one with a swapped payload fails at the second check. Both exit 6.

### Step 30 — Unpacking

**Goal.** Extraction, with every rule from DESIGN.md enforced.
**Files.** `src/unpack.rs`.
**Notes.** The most dangerous code in the program, so it is one function with
its own test module. Relative paths that stay inside the root; `usr/` only;
regular files, directories and symlinks only; symlink targets checked as paths;
permissions from the archive but **ownership always `root:root`** — restoring
recorded ownership is what broke helix's CI, where GNU tar as root recreated
`runner:docker` and git then refused the tree; never follow a symlink at the
destination.
**Done when.** A hand-built malicious tar per rule — `../../etc/passwd`, an
absolute path, `etc/` outside `usr/`, a device node, a hard link, a symlink to
`/etc` followed by a write through it — each rejected with its own error, and
a test asserting a file unpacked from an archive recording uid 1001 is owned by
root.

### Step 31 — Conflicts

**Goal.** Refuse rather than overwrite.
**Files.** `src/ops/install.rs`.
**Notes.** Build the file list by reading the payload without writing anything,
then check every path: claimed by another record is a conflict naming both
packages; present on disk and claimed by nothing is a conflict too, because
adopting it would mean `remove` later deleting something `spm` never installed.
**Done when.** Two fixture packages that share a file fail on the second, and
installing over a file the image put there fails. Both exit 7, before any file
is written.

### Step 32 — Commit and recovery

**Goal.** An install that cannot leave a half-installed device.
**Files.** `src/ops/install.rs`, `src/store/db.rs`.
**Notes.** `.partial` with the full file list, then extract, then rename. A
`.partial` found at startup is rolled back before anything else runs.
**Done when.** A test that kills the process between the journal and the rename
leaves a `.partial`; the next command removes those files and the record; the
prefix matches its pre-install state.

### Step 33 — Disk space

**Goal.** Refuse before filling the root filesystem.
**Files.** `src/ops/install.rs`.
**Notes.** A package is on the card twice during an install — the archive and
the unpacked tree — so require roughly twice its size and say so when it will
not fit. On a 2 GiB card with Rust and Helix already on it, this is not
hypothetical.
**Done when.** A test with a constrained prefix refuses with a message naming
what is needed and what is free.

---

## M8 — `remove`

### Step 34 — Removal

**Goal.** Take back exactly what was installed.
**Files.** `src/ops/remove.rs`.
**Notes.** Refuse if another record depends on this package, listing the
dependents. Delete the record's files in reverse order, skipping any path
another record also claims. Remove directories that have emptied.
**Done when.** Tests for: a blocked removal names its dependents; a shared file
survives; directories are cleaned; nothing outside the record is touched.

### Step 35 — Autoremove

**Goal.** Dependencies that are no longer needed go too.
**Files.** `src/ops/remove.rs`.
**Notes.** Repeatedly drop any `dependency` record that nothing remaining
depends on, until a pass changes nothing. A package installed explicitly is
never taken automatically, however unreferenced it is.
**Done when.** Removing the head of a three-deep chain removes all three; an
explicitly installed middle package stops the cascade.

---

## M9 — `upgrade`

### Step 36 — `upgrade`

**Goal.** Move installed packages onto newer versions.
**Files.** `src/ops/upgrade.rs`.
**Notes.** Read the local indexes only — it finds nothing the last `update` did
not. Compute the whole set first, including new dependencies. A package whose
new version cannot have its dependencies satisfied is left alone and reported,
the rest still upgrade, and the exit is non-zero. Then reuse `install` from the
planning step onward rather than writing a second install path.
**Done when.** Tests for: one of three unsatisfiable, the other two upgraded,
exit non-zero; `upgrade <name>` touching only that package; `--dry-run`
changing nothing.

---

## M10 — Shipping

### Step 37 — Cross-build

**Goal.** An `aarch64-unknown-linux-musl` binary.
**Files.** `.cargo/config.toml`, `README.md`.
**Notes.** Static by default on that target, which is the point — it must run
on a card built `WITH_LLVM=0`, where there is no `libgcc_s`. `ring`, under
`rustls`, needs a C compiler for the target: the cross toolchain the sibling
repositories already use. A build script needs a *host* compiler too — the trap
that broke `Sepia-OS/grit`'s first CI run.
**Done when.** `readelf -l` shows no interpreter and `readelf -d` no
`NEEDED` — the same assertion `grit-check` makes.

### Step 38 — Run the tests on the target

**Goal.** The cross-built binary is executed, not just built.
**Files.** `.github/workflows/ci.yml`.
**Notes.** CI runners are x86_64, so run the target test binaries under
`qemu-user-static` in the same container — the suites here are small, and the
compile dominates. On an Apple Silicon workstation the same binaries run at
native speed in a `linux/arm64` container. This is how helix's runtime
behaviour was verified.
**Done when.** The full suite passes on the host and again on `aarch64`, in CI.

### Step 39 — CI

**Goal.** Every commit on every branch is checked.
**Files.** `.github/workflows/ci.yml`.
**Notes.** `cargo fmt --check`, `cargo clippy -D warnings`, host tests,
cross-build, emulated tests. Install the build tools before `actions/checkout`:
without `git` in the container, checkout silently falls back to a tarball.
**Done when.** A green run on a branch, and a deliberately broken commit fails
the expected step.
**Partly done**, ahead of its place in the order: the host half of this exists
now — `.github/workflows/ci.yml` checks the licence header on every source
file, then runs `fmt --check`, `clippy --all-targets -- -D warnings`, `build`
and `test`, all with `--locked` so a stale `Cargo.lock` fails the run. What is
still missing is the cross-build and the emulated run, which are Steps 37 and
38 and want the target toolchain first.

### Step 40 — Release

**Goal.** A published binary with its checksum.
**Files.** `.github/workflows/release.yml`.
**Notes.** Manual dispatch with a version; branch `main` to `rel-<version>`;
replace `0.1.0-replace-me` in `Cargo.toml`; build; publish the binary and a
`SHA256SUMS`. Reuse an existing `rel-<version>` branch rather than failing.
**Done when.** A dry run on a scratch tag produces the assets, and the version
in `--version` matches the tag.

### Step 41 — `spm` packages itself

**Goal.** The first real package.
**Files.** `metadata.json`, `.github/workflows/release.yml`.
**Notes.** The release runs `spm create` on its own staged tree, so the package
manager ships as a package like any other. Tag the repository `package` and the
index picks it up on its next scan — which is also the first end-to-end test of
the notification path.
**Done when.** The release publishes a package, its `metadata.json` and a
`SHA256SUMS`; a device with the source configured can `spm install spm`.

---

## Deferred

Named here so that leaving them out stays a decision rather than an oversight.
None of them blocks a first release.

- **Signing an index.** The digest chain protects a download from the network,
  not a device from a source that has been taken over. Signing, with a key
  pinned per source in `sources.json`, is the next layer and wants designing
  before it is built.
- **Downgrades.** `install --version` naming an older version than the one
  installed is undecided: downgrade, error, or no-op.
- **A `verify` command** to re-check installed files against their records. The
  data is already there; the command is not specified.
- **Addressing a source by name** in `add-source`, `remove-source` and
  `source-info`, which take a URL while every other command takes a name.
- **Configuration in packages.** `create` refuses anything outside `usr/`, so a
  package cannot ship defaults in `/etc`.
