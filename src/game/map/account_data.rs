//! The client's caches and tutorials, as Thistle Tea keeps them: eight
//! caches (config, key bindings and macros for the whole account; each
//! character's own config, bindings, macros, layout and chat settings),
//! whose MD5s go out at login so the client uploads what changed and asks
//! for what it lacks; and the tutorials seen, shared by the account.
//!
//! The caches come zlib-compressed; wow_messages's reading of them fails on
//! real data, so `CMSG_UPDATE_ACCOUNT_DATA` is read by hand.

use std::io::{Read, Write};
use std::sync::Arc;

use md5::{Digest, Md5};
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;

use crate::game::Realm;
use crate::game::data::items::Body;
use crate::game::map::Map;

pub const CMSG_UPDATE_ACCOUNT_DATA: u16 = 0x20B;
pub const SMSG_UPDATE_ACCOUNT_DATA: u16 = 0x20C;
pub const SMSG_ACCOUNT_DATA_TIMES: u16 = 0x209;
/// The caches: 0 to 7. Those of the whole account.
pub const TYPES: u8 = 8;
const ACCOUNT_TYPES: [u8; 3] = [0, 2, 4];
/// The kind the account's tutorials are kept under.
pub const TUTORIALS: u8 = 8;
/// The largest cache the client sends.
const MAX_SIZE: usize = 0xFFFF;

/// Whose a cache is: the account's, or the character's.
pub fn account_wide(kind: u8) -> bool {
    ACCOUNT_TYPES.contains(&kind)
}

/// The store's key for a cache.
pub fn key(account: &str, character: u32, kind: u8) -> String {
    if account_wide(kind) || kind == TUTORIALS {
        account.to_owned()
    } else {
        format!("{account}#{character}")
    }
}

/// A character's caches and the account's tutorials, from the store.
pub async fn load(
    realm: &Realm,
    account: &str,
    character: u32,
) -> (Vec<(u8, Vec<u8>)>, Option<[u32; 8]>) {
    let mut caches = Vec::new();
    let mut tutorials = None;
    for (kind, _, data) in realm.store.account_data(account).await {
        if kind == TUTORIALS && data.len() == 32 {
            tutorials = Some(std::array::from_fn(|i| {
                u32::from_le_bytes([
                    data[i * 4],
                    data[i * 4 + 1],
                    data[i * 4 + 2],
                    data[i * 4 + 3],
                ])
            }));
        } else if account_wide(kind) {
            caches.push((kind, data));
        }
    }
    for (kind, _, data) in realm.store.account_data(&key(account, character, 1)).await {
        if kind < TYPES && !account_wide(kind) {
            caches.push((kind, data));
        }
    }
    (caches, tutorials)
}

/// An empty cache's digest is zeros; a cache's, its MD5.
pub fn digest(data: &[u8]) -> [u8; 16] {
    if data.is_empty() {
        return [0; 16];
    }
    Md5::digest(data).into()
}

/// `SMSG_ACCOUNT_DATA_TIMES`: the eight caches' MD5s, then zeros.
pub fn digests_frame(caches: &[(u8, Vec<u8>)]) -> Arc<[u8]> {
    let mut b = Body::default();
    for kind in 0..TYPES {
        let data = caches
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(&[][..], |(_, d)| &d[..]);
        b.0.extend_from_slice(&digest(data));
    }
    b.0.resize(128, 0);
    b.frame(SMSG_ACCOUNT_DATA_TIMES)
}

/// `SMSG_UPDATE_ACCOUNT_DATA`: a cache, compressed.
pub fn cache_frame(kind: u8, data: &[u8]) -> Arc<[u8]> {
    let mut b = Body::default();
    b.u32(u32::from(kind)).u32(data.len() as u32);
    if !data.is_empty() {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = z.write_all(data);
        if let Ok(compressed) = z.finish() {
            b.0.extend_from_slice(&compressed);
        }
    }
    b.frame(SMSG_UPDATE_ACCOUNT_DATA)
}

/// `CMSG_UPDATE_ACCOUNT_DATA`, read by hand: the kind, the size
/// uncompressed, and the zlib stream.
pub fn parse_update(body: &[u8]) -> Option<(u8, Vec<u8>)> {
    if body.len() < 8 {
        return None;
    }
    let kind = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let size = u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize;
    if kind >= u32::from(TYPES) || size > MAX_SIZE {
        return None;
    }
    if size == 0 {
        return Some((kind as u8, Vec::new()));
    }
    let mut out = Vec::with_capacity(size);
    let read = flate2::read::ZlibDecoder::new(&body[8..])
        .take(MAX_SIZE as u64 + 1)
        .read_to_end(&mut out);
    // A stream cut off before its checksum still inflates whole: flate2
    // (from 1.1.10) calls that an error, but the bytes it read are kept, and
    // all the size said were there.
    if read.is_err() && out.len() != size {
        return None;
    }
    (out.len() <= MAX_SIZE).then_some((kind as u8, out))
}

fn store(map: &Map, g: Guid, kind: u8, data: Vec<u8>) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let now = u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
    let k = key(&p.account, crate::game::guid::counter(g), kind);
    map.realm.store.set_account_data(&k, kind, now, data);
}

/// A cache the client uploaded: kept, for the next login's digests and
/// its requests.
pub fn update_raw(map: &mut Map, g: Guid, body: &[u8]) {
    if let Some((kind, data)) = parse_update(body) {
        update(map, g, kind, data);
    }
}

/// The least time between two store writes of one cache: one changed again
/// sooner waits, kept, for its next write ([`flush`]).
pub const WRITE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// A cache the client uploaded: held at once (its requests answer it), and
/// written unless it is what is held already, at most once a
/// [`WRITE_EVERY`] a kind: an upload is up to 64 KB inflated, and a client
/// may send a hundred a second.
fn update(map: &mut Map, g: Guid, kind: u8, data: Vec<u8>) {
    let now = map.now;
    let Some(p) = map.players.get_mut(&g) else {
        return;
    };
    let caches = &mut p.row.state.caches;
    let held = caches
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or(&[][..], |(_, d)| &d[..]);
    if held == data {
        return;
    }
    caches.retain(|(k, _)| *k != kind);
    if !data.is_empty() {
        caches.push((kind, data));
    }
    let i = usize::from(kind);
    let misc = &mut p.misc;
    if misc.caches_written[i].is_some_and(|at| now.saturating_duration_since(at) < WRITE_EVERY) {
        misc.caches_unwritten |= 1 << kind;
        return;
    }
    misc.caches_written[i] = Some(now);
    misc.caches_unwritten &= !(1 << kind);
    write(map, g, kind);
}

/// Write cache `kind` as the player holds it.
fn write(map: &Map, g: Guid, kind: u8) {
    if let Some(p) = map.players.get(&g) {
        put(&map.realm, p, kind);
    }
}

fn put(realm: &Realm, p: &crate::game::map::player::Player, kind: u8) {
    let data = p
        .row
        .state
        .caches
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or_else(Vec::new, |(_, d)| d.clone());
    let now = u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
    let k = key(&p.account, crate::game::guid::counter(p.guid), kind);
    realm.store.set_account_data(&k, kind, now, data);
}

/// Write the caches changed since their last write ([`WRITE_EVERY`]): at
/// each save of the player, and as they leave the world.
pub fn flush(realm: &Realm, p: &mut crate::game::map::player::Player, now: std::time::Instant) {
    let unwritten = std::mem::take(&mut p.misc.caches_unwritten);
    for kind in (0..TYPES).filter(|k| unwritten & (1 << k) != 0) {
        p.misc.caches_written[usize::from(kind)] = Some(now);
        put(realm, p, kind);
    }
}

fn save_tutorials(map: &Map, g: Guid) {
    let Some(p) = map.players.get(&g) else {
        return;
    };
    let bytes: Vec<u8> = p
        .row
        .state
        .tutorials
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    store(map, g, TUTORIALS, bytes);
}

/// The cache requests, and the tutorials, kept for the account.
pub fn handle(map: &mut Map, g: Guid, msg: ClientOpcodeMessage) -> Option<ClientOpcodeMessage> {
    use ClientOpcodeMessage as C;
    match msg {
        // Read by wow_messages (as the harness sends it): decompressed.
        C::CMSG_UPDATE_ACCOUNT_DATA(m) => {
            let kind = m.data_type.as_int();
            if kind < TYPES && m.compressed_data.len() <= MAX_SIZE {
                update(map, g, kind, m.compressed_data);
            }
            None
        }
        C::CMSG_REQUEST_ACCOUNT_DATA(m) => {
            let p = map.players.get(&g)?;
            let kind = m.data_type.min(255) as u8;
            if kind < TYPES {
                let data = p
                    .row
                    .state
                    .caches
                    .iter()
                    .find(|(k, _)| *k == kind)
                    .map_or(&[][..], |(_, d)| &d[..]);
                p.outbound.send_frame(cache_frame(kind, data));
            }
            None
        }
        C::CMSG_TUTORIAL_FLAG(m) => {
            let p = map.players.get_mut(&g)?;
            let word = (m.tutorial_flag / 32) as usize;
            if let Some(w) = p.row.state.tutorials.get_mut(word) {
                *w |= 1 << (m.tutorial_flag % 32);
            }
            save_tutorials(map, g);
            None
        }
        C::CMSG_TUTORIAL_CLEAR => {
            map.players.get_mut(&g)?.row.state.tutorials = [u32::MAX; 8];
            save_tutorials(map, g);
            None
        }
        C::CMSG_TUTORIAL_RESET => {
            map.players.get_mut(&g)?.row.state.tutorials = [0; 8];
            save_tutorials(map, g);
            None
        }
        other => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_round_trips_and_digests_like_thistle_tea() {
        let frame = cache_frame(4, b"SET macro 1");
        // The frame's body is what the client sends back the other way.
        let back = parse_update(&frame[4..]).unwrap();
        assert_eq!(back, (4, b"SET macro 1".to_vec()));
        assert_eq!(digest(b""), [0; 16]);
        // MD5("abc").
        assert_eq!(
            digest(b"abc"),
            [
                0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1,
                0x7f, 0x72
            ]
        );
        assert_eq!(key("A", 7, 0), "A");
        assert_eq!(key("A", 7, 1), "A#7");
    }
}
