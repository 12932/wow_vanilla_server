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
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const BAT: u32 = 304;
const GULL: u32 = 305;

// The opcodes the checker knows; others it would only skip.
const CHECKED: &[u16] = &[0x0A9, 0x1F6, 0x0DD];

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
    let (mut a, _ga) = enter(&realm, "A", "Alice").await;
    for b in &bats {
        a.sees(*b).await;
    }
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
    let mut seen = AHashSet::new();
    let mut capture = Vec::new();
    let mut counts = std::collections::BTreeMap::new();
    let mut clients = vec![&mut a];
    clients.extend(crowd.iter_mut().map(|(c, _)| c));
    for c in clients {
        for frame in c.captured() {
            let opcode = u16::from_le_bytes([frame[2], frame[3]]);
            if CHECKED.contains(&opcode) && seen.insert(frame.to_vec()) {
                capture.extend_from_slice(&frame[..]);
                *counts.entry(opcode).or_insert(0u32) += 1;
            }
        }
    }
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "some update frames were captured"
    );
    assert!(
        counts.get(&0x0DD).copied().unwrap_or(0) > 0,
        "some movement frames were captured (the flyers)"
    );
    eprintln!("captured {} unique frame(s): {counts:?}", seen.len());

    let path = std::env::temp_dir().join(format!("wvs-capture-{}.bin", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&capture))
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
