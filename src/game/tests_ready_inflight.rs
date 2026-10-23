//! A raid attempt's reset leaves nothing of the last pull to start the
//! next, through the protocol: an imp's Firebolt on its way as its master
//! says `.ready load` is lost, the imp's cast under way is interrupted and
//! its fight ended, its totems go; a Frostbolt on its way to a creature
//! `.respawn` brings back as new misses the new one, as does the old one's
//! own bolt on its way; and a player's ground area and damage over time
//! go at their `.ready load`.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::{CreatureSpawn, GameData};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::Client;
use crate::game::map::auras::{AURA_PERIODIC_DAMAGE, auras};
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, pets};
use crate::game::tests_p3_combat::{BRUTE, START};
use crate::game::tests_pets as tp;

/// A creature that notices no one, on the imp's enemies' side.
const DUMMY: u32 = 2_100;
/// The imp's Firebolt flies a yard a second; its cast takes ten seconds.
const FIREBOLT_SPEED: f32 = 1.0;
const LONG_CAST: u32 = 90;

/// A mage's bolt and a brute's, a hundred each, a yard a second; a ground
/// area of 50 a second; a damage over time of 10 a second.
const BOLT: u32 = 97_400;
const BRUTE_BOLT: u32 = 97_401;
const GROUND: u32 = 97_402;
const DOT: u32 = 97_403;
const RADIUS: u32 = 97;
/// A brute's spawn ten yards from Alice.
const NEAR: u32 = 911;

async fn say(c: &mut Client, line: &str, reply: &str) -> String {
    c.say(line).await;
    c.system_message(reply).await
}

fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
}

fn health(m: &Map, g: Guid) -> (u32, u32) {
    m.unit_fields(g).map_or((0, 0), |f| {
        (
            f.get_u32(UNIT_FIELD_HEALTH),
            f.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    })
}

/// The creature's fight: hurt, its threat list, attacking, in combat.
fn fight(m: &Map, c: Guid) -> (bool, usize, bool, bool) {
    let (hp, max) = health(m, c);
    let cr = m.creature(c).expect("the creature");
    (
        hp < max,
        cr.threat.len(),
        cr.combat.attacking,
        in_combat(m, c),
    )
}

/// The map's clock `ms` on, in tenths of a second: spells land, pets and
/// creatures take their turns.
fn pass(m: &mut Map, ms: u64) {
    for _ in 0..ms / 100 {
        m.now += Duration::from_millis(100);
        spells::update(m);
        crate::game::map::auras::update(m);
        pets::update(m);
    }
}

fn pets_data() -> GameData {
    let mut d = tp::data();
    let mut dummy = tp::template(DUMMY, "Dummy", 14, 1);
    dummy.detection_range = 0.1;
    d.creature_templates.insert(DUMMY, dummy);
    d.spells.cast_times.insert(LONG_CAST, 10_000);
    let bolt = d.spells.spells.get_mut(&tp::FIREBOLT).unwrap();
    bolt.speed = FIREBOLT_SPEED;
    bolt.casting_time_index = LONG_CAST;
    d
}

/// A warlock with her imp out, and a dummy 20 yards before her.
async fn warlock(save_with_imp: bool) -> (Client, Guid, Guid, Guid, crate::game::map::MapHandle) {
    let realm = crate::game::harness::realm_with(pets_data());
    let (mut w, gw, map) = tp::enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    tp::learn(&map, gw, &[tp::SUMMON_IMP]);
    if !save_with_imp {
        say(&mut w, ".ready save", "Ready state saved").await;
    }
    tp::cast(&w, tp::SUMMON_IMP, None).await;
    let imp = tp::pet_bar(&mut w).await;
    assert!(!imp.is_zero());
    w.sees(imp).await;
    if save_with_imp {
        let saved = say(&mut w, ".ready save", "Ready state saved").await;
        assert!(saved.contains("your pet"), "{saved}");
    }
    let dummy = tp::place(&map, gw, DUMMY, 0x70_0001, 20.0).await;
    w.sees(dummy).await;
    (w, gw, imp, dummy, map)
}

/// The imp's Firebolt is on its way to the dummy (twenty seconds off) as
/// its master says `.ready load`: it is lost, and the dummy, the imp and
/// the warlock are none of them in a fight once it would have landed.
#[tokio::test]
async fn ready_load_loses_an_imps_firebolt_in_flight() {
    let (mut w, gw, imp, dummy, map) = warlock(true).await;
    let flying = map
        .query(move |m| {
            m.spells.travel = Some(true);
            spells::cast_triggered(m, imp, tp::FIREBOLT, Some(dummy));
            m.spells
                .in_flight
                .iter()
                .filter(|f| f.caster == imp && f.target == dummy)
                .count()
        })
        .await
        .unwrap();
    assert_eq!(flying, 1, "the Firebolt is on its way");
    say(&mut w, ".ready load", "is ready").await;
    let after = map
        .query(move |m| {
            let lost = m.spells.in_flight.is_empty();
            pass(m, 25_000);
            (lost, fight(m, dummy), in_combat(m, gw), in_combat(m, imp))
        })
        .await
        .unwrap();
    assert!(after.0, "nothing on its way");
    assert_eq!(after.1, (false, 0, false, false), "the dummy untouched");
    assert!(!after.2 && !after.3, "the warlock and her imp out of it");
}

/// The imp ten seconds into a Firebolt at the dummy, sent at it by its
/// master, and a totem of hers out, her ready state saved before she had
/// either: `.ready load` interrupts the cast (the client sees it fail),
/// takes the imp's victim, target and orders away, and the totem; nothing
/// reaches the dummy after.
#[tokio::test]
async fn ready_load_interrupts_an_imp_mid_cast_and_stands_it_down() {
    let (mut w, gw, imp, dummy, map) = warlock(false).await;
    tp::learn(&map, gw, &[tp::STRENGTH_OF_EARTH_TOTEM]);
    let began = map
        .query(move |m| {
            m.spells.travel = Some(true);
            spells::cast_triggered(m, gw, tp::STRENGTH_OF_EARTH_TOTEM, None);
            let totem = m.players[&gw].owned.totems.iter().flatten().next().copied();
            let cast = spells::cast(
                m,
                imp,
                tp::FIREBOLT,
                Targets {
                    unit: Some(dummy),
                    ..Default::default()
                },
                None,
            );
            if let Some(s) = pets::state_mut(m, imp) {
                s.commanded = Some(dummy);
            }
            let casting = m
                .caster(imp)
                .and_then(|c| c.casting.as_ref())
                .map(|c| c.spell);
            (cast, casting, totem)
        })
        .await
        .unwrap();
    assert_eq!(
        (began.0, began.1),
        (true, Some(tp::FIREBOLT)),
        "the imp casts"
    );
    let totem = began.2.expect("the totem is out");
    w.clear();
    say(&mut w, ".ready load", "is ready").await;
    w.wait_for("the imp's cast failing", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_FAILED_OTHER(f) if f.caster == imp => Some(()),
        _ => None,
    })
    .await;
    let after = map
        .query(move |m| {
            let c = m.creature(imp).unwrap();
            let s = pets::state(m, imp).unwrap();
            let stood_down = (
                c.caster.casting.is_none(),
                c.combat.attacking,
                c.fields.get_guid(UNIT_FIELD_TARGET),
                s.commanded,
            );
            let totem_gone = m.creature(totem).is_none()
                && m.players[&gw].owned.totems.iter().all(|t| t.is_none());
            pass(m, 15_000);
            (stood_down, totem_gone, fight(m, dummy), in_combat(m, imp))
        })
        .await
        .unwrap();
    assert_eq!(
        after.0,
        (true, false, Guid::zero(), None),
        "no cast, no attack, no target, no order"
    );
    assert!(after.1, "the totem gone");
    assert_eq!(after.2, (false, 0, false, false), "the dummy untouched");
    assert!(!after.3, "the imp out of it");
}

fn world() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    d.spells.radii.insert(RADIUS, 5.0);
    let bolt = |id| {
        let mut s = SpellTemplate {
            id,
            name: format!("Bolt {id}"),
            school: 4,
            dmg_class: 1,
            equipped_item_class: -1,
            range_index: 1,
            speed: 1.0,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 2,
            base_points: 99,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        };
        s
    };
    let mut ground = SpellTemplate {
        id: GROUND,
        name: "Ground".into(),
        school: 4,
        dmg_class: 1,
        equipped_item_class: -1,
        range_index: 1,
        duration_index: 8_000,
        ..Default::default()
    };
    ground.effects[0] = SpellEffect {
        effect: spells::EFFECT_PERSISTENT_AREA_AURA,
        aura: AURA_PERIODIC_DAMAGE,
        amplitude: 1000,
        base_points: 49,
        base_dice: 1,
        target_a: 18,
        radius_index: RADIUS,
        ..Default::default()
    };
    let mut dot = crate::game::tests_p3_aura2::aura_spell(
        DOT,
        Some(30_000),
        &[(AURA_PERIODIC_DAMAGE, 0, 1000)],
    );
    dot.effects[0].target_a = 6;
    crate::game::tests_p3_aura2::add_spells(&mut d, [bolt(BOLT), bolt(BRUTE_BOLT), ground, dot]);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: NEAR,
            entries: vec![BRUTE],
            map: 0,
            x: START.0 + 10.0,
            y: START.1,
            z: START.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        }],
    );
    d
}

/// Alice's bolt on its way to a brute (ten seconds off), and the brute's
/// own on its way to her, as `.respawn` brings the brute back as new under
/// its GUID: neither lands, and the new brute is in no fight.
#[tokio::test]
async fn respawn_loses_the_bolts_in_flight_to_and_from_the_old_life() {
    let (_realm, mut a, ga, map) =
        crate::game::tests_p3_combat::setup_with(world(), Class::Mage).await;
    let brute = guid::unit(BRUTE, NEAR);
    let flying = map
        .query(move |m| {
            m.spells.travel = Some(true);
            spells::cast_triggered(m, ga, BOLT, Some(brute));
            spells::cast_triggered(m, brute, BRUTE_BOLT, Some(ga));
            m.spells.in_flight.len()
        })
        .await
        .unwrap();
    assert_eq!(flying, 2, "both bolts on their way");
    let reply = say(&mut a, ".respawn 50", "Respawned").await;
    assert!(reply.contains("1 creatures"), "{reply}");
    let after = map
        .query(move |m| {
            let lost = m.spells.in_flight.is_empty();
            pass(m, 15_000);
            let (hp, max) = health(m, ga);
            (lost, fight(m, brute), hp == max)
        })
        .await
        .unwrap();
    assert!(after.0, "nothing on its way");
    assert_eq!(after.1, (false, 0, false, false), "the new brute untouched");
    assert!(after.2, "nor Alice by the old one's bolt");
}

/// Alice's ground area by a brute and her damage over time on it as she
/// says `.ready load`: the area and its object go, the damage over time
/// with them, and the brute is hurt no more and in no fight with her.
#[tokio::test]
async fn ready_load_takes_a_players_ground_area_and_damage_over_time() {
    let (_realm, mut a, ga, map) =
        crate::game::tests_p3_combat::setup_with(world(), Class::Mage).await;
    say(&mut a, ".ready save", "Ready state saved").await;
    let brute = guid::unit(BRUTE, NEAR);
    let mine = move |m: &Map| {
        (
            m.spells.areas.iter().filter(|x| x.caster == ga).count(),
            m.spells.dynamic.values().filter(|d| d.caster == ga).count(),
            auras(m, brute).is_some_and(|x| x.has_spell(DOT)),
        )
    };
    let laid = map
        .query(move |m| {
            let at = m.position_of(brute);
            spells::cast_triggered_with(
                m,
                ga,
                GROUND,
                Targets {
                    dest: at,
                    ..Default::default()
                },
            );
            crate::game::tests_p3_aura2::put(m, ga, brute, DOT, &[10]).unwrap();
            mine(m)
        })
        .await
        .unwrap();
    assert_eq!(laid, (1, 1, true), "the area, its object and the DoT");
    say(&mut a, ".ready load", "is ready").await;
    let after = map
        .query(move |m| {
            let gone = mine(m);
            let before = health(m, brute).0;
            pass(m, 5_000);
            let c = m.creature(brute).unwrap();
            let held = c.threat.iter().any(|(t, _)| *t == ga);
            (gone, health(m, brute).0 == before, held, in_combat(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(after.0, (0, 0, false), "all gone");
    assert!(after.1, "the brute hurt no more");
    assert!(!after.2 && !after.3, "and no fight between them");
}
