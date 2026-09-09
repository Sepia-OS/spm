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

**A module is a branch.** Starting one begins with
`git switch -c feat/<module>-<title>` — `feat/M1-the-model` for the milestone
below — and every step in it is committed there.
The branch merges to `main` when the module is finished, which is when it earns
its ✅. See [the development guidelines](DEVELOPMENT-GUIDELINES.md).

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

## M1 — The model ✅

### Step 4 — `Version` and its ordering ✅

**Goal.** Upstream versions compare correctly.
**Files.** `src/model/version.rs`.
**Notes.** Not semver. Numeric components compare numerically, so `25.07.1`
equals `25.7.1`; a numeric component sorts above a non-numeric one, so `1.0` is
above `1.0-rc1`; a missing component is zero, so `1.2` equals `1.2.0`.
**Done when.** Tests cover all four rules, `25.07.1 == 25.7.1`, `1.10 > 1.9`
(the comparison a string sort gets wrong), and sorting a shuffled list of real
SepiaOS versions — `1.2.6`, `23.1.0`, `25.07.1`, `4.4.1` — gives the expected
order.
**Done.** 12 tests, all four rules and both named cases. Three things the step
did not say but the implementation had to settle:

- **A `Version` keeps the text it was written as and the components it compares
  by.** `info` should show a package the way its own index spells it, so
  `25.07.1` prints as `25.07.1` while comparing equal to `25.7.1`.
- **`Eq` and `Hash` follow the ordering, not the text.** Two versions that
  compare equal must hash alike or a `HashMap` keyed on one loses entries;
  trailing zero components are dropped at parse time so that `1.2` and `1.2.0`
  are the same key.
- **No `FromStr`.** The only text that is not a version is empty text, and the
  right error for it depends on where it came from — a file wants `Parse` with
  a path, an argument wants `Usage` — so `parse` returns an `Option` and the
  caller supplies the context.

A component too long for a `u64` becomes text and therefore sorts below every
number. That falls out of rule two rather than being chosen, and the test for
it failed twice before the expectation was right — 20 ones fit in a `u64`, 20
nines do not.

### Step 5 — Names ✅

**Goal.** `PackageName`, `SourceName`, and the `<source>/<package>` form.
**Files.** `src/model/name.rs`.
**Notes.** One type that parses both the qualified and unqualified spellings,
so no command re-implements the split. Reject empty parts, whitespace, and a
name with more than one `/`.
**Done when.** Round-trip tests for both forms, and rejection tests for the
malformed ones.
**Done.** `PackageName`, `SourceName` and `PackageRef`, with 13 tests. The
rules came out stricter than the step describes, for a reason the step does not
mention: **a name becomes a filename** — `installed/<name>.json`,
`index/<source>.json` — so a package called `../../etc/passwd` would be a
package that writes wherever it likes. Refusing the traversal at the type
boundary is cheaper than guarding every place a path is built, and it is the
same argument the extraction rules make in Step 30.

Two rules follow from that: a name starts with a letter or a digit, which is
what rules out `.`, `..` and anything that reads as a command-line option; and
a name is lower case, because the record is a file and on a case-insensitive
filesystem `Helix` and `helix` would be one file. `InvalidName` says which rule
was broken and the caller decides what that amounts to — `Parse` with a path
for a file, `Usage` for an argument — the same split `Version::parse` makes.

### Step 6 — `Metadata` ✅

**Goal.** A package's `metadata.json` as a type.
**Files.** `src/model/metadata.rs`.
**Notes.** `name`, `version`, `target`, `description`, `dependencies`,
`sha256`. Deserialising must reject an unknown field rather than ignore it: a
misspelled key in a hand-written metadata file is a mistake to report, not to
drop silently.
**Done when.** The example from ARCHITECTURE.md round-trips, an empty `sha256`
is accepted (that is what an author writes), and a missing `name` is an error.
**Done.** `Metadata`, `Dependency` and `Sha256`, with 9 tests whose fixture is
the ARCHITECTURE.md example copied verbatim — so the parser and the
documentation cannot drift apart without a test saying so. `serde` and
`serde_json` are in, and with them serde implementations for `Version` and the
name types, which validate on the way in: a name that reached a file by some
route other than this crate is refused when it is read.

Three things beyond the field list:

- **`Sha256` is a type.** Two digests are in play and the design is blunt about
  what confusing them costs. It also refuses upper case, since that would be a
  second spelling of one digest and two equal digests could then fail to
  compare equal.
- **`sha256` is `Option<Sha256>`, written as `""` when absent.** That is what an
  author writes and what `create` replaces, so the file format keeps one
  spelling for "not filled in yet".
- **`Target` was added** in `name.rs`, validated like a name. It lands in the
  filename `create` writes, so a target that walks up a directory would put a
  package outside the output directory.

### Step 7 — `Index` and `Record` ✅

**Goal.** The other two formats as types.
**Files.** `src/model/index.rs`, `src/model/installed.rs`.
**Notes.** The index carries both digests under distinct names — `sha256` for
the package and `payload_sha256` for `data.tar.gz`. Do not shorten either;
confusing them is a verification that passes while checking nothing.
**Done when.** Both round-trip, and a test asserts an `Index` selects the
highest version for a given target.
**Done.** 15 tests, both fixtures taken from the examples in DESIGN.md with only
the elisions filled in. `Index::newest` picks the highest version *for a
target*, tested against an index whose versions are deliberately out of order
and split across two targets. `sha256` and `payload_sha256` are separate fields
with separate tests, including one that asserts they hold different values —
the failure this format has to make impossible is a reader that swaps them and
verifies nothing while appearing to.

Two choices worth recording:

- **`url` is text, not a checked type.** A non-`https` URL is refused by the
  transport, where refusing costs one package; refusing it here would throw a
  whole index away over one bad entry.
- **`files` is `Vec<PathBuf>`, and that has a consequence.** A record is JSON,
  so a path that is not valid UTF-8 cannot be written into one. A package
  carrying such a path therefore cannot be recorded, which means **Step 30 has
  to refuse one at extraction** rather than discovering it at the commit.

---

## M2 — Local state ✅

### Step 8 — Root prefix ✅

**Goal.** Every path is built from one configurable prefix.
**Files.** `src/store/mod.rs`.
**Notes.** Defaults to `/`. Not a user-facing option; it exists so the tests
can install into a temporary directory. Introduce it before anything computes a
path, or every later step has to be revisited.
**Done when.** `/etc/spm/sources.json`, `/var/lib/spm/…` and `/var/cache/spm/`
all come from it, and a test with a temporary prefix sees them relocate.
**Done.** `Store`, with 5 tests: the real paths are what the documents say, and
every one of them moves when the prefix does. No I/O — these are the names of
files, not the files, so the module stays pure and the tests need no
filesystem.

The prefix is deliberately **not** a command-line option. A package manager
with a `--root` flag is one that can be pointed at the wrong system, and
nothing in the architecture asks for it; it exists for the tests.

One trap is documented in the code because it would be silent: `Path::join`
throws the base away if its argument is absolute, so a single leading `/` in
one of these fragments would write to the real system while a test believed it
was sandboxed. Every fragment is relative, and the test that everything starts
with the prefix is what would catch it.

### Step 9 — Atomic writes ✅

**Goal.** `write_atomic(path, bytes)`.
**Files.** `src/store/atomic.rs`.
**Notes.** Temporary file in the *destination directory* — a rename across
filesystems is not atomic, and `/tmp` may be a different one — then `fsync`,
then `rename`. This is what makes an interrupted `update` leave the old index
rather than half a file.
**Done when.** A test writes over an existing file and asserts the old content
is intact when the write fails partway.
**Done.** `store::atomic::write` and `write_with`, with 7 tests. The failure
case is the one that matters and it is tested directly: a write that dies after
some bytes leaves the previous file byte-for-byte and no temporary behind.

Three details the step named or implied, each with a test:

- **The temporary file is in the destination directory.** A test reads that
  directory *from inside the write* and asserts the temporary is there, so
  moving it to `/tmp` — where a rename becomes a copy and a delete — fails a
  test rather than quietly stopping being atomic.
- **`sync_all` before the rename**, or a power cut can leave a renamed file
  full of nothing, and a second sync of the directory afterwards so the rename
  itself survives one. A filesystem that will not sync a directory is not a
  reason to fail the write.
- **`write_with` takes a closure** rather than only bytes, so an index can be
  serialised straight into the file and a download streamed through it. Nothing
  proportional to a package is ever held in memory.

`tempfile` is the dependency, as the design's table said it would be.

### Step 10 — The lock ✅

**Goal.** One writer at a time.
**Files.** `src/store/lock.rs`.
**Notes.** `flock(LOCK_EX)` on `/var/lib/spm/lock` via `libc`. Read-only
commands do not take it. The kernel releases it on death, so there is no stale
lock to reason about.
**Done when.** A test takes the lock in a child process and asserts the parent
blocks and then reports what holds it.
**Done.** `Lock::try_acquire`, `Lock::wait` and `Lock::holder`, with 5 tests.

**No `libc`, and no `unsafe`.** `std::fs::File::lock` has been stable since Rust
1.89 and does exactly what was wanted, so the crate now contains no `unsafe` at
all. `DESIGN.md` and the development guidelines have been corrected; the
guidelines' `unsafe` section had been written around this one block.

**Threads rather than a child process.** A lock belongs to the open file and not
to the process that opened it, so two handles in one process contend exactly as
two processes do — verified before relying on it. That is the whole content of
the child-process test, without a helper binary to build and run.

The holder writes its process id into the file *after* taking the lock, so a
waiter can say what it is waiting for. Reading it takes nothing, so it can be
stale by the time it is printed; it is a sentence for a person, not a decision.
`store` prints nothing, so the sequence a command uses is try, read the holder,
report through `ui`, then wait.

### Step 11 — `sources.json` ✅

**Goal.** Read and write the configuration.
**Files.** `src/store/config.rs`.
**Notes.** Absent file means no sources — that is a fresh device, not an error.
Enforce on write: names unique, at most one default.
**Done when.** Round-trip tests, a missing-file test that yields an empty list,
and a test that rejects two sources with one name.
**Done.** `Source` and `Sources`, with 12 tests, round-tripping through a real
file under a temporary prefix rather than only through a string.

**The two invariants are enforced in two different ways, on purpose.** Reading
checks them, because a hand-edited file can break either and the person who
broke it needs to be told which — the message names both sources and both URLs.
Writing cannot break them, because the only ways to change the list maintain
them: `insert` replaces any entry with the same name *or* the same URL, and
`set_default` takes the flag from whoever held it. That is why `save` does not
validate: a check there would have no good error to give, since nothing is
being read, and it would be checking something already true.

Replacing by URL is also what `add-source` needs — re-running it on a
configured URL updates that entry instead of adding a second one — and refusing
a name another URL already holds stays a decision for `add-source`, which can
say so properly.

Written pretty-printed with a trailing newline: it is the one file somebody may
open in an editor on the device, and a test says so rather than trusting it.

### Step 12 — The installed database ✅

**Goal.** Read, write and enumerate `installed/<name>.json`.
**Files.** `src/store/db.rs`.
**Notes.** Also the `.partial` journal: `begin(record)` writes the `.partial`,
`commit()` renames it, `recover()` finds any left over, removes the files they
list and deletes them. `recover()` runs before any other work in every mutating
command.
**Done when.** A test writes a `.partial`, calls `recover()`, and asserts the
files listed in it are gone and the record with them.
**Done.** `Database`, with `get`, `all`, `is_installed`, `begin`, `commit`,
`forget` and `recover`, and 10 tests.

**A journal can only mean "undo", never "finish".** It is written before any
file is, and nothing in it records how far the install got — so the only safe
reading of one left behind is to take back what it claims. That asymmetry is
what makes recovery possible at all, and it decides the next two points.

- **A file a journal names and that does not exist is the ordinary case**, not a
  failure: most of them will not exist, because the journal is written first.
- **A file that exists and cannot be deleted is a failure**, and the journal is
  left in place so the next command tries again rather than leaving a
  half-installed package nothing remembers.

A `.partial` is not an installed package: `all` and `is_installed` skip it, with
a test, because an unfinished install must not look like a finished one.

A record claiming a path outside the device's root is refused rather than
obeyed — an absolute path would make `join` throw the root away, the same trap
the store documents. Nothing here can write such a record; refusing beats
deleting whatever it points at. **Step 30 needs the same rule** and should share
this one rather than write a second.

Directories are left where a rollback empties them. Pruning them is Step 34's,
and rollback will use it once it exists.

---

## M3 — `create` ✅

### Step 13 — Packing ✅

**Goal.** A staged tree becomes `data.tar.gz`, hashed as it is written.
**Files.** `src/ops/create.rs`.
**Notes.** Write the payload to a temporary file through the hasher, because
the digest is not known until it is finished and it must go into the metadata
that is packed. Store paths relative, no leading `./`, sorted, with a fixed
mtime and `root:root` ownership so the same tree gives the same archive.
**Done when.** Packing a fixture tree twice byte-for-byte matches.
**Done.** `pack_payload`, with 11 tests. Packing the same tree twice gives the
same bytes, and — the stronger claim, since it is what actually differs between
two builds — **touching a file's mtime and repacking gives the same bytes too**.

The digest is taken of the compressed file, in the same pass that writes it:
`tar -> gzip -> hash -> disk`. So the size and the digest both come out of one
walk of the tree, and nothing is read back to compute them.

Three things the step's list implied and the tests pin down:

- **A symlink is packed as a link.** `grit` ships `git -> grit`, and following
  it here would put a second copy of the binary on a card under another name.
  The walk uses `symlink_metadata` for exactly this.
- **Modes are normalised to 755 or 644.** A staged tree carries the umask of the
  machine that built it, which is not a property of the package; the executable
  bit is the one distinction that matters on the device.
- **Directories are entries too**, so an empty one survives and the order is the
  sorted order rather than whatever order a directory happened to be read in.

`tar`, `flate2` (with the Rust backend, no `zlib` to cross-compile) and `sha2`
are the new dependencies, all three named in the design's table.

### Step 14 — The refusals ✅

**Goal.** The four rules from ARCHITECTURE.md.
**Files.** `src/ops/create.rs`.
**Notes.** Everything under `usr/`; a licence under
`usr/share/licenses/<name>/`; no libc and no dynamic loader; `metadata.json`
names the package.
**Done when.** A test tree for each refusal fails with its own message, and the
good tree passes.
**Done.** `check_tree`, with 8 tests — one per refusal, plus the good tree, plus
the cases that would otherwise be assumed.

**Only three of the four rules are checked, and the fourth is the interesting
one.** "The metadata has to name a package" is not checked because it *cannot
be broken*: a `Metadata` without a name, a version or a target does not parse,
so no such value exists to hand to this function. The test for it asserts the
parse fails, which is where the rule actually lives.

A new error variant, `NotPackageable { path, reason }`, mapped to exit 2. It
carries the path rather than formatting it into a message, as the guidelines
ask, so the offending file is a value and not a sentence.

Two things the rules leave open, decided here and tested:

- **The licence has to be under the package's own name.** A tree carrying
  `usr/share/licenses/helix/LICENSE` is not a licensed `grit` package, and a
  test says so — that is the mistake a copied build script makes.
- **Any non-empty file counts**, so `COPYING` or `NOTICE` will do. An empty
  `LICENSE` does not, which is the other way a build script gets this wrong.

The libc rule uses the same names the `rootfs` build already refuses in every
sibling package — `libc.so*`, `ld-musl-*`, `ld-linux*` — because it is the same
rule for the same reason, and a second one would eventually disagree.

### Step 15 — The three outputs ✅

**Goal.** `create` writes the package, the metadata and `SHA256SUMS`.
**Files.** `src/ops/create.rs`, `src/cli.rs`.
**Notes.** The metadata packed *inside* the archive is the author's with the
payload digest filled in — not the file they wrote. `SHA256SUMS` covers the
outer archive and is what the index's package checksum comes from.

**A version is not a validated filename.** `PackageName` and `Target` are
checked where they are made, so they cannot escape a directory; `Version`
deliberately accepts whatever upstream chose, including text with a `/` in it.
The output name here is `<name>-<version>-<target>.tar.gz`, so this step is
where that has to be refused — found while building the paths in Step 8, where
it does not yet bite because nothing there is derived from a version.
**Done when.** `spm create --root … --metadata …` produces all three,
`sha256sum -c SHA256SUMS` passes, and the metadata inside the archive has a
digest that matches its own `data.tar.gz`.
**Done.** `create`, the CLI around it, and 10 tests. All three conditions were
checked by running the real thing rather than only in tests: `spm create`
against a staged tree wrote the three files, `shasum -a 256 -c SHA256SUMS` said
`OK`, and the digest in the packed metadata matched the `data.tar.gz` beside it
byte for byte. The same checks are now tests, so they are not a thing I did
once.

- **The author's own metadata file is never written to.** The digest goes into
  the copy that is packed and the copy published beside it; the file in the
  package repository is read and left alone, which a test asserts.
- **The version hazard recorded in Step 8 is closed here.** A name and a target
  cannot escape a directory because they are validated where they are made; a
  version is whatever upstream chose, and this is the one place one becomes a
  filename. `../../evil` as a version is refused, naming the metadata file it
  came from.
- **The whole package is deterministic**, not just the payload: building the
  same tree twice gives the same bytes through both layers.
- **A tree that breaks a rule is refused before anything is written**, so a
  failed `create` leaves no half-published release behind.

`clap` is the last dependency the design's table named. A malformed command
line is `clap`'s to report, and it exits 2 — the code the user guide documents
for wrong usage.

### Step 16 — Fixture packages ✅

**Goal.** Test fixtures built by the tool itself.
**Files.** `tests/fixtures/`, `tests/support/mod.rs`.
**Notes.** A helper that builds small packages on demand — one plain, one with
a dependency, one that conflicts with another over a file. Every later test
uses these, and they are produced by `create` rather than checked in, so the
format has exactly one implementation.
**Done when.** The helper builds all three and a test installs nothing yet but
asserts they exist and verify.
**Done.** `tests/support/mod.rs` and `tests/fixtures.rs`, 7 tests. The three
shapes are there — `plain` stands alone, `dependent` needs it, and `rival`
ships the same file `plain` does, which is the conflict Step 31 has to refuse.
Each one is built by `create`, so a change to the package format cannot leave a
fixture describing the old one.

The builder takes a version, a target, arbitrary files, dependencies and a
missing licence, because later steps need more than the three: Step 25 needs a
package for another target, Step 27 needs a chain and a diamond, and Step 36
needs two versions of one package.

`Built::verify` is the whole chain in one call — the package matches the digest
beside it, the digest file is what `sha256sum -c` reads, the packed metadata
describes the payload beside it, and the metadata published beside the package
is the same bytes as the one inside.

**There is no `tests/fixtures/` directory**, as this step's file list expected:
nothing is checked in, because the fixtures are built. The builder lives in
`tests/support/` and the test that they hold together is `tests/fixtures.rs`.

One thing this step found: **an integration test is its own crate**, so the
`#[cfg(test)]` allows that let unit tests use `unwrap` do not reach `tests/`.
Each file there carries the allow at crate level instead, and the development
guidelines now say so rather than leaving the next test crate to rediscover it.

---

## M4 — Network ✅

### Step 17 — `trait Transport` ✅

**Goal.** The seam the tests replace.
**Files.** `src/net/transport.rs`.
**Notes.** `get(url) -> Result<impl Read>` and nothing more. Every layer above
takes a `&dyn Transport`, so no test ever touches the network.
**Done when.** A fake transport serving a temporary directory exists in
`tests/support/`, and a test reads a fixture index through it.
**Done.** `Transport`, the fake in `tests/support/net.rs`, and 6 tests. The
done-when test builds two fixture packages, serves them and an index describing
them, reads the index through the transport and then asks it for the newest
version — which is the sequence `update` and `install` will follow.

**The trait returns `Box<dyn Read>` rather than `impl Read`.** A
return-position `impl Trait` makes a trait not object-safe, and `&dyn
Transport` is the entire point of having one: it is what lets a test drive the
real code path with fixtures behind it.

The fake does three things beyond serving files, each because a later step needs
it: it can be told to **break a URL** (Step 24 wants one source of three
unreachable), it **records what was asked for** in order, and it hands back
bytes rather than text so a package can go through it whole.

`index_of` builds a fixture index out of the real `model::index` types rather
than out of text, so a fixture cannot describe a format that no longer exists —
it would stop compiling instead. It groups versions under one package, because
that is what an index does and what `upgrade` will need to see.

### Step 18 — HTTPS ✅

**Goal.** The real transport.
**Files.** `src/net/https.rs`.
**Notes.** `ureq` + `rustls` + `webpki-roots` compiled in — the card has no
trust store to read. Refuse a non-`https` URL. Retry three times on connection
failures and 5xx with a growing delay, never on 4xx. Send `GITHUB_TOKEN` when
it is set.
**Done when.** A `http://` URL is refused without a request being made, and a
test against a local TLS server with a known-bad certificate fails closed.
**Done.** `Https`, with 9 tests. Both halves of the done-when are tested as
stated rather than approximated:

- **Refused before anything is opened**, proved by a listener that counts what
  reaches it. A test that only checked the error would pass just as well if the
  refusal happened *after* the connection — and a request already made has
  already told somebody what this device is looking for.
- **A self-signed certificate is rejected**, against a `rustls` server started
  by the test on the loopback address with a certificate generated and thrown
  away with it. This is the property a card depends on: the compiled-in roots
  are the only thing `spm` trusts, because there is no system trust store to
  fall back on. Nothing here resolves a name or leaves the machine.

Two decisions beyond the step's list:

- **`ureq`'s `gzip` feature is off.** Everything is verified against a digest of
  a file, and an automatic transfer decoding is one more difference between
  what a server sent and what a digest describes. `curl`, which every sibling
  repository uses, does not ask for one either.
- **`GITHUB_TOKEN` goes to GitHub and nowhere else**, checked on the whole host
  so that `github.com.example.test` is not GitHub. A source is a URL somebody
  typed; sending a token to it because it happens to be in the environment
  would hand the token to whoever runs that host. The check is a pure function
  so it can be tested — reading the environment is `unsafe` in this edition,
  and a function that read it could not be.

The retry policy is tested twice over: which failures are worth retrying, as a
unit test, and that a dying connection really is tried `ATTEMPTS` times, by
counting connections.

### Step 19 — The clock error ✅

**Goal.** A certificate that is not yet valid says why.
**Files.** `src/net/https.rs`, `src/error.rs`.
**Notes.** A device with an unset clock sits in 1970, so every certificate is
"not valid before" some later date and TLS fails. The message names the clock
and points at `sepia-time`, because "certificate error" sends the user looking
in the wrong place entirely. Read `/etc/sepia-build-date` to say how far behind
the clock is.
**Done when.** A test with the validator's clock set to 1970 produces the clock
message rather than the generic TLS one.
**Done.** Both directions are tested against the self-signed server from Step
18: a clock reading 1970 gets the clock message, and a clock that is right gets
the certificate error — because a genuinely bad certificate must not be blamed
on the time forever.

**Only a TLS failure can become a clock message.** A refused connection has
nothing to do with what the device thinks the time is, and a third test holds
that line.

Two things this step found rather than assumed:

- **`ureq` flattens a certificate failure into `Error::Io`.** Matching on
  `Error::Rustls` never fired, and the first version of this failed with
  `invalid peer certificate: UnknownIssuer` arriving as an I/O error. The
  classification now looks at the message for that case, deliberately: the
  alternative is depending on `rustls` directly and downcasting, which fails
  *silently* if the two ever resolve to different versions. A string check
  fails loudly, and this step's own test is what holds it up.
- **The clock is compared against `/etc/sepia-build-date`**, which `rootfs`
  writes and `sepia-time` already uses for the same purpose. Where there is no
  such file — a workstation, or an older card — the floor is 2025-01-01, since
  a clock earlier than that is not a clock anybody set. A build date that will
  not parse falls back too, rather than making every certificate failure a
  clock message.

`date_of` is Howard Hinnant's civil-from-days, written out rather than taken
from a crate: one date in one format against a dozen lines of arithmetic, and
the leap day is tested.

### Step 20 — Downloads ✅

**Goal.** Stream to disk, hashing as it goes.
**Files.** `src/net/download.rs`.
**Notes.** Never buffer a package in memory: 216 MiB on a 512 MiB board. The
hash comes free from the same pass. A partial download is discarded, not
resumed.
**Done when.** A test downloads a 50 MiB fixture through the fake transport,
asserts the digest, and asserts peak memory does not scale with the file (by
construction — the reader is bounded).
**Done.** `download::to_file` and `to_string`, with 8 tests.

**Boundedness is measured, not asserted by construction.** A transport in the
test records the largest buffer it is ever asked to fill, and the test fails if
that grows with the file: 50 MiB in, and nothing reads more than a megabyte at
a time. "By construction" is how it is true; a test is how it stays true.

The download goes through `store::atomic`, so the file appears whole or not at
all — a download interrupted by a power cut cannot be mistaken for a complete
one, and a failed retry does not destroy the copy that worked. Both are tested
with a transport whose body dies partway.

Two things beyond the step:

- **The fake transport now streams from disk** instead of reading a file into
  memory first. A fake that slurped would hide a caller that slurped.
- **`to_string` takes a limit.** An index has to be parsed in one go, so
  something has to read it whole — and the other end decides how much it sends.
  The limit is what makes that decision this side's.

---

## M5 — Sources ✅

### Step 21 — `list-sources` and `source-info` ✅

**Goal.** The two read-only source commands.
**Files.** `src/ops/query.rs`, `src/ui.rs`, `src/cli.rs`.
**Notes.** Build them first: they need no network, and they are what makes
every later step inspectable by hand. A source whose index was never fetched
must read as exactly that, not as one offering no packages. No sources at all
is a fresh device, so say so and name `add-source`.
**Done when.** Tests for: no sources, one never-updated source, two sources
with one default.
**Done.** `list_sources` and `source_info`, the two subcommands, and 8 tests
covering all three cases plus the counting of installed packages.

**A never-fetched index is an `Option`, not an empty index.** That is the
distinction the step names, and making it a type rather than a convention is
what stops it being lost: a caller has to say which it means, and both printers
do — `never` against a count, and "unknown until it has been updated".

Two things this step needed that the design had not placed:

- **`store/index.rs`**, to read and write the local copy of an index. `update`
  writes it and everything else reads it, so it is mechanism and belongs in
  `store`; the design's tree missed it, as it missed `ops/source.rs`, and both
  are now in it.
- **One date formatter, in `ui`.** Step 19 had written one inside `https.rs`
  for the clock message; a listing needs the same thing, and two would
  eventually disagree. `net` calling a formatter is not `net` printing.

**The listing shows a date rather than "2 hours ago"**, which is what the user
guide had promised. A relative time needs to know what the time is now, and on
a device that has just booted that is precisely what cannot be relied on — the
same fact Step 19 exists for. The guide has been corrected to what the command
prints.

One limitation worth naming: `ui` prints straight to stdout, so its output
cannot be asserted in a test. Everything here is tested through the `ops`
functions instead. Giving `ui` a writer would fix that, and is not this step.

### Step 22 — `add-source` ✅

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
**Done.** `add_source`, the subcommand, and 9 tests — every case on that list,
plus `--name`, plus that the index really is fetched so the source is usable
without an `update` first.

**A non-`https` URL is refused before the fetch**, and the test asserts the
transport was never asked. Refusing afterwards would already have told somebody
what this device was about to look for. The transport refuses one too; this one
exists to cost nothing and to say why in the words of the command.

**Re-adding a source does not quietly drop its default flag.** The rule is
`--default`, or the config is empty, or the entry already had it — so running
`add-source` twice is idempotent rather than a way to lose the default by
accident.

One thing the step's design did not anticipate: **`--name` means the name a
device files an index under is not always the name the index declares.**
`index::write` therefore takes the local name rather than reading it off the
index, and the index itself is stored exactly as fetched. What a source calls
itself is its business; what this device calls it is the device's.

The order of writes is deliberate: the index first, then the configuration. A
configured source whose index is missing is a source that needs an `update`,
which is recoverable and obvious. The other way round leaves an index file that
nothing refers to.

### Step 23 — `remove-source` ✅

**Goal.** Remove a source without touching what it installed.
**Files.** `src/ops/source.rs`.
**Notes.** Delete the entry and the local index. Installed packages stay and
keep their recorded source. Removing the default promotes the last remaining
source, or reports that there is now none.
**Done when.** Tests for: installed packages survive; the count of affected
packages is reported; the default is promoted when one source remains and not
when two do.
**Done.** `remove_source`, the subcommand, and 7 tests covering all three, plus
removing the only source, removing one nothing came from, and a URL that is not
configured.

**The rule the command exists to keep is that it removes nothing.** A test
installs a package from the source, removes the source, and then reads the
record back: still installed, still recording the source it came from. That is
why a record can name a source that is no longer configured, and why
`list-sources` had to cope with exactly that in Step 21.

**The default moves rather than disappearing.** With one source left it takes
the flag, for the same reason the first source added has it; with several left
nothing here can guess, so there is no default and the command says so and says
how to name one. Both are tested, because "promote when there is one" and "do
not guess when there are several" are two rules and a single test would only
cover one.

`losing_upgrades` is counted before anything changes and reported afterwards.
It costs no files, which is the point — what those packages lose is upgrades,
and somebody removing a source should be told that in words.

---

## M6 — `update` and the read-only queries ✅

### Step 24 — `update` ✅

**Goal.** Fetch indexes and put them in place atomically.
**Files.** `src/ops/update.rs`.
**Notes.** Parse fully, then rename. Collect failures rather than stopping at
the first; report each with its source name; exit non-zero if any failed.
`--all` and `--source` are mutually exclusive, and `--all` is what happens when
neither is given.
**Done when.** Tests for: a broken index leaves the previous one intact; one
failing source of three still updates the other two and exits non-zero; both
options together is a usage error.
**Done.** `update`, the subcommand, and 10 tests — all three cases plus the
counting of what is new, a named source fetching only itself, and a device with
no sources succeeding at doing nothing.

**Doing the work and deciding the outcome are separate.** `update` returns a
report; `Report::outcome` turns it into a failure. The caller prints everything
that happened *and then* fails, so a script sees both the two sources that
updated and the one that did not. A command that failed on the first problem
would hide the successes it had already had.

**The mutual exclusion is `clap`'s**, with `conflicts_with`, so a wrong command
line never reaches `ops` at all — and the test checks the definition rather
than the command, which is where the rule actually lives.

Two things this step needed around it:

- **The lock.** `update` writes, so `main` now takes the single-writer lock for
  it and for `add-source` and `remove-source`. The sequence the design asks for
  — try, say who holds it, then wait — lives in `main`, because `store` prints
  nothing and `ops` prints nothing.
- **Failures go to stderr**, the listing to stdout, so a script reading the
  listing does not have to sift failures out of it.

### Step 25 — Name resolution ✅

**Goal.** A typed name becomes a package in a source.
**Files.** `src/ops/resolve.rs`.
**Notes.** Unqualified and unique: that one. Unqualified and offered by
several: refuse, listing them qualified. Qualified: that source or an error.
Version selection: the one asked for, else the highest whose `target` matches
the device.
**Done when.** Tests for all four outcomes, plus a package present only for
another target being reported as not available for this one.
**Done.** `candidates`, `find` and `select`, with 12 tests: all four outcomes,
both target cases, and the version selection either way.

**Three failures that look alike are kept apart**, because each has a different
fix:

- *no such source* — the source was named and is not configured;
- *that source does not have it* — the source is fine, the package is not
  there;
- *not built for this machine* — it exists, for other targets, and the message
  lists them.

A single "not found" would have been easier and would have sent people looking
in the wrong place. The same care applies to a version asked for by name: one
that exists only for another target is a target problem, not a missing version.

**`Target::current()`** is how a device knows what it can install, built from
the compiler's own idea of the machine — `aarch64-musl` on a card. On anything
else it gives that machine's honest answer, so a workstation is told that a
package built for a card is not built for it. Which is true, and better than a
card-shaped lie.

An ambiguous name is listed **in the form the user has to type back**, sorted,
so the listing does not depend on the order the configuration happens to be in.

### Step 26 — `search`, `info`, `list` ✅

**Goal.** The three read-only package queries.
**Files.** `src/ops/query.rs`.
**Notes.** `search` matches anywhere in a name, case-insensitively, and exits
non-zero on no matches so a script can tell the two apart. `info` shows
metadata plus what only the client knows — source, installed state, the other
versions. `list` is one line per package with an installed marker.
**Done when.** Tests for each, including `search` finding a package by a
substring of its name and `info --version` selecting an older one.
**Done.** `search`, `list` and `info`, three subcommands, and 15 tests.

**`info` does not refuse an ambiguous name.** Where `install` has to — picking
one would install something other than what was meant — `info` shows every
source that offers the package, because telling somebody about all of them *is*
the answer to what they asked. Only the commands that do something have to
refuse.

**A package built for another machine is listed without a version** rather than
hidden. It exists, and "it exists, just not for you" is worth knowing; hiding
it would leave somebody searching for a name they had seen elsewhere.

**Installed is per source.** The same name from two sources is two packages,
and only one of them is on the device — so a line is marked installed only when
the record's source matches the line's.

`search` finding nothing gets `Error::NothingMatched` rather than
`PackageNotFound`, whose advice is to run `spm search` — which is what has just
happened. Same exit code, an answer that helps.

One test of mine was wrong and the code was right: I expected the version list
descending as `25.07.1, 4.4.1, 23.1.0`, which is a string sort. It is
`25.07.1, 23.1.0, 4.4.1`, exactly as Step 4's ordering says.

---

## M7 — `install` ✅

### Step 27 — Dependency resolution ✅

**Goal.** A package name becomes the full set to install.
**Files.** `src/ops/resolve.rs`.
**Notes.** Breadth-first over the transitive dependencies. A version in
`dependencies` means *that version or newer*; take the lowest that satisfies it
and is not older than what is installed. Skip a dependency already satisfied.
Detect cycles with the visited set and report rather than loop.
**Done when.** Tests for: a chain three deep; a diamond resolved once; a cycle
reported; an unsatisfiable dependency named with what wanted it.
**Done.** `with_dependencies`, and `IndexPackage::lowest_from` under it, with
all four cases tested against packages `create` built.

**A circle and a diamond are the same thing seen from the package that closes
them**, and telling them apart is the whole of the cycle detection. Breadth-first
with a visited set never loops, so "reported rather than followed" needs
something more than the visited set: each package records what pulled it in, and
reaching one that is already in the set is a circle exactly when it is an
*ancestor* of where we are. Anything else is a diamond, and a diamond is
resolved once. The error names the circle in the order somebody has to read it —
`snake -> tail -> snake`.

Three things the step's list did not settle:

- **A dependency is a bare name, and a name only identifies a package within a
  source.** It is resolved against the source of the package that named it
  first, and only then against everything configured. Pulling a package out of
  an unrelated source because its name matched is how a device ends up with
  something nobody chose.
- **A package the user once asked for stays theirs.** One that is already
  installed explicitly and now also arrives as a dependency keeps
  `reason: explicit`, or autoremove would eventually take away something they
  chose by name.
- **A diamond whose two sides disagree about how new the shared package has to
  be** raises the floor and works out what that version needs in turn. Floors
  only ever rise, so it terminates.

"Not older than what is installed" turns out to be a guard rather than a rule
with teeth: a dependency the device already satisfies is skipped before the
floor is ever used. It is implemented anyway, because the rule is easier to keep
than to re-derive.

### Step 28 — The plan and `--dry-run` ✅

**Goal.** Say what would happen before anything happens.
**Files.** `src/ops/install.rs`, `src/ui.rs`.
**Notes.** New packages, upgrades, and the total download. `--dry-run` stops
here and touches nothing — assert *that* in the test, not just the output.
**Done when.** A dry run over a fixture source leaves the prefix
byte-for-byte unchanged.
**Done.** `plan`, `Change`, and `ui::installed`. The done-when is tested as
stated: the whole prefix is walked into a map of path to contents before and
after, and the two are compared. The transport is asked for nothing either — a
dry run that fetched an index would already have told somebody what this device
was about to do.

**The index had to grow a `bytes` field**, and that is the one format change in
this milestone. Both things that need a package's size happen *before* it is
fetched — the download total a plan prints, and the check that the card has room
— and the size was in neither the index nor anywhere else the client can see. A
scan reads it off a release listing without downloading anything, which is the
property the whole index format is built around, so it costs the source nothing.
`docs/dev/DESIGN.md` and `docs/dev/ARCHITECTURE.md` are updated with it.

**`install` does not stop to ask.** The user guide showed a `Proceed? [Y/n]`
prompt that neither the architecture nor this document ever specified — the
architecture lists exactly two options for `install`, and neither is a `--yes`.
A prompt without one would make `spm install` unusable from a script, and Step
41 has the release workflow installing `spm` with it. The guide has been
corrected to what the command prints, as it was in Step 21 over relative dates.

**Downgrades are settled, and are no longer a deferred question.**
`install --version` naming an older version than the installed one replaces it
and the plan says `(downgrade from …)`. Any behaviour settles that question, and
refusing would have meant a device that cannot be put back.

### Step 29 — Verification ✅

**Goal.** Both digests, at the right moments.
**Files.** `src/ops/install.rs`.
**Notes.** The archive against the index's `sha256` **before it is opened at
all**; then `data.tar.gz` against the `sha256` inside the archive's own
`metadata.json`. Two checks, two failure messages — a test that cannot tell
them apart is not testing this.
**Done when.** A fixture with a corrupted outer archive fails before it is
opened, and one with a swapped payload fails at the second check. Both exit 6.
**Done.** Both, and the two are told apart by what the failure names.

The first test serves something that **is not an archive at all** rather than a
corrupted one: anything that had opened it would fail to parse it, so a
`Verification` failure rather than a `Parse` failure is what proves the digest
was checked first. The second needed a package `create` will not build — a
well-formed archive holding one package's payload beside another's metadata —
so `tests/support` gained `swap_payload`, and the index carries that archive's
own digest so the first check passes and only the second can fail.

**The payload is checked against the metadata packed beside it, not against the
index's `payload_sha256`.** They hold the same value, and checking the index's
copy would be checking the index against itself: the outer digest already
establishes that the archive is exactly what the index described. What is left
to establish is that the metadata inside describes the payload inside, and only
the packed copy says that.

### Step 30 — Unpacking ✅

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
**Done.** `inspect` and `extract` over one checked walk, with 18 tests: every
refusal on that list, a `.` component, a name that is not valid UTF-8, and the
good package that has to keep working.

**The hostile archives could not be built with the `tar` crate's own API**,
which refuses `..`, an absolute path and a `.` component when it writes a name —
so the test builder writes those names straight into the header. An archive this
tool could not produce is exactly what these rules are for.

Five things the step's list did not say:

- **A path has to be valid UTF-8.** Not a safety rule but a record one, and it
  is the consequence Step 7 wrote down: a record is JSON, so a file whose name
  cannot be written into one could never be removed again.
- **"Nothing is followed" is about the directories too.** `create_dir_all` walks
  happily through a symlink standing in for one of them, so the directories are
  made one component at a time and a link among them is refused. A link at the
  destination itself is unlinked rather than written through — `remove_file` on
  a symlink removes the link — and the file is then created with `create_new`.
- **Ownership is tested by comparison, not by asserting root.** The tests do not
  run as root, so the claim is that the archive did not get a say: a file
  unpacked from a header recording uid 1001 has the same owner as one the test
  wrote itself.
- **Set-user-id and set-group-id do not come from the archive**, though the rest
  of the permissions do. `create` normalises every mode to 644 or 755 and cannot
  produce one, so a package carrying one did not come from this tool.
- **A directory entry is forced traversable by its owner.** A package asking for
  mode 000 on a directory is asking for one the rest of it cannot be unpacked
  into.

**The payload is streamed rather than staged**, which is why `inspect` and
`extract` take something to read rather than a path: writing `data.tar.gz` out
before unpacking it would put the package on the card a third time, next to the
archive and the tree it unpacks to.

### Step 31 — Conflicts ✅

**Goal.** Refuse rather than overwrite.
**Files.** `src/ops/install.rs`.
**Notes.** Build the file list by reading the payload without writing anything,
then check every path: claimed by another record is a conflict naming both
packages; present on disk and claimed by nothing is a conflict too, because
adopting it would mean `remove` later deleting something `spm` never installed.
**Done when.** Two fixture packages that share a file fail on the second, and
installing over a file the image put there fails. Both exit 7, before any file
is written.
**Done.** `unclaimed`, and both cases tested — including that the refused
package left nothing behind, not even the files it alone owns, and that the file
already there still holds what it held.

**A package's own files are neither kind of conflict.** That is what an upgrade
is, and without the exception no package could ever be replaced by a newer one.

### Step 32 — Commit and recovery ✅

**Goal.** An install that cannot leave a half-installed device.
**Files.** `src/ops/install.rs`, `src/store/db.rs`.
**Notes.** `.partial` with the full file list, then extract, then rename. A
`.partial` found at startup is rolled back before anything else runs.
**Done when.** A test that kills the process between the journal and the rename
leaves a `.partial`; the next command removes those files and the record; the
prefix matches its pre-install state.
**Done.** The journal, and the recovery in front of every command that writes.

**Nothing is killed, and nothing needs to be.** A package whose files straddle
an obstruction gives a genuine interrupted install: the device carries a
symbolic link where one of its directories has to go, so the first file is
written and the second refuses. What is left is exactly what a power cut leaves
— a journal on disk and a file already on the card — and the next command,
which is an ordinary `install` of something else, takes it back before doing
what it was asked.

**`recover` had to learn to leave committed records alone**, and this is the
part that would have been a bug. An interrupted *upgrade* has a journal naming
the files of the version still installed; taking those back would leave a
package whose record says it is whole and whose files are gone. A file some
committed record claims is now skipped — leaving one holding the newer version's
contents, which the next install puts right, rather than deleting one the device
is still using.

**An upgrade takes away what the old version no longer ships**, after the rename
rather than before it. Without that the file stays on the card owned by nobody,
where `remove` would never reach it and the next install would refuse to
overwrite it. A crash between the two leaves a stale file, which is recoverable;
the other order leaves a missing one, which is not.

Recovery runs in `main` for `update`, `add-source` and `remove-source`, which
Step 12 asked for and nothing had done yet, and inside `install` itself —
because a caller of the library does not come through `main` and has to be as
safe as one that does.

### Step 33 — Disk space ✅

**Goal.** Refuse before filling the root filesystem.
**Files.** `src/ops/install.rs`.
**Notes.** A package is on the card twice during an install — the archive and
the unpacked tree — so require roughly twice its size and say so when it will
not fit. On a 2 GiB card with Rust and Helix already on it, this is not
hypothetical.
**Done when.** A test with a constrained prefix refuses with a message naming
what is needed and what is free.
**Done.** `store::space::available` and `enough_room`, refused before a byte is
fetched — which the test asserts by looking at what the transport was asked for.

**The prefix is not constrained; the package is inflated.** A small filesystem
cannot be made in a test without root or platform-specific tooling, and it does
not have to be: the index is where a device learns how big a download is, so an
index claiming a package no card could hold exercises the real check against the
real free space of the real filesystem. It also shows what a wrong index costs —
a refusal, and not a full card.

**There is no free-space call in `std`.** The alternative to a dependency was
`statvfs` through `libc`, which would have been this crate's only `unsafe`
block. `rustix` wraps the same call safely and `tempfile` already pulls it in,
so enabling one more of its features adds nothing to the tree and the crate
still contains no `unsafe` at all — the same conclusion Step 10 reached about
`flock`, by a different route.

## M8 — `remove` ✅

### Step 34 — Removal ✅

**Goal.** Take back exactly what was installed.
**Files.** `src/ops/remove.rs`.
**Notes.** Refuse if another record depends on this package, listing the
dependents. Delete the record's files in reverse order, skipping any path
another record also claims. Remove directories that have emptied.
**Done when.** Tests for: a blocked removal names its dependents; a shared file
survives; directories are cleaned; nothing outside the record is touched.
**Done.** `plan` and `remove`, with the four cases tested against devices built
by running the real `install` over packages the real `create` built — so what is
taken back is what was actually put on, rather than records a test wrote to
describe an install that never happened.

**Pruning directories is shared with the rollback**, which is what Step 12 said
would happen: it left empty directories behind and recorded that this step owned
the fix. `store::db::prune` is that fix and both halves of taking a package back
use it, so an install that did not finish now leaves no trace either — an empty
directory is a trace.

Three things the step's list did not settle:

- **A directory that will not go is not a failure.** It is not empty, it is not
  there, or the filesystem said no. An empty directory left on a card harms
  nothing, and failing a removal that has otherwise succeeded over one would.
- **The root is never removed.** The walk is over a path relative to it and
  stops when there is nothing left of that path, so there is no case in which it
  reaches the top.
- **A qualified name is a different question, not a different package.** There
  is one record per name, so a device has at most one `helix`; `sepia/helix`
  asks about the one that came from `sepia`, and if the one installed came from
  somewhere else then what was asked about is not installed. That needed
  `Error::NotInstalled`, because `PackageNotFound`'s advice is to update or
  search — and a package that exists everywhere except on this device is not one
  to go looking for.

**The shared-file rule is a net rather than a mechanism.** `install` refuses to
let two records claim one file, so on a device this tool built it takes nothing
away, and the test that covers it writes the two records directly because
nothing else can produce that state. It is kept because a record is a file
somebody can edit, and deleting a file another package is using because two
records disagreed is not a mistake worth being able to make.

### Step 35 — Autoremove ✅

**Goal.** Dependencies that are no longer needed go too.
**Files.** `src/ops/remove.rs`.
**Notes.** Repeatedly drop any `dependency` record that nothing remaining
depends on, until a pass changes nothing. A package installed explicitly is
never taken automatically, however unreferenced it is.
**Done when.** Removing the head of a three-deep chain removes all three; an
explicitly installed middle package stops the cascade.
**Done.** `with_unneeded`, with both cases, plus a dependency two packages share
staying until the second of them goes.

**The set is worked out before anything is deleted**, and the order it comes
back in is part of the answer: what somebody asked for is the first line of what
they are shown, and what followed from it comes after, in the order it followed.
A set would have sorted them alphabetically and lost that.

**The refusal is checked against what remains**, not against everything
installed. A package that only an about-to-be-autoremoved package depends on can
still go, which is the ordinary case for a chain: `middle` depends on `bottom`,
and both are leaving.

**The pass is a sweep rather than a trace.** It drops *any* unneeded dependency,
not only the ones this removal orphaned. There should never be others — a
removal always sweeps — but an upgrade to a version that dropped a dependency
can leave one, and a device quietly accumulating packages nothing needs is worse
than a removal that mentions one extra name.

## M9 — `upgrade` ✅

### Step 36 — `upgrade` ✅

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
**Done.** `upgrade`, 12 tests, and **no second install path**: `install` was
split at exactly the seam the step names, into `plan_for` — what a set that has
already been resolved amounts to — and `carry_out` — the room check, the two
digests, the extraction rules and the journal. `upgrade` works out its own set
and joins there.

**Two resolutions, on purpose.** Each candidate's dependencies are resolved
*alone* first, because that is the only way to know which package to blame:
resolving them all together would mean one unsatisfiable package taking the
whole upgrade down with it, which is the opposite of what this command is for.
The survivors are then resolved together, so a dependency two of them share is
worked out once and at one version.

Three things the step's list did not settle:

- **A newer version comes from the source the package came from.** A record
  says where it was installed from, and taking an upgrade from another source
  because the name matched would swap a package for a different package of the
  same name. So a source that has been removed offers nothing — which is
  exactly what `remove-source` says will happen, and there is now a test that
  it does.
- **A package that came in as a dependency stays one when it moves.** Making it
  explicit because it was upgraded would quietly take it out of autoremove's
  reach for ever. That is why `resolve::with_dependencies` now takes its roots
  *with the reason each is to be recorded under*: `install` makes a root
  explicit whatever it was, and `upgrade` must not.
- **Every way of having nothing to do is the same answer.** The source is gone,
  its index was never fetched, it no longer lists the package, it has nothing
  for this machine, or what it has is not newer. None is a failure and none is
  something a person can act on differently, so none of them is told apart.

**`with_dependencies` now takes several roots**, which is what let the survivors
be resolved as one set. That has one consequence worth recording: **a circle is
only found below a root.** Two roots that need each other are not reported as
one, and should not be — both are in the set already, so the set *can* be
completed, which is the only thing a circle would have made impossible.

**The exit is separate from the work**, as `update`'s is: the caller prints
everything that happened *and then* fails, so a script sees the two packages
that moved as well as the one that did not. `Error::UpgradeIncomplete` is its
own variant rather than `Incomplete`, which is about sources and says so; both
leave code 8.

One thing the output got wrong at first and a test would not have caught,
because it is a sentence rather than a state: with nothing upgradable *and* a
package held back, it said "everything is already at the newest version" and
then named a package that was not. Those are opposite things, and it now says
"nothing could be upgraded" instead.

## M10 — Shipping

**Every step here is done; the milestone has no ✅ yet**, and that is deliberate
rather than an oversight. Two of the four "done when" conditions can only be
observed on GitHub — a green CI run on a branch, and a dispatched release — and
this is the one milestone whose work is mostly *not* code, so there is no local
test that can stand in for them. The tick belongs on the first green run of
`ci.yml` with the `cross` job in it, and the first release that publishes a
package.


### Step 37 — Cross-build ✅

**Goal.** An `aarch64-unknown-linux-musl` binary.
**Files.** `.cargo/config.toml`, `README.md`.
**Notes.** Static by default on that target, which is the point — it must run
on a card built `WITH_LLVM=0`, where there is no `libgcc_s`. `ring`, under
`rustls`, needs a C compiler for the target: the cross toolchain the sibling
repositories already use. A build script needs a *host* compiler too — the trap
that broke `Sepia-OS/grit`'s first CI run.
**Done when.** `readelf -l` shows no interpreter and `readelf -d` no
`NEEDED` — the same assertion `grit-check` makes.
**Done.** `cargo build --release --locked --target aarch64-unknown-linux-musl`
produces a 4.6 MiB `EXEC` for AArch64 with no `INTERP` segment and zero
`NEEDED` entries, from a `.cargo/config.toml` of two tables and no container.

**Both halves of the toolchain question turned out to be the same answer.**
rustc needs a C compiler to drive the link and cannot use the host's, and
`ring` needs one that compiles *for* the target; pointing
`CARGO_TARGET_..._LINKER`, `CC_<target>` and `AR_<target>` at the one
musl-targeting toolchain the family already downloads settles both. Verified
rather than assumed: `ring`'s objects in the target directory are
`ELF 64-bit ... ARM aarch64`, and its build log names
`aarch64-unknown-linux-musl-gcc`, so the `CC_` entry is load-bearing and not
decoration.

**The names are defaults, not requirements**, which is why they are in `[env]`
rather than baked in. No single vendor publishes a musl-targeting `aarch64`
toolchain for both hosts — messense is macOS-hosted and spells the triple out
in full, bootlin is Linux-hosted and prefixes `aarch64-linux-` — and cargo's
`[env]` yields to a variable that is already set, so the other vendor is three
environment variables rather than a patch. Step 39 will need exactly that.

**Nothing here reaches a host build.** A `[target.<triple>]` table applies only
when that triple is asked for, and `CC_<target>`/`AR_<target>` are read only by
cc-rs when compiling for it. `cargo build`, `cargo clippy --all-targets` and
the full suite behave exactly as they did before the file existed, which was
checked rather than reasoned about — the alternative, a `[build] target = ...`
line, would have quietly made every `cargo test` a cross-build that could not
run.

**The failure mode is already legible**, so it wanted no help: with no
toolchain on `PATH` the build stops at ``error: linker
`aarch64-unknown-linux-musl-gcc` not found``, which names the missing thing and
the file that asked for it.

**What this does not prove** is that the binary runs. It is an assertion about
an ELF header made by a `readelf` that never executed a single instruction of
it — which is Step 38, and why Step 38 exists.

### Step 38 — Run the tests on the target ✅

**Goal.** The cross-built binary is executed, not just built.
**Files.** `.github/workflows/ci.yml`.
**Notes.** CI runners are x86_64, so run the target test binaries under
`qemu-user-static` in the same container — the suites here are small, and the
compile dominates. On an Apple Silicon workstation the same binaries run at
native speed in a `linux/arm64` container. This is how helix's runtime
behaviour was verified.
**Done when.** The full suite passes on the host and again on `aarch64`, in CI.
**Done.** A `cross` job in `ci.yml` builds for the target and then runs the
suite on it, all 305 tests, through
`CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUNNER=qemu-aarch64-static` — cargo
launches each target test binary through the emulator, so `cargo test --target`
is the whole command and no separate "find the test binaries and run them" step
exists to drift.

**Both halves were rehearsed before the job was written.** The binaries were
cross-built on the workstation and run twice: natively in a `linux/arm64`
container, where all 305 pass, and under `qemu-aarch64-static` in an x86_64
container, which is what a runner is — `uname -m` says `x86_64` and the suite
still passes. So the emulation is known to work on the architecture CI actually
has, rather than assumed to.

**Nothing had to change to make the tests pass on aarch64**, which is worth
recording because it is the outcome the guidelines' "the build host is not the
target" rule was written to produce: no test depended on the host's word size,
page size or path conventions, so there was nothing to fix.

### Step 39 — CI ✅

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
**Done.** The missing half is there: two jobs, `host` and `cross`. The host job
is what it was. The cross job runs in `rust:1.98-trixie` and does the
cross-build, the ELF assertion and the emulated suite.

**The container is the floor, deliberately.** 1.98.1 is what
`Sepia-OS/rust-toolchain` puts on a card, so the device half is built with the
oldest compiler that has to work while the host job uses the newest stable.
Between them both ends of the supported range are checked on every commit,
which is more than either alone would say.

**The non-slim `rust:` image, also deliberately.** It carries a host C compiler,
and a cross-build needs one — build scripts and proc macros are compiled and run
on the build machine. A container with only the cross toolchain fails partway
through a dependency with ``linker `cc` not found``, which is the trap Step 37
recorded and this image avoids by construction.

**Step 37's acceptance test is now a build step**, not a thing somebody
remembers to check: aarch64, no `INTERP`, no `NEEDED`, on every commit.

**The toolchain overrides are the ones Step 37 documented.** `.cargo/config.toml`
names messense, which publishes no Linux build; the job exports the bootlin
spellings and nothing in the repository is patched. That the file was written to
be overridden exactly this way is now load-bearing rather than theoretical.

**Not yet observed:** a green run, and the deliberately broken commit failing
the expected step. Both need the branch pushed — GitHub Actions is the only
place either can happen. Every mechanism the run depends on was rehearsed
locally: the image tag and both bootlin URLs resolve, `qemu-aarch64-static` runs
the binaries on x86_64, and the workflow parses.

### Step 40 — Release ✅

**Goal.** A published binary with its checksum.
**Files.** `.github/workflows/release.yml`.
**Notes.** Manual dispatch with a version; branch `main` to `rel-<version>`;
replace `0.1.0-replace-me` in `Cargo.toml`; build; publish the binary and a
`SHA256SUMS`. Reuse an existing `rel-<version>` branch rather than failing.
**Done when.** A dry run on a scratch tag produces the assets, and the version
in `--version` matches the tag.
**Done.** `release.yml`, in the shape the sibling repositories use — a `gate`
that can say no in seconds, a `build` on the release branch, a `publish` on the
bare runner where `gh` lives — with two deliberate differences.

**Something is committed to the release branch**, which is not true of `boot`.
`Cargo.toml` carries `0.1.0-replace-me` on `main`, and the release is where a
real number is written; the stamp is a commit on `rel-<version>`, so the
released version is recorded in git rather than existing only inside an asset.

**So the branch is reused rather than refused.** `boot` deletes its release
branch when a build fails, because the branch holds nothing `main` does not.
Here it can hold the stamp commit, and deleting it would throw that away — so a
retry is just dispatching the same version again, and the stamp step does
nothing when the version is already right. That is why there is no `rollback`
job: reuse *is* the recovery, exactly as this step's note asked for.

**Three files carry the version, not one.** `Cargo.lock` records the version of
the root package too, and `--locked` fails on a lockfile that disagrees with
`Cargo.toml` — so a release that stamped only `Cargo.toml` would not build at
all. `metadata.json` is stamped with them, from the same placeholder.

**The binary is asked, not the file it was built from.** `spm --version` is run
under `qemu-aarch64-static` and compared against the tag, because a stamp that
did not reach the compiled artifact is precisely the failure worth catching and
only running it can tell. Rehearsed on the workstation: stamping the version
does cause cargo to rebuild, and the rebuilt binary reports the stamped version.

**A false alarm worth recording**, because it cost time and would cost it again:
the first rehearsal reported a version mismatch and looked like cargo failing to
rebuild. It was the rehearsal script, not the release — BSD `sed` on macOS
rejects GNU's `0,/re/` address form, so nothing had been stamped. The workflow's
own stamping was verified in a Debian container, where it is correct and
idempotent across a second run.

**Not yet observed:** a dispatched run. The build path was rehearsed end to end
locally — stamp, cross-build, ELF assertion, `--version` against the tag — but
the gate's CI check, the branching and the publish only exist on GitHub.

### Step 41 — `spm` packages itself ✅

**Goal.** The first real package.
**Files.** `metadata.json`, `.github/workflows/release.yml`.
**Notes.** The release runs `spm create` on its own staged tree, so the package
manager ships as a package like any other. Tag the repository `package` and the
index picks it up on its next scan — which is also the first end-to-end test of
the notification path.
**Done when.** The release publishes a package, its `metadata.json` and a
`SHA256SUMS`; a device with the source configured can `spm install spm`.
**Done.** `metadata.json` is checked in beside the sources it describes, and the
release stages the tree as it appears on a device — the binary under `usr/bin`,
the licence under `usr/share/licenses/spm/` — then runs `spm create` on it
**using the binary being packaged**, under the emulator. So `spm` is packaged by
`spm`, and the release is the first exercise of `create` on a real tree rather
than on a fixture one of its own tests built.

**Verified on aarch64, end to end:** the cross-built binary packs its own staged
tree, writes the three documented files, and `sha256sum --check` passes. The
package holds exactly `data.tar.gz` and `metadata.json`, as the architecture
says it must.

**A fourth asset, which this step did not list: the bare binary.** A device with
no `spm` on it cannot install one with `spm`, and a package is the wrong shape
for that first install. `create`'s `SHA256SUMS` covers the package; the binary's
line is appended, which leaves an ordinary `sha256sum` list — both entries check,
and a reader looking for the package still finds it by name.

**The placeholder version parses**, which was not obvious and is why it was
tried: `spm create` accepts `0.1.0-replace-me` and writes
`spm-0.1.0-replace-me-aarch64-musl.tar.gz`. So the checked-in `metadata.json` is
a usable file rather than a template that only works after substitution.

**Not yet done, and not something this repository can do to itself:** tagging
the repository `package` so a source's scan picks it up, and the
`spm install spm` that follows from it. The first needs a topic set on GitHub,
the second needs a device with that source configured — and the release has to
have been cut before either means anything.

---

## Once Deferred

**This list is empty, and that is what it now records.** Every item on it was
named here so that leaving it out stayed a decision rather than an oversight,
and each has since been built. They are kept rather than deleted because what a
project decided *not* to do at the time, and why, is the part that is otherwise
lost — a list that only ever held pending work would have quietly become an
empty heading.

In the order they were built:

**Configuration in packages.** `create` refused anything outside `usr/`, so a
package could not ship defaults in `/etc`. A package may now ship a top-level
`etc/`; `spm` records the digest of what it wrote there and treats an edited
file as the administrator's — never overwriting it, never deleting it, and
writing a new default beside it as `<name>.spmnew` instead. The behaviour is in
[ARCHITECTURE.md](ARCHITECTURE.md) under *Configuration* and the mechanism in
[DESIGN.md](DESIGN.md) under *Configuration files*.

**Addressing a source by name.** `add-source`, `remove-source` and
`source-info` took a URL while every other command took a name.
`remove-source` and `source-info` now take either, told apart without a flag
because a name cannot hold the `:` and `/` a URL's scheme needs. `add-source`
still takes a URL, and that is not the same inconsistency: a source that has not
been added yet has no name to be addressed by.

**A `verify` command** to re-check installed files against their records. It
checks one package or the whole device — present, still a file, and still the
bytes that were installed — reporting an edited configuration file as the
non-fault it is, and exiting 6 when something is actually wrong.

**A digest for every installed file**, which is what let `verify` check contents
rather than only presence. A record carries the digest of every regular file as
it was written, taken during the extraction rather than by a second pass over
the card. The cost was measured before it was accepted: 176 bytes of record per
file, about 1.9 MB for a package the size of Helix.

**Signing an index** — and, as it turned out, packages too. The digest chain
protected a download from the network, not a device from a source that had been
taken over. Indexes are now signed by their source against a key pinned in
`sources.json`, packages by their publisher against a key the verified index
names, and a key is required rather than optional. The behaviour is in
[ARCHITECTURE.md](ARCHITECTURE.md) under *Signing* and the trust chain in
[DESIGN.md](DESIGN.md) under the same name.
