//! A client connection: a reader task that decodes packets, and a writer
//! task that encrypts headers and coalesces writes. Everything else talks
//! to the client through an [`Outbound`].

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Semaphore, mpsc};
use tracing::{debug, warn};
use wow_srp::vanilla_header::{DecrypterHalf, EncrypterHalf, HeaderCrypto};
use wow_world_messages::errors::ExpectedOpcodeError;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::{SMSG_PONG, ServerMessage};

/// Packets written to all clients since start-up.
pub static PACKETS_OUT: AtomicU64 = AtomicU64::new(0);
/// Packets read from all clients since start-up.
pub static PACKETS_IN: AtomicU64 = AtomicU64::new(0);

/// Encode a server message as `[size BE][opcode LE][body]`, the shape the
/// writer task encrypts. Encode once and share the frame between clients.
pub fn encode<M: ServerMessage>(m: &M) -> Option<Arc<[u8]>> {
    let mut buf = Vec::with_capacity(m.size_without_header() as usize + 4);
    m.write_unencrypted_server(&mut buf).ok()?;
    Some(Arc::from(buf))
}

/// Where packets for one client go: a queue the writer task drains, with
/// a byte budget. A client too slow to keep its queue under the budget is
/// disconnected: dropping some of its packets would leave it out of step
/// with the world.
#[derive(Clone)]
pub struct Outbound {
    inner: Arc<OutboundInner>,
}

struct OutboundInner {
    tx: kanal::AsyncSender<Arc<[u8]>>,
    budget: Arc<Semaphore>,
    overflowed: AtomicBool,
}

impl std::fmt::Debug for Outbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outbound")
            .field("budget", &self.inner.budget.available_permits())
            .finish()
    }
}

impl Outbound {
    fn new(tx: kanal::AsyncSender<Arc<[u8]>>, budget: Arc<Semaphore>) -> Self {
        Self {
            inner: Arc::new(OutboundInner {
                tx,
                budget,
                overflowed: AtomicBool::new(false),
            }),
        }
    }

    /// A queue nothing drains, with an unlimited budget, and the receiving
    /// end for tests to read.
    pub fn detached() -> (Self, kanal::Receiver<Arc<[u8]>>) {
        let (tx, rx) = kanal::unbounded_async::<Arc<[u8]>>();
        (
            Self::new(tx, Arc::new(Semaphore::new(Semaphore::MAX_PERMITS))),
            rx.to_sync(),
        )
    }

    /// Queue a framed packet. False if the client is gone or over budget;
    /// over budget, its connection is closed.
    pub fn send_frame(&self, frame: Arc<[u8]>) -> bool {
        let n = u32::try_from(frame.len()).unwrap_or(u32::MAX);
        match self.inner.budget.try_acquire_many(n) {
            Ok(permit) => {
                permit.forget();
                self.inner.tx.try_send(frame).unwrap_or(false)
            }
            Err(_) => {
                if !self.inner.overflowed.swap(true, Ordering::Relaxed) {
                    warn!("a client's outbound queue is over budget; disconnecting it");
                }
                self.close();
                false
            }
        }
    }

    pub fn send<M: ServerMessage>(&self, m: &M) -> bool {
        match encode(m) {
            Some(frame) => self.send_frame(frame),
            None => false,
        }
    }

    /// Close the queue: the writer task ends once it has written what is
    /// queued, and the connection closes.
    pub fn close(&self) {
        let _ = self.inner.tx.close();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.tx.is_closed()
    }

    /// Two handles to the same client.
    pub fn same(&self, other: &Outbound) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// A packet a client sent: one wow_messages reads, or (for those it lays
/// out differently from 1.12) its opcode and body.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // Each packet moves through the channel once.
pub enum Inbound {
    Msg(ClientOpcodeMessage),
    Raw(u16, Vec<u8>),
}

impl From<ClientOpcodeMessage> for Inbound {
    fn from(m: ClientOpcodeMessage) -> Self {
        Inbound::Msg(m)
    }
}

/// The client opcodes read by hand: `CMSG_SET_FACTION_ATWAR`,
/// `CMSG_SET_FACTION_INACTIVE` and `CMSG_SET_WATCHED_FACTION` carry a
/// 32-bit reputation index where wow_messages reads a faction,
/// wow_messages's decompression of `CMSG_UPDATE_ACCOUNT_DATA` fails, and
/// it wants ten answers in `CMSG_GMSURVEY_SUBMIT` where the client stops
/// at a zero question.
pub const RAW_OPCODES: &[u32] = &[0x125, 0x317, 0x318, 0x20B, 0x32A];

/// A started connection: its outbound queue, and the packets it reads.
pub struct Connection {
    pub outbound: Outbound,
    pub inbound: mpsc::Receiver<Inbound>,
}

/// Start the reader and writer tasks for an authenticated stream.
pub fn start(stream: TcpStream, crypto: HeaderCrypto) -> Connection {
    let _ = stream.set_nodelay(true);
    let (read, write) = stream.into_split();
    let (encrypter, decrypter) = crypto.split();
    let (tx, rx) = kanal::unbounded_async::<Arc<[u8]>>();
    let budget = Arc::new(Semaphore::new(
        crate::config::config().network.outbound_channel_bytes,
    ));
    let outbound = Outbound::new(tx, budget.clone());
    tokio::spawn(run_writer(write, encrypter, rx, budget));
    let (in_tx, in_rx) = mpsc::channel(256);
    tokio::spawn(run_reader(read, decrypter, in_tx, outbound.clone()));
    Connection {
        outbound,
        inbound: in_rx,
    }
}

/// Decode packets and pass them on. Pings are answered here, so their
/// round trip measures the network and not the world.
pub(crate) async fn run_reader<R: AsyncRead + Unpin + Send>(
    mut read: R,
    mut decrypter: DecrypterHalf,
    tx: mpsc::Sender<Inbound>,
    outbound: Outbound,
) {
    let idle = crate::config::config().network.client_idle_timeout();
    let mut bad = BadPacketLimit::new();
    loop {
        let read_one = read_packet(&mut read, &mut decrypter);
        let Ok(msg) = tokio::time::timeout(idle, read_one).await else {
            debug!("client sent nothing for {idle:?}; disconnecting");
            break;
        };
        let msg = match msg {
            Ok(Some(Inbound::Msg(m))) => m,
            // wow_messages panicked reading it.
            Ok(None) => {
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Ok(Some(raw)) => {
                PACKETS_IN.fetch_add(1, Ordering::Relaxed);
                if tx.send(raw).await.is_err() {
                    break;
                }
                continue;
            }
            // wow_messages reads a packet's whole body before dispatching on
            // its opcode, so an unknown or unparseable packet is already
            // consumed: skip it.
            Err(ExpectedOpcodeError::Opcode { opcode, size, name }) => {
                debug!("unhandled opcode {name:?} (0x{opcode:X}, {size} bytes)");
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Err(ExpectedOpcodeError::Parse(p)) => {
                debug!("unparseable client packet: {p:?}");
                if bad.exceeded() {
                    break;
                }
                continue;
            }
            Err(ExpectedOpcodeError::Io(_)) => break,
        };
        PACKETS_IN.fetch_add(1, Ordering::Relaxed);
        if let ClientOpcodeMessage::CMSG_PING(ping) = &msg {
            outbound.send(&SMSG_PONG {
                sequence_id: ping.sequence_id,
            });
            continue;
        }
        if tx.send(Inbound::Msg(msg)).await.is_err() {
            break;
        }
        if outbound.is_closed() {
            break;
        }
    }
    outbound.close();
}

/// Read one packet: its header decrypted, its body whole; those read by
/// hand stay raw, the rest go to wow_messages, which reserves no more than
/// a body could hold however much it claims. `None` for a packet that
/// wow_messages panicked on, as a backstop.
async fn read_packet<R: AsyncRead + Unpin + Send>(
    read: &mut R,
    decrypter: &mut DecrypterHalf,
) -> Result<Option<Inbound>, ExpectedOpcodeError> {
    let mut header = [0u8; 6];
    read.read_exact(&mut header).await?;
    let header = decrypter.decrypt_client_header(header);
    let size = header.size.saturating_sub(4) as usize;
    let mut body = vec![0u8; size];
    read.read_exact(&mut body).await?;
    if RAW_OPCODES.contains(&header.opcode) {
        return Ok(Some(Inbound::Raw(header.opcode as u16, body)));
    }
    let mut plain = Vec::with_capacity(6 + size);
    plain.extend_from_slice(&header.size.to_be_bytes());
    plain.extend_from_slice(&header.opcode.to_le_bytes());
    plain.extend_from_slice(&body);
    match crate::game::supervisor::catch(|| ClientOpcodeMessage::read_unencrypted(&plain[..])) {
        Ok(m) => m.map(|m| Some(Inbound::Msg(m))),
        Err(e) => {
            debug!(
                "wow_messages panicked reading client packet 0x{:X}: {e}; dropped",
                header.opcode
            );
            Ok(None)
        }
    }
}

/// The largest auth session the world handshake reads.
pub const MAX_AUTH_SESSION: usize = 8 * 1024;

/// Drain queued frames, encrypt each header in order (ARC4 is a stream
/// cipher) and write each burst with one `write_all`.
pub(crate) async fn run_writer<W>(
    mut write: W,
    mut encrypter: EncrypterHalf,
    rx: kanal::AsyncReceiver<Arc<[u8]>>,
    budget: Arc<Semaphore>,
) where
    W: AsyncWrite + Unpin,
{
    const BATCH_LIMIT: usize = 128;
    let mut staging: Vec<Arc<[u8]>> = Vec::with_capacity(BATCH_LIMIT);
    let mut scratch: Vec<u8> = Vec::with_capacity(16 * 1024);
    let rx_sync = rx.clone_sync();
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
        scratch.clear();
        let mut written = 0u64;
        let mut drained = 0usize;
        for buf in &staging {
            drained += buf.len();
            if buf.len() < 4 {
                continue;
            }
            let size = u16::from_be_bytes([buf[0], buf[1]]);
            let opcode = u16::from_le_bytes([buf[2], buf[3]]);
            scratch.extend_from_slice(&encrypter.encrypt_server_header(size, opcode));
            scratch.extend_from_slice(&buf[4..]);
            written += 1;
        }
        budget.add_permits(drained);
        if scratch.is_empty() {
            continue;
        }
        if write.write_all(&scratch).await.is_err() {
            break;
        }
        PACKETS_OUT.fetch_add(written, Ordering::Relaxed);
    }
    let _ = rx.close();
    let _ = write.shutdown().await;
}

/// How many unknown or unparseable packets a client may send in a window
/// before it is disconnected.
const BAD_PACKETS_PER_WINDOW: u32 = 32;
const BAD_PACKET_WINDOW: Duration = Duration::from_secs(10);

struct BadPacketLimit {
    window_start: Instant,
    count: u32,
}

impl BadPacketLimit {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            count: 0,
        }
    }

    fn exceeded(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) > BAD_PACKET_WINDOW {
            self.window_start = now;
            self.count = 0;
        }
        self.count += 1;
        if self.count > BAD_PACKETS_PER_WINDOW {
            warn!(
                "client sent {} bad packets in {BAD_PACKET_WINDOW:?}; disconnecting",
                self.count
            );
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, duplex};
    use wow_srp::normalized_string::NormalizedString;
    use wow_srp::vanilla_header::ProofSeed;

    pub(crate) fn paired_crypto() -> (HeaderCrypto, HeaderCrypto) {
        let session_key = [0u8; 40];
        let username = NormalizedString::new("test".to_string()).unwrap();
        let server_seed = ProofSeed::new();
        let server_seed_value = server_seed.seed();
        let client_seed = ProofSeed::new();
        let client_seed_value = client_seed.seed();
        let (client_proof, client) =
            client_seed.into_client_header_crypto(&username, session_key, server_seed_value);
        let server = server_seed
            .into_server_header_crypto(&username, session_key, client_proof, client_seed_value)
            .unwrap();
        (server, client)
    }

    fn frame(opcode: u16, body: &[u8]) -> Arc<[u8]> {
        crate::game::update::frame(opcode, body)
    }

    #[tokio::test]
    async fn the_writer_keeps_the_cipher_in_step_across_bursts() {
        let (server, client) = paired_crypto();
        let (encrypter, _) = server.split();
        let (_, mut decrypter) = client.split();
        let (mut a, b) = duplex(64 * 1024);
        let (tx, rx) = kanal::unbounded_async::<Arc<[u8]>>();
        let budget = Arc::new(Semaphore::new(1 << 20));
        let writer = tokio::spawn(run_writer(b, encrypter, rx, budget));
        for burst in [1u16, 3, 2] {
            for i in 0..burst {
                tx.send(frame(100 + i, &[i as u8; 4])).await.unwrap();
            }
            for i in 0..burst {
                let mut header = [0u8; 4];
                a.read_exact(&mut header).await.unwrap();
                let h = decrypter.decrypt_server_header(header);
                assert_eq!(h.opcode, 100 + i);
                assert_eq!(h.size, 6);
                let mut body = [0u8; 4];
                a.read_exact(&mut body).await.unwrap();
                assert_eq!(body, [i as u8; 4]);
            }
        }
        drop(tx);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn the_reader_hands_on_packets_wow_messages_reads_and_raw_ones() {
        let (server, client) = paired_crypto();
        let (_, decrypter) = server.split();
        let (mut encrypter, _) = client.split();
        let (mut a, b) = duplex(64 * 1024);
        let (tx, mut rx) = mpsc::channel(8);
        let (out_tx, _out_rx) = kanal::unbounded_async::<Arc<[u8]>>();
        let outbound = Outbound::new(out_tx, Arc::new(Semaphore::new(1 << 20)));
        tokio::spawn(run_reader(b, decrypter, tx, outbound));
        // CMSG_SET_WATCHED_FACTION, read by hand: index 3.
        let body = 3u32.to_le_bytes();
        a.write_all(&encrypter.encrypt_client_header(4 + 4, 0x318))
            .await
            .unwrap();
        a.write_all(&body).await.unwrap();
        // CMSG_LOGOUT_REQUEST, which wow_messages reads.
        a.write_all(&encrypter.encrypt_client_header(4, 0x4B))
            .await
            .unwrap();
        match rx.recv().await {
            Some(Inbound::Raw(0x318, b)) => assert_eq!(b, body),
            other => panic!("{other:?}"),
        }
        match rx.recv().await {
            Some(Inbound::Msg(ClientOpcodeMessage::CMSG_LOGOUT_REQUEST)) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_client_over_budget_is_disconnected() {
        let (tx, _rx) = kanal::unbounded_async::<Arc<[u8]>>();
        let out = Outbound::new(tx, Arc::new(Semaphore::new(10)));
        assert!(out.send_frame(frame(1, &[0; 4])));
        assert!(!out.send_frame(frame(1, &[0; 4])));
        assert!(out.is_closed());
    }

    #[test]
    fn a_burst_of_bad_packets_disconnects_and_a_few_do_not() {
        let mut limit = BadPacketLimit::new();
        for _ in 0..BAD_PACKETS_PER_WINDOW {
            assert!(!limit.exceeded());
        }
        assert!(limit.exceeded());
    }
}
