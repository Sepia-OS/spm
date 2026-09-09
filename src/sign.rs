/*
  sign.rs

  Created on 2026-09-09 by Thomas Bonk <thomas@meandmymac.de>
  Copyright 2026 SepiaOS Development Team

  Licensed under the Apache License, Version 2.0 (the "License");
  you may not use this file except in compliance with the License.
  You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

  Unless required by applicable law or agreed to in writing, software
  distributed under the License is distributed on an "AS IS" BASIS,
  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
  See the License for the specific language governing permissions and
  limitations under the License.
*/

//! Ed25519 keys and signatures, and the two things `spm` signs.
//!
//! **What the digests could not do.** Every download is already checked against
//! a digest, and that protects the bytes from the network - but every one of
//! those digests is published by the source itself, so a source that has been
//! taken over can publish a malicious package and a digest that matches it
//! perfectly. A digest says "these are the bytes somebody meant to send"; a
//! signature says "and that somebody holds this key".
//!
//! **Two layers, and they answer different questions.**
//!
//! - **The index is signed by the source**, with a key pinned in
//!   `sources.json` when the source was added. The device trusts that key
//!   because a person put it there, and nothing the source can do afterwards
//!   changes it. This is the root: everything else is trusted because the index
//!   said so, and the index is trusted because it verifies.
//! - **A package is signed by whoever published it**, with a key that lives in
//!   that package repository's own secrets. The index carries the matching
//!   public key, so the device learns which key to expect from a document it
//!   has already verified. A package therefore attests to itself: opening it is
//!   enough to know who built it, without asking the source again.
//!
//! **What a package's signature covers is its identity as well as its
//! payload.** Signing the payload digest alone would let a signature be lifted
//! from one package onto another that happened to carry the same files - a
//! downgrade, or a package renamed to shadow something else. So the signed
//! bytes bind the name, the version, the target and the payload digest
//! together, and a signature is worth nothing anywhere but on the package it
//! was made for.
//!
//! **Ed25519, through `ring`.** Chosen for what it does not cost: `ring` is
//! already in the tree - `ureq`'s `rustls` pulls it in and the cross-build
//! already compiles its C and assembly for `aarch64-musl` - so signatures
//! arrived without a new dependency, a new build requirement, or a new thing
//! that might not cross-compile. Keys are 32 bytes and signatures 64, which
//! matters when both travel inside an index a device downloads.

use std::fmt;
use std::result::Result as Result2;

use ring::rand::SystemRandom;
use ring::signature::{self, Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};
use crate::model::metadata::Sha256;
use crate::model::name::{PackageName, Target};
use crate::model::version::Version;

/// What the bytes a package signature covers begin with.
///
/// A version marker, so that a later change to what is signed cannot be made to
/// look like the present one, and a domain marker, so that a package signature
/// can never be mistaken for an index signature by anything that verifies.
const PACKAGE_CONTEXT: &str = "spm-package-v1";

/// The same, for an index.
const INDEX_CONTEXT: &str = "spm-index-v1";

/// Hexadecimal, lower case, of a fixed length - the shape both types below
/// share with [`Sha256`], and for the same reason: two spellings of one value
/// would compare unequal while meaning the same thing.
fn hex_of(text: &str, want: usize) -> Option<Vec<u8>> {
    if text.len() != want * 2 {
        return None;
    }
    if !text
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(want);
    let raw = text.as_bytes();
    for i in 0..want {
        let pair = raw.get(i * 2..i * 2 + 2)?;
        let text = std::str::from_utf8(pair).ok()?;
        bytes.push(u8::from_str_radix(text, 16).ok()?);
    }
    Some(bytes)
}

/// Bytes as the lower-case hexadecimal everything here is written in.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// An Ed25519 public key: what a device pins, and what an index carries.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicKey(String);

impl PublicKey {
    /// How many bytes an Ed25519 public key is.
    pub const BYTES: usize = 32;

    /// Read a key, or `None` if the text is not one.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        hex_of(text, Self::BYTES).map(|_| PublicKey(text.to_owned()))
    }

    /// The key as text, which is how it is written down and pasted about.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key as bytes, for verifying with.
    #[must_use]
    fn bytes(&self) -> Vec<u8> {
        hex_of(&self.0, Self::BYTES).unwrap_or_default()
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for PublicKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result2<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result2<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        PublicKey::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "'{text}' is not an Ed25519 public key: {} lower-case hexadecimal characters",
                PublicKey::BYTES * 2
            ))
        })
    }
}

/// An Ed25519 signature.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Signature(String);

impl Signature {
    /// How many bytes an Ed25519 signature is.
    pub const BYTES: usize = 64;

    /// Read a signature, or `None` if the text is not one.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        hex_of(text, Self::BYTES).map(|_| Signature(text.to_owned()))
    }

    /// The signature as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn bytes(&self) -> Vec<u8> {
        hex_of(&self.0, Self::BYTES).unwrap_or_default()
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result2<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result2<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Signature::parse(&text).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "'{text}' is not an Ed25519 signature: {} lower-case hexadecimal characters",
                Signature::BYTES * 2
            ))
        })
    }
}

/// A private key, held only by whoever signs.
///
/// Never serialised into anything a device reads: it is written to a file by
/// `keygen` and read back by `create --sign`, and that is the whole of its life
/// in this program. It exists on a build machine, in a repository's secrets,
/// and nowhere else.
pub struct PrivateKey {
    pair: Ed25519KeyPair,
}

// `Ed25519KeyPair` deliberately has no `Debug`, and neither should this: the one
// thing that must never end up in a log is the contents of this struct.
impl fmt::Debug for PrivateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateKey(<not shown>)")
    }
}

impl PrivateKey {
    /// Make a new keypair, returning the private key as text to be written down.
    ///
    /// The text is the PKCS#8 encoding in hexadecimal, on one line, because
    /// where it is going is a repository's secret and those hold one line of
    /// text.
    ///
    /// # Errors
    ///
    /// [`Error::Signing`] if the system's random source will not produce a key.
    pub fn generate() -> Result<(String, PublicKey)> {
        let rng = SystemRandom::new();
        let document = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| {
            Error::Signing("the system's random source would not produce a key".to_owned())
        })?;
        let text = to_hex(document.as_ref());
        let key = Self::parse(&text)?;
        let public = key.public();
        Ok((text, public))
    }

    /// Read a private key from the text `generate` produced.
    ///
    /// # Errors
    ///
    /// [`Error::Signing`] if the text is not a key this can use.
    pub fn parse(text: &str) -> Result<Self> {
        let trimmed = text.trim();
        let bytes = hex_of(trimmed, trimmed.len() / 2).ok_or_else(|| {
            Error::Signing(
                "a private key is lower-case hexadecimal, and this is not - check the file holds the key `spm keygen` wrote and nothing else".to_owned(),
            )
        })?;
        let pair = Ed25519KeyPair::from_pkcs8(&bytes).map_err(|_| {
            Error::Signing(
                "that is hexadecimal but not an Ed25519 private key - check the file holds the key `spm keygen` wrote".to_owned(),
            )
        })?;
        Ok(PrivateKey { pair })
    }

    /// The public half, which is what gets published.
    #[must_use]
    pub fn public(&self) -> PublicKey {
        PublicKey(to_hex(self.pair.public_key().as_ref()))
    }

    /// Sign a package: its identity and its payload, bound together.
    #[must_use]
    pub fn sign_package(
        &self,
        name: &PackageName,
        version: &Version,
        target: &Target,
        payload: &Sha256,
    ) -> Signature {
        Signature(to_hex(
            self.pair
                .sign(&package_bytes(name, version, target, payload))
                .as_ref(),
        ))
    }

    /// Sign an index, over exactly the bytes that were published.
    #[must_use]
    pub fn sign_index(&self, index: &[u8]) -> Signature {
        Signature(to_hex(self.pair.sign(&index_bytes(index)).as_ref()))
    }
}

/// What a package signature is taken over.
///
/// The identity and the payload digest, one field per line, in an order that
/// never changes. Not the `metadata.json` itself: that is JSON, and two
/// serialisations of one document differ in whitespace and key order while
/// meaning the same thing - so a signature over it would break for reasons that
/// are not about the package.
fn package_bytes(
    name: &PackageName,
    version: &Version,
    target: &Target,
    payload: &Sha256,
) -> Vec<u8> {
    format!("{PACKAGE_CONTEXT}\n{name}\n{version}\n{target}\n{payload}\n").into_bytes()
}

/// What an index signature is taken over: the context, then the index verbatim.
///
/// Verbatim, because the device verifies the bytes it received rather than a
/// re-serialisation of what it parsed out of them - anything else would be
/// checking a signature against a document that is merely equivalent.
fn index_bytes(index: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(INDEX_CONTEXT.len() + 1 + index.len());
    bytes.extend_from_slice(INDEX_CONTEXT.as_bytes());
    bytes.push(b'\n');
    bytes.extend_from_slice(index);
    bytes
}

/// Check a package's signature against the key the index published for it.
///
/// # Errors
///
/// [`Error::BadSignature`] naming the package, if it does not verify.
pub fn verify_package(
    key: &PublicKey,
    signature: &Signature,
    name: &PackageName,
    version: &Version,
    target: &Target,
    payload: &Sha256,
) -> Result<()> {
    verify(
        key,
        signature,
        &package_bytes(name, version, target, payload),
        &format!("the package {name} {version}"),
    )
}

/// Check an index's signature against the key pinned for its source.
///
/// # Errors
///
/// [`Error::BadSignature`] naming the source, if it does not verify.
pub fn verify_index(
    key: &PublicKey,
    signature: &Signature,
    index: &[u8],
    source: &str,
) -> Result<()> {
    verify(
        key,
        signature,
        &index_bytes(index),
        &format!("the index for '{source}'"),
    )
}

/// The one place a signature is actually checked.
fn verify(key: &PublicKey, signature: &Signature, message: &[u8], what: &str) -> Result<()> {
    signature::UnparsedPublicKey::new(&signature::ED25519, key.bytes())
        .verify(message, &signature.bytes())
        .map_err(|_| Error::BadSignature {
            what: what.to_owned(),
        })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything here is test code, and a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    fn key() -> PrivateKey {
        let (text, _) = PrivateKey::generate().unwrap();
        PrivateKey::parse(&text).unwrap()
    }

    fn package() -> (PackageName, Version, Target, Sha256) {
        (
            PackageName::parse("helix").unwrap(),
            Version::parse("25.07.1").unwrap(),
            Target::parse("aarch64-musl").unwrap(),
            Sha256::parse(&"a".repeat(64)).unwrap(),
        )
    }

    #[test]
    fn a_generated_key_round_trips_through_its_text() {
        let (text, public) = PrivateKey::generate().unwrap();
        let read = PrivateKey::parse(&text).unwrap();
        assert_eq!(read.public(), public);
        assert_eq!(public.as_str().len(), PublicKey::BYTES * 2);
    }

    #[test]
    fn a_key_file_with_whitespace_still_reads() {
        // It comes back from a repository's secret or from `cat`, and a trailing
        // newline is not a reason to refuse.
        let (text, public) = PrivateKey::generate().unwrap();
        let read = PrivateKey::parse(&format!("  {text}\n")).unwrap();
        assert_eq!(read.public(), public);
    }

    #[test]
    fn nonsense_is_not_a_private_key() {
        assert!(PrivateKey::parse("not a key").is_err());
        assert!(PrivateKey::parse("").is_err());
        // Hexadecimal, but not a key.
        assert!(PrivateKey::parse(&"ab".repeat(40)).is_err());
    }

    #[test]
    fn a_package_signature_verifies_and_a_changed_payload_does_not() {
        let key = key();
        let (name, version, target, payload) = package();
        let signature = key.sign_package(&name, &version, &target, &payload);

        verify_package(
            &key.public(),
            &signature,
            &name,
            &version,
            &target,
            &payload,
        )
        .unwrap();

        let other = Sha256::parse(&"b".repeat(64)).unwrap();
        assert!(
            verify_package(&key.public(), &signature, &name, &version, &target, &other).is_err()
        );
    }

    #[test]
    fn a_signature_does_not_carry_to_another_package() {
        // The reason identity is signed and not only the payload: two packages
        // with the same files must not share a signature, or one could be
        // renamed over the other.
        let key = key();
        let (name, version, target, payload) = package();
        let signature = key.sign_package(&name, &version, &target, &payload);

        let elsewhere = PackageName::parse("grit").unwrap();
        assert!(
            verify_package(
                &key.public(),
                &signature,
                &elsewhere,
                &version,
                &target,
                &payload
            )
            .is_err()
        );

        let older = Version::parse("25.07.0").unwrap();
        assert!(
            verify_package(&key.public(), &signature, &name, &older, &target, &payload).is_err()
        );
    }

    #[test]
    fn another_key_cannot_vouch_for_it() {
        let mine = key();
        let theirs = key();
        let (name, version, target, payload) = package();
        let signature = mine.sign_package(&name, &version, &target, &payload);

        assert!(
            verify_package(
                &theirs.public(),
                &signature,
                &name,
                &version,
                &target,
                &payload
            )
            .is_err()
        );
    }

    #[test]
    fn an_index_signature_covers_the_bytes_that_were_published() {
        let key = key();
        let index = br#"{"name":"sepia","updated":1,"packages":[]}"#;
        let signature = key.sign_index(index);

        verify_index(&key.public(), &signature, index, "sepia").unwrap();

        // One byte different anywhere in it.
        let tampered = br#"{"name":"sepia","updated":2,"packages":[]}"#;
        assert!(verify_index(&key.public(), &signature, tampered, "sepia").is_err());
    }

    #[test]
    fn a_package_signature_is_not_an_index_signature() {
        // The context strings keep the two apart, so a signature made for one
        // purpose cannot be presented for the other.
        let key = key();
        let (name, version, target, payload) = package();
        let package_signature = key.sign_package(&name, &version, &target, &payload);
        let message = package_bytes(&name, &version, &target, &payload);

        assert!(verify_index(&key.public(), &package_signature, &message, "sepia").is_err());
    }

    #[test]
    fn keys_and_signatures_are_the_shape_they_say() {
        assert!(PublicKey::parse(&"a".repeat(64)).is_some());
        assert!(PublicKey::parse(&"a".repeat(63)).is_none());
        assert!(PublicKey::parse(&"A".repeat(64)).is_none(), "upper case");
        assert!(PublicKey::parse(&"z".repeat(64)).is_none(), "not hex");
        assert!(Signature::parse(&"a".repeat(128)).is_some());
        assert!(Signature::parse(&"a".repeat(64)).is_none());
    }
}
