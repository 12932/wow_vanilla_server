use crate::world::world::client::{
    Client, OutboundTx, OUTBOUND_CHANNEL_CAPACITY, OUTGOING_PACKETS,
};
use crate::world::world_opcode_handler::character::Character;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use tracing::warn;
use wow_srp::vanilla_header::HeaderCrypto;
use wow_world_base::shared::Guid;
use wow_world_messages::Message;
use wow_world_messages::errors::ExpectedOpcodeError;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::{SMSG_PONG, ServerMessage};

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

        let (outbound_tx, outbound_rx) =
            mpsc::channel::<Vec<u8>>(OUTBOUND_CHANNEL_CAPACITY);
        let dropped_packets = Arc::new(AtomicU64::new(0));

        // Writer task: owns the socket write half + encrypter, drains the
        // outbound channel, and re-encrypts the 4-byte header per item. The
        // world tick never blocks on socket writes — at worst a slow client's
        // channel fills and packets are dropped via try_send in send_*.
        let writer_handle = tokio::spawn(async move {
            let mut write = write;
            let mut encrypter = encrypter;
            let mut rx = outbound_rx;
            while let Some(buf) = rx.recv().await {
                if buf.len() < 4 {
                    continue;
                }
                let size_be = u16::from_be_bytes([buf[0], buf[1]]);
                let opcode = u16::from_le_bytes([buf[2], buf[3]]);
                let enc_header = encrypter.encrypt_server_header(size_be, opcode);
                if write.write_all(&enc_header).await.is_err() {
                    break;
                }
                if buf.len() > 4 && write.write_all(&buf[4..]).await.is_err() {
                    break;
                }
                OUTGOING_PACKETS.fetch_add(1, Ordering::Relaxed);
            }
        });

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
                    match reader_outbound.try_send(buf) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            reader_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
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
        self.queue_buf(buf);
    }

    pub async fn send_opcode(&mut self, m: &ServerOpcodeMessage) {
        let mut buf = Vec::new();
        if m.write_unencrypted_server(&mut buf).is_err() {
            return;
        }
        self.queue_buf(buf);
    }

    fn queue_buf(&self, buf: Vec<u8>) {
        match self.outbound.try_send(buf) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped_packets.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }
}
