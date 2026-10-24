//! A crowd's values in packets shared among it (`map::values`): after an
//! update in which every player of a crowd changed, each client is told
//! of each other player's change once, in its public fields only, and of
//! its own once, private fields and all; no packet carries two blocks of
//! one object; and the clients of the crowd get their values in the same
//! few packets, not one batch each.

use std::sync::Arc;

use ahash::AHashMap;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};

/// A values block of a player: its object, health and experience (if
/// sent).
type Block = (Guid, Option<i32>, Option<i32>);

/// What a client was told of each player's values since the last look:
/// each block, and how many packets carried them.
fn told(c: &mut Client) -> (Vec<Block>, usize) {
    let mut blocks = Vec::new();
    let mut packets = 0;
    for m in std::mem::take(&mut c.backlog) {
        if !matches!(
            m,
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(_)
                | ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(_)
        ) {
            continue;
        }
        let values = harness::values(&m);
        if values.is_empty() {
            continue;
        }
        packets += 1;
        let mut objects: Vec<Guid> = values.iter().map(|(g, _)| *g).collect();
        objects.sort_unstable();
        let n = objects.len();
        objects.dedup();
        assert_eq!(
            objects.len(),
            n,
            "{}: a packet with two blocks of one object",
            c.account
        );
        for (g, mask) in values {
            if let UpdateMask::Player(p) = mask {
                blocks.push((g, p.unit_health(), p.player_xp()));
            }
        }
    }
    (blocks, packets)
}

#[tokio::test]
async fn a_crowd_is_told_each_change_once_in_shared_packets() {
    let realm = harness::realm();
    let n = 24;
    let mut clients = Vec::new();
    for i in 0..n {
        let mut c = Client::connect(&realm, &format!("crowdv{i}"));
        let letters: String = format!("{i:02}")
            .bytes()
            .map(|b| char::from(b'a' + (b - b'0')))
            .collect();
        c.enter(&format!("Crowd{letters}"), Race::Human, Class::Warrior)
            .await;
        clients.push(c);
    }
    let guids: Vec<Guid> = clients.iter().map(|c| c.guid).collect();
    for c in &mut clients {
        for &g in &guids {
            if g != c.guid {
                c.sees(g).await;
            }
        }
    }
    let map = realm.maps.get(&realm, MapKey::world(0));
    harness::an_update(&map).await;
    let mut seen_before = Vec::new();
    for c in &mut clients {
        c.clear();
        seen_before.push(c.captured().len());
    }
    // Every player at a health of its own (its maximum too, so that it
    // doesn't regenerate in the updates watched), and every third
    // given experience (a field only its own client sees), in one go.
    let changes: Arc<Vec<(Guid, u32, Option<u32>)>> = Arc::new(
        guids
            .iter()
            .enumerate()
            .map(|(i, &g)| (g, 20 + i as u32, (i % 3 == 0).then_some(7 + i as u32)))
            .collect(),
    );
    let each = changes.clone();
    let updates = map
        .query(move |m: &mut Map| {
            for &(g, health, xp) in each.iter() {
                let p = m.players.get_mut(&g).expect("in the map");
                p.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
                p.fields.set_u32(UNIT_FIELD_HEALTH, health);
                if let Some(xp) = xp {
                    p.fields.set_u32(PLAYER_XP, xp);
                }
            }
            m.stats.updates
        })
        .await
        .expect("the map answers");
    // Its next update tells them; then a second, so that all of it is in.
    while map.query(|m| m.stats.updates).await.unwrap_or(u64::MAX) < updates + 2 {
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let want: AHashMap<Guid, (u32, Option<u32>)> =
        changes.iter().map(|&(g, h, x)| (g, (h, x))).collect();
    let mut most_packets = 0;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    for c in &mut clients {
        // What has come, read.
        let _ = c.gets(std::time::Duration::ZERO, |_| None::<()>).await;
        let (blocks, packets) = told(c);
        most_packets = most_packets.max(packets);
        for &g in &guids {
            let of: Vec<_> = blocks.iter().filter(|(o, _, _)| *o == g).collect();
            assert_eq!(
                of.len(),
                1,
                "{}: told of {g:?} {} times",
                c.account,
                of.len()
            );
            let (_, health, xp) = of[0];
            let (want_health, want_xp) = want[&g];
            assert_eq!(
                *health,
                Some(want_health as i32),
                "{}: {g:?}'s health",
                c.account
            );
            if g == c.guid {
                // Its own: private fields too.
                assert_eq!(
                    *xp,
                    want_xp.map(|x| x as i32),
                    "{}: its own experience",
                    c.account
                );
            } else {
                assert_eq!(
                    *xp, None,
                    "{}: {g:?}'s experience is theirs alone",
                    c.account
                );
            }
        }
    }
    // The crowd's blocks came in a few packets each, not one each.
    assert!(
        most_packets <= 8,
        "{most_packets} packets of values for one client"
    );
    // And the same packets: one frame, built once, reached most of them.
    let mut reached: AHashMap<usize, usize> = AHashMap::new();
    for (c, from) in clients.iter_mut().zip(seen_before) {
        for f in &c.captured()[from..] {
            let opcode = u16::from_le_bytes([f[2], f[3]]);
            if opcode == 0x0A9 || opcode == 0x1F6 {
                *reached.entry(f.as_ptr() as usize).or_default() += 1;
            }
        }
    }
    let widest = reached.values().copied().max().unwrap_or(0);
    assert!(
        widest >= n / 2,
        "the widest-shared values packet reached {widest} of {n}"
    );
}

/// What each client of `c` was sent since the last look: every values
/// block, as its object and fields, sorted.
fn blocks_of(c: &mut crate::game::map::vis_bench::Crowd) -> Vec<Vec<(Guid, String)>> {
    (0..c.guids.len())
        .map(|i| {
            let mut got: Vec<(Guid, String)> = c
                .take_frames(i)
                .iter()
                .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
                .flat_map(|m| harness::values(&m))
                .map(|(g, mask)| (g, format!("{mask:?}")))
                .collect();
            got.sort();
            got
        })
        .collect()
}

/// The shared packets carry each client exactly the blocks it got before
/// they were shared, one by one into its batch: in a crowd wider than
/// sight (each sees some of the others, many at the edge), over rounds of
/// moves and of changes to a part of the crowd or all of it.
#[tokio::test]
async fn shared_values_are_the_blocks_each_client_had_one_by_one() {
    use crate::game::map::vis_bench::Crowd;
    let n = 300;
    let radius = 90.0;
    let mut before = Crowd::new(n, radius);
    let mut after = Crowd::new(n, radius);
    before.values_one_by_one(true);
    for round in 0..12u32 {
        for c in [&mut before, &mut after] {
            if round % 3 == 2 {
                c.shift(5, round as usize % 5, 9.0);
                c.refresh();
            }
            c.hurt(1 + round as usize % 4, round as usize % 3, round * 7);
            c.flush_values();
        }
        let (a, b) = (blocks_of(&mut before), blocks_of(&mut after));
        assert_eq!(a.len(), b.len());
        let told: usize = a.iter().map(Vec::len).sum();
        assert!(told > 0, "round {round}: something was told");
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(x, y, "round {round}: client {i}'s blocks");
        }
    }
}
