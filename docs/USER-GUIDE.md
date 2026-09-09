# Using `spm`

`spm` is the SepiaOS package manager. It installs software onto a SepiaOS
device, keeps it up to date, and takes it off again.

> `spm` is still being built. This guide describes the behaviour it is being
> built to; [docs/dev/IMPLEMENTATION-PLAN.md](dev/IMPLEMENTATION-PLAN.md) is
> where you can see how much of it exists so far.

## A fresh device

A newly installed card has no package sources configured. That is not a fault —
there is simply nothing yet to say where packages should come from:

```console
# spm list-sources
No package sources are configured.
Add one with: spm add-source <url>
```

A source is a repository that publishes an index of packages. Add one, and
`spm` fetches its index straight away so the source is usable immediately:

```console
# spm add-source https://sepia-os.github.io/packages/index.json --key <key>
Fetched the index for 'sepia': 24 packages.
Added source 'sepia' (default).
```

The name — `sepia` here — comes from the index itself, and it is the name every
other command uses. The first source you add becomes the default, because a
single source is the only one it could be.

## Finding something to install

`search` looks through the indexes of every source. It matches anywhere in a
package's name and ignores case, so a partial name is enough:

```console
# spm search hel
helix        25.07.1   sepia   The Helix editor, with its tree-sitter grammars.
```

`info` shows everything known about one package:

```console
# spm info helix
Name          helix
Version       25.07.1
Target        aarch64-musl
Source        sepia
Description   The Helix editor, with its tree-sitter grammars.
Dependencies  llvm-runtime >= 23.1.0
Installed     no
Versions      25.07.1, 25.01.1, 24.07.0
```

Both read the copy of the index that is on the device. They never go to the
network, so what they tell you is as current as your last `update` — and no
more.

## Keeping the indexes current

```console
# spm update
sepia: 24 packages (3 new).
```

`update` is the only command that fetches an index. Everything else works from
the local copy, so run it before you go looking for something new. It updates
every source unless you name one:

```console
# spm update --source sepia
```

If one source of several cannot be reached, the others are still updated, the
failure is named, and `spm` exits with a non-zero status — the picture it has
left you with is incomplete, and a script should be able to notice that.

## Installing

```console
# spm install helix
Installed:
  llvm-runtime  23.1.0     (dependency)
  helix         25.07.1
Download: 15.4 MiB.
```

Dependencies are worked out first and installed with the package. `spm`
remembers that `llvm-runtime` came in as a dependency rather than because you
asked for it, which is what lets it clean up after itself later. A dependency
you already have at a new enough version is left exactly where it is.

`install` does not stop to ask. It works the whole set out before it fetches
anything — so a package that cannot be resolved, a file that is already spoken
for, or a card without the room fails before the first byte is downloaded — and
`--dry-run` is how you look first:

```console
# spm install helix --dry-run
The following will be installed:
  llvm-runtime  23.1.0     (dependency)
  helix         25.07.1
Download: 15.4 MiB.
Nothing was changed.
```

Before anything is unpacked, each download is checked twice: the package itself
against the checksum the index carries for it, and then the payload inside it
against the checksum its own metadata carries. A package that fails either
check is not opened and not installed.

To install a particular version rather than the newest:

```console
# spm install helix --version 25.01.1
```

That is also how a package is put back to an older version. Installing a
version you already have does nothing and says so.

### When two sources offer the same package

Package names are only unique within a source. If two sources offer a package
by the same name, `spm` will not guess:

```console
# spm install helix
'helix' is offered by more than one source:
  sepia/helix       25.07.1
  local/helix       25.07.1-dev
Install it by its full name, for example: spm install sepia/helix
```

The `<source>/<package>` form works with `install`, `remove`, `info` and
`search`. `spm list-sources` is where you can look up the source names.

## Upgrading

```console
# spm upgrade
Upgraded:
  llvm-runtime  23.1.0     (dependency)
  helix         25.07.1    (upgrade from 25.01.1)
Download: 15.4 MiB.
```

`upgrade` looks at everything installed and finds the newer versions your
indexes know about — so `spm update` first, or it will find nothing. It works
out the whole set before changing anything, including any new dependencies the
newer versions need.

A newer version comes from the source the package was installed from. If that
source has been removed, there is nowhere for it to be upgraded from, which is
what `spm remove-source` warns about at the time.

If one package cannot be upgraded — its new version needs something that cannot
be satisfied — that package is left at the version it has and named, the rest
are still upgraded, and `spm` exits non-zero. One package that cannot move is
not a reason to leave the whole device unpatched.

```console
# spm upgrade
Upgraded:
  helix  25.07.1    (upgrade from 25.01.1)
Download: 15.4 MiB.
grit stays at 1.2.6 - 1.3.0 is offered and cannot be installed: 'grit' needs …
```

To upgrade just one package, or to look before doing anything:

```console
# spm upgrade helix
# spm upgrade --dry-run
```

## Configuration you have edited

Some packages ship a default configuration file under `/etc`. `spm` remembers
exactly what it wrote there, which is how it can tell a default nobody wanted
from a decision you made.

**If you never touched the file**, an upgrade quietly replaces it with the new
default and a removal takes it away with the package. There is nothing to think
about.

**If you edited it**, `spm` will not overwrite it and will not delete it. On an
upgrade it writes the new default beside yours instead:

```
$ spm upgrade
Upgraded:
  helix     25.07.1 -> 25.09.0
1 configuration file you had edited was left as it is; the new default is beside it:
  /etc/helix.conf.spmnew
```

Your `/etc/helix.conf` is untouched. `/etc/helix.conf.spmnew` is what the new
version would have installed. Compare them, take whatever you want from the new
one, and delete the `.spmnew` when you are done:

```sh
diff /etc/helix.conf /etc/helix.conf.spmnew
rm /etc/helix.conf.spmnew
```

Nothing goes wrong if you leave it there — it is an ordinary file and nothing
reads it — but the next upgrade will write over it with a newer default, so it
is worth dealing with while you remember what it was about.

Removing a package tells you the same thing:

```
$ spm remove helix
Removed:
  helix     25.09.0
41 files removed.
1 configuration file was edited since it was installed, and is left in place:
  /etc/helix.conf
```

That file is yours now. If you reinstall the package later, `spm` will refuse
rather than write over it, because it belongs to nobody — delete it first if you
want the package's default back.

## Removing

```console
# spm remove helix
Removed:
  helix         25.07.1
  llvm-runtime  23.1.0     (no longer needed)
418 files removed.
```

Exactly the files that were installed are removed, and nothing else. A file
that another installed package also owns stays. Anything `spm` did not install
is never touched, and a directory goes only once the last thing in it has.

Anything that came in as a dependency and is not needed by anything else goes
with it. A package you asked for by name is never removed automatically,
however unused it looks — which is why `spm install` records whether you asked
for something or whether it came along.

Like `install`, `remove` does not stop to ask, and `--dry-run` is how to look
first:

```console
# spm remove helix --dry-run
The following will be removed:
  helix         25.07.1
  llvm-runtime  23.1.0     (no longer needed)
418 files removed.
Nothing was changed.
```

If something else still depends on what you are removing, `spm` refuses and
tells you what:

```console
# spm remove llvm-runtime
spm: 'llvm-runtime' is required by helix 25.07.1 - remove those first, or leave it in place
```

## Seeing what is installed

```console
# spm list --installed
helix         25.07.1   sepia
llvm-runtime  23.1.0    sepia
```

Without `--installed`, `list` shows everything every source offers, marking
what is installed. `--source <name>` narrows it to one source.

## Checking a device

`spm verify` re-reads what is on the card and compares it against the records:

```console
$ spm verify
  helix     25.07.1   412 files, all present
  grit      0.5.0     38 files, all present
Everything installed is present and is what the records say.
```

It touches the network for nothing and writes nothing, so it is safe to run at
any time. Give it a package name to check just that one.

When something is wrong it names the files, because a count is not something you
can act on:

```console
$ spm verify
  helix     25.07.1   412 files, 1 wrong
      /usr/bin/hx  - missing
spm: 1 file of 1 installed package is missing or no longer a file
$ echo $?
6
```

A file goes missing when something took it — a hand, or an operation that did not
finish. Reinstalling the package puts it back.

**An edited configuration file is not a fault**, and `verify` says so rather than
failing:

```console
$ spm verify
  helix     25.07.1   412 files, 1 edited
      /etc/helix.conf  - edited since it was installed
Everything installed is present. 1 configuration file is edited, which is not a fault.
```

That is the expected result of configuring a device, and `verify` is the only
command that will tell you which files you have changed.

`verify` checks contents, not just that files are there. `spm` records the
checksum of every file as it installs it, so a program that was quietly
corrupted — a card that lost a block, most likely — is found here rather than the
next time you run it:

```console
$ spm verify
  helix     25.07.1   412 files, 1 wrong
      /usr/bin/hx  - contents are not what was installed
spm: 1 file of 1 installed package is missing or no longer a file
```

Reinstalling the package puts it right.

A symlink is checked for being there and still being a link, and no further: it
has no contents of its own.

## Signing

Every source has a key, and it is not optional. `spm` checks that the index it
fetches was signed by the key you pinned when you added the source, and that
each package was signed by the key the index names for it. Without that, a
source that somebody else takes over can hand your device anything at all — the
checksums would match, because whoever published the package also published the
checksums.

Adding a source therefore means having its public key:

```console
# spm add-source https://packages.sepia-os.org/index.json --key 3a7f…c21b
```

Get the key from whoever runs the source, by some route that is not the source
itself — a key fetched from the thing it is supposed to protect protects
nothing. If the key is wrong, `add-source` fails and nothing is written.

### Making a keypair

`spm keygen` makes one. There are two halves and they are treated completely
differently, so it is worth being clear about which is which before you have
one.

```console
$ spm keygen --out source.key
Private key written to source.key
Keep it where only the build that signs can reach it - a repository's
secrets, not the repository. Anybody holding it can sign as you.

Public key (this is what goes on a device, and is not a secret):
  d10ede8cb4f30cb2a291bf1298056d5251415ef8e4015981ceba43c81228aa59
```

| | |
|---|---|
| **Private key** | The file it wrote. It signs. Anybody who has it can sign as you, so it never leaves the machine or the secret store that signs with it. |
| **Public key** | The line it printed. It verifies, and only verifies. It is not a secret — it is meant to be copied about, pasted into issues, and typed onto devices. |

`--out` writes the private key with permissions that let nobody but its owner
read it (`-rw-------`), and it sets them before the file has its final name, so
there is no moment when it is readable and shouldn't be.

Without `--out` the private key goes to standard output instead, for piping
straight into something that stores secrets without it ever touching a disk:

```console
$ spm keygen | gh secret set SPM_SIGNING_KEY
```

The guidance text goes to standard error in that form, so only the key itself
ends up in the pipe.

### Where to keep each half

**The private key belongs in a secret store, not in a repository.** For a
package built by GitHub Actions, that means a repository secret — the same place
a token would go — read into the job that signs and nowhere else:

```yaml
- name: Sign and package
  env:
    SPM_SIGNING_KEY: ${{ secrets.SPM_SIGNING_KEY }}
  run: |
    printf '%s' "$SPM_SIGNING_KEY" > signing.key
    spm create --root stage --sign signing.key
    rm -f signing.key
```

Three things worth doing whatever your build system is:

- **Never commit it.** A key in git history is a key you have to replace, even
  if you delete it in the next commit.
- **Never publish it beside what it signs.** A key served from the same place as
  the index it protects protects nothing — anybody who can replace the index can
  replace the key.
- **One key per job, not one key everywhere.** A source signs indexes; a package
  repository signs its own releases. They are different keys held by different
  people, and that separation is the reason a source cannot quietly substitute
  somebody's package.

**The public key is the thing you have to distribute**, and how it reaches a
device matters more than it looks. Put a source's public key in its README, its
release notes, an announcement — anywhere that is *not* the source itself.
Somebody adding the source types it in:

```console
# spm add-source https://packages.sepia-os.org/index.json --key d10ede…aa59
```

A package's public key goes into the index entry the source builds for it, so
devices pick it up from an index they have already verified.

### If you lose a key, or one leaks

**If the private key leaks**, treat everything it signed as suspect and replace
it. Make a new pair, sign the index again with the new key, and tell everyone
the new public key.

**If you lose the private key**, the effect is the same minus the urgency: you
cannot sign anything more until you make a new pair.

Either way, moving a source to a new key is one command on each device — the
same `add-source`, with the new key:

```console
# spm add-source https://packages.sepia-os.org/index.json --key <new key>
```

Re-adding a URL updates the source that is already there rather than adding a
second one, and the new key replaces the pinned one. Until a device does that it
will refuse the newly signed index, which is the pin doing its job: a key
changing is exactly what an attack would look like, so it takes a person to
accept it.

**If you have the private key but have mislaid the public half**, sign anything
with it — `sign-index` prints the public key of whatever key it used:

```console
$ spm sign-index index.json --key source.key
Signed index.json
  signature  index.json.sig
  public key d10ede8cb4f30cb2a291bf1298056d5251415ef8e4015981ceba43c81228aa59
```

### Running a source

With a key made and stored, sign each index you publish and publish the
signature beside it:

```console
$ spm sign-index index.json --key source.key
```

That writes `index.json.sig`. A device fetches both.

### Publishing a package

A package is signed by whoever built it, with its own key rather than the
source's:

```console
$ spm create --root stage --sign package.key
```

The public half goes in the index entry the source builds for you, so a device
knows which key to expect. A package signed by any other key is refused, even
though its signature is perfectly valid — the index is what says who is allowed
to sign this package.

## Managing sources

```console
# spm list-sources
NAME   DEFAULT  UPDATED      PACKAGES  INSTALLED
sepia  yes      2026-09-07         24          2
       https://sepia-os.github.io/packages/index.json
local  no       never               -          0
       https://example.invalid/pkgs/index.json
```

The date is the day the source last rebuilt its index, not the day you last
fetched it. It is shown as a date rather than as "2 hours ago" because a
relative time has to know what the time is now, and on a device that has just
booted that is the one thing that cannot be relied on.

A source shown as `never` updated has an index that has not been fetched yet,
which is not the same as a source offering nothing — run `spm update` and it
will fill in.

`spm source-info <name|url>` shows the same detail for a single source. Either
spelling works — the name is usually shorter, and it is what `list-sources`
shows in the first column:

```console
$ spm source-info local
```

To remove one, the same way:

```console
# spm remove-source local
Removed source 'local'. 0 installed packages came from it.
```

The URL still works everywhere the name does, so nothing you have written down
stops working:

```console
# spm remove-source https://example.invalid/pkgs/index.json
Removed source 'local'. 0 installed packages came from it.
```

`add-source` is the one that still needs a URL, and for a plain reason: a source
you have not added yet has no name to call it by. `spm` takes the name from the
index the source publishes, or from `--name` if you would rather choose it.

Removing a source does **not** uninstall anything. Packages installed from it
stay exactly where they are; what they lose is upgrades, because nothing is
left to tell `spm` that a newer version exists.

### The default source

One source is the default. It is used when a command needs a source and none
was given — in particular when a package is created and published. To move it:

```console
# spm add-source https://sepia-os.github.io/packages/index.json --key <key> --default
```

Adding a source that is already configured updates it rather than adding it
twice, so this is also how you change the default.

## Publishing a package

`create` is the one command not meant for a device. It runs where a package is
built — normally in a repository's release pipeline — and turns a staged
directory tree into a package.

The tree has to be laid out exactly as it will appear on the device:

```
stage/
  usr/bin/mytool
  usr/share/licenses/mytool/LICENSE
```

Beside it, a `metadata.json` describing what you are packaging:

```json
{
  "name": "mytool",
  "version": "1.0.0",
  "target": "aarch64-musl",
  "description": "Does the thing.",
  "dependencies": [
    { "name": "llvm-runtime", "version": "23.1.0" }
  ],
  "sha256": "",
  "public_key": "",
  "signature": ""
}
```

Leave `sha256` empty — `create` fills it in, because the digest it holds cannot
exist until the payload has been written.

```console
$ spm create --root stage --metadata metadata.json --output dist
dist/mytool-1.0.0-aarch64-musl.tar.gz
dist/metadata.json
dist/SHA256SUMS
```

Publish all three as release assets. Tag the repository `package`, and the
index picks it up — on notification if it can, and on its next scan regardless.
There is nothing else to register.

`create` will refuse to build a package that could not be installed safely:
everything must be under `bin/`, `etc/`, `lib/`, `sbin/` or `usr/`, there must
be a licence under `usr/share/licenses/<name>/`, and the metadata must name the
package.

Most packages only ever want `usr/`, and `etc/` for a default somebody may
edit. The other three are for the base system: `bin/` and `sbin/` for a package
whose commands the device boots into, and `lib/` for the dynamic loader, whose
path is compiled into every binary on the card. Anywhere else — `var/`, `opt/`,
`home/`, `boot/` — is refused, because it belongs to this program's own
database, to the image, or to whoever runs the device.

A dependency's version means *that version or a newer one*, so name the oldest
version your package actually works with.

## When something goes wrong

**"certificate is not yet valid", or TLS failures on a device that has just
booted.** A Raspberry Pi has no battery-backed clock, so until the network is
up and the time has been set, the device thinks it is 1970 — and every
certificate on earth begins later than that. Set the clock and try again:

```console
# sepia-time sync
```

**"checksum mismatch".** The package that arrived is not the package the index
describes. Usually a truncated or corrupted download: run the command again. If
it happens repeatedly for the same package, the source has published something
that does not match its own index, and the package should not be installed.

**"would overwrite a file owned by …".** Two packages disagree about one file,
and `spm` will not decide for you which should win. Remove one of them, or take
it up with whoever publishes them.

**"would overwrite a file that no package owns".** The file was put there by
the system image or by hand. `spm` will not adopt it, because removing the
package later would then delete something `spm` never installed. Move the file
aside if you want the package to own it.

**An interrupted install.** Pull the power out mid-install and nothing is left
half-done: the next `spm` command finds the unfinished record, removes the
files it had written, and starts from a clean state.

**"not enough space".** A package is on the card twice while it installs —
the download and the unpacked files — so it needs roughly twice its size free.
Clearing the download cache is safe at any time:

```console
# rm -rf /var/cache/spm/*
```

## Exit codes

For scripting:

| code | meaning |
|---|---|
| 0 | success |
| 1 | a general failure |
| 2 | wrong usage |
| 3 | not found — no such package, source, or version |
| 4 | ambiguous — the name is offered by several sources |
| 5 | a network or TLS failure |
| 6 | a checksum or verification failure |
| 7 | a conflict — a file owned by another package, or a dependent blocking a removal |
| 8 | incomplete — `update` reached some sources but not all |

`search` finding nothing exits 3, so a script can tell "no such package" from
"here it is".

## Where things are kept

| path | |
|---|---|
| `/etc/spm/sources.json` | the sources you have configured — the one file here you may edit by hand |
| `/var/lib/spm/index/` | the local copy of each source's index |
| `/var/lib/spm/installed/` | what is installed, which files each package owns, and why it is there |
| `/var/cache/spm/` | downloaded packages; safe to delete at any time |

Packages only ever write under `/usr`. Nothing `spm` installs can place a file
in `/etc`, `/var` or your home directory, and a package contains files and
nothing else — there are no install scripts, so installing a package cannot run
a program as root.
