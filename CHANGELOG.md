# Changelog

All notable changes to this repository are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project will adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once it has something to version.

## [Unreleased]

### Changed

- **A symlink may point at an absolute path, and `create` now checks where one
  lands at all.** Two halves of one bug, found packaging musl. Its loader is
  `lib/ld-musl-aarch64.so.1 -> /usr/lib/libc.so`, spelled absolutely by upstream
  because that is the path every binary on the card names in its `PT_INTERP`.
  `unpack` refused any absolute target outright, on the reasoning that a link
  should point inside the package rather than at wherever it lands - but the
  place it lands *is* the device, and `/usr/lib/libc.so` is `usr/lib/libc.so`
  there. The relative spelling of the same place always passed, so the refusal
  was about punctuation rather than about reach; both now resolve through
  `layout::resolve_link` and are judged by where they end up.
- The other half is worse and was invisible: **`create` checked no symlink
  target at all**, while `unpack` checked every one. So `create` would happily
  build a package that no device would install - one this program made and this
  program then refused, with nothing between the two to say so. Both call the
  same resolver now.

- **A package may now write under `bin/`, `sbin/` and `lib/`, as well as `usr/`
  and `etc/`.** The old pair made two packages the operating system actually
  needs impossible to express: busybox, whose applets declare where they belong
  and which ships `/bin/sh` and `/sbin/init` — a card whose `/bin/sh` does not
  exist cannot run a script — and musl, whose loader lives at
  `/lib/ld-musl-aarch64.so.1` because that path is compiled into every
  dynamically linked binary on the card. The original reasoning, that a package
  writing outside `usr/` was altering the system rather than adding to it, held
  only while everything below the applications was baked into the image.
- The roots are an **allowlist**, not a denylist, because a denylist silently
  permits every directory somebody invents later and this is the code that
  writes into `/` as root. `var/` is the pointed omission: `/var/lib/spm` is
  this program's own database, and a package able to write there could forge an
  install record. `boot/`, `dev/`, `proc/`, `sys/`, `run/`, `tmp/`, `home/`,
  `root/`, `mnt/`, `media/`, `opt/` and `srv/` are out too, each for a reason
  written down in `ARCHITECTURE.md`.
- **`create` no longer refuses a tree because of a `libc.so*` or an
  `ld-musl-*`.** The danger that rule named is real — a second libc or loader on
  a card is a card that stops booting — but the rule was in the wrong place: it
  made the libc unpackageable rather than making a *second* libc unpackageable,
  which is not the same thing once musl is a package. What actually prevents the
  second one is `install`, which refuses to write over a file another record
  claims or a file no record claims at all, so a musl package cannot land on a
  card whose image already carries one and two of them cannot both install.
  `remove` still refuses to take away a package something else depends on. Those
  checks hold however a file is named; the old one only held for three spellings.
- New `src/layout.rs` holds the roots and the reasoning for each. `create` and
  `unpack` both consult it, so the rule enforced when packing and the rule
  enforced when unpacking cannot drift — they could before, being two literals
  in two modules.

### Added

- **Indexes and packages are signed, and a device refuses anything that is
  not.** The digests always protected a download from the network; they never
  protected a device from the source itself, because every digest in an index is
  published by that source. A source that has been taken over could serve a
  malicious package with a digest matching it perfectly. Now it would also need
  a key it does not have.
- **Two layers.** The index is signed by the source, with a key pinned on the
  device by `add-source --key` and unchangeable by anything on the network. Each
  package is signed by whoever published it, with a key that lives in that
  repository's own secrets; the index reports which key that was, and a device
  learns it from an index it has already verified. A package signed by any other
  key is refused even though its signature is perfectly valid.
- The index's signature is checked **before the index is parsed**. Deciding what
  a document says before knowing whether to believe it would be reading an
  attacker's instructions.
- A package's signature covers its **identity as well as its payload** - name,
  version, target and payload digest, bound together - so a signature cannot be
  lifted onto a different package carrying the same files, whether as a
  downgrade or as a package renamed to shadow another.
- **`spm keygen`** makes a keypair, writing the private half readable only by
  its owner and printing the public half. **`spm sign-index`** signs an index and
  writes the signature beside it. **`spm create --sign`** signs a package as it
  is built.
- `spm keygen` with no `--out` puts the private key on stdout and **nothing
  else** - the guidance and the public key both go to stderr - so
  `spm keygen | gh secret set …` stores a key and not a key with four lines
  appended. Found while writing the user guide, which had claimed as much before
  it was true.
- Ed25519, through `ring` - which was already in the tree behind `rustls` and
  already compiled for `aarch64-musl` by the cross-build. Signatures cost no new
  dependency and nothing new that might fail to build for a device.

### Changed

- **A source must now have a key.** `add-source --key` is required and
  `sources.json` has no shape without one: a source nothing can be checked
  against is what this whole change exists to stop. Existing configurations do
  not carry a key, so this is a breaking change to that file - nothing has been
  released yet, so no device has one.
- `metadata.json` gained `public_key` and `signature`, written by
  `create --sign` as `sha256` is - an author leaves them empty. An index entry
  gained `public_key`, the publisher key a package must be signed with.

- **A digest for every installed file**, so `verify` checks contents and not
  only presence. A binary that lost a block to a tired card is now found by
  `spm verify` rather than the next time somebody runs it. A symlink has no
  digest: it has no contents of its own, and hashing what it points at would be
  a digest of somebody else's file.
- The digests are taken **during** the extraction, by a writer that hashes on
  the way past, so an install still makes one pass over a package rather than
  two. A package is 216 MiB on an SD card and reading it back to hash it would
  have roughly doubled what installing costs.
- **The cost, measured rather than estimated:** 176 bytes of record per
  installed file, of which the digest is about 121. For a package the size of
  Helix that is about 1.9 MB of record. That is the price of telling a corrupted
  binary from a sound one, and it was judged worth paying.
- The same mismatch means two things, and where the file lives decides which.
  Under `usr/` the package owns the file, so contents that changed underneath it
  are a fault; under `etc/` the identical change is somebody administering their
  device, and is not. `verify` reports them separately and only the first fails
  the command.

### Changed

- **The installed record's `config` map is now `digests`**, and covers every
  regular file rather than configuration alone - it was always the same fact
  ("the bytes `spm` wrote"), and keeping two maps of the same type for it would
  have been two names for one thing. Nothing has been released yet, so no device
  carries a record in the old shape.

- **`spm verify`**, which re-checks what is installed against the records - one
  package or the whole device. It reads the card and the records and nothing
  else: no network, no index, nothing written, so it is safe to run at any time.
- A missing file and a directory standing where a file should be are faults, and
  the command names them rather than counting them, because a path is something
  you can act on and a tally is not. It exits 6, the verification code, when it
  finds any.
- **An edited configuration file is reported and is not a fault.** It is the
  expected result of administering a device, and a `verify` that failed because
  somebody had configured their card is one nobody would run twice. It is also
  the only way to find out which files you have changed.
- What `verify` does not prove is that a file's *contents* are right: a record
  carries a digest for configuration and nothing else, so a corrupted binary
  under `usr/` looks present and correct. Recording a digest for every installed
  file would close that, at some eleven thousand entries for a package the size
  of Helix - a decision about the record format, kept out of this command and
  written down in the design instead.

- **`remove-source` and `source-info` take a source's name**, not only its URL.
  They were the last commands in the set that made you type a URL where
  everything else takes a name, and pasting an index URL to ask about a source
  you already call `local` was busywork the tool was creating for itself.
- Either spelling works, with no flag to say which: a source name may hold only
  lower-case letters, digits, `-`, `_`, `.` and `+`, so a URL - which needs at
  least a `:` and a `/` for its scheme - can never be read as a name. Every URL
  that worked before still works, so nothing written down stops working.
- The two failures now read as different sentences, because they are: a mistyped
  name says "no source named 'sepiaa'" and an unconfigured URL says "no source
  at 'https://…'". One wording for both was wrong for half the callers.
- `add-source` still takes a URL, deliberately: a source that has not been added
  has no name to be addressed by yet. The name comes from the index it
  publishes, or from `--name`.

- **A package can ship configuration.** `create` and the extraction rules now
  accept a top-level `etc/` beside `usr/`, so a package can carry the defaults
  it needs rather than expecting somebody to write them by hand. The rule was
  widened by exactly one directory: anything else is still refused, by both the
  packing and the unpacking, because a package that writes elsewhere is altering
  the system rather than adding to it.
- **An edit to a configuration file is never lost.** `spm` records the digest of
  each `etc/` file as it wrote it, and every later decision about that file asks
  whether what is on the card still matches. A file nobody touched is a stale
  default and is replaced on upgrade and removed with the package; a file
  somebody edited is theirs, and is never overwritten and never deleted - not by
  an upgrade, not by a removal, and not by the rollback of an install that
  stopped halfway.
- On upgrade the new default is written beside an edited file rather than over
  it, as `<name>.spmnew` - `helix.conf` gains `helix.conf.spmnew`, keeping the
  whole original name so two files differing only by extension cannot collide.
  Both `upgrade` and `remove` name the files they left rather than counting
  them, because a file nobody is told about is a decision nobody will make.
- The digest kept is always of what was shipped, never of the edit, so a file
  stays edited for every upgrade after the first. Recording the administrator's
  own bytes would make the next upgrade believe nobody had touched it.

- The suite runs on the architecture it ships for. A cross-build only proves
  the compiler was willing; CI now executes all 305 tests on `aarch64` as well
  as on the host - under `qemu-user` on the x86_64 runners, which costs little
  because the compile dominates. The device half builds with Rust 1.98.1, the
  version a SepiaOS card carries, so the floor and the newest stable are both
  checked on every commit.
- The static-binary assertion is enforced rather than remembered: every commit
  is checked for an `aarch64` ELF with no interpreter and no shared library, so
  a change that quietly made `spm` need `libgcc_s` fails the build instead of
  failing on a card built `WITH_LLVM=0`.
- A release workflow. Manual dispatch with a version, which is branched from
  `main` into `rel-<version>` and built there, so the released commit still
  exists after `main` moves on. It refuses a commit CI has not passed, refuses
  a version already released, and **reuses** an existing release branch rather
  than refusing it - a run that failed after stamping the version is retried by
  dispatching it again, with nothing to delete by hand.
- The version is no longer a placeholder at release time: `0.1.0-replace-me` is
  written into `Cargo.toml`, `Cargo.lock` and `metadata.json` together, and the
  built binary is then asked what it thinks its version is. A stamp that did not
  reach the artifact fails the release rather than shipping.
- **`spm` ships as a package, made by `spm`.** The release stages the tree as it
  appears on a device - the binary under `usr/bin`, the licence under
  `usr/share/licenses/spm/` - and runs `spm create` on it using the very binary
  being packaged. The package manager is therefore packaged like everything
  else it installs, and the release is the first exercise of `create` on a real
  tree rather than a fixture.
- Releases carry four assets: the package, its `metadata.json` for a source's
  scan to read without downloading anything, `SHA256SUMS` covering both, and the
  bare binary - because a device with no `spm` on it cannot install one with
  `spm`.

- `spm` cross-builds for a device:
  `cargo build --release --locked --target aarch64-unknown-linux-musl` produces
  a **static** `aarch64` binary that needs no interpreter and no shared library
  at all. That is not a preference: a card built `WITH_LLVM=0` carries no
  `libgcc_s`, so a binary that needed one would install perfectly and then
  refuse to start. What is shipped instead asks the card for nothing.
- A [`.cargo/config.toml`](.cargo/config.toml) that makes the cross-build one
  command rather than a container. It points rustc's link step and `ring`'s
  C-and-assembly compilation at the same musl-targeting toolchain the sibling
  repositories already download, so no `cross`, no Docker, and no C compiler of
  its own is needed for the target. The names in it are defaults and the
  environment still wins, because the two vendors that publish such a toolchain
  prefix it differently. A host build is untouched by any of it.
- How to build, in [README.md](README.md): the host commands, what has to be
  installed to cross-build, the `readelf` pair that is the acceptance test, and
  the reminder that a cross-build still needs a *host* C compiler for build
  scripts and proc macros — the trap that broke `Sepia-OS/grit`'s first CI run.

- `spm upgrade`, which moves installed packages onto the newest versions the
  local indexes offer — so it finds nothing an `spm update` did not, which is
  what makes what it will do the same as what `--dry-run` said it would. A
  newer version comes from the source the package was installed from: taking
  one from elsewhere because the name matched would swap a package for a
  different package of the same name, so a source that has been removed offers
  nothing, exactly as `remove-source` warns at the time.
- One package that cannot be upgraded is no longer a reason to leave a device
  unpatched. A package whose new version needs something that cannot be
  satisfied is left where it is and named, everything else still moves, and the
  command finishes non-zero so a script can tell a complete job from a partial
  one — the same shape as `spm update`, for the same reason.
- `spm upgrade <package>` for one package, and `spm upgrade --dry-run` to look
  first.
- `spm remove`, which takes back exactly what was installed and nothing else.
  A file another installed package also claims stays; a file no record claims
  is never touched, because `spm` does not remove what it did not install; and
  a directory goes only once the last thing in it has. A package something else
  still needs is refused, and the packages that need it are named, so what
  would have to go first is on the screen rather than left to be worked out.
- Autoremove, which is the other half of `install` recording *why* a package is
  on the device: anything that came in as a dependency and that nothing
  remaining needs goes with what pulled it in, so removing the head of a chain
  three deep takes all three. A package somebody asked for by name is never
  taken automatically, however unreferenced it looks — which is what stops the
  cascade at anything the user chose.
- `spm remove --dry-run`, which works the set out, prints it, and takes
  nothing.
- An install that did not finish now leaves no trace at all: the rollback
  removes the directories it emptied as well as the files it wrote. That was
  left open when the journal was built, with the note that it belonged to
  `remove` — it does, and both halves share it.
- `spm install`, which puts a package and everything it needs on the device.
  Dependencies are worked out first and brought in at the **oldest** version
  that satisfies them, because a floor is a floor and taking the newest would
  upgrade half a card on the strength of one package asking for something old;
  one already installed at a new enough version is left exactly where it is,
  and one the user had asked for by name stays theirs rather than being demoted
  to a dependency. Packages that need each other in a circle are reported as
  the circle they form rather than looped over, and a dependency that exists
  but is too old names the package that wanted it and the newest there is.
- Two checksums on every install, at two different moments. The package is
  checked against the digest the index carries for it **before it is opened at
  all**, so an archive that was corrupted or substituted on the way is never
  parsed; the `data.tar.gz` inside it is then checked against the digest its own
  `metadata.json` carries, which catches a package rebuilt around a different
  payload even when the archive is perfectly well formed. Either failure exits
  6, and the message says which of the two it was.
- Extraction that refuses rather than trusts. A package writes under `usr/` and
  nowhere else; paths are relative and stay inside the device's root; only
  regular files, directories and symbolic links, so no device node and no hard
  link to `/etc/shadow`; a link's target is checked the same way a path is;
  permissions come from the archive and **ownership never does** — restoring
  the uid a package was built under is what broke helix's CI. Nothing is
  followed: a symbolic link at a destination, or standing in for one of the
  directories on the way to it, is refused and never written through.
- `install` refuses rather than overwrites, before a single file is written. A
  file another package owns is a conflict naming both; a file nothing owns came
  from the system image or from somebody's hand, and adopting it would mean
  `remove` later deleting something `spm` never installed. Both exit 7.
- An install that does not finish leaves nothing half-done. The record is
  written before the first file and renamed into place after the last, so the
  next command finds the unfinished one, removes what it had written and starts
  clean — leaving alone any file a package that *did* finish still owns, which
  is what an interrupted upgrade would otherwise take away from the version
  still running.
- `spm install --dry-run`, which works the whole set out, prints it with what it
  would download, and touches nothing — not a file, not a record, not the
  network. `install` itself does not stop to ask; `--dry-run` is how to look
  first. `--version` installs a particular version, including an older one than
  the one installed, which the plan names as a downgrade.
- A check that the card has room, before anything is fetched. A package is on
  it twice while it installs — the download and the unpacked files — so an
  install needs about twice what it downloads, and one that would not fit is
  refused with what is needed and what is free rather than filling the root
  filesystem finding out.
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
- `rustix`, for the one thing `std` has no equivalent of: how much room is left
  on a filesystem. `tempfile` already pulls it into the tree, so it adds nothing
  to the build, and it wraps `statvfs` safely — which is what keeps the promise
  that this crate contains no `unsafe` at all.
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

- `upgrade` is not a second install path. `install` is split at the seam the
  design named — working out what a resolved set amounts to, and then carrying
  it out — and `upgrade` works out its own set and joins there, so the room
  check, the two digests, the extraction rules and the journal have exactly one
  implementation between the two commands.
- A package that came in as a dependency stays one when it is upgraded.
  Promoting it because it moved would quietly take it out of autoremove's reach
  for ever, so dependency resolution now takes each root together with the
  reason it is to be recorded under: `install` makes what you asked for
  explicit whatever it was before, and `upgrade` must not.
- **An index entry now carries the size of the package**, as `bytes`, and one
  written without it is refused. Both things that need a package's size happen
  before it is fetched — the download total `spm install` prints, and the check
  that there is room for it — and it was in neither the index nor anywhere else
  a device can see. A source's scan reads it off a release listing without
  downloading anything, which is the property the index format is built around,
  so it costs a source nothing.
- The user guide showed `spm install` and `spm remove` asking `Proceed? [Y/n]`,
  which they do not. Neither the architecture nor the design ever specified a prompt, and one
  without a `--yes` would make `install` unusable from a script — including from
  the release workflow that will publish `spm` as a package. `--dry-run` is how
  to look before installing, and the guide now says so.
- Installing an older version than the one on the device is now settled: it
  replaces it, and the plan names it as a downgrade. It had been left open in
  both the design and the plan, and any behaviour settles it.
- `spm update`, `spm add-source` and `spm remove-source` now take back an
  unfinished install before they do anything else, which is what the record
  format has always asked of every command that writes and nothing had done yet.
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
