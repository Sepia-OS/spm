# spm

`spm` is the SepiaOS package manager.

## High-level architecture

`spm` accepts commands for executing a certain use case. The specification of
of each command is described in the following sections.

### Commands

#### `update`

#### `upgrade`

#### `install`

#### `remove`

#### `list`

#### `create`

### Package Index

- `spm` is the package manager client itself. It searches for packages, displays
  package infos, installs or removes packages, updates the package index and
  updates packages if new versions are available.
- `spm-creator` creates the package archive that is published. Usually the tool
  is executed in Release pipeline and the result is then published.

The package index as well as the packages are hosted on Github. The package
index is frequently updated by a recuring action. It scans all package
repositories to check whether new releases are available. If this is the case,
the corresponding entry for the package in the index is updated by adding this
new package version.

The package manager client has a local version of the package index. The package
index is automatically loaded the first time `spm` is started by the user, iff
and only if there is no local package index. Users can update the package index,
update all installed packages if new versions are available, get package infos,
search for packages, install and deleted packages.

In the following sections `spm` and `spm-creator` are described in detail.

## `spm-creator`

## `spm`

## Building the binaries for release

- Two actions are available:
  - `CI` is started everytime a commit/push happens, no matter which branch
    it is.
  - `Release` is manually started by the user. The user must enter the version
    number and a release branch (`rel-<version>`) is created where the release
    build is started on. If the release branch is already existing, no new one
    needs to be created, it just shall be used.
- Replace the version string in the Cargo.toml (`version = "0.1.0-replace-me"`)
  with the release version after creating the release branch
  (e.g. `version = "0.3.2"`) and before building it.
