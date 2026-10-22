//! Thistle Tea's packet decoding tests (Phase 3, `game/inbound`,
//! `game/network`), through wvs's connection reader: the bytes Thistle
//! Tea's tests decode are written to it as a client sends them, encrypted
//! header and all, and what comes out is what the session would get. A
//! packet wow_messages can't read is skipped, and the next one still read.

use std::time::Duration;

use tokio::io::{AsyncWriteExt, DuplexStream, duplex};
use tokio::sync::mpsc;
use wow_srp::normalized_string::NormalizedString;
use wow_srp::vanilla_header::{EncrypterHalf, ProofSeed};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};

use crate::game::conn::{self, Inbound, Outbound};

struct Reader {
    to: DuplexStream,
    encrypter: EncrypterHalf,
    rx: mpsc::Receiver<Inbound>,
    out: conn::Frames,
}

/// wvs's reader on one end of a pipe, the client's encryption on the other.
fn reader() -> Reader {
    let key = [7u8; 40];
    let name = NormalizedString::new("WIRE").unwrap();
    let (server_seed, client_seed) = (ProofSeed::new(), ProofSeed::new());
    let (s, c) = (server_seed.seed(), client_seed.seed());
    let (proof, client) = client_seed.into_client_header_crypto(&name, key, s);
    let server = server_seed
        .into_server_header_crypto(&name, key, proof, c)
        .unwrap();
    let (_, decrypter) = server.split();
    let (encrypter, _) = client.split();
    let (to, from) = duplex(64 * 1024);
    let (tx, rx) = mpsc::channel(64);
    let (outbound, out) = Outbound::detached();
    tokio::spawn(conn::run_reader(from, decrypter, tx, outbound));
    Reader {
        to,
        encrypter,
        rx,
        out,
    }
}

impl Reader {
    async fn send(&mut self, opcode: u16, body: &[u8]) {
        let header = self
            .encrypter
            .encrypt_client_header(body.len() as u16 + 4, u32::from(opcode));
        self.to.write_all(&header).await.unwrap();
        self.to.write_all(body).await.unwrap();
    }

    /// What the reader passes on next.
    async fn next(&mut self) -> Inbound {
        tokio::time::timeout(Duration::from_secs(10), self.rx.recv())
            .await
            .expect("the reader passes something on")
            .expect("the reader is running")
    }

    async fn msg(&mut self, opcode: u16, body: &[u8]) -> ClientOpcodeMessage {
        self.send(opcode, body).await;
        match self.next().await {
            Inbound::Msg(m) => m,
            Inbound::Raw(o, _) => panic!("0x{o:X} came raw"),
        }
    }

    /// `opcode` with `body` is skipped: the logout request after it comes
    /// out next.
    async fn skipped(&mut self, opcode: u16, body: &[u8]) {
        self.send(opcode, body).await;
        self.send(0x4B, &[]).await;
        match self.next().await {
            Inbound::Msg(ClientOpcodeMessage::CMSG_LOGOUT_REQUEST) => {}
            other => panic!("0x{opcode:X} with {} bytes: {other:?}", body.len()),
        }
    }
}

fn cstr(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

/// Thistle Tea's dispatch_test, packet_test and the "rejects malformed
/// payloads" tests: a ping is answered on the connection and not passed
/// on, an unknown opcode is skipped, and a payload short or long for its
/// message is skipped.
#[tokio::test]
async fn the_reader_answers_pings_and_skips_what_it_cant_read() {
    let mut r = reader();
    // CMSG_PING: its sequence back in SMSG_PONG, here.
    let mut ping = 1u32.to_le_bytes().to_vec();
    ping.extend_from_slice(&2u32.to_le_bytes());
    r.send(0x1DC, &ping).await;
    let pong = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(f) = r.out.try_recv() {
                break f;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("SMSG_PONG");
    match ServerOpcodeMessage::read_unencrypted(&pong[..]).unwrap() {
        ServerOpcodeMessage::SMSG_PONG(p) => assert_eq!(p.sequence_id, 1),
        other => panic!("{other}"),
    }
    // An opcode no client message has.
    r.skipped(0x123, &[1, 2, 3]).await;
    // CMSG_AUTOEQUIP_ITEM_SLOT: nine bytes, not none, eight or ten.
    let mut equip = 0x4000_0000_0012_3456u64.to_le_bytes().to_vec();
    equip.push(13);
    for body in [&[][..], &equip[..8], &[&equip[..], &[0]].concat()[..]] {
        r.skipped(0x10F, body).await;
    }
    match r.msg(0x10F, &equip).await {
        ClientOpcodeMessage::CMSG_AUTOEQUIP_ITEM_SLOT(m) => {
            assert_eq!(m.guid, Guid::new(0x4000_0000_0012_3456));
            assert_eq!(m.destination_slot, 13);
        }
        other => panic!("{other}"),
    }
    // CMSG_TOGGLE_PVP: none or one byte, never two.
    r.skipped(0x253, &[1, 0]).await;
    match r.msg(0x253, &[]).await {
        ClientOpcodeMessage::CMSG_TOGGLE_PVP(m) => assert!(m.set.is_none(), "a toggle"),
        other => panic!("{other}"),
    }
    // CMSG_ZONEUPDATE with no area: nothing to do.
    r.skipped(0x1F4, &[]).await;
    // The banker's four, a byte short.
    for opcode in [0x1B7, 0x1B9, 0x283, 0x282] {
        r.skipped(opcode, &[1]).await;
    }
    // CMSG_AUCTION_LIST_BIDDER_ITEMS: its outbid items as counted, not
    // fewer. A byte over is let by, as mangos's
    // HandleAuctionListBidderItems lets a bad size by (Thistle Tea
    // refuses it).
    let mut bidder = 123u64.to_le_bytes().to_vec();
    for v in [0u32, 2, 7, 9] {
        bidder.extend_from_slice(&v.to_le_bytes());
    }
    r.skipped(0x264, &bidder[..20]).await;
    for body in [bidder.clone(), [&bidder[..], &[0]].concat()] {
        match r.msg(0x264, &body).await {
            ClientOpcodeMessage::CMSG_AUCTION_LIST_BIDDER_ITEMS(m) => {
                assert_eq!(m.auctioneer, Guid::new(123));
                assert_eq!(m.start_from_page, 0);
                assert_eq!(m.outbid_item_ids, vec![7, 9]);
            }
            other => panic!("{other}"),
        }
    }
    // A frame's size counts its opcode: two for an empty body.
    let empty = crate::game::update::frame(0x100, &[]);
    assert_eq!(&empty[..], &[0, 2, 0, 1]);
}

/// Thistle Tea's channel_commands_test, dispatch_test's every channel
/// command, and binary_utils_test's strings: each of the fourteen channel
/// commands reads its channel, its player or password after it, and an
/// empty string.
#[tokio::test]
async fn the_reader_reads_every_channel_command() {
    let mut r = reader();
    let channel = cstr("Custom");
    // List, owner, announcements, moderate: the channel alone.
    for opcode in [0x09A, 0x09E, 0x0A7, 0x0A8] {
        let name = match r.msg(opcode, &channel).await {
            ClientOpcodeMessage::CMSG_CHANNEL_LIST(m) => m.channel_name,
            ClientOpcodeMessage::CMSG_CHANNEL_OWNER(m) => m.channel_name,
            ClientOpcodeMessage::CMSG_CHANNEL_ANNOUNCEMENTS(m) => m.channel_name,
            ClientOpcodeMessage::CMSG_CHANNEL_MODERATE(m) => m.channel_name,
            other => panic!("0x{opcode:X}: {other}"),
        };
        assert_eq!(name, "Custom");
    }
    // Set owner, moderator, unmoderator, mute, unmute, invite, kick, ban,
    // unban: the channel, then the player.
    let both = [channel.clone(), cstr("Player")].concat();
    for opcode in [
        0x09D, 0x09F, 0x0A0, 0x0A1, 0x0A2, 0x0A3, 0x0A4, 0x0A5, 0x0A6,
    ] {
        let (c, p) = match r.msg(opcode, &both).await {
            ClientOpcodeMessage::CMSG_CHANNEL_SET_OWNER(m) => (m.channel_name, m.new_owner),
            ClientOpcodeMessage::CMSG_CHANNEL_MODERATOR(m) => (m.channel_name, m.player_name),
            ClientOpcodeMessage::CMSG_CHANNEL_UNMODERATOR(m) => (m.channel_name, m.player_name),
            ClientOpcodeMessage::CMSG_CHANNEL_MUTE(m) => (m.channel_name, m.player_name),
            ClientOpcodeMessage::CMSG_CHANNEL_UNMUTE(m) => (m.channel_name, m.player_name),
            ClientOpcodeMessage::CMSG_CHANNEL_INVITE(m) => (m.channel_name, m.player_name),
            ClientOpcodeMessage::CMSG_CHANNEL_KICK(m) => (m.channel_name, m.player_name),
            ClientOpcodeMessage::CMSG_CHANNEL_BAN(m) => (m.channel_name, m.player_name),
            ClientOpcodeMessage::CMSG_CHANNEL_UNBAN(m) => (m.channel_name, m.player_name),
            other => panic!("0x{opcode:X}: {other}"),
        };
        assert_eq!(
            (c.as_str(), p.as_str()),
            ("Custom", "Player"),
            "0x{opcode:X}"
        );
    }
    // Password: the channel, then the password; and an empty password.
    for (password, want) in [(cstr("secret"), "secret"), (cstr(""), "")] {
        match r.msg(0x09C, &[channel.clone(), password].concat()).await {
            ClientOpcodeMessage::CMSG_CHANNEL_PASSWORD(m) => {
                assert_eq!(m.channel_name, "Custom");
                assert_eq!(m.channel_password, want);
            }
            other => panic!("{other}"),
        }
    }
}

/// Thistle Tea's client_reports_test: a suggestion's text and category,
/// and a GM survey's answers up to its zero question and its comment
/// (read by hand, wow_messages wanting all ten).
#[tokio::test]
async fn the_reader_reads_bug_reports_and_surveys() {
    let mut r = reader();
    let mut bug = 1u32.to_le_bytes().to_vec();
    bug.extend_from_slice(&9u32.to_le_bytes());
    bug.extend_from_slice(&cstr("Add mail"));
    bug.extend_from_slice(&4u32.to_le_bytes());
    bug.extend_from_slice(&cstr("Map"));
    match r.msg(0x1CA, &bug).await {
        ClientOpcodeMessage::CMSG_BUG(m) => {
            assert_eq!(m.suggestion, 1);
            assert_eq!(m.content, "Add mail");
            assert_eq!(m.bug_type, "Map");
        }
        other => panic!("{other}"),
    }
    let mut survey = 2u32.to_le_bytes().to_vec();
    survey.extend_from_slice(&30u32.to_le_bytes());
    survey.push(4);
    survey.extend_from_slice(&cstr("fast"));
    survey.extend_from_slice(&31u32.to_le_bytes());
    survey.push(1);
    survey.extend_from_slice(&cstr(""));
    survey.extend_from_slice(&0u32.to_le_bytes());
    survey.extend_from_slice(&cstr("thanks"));
    r.send(0x32A, &survey).await;
    let body = match r.next().await {
        Inbound::Raw(0x32A, body) => body,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        crate::game::map::tickets::parse_survey(&body),
        Some(crate::game::map::tickets::Survey {
            id: 2,
            answers: vec![(30, 4, "fast".into()), (31, 1, String::new())],
            comment: "thanks".into(),
        })
    );
}

/// 28 bytes of movement: flags, time, position, facing and fall time.
fn movement(flags: u32, extra: &[u8]) -> Vec<u8> {
    let mut v = flags.to_le_bytes().to_vec();
    v.extend_from_slice(&123u32.to_le_bytes());
    for f in [1.0f32, 2.0, 3.0, 4.0] {
        v.extend_from_slice(&f.to_le_bytes());
    }
    v.extend_from_slice(extra);
    v.extend_from_slice(&0u32.to_le_bytes());
    v
}

/// The Maiden's Fancy, the Booty Bay boat.
const BOAT: u64 = 0x1FC0_0000_0000_5148;

/// A heartbeat on deck as the 1.12.1 client sends it: on a transport
/// (`0x0200_0000`), the boat's whole GUID and the place and facing on
/// deck after the world's, and no timestamp (its reader `0x47EBA0`,
/// mangos-classic's `MovementInfo::Read`).
fn aboard(fall_ms: u32) -> Vec<u8> {
    let mut v = (0x0200_0000u32 | 0x1).to_le_bytes().to_vec();
    v.extend_from_slice(&777u32.to_le_bytes());
    for f in [-14_300.0f32, 600.0, 2.0, 1.5] {
        v.extend_from_slice(&f.to_le_bytes());
    }
    v.extend_from_slice(&BOAT.to_le_bytes());
    for f in [3.0f32, -1.0, 6.5, 0.25] {
        v.extend_from_slice(&f.to_le_bytes());
    }
    v.extend_from_slice(&fall_ms.to_le_bytes());
    v
}

/// A real client's movement aboard reads as aboard that boat, at that
/// place on deck, with its fall time in milliseconds; an acknowledgement's
/// too, with what follows it. And what is relayed of it to those who see
/// the passenger is what the client sent. (wow_messages read it as on no
/// transport until its 1.12 layout was fixed, and wvs translated it.)
#[tokio::test]
async fn the_reader_reads_a_passengers_movement_and_it_is_relayed_as_sent() {
    let mut r = reader();
    let real = aboard(1229);
    let info = match r.msg(0xEE, &real).await {
        ClientOpcodeMessage::MSG_MOVE_HEARTBEAT(m) => m.info,
        other => panic!("{other}"),
    };
    let t = info.flags.get_on_transport().expect("aboard").transport;
    assert_eq!(t.guid, Guid::new(BOAT));
    let deck = (t.position.x, t.position.y, t.position.z, t.orientation);
    assert_eq!(deck, (3.0, -1.0, 6.5, 0.25));
    assert!(info.flags.get_forward());
    assert_eq!((info.position.x, info.orientation), (-14_300.0, 1.5));
    assert_eq!(info.fall_time, 1229);
    let mut ack = 7u64.to_le_bytes().to_vec();
    ack.extend_from_slice(&1u32.to_le_bytes());
    ack.extend_from_slice(&real);
    ack.extend_from_slice(&7.0f32.to_le_bytes());
    match r.msg(0x0E3, &ack).await {
        ClientOpcodeMessage::CMSG_FORCE_RUN_SPEED_CHANGE_ACK(m) => {
            assert_eq!(m.info, info);
            assert_eq!(m.new_speed, 7.0);
        }
        other => panic!("{other}"),
    }
    let heartbeat = wow_world_messages::vanilla::MSG_MOVE_HEARTBEAT_Client { info };
    let Ok((_, relay)) = crate::game::map::movement::movement_of(heartbeat.into()) else {
        panic!("a heartbeat is relayed");
    };
    let frame = relay(Guid::new(42), info).expect("it encodes");
    assert_eq!(u16::from_le_bytes([frame[2], frame[3]]), 0xEE);
    // The packed GUID of 42, then exactly what the client sent.
    assert_eq!(&frame[4..6], &[1, 42]);
    assert_eq!(&frame[6..], &real[..]);
}

/// Thistle Tea's movement acknowledgement decoding: root and unroot,
/// feather fall and water walk with the apply word build 5875 sends, and
/// a spline's end with and without a swimmer's pitch.
#[tokio::test]
async fn the_reader_reads_movement_acknowledgements() {
    let mut r = reader();
    let ack = |guid: u64, counter: u32, tail: &[u8]| {
        let mut v = guid.to_le_bytes().to_vec();
        v.extend_from_slice(&counter.to_le_bytes());
        v.extend_from_slice(&movement(0, &[]));
        v.extend_from_slice(tail);
        v
    };
    match r.msg(0x0E9, &ack(1, 2, &[])).await {
        ClientOpcodeMessage::CMSG_FORCE_MOVE_ROOT_ACK(m) => {
            assert_eq!((m.guid, m.movement_counter), (Guid::new(1), 2));
            assert_eq!(m.info.position.x, 1.0);
        }
        other => panic!("{other}"),
    }
    match r.msg(0x0EB, &ack(3, 4, &[])).await {
        ClientOpcodeMessage::CMSG_FORCE_MOVE_UNROOT_ACK(m) => {
            assert_eq!((m.guid, m.movement_counter), (Guid::new(3), 4));
            assert_eq!(m.info.orientation, 4.0);
        }
        other => panic!("{other}"),
    }
    match r.msg(0x2CF, &ack(42, 7, &1u32.to_le_bytes())).await {
        ClientOpcodeMessage::CMSG_MOVE_FEATHER_FALL_ACK(m) => {
            assert_eq!((m.guid, m.movement_counter, m.apply), (Guid::new(42), 7, 1));
        }
        other => panic!("{other}"),
    }
    match r
        .msg(0x2CF, &ack(42, 7, &0x3F80_0000u32.to_le_bytes()))
        .await
    {
        ClientOpcodeMessage::CMSG_MOVE_FEATHER_FALL_ACK(m) => assert_ne!(m.apply, 0),
        other => panic!("{other}"),
    }
    match r
        .msg(0x2D0, &ack(42, 7, &0x3F80_0000u32.to_le_bytes()))
        .await
    {
        ClientOpcodeMessage::CMSG_MOVE_WATER_WALK_ACK(m) => {
            assert_eq!((m.guid, m.movement_counter), (Guid::new(42), 7));
            assert_eq!(m.apply, 0x3F80_0000);
        }
        other => panic!("{other}"),
    }
    let tail = [987u32.to_le_bytes(), 1.0f32.to_le_bytes()].concat();
    for (flags, pitch) in [(0u32, None), (0x20_0000, Some(0.5f32))] {
        let extra = pitch.map(f32::to_le_bytes).unwrap_or_default();
        let extra = if pitch.is_some() { &extra[..] } else { &[][..] };
        let body = [movement(flags, extra), tail.clone()].concat();
        match r.msg(0x2C9, &body).await {
            ClientOpcodeMessage::CMSG_MOVE_SPLINE_DONE(m) => {
                assert_eq!(m.info.timestamp, 123);
                assert_eq!(m.info.position.z, 3.0);
                assert_eq!(m.movement_counter, 987, "the spline's id");
                assert_eq!(m.unknown1, 1.0f32.to_bits());
                assert_eq!(m.info.flags.get_swimming().map(|s| s.pitch), pitch);
            }
            other => panic!("{other}"),
        }
    }
}

/// `MSG_MOVE_TELEPORT_ACK` as a 1.12.1 client writes it: the whole GUID,
/// the counter and the time, 16 bytes (mangos-classic's
/// `HandleMoveTeleportAckOpcode`), read by hand; the packed form wow_messages
/// writes is still read, by its size.
#[tokio::test]
async fn a_teleport_ack_is_read_with_the_whole_guid_or_packed() {
    let mut r = reader();
    let full = [
        &0x0000_0000_0001_0203u64.to_le_bytes()[..],
        &7u32.to_le_bytes(),
        &1234u32.to_le_bytes(),
    ]
    .concat();
    match r.msg(0xC7, &full).await {
        ClientOpcodeMessage::MSG_MOVE_TELEPORT_ACK(m) => {
            assert_eq!(m.guid, Guid::new(0x1_0203));
            assert_eq!(m.movement_counter, 7);
            assert_eq!(m.time, Duration::from_millis(1234));
        }
        other => panic!("{other}"),
    }
    // A packed GUID: its mask, its three bytes set.
    let packed = [
        &[0b0111, 0x03, 0x02, 0x01][..],
        &7u32.to_le_bytes(),
        &1234u32.to_le_bytes(),
    ]
    .concat();
    match r.msg(0xC7, &packed).await {
        ClientOpcodeMessage::MSG_MOVE_TELEPORT_ACK(m) => {
            assert_eq!(m.guid, Guid::new(0x1_0203));
            assert_eq!((m.movement_counter, m.time.as_millis()), (7, 1234));
        }
        other => panic!("{other}"),
    }
}
