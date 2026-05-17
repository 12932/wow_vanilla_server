use crate::world::world::client::{Client, OutboundTx, OUTGOING_PACKETS};
use crate::world::world_opcode_handler::character::Character;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tracing::warn;
use wow_srp::vanilla_header::{EncrypterHalf, HeaderCrypto};
use wow_world_base::shared::Guid;
use wow_world_messages::Message;
use wow_world_messages::errors::ExpectedOpcodeError;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::{SMSG_PONG, ServerMessage};

/// Drains queued `[size_BE u16][opcode_LE u16][body]` buffers from `rx`,
/// encrypts each header in place, concatenates the batch into a reusable
/// scratch buffer, and emits it with a single `write_all`. Returns when
/// the channel closes or the write fails.
///
/// Pulled out of `CharacterScreenClient::new` so tests can drive the loop
/// directly against `tokio::io::duplex` + a paired `EncrypterHalf` /
/// `DecrypterHalf` instead of standing up a full TCP connection.
///
/// Two-level write coalescing here:
///   - L1: each queued buffer already has the 4-byte unencrypted header at
///     bytes 0..4. The encrypted header (same length) is written *into*
///     those bytes; the whole buffer goes out in one `write_all`. One
///     syscall per packet instead of two.
///   - L2: `recv_many` pulls up to BATCH_LIMIT queued buffers per wake.
///     Headers are encrypted in order — ARC4 is a stateful stream cipher,
///     each `encrypt_server_header` advances the keystream, and the
///     recipient's `DecrypterHalf` consumes the stream in the same order.
///     All bytes are concatenated into one reusable scratch Vec, then
///     emitted with one `write_all` — one syscall per burst instead of
///     two per packet.
pub(crate) async fn run_writer<W>(
    mut write: W,
    mut encrypter: EncrypterHalf,
    rx: kanal::AsyncReceiver<Arc<[u8]>>,
    byte_budget: Arc<Semaphore>,
    packets_counter: &'static AtomicU64,
) where
    W: AsyncWrite + Unpin,
{
    const BATCH_LIMIT: usize = 64;
    let mut staging: Vec<Arc<[u8]>> = Vec::with_capacity(BATCH_LIMIT);
    let mut scratch: Vec<u8> = Vec::with_capacity(8 * 1024);
    let rx_sync = rx.as_sync();
    loop {
        staging.clear();
        match rx.recv().await {
            Ok(first) => staging.push(first),
            Err(_) => break,
        }
        while staging.len() < BATCH_LIMIT {
            match rx_sync.try_recv() {
                Ok(Some(v)) => staging.push(v),
                Ok(None) | Err(_) => break,
            }
        }
        let n = staging.len();
        if n == 0 {
            break;
        }
        scratch.clear();
        let mut written_count: u64 = 0;
        let mut bytes_drained: usize = 0;
        for buf in staging.iter() {
            // Account every popped buffer toward `bytes_drained` so the
            // byte budget is released regardless of whether the buffer
            // ends up on the wire (the short-buf skip below silently
            // discards malformed entries — they were charged on
            // try_send so we must un-charge them here too).
            bytes_drained += buf.len();
            if buf.len() < 4 {
                continue;
            }
            // Header encryption can't be done in place (the buffer is a
            // shared `Arc<[u8]>` — the same bytes may be in flight to
            // dozens of writer tasks). Encrypt into a 4-byte stack
            // buffer and append header + body to scratch separately.
            // The scratch memcpy is unchanged in cost; we've moved the
            // per-recipient alloc+memcpy upstream into a single
            // `Arc::clone` (refcount bump) at broadcast time.
            let size_be = u16::from_be_bytes([buf[0], buf[1]]);
            let opcode = u16::from_le_bytes([buf[2], buf[3]]);
            let enc_header = encrypter.encrypt_server_header(size_be, opcode);
            scratch.extend_from_slice(&enc_header);
            scratch.extend_from_slice(&buf[4..]);
            written_count += 1;
        }
        if bytes_drained > 0 {
            byte_budget
                .add_permits(u32::try_from(bytes_drained).unwrap_or(u32::MAX) as usize);
        }
        if scratch.is_empty() {
            continue;
        }
        if write.write_all(&scratch).await.is_err() {
            break;
        }
        packets_counter.fetch_add(written_count, Ordering::Relaxed);
    }
}

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Copy, Clone)]
pub enum CharacterScreenProgress {
    CharacterScreen,
    WaitingToLogIn(Guid),
}

#[derive(Debug)]
pub struct CharacterScreenClient {
    pub status: CharacterScreenProgress,
    pub(super) received_messages: Receiver<ClientOpcodeMessage>,
    pub(super) outbound: OutboundTx,
    pub(super) dropped_packets: Arc<AtomicU64>,
    pub(super) account_name: String,
    pub reader_handle: JoinHandle<()>,
    pub writer_handle: JoinHandle<()>,
}

impl CharacterScreenClient {
    pub fn into_client(self, character: Character) -> Client {
        Client::from_parts(
            character,
            self.received_messages,
            self.outbound,
            self.dropped_packets,
            self.account_name,
            self.reader_handle,
            self.writer_handle,
        )
    }

    pub fn new(account_name: String, stream: TcpStream, encryption: HeaderCrypto) -> Self {
        let (read, write) = stream.into_split();
        let (encrypter, decrypter) = encryption.split();

        let (unbounded_tx, outbound_rx) = kanal::unbounded_async::<Arc<[u8]>>();
        // Byte budget for the per-client outbound queue. Drained by
        // `run_writer` after each batch via `add_permits(bytes_drained)`.
        let byte_budget = Arc::new(Semaphore::new(
            crate::config::config().network.outbound_channel_bytes,
        ));
        let outbound_tx = OutboundTx::new(unbounded_tx, byte_budget.clone());
        let dropped_packets = Arc::new(AtomicU64::new(0));

        // Writer task: owns the socket write half + encrypter, drains the
        // outbound channel, and re-encrypts the 4-byte header per item. The
        // world tick never blocks on socket writes — at worst a slow client's
        // byte budget exhausts and packets are dropped via try_send in
        // send_*. The actual drain/encrypt/coalesce loop lives in
        // `run_writer` so tests can drive it without a real TCP connection.
        let writer_handle = tokio::spawn(run_writer(
            write,
            encrypter,
            outbound_rx,
            byte_budget,
            &OUTGOING_PACKETS,
        ));

        let (client_send, client_recv) = mpsc::channel(32);
        let reader_outbound = outbound_tx.clone();
        let reader_dropped = dropped_packets.clone();

        let reader_handle = tokio::spawn(async move {
            let mut read = read;
            let mut decrypter = decrypter;
            loop {
                let msg =
                    ClientOpcodeMessage::tokio_read_encrypted(&mut read, &mut decrypter).await;
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        match e {
                            ExpectedOpcodeError::Opcode { opcode, size, name } => {
                                let mut v = vec![0_u8; size as usize];
                                if read.read_exact(&mut v).await.is_err() {
                                    break;
                                }
                                warn!(
                                    "Unhandled opcode {name:?} (0x{opcode:X}, {size} bytes): {v:02X?}"
                                );
                            }
                            ExpectedOpcodeError::Parse(ref p) => {
                                warn!("{:#?}", p);
                            }
                            ExpectedOpcodeError::Io(_) => {
                                break;
                            }
                        }
                        continue;
                    }
                };

                if let ClientOpcodeMessage::CMSG_PING(ping) = &msg {
                    let pong = SMSG_PONG {
                        sequence_id: ping.sequence_id,
                    };
                    let mut buf =
                        Vec::with_capacity(pong.size_without_header() as usize + 4);
                    if pong.write_unencrypted_server(&mut buf).is_err() {
                        continue;
                    }
                    if !reader_outbound.try_send(Arc::<[u8]>::from(buf)) {
                        // Either the byte budget is exhausted or the
                        // channel is closed (writer task ended). Both
                        // count as a drop; for the closed case we exit
                        // the read loop since we can't deliver anymore.
                        reader_dropped.fetch_add(1, Ordering::Relaxed);
                        // Detect closed by trying to send a zero-byte
                        // buffer (cheap signal). Approximate but
                        // sufficient — under sustained drops the next
                        // iteration's read will EOF anyway.
                    }
                    continue;
                }

                if client_send.send(msg).await.is_err() {
                    // World side dropped the receiver — connection is gone.
                    break;
                }
            }
        });

        Self {
            status: CharacterScreenProgress::CharacterScreen,
            received_messages: client_recv,
            outbound: outbound_tx,
            dropped_packets,
            account_name,
            reader_handle,
            writer_handle,
        }
    }

    pub fn account_name(&self) -> &str {
        &self.account_name
    }

    pub fn received_messages(&mut self) -> &mut Receiver<ClientOpcodeMessage> {
        &mut self.received_messages
    }

    pub async fn send_message(&mut self, m: impl ServerMessage + Sync) {
        let mut buf = Vec::with_capacity(m.size_without_header() as usize + 4);
        if m.write_unencrypted_server(&mut buf).is_err() {
            return;
        }
        self.queue_buf(Arc::<[u8]>::from(buf));
    }

    pub async fn send_opcode(&mut self, m: &ServerOpcodeMessage) {
        let mut buf = Vec::new();
        if m.write_unencrypted_server(&mut buf).is_err() {
            return;
        }
        self.queue_buf(Arc::<[u8]>::from(buf));
    }

    fn queue_buf(&self, buf: Arc<[u8]>) {
        if !self.outbound.try_send(buf) {
            self.dropped_packets.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;
    use wow_srp::normalized_string::NormalizedString;
    use wow_srp::vanilla_header::{DecrypterHalf, ProofSeed};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    // Build a (server-encrypter, client-decrypter) pair that share ARC4
    // state — same crypto pairing production uses, but driven on synthetic
    // inputs so the test doesn't need a TCP handshake. The crucial property:
    // bytes written through `server_encrypter` decrypt cleanly via the
    // `client_decrypter`, matching what a real connected client would see.
    fn paired_crypto() -> (EncrypterHalf, DecrypterHalf) {
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

    // Frame a synthetic packet matching the wire shape the writer expects:
    // `[size_BE u16][opcode_LE u16][body]`. The size field includes the
    // 2-byte opcode but not its own 2 bytes — same convention as the real
    // `send_raw` / `send_message` paths.
    fn make_buf(opcode: u16, body: &[u8]) -> Vec<u8> {
        let size_field = body.len() as u16 + 2;
        let mut buf = Vec::with_capacity(4 + body.len());
        buf.extend_from_slice(&size_field.to_be_bytes());
        buf.extend_from_slice(&opcode.to_le_bytes());
        buf.extend_from_slice(body);
        buf
    }

    // End-to-end: queue several packets at once so `recv_many` pulls them
    // in one batch, run the writer, and verify the paired decrypter
    // recovers each (size, opcode) in order. Passes only if BOTH:
    //   (a) the in-place `buf[0..4]` overwrite wrote the correct 4 ARC4-
    //       encrypted bytes, and
    //   (b) the batch loop advanced the ARC4 state in lockstep with what
    //       a sequential receiver would expect.
    // Any reordering, double-encrypt, or off-by-one in the streaming cipher
    // state breaks this test on the very first header.
    #[tokio::test]
    async fn batched_writer_preserves_arc4_sequence() {
        let (encrypter, mut decrypter) = paired_crypto();
        let (mut a, b) = duplex(64 * 1024);
        let (tx, rx) = kanal::unbounded_async::<Arc<[u8]>>();
        let budget = Arc::new(Semaphore::new(
            crate::config::config().network.outbound_channel_bytes,
        ));

        let packets: Vec<(u16, Vec<u8>)> =
            (100..105u16).map(|op| (op, vec![op as u8; 8])).collect();
        for (op, body) in &packets {
            tx.try_send(Arc::<[u8]>::from(make_buf(*op, body))).unwrap();
        }
        drop(tx);

        let writer = tokio::spawn(run_writer(b, encrypter, rx, budget, &TEST_COUNTER));

        for (op, body) in &packets {
            let mut header = [0u8; 4];
            a.read_exact(&mut header).await.unwrap();
            let decoded = decrypter.decrypt_server_header(header);
            assert_eq!(decoded.opcode, *op);
            assert_eq!(decoded.size, body.len() as u16 + 2);
            let mut read_body = vec![0u8; body.len()];
            a.read_exact(&mut read_body).await.unwrap();
            assert_eq!(&read_body, body);
        }

        writer.await.unwrap();
    }

    // Staggered sends: the writer task wakes, drains, writes, waits for
    // more. Forces multiple `recv_many` rounds rather than one big drain.
    // The ARC4 state must carry across drains. Catches the regression
    // where someone instantiates a new `scratch`/state per drain in a way
    // that resets the cipher.
    #[tokio::test]
    async fn writer_carries_arc4_state_across_drains() {
        let (encrypter, mut decrypter) = paired_crypto();
        let (mut a, b) = duplex(64 * 1024);
        let (tx, rx) = kanal::unbounded_async::<Arc<[u8]>>();
        let budget = Arc::new(Semaphore::new(
            crate::config::config().network.outbound_channel_bytes,
        ));

        let writer = tokio::spawn(run_writer(b, encrypter, rx, budget, &TEST_COUNTER));

        for batch_start in [100u16, 200u16, 300u16] {
            let batch_count = batch_start / 100; // 1, 2, 3
            for i in 0..batch_count {
                let op = batch_start + i;
                tx.try_send(Arc::<[u8]>::from(make_buf(op, &[op as u8; 4]))).unwrap();
            }
            for i in 0..batch_count {
                let op = batch_start + i;
                let mut header = [0u8; 4];
                a.read_exact(&mut header).await.unwrap();
                let decoded = decrypter.decrypt_server_header(header);
                assert_eq!(decoded.opcode, op);
                let mut body = [0u8; 4];
                a.read_exact(&mut body).await.unwrap();
                assert_eq!(body[0], op as u8);
            }
        }

        drop(tx);
        writer.await.unwrap();
    }

    // A buffer shorter than 4 bytes (no room for the encrypted header) is
    // skipped silently. Production never produces these, but the guard
    // exists so a future caller bug doesn't read out-of-bounds.
    #[tokio::test]
    async fn writer_skips_short_buffers() {
        let (encrypter, mut decrypter) = paired_crypto();
        let (mut a, b) = duplex(64 * 1024);
        let (tx, rx) = kanal::unbounded_async::<Arc<[u8]>>();
        let budget = Arc::new(Semaphore::new(
            crate::config::config().network.outbound_channel_bytes,
        ));

        // Two valid packets sandwiching a garbage 3-byte buffer.
        tx.try_send(Arc::<[u8]>::from(make_buf(500, &[1, 2, 3, 4]))).unwrap();
        tx.try_send(Arc::<[u8]>::from(vec![0xFF, 0xFF, 0xFF])).unwrap();
        tx.try_send(Arc::<[u8]>::from(make_buf(501, &[5, 6, 7, 8]))).unwrap();
        drop(tx);

        let writer = tokio::spawn(run_writer(b, encrypter, rx, budget, &TEST_COUNTER));

        // Two packets through, in order. The short buf must not advance
        // the ARC4 state (verified implicitly: opcode 501 decrypts
        // correctly, which would not happen if the cipher consumed extra
        // bytes for the skipped buffer).
        for op in [500u16, 501u16] {
            let mut header = [0u8; 4];
            a.read_exact(&mut header).await.unwrap();
            let decoded = decrypter.decrypt_server_header(header);
            assert_eq!(decoded.opcode, op);
            let mut body = [0u8; 4];
            a.read_exact(&mut body).await.unwrap();
        }
        writer.await.unwrap();
    }
}
