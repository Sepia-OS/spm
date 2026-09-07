# spm

`spm` is the SepiaOS package manager.

## What it is for

SepiaOS is an operating system for the Raspberry Pi. What a card carries is
decided when the image is built: the editor, the compiler, the filesystem
tools. Adding something afterwards has meant rebuilding the image and writing
a new card.

`spm` is how a running device gets software instead — install a package, remove
one, keep what is there up to date. It is the difference between a card that is
finished when it is written and one that can be added to for as long as it is
in use.

## Status

**Early.** The specification is written and the implementation has not started.
The documents below describe the behaviour `spm` is being built to, not
behaviour it has; [the implementation plan](docs/dev/IMPLEMENTATION-PLAN.md) is
the honest account of how much exists.

Nothing in this repository is ready to install on a device yet.

## Documentation

| | |
|---|---|
| [User guide](docs/USER-GUIDE.md) | For somebody with a device: installing, upgrading, removing, publishing a package, and what to do when something goes wrong. |
| [Architecture](docs/dev/ARCHITECTURE.md) | What `spm` does — every command, what a source is, and the package format. |
| [Design](docs/dev/DESIGN.md) | How it is built — module layout, on-disk formats, algorithms, dependencies, and the constraints behind each. |
| [Implementation plan](docs/dev/IMPLEMENTATION-PLAN.md) | The order the work happens in, step by step. |
| [Development guidelines](docs/dev/DEVELOPMENT-GUIDELINES.md) | The house rules for changing this repository. |

Start with the architecture if you want to know what `spm` is; start with the
user guide if you want to know what using it will feel like.

## How it works, briefly

A **package** is a tree of files with a description of itself — what it is
called, what version it is, and what else has to be installed for it to work.

A **source** is a repository that publishes an index of packages, and that
keeps the index up to date on its own by watching the repositories around it. A
repository joins a source by being tagged, and publishes a new version by
cutting a release. Nobody maintains a list.

A device can be configured with more than one source, and everything it
installs is checked against a checksum the index published before a single file
is written.

## Part of SepiaOS

`spm` is one repository of several. Each builds one part of what ends up on a
card, and each publishes releases the others consume:

| | |
|---|---|
| [boot](https://github.com/Sepia-OS/boot) | The boot partition — firmware, `config.txt`, device trees, the kernel. |
| [rootfs](https://github.com/Sepia-OS/rootfs) | Assembles the root filesystem and the bootable image from everything else. |
| [musl](https://github.com/Sepia-OS/musl) | The C library the card runs on. |
| [llvm](https://github.com/Sepia-OS/llvm) | `clang` and `lld` for the device, and the runtime libraries other packages need. |
| [make](https://github.com/Sepia-OS/make) | GNU make for the device. |
| [e2fsprogs](https://github.com/Sepia-OS/e2fsprogs) | The ext filesystem tools. |
| [wifi](https://github.com/Sepia-OS/wifi) | `wpa_supplicant` and `libnl`. |
| [rust-toolchain](https://github.com/Sepia-OS/rust-toolchain) | `rustc` and `cargo` for the device. |
| [grit](https://github.com/Sepia-OS/grit) | Git, as a Rust implementation, and the `git` command. |
| [helix](https://github.com/Sepia-OS/helix) | The Helix editor and its grammars. |
| **spm** | This one. |

## Contributing

- **The documents come first.** A change in behaviour is a change to
  [the architecture](docs/dev/ARCHITECTURE.md) before it is a change to the
  code, so that what `spm` is meant to do never has to be inferred from what it
  currently does.
- **Every change gets a changelog entry**, under `## [Unreleased]` in
  [CHANGELOG.md](CHANGELOG.md), in the same commit as the change. The format is
  [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/).
- **Read [the development guidelines](docs/dev/DEVELOPMENT-GUIDELINES.md)**
  before the first pull request. They are short, and most of what is in them
  was learned the expensive way somewhere else in SepiaOS.

## Licence

Apache License 2.0. See [LICENSE](LICENSE).
