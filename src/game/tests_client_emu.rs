//! The packets wvs sends, run through the 1.12.1 client's own parsing code.
//!
//! A scenario with movement in it (flyers on their paths, a crowd coming
//! into view, a player leaving) is played out against a real realm, every
//! frame the clients receive is kept (`Client::captured`), and the frames
//! are handed to `tools/client_check`, which drives `WoW.exe`'s own
//! handlers over them under an emulator. A packet that faults the client,
//! over-reads or builds an absurd array fails the test, before it could
//! reach a real client. See docs/TESTING.md.
//!
//! This is an extended test: it needs `tools/client_check/WoW.exe` (in the checkout)
//! and `uv` on the path, and it is slow (it emulates x86). Without either
//! it fails loudly, as the real-world tests do without their data.

use ahash::AHashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const BAT: u32 = 304;
const GULL: u32 = 305;

// The opcodes the checker knows; others it would only skip.
const CHECKED: &[u16] = &[0x0A9, 0x1F6, 0x0DD, 0x24B];

fn template(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        display_scales: [1.0, 0.0, 0.0, 0.0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_max: 1000,
        ..Default::default()
    }
}

fn spawn(guid: u32, entry: u32, at: (f32, f32, f32)) -> CreatureSpawn {
    CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: at.0,
        y: at.1,
        z: at.2,
        ..Default::default()
    }
}

async fn enter(realm: &Arc<crate::game::Realm>, account: &str, name: &str) -> (Client, Guid) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    c.heartbeat(START.0, START.1, START.2).await;
    (c, g)
}

/// A world of flyers on their paths: a circler over its spawn and a cyclic
/// one on a four-point loop, as `flyers_circle_and_fly_their_paths` has
/// them (no navmesh needed). They are always airborne and moving, so a
/// client that comes into view is told of them with a spline in the create
/// block — the shape that crashed the client.
fn flyers() -> GameData {
    let mut d = GameData::empty();
    let mut bat = template(BAT, "Bat", 7);
    bat.inhabit_type = 4;
    d.creature_templates.insert(BAT, bat);
    let mut gull = template(GULL, "Gull", 7);
    gull.inhabit_type = 4;
    d.creature_templates.insert(GULL, gull);
    let mut circler = spawn(1, BAT, (START.0 + 12.0, START.1, START.2 + 20.0));
    circler.movement_type = 1;
    circler.wander_distance = 10.0;
    let mut cyclic = spawn(2, GULL, (START.0 - 12.0, START.1, START.2 + 20.0));
    cyclic.movement_type = 3;
    d.creature_spawns.insert(0, vec![circler, cyclic]);
    d.waypoints.insert(
        2,
        (0..4)
            .map(|i| Waypoint {
                x: START.0 - 12.0 + 8.0 * (i as f32).cos(),
                y: START.1 + 8.0 * (i as f32).sin(),
                z: START.2 + 20.0,
                ..Default::default()
            })
            .collect(),
    );
    d
}

#[tokio::test]
#[ignore = "extended"]
async fn the_packets_wvs_sends_pass_the_client() {
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");

    let realm = harness::realm_with(flyers());
    let map = realm.maps.get(&realm, MapKey::world(0));
    let bats: Vec<Guid> = map
        .query(|m| m.creatures.iter().map(|(_, c)| c.guid).collect())
        .await
        .unwrap();
    assert_eq!(bats.len(), 2, "two flyers");

    // The first player sees the flyers and watches them fly for a while:
    // the create blocks (with splines) and SMSG_MONSTER_MOVE.
    let (mut a, ga) = enter(&realm, "A", "Alice").await;
    for b in &bats {
        a.sees(*b).await;
    }
    // Her spells missing a flyer every way one can (SMSG_SPELLLOGMISS, laid
    // out by hand as mangos-classic's `SendSpellMiss`: wow_messages reads a
    // target's reason as a u32).
    let bat = bats[0];
    map.query(move |m| {
        use crate::game::map::spells::outcome::{Hit, send_miss};
        for hit in [
            Hit::Miss,
            Hit::Resist,
            Hit::Dodge,
            Hit::Parry,
            Hit::Evade,
            Hit::Immune,
            Hit::Deflect,
            Hit::Absorb,
            Hit::Reflect,
        ] {
            send_miss(m, ga, bat, 133, hit);
        }
    })
    .await
    .unwrap();
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        a.captured();
    }

    // A crowd comes into view at once: each sees the others and the moving
    // flyers (more spline creates), and a late joiner's first batch is big
    // enough to arrive compressed (SMSG_COMPRESSED_UPDATE_OBJECT).
    let mut crowd = Vec::new();
    for i in 0..9 {
        let name = format!("Crowder{}", (b'a' + i) as char);
        let (mut c, g) = enter(&realm, &format!("C{i}"), &name).await;
        for b in &bats {
            c.sees(*b).await;
        }
        crowd.push((c, g));
    }
    a.sees(crowd[0].1).await;
    a.sees(crowd[8].1).await;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        a.captured();
        for (c, _) in &mut crowd {
            c.captured();
        }
    }

    // One leaves: the others are told to forget it (an out-of-range block).
    let (gone, gone_guid) = crowd.remove(0);
    gone.disconnect();
    a.forgets(gone_guid).await;
    for (c, _) in &mut crowd {
        let _ = c.sees_within(gone_guid, Duration::ZERO).await; // drain
    }

    // Every frame everyone saw, of a kind the checker knows, deduplicated.
    let mut clients = vec![&mut a];
    clients.extend(crowd.iter_mut().map(|(c, _)| c));
    let (capture, counts) = capture(clients);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "some update frames were captured"
    );
    assert!(
        counts.get(&0x0DD).copied().unwrap_or(0) > 0,
        "some movement frames were captured (the flyers)"
    );
    assert_eq!(
        counts.get(&0x24B).copied().unwrap_or(0),
        9,
        "every spell miss was captured"
    );
    run_checker(&check_py, &wow_exe, &capture);
}

/// The Stormwind Elite Guard (`map/elites.rs`), new to the world and made
/// in code: its create (the Royal Guard's look and weapons on an elite of
/// level 62) and its Charge's run, on the real data, pass the client.
#[tokio::test]
#[ignore = "extended"]
async fn the_elite_guards_packets_pass_the_client() {
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let dir = harness::real_data("db");
    if !dir.join("vmangos.sqlite").exists() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(&dir));
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (mut a, ga) = enter(&realm, "A", "Alice").await;
    // Her intro's camera over, she sees from where she stands.
    a.send(CMSG_COMPLETE_CINEMATIC {}).await;
    // An Elite Guard, held from fighting until she knows it, and a Defias
    // Thug 20 yards from it.
    let (elite, thug) = map
        .query(move |m| {
            let at = m.players[&ga].position();
            let near = |dx: f32| wow_world_messages::vanilla::Vector3d { x: at.x + dx, ..at };
            let elite = crate::game::map::scripts::summon(
                m,
                ga,
                crate::game::map::elites::ELITE_GUARD,
                near(5.0),
                0.0,
                None,
            )
            .expect("the Elite Guard");
            if let Some(s) = crate::game::map::event_ai::scripted_mut(m, elite) {
                s.passive = true;
            }
            let t = m
                .realm
                .data
                .creature_template(DEFIAS_THUG)
                .cloned()
                .unwrap();
            let counter = m.next_creature_counter;
            m.next_creature_counter += 1;
            let thug = m.add_creature(
                crate::game::map::creature::Creature::new(
                    crate::game::guid::unit(DEFIAS_THUG, counter | 0x80_0000),
                    &t,
                    5,
                    near(25.0),
                    0.0,
                )
                .with_base(),
            );
            // It lasts the fight out.
            if let Some(c) = m.creature_mut(thug) {
                c.fields
                    .set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
                c.fields
                    .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
            }
            (elite, thug)
        })
        .await
        .unwrap();
    a.sees(elite).await;
    a.sees(thug).await;
    // It takes the thug on: its Charge goes at once.
    map.query(move |m| {
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, elite) {
            s.passive = false;
            s.next_tick = None;
        }
        crate::game::map::ai::attacked(m, elite, thug);
        crate::game::map::event_ai::tick(m, elite);
    })
    .await
    .unwrap();
    // Its Charge, and its run.
    a.wait_for("its Charge", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.caster == elite && s.spell == CHARGE => Some(()),
        _ => None,
    })
    .await;
    a.wait_frame("its charge's run", 0x0DD, |b| {
        b.first().is_some_and(|mask| {
            let mut v = 0u64;
            let mut at = 1;
            for bit in 0..8 {
                if mask & (1 << bit) != 0 {
                    v |= u64::from(b.get(at).copied().unwrap_or(0)) << (bit * 8);
                    at += 1;
                }
            }
            Guid::new(v) == elite
        })
    })
    .await;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        a.captured();
    }
    let (capture, counts) = capture(vec![&mut a]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "its create was captured"
    );
    assert!(
        counts.get(&0x0DD).copied().unwrap_or(0) > 0,
        "its charge's run was captured"
    );
    run_checker(&check_py, &wow_exe, &capture);
}

/// A passenger's creates, with 1.12's transport in their movement (its
/// flag, the boat's whole GUID, the place on deck): the one Bob, on
/// Kalimdor, is told as the boat brings Alice there, and her own after the
/// crossing, the boat's before it; they pass the client.
#[tokio::test]
#[ignore = "extended"]
async fn a_passengers_creates_pass_the_client() {
    use crate::game::tests::transports::{BOAT, aboard_in, data};
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let realm = harness::realm_with(data());
    let boat = crate::game::guid::make(crate::game::guid::HighGuid::MoTransport, 0, BOAT);
    // Bob waits on Kalimdor, where the boat comes.
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.say(".go xyz 1000 1005 83.5312 1").await;
    b.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    b.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    b.sees(gb).await;
    // Alice boards on Eastern Kingdoms.
    let (mut a, ga) = enter(&realm, "A", "Alice").await;
    a.sees(boat).await;
    a.send(MSG_MOVE_HEARTBEAT_Client {
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::empty().set_on_transport(
                MovementInfo_MovementFlags_OnTransport {
                    transport: TransportInfo {
                        guid: boat,
                        position: Vector3d {
                            x: 1.0,
                            y: 0.5,
                            z: 2.0,
                        },
                        orientation: 0.0,
                        timestamp: 0,
                    },
                },
            ),
            timestamp: 0,
            position: Vector3d {
                x: START.0 + 1.0,
                y: START.1,
                z: START.2,
            },
            orientation: 0.0,
            fall_time: 0.0,
        },
    })
    .await;
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    let (first, _) = a
        .wait_for("her own create, aboard", |m| aboard_in(m, ga, boat))
        .await;
    assert!(first, "the boat's create before hers");
    b.wait_for("hers, aboard", |m| aboard_in(m, ga, boat)).await;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        a.captured();
        b.captured();
    }
    let (capture, counts) = capture(vec![&mut a, &mut b]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "the creates were captured"
    );
    run_checker(&check_py, &wow_exe, &capture);
}

/// A Defias Thug of Northshire, for the Elite Guard to charge, and its
/// Charge.
const DEFIAS_THUG: u32 = 38;
const CHARGE: u32 = 22911;

/// A corpse and the bones it turns to (`map/bones.rs`) pass the client:
/// the corpse's create to the living and to its ghost, its forget with
/// the bones' create at the reclaim, and bones whose owner has gone, to a
/// client that never knew them.
///
/// Alice and Bob are strangers, not a group, so Bob loses sight of her
/// ghost when she releases and is told of her anew when she rises: her
/// create and a values block for her come in one packet (what a ghost
/// rising in view of the living gets). That is a shape the 1.12 client
/// handles; `a_ghost_rising_before_a_stranger_passes_the_client` drives it
/// on its own and asserts it is present.
#[tokio::test]
#[ignore = "extended"]
async fn a_corpse_and_its_bones_pass_the_client() {
    use crate::game::map::death::{self, Death};
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let realm = harness::realm();
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (mut a, ga) = enter(&realm, "A", "Alice").await;
    let (mut b, _) = enter(&realm, "B", "Bob").await;
    b.sees(ga).await;
    // Alice dies and releases where she fell (the test world has no
    // graveyard): Bob is told of her corpse, and so is her ghost.
    map.query(move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 0);
        death::kill(m, ga);
    })
    .await
    .unwrap();
    a.send(CMSG_REPOP_REQUEST {}).await;
    a.wait_for("the teleport to the graveyard", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    let corpse = crate::game::guid::corpse(crate::game::guid::counter(ga));
    b.sees(corpse).await;
    a.sees(corpse).await;
    // She reclaims it: the corpse forgotten, the bones made.
    map.query(move |m| {
        let since = m.now - Duration::from_secs(60);
        m.players.get_mut(&ga).unwrap().death = Death::Ghost { since };
    })
    .await
    .unwrap();
    a.send(CMSG_RECLAIM_CORPSE { guid: corpse }).await;
    b.forgets(corpse).await;
    let bones = map
        .query(move |m| {
            m.corpses
                .values()
                .find(|c| c.owner == ga && c.is_bones())
                .map(|c| c.guid)
        })
        .await
        .unwrap()
        .expect("her bones");
    b.sees(bones).await;
    a.sees(bones).await;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        a.captured();
        b.captured();
    }
    // She logs out; a newcomer is told of her bones, naming one it never
    // knew.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    b.forgets(ga).await;
    let (mut c, _) = enter(&realm, "C", "Carol").await;
    c.sees(bones).await;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        b.captured();
        c.captured();
    }
    let (capture, counts) = capture(vec![&mut a, &mut b, &mut c]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "the corpse's and the bones' creates were captured"
    );
    run_checker(&check_py, &wow_exe, &capture);
}

/// A ghost rising in view of a living stranger: her create and a values
/// block for her in one packet, what a player coming into view while her
/// fields change gets. Alice and Bob are strangers, so Bob loses sight of
/// Alice while she is a ghost and is told of her afresh when she rises,
/// the same update her health, flags and auras change; the create and the
/// values ride one `SMSG_UPDATE_OBJECT`. The 1.12 client copies another
/// player's field groups into storage it lays down at the create, so this
/// packet is safe — the shape the earlier bones work grouped its players
/// to avoid and the checker's model took for an overrun at `0x466816`
/// (see docs/TESTING.md).
#[tokio::test]
#[ignore = "extended"]
async fn a_ghost_rising_before_a_stranger_passes_the_client() {
    use crate::game::fields::UNIT_FIELD_HEALTH;
    use crate::game::map::death;
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let realm = harness::realm();
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (mut a, ga) = enter(&realm, "A", "Alice").await;
    let (mut b, _) = enter(&realm, "B", "Bob").await;
    b.sees(ga).await;
    // Alice dies and becomes a ghost where she stands (no corpse, no
    // graveyard): a living stranger cannot see a ghost, so Bob is told to
    // forget her.
    map.query(move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 0);
        death::kill(m, ga);
        let now = m.now;
        death::become_ghost(m.players.get_mut(&ga).unwrap(), now);
        m.visibility_dirty.insert(ga);
    })
    .await
    .unwrap();
    b.forgets(ga).await;
    // She rises where she fell, next to Bob, her health, flags and auras
    // all changed: Bob is told of her anew and, the same update, of her
    // values.
    map.query(move |m| assert!(death::resurrect(m, ga, 1.0), "Alice rose"))
        .await
        .unwrap();
    // The create and the values arrive together, in one packet.
    b.wait_for("Alice's create and values in one packet", |m| {
        (harness::creates(m).contains(&ga) && harness::values(m).iter().any(|(g, _)| *g == ga))
            .then_some(())
    })
    .await;
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        a.captured();
        b.captured();
    }
    let (capture, counts) = capture(vec![&mut a, &mut b]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "her create and values were captured"
    );
    run_checker(&check_py, &wow_exe, &capture);
}

/// Every frame `clients` received, of a kind the checker knows,
/// deduplicated, back to back; and how many of each opcode.
fn capture(clients: Vec<&mut Client>) -> (Vec<u8>, std::collections::BTreeMap<u16, u32>) {
    let mut seen = AHashSet::new();
    let mut capture = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    for c in clients {
        for frame in c.captured() {
            let opcode = u16::from_le_bytes([frame[2], frame[3]]);
            if CHECKED.contains(&opcode) && seen.insert(frame.to_vec()) {
                capture.extend_from_slice(&frame[..]);
                *counts.entry(opcode).or_insert(0u32) += 1;
            }
        }
    }
    eprintln!("captured {} unique frame(s): {counts:?}", seen.len());
    (capture, counts)
}

/// Run a capture through the client's own parsers, failing if the client
/// would.
fn run_checker(check_py: &Path, wow_exe: &Path, capture: &[u8]) {
    let path = std::env::temp_dir().join(format!("wvs-capture-{}.bin", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(capture))
        .expect("write the capture");

    let out = Command::new("uv")
        .args([
            "run",
            "--python",
            "3.14",
            "--with",
            "unicorn",
            "--with",
            "pefile",
            "python",
            "-I",
            check_py.to_str().unwrap(),
            "--exe",
            wow_exe.to_str().unwrap(),
            path.to_str().unwrap(),
        ])
        .output()
        .expect("run the client-emulation checker through uv");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    eprintln!("{stderr}");
    assert!(
        out.status.success(),
        "the 1.12 client rejected a packet wvs sent:\n{stdout}\n{stderr}"
    );
}

/// `check.py` and `WoW.exe`, or `None` with a reason logged, so a setup
/// without them fails loudly rather than passing quietly.
fn checker() -> Option<(PathBuf, PathBuf)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let check_py = manifest.join("tools/client_check/check.py");
    let wow_exe = manifest.join("tools/client_check/WoW.exe");
    if !check_py.exists() {
        eprintln!("no checker at {}", check_py.display());
        return None;
    }
    if !wow_exe.exists() {
        eprintln!("no WoW.exe at {}", wow_exe.display());
        return None;
    }
    if Command::new("uv").arg("--version").output().is_err() {
        eprintln!("`uv` is not on the path");
        return None;
    }
    Some((check_py, wow_exe))
}
