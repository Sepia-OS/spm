/*
  https.rs

  Created on 2026-09-08 by Thomas Bonk <thomas@meandmymac.de>
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

//! That the real transport fails closed.
//!
//! Two claims, and both are worth a test rather than a reading of the
//! configuration: a plain `http://` URL is refused *before anything is
//! opened*, and a certificate signed by nobody the compiled-in roots know is
//! rejected.
//!
//! Everything here talks to a listener on the loopback address, started by the
//! test and gone when it ends. Nothing resolves a name or leaves the machine.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "everything under tests/ is test code, and a test that cannot fail loudly is worse"
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use spm::error::Error;
use spm::net::https::Https;
use spm::net::transport::Transport;

/// A listener that counts what reaches it, so a test can say that nothing did.
struct Watched {
    port: u16,
    connections: Arc<AtomicUsize>,
}

impl Watched {
    /// Accept and immediately drop anything that connects, counting it.
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&connections);

        thread::spawn(move || {
            for stream in listener.incoming() {
                counter.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });

        Watched { port, connections }
    }

    fn seen(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// A TLS server with a certificate nobody has any reason to trust.
///
/// Self-signed, for `localhost`, generated here and thrown away with the test.
/// The client should refuse it, which is the whole point: the compiled-in
/// roots are the only thing this program trusts, and a card has no others to
/// fall back on.
fn self_signed_server() -> u16 {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate = certified.cert.der().clone();
    let key =
        rustls::pki_types::PrivateKeyDer::try_from(certified.signing_key.serialize_der()).unwrap();

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .unwrap();
    let config = Arc::new(config);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut connection = rustls::ServerConnection::new(Arc::clone(&config)).unwrap();
            // The client is expected to walk away during the handshake, so
            // whatever happens here is not the test's business.
            let _ = handshake_and_reply(&mut connection, &mut stream);
        }
    });

    port
}

fn handshake_and_reply(
    connection: &mut rustls::ServerConnection,
    stream: &mut TcpStream,
) -> std::io::Result<()> {
    let mut tls = rustls::Stream::new(connection, stream);
    let mut buffer = [0_u8; 1024];
    let _ = tls.read(&mut buffer)?;
    tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nhi")?;
    Ok(())
}

#[test]
fn a_plain_http_url_is_refused_before_anything_is_opened() {
    // Not upgraded, not attempted: refused. A request that had already been
    // made would have told somebody what this device is looking for.
    let watched = Watched::start();
    let url = format!("http://127.0.0.1:{}/index.json", watched.port);

    match Https::without_backoff().get(&url) {
        Err(Error::Network {
            url: named,
            message,
        }) => {
            assert_eq!(named, url);
            assert!(message.contains("https"), "{message}");
        }
        Err(other) => panic!("expected a refusal, got {other:?}"),
        Ok(_) => panic!("a plain http URL was fetched"),
    }

    // Give anything that was going to connect a moment to do so.
    thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(watched.seen(), 0, "a connection was opened anyway");
}

#[test]
fn a_certificate_signed_by_nobody_we_trust_is_refused() {
    // The property the card depends on: `spm` trusts the roots compiled into
    // it and nothing else, because there is no system trust store to consult.
    let port = self_signed_server();
    let url = format!("https://localhost:{port}/index.json");

    match Https::without_backoff().get(&url) {
        Err(Error::Network { url: named, .. }) => assert_eq!(named, url),
        Err(other) => panic!("expected the certificate to be refused, got {other:?}"),
        Ok(_) => panic!("a self-signed certificate was accepted"),
    }
}

#[test]
fn a_connection_that_dies_is_tried_again() {
    // A listener that accepts and hangs up: the handshake fails, which is the
    // kind of failure that could come out differently next time. It should be
    // tried exactly as many times as the policy says, and no more.
    let watched = Watched::start();
    let url = format!("https://127.0.0.1:{}/index.json", watched.port);

    assert!(Https::without_backoff().get(&url).is_err());

    thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(
        watched.seen(),
        spm::net::https::ATTEMPTS as usize,
        "tried {} times, expected {}",
        watched.seen(),
        spm::net::https::ATTEMPTS
    );
}

#[test]
fn a_device_whose_clock_is_unset_is_told_about_the_clock() {
    // The done-when. A Pi has no battery-backed clock, so before `sepia-time`
    // runs it believes it is 1970 - and every certificate on earth begins
    // later than that. Reporting "certificate error" sends somebody to look
    // at the server, the source, or their network, none of which is wrong.
    let port = self_signed_server();
    let url = format!("https://localhost:{port}/index.json");

    // now = 1970, floor = 2025: the clock cannot be right.
    match Https::with_clock(0, 1_735_689_600).get(&url) {
        Err(Error::ClockBehind {
            url: named,
            reading,
        }) => {
            assert_eq!(named, url);
            assert_eq!(reading, "1970-01-01");
        }
        Err(other) => panic!("expected the clock message, got: {other}"),
        Ok(_) => panic!("a self-signed certificate was accepted"),
    }
}

#[test]
fn a_device_whose_clock_is_right_is_told_about_the_certificate() {
    // The other half: once the clock is set, the real error has to come back,
    // or a genuinely bad certificate would be blamed on the time forever.
    let port = self_signed_server();
    let url = format!("https://localhost:{port}/index.json");

    match Https::with_clock(1_757_260_800, 1_735_689_600).get(&url) {
        Err(Error::Network {
            url: named,
            message,
        }) => {
            assert_eq!(named, url);
            assert!(!message.contains("sepia-time"), "{message}");
        }
        Err(other) => panic!("expected the generic TLS error, got: {other}"),
        Ok(_) => panic!("a self-signed certificate was accepted"),
    }
}

#[test]
fn a_clock_that_is_behind_does_not_excuse_a_refused_connection() {
    // Only a TLS failure can be a clock failure. Nothing is listening here.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let url = format!("https://127.0.0.1:{port}/index.json");

    match Https::with_clock(0, 1_735_689_600).get(&url) {
        Err(Error::Network { .. }) => {}
        Err(other) => panic!("expected a plain network error, got: {other}"),
        Ok(_) => panic!("something answered"),
    }
}

#[test]
fn a_url_that_is_not_https_is_refused_whatever_it_says() {
    for url in [
        "http://example.test/index.json",
        "ftp://example.test/index.json",
        "file:///etc/passwd",
        "example.test/index.json",
    ] {
        let outcome = Https::without_backoff().get(url);
        assert!(outcome.is_err(), "{url} was not refused");
    }
}

#[test]
fn a_server_that_is_not_there_is_a_network_error_and_not_a_hang() {
    // Nothing is listening on this port; the transport has to come back.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let url = format!("https://127.0.0.1:{port}/index.json");
    match Https::without_backoff().get(&url) {
        Err(Error::Network { url: named, .. }) => assert_eq!(named, url),
        Err(other) => panic!("expected a network error, got {other:?}"),
        Ok(_) => panic!("something answered"),
    }
}
