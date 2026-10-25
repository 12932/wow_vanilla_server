//! The fight record (`map::record`, `[record]`, `.record`): a recorded
//! copy's file has its moves, casts, auras, blows and deaths as the server
//! took and told them, with the right fields; one not recorded writes
//! nothing; a GM starts, marks and stops it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{dummy, has, ready, set_health};
use crate::game::Realm;
use crate::game::data::spells::SpellTemplate;
use crate::game::harness::{self, Client};
use crate::game::map::auras::AURA_MOD_DECREASE_SPEED;
use crate::game::map::creature::Motion;
use crate::game::map::spells::{Targets, wire_targets};
use crate::game::map::{Map, MapHandle};
use crate::game::store::Store;

const SLOW: u32 = 990_700;
const BLAST: u32 = 990_701;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wvs-record-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// A realm of made-up data (a slow that lasts, a blast of 500) whose
/// config records `maps` into `dir`.
fn realm(dir: &Path, maps: Vec<u32>) -> Arc<Realm> {
    let data = data_with(|d| {
        let mut slow = bare(SLOW, 4);
        slow.duration_index = LASTS_15S;
        effect(&mut slow, 0, aura(AURA_MOD_DECREASE_SPEED, 0, -50, 6));
        let mut blast = bare(BLAST, 4);
        effect(&mut blast, 0, damage(500));
        insert(d, [slow, blast] as [SpellTemplate; 2]);
    });
    let mut cfg = harness::test_config();
    cfg.record = crate::config::RecordConfig {
        maps,
        every_ms: 50,
        dir: dir.to_path_buf(),
    };
    let (store, startup) = Store::open(":memory:").expect("an in-memory store");
    Realm::new(cfg, store, startup, data, None)
}

async fn on<T: Send + 'static>(map: &MapHandle, f: impl FnOnce(&mut Map) -> T + Send + 'static) -> T {
    map.query(f).await.expect("the map is up")
}

/// Wait for `check` to hold on the map, a short poll up to a limit.
async fn until(map: &MapHandle, what: &str, check: impl Fn(&Map) -> bool + Send + Clone + 'static) {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let c = check.clone();
        if on(map, move |m| c(m)).await {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// `n` more of the map's updates.
async fn updates(map: &MapHandle, n: u64) {
    let from = on(map, |m| m.stats.updates).await;
    until(map, "the map's updates", move |m| m.stats.updates >= from + n).await;
}

async fn cast_at(c: &Client, spell: u32, target: Guid) {
    c.send(CMSG_CAST_SPELL {
        spell,
        targets: wire_targets(&Targets {
            unit: Some(target),
            ..Default::default()
        }),
    })
    .await;
}

/// Every line of every file in `dir`, parsed.
fn lines(dir: &Path) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let Ok(files) = std::fs::read_dir(dir) else {
        return out;
    };
    for f in files.flatten() {
        let text = std::fs::read_to_string(f.path()).expect("the record reads");
        // A last line still being written (no newline yet) is left for later.
        for l in text.split_inclusive('\n').filter(|l| l.ends_with('\n')) {
            let l = l.trim_end();
            out.push(serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")));
        }
    }
    out
}

fn hex(g: Guid) -> String {
    format!("{:x}", g.guid())
}

fn of<'a>(ls: &'a [serde_json::Value], e: &str) -> Vec<&'a serde_json::Value> {
    ls.iter().filter(|l| l["e"] == e).collect()
}

/// A recorded copy: the player's heartbeat (its opcode, its place), the
/// kobold's chase (its spline, its points), the slow's cast, its going
/// off and its aura on the kobold, the blast's damage and how it landed,
/// the kobold's death at the player's hand, the samples of both and the
/// file's start and end, each line with its time.
#[tokio::test]
async fn a_recorded_copy_has_its_moves_casts_auras_blows_and_deaths() {
    let d = dir("fight");
    let realm = realm(&d, vec![0]);
    let (c, g, map) = human(&realm, "REC1", "Recorder", Class::Mage).await;
    let (kobold, start) = on(&map, move |m| {
        ready(m, g, 60, &[SLOW, BLAST]);
        let k = dummy(m, g, 60, 8.0);
        set_health(m, k, 300);
        (k, m.players[&g].position())
    })
    .await;
    let to = Vector3d {
        x: start.x + 0.3,
        ..start
    };
    c.heartbeat(to.x, to.y, to.z).await;
    until(&map, "the heartbeat is taken", move |m| {
        (m.players[&g].position().x - to.x).abs() < 0.01
    })
    .await;
    cast_at(&c, SLOW, kobold).await;
    until(&map, "the slow is on the kobold", move |m| has(m, kobold, SLOW)).await;
    // Hit, it comes at the player, eight yards off.
    until(&map, "the kobold runs at the player", move |m| {
        m.creature(kobold)
            .is_some_and(|k| matches!(k.motion, Motion::Moving(_)))
    })
    .await;
    updates(&map, 2).await;
    cast_at(&c, BLAST, kobold).await;
    until(&map, "the kobold dies", move |m| {
        m.creature(kobold).is_none_or(|k| !k.alive())
    })
    .await;
    updates(&map, 2).await;
    // Stopped: its file written to the end.
    on(&map, |m| drop(m.record.take())).await;
    let ls = lines(&d);
    let _ = std::fs::remove_dir_all(&d);
    assert!(ls.iter().all(|l| l["t"].as_u64().is_some_and(|t| t > 1_600_000_000_000)));
    let start_line = of(&ls, "start");
    assert_eq!(start_line.len(), 1, "{ls:?}");
    assert_eq!(start_line[0]["map"], 0);
    assert_eq!(start_line[0]["why"], "config");
    assert_eq!(of(&ls, "end").len(), 1);
    let (me, it) = (hex(g), hex(kobold));
    let units = of(&ls, "unit");
    assert!(
        units
            .iter()
            .any(|u| u["g"] == me.as_str() && u["kind"] == "player" && u["name"] == "Recorder")
    );
    assert!(units
        .iter()
        .any(|u| u["g"] == it.as_str() && u["kind"] == "creature" && u["entry"] == KOBOLD));
    // The heartbeat, as the client said it.
    let mv = of(&ls, "move");
    let beat = mv
        .iter()
        .find(|m| m["g"] == me.as_str() && m["op"] == "MSG_MOVE_HEARTBEAT")
        .unwrap_or_else(|| panic!("no heartbeat in {mv:?}"));
    assert!((beat["p"][0].as_f64().unwrap() - f64::from(to.x)).abs() < 0.02);
    assert!((beat["p"][1].as_f64().unwrap() - f64::from(to.y)).abs() < 0.02);
    // The kobold's chase: a spline from where it stood, toward the player.
    let spline = of(&ls, "spline")
        .into_iter()
        .find(|s| s["g"] == it.as_str())
        .unwrap_or_else(|| panic!("no spline of the kobold in {ls:?}"));
    assert_eq!(spline["kind"], "chase");
    assert!(spline["pts"].as_array().is_some_and(|p| p.len() >= 2));
    assert!(spline["ms"].as_u64().is_some_and(|ms| ms > 0));
    // The slow: cast, gone off, on the kobold from the player.
    let cast = of(&ls, "cast");
    assert!(cast.iter().any(|x| x["g"] == me.as_str() && x["sp"] == SLOW && x["tg"] == it.as_str() && x["ms"] == 0));
    assert!(of(&ls, "go").iter().any(|x| x["g"] == me.as_str() && x["sp"] == SLOW));
    assert!(of(&ls, "aura").iter().any(|x| {
        x["g"] == it.as_str() && x["by"] == me.as_str() && x["sp"] == SLOW && x["on"] == 1
    }));
    // The blast: how it landed, the damage that got through, the death.
    assert!(of(&ls, "out").iter().any(|x| {
        x["by"] == me.as_str() && x["g"] == it.as_str() && x["sp"] == BLAST && (x["out"] == "hit" || x["out"] == "crit")
    }));
    let dmg = of(&ls, "dmg");
    let blow = dmg
        .iter()
        .find(|x| x["by"] == me.as_str() && x["g"] == it.as_str() && x["sp"] == BLAST)
        .unwrap_or_else(|| panic!("no damage in {dmg:?}"));
    assert_eq!(blow["left"], 0);
    assert!(blow["n"].as_u64().is_some_and(|n| n >= 300));
    assert!(of(&ls, "death")
        .iter()
        .any(|x| x["g"] == it.as_str() && x["by"] == me.as_str() && x["sp"] == BLAST));
    // Its aura went as it died.
    assert!(of(&ls, "aura").iter().any(|x| x["g"] == it.as_str() && x["sp"] == SLOW && x["on"] == 0));
    // Both sampled: the player's health, the kobold's at the last dead.
    let pos = of(&ls, "pos");
    assert!(pos.iter().any(|p| p["g"] == me.as_str() && p["hp"].as_u64().is_some_and(|h| h > 0)));
    assert!(pos.iter().any(|p| p["g"] == it.as_str() && p["dead"] == 1));
    drop(c);
}

/// A copy not recorded writes nothing, whatever happens in it.
#[tokio::test]
async fn a_copy_not_recorded_writes_nothing() {
    let d = dir("off");
    let realm = realm(&d, Vec::new());
    let (c, g, map) = human(&realm, "REC2", "Unrecorded", Class::Mage).await;
    let kobold = on(&map, move |m| {
        ready(m, g, 60, &[SLOW, BLAST]);
        dummy(m, g, 60, 5.0)
    })
    .await;
    cast_at(&c, SLOW, kobold).await;
    until(&map, "the slow is on the kobold", move |m| has(m, kobold, SLOW)).await;
    updates(&map, 3).await;
    assert!(on(&map, |m| m.record.is_none()).await);
    assert!(!d.exists(), "{} was made", d.display());
    drop(c);
}

/// `.record on <name>` starts a GM's record of their copy, `.record mark`
/// marks it, `.record` tells of it and `.record off` ends it; a name that
/// would leave the directory is refused.
#[tokio::test]
async fn a_gm_starts_marks_and_stops_a_record() {
    let d = dir("gm");
    let realm = realm(&d, Vec::new());
    let (mut c, g, map) = human(&realm, "REC3", "Marker", Class::Warrior).await;
    c.say(".record on ../out").await;
    c.system_message("A record's name").await;
    c.say(".record mark nothing yet").await;
    c.system_message("Not recording").await;
    c.say(".record on attempts").await;
    c.system_message("Recording 0 #0 to attempts.jsonl").await;
    c.say(".record mark attempt 1 start").await;
    c.system_message("Marked: attempt 1 start").await;
    c.say(".record").await;
    c.system_message("Recording 0 #0 to attempts.jsonl for").await;
    updates(&map, 2).await;
    c.say(".record off").await;
    c.system_message("Recorded to attempts.jsonl").await;
    // Its end is written by its thread, which the map doesn't wait for.
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    let ls = loop {
        let ls = lines(&d);
        if !of(&ls, "end").is_empty() || tokio::time::Instant::now() >= deadline {
            break ls;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let _ = std::fs::remove_dir_all(&d);
    assert!(Path::new(&d).join("attempts.jsonl") != d);
    assert_eq!(of(&ls, "start")[0]["why"], "gm");
    let mark = of(&ls, "mark");
    assert_eq!(mark.len(), 1);
    assert_eq!(mark[0]["text"], "attempt 1 start");
    assert_eq!(mark[0]["by"], hex(g).as_str());
    assert!(of(&ls, "unit").iter().any(|u| u["g"] == hex(g).as_str()));
    assert_eq!(of(&ls, "end").len(), 1);
    drop(c);
}
