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

**Early.** Every command the specification describes is implemented and
tested, and the binary cross-builds for a device. What is left is shipping it:
[the implementation plan](docs/dev/IMPLEMENTATION-PLAN.md) is the honest
account of how much exists, step by step.

There is no release yet, so nothing reaches a device except by being built from
this repository.

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

## Building

`spm` is an ordinary Rust crate, so building it for the machine you are sitting
at is the ordinary command:

```sh
cargo build              # a binary for this machine
cargo test               # the suite, which touches neither the network nor /
```

A Rust toolchain and nothing else — no system OpenSSL, no `zlib`, no
`pkg-config`. That is a rule rather than luck: a dependency that cannot
cross-compile statically to the device does not go in, which is why the TLS is
`rustls` and the decompression is `flate2`'s Rust backend. The floor is Rust
1.98.1, because that is the version `Sepia-OS/rust-toolchain` puts on a card and
a package manager its own operating system cannot rebuild would be an odd
thing.

### Cross-building for a device

A device is `aarch64` running musl, so that is what a release is built for:

```sh
cargo build --release --locked --target aarch64-unknown-linux-musl
```

Two things have to be in place first:

- **The target**, which is one command: `rustup target add aarch64-unknown-linux-musl`.
- **A musl-targeting `aarch64` cross toolchain on `PATH`.** rustc drives the
  link through a C compiler, and the host's cannot produce aarch64 ELF; `ring`,
  which arrives under `rustls`, additionally compiles C and assembly *for the
  target*. Both jobs go to the same toolchain, named in
  [`.cargo/config.toml`](.cargo/config.toml). It is the one the sibling
  repositories already download: [messense](https://github.com/messense/homebrew-macos-cross-toolchains)
  on a macOS host, [bootlin](https://toolchains.bootlin.com/) on Linux. Without
  it the build stops at ``linker `aarch64-unknown-linux-musl-gcc` not found``,
  which is the whole diagnosis.

A **host** C compiler is needed as well, and it is easy to forget why: build
scripts and proc-macro crates are compiled and run on the build machine, and
rustc links those with plain `cc`. A container carrying only the cross
toolchain fails partway through a dependency with ``linker `cc` not found``.
That is what broke [grit](https://github.com/Sepia-OS/grit)'s first CI run.

The two vendors prefix their toolchains differently — messense spells the
triple out in full, bootlin calls it `aarch64-linux-` — so the names in
`.cargo/config.toml` are defaults rather than requirements. The environment
overrides them:

```sh
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-gcc \
CC_aarch64_unknown_linux_musl=aarch64-linux-gcc \
AR_aarch64_unknown_linux_musl=aarch64-linux-ar \
  cargo build --release --locked --target aarch64-unknown-linux-musl
```

### What comes out

A **static** binary, because `aarch64-unknown-linux-musl` links `crt-static`
by default and that default is wanted here. A card built `WITH_LLVM=0` carries
no `libgcc_s`, so a binary that needed one would install perfectly and then
refuse to start. This one asks the card for nothing at all, which is a stronger
claim than any list of libraries that happen to be present today:

```sh
BIN=target/aarch64-unknown-linux-musl/release/spm
aarch64-unknown-linux-musl-readelf -l $BIN | grep INTERP   # nothing: no interpreter to find
aarch64-unknown-linux-musl-readelf -d $BIN | grep NEEDED   # nothing: no shared library to load
```

Both print nothing, and that is the acceptance test — the same assertion
`grit-check` makes about the `git` that ships beside it.

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
