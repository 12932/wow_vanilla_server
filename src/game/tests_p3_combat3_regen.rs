//! Phase 3's power tests, part one (Thistle Tea's `regen_test.exs`): a
//! player's health, mana, rage and energy each tick, the auras that change
//! them, the five-second rule, a druid's hidden reserves, a polymorphed
//! player's quick healing; a creature's third a tick, its mana in a fight,
//! what it is set to regenerate; and a hunter's pet's focus.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::creature_combat::{self, REGEN_EVERY, REGEN_HEALTH};
use crate::game::map::death::Death;
use crate::game::map::pets::Kind;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::stats::{self, FIVE_SECOND_RULE, REGEN_INTERVAL};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, aura_of, data, spawn};

/// Polymorph: a mage's confusion that transforms.
pub(crate) const SHEEP: u32 = 95_300;
/// A transform that isn't Polymorph.
const DISGUISE: u32 = 95_301;

pub(crate) fn world() -> GameData {
    let mut d = data();
    let mut sheep = SpellTemplate {
        id: SHEEP,
        name: "Polymorph".into(),
        family_name: 3,
        prevention_type: 1,
        equipped_item_class: -1,
        duration_index: 0,
        ..Default::default()
    };
    sheep.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MOD_CONFUSE,
        target_a: 6,
        ..Default::default()
    };
    sheep.effects[1] = SpellEffect {
        effect: 6,
        aura: AURA_TRANSFORM,
        misc_value: 856,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(SHEEP, sheep);
    let mut disguise = SpellTemplate {
        id: DISGUISE,
        name: "Disguise".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    disguise.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_TRANSFORM,
        misc_value: 856,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(DISGUISE, disguise);
    d
}

/// Put `spell`'s auras on `g`.
pub(crate) fn put(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).cloned().expect("the spell");
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|&i| s.effects[i].effect != 0)
        .map(|i| (i, 0))
        .collect();
    auras::apply_spell(m, g, g, &s, &effects, auras::Context::default()).expect("applied");
}

/// A player of `class` in the world.
async fn player(class: Class) -> (crate::game::harness::Client, Guid, MapHandle) {
    let race = match class {
        Class::Druid => Race::NightElf,
        Class::Hunter => Race::Dwarf,
        Class::Shaman => Race::Orc,
        _ => Race::Human,
    };
    let realm = crate::game::harness::realm_with(world());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let g = a.enter("Alice", race, class).await;
    let map = realm.registry.online(g).unwrap().map();
    (a, g, map)
}

/// The player's pools: health of `max_health`, spirit, and the four powers
/// with their maximums (mana, rage, focus, energy).
#[derive(Clone, Copy, Default)]
struct Pools {
    health: u32,
    max_health: u32,
    spirit: u32,
    mana: (u32, u32),
    rage: u32,
    energy: (u32, u32),
}

fn set(m: &mut Map, g: Guid, p: Pools) {
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_u32(UNIT_FIELD_MAXHEALTH, p.max_health);
    f.set_u32(UNIT_FIELD_HEALTH, p.health);
    f.set_u32(UNIT_FIELD_STAT0 + 4, p.spirit);
    f.set_u32(UNIT_FIELD_MAXPOWER1, p.mana.1);
    f.set_u32(UNIT_FIELD_POWER1, p.mana.0);
    f.set_u32(UNIT_FIELD_MAXPOWER1 + 1, 1000);
    f.set_u32(UNIT_FIELD_POWER1 + 1, p.rage);
    f.set_u32(UNIT_FIELD_MAXPOWER1 + 3, p.energy.1);
    f.set_u32(UNIT_FIELD_POWER1 + 3, p.energy.0);
}

fn fight(m: &mut Map, g: Guid, on: bool) {
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, on);
}

/// One regeneration tick for `g`, now.
fn tick(m: &mut Map, g: Guid) {
    let now = m.now;
    m.players.get_mut(&g).unwrap().regen_at = now;
    stats::regen(m);
}

/// What the player holds: health, mana, rage and energy.
fn pools(m: &Map, g: Guid) -> (u32, u32, u32, u32) {
    let f = &m.players[&g].fields;
    (
        f.get_u32(UNIT_FIELD_HEALTH),
        f.get_u32(UNIT_FIELD_POWER1),
        f.get_u32(UNIT_FIELD_POWER1 + 1),
        f.get_u32(UNIT_FIELD_POWER1 + 3),
    )
}

/// Take every aura off `g`.
fn bare(m: &mut Map, g: Guid) {
    let spells: Vec<u32> = m.players[&g]
        .auras
        .holders
        .iter()
        .map(|h| h.spell)
        .collect();
    for s in spells {
        auras::remove_spell(m, g, s, None);
    }
    m.players.get_mut(&g).unwrap().health_regen_carry = 0.0;
}

#[tokio::test]
async fn a_warrior_heals_by_spirit_out_of_a_fight_only_and_never_past_full_nor_dead() {
    let (_a, g, map) = player(Class::Warrior).await;
    let out = map
        .query(move |m| {
            let warrior = Pools {
                health: 50,
                max_health: 100,
                spirit: 50,
                ..Default::default()
            };
            let mut out = Vec::new();
            // 1.26 spirit - 22.6: 40 a tick.
            set(m, g, warrior);
            tick(m, g);
            out.push(pools(m, g).0);
            // In a fight, none.
            set(m, g, warrior);
            m.players.get_mut(&g).unwrap().health_regen_carry = 0.0;
            fight(m, g, true);
            tick(m, g);
            out.push(pools(m, g).0);
            fight(m, g, false);
            // Never past full.
            set(
                m,
                g,
                Pools {
                    health: 99,
                    spirit: 200,
                    ..warrior
                },
            );
            tick(m, g);
            out.push(pools(m, g).0);
            // Dead, then a ghost: nothing.
            let now = m.now;
            set(
                m,
                g,
                Pools {
                    health: 0,
                    ..warrior
                },
            );
            m.players.get_mut(&g).unwrap().death = Death::Dead { since: now };
            tick(m, g);
            out.push(pools(m, g).0);
            set(
                m,
                g,
                Pools {
                    health: 1,
                    ..warrior
                },
            );
            m.players.get_mut(&g).unwrap().death = Death::Ghost { since: now };
            tick(m, g);
            out.push(pools(m, g).0);
            m.players.get_mut(&g).unwrap().death = Death::Alive;
            out
        })
        .await
        .unwrap();
    assert_eq!(out, [90, 50, 100, 0, 1]);
}

#[tokio::test]
async fn health_carries_its_fractions_sitting_and_auras_scale_it_and_flat_regeneration_holds_in_a_fight()
 {
    let (_a, g, map) = player(Class::Paladin).await;
    let paladin = map
        .query(move |m| {
            // A quarter of 50 spirit: 12.5, so 22 and then 35.
            set(
                m,
                g,
                Pools {
                    health: 10,
                    max_health: 100,
                    spirit: 50,
                    ..Default::default()
                },
            );
            tick(m, g);
            let first = (pools(m, g).0, m.players[&g].health_regen_carry);
            tick(m, g);
            let second = (pools(m, g).0, m.players[&g].health_regen_carry);
            (first, second)
        })
        .await
        .unwrap();
    assert_eq!(paladin, ((22, 0.5), (35, 0.0)));
    let (_b, w, map) = player(Class::Warrior).await;
    let warrior = Pools {
        health: 10,
        max_health: 100,
        spirit: 50,
        ..Default::default()
    };
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // Sitting: half as much again, 60.
            set(m, w, warrior);
            m.players.get_mut(&w).unwrap().set_stand_state(1);
            tick(m, w);
            out.push(pools(m, w).0);
            m.players.get_mut(&w).unwrap().set_stand_state(0);
            bare(m, w);
            // Ten percent more out of a fight: 44.
            aura_of(m, w, 95_310, AURA_MOD_HEALTH_REGEN_PERCENT, 0, 10);
            set(m, w, warrior);
            tick(m, w);
            out.push(pools(m, w).0);
            bare(m, w);
            // In a fight, a tenth of spirit's share: 4.
            aura_of(m, w, 95_311, AURA_MOD_REGEN_DURING_COMBAT, 0, 10);
            set(m, w, warrior);
            fight(m, w, true);
            tick(m, w);
            out.push(pools(m, w).0);
            bare(m, w);
            // Flat health in a fight: 50 every five seconds, 20 a tick.
            aura_of(m, w, 95_312, AURA_MOD_HEALTH_REGEN_IN_COMBAT, 0, 50);
            set(m, w, warrior);
            tick(m, w);
            out.push(pools(m, w).0);
            fight(m, w, false);
            bare(m, w);
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [70, 54, 14, 30]);
}

#[tokio::test]
async fn low_spirit_takes_nothing_from_flat_health_regeneration() {
    for class in [Class::Warrior, Class::Hunter, Class::Rogue, Class::Shaman] {
        let (_a, g, map) = player(class).await;
        let got = map
            .query(move |m| {
                let low = Pools {
                    health: 10,
                    max_health: 100,
                    spirit: 0,
                    ..Default::default()
                };
                set(m, g, low);
                m.players.get_mut(&g).unwrap().set_stand_state(1);
                tick(m, g);
                let plain = (pools(m, g).0, m.players[&g].health_regen_carry);
                aura_of(m, g, 95_312, AURA_MOD_HEALTH_REGEN_IN_COMBAT, 0, 20);
                set(m, g, low);
                tick(m, g);
                (plain, pools(m, g).0)
            })
            .await
            .unwrap();
        assert_eq!(got, ((10, 0.0), 18), "{class:?}");
    }
}

#[tokio::test]
async fn a_polymorphed_player_heals_a_tenth_a_tick_in_a_fight_in_place_of_spirit() {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            put(m, g, SHEEP);
            // In a fight, a tenth of 105: 11, and the half carried.
            set(
                m,
                g,
                Pools {
                    health: 1,
                    max_health: 105,
                    ..Default::default()
                },
            );
            fight(m, g, true);
            tick(m, g);
            out.push((pools(m, g).0, m.players[&g].health_regen_carry));
            tick(m, g);
            out.push((pools(m, g).0, m.players[&g].health_regen_carry));
            fight(m, g, false);
            // Spirit, sitting and the percent are replaced; the flat five
            // every five seconds stays: 10 + 2.
            aura_of(m, g, 95_310, AURA_MOD_HEALTH_REGEN_PERCENT, 0, 100);
            aura_of(m, g, 95_312, AURA_MOD_HEALTH_REGEN_IN_COMBAT, 0, 5);
            m.players.get_mut(&g).unwrap().health_regen_carry = 0.0;
            set(
                m,
                g,
                Pools {
                    health: 1,
                    max_health: 100,
                    spirit: 1000,
                    ..Default::default()
                },
            );
            m.players.get_mut(&g).unwrap().set_stand_state(1);
            tick(m, g);
            out.push((pools(m, g).0, m.players[&g].health_regen_carry));
            m.players.get_mut(&g).unwrap().set_stand_state(0);
            bare(m, g);
            // No spirit wanted, and never past full.
            put(m, g, SHEEP);
            set(
                m,
                g,
                Pools {
                    health: 99,
                    max_health: 100,
                    ..Default::default()
                },
            );
            tick(m, g);
            out.push((pools(m, g).0, 0.0));
            bare(m, g);
            // Another transform heals nothing in a fight.
            put(m, g, DISGUISE);
            set(
                m,
                g,
                Pools {
                    health: 50,
                    max_health: 100,
                    ..Default::default()
                },
            );
            fight(m, g, true);
            tick(m, g);
            out.push((pools(m, g).0, 0.0));
            fight(m, g, false);
            bare(m, g);
            // A dead sheep stays dead.
            put(m, g, SHEEP);
            let now = m.now;
            set(
                m,
                g,
                Pools {
                    health: 0,
                    max_health: 100,
                    ..Default::default()
                },
            );
            m.players.get_mut(&g).unwrap().death = Death::Dead { since: now };
            tick(m, g);
            out.push((pools(m, g).0, 0.0));
            m.players.get_mut(&g).unwrap().death = Death::Alive;
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            (11, 0.5),
            (22, 0.0),
            (13, 0.0),
            (100, 0.0),
            (50, 0.0),
            (0, 0.0)
        ]
    );
}

#[tokio::test]
async fn mana_comes_from_spirit_outside_the_five_second_rule_in_a_fight_or_out() {
    let (_a, g, map) = player(Class::Mage).await;
    let mage = Pools {
        health: 100,
        max_health: 100,
        spirit: 40,
        mana: (100, 200),
        ..Default::default()
    };
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // A quarter of 40 spirit and 12.5: 22.
            set(m, g, mage);
            tick(m, g);
            out.push(pools(m, g).1);
            // In a fight, the same.
            set(m, g, mage);
            fight(m, g, true);
            tick(m, g);
            out.push(pools(m, g).1);
            fight(m, g, false);
            // Spent two seconds ago: nothing; three seconds on, it comes.
            let now = m.now;
            set(m, g, mage);
            m.players.get_mut(&g).unwrap().mana_spent_at = Some(now - Duration::from_secs(2));
            tick(m, g);
            out.push(pools(m, g).1);
            m.players.get_mut(&g).unwrap().mana_spent_at = Some(now - Duration::from_secs(5));
            tick(m, g);
            out.push(pools(m, g).1);
            // The rule's edge: a millisecond short still holds it.
            set(m, g, mage);
            m.players.get_mut(&g).unwrap().mana_spent_at =
                Some(now - FIVE_SECOND_RULE + Duration::from_millis(1));
            tick(m, g);
            out.push(pools(m, g).1);
            m.players.get_mut(&g).unwrap().mana_spent_at = None;
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [122, 122, 100, 122, 100]);
}

#[tokio::test]
async fn mana_auras_mp5_the_interrupt_share_and_the_percent_work_within_the_five_second_rule() {
    let (_a, g, map) = player(Class::Mage).await;
    let mage = Pools {
        health: 100,
        max_health: 100,
        spirit: 40,
        mana: (100, 500),
        ..Default::default()
    };
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            let now = m.now;
            let casting = |m: &mut Map| {
                m.players.get_mut(&g).unwrap().mana_spent_at = Some(now - Duration::from_secs(1));
            };
            // 41 mana every five seconds: 16 a tick, while casting too.
            aura_of(m, g, 95_320, AURA_MOD_POWER_REGEN, 0, 41);
            set(m, g, mage);
            casting(m);
            tick(m, g);
            out.push(pools(m, g).1);
            bare(m, g);
            // Thirty percent of spirit's while casting: 6.
            aura_of(m, g, 95_321, AURA_MOD_MANA_REGEN_INTERRUPT, 0, 30);
            set(m, g, mage);
            casting(m);
            tick(m, g);
            out.push(pools(m, g).1);
            bare(m, g);
            // Evocation: fifteen hundred percent more and all of it while
            // casting, 22.5 sixteen times over.
            aura_of(m, g, 95_322, AURA_MOD_POWER_REGEN_PERCENT, 0, 1500);
            aura_of(m, g, 95_321, AURA_MOD_MANA_REGEN_INTERRUPT, 0, 100);
            set(
                m,
                g,
                Pools {
                    mana: (100, 5000),
                    ..mage
                },
            );
            casting(m);
            tick(m, g);
            out.push(pools(m, g).1);
            bare(m, g);
            // More than all of it is still all of it.
            aura_of(m, g, 95_321, AURA_MOD_MANA_REGEN_INTERRUPT, 0, 150);
            set(m, g, mage);
            casting(m);
            tick(m, g);
            out.push(pools(m, g).1);
            bare(m, g);
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [116, 106, 460, 122]);
}

#[tokio::test]
async fn energy_comes_in_a_fight_or_out_and_rage_decays_only_out_of_one() {
    let (_a, r, map) = player(Class::Rogue).await;
    let got = map
        .query(move |m| {
            let rogue = Pools {
                health: 100,
                max_health: 100,
                energy: (50, 100),
                ..Default::default()
            };
            set(m, r, rogue);
            tick(m, r);
            let out_of = pools(m, r).3;
            set(
                m,
                r,
                Pools {
                    energy: (95, 100),
                    ..rogue
                },
            );
            fight(m, r, true);
            tick(m, r);
            let fighting = pools(m, r).3;
            fight(m, r, false);
            // Twice as fast.
            aura_of(m, r, 95_330, AURA_MOD_POWER_REGEN_PERCENT, 3, 100);
            set(
                m,
                r,
                Pools {
                    energy: (10, 100),
                    ..rogue
                },
            );
            tick(m, r);
            (out_of, fighting, pools(m, r).3)
        })
        .await
        .unwrap();
    assert_eq!(got, (70, 100, 50));
    let (_b, w, map) = player(Class::Warrior).await;
    let got = map
        .query(move |m| {
            let warrior = Pools {
                health: 100,
                max_health: 100,
                rage: 30,
                ..Default::default()
            };
            let mut out = Vec::new();
            set(m, w, warrior);
            tick(m, w);
            out.push(pools(m, w).2);
            tick(m, w);
            out.push(pools(m, w).2);
            // In a fight it holds.
            set(m, w, warrior);
            fight(m, w, true);
            tick(m, w);
            out.push(pools(m, w).2);
            fight(m, w, false);
            // And while regeneration is stopped.
            aura_of(m, w, 95_331, AURA_INTERRUPT_REGEN, 0, 0);
            set(m, w, warrior);
            tick(m, w);
            out.push(pools(m, w).2);
            bare(m, w);
            // A rage percent aura slows the decay.
            aura_of(m, w, 95_332, AURA_MOD_POWER_REGEN_PERCENT, 1, -50);
            set(m, w, warrior);
            tick(m, w);
            out.push(pools(m, w).2);
            bare(m, w);
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [10, 0, 30, 30, 20]);
}

#[tokio::test]
async fn a_druid_regenerates_every_reserve_whichever_is_shown_each_to_its_own_cap() {
    let (_a, g, map) = player(Class::Druid).await;
    let druid = Pools {
        health: 100,
        max_health: 100,
        spirit: 40,
        mana: (100, 200),
        rage: 30,
        energy: (50, 100),
    };
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // Shown as mana, rage and energy: the same each time.
            for power in [0u8, 1, 3] {
                set(m, g, druid);
                m.players
                    .get_mut(&g)
                    .unwrap()
                    .fields
                    .set_byte(UNIT_FIELD_BYTES_0, 3, power);
                tick(m, g);
                out.push(pools(m, g));
            }
            // Each to its own cap.
            set(
                m,
                g,
                Pools {
                    mana: (195, 200),
                    rage: 10,
                    energy: (95, 100),
                    ..druid
                },
            );
            tick(m, g);
            out.push(pools(m, g));
            // Full energy, short of mana: the mana still comes.
            set(
                m,
                g,
                Pools {
                    rage: 0,
                    energy: (100, 100),
                    ..druid
                },
            );
            tick(m, g);
            out.push(pools(m, g));
            // Full mana, short of energy: the energy still comes.
            set(
                m,
                g,
                Pools {
                    mana: (200, 200),
                    rage: 0,
                    ..druid
                },
            );
            tick(m, g);
            out.push(pools(m, g));
            // Everything full: nothing changes.
            set(
                m,
                g,
                Pools {
                    mana: (200, 200),
                    rage: 0,
                    energy: (100, 100),
                    ..druid
                },
            );
            tick(m, g);
            out.push(pools(m, g));
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_0, 3, 0);
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            (100, 123, 10, 70),
            (100, 123, 10, 70),
            (100, 123, 10, 70),
            (100, 200, 0, 100),
            (100, 123, 0, 100),
            (100, 200, 0, 70),
            (100, 200, 0, 100),
        ]
    );
}

#[tokio::test]
async fn a_shifted_druid_keeps_the_five_second_rule_its_mana_auras_and_nothing_on_a_corpse() {
    let (_a, g, map) = player(Class::Druid).await;
    let druid = Pools {
        health: 100,
        max_health: 100,
        spirit: 40,
        mana: (100, 200),
        rage: 30,
        energy: (50, 100),
    };
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            let now = m.now;
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_0, 3, 3);
            // In cat form in a fight, mana spent four seconds ago: energy
            // comes, mana doesn't; a second on, both.
            set(m, g, druid);
            fight(m, g, true);
            m.players.get_mut(&g).unwrap().mana_spent_at = Some(now - Duration::from_millis(4_999));
            tick(m, g);
            out.push(pools(m, g));
            m.players.get_mut(&g).unwrap().mana_spent_at = Some(now - Duration::from_secs(5));
            set(m, g, druid);
            tick(m, g);
            out.push(pools(m, g));
            // In bear form: mana auras work, rage and energy are as ever:
            // (10 / 5 * 2) + 23 * 5 = 119.
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_0, 3, 1);
            aura_of(m, g, 95_320, AURA_MOD_POWER_REGEN, 0, 10);
            aura_of(m, g, 95_322, AURA_MOD_POWER_REGEN_PERCENT, 0, 400);
            aura_of(m, g, 95_321, AURA_MOD_MANA_REGEN_INTERRUPT, 0, 100);
            set(
                m,
                g,
                Pools {
                    mana: (100, 500),
                    ..druid
                },
            );
            m.players.get_mut(&g).unwrap().mana_spent_at = Some(now - Duration::from_secs(1));
            tick(m, g);
            out.push(pools(m, g));
            bare(m, g);
            fight(m, g, false);
            // A corpse and a ghost: no reserve moves.
            set(m, g, Pools { health: 0, ..druid });
            m.players.get_mut(&g).unwrap().death = Death::Dead { since: now };
            tick(m, g);
            out.push(pools(m, g));
            set(m, g, Pools { health: 1, ..druid });
            m.players.get_mut(&g).unwrap().death = Death::Ghost { since: now };
            tick(m, g);
            out.push(pools(m, g));
            let p = m.players.get_mut(&g).unwrap();
            p.death = Death::Alive;
            p.mana_spent_at = None;
            p.fields.set_byte(UNIT_FIELD_BYTES_0, 3, 0);
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            (100, 100, 30, 70),
            (100, 123, 30, 70),
            (100, 219, 30, 70),
            (0, 100, 30, 50),
            (1, 100, 30, 50),
        ]
    );
}

#[test]
fn players_tick_every_two_seconds_and_creatures_every_five() {
    assert_eq!(REGEN_INTERVAL, Duration::from_secs(2));
    assert_eq!(REGEN_EVERY, Duration::from_secs(5));
    assert_eq!(FIVE_SECOND_RULE, Duration::from_secs(5));
}

/// A creature of `class` with the mana it says, `yards` from `g`.
fn creature(m: &mut Map, g: Guid, class: u8, health: (u32, u32), mana: (u32, u32)) -> Guid {
    let c = spawn(m, g, BRUTE, 20, 30.0);
    let now = m.now;
    let cr = m.creature_mut(c).unwrap();
    cr.fields.set_byte(UNIT_FIELD_BYTES_0, 1, class);
    cr.fields.set_u32(UNIT_FIELD_MAXHEALTH, health.1);
    cr.fields.set_u32(UNIT_FIELD_HEALTH, health.0);
    cr.fields.set_u32(UNIT_FIELD_MAXPOWER1, mana.1);
    cr.fields.set_u32(UNIT_FIELD_POWER1, mana.0);
    cr.engagement.regen_at = Some(now);
    c
}

fn creature_tick(m: &mut Map, c: Guid) {
    let now = m.now;
    if let Some(cr) = m.creature_mut(c) {
        cr.engagement.regen_at = Some(now);
    }
    creature_combat::regenerate(m);
}

fn held(m: &Map, c: Guid) -> (u32, u32) {
    let f = m.unit_fields(c).unwrap();
    (f.get_u32(UNIT_FIELD_HEALTH), f.get_u32(UNIT_FIELD_POWER1))
}

#[tokio::test]
async fn a_creature_regains_a_third_out_of_a_fight_none_in_one_and_nothing_dead() {
    let (_a, g, map) = player(Class::Warrior).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // A third of health and mana, tick by tick.
            let c = creature(m, g, 1, (50, 300), (0, 90));
            creature_tick(m, c);
            out.push(held(m, c));
            creature_tick(m, c);
            out.push(held(m, c));
            // A warrior in a fight: health holds, and of mana only what
            // intellect gives, here none.
            let c = creature(m, g, 1, (50, 300), (0, 185));
            crate::game::map::ai::attacked(m, c, g);
            creature_tick(m, c);
            out.push(held(m, c));
            // Dead: nothing.
            let c = creature(m, g, 1, (50, 300), (0, 90));
            crate::game::map::combat::deal_damage(m, g, c, 1000);
            creature_tick(m, c);
            out.push(held(m, c));
            out
        })
        .await
        .unwrap();
    assert_eq!(got[0], (150, 30));
    assert_eq!(got[1], (250, 60));
    assert_eq!(got[2].0, 50, "no health in a fight");
    assert_eq!(got[3].0, 0, "the dead don't heal");
}

#[tokio::test]
async fn a_creature_regenerates_only_what_it_is_set_to() {
    let (_a, g, map) = player(Class::Warrior).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for regenerate in [Some(0), Some(REGEN_HEALTH)] {
                let c = creature(m, g, 8, (50, 300), (0, 90));
                crate::game::map::event_ai::scripted_mut(m, c)
                    .expect("a script's state")
                    .regenerate = regenerate;
                creature_tick(m, c);
                out.push(held(m, c));
            }
            // Health only, in a fight: no mana either.
            let c = creature(m, g, 8, (300, 300), (0, 185));
            crate::game::map::event_ai::scripted_mut(m, c)
                .expect("a script's state")
                .regenerate = Some(REGEN_HEALTH);
            crate::game::map::ai::attacked(m, c, g);
            creature_tick(m, c);
            out.push(held(m, c));
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [(50, 0), (150, 0), (300, 0)]);
}

#[tokio::test]
async fn a_hunters_pet_regains_focus_in_a_fight_or_out_and_its_percent_auras_count() {
    let (_a, g, map) = player(Class::Hunter).await;
    let got = map
        .query(move |m| {
            let at = m.players[&g].position();
            let pet =
                crate::game::map::pets::testing::spawn(m, g, BRUTE, Kind::Hunter, 20, at, 0.0);
            m.players.get_mut(&g).unwrap().owned.pet = Some(pet);
            let set_focus = |m: &mut Map, focus: u32| {
                let now = m.now;
                let c = m.creature_mut(pet).unwrap();
                c.fields.set_u32(UNIT_FIELD_MAXPOWER1 + 2, 100);
                c.fields.set_u32(UNIT_FIELD_POWER1 + 2, focus);
                if let Some(s) = c.pet.as_deref_mut() {
                    s.regen_at = now;
                }
            };
            let focus = |m: &Map| m.unit_fields(pet).unwrap().get_u32(UNIT_FIELD_POWER1 + 2);
            let now = m.now;
            set_focus(m, 10);
            crate::game::map::pets::testing::regen(m, pet, now);
            let out_of = focus(m);
            set_focus(m, 95);
            m.creature_mut(pet).unwrap().fields.set_flag(
                UNIT_FIELD_FLAGS,
                UNIT_FLAG_IN_COMBAT,
                true,
            );
            crate::game::map::pets::testing::regen(m, pet, now);
            let fighting = focus(m);
            aura_of(m, pet, 95_340, AURA_MOD_POWER_REGEN_PERCENT, 2, 100);
            set_focus(m, 10);
            crate::game::map::pets::testing::regen(m, pet, now);
            (out_of, fighting, focus(m))
        })
        .await
        .unwrap();
    // vmangos's 24 every four seconds, 12 every two, and twice that.
    assert_eq!(got, (22, 100, 34));
}
