# `spm` Design

[ARCHITECTURE.md](ARCHITECTURE.md) says what `spm` does: the commands, the
package format, what a source is. This document says how it is built — the
module layout, the on-disk formats, the algorithms, the dependencies and the
reasoning behind each. Where the two disagree, the architecture wins and this
document is wrong.

## Constraints

Five facts about the machine this runs on decide most of what follows. None of
them is a preference.

- **The device is a Raspberry Pi running musl.** The target is
  `aarch64-unknown-linux-musl`. The smallest supported board, the Zero 2 W, has
  512 MiB of RAM, and a package can be 200 MiB unpacked — the Helix package is
  216 MiB, most of it tree-sitter grammars. Nothing may be held in memory that
  is proportional to a package's size.
- **There is no trust store on the card.** SepiaOS ships no `/etc/ssl`, no CA
  bundle, no `ca-certificates`. Anything verifying a TLS certificate has to
  carry its own roots.
- **There is no clock until the network is up.** A Pi has no battery-backed
  clock, so it boots in 1970 until `sepia-time` sets it. Every certificate on
  earth is "not valid before" a date after that, so TLS fails outright until
  then — this is what stopped `grit` cloning over HTTPS before `sepia-time`
  existed.
- **`spm` runs as root and writes into `/`.** A bug here does not corrupt a
  document, it corrupts the operating system the device boots from.
- **It has to work on a card built with `WITH_LLVM=0`.** The LLVM package is
  what supplies `libgcc_s.so.1` and `libstdc++.so.6`; a dynamically linked
  `spm` would inherit that dependency and stop working on a minimal card. So
  `spm` is **statically linked**, like `grit` and unlike everything else on the
  card. It needs no `dlopen`, so nothing is lost by it.

## Layout

One binary, `spm`, with `create` as a subcommand of it. The architecture
describes `create` as one of `spm`'s commands, and a second binary would
duplicate the metadata types, the archive writer and the hashing for the sake
of a separation nobody asked for.

It is also a **library with a thin binary on top of it**, rather than a binary
alone. An integration test in `tests/` cannot reach inside a binary crate, and
the testing section below asks for tests that drive whole commands, so
everything lives in the library and `main.rs` is the mapping from a result to
an exit code.

```
src/
  lib.rs            the crate: every module below is public from here
  main.rs           argument parsing, dispatch, exit codes
  cli.rs            the clap definitions - one struct per command
  error.rs          Error, and the exit code each variant maps to
  conffile.rs       etc/: which files are the administrator's, and when
  sign.rs           Ed25519 keys, and the two things that are signed
  model/
    version.rs      Version and its ordering
    metadata.rs     Metadata: what is in a package's metadata.json
    index.rs        Index: what a source publishes
    installed.rs    Record: what is installed, its files, and why
    name.rs         PackageName, SourceName, <source>/<package>, and how a
                    source is addressed - by name or by URL
  store/
    config.rs       /etc/spm/sources.json
    db.rs           /var/lib/spm/installed/
    index.rs        /var/lib/spm/index/ - the local copy of each index
    cache.rs        /var/cache/spm/
    atomic.rs       write-then-rename, and the install journal
    lock.rs         the single-writer lock
    space.rs        how much room is left on the card
  net/
    transport.rs    trait Transport - the seam the tests replace
    https.rs        the real one: ureq + rustls + compiled-in roots
    download.rs     stream to disk, hashing as it goes
  ops/
    resolve.rs      names to candidates; dependency resolution
    update.rs       fetch and replace indexes
    install.rs      plan, verify, unpack, record
    verify.rs       re-check what is installed against the records
    remove.rs       reverse-record, autoremove
    upgrade.rs      compute the set, then reuse install
    create.rs       pack a staged tree into a package
    query.rs        search, info, list, list-sources, source-info
    source.rs       add-source, remove-source - the ones that write
  unpack.rs         tar extraction with the safety rules
  ui.rs             output formatting - one place, so it is consistent
```

`ops/` is where policy lives; `store/` and `net/` know nothing about commands.
A command is a function from parsed arguments to a `Result`, so the tests call
the same entry point the CLI does.

## On-disk formats

All three are JSON, because one serialiser is enough and a person may have to
read them on a device with only `vi`.

### `/etc/spm/sources.json`

```json
{
  "sources": [
    { "name": "sepia", "url": "https://…/index.json", "default": true,
      "key": "…" }
  ]
}
```

`name` is unique — that is what makes `<source>/<package>` unambiguous — and at
most one entry has `default: true`.

### `/var/lib/spm/index/<source>.json`

What a source publishes, stored verbatim as fetched, plus nothing. Storing it
unmodified means a fetch is a download and a write, with no transformation step
that could differ between versions of `spm`.

```json
{
  "name": "sepia",
  "updated": 1757260800,
  "packages": [
    {
      "name": "helix",
      "description": "The Helix editor, with its tree-sitter grammars.",
      "versions": [
        {
          "version": "25.07.1",
          "target": "aarch64-musl",
          "url": "https://…/helix-25.07.1-aarch64-musl.tar.gz",
          "bytes": 16148070,
          "sha256": "…",
          "payload_sha256": "…",
          "public_key": "…",
          "dependencies": [ { "name": "llvm-runtime", "version": "23.1.0" } ]
        }
      ]
    }
  ]
}
```

`sha256` is of the package as published; `payload_sha256` is the digest
`metadata.json` carries for `data.tar.gz`. The architecture explains why both
exist; the client checks them at different moments and must not confuse them,
so they are not called the same thing.

`bytes` is the size of the package as published. It is in the index rather than
discovered from the server because the two things that need it — the plan
`install` prints, and the check that the card has room — both happen *before*
anything is fetched.

### `/var/lib/spm/installed/<name>.json`

```json
{
  "metadata": { "…the package's own metadata.json, verbatim…" },
  "source": "sepia",
  "reason": "explicit",
  "installed_at": 1757260800,
  "files": [ "usr/bin/hx", "usr/lib/helix/runtime/grammars/rust.so", "etc/helix.conf" ],
  "digests": {
    "usr/bin/hx": "4a1e…",
    "usr/lib/helix/runtime/grammars/rust.so": "b70d…",
    "etc/helix.conf": "9f2c…"
  }
}
```

`reason` is `explicit` or `dependency`, and it is the whole basis of
`remove`'s autoremove. `files` are relative to `/`, in the order they were
written, so undoing an install is walking the list backwards. Directories are
not listed: they are removed when they empty out.

`digests` holds the digest of each file **as `spm` wrote it** — every regular
file in `files`, not only configuration. A symlink has no entry: it has no
contents of its own, and hashing what it points at would be a digest of somebody
else's file.

The digests are taken **during** the extraction, by a writer that hashes on the
way past, rather than by reading the card back afterwards. A package is 216 MiB
on an SD card and a second pass over it would roughly double what an install
costs.

**It costs what it costs, and the number is measured rather than guessed:** 176
bytes of record per installed file, of which the digest is about 121 and the
path in `files` the rest. For a package the size of Helix — some eleven thousand
files — that is about 1.9 MB of record, 1.3 MB of it these digests. On a 2 GiB
card that is the price of being able to tell a corrupted binary from a sound
one, and it was judged worth paying.

One map, and two readings of it. Which one applies is decided by where the file
is, and that is the next section.

### Configuration files

A file under `usr/` belongs to the package that put it there: `spm` replaces it
on upgrade and deletes it on remove without asking anybody. `etc/` is the one
place where that is wrong, because the whole point of a default is that somebody
may change it.

So every decision about an `etc/` file asks one question first — *is this still
the bytes we wrote?* — by hashing what is on the card and comparing it against
`digests`. Two answers, and the whole policy follows from them:

- **Untouched.** Nobody wanted it, so it is a stale default: replaced on
  upgrade, deleted on remove, exactly like anything under `usr/`.
- **Edited.** Somebody decided something, so it is theirs: never overwritten and
  never deleted. On upgrade the new default is written beside it with `.spmnew`
  appended to the whole name — `helix.conf` becomes `helix.conf.spmnew`, so the
  original name stays legible and two files differing only by extension cannot
  collide — and both the upgrade and the removal name the files they left, since
  a file nobody is told about is a decision nobody will make.

**Only `etc/` gets this protection.** A record now carries a digest for files
under `usr/` too, and a mismatch there means the opposite thing — the package
owns that file, so contents that changed underneath it are damage rather than a
decision. `conffile::may_delete` therefore asks where the file is before it asks
whether it matches; a digest existing is no longer what makes a file somebody
else's.

**The recorded digest is of what was shipped, never of the edit.** Once a file
has been diverted the record keeps the digest it already had, so it stays edited
for every upgrade after that. Recording the administrator's own bytes would make
the next upgrade believe nobody had touched it and overwrite it, which is the
single outcome this exists to prevent.

**The same question is asked at all three places files are deleted** — `remove`
taking a package back, `upgrade` dropping what the new version no longer ships,
and the rollback of an install that did not finish — because an answer that
differed between them would be a way to lose the file at whichever one forgot.
It is one function, `conffile::may_delete`, and all three call it.

A diverted file is recorded too, under its `.spmnew` name, together with the
file it was diverted around. Leaving the latter out of the record would hand the
administrator's file to nobody: `remove` would never reach it and a later install
would refuse to overwrite it.

## Signing

The digest chain has always protected a download from the network. It never
protected a device from the source itself: every digest in an index is published
by the source, so a source that has been taken over can serve a malicious
package with a digest that matches it exactly. Signatures are the layer that
closes it.

**The chain, in the order a device walks it:**

1. **A key is pinned** in `sources.json` when the source is added. A person put
   it there; nothing on the network can change it. Required — a source with no
   key is a source nothing can be checked against.
2. **The index is fetched with its signature** from `<url>.sig`, and the
   signature is checked **before the index is parsed**. Parsing first would mean
   deciding what the document says before knowing whether to believe any of it,
   and every field in it — which packages exist, where they are downloaded from,
   which keys signed them — is something an unsigned index could lie about.
3. **The index names a key per package version**: the publisher's, not the
   source's. A package repository holds its key in its own secrets and signs
   what it releases; the source only reports which key that was.
4. **The package carries its own signature**, over its identity and payload
   digest. `install` checks that the key the package names is the key the index
   named, then that the signature verifies, and only then unpacks.

**Identity is signed, not only the payload.** The signed bytes are a context
marker, the name, the version, the target and the payload digest, one per line.
Signing the payload alone would let a signature be lifted onto a different
package that happened to carry the same files — a downgrade, or a package
renamed to shadow another. The context marker differs between an index signature
and a package signature, so one can never be presented as the other.

**Signed over bytes, not over documents.** An index signature covers the bytes
that were published, and the device verifies the bytes it received rather than a
re-serialisation of what it parsed. Two serialisations of one JSON document
differ in whitespace and key order while meaning the same thing, so a signature
over the parsed form would break for reasons that are not about the index. For
the same reason a package's signature is over the field list above rather than
over its `metadata.json`.

**Ed25519, through `ring`.** Chosen for what it did not cost: `ring` was already
in the tree — `ureq`'s `rustls` pulls it in, and the cross-build already compiled
its C and assembly for `aarch64-musl` — so this arrived with no new dependency,
no new build requirement, and nothing new that might fail to cross-compile. Keys
are 32 bytes and signatures 64, which matters when both travel inside an index a
device downloads.

**A private key is the one secret this program writes.** `keygen` writes it with
mode 0600, set on the temporary file before the rename so it is never briefly
world-readable under its final name. `PrivateKey` has a hand-written `Debug` that
prints nothing, because the one thing that must never reach a log is its
contents.

## Versions

Package versions are upstream versions — `25.07.1`, `1.2.6`, `23.1.0` — and
upstream versions are not semver. `25.07.1` has a leading zero that semver
forbids, and the helix repository publishes tag `25.07.1` for what its own
crate calls `25.7.1`.

So `Version` is a list of dot-separated components, compared left to right:

- Two numeric components compare numerically, so `25.07.1` and `25.7.1` are
  equal and `1.10.0` is above `1.9.0`.
- A numeric component sorts above a non-numeric one, so `1.0` is above
  `1.0-rc1`.
- Two non-numeric components compare as strings.
- A missing component counts as zero, so `1.2` and `1.2.0` are equal.

This is deliberately not semver and does not pretend to understand what a major
version means. It is an ordering, which is all `dependencies` ("that version or
a newer one") and `upgrade` ("is there a higher one") need.

## Flows

### `update`

For each selected source: fetch the index into a temporary file in the same
directory as its destination, parse it completely, then `rename` it into place.
`rename` within a directory is atomic, so a reader either sees the whole old
index or the whole new one. A parse failure discards the temporary file and
leaves the previous index untouched.

Sources are fetched one after another, not in parallel. A device on a phone
tether is the normal case, and four concurrent fetches on a Zero 2 W buy
nothing worth the complexity.

Failures are collected rather than thrown: every source is attempted, each
failure is reported with the source's name, and the process exits non-zero if
any failed.

### `install`

1. **Resolve the name.** Unqualified and offered by one source: that one.
   Unqualified and offered by several: refuse, listing them as
   `<source>/<package>`. Qualified: that source, or an error naming what is
   configured.
2. **Select the version** — the one asked for, else the highest whose `target`
   matches the device's.
3. **Resolve dependencies** breadth-first, gathering the transitive set. For
   each dependency take the lowest version that satisfies "that version or a
   newer one" and is not older than what is already installed. A dependency
   already installed at a satisfying version is not reinstalled. A cycle is
   detected by the visited set and reported rather than followed.
4. **Plan and show.** The set, what is new, what is an upgrade, the total
   download. `--dry-run` stops here and touches nothing at all. Then the room:
   a package is on the card twice while it installs, so an install that would
   need more than is free is refused here, before anything is fetched.
5. **Download** each package to `/var/cache/spm/`, hashing the stream as it is
   written, and compare with the index's `sha256` **before the archive is
   opened**. A mismatch is refused there.
6. **Open** the outer archive, read `metadata.json`, hash `data.tar.gz` and
   compare with its `sha256`. Refuse on mismatch. The payload is read where it
   lies rather than written out first: staging it would put the package on the
   card a third time, on top of the archive and the tree it unpacks to.
7. **Dry-run the extraction**: read the payload's entries and build the file
   list without writing anything, and check every path against the rules in
   *Unpacking*, and against ownership — a path claimed by another package's
   record, or already present on disk and claimed by none, stops the install
   before a single file is written. A path the package's *own* record claims is
   neither: replacing those is what an upgrade is.
8. **Commit.** Write `installed/<name>.json.partial` with the full file list,
   then extract, then rename the record to `.json`. The record exists before
   the files do, so an interrupted install is recoverable in exactly one
   direction. An upgrade then takes away the files the version it replaced had
   put there and the new one does not ship — after the rename, so that a crash
   leaves files that are merely stale rather than files nothing remembers.
9. **Recover, if needed.** Any `.partial` found at startup is an install that
   did not finish: its files are removed and the record deleted, before
   anything else runs. Except a file some committed record claims — an
   interrupted upgrade's journal names the files of the version still installed,
   and taking those back would leave a package whose record says it is whole and
   whose files are gone. There is no half-installed state that survives the next
   invocation.

### `remove`

The reverse, and simpler because the record already says what to do. Refuse if
another record depends on this package. Delete the files in the record's list
in reverse order, skipping any path another record also claims, and remove
directories that have become empty. Then the autoremove pass: repeatedly drop
any `dependency` record that no remaining record depends on, until a pass
changes nothing.

### `upgrade`

Compute per package: the installed version, and the highest in the index for
this target. For each candidate, resolve its dependencies as `install` would.
A package whose dependencies cannot be satisfied is dropped from the set and
reported — not fatal, because the point is to upgrade what can be upgraded.
Then hand the surviving set to `install`'s steps 4 onward.

### `create`

Read the metadata, check the tree against the four refusals, write
`data.tar.gz` while hashing it, fill the digest into the metadata, write the
outer archive containing `data.tar.gz` and `metadata.json`, then write
`metadata.json` and `SHA256SUMS` beside it. Because the digest is only known
after the payload is written, the payload is written first and to a temporary
file — the metadata that goes *into* the archive is not the metadata the author
supplied.

## Unpacking

Extraction is the one place where a hostile or careless package can do real
damage, so it is a single function with its own tests, and every entry is
checked before it is created:

- **The path must be relative and must stay inside the root.** No leading `/`,
  no component equal to `..` — after normalisation, an entry that escapes is a
  refusal, not a clamp.
- **The path must start with `usr/` or `etc/`.** `create` enforces this when
  packing; `install` enforces it again when unpacking, because a package can
  reach a device without having passed through this `create`. `usr/` is the
  package's own; `etc/` is where it ships defaults somebody may then edit, and
  what happens to one of those afterwards is in *Configuration files* above.
- **Only regular files, directories and symlinks.** No devices, no FIFOs, no
  sockets, no hard links — a hard link to `/etc/shadow` is a way to hand out
  its contents.
- **A symlink's target is checked the same way as a path**, so a package cannot
  drop a link pointing at `/etc` and then write "through" it in a later entry.
- **Permissions come from the archive, ownership does not.** Everything is
  written `root:root`. The uid a package was built under is an accident of the
  build machine, and this is the mistake that broke helix's CI: GNU tar as root
  restored `runner:docker` from the archive and git then refused the tree.
- **Nothing is followed.** Files are created with `O_NOFOLLOW` semantics so an
  existing symlink at the destination cannot redirect a write.

## Networking

`ureq` with `rustls`, and **`webpki-roots` compiled into the binary**, because
the card has no trust store to read. A blocking HTTP client with no async
runtime suits a CLI that makes a handful of sequential requests; `reqwest`
would pull `tokio` in for nothing.

- **HTTPS only.** A plain-`http` URL is refused, not upgraded. This mirrors
  what every SepiaOS Makefile already does when resolving a release.
- **Downloads stream to disk** through a hasher, never into memory. A 200 MiB
  package on a 512 MiB board leaves no other option, and it means the digest
  costs nothing extra.
- **Retries** on connection failure and 5xx, three times with a growing delay;
  never on a 4xx, which will not improve.
- **Interrupted downloads are discarded**, not resumed. Resume needs range
  support and a way to know the partial file belongs to the same object; the
  checksum is what tells us we got it right, and a fresh start always passes
  that test.
- **The clock.** A TLS failure that says the certificate is not yet valid is
  almost always a device whose clock has not been set. The error names that
  possibility and points at `sepia-time`, rather than reporting a certificate
  error the user cannot act on.
- **`GITHUB_TOKEN` is used if it is set**, for the same reason the sibling
  Makefiles do: the anonymous GitHub API allowance is small, and a device
  behind a shared address can exhaust it.

## Locking and concurrency

One lock file, `/var/lib/spm/lock`, taken by every command that writes and
never by one that only reads. A second `spm` blocks with a message saying what
holds it — the holder writes its process id into the file once it has the lock,
so a waiter can say what it is waiting for. The kernel releases the lock when
the process dies, so a killed `spm` does not need a stale-lock story.

`std::fs::File::lock`, stable since Rust 1.89, is what takes it. This was
planned as `flock` through `libc`, which would have been the crate's only
`unsafe` block and its only C dependency; std having grown the same thing means
**the crate contains no `unsafe` at all**.

Within a command there is no concurrency at all. The work is dominated by one
download and one extraction, both sequential by nature.

## Errors and exit codes

`error.rs` defines one enum; every variant carries what the user needs to act
and maps to an exit code, so scripts can branch without parsing text:

| code | meaning |
|---|---|
| 0 | success |
| 1 | a general failure |
| 2 | wrong usage — bad arguments, mutually exclusive options |
| 3 | not found — no such package, source, or version |
| 4 | ambiguous — a name offered by several sources |
| 5 | a network or TLS failure |
| 6 | a checksum or verification failure |
| 7 | a conflict — a file owned by another package, or a dependent that blocks a removal |
| 8 | incomplete — `update` reached some sources but not all |

Verification failure has a code of its own because it is the one failure that
may mean something other than bad luck.

## Dependencies

Kept short, and every one of them chosen against the constraint that this
cross-compiles statically to `aarch64-unknown-linux-musl` from a Linux CI
container and a macOS workstation:

| crate | for | why this one |
|---|---|---|
| `clap` (derive) | arguments | The subcommands and their options are a datatype; hand-rolling this is where CLI bugs live. |
| `serde`, `serde_json` | the three formats | One serialiser for all of them. |
| `ureq` | HTTP | Blocking, small, `rustls` without an async runtime. |
| `rustls` + `webpki-roots` | TLS | Roots compiled in — the card has no trust store. No OpenSSL to cross-compile. |
| `sha2` | checksums | Pure Rust; no `libcrypto` on the target. |
| `flate2` (`rust_backend`) | gzip | The Rust backend avoids linking `zlib`; a C dependency is the usual reason a musl cross-build stops working. |
| `tar` | archives | Reading and writing, streaming both ways. |
| `tempfile` | staging | Temporary files in the destination directory, cleaned up on drop. |
| `thiserror` | errors | The enum above, without the boilerplate. |
| `rustix` (`fs`) | free space | `statvfs`, which `std` has no equivalent of. Already in the tree under `tempfile`, and safe, so the crate still has no `unsafe`. |

`Cargo.lock` is committed. `spm` is a binary, its builds have to be
reproducible, and the `.gitignore` this repository started from ignores the
lockfile — a template default that is wrong for an executable and needs
removing.

## Testing

- **Unit tests** for the parts with real logic and no I/O: version ordering
  (including `25.07.1` against `25.7.1`), name resolution, dependency
  resolution over a fixture index, autoremove, and every rejection in
  *Unpacking* against a hand-built malicious tar.
- **`trait Transport` is the seam.** Integration tests inject a transport that
  serves a fixture index and fixture packages out of a temporary directory, so
  the whole of `update`, `install`, `remove` and `upgrade` runs end to end
  against a real filesystem with no network and no HTTPS.
- **Root-relative operations are tested against a temporary root.** Every path
  in `store/` and `unpack.rs` is built from a configurable prefix, defaulting
  to `/`, so a test installs into a directory and inspects the result. That
  prefix is not a user-facing option, but it is what makes the tests possible.
- **The cross-built binary is executed, not only built.** The CI runner is
  x86_64, so the `aarch64-musl` binary runs under `qemu-user-static` in the
  same container; on an Apple Silicon workstation it runs at full speed in a
  `linux/arm64` container. This is how the helix package's runtime behaviour
  was verified, and it is the only way a test says anything about the machine
  the program is for.

## Build and release

The two workflows the README describes, on the pattern the sibling repositories
already use:

- **CI** on every commit and every branch: `cargo fmt --check`, `cargo clippy
  -D warnings`, `cargo test` on the host, then the cross-build for
  `aarch64-unknown-linux-musl` and the test suite again under emulation.
- **Release**, manually dispatched with a version: branch `main` to
  `rel-<version>`, replace `0.1.0-replace-me` in `Cargo.toml`, build, and
  publish the binary with a `SHA256SUMS` beside it.

`spm` packages itself with `create` once it can, which is the first real test
of the format: the package that installs the package manager is a package like
any other.

## Resource budget

- **Memory**: bounded by the index, which is parsed whole. A few hundred
  packages with a few versions each is a few hundred kilobytes. Downloads and
  extraction are streamed, so a 216 MiB package needs no more memory than a
  1 MiB one.
- **Disk**: a package is on the card twice during an install — the archive in
  `/var/cache/spm/` and the unpacked files under `/usr` — so an install needs
  roughly twice the package's size free. `install` checks before downloading
  and says so if it will not fit, rather than filling the root filesystem.
- **Binary size**: a static Rust binary with `clap`, `rustls` and `tar` is a
  few megabytes. That is the cost of `spm` being the one thing on the card that
  cannot depend on anything else.

## Security model

What `spm` trusts, and what it does not:

- **Trusted**: the configured sources, and TLS to them. Adding a source is
  giving it the right to put files on the device.
- **Not trusted**: the network in between — every download is checked against a
  digest that came from the index over TLS. The notification path is not
  trusted at all, and the index reads releases itself rather than believing an
  event; that is the architecture's rule and this document does not soften it.
- **Not executed**: a package contains files, and nothing else. There are no
  maintainer scripts, no hooks, no post-install step, so installing a package
  cannot run code as root. musl has no `ld.so.cache`, so there is nothing a
  shared library needs done after it lands — the usual reason a package manager
  grows a post-install hook does not arise here.
- **Not privileged by default**: the device runs as root, so this is a
  statement about blast radius rather than a boundary. It is the reason
  *Unpacking* is as strict as it is.

## Open questions

- **Nothing signs an index.** The chain of digests protects a download from the
  network; it does not protect the device from a source that has been taken
  over. Signing the index, and pinning a key per source in `sources.json`, is
  the obvious next layer.
