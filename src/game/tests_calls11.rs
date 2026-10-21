//! Round 11's calls, as the user answered them (2026-10-21), each as
//! mangos-classic has it: an instant spell's misses in its
//! `SMSG_SPELL_GO` (`Spell::WriteSpellGoTargets`), not in a miss log; a
//! player's own guardian gone at its time, mid-fight (`Pet::Update`); a
//! map with no navmesh nor terrain in `Map.dbc`'s zone ahead of its only
//! `AreaTable` one; and a party made of players alone at a meeting stone
//! only when five wait (`LFGQueue::Update`'s `m_groupSize`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use crate::game::Realm;
use crate::game::data::spells::{SpellDuration, SpellEffect};
use crate::game::data::{AreaInfo, GameData, MapInfo};
use crate::game::fields::UNIT_FIELD_FLAGS;
use crate::game::harness::{self, Client};
use crate::game::map::auras::*;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::spells::{self, outcome};
use crate::game::map::{Map, MapKey, auras, combat, pets};

async fn on<T: Send + 'static>(
    map: &crate::game::map::MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// A frost bolt, instant; a ward that turns every spell back; a hit chance
/// a hundred less; a frost bolt of no damage class, which can't miss.
const BOLT: u32 = 997_901;
const WARD: u32 = 997_902;
const CLUMSY: u32 = 997_903;
const SURE: u32 = 997_904;

fn bolts(d: &mut GameData) {
    let mut bolt = bare(BOLT, 4);
    effect(&mut bolt, 0, damage(50));
    let mut sure = bare(SURE, 4);
    sure.dmg_class = 0;
    effect(&mut sure, 0, damage(50));
    let mut ward = bare(WARD, 0);
    ward.duration_index = LASTS_LONG;
    effect(&mut ward, 0, aura(AURA_REFLECT_SPELLS, 0, 100, 1));
    let mut clumsy = bare(CLUMSY, 0);
    clumsy.duration_index = LASTS_LONG;
    effect(&mut clumsy, 0, aura(AURA_MOD_SPELL_HIT_CHANCE, 0, -100, 1));
    insert(d, [bolt, sure, ward, clumsy]);
}

/// The next `SMSG_SPELL_GO` of `spell`: its hits, and its misses with
/// their reasons.
async fn go(c: &mut Client, spell: u32) -> (Vec<Guid>, Vec<(Guid, u8)>) {
    c.wait_for("SMSG_SPELL_GO", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => Some((
            g.hits.clone(),
            g.misses
                .iter()
                .map(|x| (x.target, harness::miss_reason(&x.miss_info)))
                .collect(),
        )),
        _ => None,
    })
    .await
}

/// Whether a miss log of `spell` comes soon.
async fn miss_logged(c: &mut Client, spell: u32) -> bool {
    crate::game::tests_p3_entity::gets_frame(c, outcome::SMSG_SPELLLOGMISS, |b| {
        b[..4] == spell.to_le_bytes()
    })
    .await
}

/// The user's call of round 11, as mangos-classic's `WriteSpellGoTargets`:
/// an instant spell resisted lists its target among the go's misses
/// (reason 2), not its hits, and no `SMSG_SPELLLOGMISS` follows; one
/// turned back by a ward lists it as reflected (11), the caster hurt; one
/// that lands lists it as a hit, no miss. Before, the go listed every
/// target as a hit and the miss came in a miss log.
#[tokio::test]
async fn an_instant_spells_misses_are_in_its_go_and_nowhere_else() {
    let realm = fast_realm(bolts);
    let (mut c, a, map) = human(&realm, "C11A", "Misser", Class::Mage).await;
    let kobold = on(&map, move |m| {
        m.spells.rolls = Some(false);
        kit::ready(m, a, 60, &[BOLT, SURE]);
        kit::apply(m, a, a, CLUMSY);
        let data = m.realm.data.clone();
        m.players.get_mut(&a).unwrap().apply_stats(&data);
        kit::dummy(m, a, 60, 5.0)
    })
    .await;
    c.sees(kobold).await;
    c.clear();
    let hp = on(&map, move |m| {
        cast_fresh(m, a, BOLT, at(kobold));
        kit::health(m, kobold)
    })
    .await;
    assert_eq!(hp, 1_000_000, "resisted: nothing done");
    assert_eq!(
        go(&mut c, BOLT).await,
        (vec![], vec![(kobold, 2)]),
        "a resist, in the go"
    );
    assert!(!miss_logged(&mut c, BOLT).await, "and no miss log");
    // A ward on the kobold: turned back, and told so in the go.
    let (hp, mine) = on(&map, move |m| {
        kit::apply(m, kobold, kobold, WARD);
        kit::set_health(m, a, 1_000);
        cast_fresh(m, a, BOLT, at(kobold));
        (kit::health(m, kobold), kit::health(m, a))
    })
    .await;
    assert_eq!(hp, 1_000_000, "the kobold untouched");
    assert!(mine < 1_000, "the caster hurt: {mine}");
    assert_eq!(
        go(&mut c, BOLT).await,
        (vec![], vec![(kobold, 11)]),
        "a reflection, in the go"
    );
    assert!(!miss_logged(&mut c, BOLT).await, "and no miss log");
    // One that can't miss: a hit, no miss.
    let hp = on(&map, move |m| {
        auras::remove_spell(m, kobold, WARD, None);
        cast_fresh(m, a, SURE, at(kobold));
        kit::health(m, kobold)
    })
    .await;
    assert_eq!(hp, 1_000_000 - 50);
    assert_eq!(go(&mut c, SURE).await, (vec![kobold], vec![]), "a hit");
}

/// A guardian of a minute (a Mechanical Dragonling's kind).
const DRAGONLING: u32 = 98_960;
const CALL_DRAGONLING: u32 = 98_961;
const MINUTE: u32 = 4;

fn guardian_world() -> GameData {
    use crate::game::tests_p3_combat::{HUMANS, data, template};
    let mut d = data();
    d.spells.durations.insert(
        MINUTE,
        SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    let mut t = template(DRAGONLING, 9);
    t.faction = HUMANS;
    d.creature_templates.insert(DRAGONLING, t);
    let mut s = crate::game::tests_p3_entity::spell(
        CALL_DRAGONLING,
        &[SpellEffect {
            effect: pets::EFFECT_SUMMON_GUARDIAN,
            misc_value: DRAGONLING as i32,
            base_points: 0,
            base_dice: 1,
            target_a: 32,
            ..Default::default()
        }],
    );
    s.duration_index = MINUTE;
    d.spells.spells.insert(CALL_DRAGONLING, s);
    d
}

/// The user's call of round 11, as mangos-classic's `Pet::Update` (its
/// `m_duration` run out, `Unsummon`, in a fight or not): a player's
/// guardian fighting is still there at half its time, and gone at its
/// time, out of its owner's hands, its fight not over. Before, it waited
/// for its fight's end (Thistle Tea's `TemporarySummon`).
#[tokio::test]
async fn a_players_guardian_goes_at_its_time_mid_fight() {
    use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
    let (_realm, _a, ga, map) = setup_with(guardian_world(), Class::Warrior).await;
    let out = on(&map, move |m| {
        spells::cast(m, ga, CALL_DRAGONLING, spells::Targets::default(), None);
        let g = m
            .creatures
            .iter()
            .map(|(_, c)| c)
            .find(|c| c.owner == Some(ga) && c.entry == DRAGONLING)
            .map(|c| c.guid)
            .expect("the dragonling");
        let brute = spawn(m, ga, BRUTE, 20, 4.0);
        combat::start_attack(m, g, brute);
        combat::engage(m, g, brute);
        let fighting = |m: &mut Map| {
            if let Some(c) = m.creature_mut(g) {
                c.fields
                    .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
            }
        };
        let start = m.now;
        fighting(m);
        m.now = start + Duration::from_secs(30);
        pets::update(m);
        let midway = m.creature(g).is_some();
        fighting(m);
        m.now = start + Duration::from_secs(61);
        pets::update(m);
        let in_fight = m.creature(brute).is_some_and(|b| b.alive());
        (
            midway,
            in_fight,
            m.creature(g).is_none(),
            m.players[&ga].owned.guardians.contains(&g),
        )
    })
    .await;
    assert_eq!(
        out,
        (true, true, true, false),
        "there midway, gone at its time mid-fight, out of its owner's hands"
    );
}

const SUNKEN_TEMPLE: u32 = 109;
const TEMPLE_OWN: u32 = 1417;
const TEMPLE_LINKED: u32 = 1477;
const SHADOWFANG: u32 = 33;
const SHADOWFANG_ZONE: u32 = 209;

fn zone_of(realm: &Arc<Realm>, map: u32) -> impl Future<Output = (bool, Option<(u32, u32)>)> {
    let handle = realm.maps.get(realm, MapKey { map, instance: 1 });
    async move {
        handle
            .query(|m| {
                let blind = m.nav.is_none() && m.terrain.is_none();
                let at = Vector3d {
                    x: 10.0,
                    y: 10.0,
                    z: 0.0,
                };
                (blind, crate::game::map::world::zone_and_area(m, at))
            })
            .await
            .unwrap()
    }
}

/// The user's call of round 11: a dungeon with neither a navmesh nor
/// terrain is in `Map.dbc`'s zone ahead of its only `AreaTable` one
/// (Sunken Temple, 109: the Temple of Atal'Hakkar, 1477, not 1417); one
/// `Map.dbc` links to none keeps its own (Shadowfang Keep, 33, in 209).
#[tokio::test]
async fn a_blind_dungeon_is_in_map_dbcs_zone_before_its_own() {
    let mut d = GameData::empty();
    for (map, name) in [(SUNKEN_TEMPLE, "SunkenTemple"), (SHADOWFANG, "Shadowfang")] {
        d.maps.insert(
            map,
            MapInfo {
                id: map,
                internal_name: name.into(),
                name: name.into(),
                instance_type: 1,
                player_limit: 5,
            },
        );
    }
    d.map_zones.insert(SUNKEN_TEMPLE, TEMPLE_LINKED);
    for (id, map) in [
        (TEMPLE_LINKED, 0),
        (TEMPLE_OWN, SUNKEN_TEMPLE),
        (SHADOWFANG_ZONE, SHADOWFANG),
    ] {
        d.areas.insert(
            id,
            AreaInfo {
                id,
                map,
                name: format!("Area {id}"),
                ..Default::default()
            },
        );
    }
    let realm = harness::realm_with(d);
    assert_eq!(
        zone_of(&realm, SUNKEN_TEMPLE).await,
        (true, Some((TEMPLE_LINKED, TEMPLE_LINKED))),
        "Map.dbc's"
    );
    assert_eq!(
        zone_of(&realm, SHADOWFANG).await,
        (true, Some((SHADOWFANG_ZONE, SHADOWFANG_ZONE))),
        "its own, Map.dbc naming none"
    );
}

/// The same with the client's own `Map.dbc` and `AreaTable.dbc`: Sunken
/// Temple in 1477, Shadowfang Keep in 209, Blackrock Depths in 1584.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_sunken_temple_is_in_the_temple_of_atalhakkar() {
    let d = harness::load_real(&harness::real_data("db"));
    let realm = harness::realm_with(d);
    let mut out = Vec::new();
    for map in [SUNKEN_TEMPLE, SHADOWFANG, 230] {
        out.push(zone_of(&realm, map).await.1);
    }
    assert_eq!(
        out,
        [
            Some((TEMPLE_LINKED, TEMPLE_LINKED)),
            Some((SHADOWFANG_ZONE, SHADOWFANG_ZONE)),
            Some((1584, 1584)),
        ]
    );
}

/// The user's call of round 11, as mangos-classic's `LFGQueue::Update`
/// (`m_groupSize` 5: the leader and four others in the area): four alone
/// at a dungeon's stone make no party; a fifth, and the five make one.
/// Before, two made one.
#[tokio::test]
async fn five_alone_at_a_stone_make_a_party_and_four_none() {
    const AREA: u32 = 1581; // the Deadmines
    let realm = harness::realm();
    let map0 = realm.maps.get(&realm, MapKey::world(0));
    let mut clients = Vec::new();
    let mut guids = Vec::new();
    for (account, name, class) in [
        ("C11S1", "Tank", Class::Warrior),
        ("C11S2", "Heal", Class::Priest),
        ("C11S3", "Mage", Class::Mage),
        ("C11S4", "Rogue", Class::Rogue),
        ("C11S5", "Lock", Class::Warlock),
    ] {
        let mut c = Client::connect(&realm, account);
        guids.push(c.enter(name, Race::Human, class).await);
        clients.push(c);
    }
    let queue = |g: Guid| {
        let map0 = map0.clone();
        async move {
            map0.query(move |m| crate::game::map::meeting_stones::queue(m, g, AREA))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    for g in &guids[..4] {
        queue(*g).await;
    }
    crate::game::map::meeting_stones::sweep(&realm);
    let grouped = |realm: &Arc<Realm>| {
        let groups = realm.social.groups.lock();
        guids
            .iter()
            .filter(|g| groups.group_of(**g).is_some())
            .count()
    };
    assert_eq!(grouped(&realm), 0, "four alone: no party");
    assert_eq!(
        realm.social.meeting_stones.lock().queue.len(),
        4,
        "all four still queued"
    );
    queue(guids[4]).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while realm.social.meeting_stones.lock().of(guids[4]).is_none() {
        assert!(Instant::now() < deadline, "the fifth never queued");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    crate::game::map::meeting_stones::sweep(&realm);
    let party = realm
        .social
        .groups
        .lock()
        .group_of(guids[0])
        .map(|g| g.members.len());
    assert_eq!(party, Some(5), "five: a party of them");
    assert_eq!(grouped(&realm), 5);
    assert!(
        realm.social.meeting_stones.lock().queue.is_empty(),
        "whole, it is out of the queue"
    );
}
