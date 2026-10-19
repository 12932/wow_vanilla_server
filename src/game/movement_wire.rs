//! A passenger's movement as the 1.12.1 client lays it out, where
//! wow_messages differs.
//!
//! The client flags a unit on a transport `0x0200_0000` and follows its
//! place and facing with the transport's whole GUID and the place and
//! facing on deck, 24 bytes (mangos-classic's `MovementInfo::Read` and
//! `Write`; the client's own reader, `0x47F050`, reads them for that flag
//! and nothing for `0x200`). wow_messages has the flag at `0x200`, with a
//! packed GUID and, in a movement packet (not a create), a timestamp after.
//! So a real client's movement aboard read as wow_messages's was read as
//! not aboard, with its fall time out of the GUID: the server never knew
//! who rode a boat, pulled a passenger walking on deck back for moving too
//! fast (into the sea), and left them behind when the boat crossed.
//!
//! wvs keeps wow_messages's form inside. A client's packets are put into it
//! as they are read ([`from_client`]); what is relayed back is put into
//! 1.12's ([`relay_to_client`], [`body_to_client`]); a create writes 1.12's
//! itself (`update::write_placement`). The tests' clients read with
//! wow_messages, so the harness puts what it receives back the other way
//! ([`relay_from_client`]).

use std::sync::Arc;

/// 1.12's flag for a unit on a transport.
pub const ON_TRANSPORT: u32 = 0x0200_0000;
/// wow_messages's.
pub const WM_ON_TRANSPORT: u32 = 0x0000_0200;

/// A movement's flags, timestamp, place and facing: what comes before a
/// transport.
const HEAD: usize = 24;
/// 1.12's transport: a GUID, a place and a facing.
const TRANSPORT: usize = 8 + 16;

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// A packed GUID at `at`: its value and length.
pub fn packed_guid_at(b: &[u8], at: usize) -> Option<(u64, usize)> {
    let mask = *b.get(at)?;
    let mut value = 0u64;
    let mut n = 1;
    for i in 0..8 {
        if mask & (1 << i) != 0 {
            value |= u64::from(*b.get(at + n)?) << (i * 8);
            n += 1;
        }
    }
    Some((value, n))
}

pub fn write_packed_guid(out: &mut Vec<u8>, value: u64) {
    let mask_at = out.len();
    out.push(0);
    let mut mask = 0u8;
    for i in 0..8 {
        let byte = (value >> (i * 8)) as u8;
        if byte != 0 {
            mask |= 1 << i;
            out.push(byte);
        }
    }
    out[mask_at] = mask;
}

/// The movement at `at` in `body`, aboard by 1.12's layout, put in
/// wow_messages's (with a timestamp of 0 after the transport if
/// `timestamp`, as a movement packet has it). `None` if it isn't aboard.
pub fn movement_from_client(body: &[u8], at: usize, timestamp: bool) -> Option<Vec<u8>> {
    let flags = u32_at(body, at)?;
    if flags & ON_TRANSPORT == 0 || body.len() < at + HEAD + TRANSPORT {
        return None;
    }
    let guid = u64::from_le_bytes(body[at + HEAD..at + HEAD + 8].try_into().ok()?);
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&body[..at]);
    out.extend_from_slice(&((flags & !ON_TRANSPORT) | WM_ON_TRANSPORT).to_le_bytes());
    out.extend_from_slice(&body[at + 4..at + HEAD]);
    write_packed_guid(&mut out, guid);
    out.extend_from_slice(&body[at + HEAD + 8..at + HEAD + TRANSPORT]);
    if timestamp {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&body[at + HEAD + TRANSPORT..]);
    Some(out)
}

/// The movement at `at` in `body`, aboard by wow_messages's layout (with
/// its timestamp if `timestamp`), put in 1.12's. `None` if it isn't
/// aboard.
pub fn movement_to_client(body: &[u8], at: usize, timestamp: bool) -> Option<Vec<u8>> {
    let flags = u32_at(body, at)?;
    if flags & WM_ON_TRANSPORT == 0 {
        return None;
    }
    let (guid, n) = packed_guid_at(body, at + HEAD)?;
    let place = at + HEAD + n;
    let rest = place + 16 + if timestamp { 4 } else { 0 };
    if body.len() < rest {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&body[..at]);
    out.extend_from_slice(&((flags & !WM_ON_TRANSPORT) | ON_TRANSPORT).to_le_bytes());
    out.extend_from_slice(&body[at + 4..at + HEAD]);
    out.extend_from_slice(&guid.to_le_bytes());
    out.extend_from_slice(&body[place..place + 16]);
    out.extend_from_slice(&body[rest..]);
    Some(out)
}

/// Where a client packet's movement starts, for those that carry one: the
/// movement packets, the fall reset and spline's end at once; the active
/// mover's change after a GUID; the acknowledgements after a GUID and a
/// counter.
fn client_movement_at(opcode: u32) -> Option<usize> {
    match opcode {
        0xB5..=0xC3 | 0xC9..=0xCB | 0xDA | 0xDB | 0xEE | 0x2C9 | 0x2CA => Some(0),
        0x2D1 => Some(8),
        0xE3 | 0xE5 | 0xE7 | 0xE9 | 0xEB | 0xF0 | 0xF6 | 0x2CF | 0x2D0 | 0x2DB | 0x2DD | 0x2DF => {
            Some(12)
        }
        _ => None,
    }
}

/// A client packet's body, as it came, with a movement aboard put in
/// wow_messages's layout, to be read.
pub fn from_client(opcode: u32, body: &mut Vec<u8>) {
    if let Some(at) = client_movement_at(opcode)
        && let Some(out) = movement_from_client(body, at, true)
    {
        *body = out;
    }
}

/// A movement packet's body (a packed GUID and the movement, maybe more
/// after), aboard, in 1.12's layout; as it was otherwise.
pub fn body_to_client(body: Vec<u8>) -> Vec<u8> {
    let Some((_, n)) = packed_guid_at(&body, 0) else {
        return body;
    };
    movement_to_client(&body, n, true).unwrap_or(body)
}

fn reframe(frame: &[u8], body: &[u8]) -> Arc<[u8]> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&frame[2..4]);
    out.extend_from_slice(body);
    Arc::from(out)
}

/// A relayed movement packet (`[size][opcode][packed GUID][movement]...`),
/// aboard, in 1.12's layout; the frame itself otherwise.
pub fn relay_to_client(frame: Arc<[u8]>) -> Arc<[u8]> {
    let Some(body) = frame.get(4..) else {
        return frame;
    };
    let Some((_, n)) = packed_guid_at(body, 0) else {
        return frame;
    };
    match movement_to_client(body, n, true) {
        Some(out) => reframe(&frame, &out),
        None => frame,
    }
}

/// The server's movement packets: a packed GUID, then the movement.
#[cfg(test)]
fn relayed(opcode: u16) -> bool {
    matches!(
        opcode,
        0xB5..=0xC3 | 0xC5 | 0xC9..=0xCB | 0xDA | 0xDB | 0xEC | 0xED | 0xEE | 0x2B0 | 0x2B1
    )
}

/// A frame the server sent, a movement packet aboard in 1.12's layout put
/// in wow_messages's, for the tests' clients to read. `None` if it is
/// something else, or not aboard.
#[cfg(test)]
pub fn relay_from_client(frame: &[u8]) -> Option<Vec<u8>> {
    let opcode = u16::from_le_bytes([*frame.get(2)?, *frame.get(3)?]);
    if !relayed(opcode) {
        return None;
    }
    let body = &frame[4..];
    let (_, n) = packed_guid_at(body, 0)?;
    let out = movement_from_client(body, n, true)?;
    Some(reframe(frame, &out).to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_world_messages::Guid;
    use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
    use wow_world_messages::vanilla::*;

    const BOAT: u64 = 0x1FC0_0000_0000_5148;

    /// A heartbeat on deck as the 1.12.1 client sends it.
    fn real_heartbeat() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&(ON_TRANSPORT | 0x1).to_le_bytes());
        b.extend_from_slice(&777u32.to_le_bytes());
        for f in [-14_300.0f32, 600.0, 2.0, 1.5] {
            b.extend_from_slice(&f.to_le_bytes());
        }
        b.extend_from_slice(&BOAT.to_le_bytes());
        for f in [3.0f32, -1.0, 6.5, 0.25] {
            b.extend_from_slice(&f.to_le_bytes());
        }
        b.extend_from_slice(&1229u32.to_le_bytes());
        b
    }

    fn read(opcode: u16, body: &[u8]) -> ClientOpcodeMessage {
        let mut plain = Vec::new();
        plain.extend_from_slice(&((body.len() + 4) as u16).to_be_bytes());
        plain.extend_from_slice(&u32::from(opcode).to_le_bytes());
        plain.extend_from_slice(body);
        ClientOpcodeMessage::read_unencrypted(&plain[..]).expect("it reads")
    }

    /// The 1.12.1 client's heartbeat on a boat, as it sends it, reads as
    /// aboard that boat at that place on deck, with its fall time; as it
    /// came, wow_messages read it as on no transport.
    #[test]
    fn a_real_clients_movement_aboard_is_read_aboard() {
        let real = real_heartbeat();
        let info = |m: ClientOpcodeMessage| match m {
            ClientOpcodeMessage::MSG_MOVE_HEARTBEAT(h) => h.info,
            other => panic!("{other:?}"),
        };
        let as_it_came = info(read(0xEE, &real));
        assert!(as_it_came.flags.get_on_transport().is_none());
        let mut body = real.clone();
        from_client(0xEE, &mut body);
        let m = info(read(0xEE, &body));
        let t = m.flags.get_on_transport().expect("aboard");
        assert_eq!(t.transport.guid, Guid::new(BOAT));
        assert_eq!(
            (t.transport.position.x, t.transport.position.y),
            (3.0, -1.0)
        );
        assert_eq!(t.transport.orientation, 0.25);
        assert!(m.flags.get_forward());
        assert_eq!(m.position.x, -14_300.0);
        assert_eq!(m.fall_time.to_bits(), 1229);
        // An acknowledgement's movement, after a GUID and a counter, too.
        let mut ack = Vec::new();
        ack.extend_from_slice(&7u64.to_le_bytes());
        ack.extend_from_slice(&1u32.to_le_bytes());
        ack.extend_from_slice(&real);
        ack.extend_from_slice(&7.0f32.to_le_bytes());
        from_client(0xE3, &mut ack);
        match read(0xE3, &ack) {
            ClientOpcodeMessage::CMSG_FORCE_RUN_SPEED_CHANGE_ACK(a) => {
                assert!(a.info.flags.get_on_transport().is_some());
                assert_eq!(a.new_speed, 7.0);
            }
            other => panic!("{other:?}"),
        }
        // Not aboard: as it came.
        let mut walk = real[..24].to_vec();
        walk[3] = 0;
        walk.extend_from_slice(&0u32.to_le_bytes());
        let before = walk.clone();
        from_client(0xEE, &mut walk);
        assert_eq!(walk, before);
    }

    /// What is relayed of a passenger's movement goes out as 1.12 lays it
    /// out, the bytes the client sent; and back, as the tests' clients
    /// read it.
    #[test]
    fn a_passengers_relayed_movement_is_in_1_12s_layout() {
        let mut body = real_heartbeat();
        from_client(0xEE, &mut body);
        let info = match read(0xEE, &body) {
            ClientOpcodeMessage::MSG_MOVE_HEARTBEAT(h) => h.info,
            other => panic!("{other:?}"),
        };
        let g = Guid::new(42);
        let frame = crate::game::conn::encode(&MSG_MOVE_HEARTBEAT_Server { guid: g, info })
            .expect("encodes");
        let out = relay_to_client(frame.clone());
        // The packed GUID of 42, then exactly what the client sent.
        assert_eq!(&out[4..6], &[1, 42]);
        assert_eq!(&out[6..], &real_heartbeat()[..]);
        assert_eq!(u16::from_be_bytes([out[0], out[1]]) as usize, out.len() - 2);
        assert_eq!(relay_from_client(&out).as_deref(), Some(&frame[..]));
    }
}
