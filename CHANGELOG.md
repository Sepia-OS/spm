# Changelog

All notable changes to this repository are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project will adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once it has something to version.

## [Unreleased]

### Added

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
