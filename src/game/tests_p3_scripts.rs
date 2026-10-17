//! Thistle Tea's encounter scripts (its `creature_script` ports of vmangos's
//! C++ AIs), as this server imports them (`data/tt_scripts.rs`), driven
//! through their phases on the real world: Molten Core's lieutenants and
//! trash here. The helpers serve the other `tests_p3_scripts_*` files.
//!
//! A scenario makes an event due rather than waiting out its timer, and
//! reads what came of it as a player's client does.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::combat::dist;
use crate::game::map::{Map, MapHandle, MapKey, ai, auras, event_ai, scripts};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

/// Northshire, where humans start.
pub(super) const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

pub(super) fn realm() -> Option<Arc<Realm>> {
    let dir = DB.as_path();
    if !dir.join("vmangos.sqlite").exists() {
        return None;
    }
    Some(harness::realm_with(crate::game::data::load::load(dir)))
}

/// Whether `SCRIPTS_ONLY` (a comma list) leaves scenario `name` in.
pub(super) fn wanted(name: &str) -> bool {
    let yes = std::env::var("SCRIPTS_ONLY").map_or(true, |v| v.split(',').any(|w| w == name));
    if yes {
        eprintln!("scripts: {name}");
    }
    yes
}

/// A level 60 warrior at the start, past the intro, whom nothing aggroes
/// and nothing harms (a GM in god mode).
pub(super) async fn hero(realm: &Arc<Realm>, name: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        p.fields.set_flag(
            PLAYER_FLAGS,
            crate::game::map::player::PLAYER_FLAGS_GM,
            true,
        );
        p.misc.god = true;
    })
    .await;
    toughen(&map, g).await;
    (c, g, map)
}

/// `g`, a player or creature, with ten million health.
pub(super) async fn toughen(map: &MapHandle, g: Guid) {
    map.query(move |m| tough(m, g)).await;
}

pub(super) fn tough(m: &mut Map, g: Guid) {
    if let Some(p) = m.players.get_mut(&g) {
        p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 10_000_000);
        p.fields.set_u32(UNIT_FIELD_HEALTH, 10_000_000);
    } else if let Some(c) = m.creature_mut(g) {
        c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 10_000_000);
        c.fields.set_u32(UNIT_FIELD_HEALTH, 10_000_000);
        c.base = crate::game::map::derive::UnitBase::capture(&c.fields, c.speeds.run);
    }
}

/// `dx`, `dy` yards from the start.
pub(super) fn at(dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
    }
}

/// `c` at `at` in map `id`'s world or copy, by a teleport across maps:
/// the map they are in then.
pub(super) async fn go(realm: &Arc<Realm>, c: &mut Client, id: u32, at: Vector3d) -> MapHandle {
    let g = c.guid;
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| m.teleport(g, MapKey::world(id), at, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    let to = realm.registry.online(g).unwrap().map();
    toughen(&to, g).await;
    to
}

/// A [`hero`] in a fresh copy of map `id` at `to`, every creature within
/// `clear` yards of it gone (but those of `keep`'s entries), so a scenario
/// has the place to itself.
pub(super) async fn hero_in(
    realm: &Arc<Realm>,
    name: &str,
    id: u32,
    to: Vector3d,
    clear: f32,
    keep: &'static [u32],
) -> (Client, Guid, MapHandle) {
    let (mut c, g, _) = hero(realm, name).await;
    let map = go(realm, &mut c, id, to).await;
    map.query(move |m| clear_around(m, to, clear, keep)).await;
    (c, g, map)
}

/// Another [`hero`], in `map`'s copy at `to`.
pub(super) async fn joins(
    realm: &Arc<Realm>,
    name: &str,
    map: &MapHandle,
    to: Vector3d,
) -> (Client, Guid) {
    let (mut c, g, from) = hero(realm, name).await;
    let key = map.query(|m| m.key).await.unwrap();
    from.call(move |m| m.teleport(g, key, to, 0.0));
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(g).await;
    toughen(map, g).await;
    (c, g)
}

/// Every creature within `radius` of `at` gone, but those of `keep`'s
/// entries.
pub(super) fn clear_around(m: &mut Map, at: Vector3d, radius: f32, keep: &[u32]) {
    let near: Vec<Guid> = m
        .creatures
        .iter()
        .filter(|(_, c)| !keep.contains(&c.entry) && dist(c.position, at) <= radius)
        .map(|(_, c)| c.guid)
        .collect();
    for g in near {
        scripts::despawn(m, g);
    }
}

/// `dx`, `dy` yards from `base`.
pub(super) fn off(base: Vector3d, dx: f32, dy: f32) -> Vector3d {
    Vector3d {
        x: base.x + dx,
        y: base.y + dy,
        z: base.z,
    }
}

/// [`summon`] from a handle, at `to`.
pub(super) async fn summon_near(map: &MapHandle, entry: u32, to: Vector3d) -> Guid {
    map.query(move |m| summon(m, entry, to, 0.0)).await.unwrap()
}

/// A creature of `entry` at `at`, facing `o`, its scripts running, nobody's
/// summon.
pub(super) fn summon(m: &mut Map, entry: u32, at: Vector3d, o: f32) -> Guid {
    let g = scripts::summon(m, Guid::zero(), entry, at, o, None).expect("the creature");
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.summoner = None;
    }
    g
}

/// [`summon`] from a handle, `dx`, `dy` from the start.
pub(super) async fn summon_at(map: &MapHandle, entry: u32, dx: f32, dy: f32) -> Guid {
    map.query(move |m| summon(m, entry, at(dx, dy), 0.0))
        .await
        .unwrap()
}

/// Those summoned go, so the next scenario starts clear.
pub(super) fn clear(map: &MapHandle, gone: Vec<Guid>) {
    map.call(move |m| {
        for g in gone {
            scripts::despawn(m, g);
        }
    });
}

/// The index of `g`'s event `id` (Thistle Tea's: its entry and number).
pub(super) fn index(m: &Map, g: Guid, id: u32) -> usize {
    let entry = m.creature(g).map(event_ai::ai_entry).expect("the creature");
    m.realm.data.scripts.ai_events[&entry]
        .iter()
        .position(|e| e.id == id)
        .unwrap_or_else(|| panic!("event {id}"))
}

/// `g`'s event `id` due now, and its timed events looked at.
pub(super) fn due(m: &mut Map, g: Guid, id: u32) {
    let i = index(m, g, id);
    let now = m.now;
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.timers[i] = Some(now);
        s.next_tick = None;
    }
    event_ai::tick(m, g);
}

/// [`due`] from a handle.
pub(super) async fn fire(map: &MapHandle, g: Guid, id: u32) {
    map.query(move |m| due(m, g, id)).await;
}

/// How long until `g`'s event `id` is due; `None` if it isn't timed.
pub(super) fn left(m: &Map, g: Guid, id: u32) -> Option<Duration> {
    let i = index(m, g, id);
    let s = event_ai::scripted(m, g)?;
    s.timers[i].map(|t| t.saturating_duration_since(m.now))
}

/// [`left`] from a handle.
pub(super) async fn timer(map: &MapHandle, g: Guid, id: u32) -> Option<Duration> {
    map.query(move |m| left(m, g, id)).await.flatten()
}

/// Whether `left` is `secs` seconds on, or within the update since.
pub(super) fn about(left: Option<Duration>, secs: f32) -> bool {
    left.is_some_and(|l| {
        let l = l.as_secs_f32();
        l <= secs + 0.05 && l > secs - 1.0
    })
}

/// Whether `left` falls in `lo..=hi` seconds (less an update).
pub(super) fn between(left: Option<Duration>, lo: f32, hi: f32) -> bool {
    left.is_some_and(|l| {
        let l = l.as_secs_f32();
        l <= hi + 0.05 && l > lo - 1.0
    })
}

/// Whether `g`'s event `id` has fired for good (a one-off).
pub(super) fn spent(m: &Map, g: Guid, id: u32) -> bool {
    let i = index(m, g, id);
    event_ai::scripted(m, g).is_some_and(|s| s.disabled[i])
}

fn aimed_at(t: &SpellCastTargets) -> Guid {
    t.target_flags
        .get_unit()
        .map_or(Guid::zero(), |u| u.unit_target)
}

/// A cast of `spell` by `caster` that `c` sees go within `within`: the
/// unit it was aimed at (zero for none).
pub(super) async fn cast_of(
    c: &mut Client,
    caster: Guid,
    spell: u32,
    within: Duration,
) -> Option<Guid> {
    c.gets(within, move |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.caster == caster && s.spell == spell => {
            Some(aimed_at(&s.targets))
        }
        _ => None,
    })
    .await
}

/// The casts by `caster` that `c` sees go over `within`: each spell and
/// the unit it was aimed at.
pub(super) async fn casts_by(c: &mut Client, caster: Guid, within: Duration) -> Vec<(u32, Guid)> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return out;
        }
        let seen = c
            .gets(left, move |m| match m {
                ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.caster == caster => {
                    Some((s.spell, aimed_at(&s.targets)))
                }
                _ => None,
            })
            .await;
        match seen {
            Some(cast) => out.push(cast),
            None => return out,
        }
    }
}

/// A text `c` hears `speaker` say, yell or emote within `within`.
pub(super) async fn heard(c: &mut Client, speaker: Guid, within: Duration) -> Option<String> {
    c.gets(within, move |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(s) => {
            let from = match &s.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterSay { sender1, .. }
                | SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. }
                | SMSG_MESSAGECHAT_ChatType::MonsterEmote {
                    monster: sender1, ..
                } => *sender1,
                SMSG_MESSAGECHAT_ChatType::RaidBossEmote { monster, .. } => *monster,
                _ => return None,
            };
            (from == speaker).then(|| s.message.clone())
        }
        _ => None,
    })
    .await
}

/// `g`'s health set to `pct` percent of its most.
pub(super) fn set_pct(m: &mut Map, g: Guid, pct: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
        f.set_u32(
            UNIT_FIELD_HEALTH,
            (max as u64 * pct as u64 / 100).max(1) as u32,
        );
    }
}

pub(super) fn health(m: &Map, g: Guid) -> (u32, u32) {
    m.unit_fields(g).map_or((0, 0), |f| {
        (
            f.get_u32(UNIT_FIELD_HEALTH),
            f.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    })
}

pub(super) fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// `g`'s threat on `on`.
pub(super) fn threat(m: &Map, g: Guid, on: Guid) -> f32 {
    m.creature(g)
        .and_then(|c| c.threat.iter().find(|(u, _)| *u == on).map(|(_, t)| *t))
        .unwrap_or(0.0)
}

pub(super) fn victim(m: &Map, g: Guid) -> Guid {
    m.creature(g).map_or(Guid::zero(), |c| c.combat.victim)
}

pub(super) fn fighting(m: &Map, g: Guid) -> bool {
    m.creature(g).is_some_and(|c| c.combat.attacking)
}

/// Out of its fight and home from it.
pub(super) fn at_peace(m: &Map, g: Guid) -> bool {
    m.creature(g).is_some_and(|c| {
        !c.combat.attacking
            && !c.evading
            && !c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
    })
}

/// Player `g` put at `to` at once, for the server.
pub(super) fn put(m: &mut Map, g: Guid, to: Vector3d) {
    if let Some(p) = m.players.get_mut(&g) {
        p.movement.position = to;
    }
}

/// A creature to fight that stands, swings at nothing, casts nothing and
/// lasts: a Stormwind protector, nobody's friend here.
pub(super) fn dummy(m: &mut Map, to: Vector3d) -> Guid {
    let g = summon(m, 487, to, 0.0);
    if let Some(s) = event_ai::scripted_mut(m, g) {
        s.passive = true;
        s.no_melee = true;
        s.no_combat_movement = true;
        s.spell_list = Some(0);
    }
    tough(m, g);
    g
}

/// Wait for `g` to stand still (a charge or a walk over): where it stands.
pub(super) async fn settled(map: &MapHandle, g: Guid) -> Vector3d {
    let mut last = map.query(move |m| m.position_of(g)).await.flatten();
    for _ in 0..40 {
        sleep_ms(150).await;
        let now = map.query(move |m| m.position_of(g)).await.flatten();
        if now.zip(last).is_some_and(|(a, b)| dist(a, b) < 0.01) {
            break;
        }
        last = now;
    }
    last.expect("still here")
}

pub(super) async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// Wait, a little at a time, up to `ms`, for `what` to hold.
pub(super) async fn until(
    map: &MapHandle,
    ms: u64,
    what: impl Fn(&mut Map) -> bool + Send + Clone + 'static,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        let w = what.clone();
        if map.query(move |m| w(m)).await.unwrap_or(false) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        sleep_ms(25).await;
    }
}

/// Ragnaros's lair, by the lava, in a copy of Molten Core: where its
/// scenarios fight, nothing near but a Lava Surger (sent away).
const LAIR: Vector3d = Vector3d {
    x: 840.0,
    y: -812.0,
    z: -229.6,
};

fn lair(dx: f32, dy: f32) -> Vector3d {
    off(LAIR, dx, dy)
}

/// A [`hero`] in a fresh copy of Molten Core, in Ragnaros's lair.
async fn raider(realm: &Arc<Realm>, name: &str) -> (Client, Guid, MapHandle) {
    hero_in(realm, name, 409, LAIR, 120.0, &[]).await
}

const LUCIFRON: u32 = 12118;
const SHAZZRAH: u32 = 12264;
const GARR: u32 = 12057;
const FIRESWORN: u32 = 12099;
const GEDDON: u32 = 12056;
const SULFURON: u32 = 12098;
const FLAMEWAKER_PRIEST: u32 = 11662;
const GOLEMAGG: u32 = 11988;
const CORE_RAGER: u32 = 11672;

/// Lucifron: Impending Doom ten seconds into the fight and every twenty,
/// his Curse at twenty and every fifteen, both on himself (they reach the
/// raid around him); Shadow Shock at an attacker every six.
async fn lucifron(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Lucy").await;
    let boss = summon_near(&map, LUCIFRON, lair(5.0, 0.0)).await;
    c.sees(boss).await;
    map.query(move |m| ai::attacked(m, boss, me)).await;
    for (id, first) in [(1211802, 10.0), (1211803, 20.0), (1211804, 6.0)] {
        let t = timer(&map, boss, id).await;
        assert!(about(t, first), "{id}: {t:?}, not {first} s from the pull");
    }
    for (id, spell, repeat, at_me) in [
        (1211802, 19702, 20.0, false),
        (1211803, 19703, 15.0, false),
        (1211804, 19460, 6.0, true),
    ] {
        c.clear();
        fire(&map, boss, id).await;
        let target = cast_of(&mut c, boss, spell, Duration::from_secs(2))
            .await
            .unwrap_or_else(|| panic!("{spell} cast"));
        if at_me {
            assert_eq!(target, me, "{spell} at his attacker");
        }
        let t = timer(&map, boss, id).await;
        assert!(about(t, repeat), "{spell} again in {repeat} s: {t:?}");
    }
    clear(&map, vec![boss]);
}

/// Sulfuron Harbinger inspires a Flamewaker Priest near him, then himself:
/// thirteen seconds in, and every twenty to twenty-six.
async fn sulfuron(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Sulfy").await;
    let boss = summon_near(&map, SULFURON, lair(5.0, 0.0)).await;
    let priest = summon_near(&map, FLAMEWAKER_PRIEST, lair(8.0, 3.0)).await;
    let far = summon_near(&map, FLAMEWAKER_PRIEST, lair(5.0, 60.0)).await;
    c.sees(boss).await;
    map.query(move |m| {
        for g in [priest, far] {
            if let Some(s) = event_ai::scripted_mut(m, g) {
                s.passive = true;
                s.spell_list = Some(0);
            }
        }
        ai::attacked(m, boss, me);
    })
    .await;
    assert!(about(timer(&map, boss, 1209803).await, 13.0));
    fire(&map, boss, 1209803).await;
    assert!(
        until(&map, 2000, move |m| has(m, boss, 19779)
            && has(m, priest, 19779))
        .await,
        "Inspire on the priest and on himself"
    );
    assert!(
        !map.query(move |m| has(m, far, 19779)).await.unwrap(),
        "not on a priest out of reach"
    );
    assert!(between(timer(&map, boss, 1209803).await, 20.0, 26.0));
    clear(&map, vec![boss, priest, far]);
}

/// Baron Geddon below five percent stops: his casts cut off, he stands, and
/// Armageddon goes with his emote; his timed spells hold from then on.
async fn geddon(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Geddy").await;
    let boss = summon_near(&map, GEDDON, lair(5.0, 0.0)).await;
    c.sees(boss).await;
    map.query(move |m| {
        ai::attacked(m, boss, me);
        set_pct(m, boss, 4);
    })
    .await;
    c.clear();
    map.query(move |m| {
        if let Some(s) = event_ai::scripted_mut(m, boss) {
            s.next_tick = None;
        }
        event_ai::tick(m, boss);
    })
    .await;
    assert!(
        cast_of(&mut c, boss, 20478, Duration::from_secs(2))
            .await
            .is_some(),
        "Armageddon"
    );
    let (phase, chases) = map
        .query(move |m| (event_ai::phase(m, boss), event_ai::combat_movement(m, boss)))
        .await
        .unwrap();
    assert_eq!((phase, chases), (1, false), "he stands for it");
    // Nothing timed goes in his last phase.
    c.clear();
    for id in [1205602, 1205603, 1205604] {
        fire(&map, boss, id).await;
    }
    let after = casts_by(&mut c, boss, Duration::from_millis(600)).await;
    assert!(
        !after.iter().any(|(s, _)| [20475, 19659, 19695].contains(s)),
        "{after:?}"
    );
    clear(&map, vec![boss]);
}

/// Garr: each Firesworn that dies enrages him (its death's spell on him,
/// his Enrage on himself), and erupts as it dies; six minutes in, he makes
/// one erupt every twenty seconds, and that one doesn't erupt again at
/// its death.
async fn garr(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Garry").await;
    let boss = summon_near(&map, GARR, lair(5.0, 0.0)).await;
    let first = summon_near(&map, FIRESWORN, lair(9.0, 2.0)).await;
    let second = summon_near(&map, FIRESWORN, lair(9.0, -2.0)).await;
    c.sees(boss).await;
    c.sees(first).await;
    c.sees(second).await;
    map.query(move |m| {
        for g in [boss, first, second] {
            tough(m, g);
        }
        ai::attacked(m, boss, me);
    })
    .await;
    // His pull binds them to him.
    assert!(
        until(&map, 2000, move |m| has(m, first, 23487)
            && has(m, second, 23487))
        .await,
        "Separation Anxiety on his Firesworn"
    );
    assert!(
        about(timer(&map, boss, 1205704).await, 360.0),
        "six minutes"
    );
    c.clear();
    map.query(move |m| crate::game::map::combat::deal_damage(m, me, first, 20_000_000))
        .await;
    assert!(
        cast_of(&mut c, first, 19497, Duration::from_secs(2))
            .await
            .is_some(),
        "Eruption as it dies"
    );
    assert!(
        until(&map, 2000, move |m| has(m, boss, 19516)).await,
        "Garr enraged"
    );
    // The six minutes up: the one left is made to erupt.
    c.clear();
    fire(&map, boss, 1205704).await;
    assert_eq!(
        cast_of(&mut c, boss, 20482, Duration::from_secs(2)).await,
        Some(second),
        "Garr's spell at the one left"
    );
    assert!(
        cast_of(&mut c, second, 20483, Duration::from_secs(2))
            .await
            .is_some(),
        "its Massive Eruption"
    );
    assert!(
        about(timer(&map, boss, 1205704).await, 20.0),
        "every twenty"
    );
    let phase = map
        .query(move |m| event_ai::phase(m, second))
        .await
        .unwrap();
    assert_eq!(phase, 1, "erupting");
    c.clear();
    map.query(move |m| {
        if m.creature(second).is_some_and(|c| c.alive()) {
            crate::game::map::combat::deal_damage(m, me, second, 20_000_000);
        }
    })
    .await;
    let after = casts_by(&mut c, second, Duration::from_millis(600)).await;
    assert!(
        !after.iter().any(|(s, _)| *s == 19497),
        "no second eruption: {after:?}"
    );
    clear(&map, vec![boss, first, second]);
}

/// Shazzrah's Blink, his Gate: beside a random player, every attacker's
/// threat gone, each of them still on his list. With none ahead he keeps to
/// whom he was fighting, as Thistle Tea's threat does at a tie.
async fn shazzrah(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Shazzy").await;
    let (_d, other) = joins(realm, "Blinky", &map, LAIR).await;
    let boss = summon_near(&map, SHAZZRAH, lair(20.0, 0.0)).await;
    c.sees(boss).await;
    map.query(move |m| {
        put(m, other, lair(20.0, 30.0));
        tough(m, boss);
        if let Some(s) = event_ai::scripted_mut(m, boss) {
            s.no_combat_movement = true;
        }
        ai::attacked(m, boss, me);
        ai::add_threat(m, boss, me, 5000.0);
        ai::add_threat(m, boss, other, 10.0);
    })
    .await;
    assert!(between(timer(&map, boss, 1226405).await, 25.0, 30.0));
    c.clear();
    fire(&map, boss, 1226405).await;
    assert!(
        cast_of(&mut c, boss, 23138, Duration::from_secs(2))
            .await
            .is_some(),
        "the Gate of Shazzrah"
    );
    let (beside, threats, listed, v) = map
        .query(move |m| {
            let here = m.position_of(boss).unwrap();
            let beside: Vec<Guid> = [me, other]
                .into_iter()
                .filter(|p| m.position_of(*p).is_some_and(|at| dist(at, here) < 6.0))
                .collect();
            let listed = m.creature(boss).map_or(0, |c| {
                c.threat
                    .iter()
                    .filter(|(t, _)| [me, other].contains(t))
                    .count()
            });
            (
                beside,
                (threat(m, boss, me), threat(m, boss, other)),
                listed,
                victim(m, boss),
            )
        })
        .await
        .unwrap();
    assert_eq!(beside.len(), 1, "beside one of them: {beside:?}");
    assert_eq!(threats, (0.0, 0.0), "threat gone");
    assert_eq!(listed, 2, "both still on his list");
    assert_eq!(v, me, "at a tie, his victim");
    assert!(between(timer(&map, boss, 1226405).await, 25.0, 35.0));
    clear(&map, vec![boss]);
}

/// Golemagg below a tenth calls his ragers (Attract Rager on himself), and
/// five seconds on shakes the ground, again every five; an evade ends it.
async fn golemagg(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Golly").await;
    let boss = summon_near(&map, GOLEMAGG, lair(5.0, 0.0)).await;
    c.sees(boss).await;
    map.query(move |m| {
        tough(m, boss);
        ai::attacked(m, boss, me);
        set_pct(m, boss, 9);
        if let Some(s) = event_ai::scripted_mut(m, boss) {
            s.next_tick = None;
        }
        event_ai::tick(m, boss);
    })
    .await;
    assert!(
        cast_of(&mut c, boss, 20544, Duration::from_secs(2))
            .await
            .is_some(),
        "Attract Rager"
    );
    for n in 0..2 {
        let quake = cast_of(&mut c, boss, 19798, Duration::from_millis(6500)).await;
        assert!(quake.is_some(), "Earthquake {n}");
    }
    map.query(move |m| ai::enter_evade(m, boss)).await;
    c.clear();
    let after = cast_of(&mut c, boss, 19798, Duration::from_millis(6000)).await;
    assert!(after.is_none(), "no quake once he evades");
    clear(&map, vec![boss]);
}

/// A Core Rager won't go below half its health while Golemagg lives: there
/// it says so and is healed whole.
async fn core_rager(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Rags").await;
    let rager = summon_near(&map, CORE_RAGER, lair(5.0, 0.0)).await;
    c.sees(rager).await;
    map.query(move |m| ai::attacked(m, rager, me)).await;
    let (hp, max) = map
        .query(move |m| {
            let (hp, max) = health(m, rager);
            crate::game::map::combat::deal_damage(m, me, rager, hp - 1);
            (health(m, rager).0, max)
        })
        .await
        .unwrap();
    assert_eq!(hp, max / 2, "held at half");
    assert!(
        cast_of(&mut c, rager, 17683, Duration::from_millis(2500))
            .await
            .is_some(),
        "healed"
    );
    assert!(
        until(&map, 2000, move |m| health(m, rager).0 > max / 2).await,
        "back up"
    );
    clear(&map, vec![rager]);
}

/// Molten Core's lieutenants, on one load of the real world.
#[tokio::test]
async fn molten_cores_lieutenants_fight_as_thistle_tea_scripts_them() {
    let Some(realm) = realm() else {
        return;
    };
    if wanted("lucifron") {
        lucifron(&realm).await;
    }
    if wanted("sulfuron") {
        sulfuron(&realm).await;
    }
    if wanted("geddon") {
        geddon(&realm).await;
    }
    if wanted("garr") {
        garr(&realm).await;
    }
    if wanted("shazzrah") {
        shazzrah(&realm).await;
    }
    if wanted("golemagg") {
        golemagg(&realm).await;
    }
    if wanted("rager") {
        core_rager(&realm).await;
    }
}

/// The spells of the auras on `g`.
pub(super) async fn auras_on(map: &MapHandle, g: Guid) -> Vec<u32> {
    map.query(move |m| {
        auras::auras(m, g)
            .map(|a| a.holders.iter().map(|h| h.spell).collect())
            .unwrap_or_default()
    })
    .await
    .unwrap()
}

const FIREWALKER: u32 = 11666;
const FIRELORD: u32 = 11668;
const ANCIENT_CORE_HOUND: u32 = 11673;
const LAVA_SURGER: u32 = 12101;

/// Its spell list, as it stands: a script's over its template's.
fn spell_list(m: &Map, g: Guid) -> u32 {
    let own = m
        .creature(g)
        .and_then(|c| m.realm.data.creature_template(c.entry))
        .map_or(0, |t| t.spell_list_id);
    event_ai::scripted(m, g)
        .and_then(|s| s.spell_list)
        .unwrap_or(own)
}

/// A Firewalker's pull puts its spell list away for its scripted casts,
/// each pull anew; at peace it has its list back.
async fn firewalker_spell_list(realm: &Arc<Realm>) {
    let (_c, me, map) = raider(realm, "Walker").await;
    let walker = summon_near(&map, FIREWALKER, lair(5.0, 0.0)).await;
    let lists = map
        .query(move |m| {
            let before = spell_list(m, walker);
            ai::attacked(m, walker, me);
            let pulled = spell_list(m, walker);
            ai::enter_evade(m, walker);
            (before, pulled)
        })
        .await
        .unwrap();
    assert_eq!(lists, (116660, 0), "its list, then none in the fight");
    assert!(
        until(&map, 6000, move |m| at_peace(m, walker)
            && spell_list(m, walker) == 116660)
        .await,
        "its list back at peace"
    );
    let again = map
        .query(move |m| {
            ai::attacked(m, walker, me);
            spell_list(m, walker)
        })
        .await
        .unwrap();
    assert_eq!(again, 0, "put away again on the next pull");
    clear(&map, vec![walker]);
}

/// A Firewalker's Fire Blossom: the aura on itself, then six blossoms a
/// second apart, each at one on its threat list as it stands then; an evade
/// or its death puts out those left.
async fn firewalker_blossoms(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Blossom").await;
    let (_d, other) = joins(realm, "Petal", &map, LAIR).await;
    let walker = summon_near(&map, FIREWALKER, lair(5.0, 0.0)).await;
    c.sees(walker).await;
    map.query(move |m| {
        put(m, other, lair(5.0, 8.0));
        tough(m, walker);
        if let Some(s) = event_ai::scripted_mut(m, walker) {
            s.no_combat_movement = true;
        }
        ai::attacked(m, walker, me);
        ai::add_threat(m, walker, me, 100.0);
        ai::add_threat(m, walker, other, 50.0);
    })
    .await;
    assert!(
        about(timer(&map, walker, 1166602).await, 6.0),
        "six seconds in"
    );
    c.clear();
    fire(&map, walker, 1166602).await;
    assert!(
        cast_of(&mut c, walker, 19636, Duration::from_secs(1))
            .await
            .is_some(),
        "the Fire Blossom aura"
    );
    let first = cast_of(&mut c, walker, 19637, Duration::from_millis(1500)).await;
    assert!(
        first == Some(me) || first == Some(other),
        "a blossom at one it fights: {first:?}"
    );
    // Now only the second is on its list: the first can't be attacked.
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&me) {
            p.fields.set_flag(UNIT_FIELD_FLAGS, 0x2, true);
        }
        if let Some(cr) = m.creature_mut(walker) {
            cr.threat.retain(|(t, _)| *t != me);
        }
    })
    .await;
    let mut rest = Vec::new();
    for _ in 0..5 {
        rest.push(cast_of(&mut c, walker, 19637, Duration::from_millis(1500)).await);
    }
    assert_eq!(rest, vec![Some(other); 5], "the rest at the one left");
    assert!(
        cast_of(&mut c, walker, 19637, Duration::from_millis(1300))
            .await
            .is_none(),
        "six in all"
    );
    map.query(move |m| {
        if let Some(p) = m.players.get_mut(&me) {
            p.fields.set_flag(UNIT_FIELD_FLAGS, 0x2, false);
        }
    })
    .await;
    // Cut short: by an evade, then by its death.
    for death in [false, true] {
        map.query(move |m| {
            if !fighting(m, walker) {
                ai::attacked(m, walker, me);
            }
        })
        .await;
        sleep_ms(100).await;
        c.clear();
        fire(&map, walker, 1166602).await;
        assert!(
            cast_of(&mut c, walker, 19636, Duration::from_secs(1))
                .await
                .is_some()
        );
        map.query(move |m| {
            if death {
                crate::game::map::combat::deal_damage(m, me, walker, 20_000_000);
            } else {
                ai::enter_evade(m, walker);
            }
        })
        .await;
        assert!(
            cast_of(&mut c, walker, 19637, Duration::from_millis(2500))
                .await
                .is_none(),
            "no blossom after its {}",
            if death { "death" } else { "evade" }
        );
        if !death {
            assert!(until(&map, 6000, move |m| at_peace(m, walker)).await);
        }
    }
    clear(&map, vec![walker]);
}

/// A Firelord: Incinerate on itself at the pull; Soul Burn at a random
/// player, never a creature it fights; Split seven and a half to twelve
/// and a half seconds in, then every fifteen to twenty.
async fn firelord(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Lordy").await;
    let lord = summon_near(&map, FIRELORD, lair(5.0, 0.0)).await;
    let foe = map.query(|m| dummy(m, lair(5.0, 6.0))).await.unwrap();
    c.sees(lord).await;
    c.clear();
    map.query(move |m| {
        tough(m, lord);
        if let Some(s) = event_ai::scripted_mut(m, lord) {
            s.no_combat_movement = true;
        }
        ai::attacked(m, lord, me);
        ai::add_threat(m, lord, foe, 10_000.0);
    })
    .await;
    assert!(
        until(&map, 1500, move |m| has(m, lord, 19396)).await,
        "Incinerate at the pull"
    );
    assert!(between(timer(&map, lord, 1166803).await, 4.0, 6.0));
    for _ in 0..4 {
        c.clear();
        fire(&map, lord, 1166803).await;
        assert_eq!(
            cast_of(&mut c, lord, 19393, Duration::from_secs(1)).await,
            Some(me),
            "Soul Burn at the player"
        );
        assert!(between(timer(&map, lord, 1166803).await, 3.0, 4.0));
    }
    assert!(between(timer(&map, lord, 1166802).await, 7.5, 12.5));
    c.clear();
    fire(&map, lord, 1166802).await;
    assert!(
        cast_of(&mut c, lord, 19569, Duration::from_secs(1))
            .await
            .is_some(),
        "Split"
    );
    assert!(between(timer(&map, lord, 1166802).await, 15.0, 20.0));
    let spawns: Vec<Guid> = map
        .query(|m| {
            m.creatures
                .iter()
                .filter(|(_, c)| c.entry == 12265)
                .map(|(_, c)| c.guid)
                .collect()
        })
        .await
        .unwrap();
    let mut gone = vec![lord, foe];
    gone.extend(spawns);
    clear(&map, gone);
}

/// An Ancient Core Hound bites with Vicious Bite for its swings, calls no
/// one, and keeps the one debuff its phase (one to six, rolled at its
/// spawn) names, a new one at each reset.
async fn ancient_core_hound_debuffs(realm: &Arc<Realm>) {
    const DEBUFFS: [u32; 6] = [19364, 19365, 19366, 19367, 19369, 19372];
    let (mut c, me, map) = raider(realm, "Hound").await;
    let hound = summon_near(&map, ANCIENT_CORE_HOUND, lair(5.0, 0.0)).await;
    c.sees(hound).await;
    sleep_ms(200).await;
    let (melee, help, phase) = map
        .query(move |m| {
            let s = event_ai::scripted(m, hound).unwrap();
            (s.melee_spell, s.call_for_help_range, s.phase)
        })
        .await
        .unwrap();
    assert_eq!((melee, help), (Some(19319), Some(0.0)), "its bite, no call");
    assert!((1..=6).contains(&phase), "a phase of the six: {phase}");
    let mut seen = Vec::new();
    for round in 0..3 {
        map.query(move |m| {
            tough(m, hound);
            ai::attacked(m, hound, me);
        })
        .await;
        let phase = map.query(move |m| event_ai::phase(m, hound)).await.unwrap();
        let expected = DEBUFFS[phase as usize - 1];
        for _ in 0..2 {
            c.clear();
            for id in 1167306..=1167311 {
                fire(&map, hound, id).await;
            }
            let cast: Vec<u32> = casts_by(&mut c, hound, Duration::from_millis(400))
                .await
                .into_iter()
                .map(|(s, _)| s)
                .filter(|s| DEBUFFS.contains(s))
                .collect();
            assert_eq!(cast, vec![expected], "round {round}, phase {phase}");
        }
        seen.push(phase);
        map.query(move |m| ai::enter_evade(m, hound)).await;
        assert!(until(&map, 6000, move |m| at_peace(m, hound)).await);
    }
    eprintln!("ancient core hound phases: {seen:?}");
    clear(&map, vec![hound]);
}

/// An Ancient Core Hound's Serrated Bite stays due while its victim is out
/// of its reach, and goes once it is in it; a bite that couldn't go (it was
/// breathing fire) waits its time again.
async fn ancient_core_hound_bite(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Biter").await;
    let hound = summon_near(&map, ANCIENT_CORE_HOUND, lair(20.0, 0.0)).await;
    c.sees(hound).await;
    sleep_ms(200).await;
    map.query(move |m| {
        tough(m, hound);
        if let Some(s) = event_ai::scripted_mut(m, hound) {
            s.no_combat_movement = true;
        }
        ai::attacked(m, hound, me);
    })
    .await;
    assert!(about(timer(&map, hound, 1167305).await, 4.0));
    c.clear();
    fire(&map, hound, 1167305).await;
    assert!(
        cast_of(&mut c, hound, 19771, Duration::from_millis(500))
            .await
            .is_none(),
        "no bite from twenty yards"
    );
    assert!(
        timer(&map, hound, 1167305)
            .await
            .is_some_and(|t| t.is_zero()),
        "still due"
    );
    map.query(move |m| put(m, me, lair(17.5, 0.0))).await;
    c.clear();
    fire(&map, hound, 1167305).await;
    assert_eq!(
        cast_of(&mut c, hound, 19771, Duration::from_millis(800)).await,
        Some(me),
        "bitten in reach"
    );
    assert!(about(timer(&map, hound, 1167305).await, 6.0));
    // Breathing fire, it can't bite: the bite waits its six seconds anew.
    c.clear();
    map.query(move |m| {
        let now = m.now;
        if let Some(cr) = m.creature_mut(hound) {
            cr.caster.casting = Some(crate::game::map::spells::Casting {
                spell: 19630,
                targets: Default::default(),
                ends: now + Duration::from_secs(3),
                item: None,
                pushbacks: 0,
                channel_until: None,
                duration: 3000,
            });
        }
        due(m, hound, 1167305);
    })
    .await;
    assert!(
        cast_of(&mut c, hound, 19771, Duration::from_millis(500))
            .await
            .is_none(),
        "not while it casts"
    );
    assert!(
        about(timer(&map, hound, 1167305).await, 6.0),
        "its time anew"
    );
    clear(&map, vec![hound]);
}

/// A Lava Surger surges at the farthest player more than seven yards off
/// (edge to edge, on the flat), and again five to six seconds on; with none
/// so far its surge stays due; stunned (banished), its time stands still.
async fn lava_surger(realm: &Arc<Realm>) {
    let (mut c, me, map) = raider(realm, "Surgy").await;
    let (_d, other) = joins(realm, "Surfer", &map, LAIR).await;
    let surger = summon_near(&map, LAVA_SURGER, lair(5.0, 0.0)).await;
    c.sees(surger).await;
    map.query(move |m| {
        put(m, me, lair(9.0, 0.0));
        put(m, other, lair(5.0, 20.0));
        tough(m, surger);
        if let Some(s) = event_ai::scripted_mut(m, surger) {
            s.no_combat_movement = true;
        }
        ai::attacked(m, surger, me);
        ai::add_threat(m, surger, other, 1.0);
    })
    .await;
    assert!(between(timer(&map, surger, 1210102).await, 1.0, 2.0));
    c.clear();
    fire(&map, surger, 1210102).await;
    assert_eq!(
        cast_of(&mut c, surger, 19196, Duration::from_secs(1)).await,
        Some(other),
        "at the farthest"
    );
    assert!(between(timer(&map, surger, 1210102).await, 5.0, 6.0));
    // Both near: it waits, due.
    let here = settled(&map, surger).await;
    map.query(move |m| {
        for p in [me, other] {
            put(
                m,
                p,
                Vector3d {
                    x: here.x + 5.0,
                    ..here
                },
            );
        }
    })
    .await;
    c.clear();
    fire(&map, surger, 1210102).await;
    assert!(
        cast_of(&mut c, surger, 19196, Duration::from_millis(500))
            .await
            .is_none(),
        "no one far enough"
    );
    assert!(
        timer(&map, surger, 1210102)
            .await
            .is_some_and(|t| t.is_zero()),
        "still due"
    );
    map.query(move |m| {
        put(
            m,
            me,
            Vector3d {
                x: here.x + 15.0,
                ..here
            },
        );
    })
    .await;
    c.clear();
    fire(&map, surger, 1210102).await;
    assert_eq!(
        cast_of(&mut c, surger, 19196, Duration::from_secs(1)).await,
        Some(me),
        "at the one gone far enough"
    );
    // Banished, its surge's time waits.
    map.query(move |m| crate::game::map::spells::cast_triggered(m, me, 710, Some(surger)))
        .await;
    assert!(
        until(&map, 1000, move |m| has(m, surger, 710)).await,
        "banished"
    );
    let before = timer(&map, surger, 1210102).await.unwrap();
    sleep_ms(2500).await;
    let after = timer(&map, surger, 1210102).await.unwrap();
    assert!(
        after + Duration::from_millis(1100) >= before,
        "held while banished: {before:?} then {after:?}"
    );
    map.query(move |m| auras::remove_spell(m, surger, 710, None))
        .await;
    sleep_ms(2200).await;
    let later = timer(&map, surger, 1210102).await.unwrap_or_default();
    assert!(
        later + Duration::from_millis(1500) < after,
        "on again after: {after:?} then {later:?}"
    );
    clear(&map, vec![surger]);
}

/// Molten Core's trash, on one load of the real world.
#[tokio::test]
async fn molten_cores_trash_fights_as_thistle_tea_scripts_it() {
    let Some(realm) = realm() else {
        return;
    };
    if wanted("firewalker") {
        firewalker_spell_list(&realm).await;
        firewalker_blossoms(&realm).await;
    }
    if wanted("firelord") {
        firelord(&realm).await;
    }
    if wanted("ancient") {
        ancient_core_hound_debuffs(&realm).await;
        ancient_core_hound_bite(&realm).await;
    }
    if wanted("surger") {
        lava_surger(&realm).await;
    }
}
