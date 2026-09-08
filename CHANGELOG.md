# Changelog

All notable changes to this repository are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project will adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once it has something to version.

## [Unreleased]

### Added

- `spm search`, `spm info` and `spm list`. A search matches part of a name and
  ignores case; a package two sources offer is shown qualified; a package built
  for another machine is listed without a version rather than hidden, because
  "it exists, just not for you" is worth knowing. `info` shows every source
  that offers a package rather than refusing — unlike `install`, telling
  somebody about all of them is the answer to what they asked — along with the
  other versions on offer, what it depends on, and whether it is installed. A
  search that matches nothing says so rather than advising a search.
- Name resolution: `helix` or `sepia/helix` becomes one package, in one source,
  at one version. A name two sources offer is refused and listed in the form it
  has to be typed back, because picking one would mean installing something
  other than what was meant. "No such source", "that source does not have it"
  and "not built for this machine" are three different answers with three
  different fixes, and the last one lists the machines it *is* built for.
- `spm update`, which fetches each source's index and puts it in place whole or
  not at all — an index that arrives as nonsense leaves the previous one
  untouched, because a device with a stale index can still install and a device
  with half an index can do nothing. One unreachable source does not stop the
  others: every source is attempted, each failure is named, and the command
  finishes non-zero so a script can tell an incomplete picture from a complete
  one. With it, the single-writer lock is now taken by the commands that write.
- `spm remove-source`, which removes a source and its local index and
  **uninstalls nothing**. Packages installed from it stay, still recording
  where they came from; what they lose is upgrades, and the command says how
  many packages that is. If the removed source held the default flag it moves
  to the last one standing, or — with several left, where nothing could guess —
  the device is told it now has no default and how to name one.
- `spm add-source`, which fetches the index before writing anything: that
  proves the URL is a source rather than a typo, it is where the source's name
  comes from, and it leaves the source usable without an `update` first. A
  non-`https` URL is refused before anything is fetched. Re-adding a configured
  URL updates its entry rather than duplicating it, and does not drop its
  default flag. A name another URL already holds is refused, naming the holder
  and pointing at `--name`.
- `spm list-sources` and `spm source-info`, the first two commands a device
  actually answers. They report what is configured, when each index was last
  rebuilt, how many packages it offers and how many installed packages came
  from it. A source whose index has never been fetched reads as exactly that
  rather than as one offering nothing — the two look alike in a listing and
  mean opposite things — and a device with no sources at all is told how to add
  one rather than shown an error, since that is what a fresh card looks like.
- Downloads that stream to disk and hash on the way past, never into memory: a
  package is 216 MiB and the smallest supported board has 512 MiB of RAM. A
  test measures the largest buffer the download ever asks for and fails if it
  grows with the file. The file is written beside its destination and renamed
  into place, so an interrupted download cannot be mistaken for a complete one
  and a failed retry does not destroy the copy that already worked.
- A device whose clock has not been set is told about its clock. A Raspberry Pi
  has no battery-backed clock, so until `sepia-time` runs it believes it is
  1970 — and every certificate on earth begins later than that, so every one of
  them is "not yet valid". Reporting that as a certificate error sends somebody
  to look at the server, the source or their network, none of which is wrong.
  The message names the clock, says what it reads, and points at `sepia-time`.
  Only a TLS failure can become that message, and only when the clock is
  behind the image's own build date.
- The real transport: HTTPS with its own root certificates compiled in, because
  a SepiaOS card has no trust store to read. A plain `http://` URL is refused
  before a connection is opened rather than upgraded behind the user's back,
  and a certificate signed by nobody the compiled-in roots know is refused —
  both tested against a listener the test starts on the loopback address. A
  `GITHUB_TOKEN` is sent to GitHub and to no other host, checked on the whole
  host name. Compressed transfers are not asked for, since everything here is
  checked against a digest of a file.
- `Transport`, the one seam between `spm` and the network: fetch a URL, get
  something to read. Everything above it takes a `&dyn Transport`, so a test
  drives the real `update`, the real `install` and the real verification with
  fixtures behind them and no network anywhere. With it a fake that serves a
  directory, can be told to break a URL, and remembers what was asked for.
- Test fixtures, built by `spm create` rather than checked in — so there is one
  implementation of the package format and the fixtures cannot drift from it.
  Three named shapes for the steps that follow: one package that stands alone,
  one that depends on it, and one that ships the same file so the two cannot
  both be installed. The builder also takes versions, targets, arbitrary files
  and a missing licence, which is what the later steps need.
- `spm create`, and with it the command line: a staged tree and a metadata file
  become the three things a release publishes — the package, its metadata with
  the payload digest filled in, and a `SHA256SUMS` that `sha256sum -c` reads.
  The package holds exactly `data.tar.gz` and `metadata.json`, the digest in
  the packed metadata describes the payload beside it, and building the same
  tree twice gives the same bytes. The author's own metadata file is read and
  never written to. A version that could not be part of a filename is refused
  here, since a version is the one identifier upstream chooses and this is
  where one becomes a file name.
- The refusals `create` makes: everything under `usr/`, a non-empty licence
  under the package's own name in `usr/share/licenses/`, and no libc or dynamic
  loader — the last using the same names the `rootfs` build already refuses in
  every sibling package. The fourth rule, that the metadata names a package, is
  not checked because it cannot be broken: such a metadata file does not parse.
  Refusals carry the offending path as a value rather than a sentence.
- The payload packer: a staged tree becomes `data.tar.gz`, hashed in the same
  pass that writes it. The same tree gives the same archive byte for byte —
  entries sorted, timestamps zero, everything owned by `root:root`, modes
  normalised to 755 or 644 — so a package's digest identifies its contents and
  not the machine or the moment that packed it. Symlinks are packed as links,
  because `grit` ships `git -> grit` and following it would put the binary on a
  card twice under two names.
- The installed database, and the journal that makes an interrupted install
  recoverable. A record is written as `<name>.json.partial` listing every file
  the install will write, *before* any of them exists, and renamed into place
  only when they all do — so a marker left behind can only mean "undo this",
  never "finish it", because nothing in it says how far the install got.
  `recover` does that undoing and every command that writes runs it first. A
  `.partial` is not an installed package, which the enumeration and the
  installed check both know. A record claiming a path outside the device's root
  is refused rather than obeyed.
- `/etc/spm/sources.json`, read and written. An absent file is not an error: it
  means no sources are configured, which is what a freshly installed card looks
  like. Two invariants hold over it — names are unique, and at most one source
  is the default — enforced when reading, where a hand-edited file can break
  them and the message says which and where, and made impossible when writing,
  because the only ways to change the list maintain them. It is written
  pretty-printed, since it is the one file somebody may edit on the device.
- The single-writer lock, held by every command that writes and by none that
  only reads. The kernel owns it, so a killed `spm` leaves nothing stale behind
  — there is no timeout to tune and no cleanup on the way back. The holder
  records its process id so a waiter can say what it is waiting for. Built on
  `std::fs::File::lock` rather than `flock` through `libc` as the design
  expected, which means **the crate contains no `unsafe` and no C dependency**;
  the design and the development guidelines have been corrected to match.
- Atomic writes: bytes go to a temporary file in the destination directory, are
  flushed to the disk, and are then renamed over the destination — so a device
  that loses power mid-write finds the previous file rather than half of the
  next one. The temporary lives beside its destination and not in `/tmp`,
  because a rename across filesystems is a copy and a delete, which is the
  non-atomic write the whole thing exists to avoid. A closure form exists so
  that an index can be serialised straight into the file without building it in
  memory first.
- `Store`, which owns the root prefix every path is built from — the sources
  file, the index copies, the installed records, the lock and the download
  cache. It exists so the tests can run against a temporary directory rather
  than the real `/`, and it is not a command-line option: a package manager
  with a `--root` flag is one that can be pointed at the wrong system.
- `Index` and `Record` — what a source publishes and what the device remembers.
  An index carries two digests per version under names that cannot be mistaken
  for one another, `sha256` for the package and `payload_sha256` for the
  payload inside it, and `Index::newest` answers the question `install` asks:
  the highest version *built for this target*. A record knows what it depends
  on and whether the user asked for it, which is what `remove` and its
  autoremove pass need. Both round-trip against the examples in the design
  document.
- `Metadata`, `Dependency` and `Sha256` — a package's `metadata.json` as a
  type, read strictly: an unknown field is a misspelling to report rather than
  a key to drop, so a hand-written file saying `dependancies` fails instead of
  quietly installing a package with no dependencies. Its round-trip test uses
  the example from the architecture document verbatim, so the two cannot drift
  apart in silence. `sha256` is empty until `create` fills it, which the type
  says by being an `Option`. Also `Target`, validated like a name because it
  becomes part of a filename, and serde for `Version` and the name types.
- `PackageName`, `SourceName` and `PackageRef`, which parses both `helix` and
  `sepia/helix` so that no command re-implements the split. Names are validated
  strictly because a name becomes a filename — an installed package is recorded
  at `installed/<name>.json` — so a name that walks up a directory is refused
  where a name is made, rather than guarded for wherever a path is built.
  Lower case is required for the same reason: on a filesystem that ignores
  case, two spellings would be one record.
- `Version`, and the ordering upstream version numbers need. Not semver:
  numeric components compare numerically so `25.07.1` and `25.7.1` are one
  version, a number sorts above text so `1.0` beats `1.0-rc1`, and a missing
  component is zero so `1.2` is `1.2.0`. A version keeps the spelling it
  arrived with for display while comparing by its components, and `Eq` and
  `Hash` follow the ordering rather than the text — otherwise two equal
  versions would hash differently and a map keyed on one would lose entries.
- A CI workflow, on every commit on every branch and every pull request against
  `main`: the licence header on every source file, `cargo fmt --check`,
  `clippy --all-targets -- -D warnings`, a build and the tests — everything
  `--locked`, so a `Cargo.lock` that has drifted fails the run rather than
  quietly resolving different versions than the commit was written against. It
  also fails if no tests are collected at all, which a green run would otherwise
  look exactly like. The cross-build and the run under emulation are Steps 37
  and 38 and are not here yet.
- The error type and the exit codes: sixteen variants covering all eight codes
  the user guide documents, each carrying the context needed to act rather than
  a formatted string. `Error::exit_code` matches exhaustively with no catch-all
  arm, so a variant added without a code is a compile error rather than a silent
  `1` — which is a stronger guarantee than the test beside it.
- `src/lib.rs`. The crate is now a library with a thin binary on top: an
  integration test cannot reach inside a binary crate, and the design asks for
  tests that drive whole commands.
- The crate skeleton: 30 modules, one per entry in the design's tree, each with
  the licence header and a `//!` describing what it owns and what it does not.
  Nothing implements anything yet — this is the scaffold the rest is built
  inside. The `[lints]` table from the development guidelines went into
  `Cargo.toml` with it, so a build in a worktree enforces what CI will.

- The start of the project: a Cargo workspace and a README describing `spm`,
  the SepiaOS package manager, as two binaries — the client itself and its
  counterpart.
- `docs/dev/ARCHITECTURE.md`, where the specification now lives and has grown
  into the whole command set: `update` (`--all`, `--source <name>`), `search`,
  `info`, `upgrade`, `install`, `remove`, `list` (`--installed`,
  `--source <name>`) and `create`. A package offered by more than one source is
  named `<source>/<package name>` so the user can say which one they mean.
- A description of `create`, which until now was a single line, and with it the
  package format: a `.tar.gz` holding exactly two members — `data.tar.gz`, the
  tree that is unpacked on the device, and `metadata.json`, which names the
  package, describes it, lists the dependencies that have to be installed
  alongside it — each with the version it needs, read as that version or a
  newer one — and carries the SHA-256 of `data.tar.gz`. `create` takes
  `--root`, `--metadata` and `--output`; everything else a package declares
  lives in `metadata.json`, in the package repository beside the sources it
  describes, rather than on a command line. It refuses a package that breaks
  any of four rules: everything under `usr/`, a licence under
  `usr/share/licenses/<name>/`, no libc or dynamic loader inside, and a
  `metadata.json` that says what the package is. The first three are the
  invariants every SepiaOS package asset already holds to and that `rootfs`
  asserts on the way in.

- Every remaining command specified to the same depth as `create`: what it
  reads, what it writes, what it refuses and why. `install` verifies the
  payload's SHA-256 before unpacking and refuses to overwrite a file owned by
  another package or by nothing at all; `remove` takes with it what was pulled
  in as a dependency and is now needed by nothing, and refuses while another
  package still depends on it; `upgrade` leaves one unsatisfiable package alone
  rather than the whole device unpatched; `search`, `info` and `list` say what
  they print. `upgrade [<package name>]` and `--version`, `--dry-run` options
  where they earn their place.
- The three source commands specified: `add-source <url>`, which fetches the
  index before writing anything — so the URL is proved to be a source, the
  source's name is learned from the index that declares it, and the source is
  usable without an `update` first — with `--default` and `--name <name>`, and
  refusals for a non-`https` URL, an index that will not parse, and a name
  another source already holds; `remove-source <url>`, which leaves packages
  installed from it alone and says how many lose their upgrades; and
  `source-info <url>`, which distinguishes a source whose index has never been
  fetched from one that offers no packages. `list-sources` gives the same
  information for every configured source, and is where a user finds the names
  that `--source <name>` and `<source>/<package name>` are written in.
- A "What a source is" section: a source is a git repository that publishes an
  index and maintains it by scanning — a recurring action walks the sibling
  repositories of the same organisation, takes an interest in those tagged
  `package`, and adds each release it has not seen. So a repository joins a
  source by being tagged and a version is published by cutting a release, with
  no register to keep in step. A package repository also notifies its index
  when it publishes — it finds it by tag too, as the sibling repository marked
  `package-index`, so neither side has the other's name written down. The
  notification only says *look now*: the index reads the release itself rather
  than believing what the event carries, so a lost notification costs
  promptness and nothing else. Scanning makes the index correct; being told
  makes it prompt. The index is derived rather than authoritative,
  so it can be discarded and rebuilt from the repositories at any time. The
  index carries each package's metadata, so the client never downloads a
  package to find out what it is, and two checksums per version: one of the
  package as published, which `install` checks before it opens the archive, and
  one of the payload inside, which binds the metadata to what it describes. A
  release publishes the package, its `metadata.json` and a `SHA256SUMS`, and
  the scan reads both digests from those rather than downloading anything —
  which is what `create` now writes.
- `docs/dev/DESIGN.md`: how the client is built, against what
  `docs/dev/ARCHITECTURE.md` says it does. The module layout, the three on-disk
  formats, the version ordering (upstream versions are not semver — `25.07.1`
  and `25.7.1` are the same version), the install journal that makes an
  interrupted install recoverable in one direction, the extraction rules, the
  crate choices and why each survives a static musl cross-build, the exit
  codes, the test seams, and the open questions. Four constraints drive most of
  it: the card has no trust store, it has no clock until `sepia-time` runs, a
  package can be 216 MiB on a 512 MiB board, and `spm` has to work on a card
  built with `WITH_LLVM=0` — which is why it is statically linked like `grit`.
- `docs/dev/IMPLEMENTATION-PLAN.md`: 41 steps in dependency order, grouped into
  eleven milestones, each with the files it touches and what has to pass before
  it counts as done. `create` is built fourth rather than last, because it
  needs neither the network nor the installed database and it is what produces
  the fixture packages every later test installs. Five deferred items are named
  so that leaving them out stays a decision — index signing among them.
- `docs/USER-GUIDE.md`: the documentation for somebody with a device rather
  than somebody building one. Adding a first source, finding and installing
  packages, what `<source>/<package>` is for, upgrading, removing, managing
  sources, publishing a package with `create`, the exit codes, and where
  everything is kept. Its troubleshooting section leads with the clock, since a
  Pi that has just booted fails TLS until `sepia-time` has run and the error it
  gives points at certificates rather than at the cause.
- Rust guidelines in `docs/dev/DEVELOPMENT-GUIDELINES.md`, beside the file
  header rule already there: Rust 1.98.1 as the floor, because that is what
  `Sepia-OS/rust-toolchain` publishes and therefore the newest compiler a
  SepiaOS device has; lints configured in `Cargo.toml` so a worktree build
  enforces what CI does, with `unwrap`, `expect` and `panic` denied outside
  tests because this program runs as root and writes into `/`; `unsafe` denied
  at crate level with one isolated exception for `flock`; nothing held in
  memory proportional to a package; `deny_unknown_fields` on every format read
  from disk or network; the module boundaries from `DESIGN.md` restated as a
  rule; a dependency policy that keeps C libraries and async runtimes out of a
  static musl cross-build; and the changelog rule this entry follows.
- A "State on the device" section, because the commands could not be specified
  without it: the configured sources in `/etc/spm/sources.json`, what `spm`
  knows in `/var/lib/spm/` (the local index copies, and for each installed
  package its metadata, its file list, its source, and whether it was asked for
  or pulled in), and downloaded packages in `/var/cache/spm/`. A package counts
  as installed only if that record says so, and a file no record claims is one
  `spm` never touches.

### Changed

- Work on a module now starts by branching `feat/<module>-<title>` —
  `feat/M1-the-model` for the next one — with every step in that module
  committed there and the branch
  merging to `main` when the module earns its ✅. Recorded in both the
  development guidelines and the implementation plan, since one is where the
  rule lives and the other is where somebody is standing when it applies.

- The specification the README used to carry has moved to
  `docs/dev/ARCHITECTURE.md`, and the README now carries what surrounds it
  instead: what `spm` is for — a card that can be added to for as long as it is
  in use, rather than one that is finished when it is written — an honest
  status, a map of the five documents, how a package and a source work in three
  paragraphs, the eleven SepiaOS repositories and what each builds, how to
  contribute, and the licence.
- The introduction spoke of configuring more than one package *index* where the
  rest of the document says *source*. A source publishes an index; the two
  words now mean one thing each.

### Fixed

- `Cargo.lock` was ignored and untracked. `spm` is an executable, so its builds
  have to be reproducible — the same commit producing the same binary on a
  workstation, in CI and on a device — and that needs the lockfile. The rule
  came from a generic template whose own comment says to remove it when the
  crate is an executable; the same class of mistake, a template `.gitignore`
  swallowing a file the build needs, broke `Sepia-OS/rust-toolchain`'s CI.

- Spelling, grammar and agreement throughout `docs/dev/ARCHITECTURE.md` —
  among them `eror`, `recuring`, `everytime`, `creats`, `more then one`, and
  `iff and only if`, which says the same thing twice. `remove <package name>`
  gained the parameter every other such heading names, and `--source <name>` is
  now spelled one way rather than two.

[Unreleased]: https://github.com/Sepia-OS/spm/commits/main
