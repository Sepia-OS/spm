/*
  https.rs

  Created on 2026-09-07 by Thomas Bonk <thomas@meandmymac.de>
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

//! The real transport: HTTPS, and nothing else.
//!
//! Carries its **own root certificates**, because a SepiaOS card has no trust
//! store to read — no `/etc/ssl`, no CA bundle, nothing. `rustls` with
//! `webpki-roots` compiled in is the whole reason this works on a device at
//! all, and it is why the platform verifier is deliberately not used.
//!
//! Two things it does not do, both on purpose:
//!
//! - **It does not accept `http://`.** An index decides which packages a
//!   device installs and where it fetches them from, so it is not read over a
//!   channel anyone in the way can rewrite. A plain URL is refused before a
//!   connection is opened, not upgraded to `https` behind the user's back.
//! - **It does not ask for compressed transfers.** `ureq`'s `gzip` feature is
//!   off, so what arrives is what was published. Everything here is verified
//!   against a digest of the file, and a transfer encoding is one more thing
//!   that can differ between what a server sent and what a digest describes —
//!   `curl`, which every sibling repository uses, does not ask for one either.

use std::io::Read;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ureq::Agent;
use ureq::config::Config;

use crate::error::{Error, Result};
use crate::net::transport::Transport;

/// How long to wait for a connection, and for the whole exchange.
///
/// A device on a phone tether is the normal case, so the overall limit is
/// generous; what it is there to stop is a command that hangs forever on a
/// server that accepted the connection and then said nothing.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(300);

/// How many times to try, and how long to wait between tries.
pub const ATTEMPTS: u32 = 3;
const BACKOFF: Duration = Duration::from_millis(500);

/// Where the image records when it was built, in seconds since the epoch.
///
/// `rootfs` writes it, and `sepia-time` uses it to move a just-booted clock
/// forward before the network is up. It is the best available answer to "what
/// is the earliest the time could possibly be".
const BUILD_DATE: &str = "/etc/sepia-build-date";

/// The floor when there is no build date to read — a workstation, or a card
/// built before that file existed. Any clock earlier than this is not a clock
/// anybody set.
///
/// 2025-01-01.
const ASSUMED_FLOOR: u64 = 1_735_689_600;

/// The real transport.
#[derive(Debug)]
pub struct Https {
    agent: Agent,
    /// How long to wait between attempts. Zero in the tests, so that a test
    /// for the retry behaviour does not spend seconds sleeping.
    backoff: Duration,
    /// What the device believes the time is. Set in the tests; otherwise read
    /// when it is needed, which is only when something has already failed.
    now: Option<u64>,
    /// The earliest the time could be.
    floor: u64,
}

impl Https {
    /// A transport with the timeouts and retries described above.
    #[must_use]
    pub fn new() -> Self {
        Https {
            now: None,
            floor: floor_from(Path::new(BUILD_DATE)),
            agent: Agent::new_with_config(
                Config::builder()
                    .timeout_connect(Some(CONNECT_TIMEOUT))
                    .timeout_global(Some(RESPONSE_TIMEOUT))
                    .user_agent(concat!("spm/", env!("CARGO_PKG_VERSION")))
                    .build(),
            ),
            backoff: BACKOFF,
        }
    }

    /// The same, but without the waiting between attempts.
    #[must_use]
    pub fn without_backoff() -> Self {
        Https {
            backoff: Duration::ZERO,
            ..Https::new()
        }
    }

    /// The same, with the clock told what to believe.
    ///
    /// For the tests. The process clock cannot be moved, and the message this
    /// step exists to produce depends entirely on what the clock says.
    #[must_use]
    pub fn with_clock(now: u64, floor: u64) -> Self {
        Https {
            backoff: Duration::ZERO,
            now: Some(now),
            floor,
            ..Https::new()
        }
    }

    /// What this device believes the time is.
    fn now(&self) -> u64 {
        self.now.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs())
        })
    }
}

impl Default for Https {
    fn default() -> Self {
        Https::new()
    }
}

impl Transport for Https {
    fn get(&self, url: &str) -> Result<Box<dyn Read>> {
        // Before anything is opened. A refusal that happened after a request
        // would already have told somebody what this device is looking for.
        if !url.starts_with("https://") {
            return Err(Error::Network {
                url: url.to_owned(),
                message: "only https is read - an index decides what a device installs, so it is not fetched over a channel that anyone in the way can rewrite".to_owned(),
            });
        }

        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut request = self.agent.get(url);
            if wants_token(url)
                && let Ok(token) = std::env::var("GITHUB_TOKEN")
                && !token.is_empty()
            {
                request = request.header("authorization", &format!("Bearer {token}"));
            }

            match request.call() {
                Ok(response) => {
                    let status = response.status().as_u16();
                    // 4xx will not improve by asking again.
                    if (500..600).contains(&status) && attempt < ATTEMPTS {
                        wait(self.backoff, attempt);
                        continue;
                    }
                    return Ok(Box::new(response.into_body().into_reader()));
                }
                Err(error) => {
                    if retryable(&error) && attempt < ATTEMPTS {
                        wait(self.backoff, attempt);
                        continue;
                    }
                    return Err(self.translate(url, &error));
                }
            }
        }
    }
}

/// Whether asking again could give a different answer.
fn retryable(error: &ureq::Error) -> bool {
    match error {
        // The connection did not happen, or died. Worth another go.
        ureq::Error::Io(_) | ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed => true,
        // A status this side treats as an error - 4xx and 5xx both arrive here
        // when the agent is configured to return them as errors.
        ureq::Error::StatusCode(status) => (500..600).contains(status),
        // A refusal, a redirect loop, a bad certificate: asking again gives
        // the same answer more slowly.
        _ => false,
    }
}

/// Wait a little longer each time.
fn wait(backoff: Duration, attempt: u32) {
    if backoff.is_zero() {
        return;
    }
    std::thread::sleep(backoff.saturating_mul(attempt));
}

impl Https {
    /// Turn a `ureq` failure into one of ours.
    ///
    /// A certificate failure on a device whose clock has not been set is
    /// almost never a certificate problem. A Raspberry Pi has no
    /// battery-backed clock, so until `sepia-time` has run it believes it is
    /// 1970 — and every certificate on earth begins later than that, so every
    /// one of them is "not yet valid".
    ///
    /// Reporting that as a certificate error sends somebody to look at the
    /// server, the source, or their network. The clock is the answer, so the
    /// clock is what the message names.
    fn translate(&self, url: &str, error: &ureq::Error) -> Error {
        if is_tls_failure(error) {
            let now = self.now();
            if now < self.floor {
                return Error::ClockBehind {
                    url: url.to_owned(),
                    reading: crate::ui::date(now),
                };
            }
        }
        Error::Network {
            url: url.to_owned(),
            message: error.to_string(),
        }
    }
}

/// Whether the failure happened in the TLS layer.
///
/// Only these become a clock message. A refused connection or a 404 has
/// nothing to do with what the device thinks the time is, and saying so would
/// send somebody to fix a clock that is already right.
fn is_tls_failure(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::Rustls(_) | ureq::Error::Tls(_) => true,
        // `ureq` flattens a certificate failure from the connector into an
        // `Io` error, so by the time it arrives here the type is gone and only
        // the words are left: "invalid peer certificate: UnknownIssuer". This
        // is a string check, and it is a string check on purpose — the
        // alternative is depending on `rustls` directly and downcasting, which
        // fails *silently* if the two ever resolve to different versions of
        // it. This fails loudly instead, because the test that a device with a
        // 1970 clock is told about its clock is what holds it up.
        ureq::Error::Io(inner) => inner.to_string().contains("certificate"),
        _ => false,
    }
}

/// The earliest the time could be, from the image's build date if it has one.
fn floor_from(path: &Path) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(ASSUMED_FLOOR)
}

/// Whether this URL is one a `GITHUB_TOKEN` may be sent to.
///
/// **Only GitHub.** A credential goes to the host it was issued for and to no
/// other: a source is a URL somebody typed, and sending a token to it because
/// it happens to be in the environment would hand it to whoever runs that
/// host. The check is on the whole host and not a substring of it, so
/// `github.com.example.test` is not GitHub.
///
/// Pure, so it can be tested. Reading the environment is unsafe to do from a
/// test in this edition, and a function that reads it could not be checked.
fn wants_token(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let Some(host) = rest.split(['/', ':']).next() else {
        return false;
    };
    host == "github.com" || host.ends_with(".github.com")
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test that cannot fail loudly is worse"
)]
mod tests {
    use super::*;

    #[test]
    fn a_token_goes_to_github_and_nowhere_else() {
        assert!(wants_token("https://github.com/Sepia-OS/spm"));
        assert!(wants_token("https://api.github.com/repos/Sepia-OS/spm"));
        assert!(!wants_token("https://example.test/index.json"));
    }

    #[test]
    fn a_host_that_merely_contains_github_is_not_github() {
        // The mistake a substring check makes, and why this one is on the
        // whole host: whoever runs these would be handed the token.
        assert!(!wants_token("https://github.com.example.test/index.json"));
        assert!(!wants_token("https://notgithub.com/index.json"));
        assert!(!wants_token("https://example.test/github.com/index.json"));
    }

    #[test]
    fn a_token_is_never_sent_over_plain_http() {
        assert!(!wants_token("http://github.com/anything"));
    }

    #[test]
    fn the_floor_is_the_build_date_when_there_is_one() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sepia-build-date");
        std::fs::write(&path, "1757260800\n").unwrap();
        assert_eq!(floor_from(&path), 1_757_260_800);
    }

    #[test]
    fn the_floor_falls_back_when_there_is_no_build_date() {
        let directory = tempfile::tempdir().unwrap();
        // Not there at all, and there but nonsense: both fall back rather
        // than making every certificate failure a clock message.
        assert_eq!(floor_from(&directory.path().join("nothing")), ASSUMED_FLOOR);
        let odd = directory.path().join("odd");
        std::fs::write(&odd, "yesterday").unwrap();
        assert_eq!(floor_from(&odd), ASSUMED_FLOOR);
    }

    #[test]
    fn only_a_tls_failure_can_be_a_clock_failure() {
        // A refused connection has nothing to do with the time, and saying so
        // would send somebody to fix a clock that is already right.
        assert!(is_tls_failure(&ureq::Error::Tls("nope")));
        // The shape `ureq` actually produces for a bad certificate.
        assert!(is_tls_failure(&ureq::Error::Io(std::io::Error::other(
            "invalid peer certificate: UnknownIssuer"
        ))));
        // And an ordinary I/O failure, which is not about the time at all.
        assert!(!is_tls_failure(&ureq::Error::Io(std::io::Error::other(
            "Connection refused (os error 61)"
        ))));
        assert!(!is_tls_failure(&ureq::Error::ConnectionFailed));
        assert!(!is_tls_failure(&ureq::Error::StatusCode(404)));
        assert!(!is_tls_failure(&ureq::Error::HostNotFound));
    }

    #[test]
    fn a_status_that_could_change_is_retried_and_one_that_cannot_is_not() {
        assert!(retryable(&ureq::Error::StatusCode(500)));
        assert!(retryable(&ureq::Error::StatusCode(503)));
        assert!(!retryable(&ureq::Error::StatusCode(404)));
        assert!(!retryable(&ureq::Error::StatusCode(401)));
        assert!(retryable(&ureq::Error::ConnectionFailed));
    }
}
