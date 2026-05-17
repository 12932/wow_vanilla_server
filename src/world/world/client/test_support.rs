//! Helpers for building game-state fixtures (no TCP, no SRP handshake) so
//! integration tests and benchmarks under `benches/` can exercise the hot
//! loops directly.
//!
//! Real production clients are constructed inside
//! [`CharacterScreenClient::new`] from a live `TcpStream` and a freshly
//! handshook `HeaderCrypto`. The helpers here mirror that wiring but
//! substitute:
//! - `tokio::io::duplex` for the socket — the writer task drains into an
//!   in-memory buffer the other side of which we throw away;
//! - a synthetic SRP-derived `HeaderCrypto` pair generated with a fixed
//!   zeroed session key, mirroring the pattern in `character_screen_client`'s
//!   test module.
//!
//! Calling [`synthetic_client`] requires a Tokio runtime context (it spawns
//! the writer task). Criterion benches under `async_tokio` provide one
//! automatically inside the closures passed to `iter_*`.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use tokio::io::duplex;
use tokio::sync::Semaphore;
use tokio::sync::mpsc;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::{DecrypterHalf, EncrypterHalf, ProofSeed};

use crate::world::world::client::OUTGOING_PACKETS;
use crate::world::world::client::character_screen_client::run_writer;
use crate::world::world::client::{Client, OutboundTx};
use crate::world::world_opcode_handler::character::Character;

/// Build a paired (server-encrypter, client-decrypter). The keystream of
/// `server_encrypter` matches what a real connected client's
/// `DecrypterHalf` would consume — same pairing production uses, but
/// driven from a zeroed session key so no network handshake is required.
pub fn paired_crypto() -> (EncrypterHalf, DecrypterHalf) {
    let session_key = [0u8; 40];
    let username = NormalizedString::new("test".to_string()).unwrap();
    let server_seed = ProofSeed::new();
    let server_seed_value = server_seed.seed();
    let client_seed = ProofSeed::new();
    let client_seed_value = client_seed.seed();

    let (client_proof, client_hc) =
        client_seed.into_client_header_crypto(&username, session_key, server_seed_value);
    let server_hc = server_seed
        .into_server_header_crypto(&username, session_key, client_proof, client_seed_value)
        .expect("paired server crypto");

    let (server_encrypter, _server_decrypter) = server_hc.split();
    let (_client_encrypter, client_decrypter) = client_hc.split();
    (server_encrypter, client_decrypter)
}

/// Construct a [`Client`] backed by an in-memory duplex pair instead of a
/// real `TcpStream`. The writer task spawns immediately and drains any
/// outbound packets into the duplex; the read half is dropped, so the
/// duplex's internal ring buffer fills and eventually back-pressures the
/// writer — fine for benches where we only care about the queue + the
/// hot-loop logic, not delivery semantics.
///
/// Requires an active Tokio runtime (the writer is `tokio::spawn`'d).
pub fn synthetic_client(character: Character, account_name: impl Into<String>) -> Client {
    let (encrypter, _client_decrypter) = paired_crypto();
    // 1 MiB duplex is plenty — typical bench scenarios produce kilobytes
    // before the bench iteration ends, well under the buffer.
    let (write_half, _read_half) = duplex(1024 * 1024);

    let (unbounded_tx, outbound_rx) = kanal::unbounded_async::<Arc<[u8]>>();
    let byte_budget = Arc::new(Semaphore::new(
        crate::config::config().network.outbound_channel_bytes,
    ));
    let outbound = OutboundTx::new(unbounded_tx, byte_budget.clone());
    let dropped_packets = Arc::new(AtomicU64::new(0));

    let writer_handle = tokio::spawn(run_writer(
        write_half,
        encrypter,
        outbound_rx,
        byte_budget,
        &OUTGOING_PACKETS,
    ));

    // Reader-side: tests/benches don't drive client → server traffic, so
    // we hand back an empty receiver. The sender goes out of scope here
    // and the channel closes; consumers iterating it just see no
    // messages, which is the desired behavior under fixtures.
    let (_client_send, client_recv) =
        mpsc::channel::<wow_world_messages::vanilla::opcodes::ClientOpcodeMessage>(32);

    // No reader task — there's nothing to read from. A finished
    // JoinHandle is fine; downstream code checks
    // `reader_handle.is_finished()` and treats that as "this client has
    // disconnected", but the world tick code in benches doesn't drive
    // the disconnect path so it never observes this.
    let reader_handle = tokio::spawn(async {});

    Client::from_parts(
        character,
        client_recv,
        outbound,
        dropped_packets,
        account_name.into(),
        reader_handle,
        writer_handle,
    )
}
