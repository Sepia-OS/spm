# `spm` Architecture

`spm` accepts commands, each of which executes a certain use case. It is
possible to configure more than one package source, each of which publishes an
index of the packages it offers. The specification of each command is described
in the following sections.

## What a source is

A source is a git repository that publishes an index of packages, and that
keeps the index up to date itself.

It does so by scanning. A recurring action walks the sibling repositories
alongside it — the other repositories of the same organisation — and takes an
interest in those tagged `package`. For each of them it reads the releases, and
for every release it does not already have it adds that version to the index.
Nothing is entered by hand.

Two things follow, and they are the reason for arranging it this way:

- **A repository joins a source by being tagged `package`.** There is no
  separate act of registering it, and nothing to keep in step with the list of
  repositories, because the list of repositories *is* the list.
- **A version is published by cutting a release.** A package repository
  publishes in the way it already publishes everything else, and the index
  catches up on its own — promptly if it is told, and on the next scan
  regardless.

### Being told, and finding out

Scanning on a timer is what makes an index **correct**. A notification is what
makes it **prompt**.

When a package repository publishes a release, it tells its index. It finds it
by tag, the same mechanism in the other direction: the sibling repository
marked `package-index`. Neither side has the other's name written down
anywhere — a package repository is tagged `package`, an index repository is
tagged `package-index`, and each finds the other by looking. A repository that
is tagged after its release was cut is picked up without anyone editing
anything.

**The index does not take the metadata from the notification.** It goes and
reads the release, exactly as the scan does. The event says *look now*, and
nothing further: an event can be sent by anyone able to send one, whereas a
release in a repository the index already scans is a thing the index already
trusts. Letting the event carry the metadata would make the index believe
whoever shouted loudest.

So both paths arrive at the same place, and the notification is only ever an
optimisation. One that is lost — the index was down, a token had expired, the
repository was tagged after the fact — costs promptness and nothing else,
because the next scan finds the release anyway.

On GitHub the event is a `repository_dispatch` to the index repository, and it
needs a token that can reach it: a workflow's own `GITHUB_TOKEN` is scoped to
the repository it runs in and cannot dispatch to another one.

## What the index holds

The index carries the metadata of every package it lists, so that the client
never has to download a package to find out what it is. Searching, comparing
versions, reading a description and working out what a package depends on are
all answered out of the index alone. Each entry holds the package's name and
description, and for every version of it: the target it was built for, its
dependencies and their versions, where the package can be downloaded, how big
it is, and two checksums.

The size is there because both things that need it happen before anything is
fetched: `install` says what it is about to download, and it refuses when the
card has no room rather than filling the root filesystem finding out. A scan
reads it off the release listing, like the rest of this, without downloading
the package.

Both checksums are needed, because they say different things:

- **The checksum of the package** — of the `.tar.gz` as the source published
  it. `install` checks a download against this *before opening it*, so an
  archive that was corrupted or substituted on the way is refused before
  anything is read out of it.
- **The checksum of the payload** — of `data.tar.gz`, which is the one
  `metadata.json` carries. It binds the metadata to the payload it describes,
  so a package rebuilt around a different payload is caught even when the outer
  archive is perfectly well-formed.

None of this is written by the scan. A release publishes three things — the
package, its `metadata.json`, and a `SHA256SUMS` covering the package, exactly
as every other SepiaOS repository already publishes a `SHA256SUMS` beside its
asset — and the scan copies the metadata from the one and the package checksum
from the other. That is what lets a source index hundreds of packages without
downloading any of them.

Because it is built by scanning, an index is never the record of what exists —
the releases are. An index can be thrown away and rebuilt from the repositories
at any time, and a source that has fallen behind catches up on its next scan
rather than needing to be repaired.

A device may have several sources configured, each its own repository with its
own scan. One of them is the default, which is the source a package is created
and published against; `add-source --default` is what sets it.

## State on the device

Every command below is described in terms of three places, so they are named
here once:

- **`/etc/spm/sources.json`** — the configured package sources, a name and a
  URL each. It is the only one of the three a person edits by hand.
- **`/var/lib/spm/`** — what `spm` knows. `index/<source>.json` is the local
  copy of one source's index, written by `update`. `installed/<name>.json` is
  the `metadata.json` of an installed package, together with the list of files
  it put on the device, the source it came from, and whether it was asked for
  or pulled in as a dependency.
- **`/var/cache/spm/`** — packages that have been downloaded. Nothing in it is
  needed twice, so it can be emptied at any time.

A package counts as installed if and only if `/var/lib/spm/installed/` records
it. A file that no record claims does not belong to `spm`, and `spm` never
removes or overwrites one.

Commands that read an index read the **local** copy. None of them fetches an
index on its own — that is what `update` is for — so a command can only be as
current as the last `update`.

## Commands

### `update`

The `update` command iterates over all registered package sources, loads their
package indexes and stores them locally. An index is put into place only once
it has been fetched and parsed in full, so an interrupted `update` leaves the
previous copy rather than half a file.

A source that cannot be reached is reported and the rest are still updated, but
`update` finishes with a non-zero status: the picture it leaves is incomplete,
and a script that carries on regardless should have to say so deliberately.

The `update` command supports the following options:

- `--all`: Loads and stores the indexes from all package sources. This is what
  happens when neither option is given.
- `--source <name>`: Loads and stores the index of the given source. `--all`
  and `--source` can't be used together. If both are given, the client stops
  with an error message.

### `search <package name>`

This command searches for a package whose name is given as a parameter. It
searches in the indexes of all sources. If a package is provided by more than
one source, it shall be listed once per source, using the pattern
`<source>/<package name>`. The user must then give the package name in that
pattern to install it.

The parameter matches anywhere in a package name and is not case-sensitive, so
a partial name finds what the user half-remembers. Each match is one line: the
name, the newest version the index offers, the source, and the first line of
the description. A package that is installed is marked as such, with its
installed version if that is not the newest one.

Finding nothing is not an error in itself, but `search` says so and exits
non-zero, so that a script can tell "no such package" from "here it is".

### `info <package name>`

Loads the metadata of the package whose name is given as a parameter and
displays it. If a package is provided by more than one source, the metadata of
each is loaded and displayed, using the pattern `<source>/<package name>` for
the package name.

What is displayed is what `metadata.json` holds — name, version, target,
description, dependencies and their versions, and the SHA-256 of the payload —
together with what only the client knows: which source it came from, whether it
is installed and at which version, and which other versions the index offers.

- `--version <version>`: Displays that version rather than the newest one.

### `upgrade [<package name>]`

The `upgrade` command iterates over all installed packages and checks whether
new versions of them are available. If there are any, they are downloaded and
installed. Given a package name, only that package is considered.

Because it reads the local indexes, `upgrade` finds nothing that the last
`update` did not. It works out the whole set first — the packages to replace,
and any new dependencies their new versions bring with them — and shows it
before changing anything.

A package whose new version needs a dependency that cannot be satisfied is left
at the version it has, and reported. The rest are still upgraded, and `upgrade`
exits non-zero. One unsatisfiable package is a reason to leave that package
alone, not a reason to leave the device unpatched.

- `--dry-run`: Works out the set and prints it, and changes nothing.

### `install <package name>`

Installs the package whose name is given as a parameter. If a package is
provided by more than one source, it shall be listed once per source, using the
pattern `<source>/<package name>`. The user must then give the package name in
that pattern to install it.

The newest version in the index is installed unless another is asked for. The
dependencies named in `metadata.json` are resolved first, and their
dependencies after that, until the set is complete; they are installed with the
package and recorded as dependencies rather than as packages the user asked
for, which is what lets `remove` clean them up later.

Each package is then downloaded and checked twice before anything is unpacked.
The archive is checked against the checksum the index carries for it, **before
it is opened at all**; `data.tar.gz` is then checked against the `sha256` in
the `metadata.json` inside it. Only then is it unpacked into `/`, and the files
it wrote are recorded.

Configuration files — anything the package ships under `etc/` — are recorded
with the digest of what was written, which is what later tells an untouched
default from one somebody has edited. See **Configuration** below.

`install` refuses rather than overwrites:

- **A file that another package owns** stops the installation, and both
  packages are named. Two packages that disagree about one file is a conflict a
  person has to resolve, not something to settle by whoever ran last.
- **A file that no package owns** stops it too. It was put there by the image
  or by hand, and taking it over silently would mean `remove` later deleting
  something `spm` never installed.

A package already installed at the version asked for is left alone and said so.

- `--version <version>`: Installs that version rather than the newest.
- `--dry-run`: Prints the packages that would be installed, and changes
  nothing.

### `remove <package name>`

Removes the package whose name is given as a parameter. If a package is
provided by more than one source, it shall be listed once per source, using the
pattern `<source>/<package name>`. The user must then give the package name in
that pattern to remove it.

Exactly the files recorded for that package are removed, and nothing else — a
file that another installed package also owns stays, and so does anything `spm`
did not install. A configuration file somebody has edited stays too, and is
named, because it is their work rather than the package's; an untouched one goes
with the package, being only a default nobody wanted. Directories are removed
once they are empty.

Anything that was installed only as a dependency, and that nothing else still
needs, is removed along with it. That is the other half of `install` recording
why a package is there.

`remove` refuses if another installed package depends on the one named, and
lists the packages that do. Removing it would leave them unable to run, and the
user can see from the list what they would have to remove first.

- `--dry-run`: Prints what would be removed, and changes nothing.

### `list`

This command lists all packages from all sources. One line per package: the
name, the newest version, the source, and a marker for a package that is
installed. A package offered by more than one source is listed once per source,
as `<source>/<package name>`. The command supports the following options:

- `--installed`: Only lists the installed packages.
- `--source <name>`: Only lists the packages from the given source — all of
  them, or only the installed ones if `--installed` is also given.

### `add-source <url> --key <key>`

Adds a source: an entry in `/etc/spm/sources.json`, and a local copy of the
index it publishes. This command and the two below it are the alternative to
editing that file by hand.

`--key` is the public key the source's index must be signed with, and it is
required. The index is fetched **and verified against it** before anything is
written, so a mistyped key fails here, where nothing has been kept, rather than
at the next `update` on a device that already trusts it.

`add-source` fetches the index before it writes anything. That establishes the
URL is a source at all, rather than a typo that would surface at the next
`update`; it is where the source's **name** comes from, which is what every
other command uses — `--source <name>`, and the `<source>/<package name>` that
disambiguates a package offered twice; and it leaves the source usable straight
away, without an `update` first.

A URL that is already configured is not an error. The entry is updated rather
than duplicated, which is also how a source that already exists is made the
default.

- `--default`: Makes this the source used when a command needs one and none is
  given — which is what a package is created and published against. Only one
  source can be the default, so this takes the flag from whichever source holds
  it. The first source added becomes the default whether or not the option is
  given, since a lone source is the only one it could be.
- `--name <name>`: Uses this name rather than the one the index declares. It is
  the way out of a collision: two sources are free to call themselves the same
  thing, and `spm` needs the names to differ.

`add-source` refuses:

- **A URL that is not `https`.** An index decides which packages a device
  installs and where it fetches them from, so it is not read over a channel
  that anyone in the way can rewrite.
- **An index it cannot fetch or parse.** Nothing is written, so a failed
  `add-source` leaves the configuration as it was.
- **A name already held by a different URL.** The source holding it is named,
  and `--name` is the way past it.

### `remove-source <name|url>`

Takes the name a source is configured under, or the URL it publishes at.
Neither needs a flag: a name may hold only lower-case letters, digits, `-`, `_`,
`.` and `+`, so a URL — which needs at least a `:` and a `/` for its scheme —
can never be read as one.

Removes a source: its entry in `/etc/spm/sources.json` and its local index copy
under `/var/lib/spm/index/`.

Packages installed from it are **not** touched. Removing a source is not a way
of uninstalling things: they stay installed, keep the source name recorded
against them, and `info` reports that source as no longer configured. What they
lose is upgrades — nothing is left to say that a newer version exists.
`remove-source` says how many installed packages this will apply to before it
does it.

Removing the default source moves the flag rather than dropping it silently: if
exactly one source remains it becomes the default, for the same reason the
first source added is. If several remain there is no default until one is named
with `add-source <url> --default`, and `remove-source` says so.

`add-source` keeps a URL as its argument, and is not an inconsistency: a source
that has not been added has no name to be addressed by yet. The name comes from
the index it publishes, or from `--name`.

### `source-info <name|url>`

Takes a name or a URL, like `remove-source`.

Shows what is known about one source: its name, its URL, whether it is the
default, when its index was last updated, how many packages that index offers,
and how many installed packages came from it.

A source whose index has never been fetched is reported as exactly that, rather
than as a source offering no packages. The two look alike in a listing and mean
opposite things, and only `update` closes the gap.

### Signing

Every download is checked against a digest, and always was. That protects the
bytes from the network — but every one of those digests is published by the
source itself, so a source that has been taken over can publish a malicious
package and a digest that matches it perfectly. A digest says *these are the
bytes somebody meant to send*. A signature says *and that somebody holds this
key*.

**Two layers, answering two different questions.**

- **The index is signed by the source.** The key is pinned in `sources.json`
  when the source is added, with `add-source --key`, and nothing the source does
  afterwards can change it. This is the root of everything: the index is
  believed because it verifies, and the rest is believed because the index said
  so. The signature is fetched from `<index url>.sig` and checked **before the
  index is parsed** — deciding what a document says before knowing whether to
  believe it would be reading an attacker's instructions.
- **A package is signed by whoever published it.** A package repository holds
  its own key in its own secrets and signs what it releases; the index reports
  which key that was, per version. A device learns the key from an index it has
  already verified, then refuses any package not signed by exactly that key. A
  package naming a key of its own choosing would be vouching for itself.

**A key is required.** A source without one is a source nothing can be checked
against, so `add-source --key` is not optional and `sources.json` has no shape
that omits it.

**What a package's signature covers is its identity as well as its payload** —
name, version, target and the payload digest, bound together. Signing the
payload alone would let a signature be lifted onto a different package carrying
the same files: a downgrade, or a package renamed to shadow another.

The algorithm is Ed25519, and the reason is what it did not cost: `ring` was
already in the tree behind `rustls`, and the cross-build already compiled it for
`aarch64-musl`. Signatures arrived with no new dependency and nothing new that
might fail to build for a device.

### `keygen`

Makes an Ed25519 keypair. The private key is written to a file readable only by
its owner, or printed for piping into whatever stores secrets; the public key is
printed, being the thing that has to reach every device that will trust this
source.

### `sign-index <file> --key <file>`

Signs an index, writing `<file>.sig` beside it. The bytes on disk are signed
exactly as they are, because that is what a device will check — not a
re-serialisation of what they parse to. The index is parsed first all the same,
so that signing something that is not an index fails once, here, rather than on
every device that fetches it.

### `verify [<package name>]`

Re-checks what is installed against the records, for one package or for every
one of them. It reads the card and the records and nothing else: no network, no
index, and nothing is written.

For each file a record lists, three questions:

- **Is it there?** A recorded file that is gone is a fault. Something deleted
  it — a hand, a failed operation, a card that lost a block — and the package is
  no longer what it says it is.
- **Is a file still what is at that path?** Records list files and symlinks,
  never directories, so a directory standing where a recorded file should be is
  a fault too.
- **If it is configuration, is it still what `spm` wrote?** Reported, but **not**
  a fault: an edited configuration file is the expected outcome of somebody
  administering the device, and the whole point of the rules in *Configuration*.
  `verify` is where you find out which files those are.

The exit is 0 when nothing is missing and nothing is of the wrong kind, and 6 —
the verification code — when something is. Edited configuration never fails it.

`verify` checks contents, not only presence: a record carries the digest of
every file as it was written, so a binary that lost a block to a tired card is
found here rather than when somebody runs it. A symlink is checked for being
there and for still being a link, and no further — it has no contents of its
own, and hashing what it points at would report on somebody else's file.

The same mismatch means two different things depending on where the file is, and
that is the whole reason the two are told apart. Under `usr/` the package owns
the file, so contents that changed underneath it are a fault. Under `etc/` the
same change is an administrator doing their job, and is not.

### `list-sources`

Lists every configured source with the same information `source-info` gives for
one: name, URL, whether it is the default, when its index was last updated, how
many packages it offers, and how many installed packages came from it. They are
listed by name, and the default is marked.

This is the command that answers "what is this device configured to install
from", and the one that supplies the **names** every other command asks for —
`--source <name>`, and the `<source>/<package name>` a package offered twice is
disambiguated by. `source-info` is the same information for a single source,
which is what is wanted once a listing runs past a screen.

Like everything else that reads state, it reads only what is on the device: a
source whose index has never been fetched is shown as exactly that, rather than
as one offering no packages.

A device with no sources configured is not an error — it is what a freshly
installed device looks like. `list-sources` says so, and names `add-source`.

### `create`

`create` creates the installable package: the archive a device unpacks, and the
metadata the index entry for it is made from. It is the one command here that
is not run by a user on a device — it runs in a package repository's release
pipeline, and what it produces is what that repository publishes.

A package is a `.tar.gz` with exactly two members:

- `data.tar.gz` — the tree that is unpacked on the device.
- `metadata.json` — everything the client and the index need to know about the
  package without unpacking `data.tar.gz` first.

`create` packs a tree that is already laid out the way it is to appear on the
device — `usr/bin`, `usr/lib`, `usr/share/licenses/<package name>/`, `etc/` and
so on.
It builds nothing itself; it packages what a build has already staged. The
command supports the following options:

- `--root <directory>` (required): the staged tree to pack. Everything below it
  becomes `data.tar.gz` as it stands, so the tree *is* the package.
- `--metadata <file>` (default: `metadata.json` in the working directory): the
  metadata described below.
- `--output <directory>` (default: the working directory): where the finished
  package is written.

`create` writes three files there, and those three are what the release
publishes and what a source's scan then reads:

- `<name>-<version>-<target>.tar.gz` — the package.
- `metadata.json` — the metadata that went into it, with the payload's digest
  filled in, so the scan can read it without downloading the package.
- `SHA256SUMS` — covering the package, so the index has a checksum for the
  archive itself and `install` has something to check a download against.

Everything else the package needs to declare is given in `metadata.json` rather
than on the command line, because it is a description of the package rather
than of one invocation of the tool, and it belongs in the package repository
beside the sources it describes:

```json
{
  "name": "helix",
  "version": "25.07.1",
  "target": "aarch64-musl",
  "description": "The Helix editor, with its tree-sitter grammars.",
  "dependencies": [
    { "name": "llvm-runtime", "version": "23.1.0" }
  ],
  "sha256": "",
  "public_key": "",
  "signature": ""
}
```

- `name`, `version`, `target` — what the package is. Together they are the name
  of the file `create` writes, and the identity the index lists it under.
- `description` — one or two sentences, shown by `search` and `info`.
- `dependencies` — the packages that have to be installed alongside this one
  for it to work, each named with the version it needs. A version is read as
  *that version or a newer one*: the index accumulates versions rather than
  replacing them, so a dependency that named an exact version would stop being
  satisfiable the first time the package it names is upgraded. `install`
  installs the dependencies with the package; `remove` has to account for one
  of them still being needed by something else.
- `sha256` — the SHA-256 of `data.tar.gz`. This is the one field `create`
  writes rather than reads: the digest cannot exist before the archive it
  describes does. The author leaves it empty, and `create` fills it in on the
  copy it packs.

### Where a package may write

A package writes under `bin/`, `etc/`, `lib/`, `sbin/` or `usr/`, and nowhere
else on the device.

This was `usr/` and `etc/` alone to begin with, on the reasoning that a package
writing anywhere else was altering the system rather than adding to it. That
reasoning holds only while everything below the applications is baked into the
image, and it made two packages the operating system actually needs impossible
to express: **busybox**, whose applets declare where they belong and which ships
`/bin/sh` and `/sbin/init`, and **musl**, whose loader lives at
`/lib/ld-musl-aarch64.so.1` because that path is compiled into every dynamically
linked binary on the card.

The roots are an allowlist rather than a denylist, because a denylist silently
permits every directory somebody invents later, and this is the one part of the
program that writes into `/` as root. Each is named for a reason:

| | |
|---|---|
| `bin`, `sbin` | the commands the system boots into, busybox's among them |
| `lib` | the dynamic loader, whose path is compiled into every binary |
| `usr` | everything above the base system — the package's own, outright |
| `etc` | defaults an administrator may then edit, below |

What is left out is left out deliberately. **`var`** holds `/var/lib/spm`, this
program's own database: a package able to write there could forge an install
record, or corrupt the one that says what it is allowed to remove. **`boot`**
belongs to `Sepia-OS/boot` and is read by the firmware before anything here
exists. **`dev`, `proc`, `sys`, `run`, `tmp`** are kernel or volatile
filesystems, where nothing installed belongs and nothing written survives to be
removed again. **`home`, `root`, `mnt`, `media`, `opt`, `srv`** are people's
files and mount points.

**This is not what stops two packages fighting over one file**, and it never
was. `install` refuses to write over a file another record claims, or a file no
record claims at all — so a musl package cannot land on a card whose image
already carries a libc, and two of them cannot both install. `remove` refuses to
take away a package something else depends on. Those are the checks; this list
only says which directories are addressable.

### Configuration

Everything a package ships is the package's own: `spm` replaces it on upgrade
and deletes it on remove without asking. `etc/` is the exception, because a
default exists to be changed.

At install time the digest of each `etc/` file is recorded. Every later decision
about that file asks whether what is on the card still matches it:

- **Nothing has touched it.** It is a stale default. An upgrade replaces it; a
  removal takes it away.
- **Somebody edited it.** It is theirs. Nothing overwrites it and nothing deletes
  it — not an upgrade, not a removal, not the rollback of an install that failed
  halfway. On upgrade the new default is written beside it with `.spmnew`
  appended to the whole name, so `helix.conf` gains `helix.conf.spmnew`, and the
  command says which files it left for somebody to look at.

The digest recorded is always of what `spm` shipped, never of the edit, so a
file stays "edited" for every upgrade after the first. There is no merging: `spm`
puts the two versions side by side and the decision stays with the person who
made the edit.

`create` refuses to write a package that could not be installed safely:

- **Everything in the tree has to be under one of the roots above.** A package
  that writes outside them is altering the system rather than adding to it.
- **A licence has to be present** under `usr/share/licenses/<name>/`. A package
  carries somebody else's work, and shipping it without its licence is not
  something this tool should make easy.
- **`metadata.json` has to name a package.** `name`, `version` and `target`
  are required; a package that cannot say what it is cannot be indexed.

There used to be a fourth, refusing any tree holding a `libc.so*` or an
`ld-musl-*` on the grounds that both belonged to the image and a second copy of
either was a device that stopped booting. The danger is real and the rule was in
the wrong place: it made the libc unpackageable rather than making a *second*
libc unpackageable. What prevents the second one is `install` never writing over
a file it does not own, which holds however the file is named.
