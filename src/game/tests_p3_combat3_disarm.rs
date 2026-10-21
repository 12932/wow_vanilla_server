//! Phase 3's combat tests: disarm (Thistle Tea's `disarm_test.exs`): a
//! disarmed player's fists and the off hand kept, attack power read anew,
//! the weapon back when the last disarm goes or at death; an armed
//! creature's blows at half (mangos-classic's, not Thistle Tea's four tenths);
//! and the weapon abilities refused.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{ATTR_ON_NEXT_SWING, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_MOD_DISARM, UNIT_FLAG_DISARMED};
use crate::game::map::combat::{self, Outcome};
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, OFF_HAND, aura_of, equip, forced_swing, level, spawn};

const MACE: u32 = 63_100;
const DAGGER: u32 = 63_101;
const DISARM: u32 = 95_700;
const DISARM_TOO: u32 = 95_701;
const WEAPON_STRIKE: u32 = 95_702;
const SHOT: u32 = 95_703;
const KICK: u32 = 95_704;

fn weapon(entry: u32, inventory_type: u32, delay: u32, min: f32, max: f32) -> ItemTemplate {
    let mut t = ItemTemplate {
        entry,
        class: 2,
        subclass: if entry == DAGGER { 15 } else { 4 },
        name: format!("Weapon {entry}"),
        display_id: 1000,
        inventory_type,
        delay,
        stackable: 1,
        ..Default::default()
    };
    t.damages[0] = (min, max, 0);
    t
}

fn ability(id: u32, class: i32, dmg_class: u32, next_swing: bool) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Ability {id}"),
        range_index: 1,
        equipped_item_class: class,
        dmg_class,
        ..Default::default()
    };
    if next_swing {
        s.attributes[0] = ATTR_ON_NEXT_SWING;
    }
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.items.insert(MACE, weapon(MACE, 21, 3000, 70.0, 90.0));
    d.items.insert(DAGGER, weapon(DAGGER, 22, 1500, 20.0, 30.0));
    for s in [
        ability(WEAPON_STRIKE, 2, 2, true),
        ability(SHOT, 2, 3, false),
        ability(KICK, -1, 2, false),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

async fn armed() -> (crate::game::harness::Client, Guid, MapHandle) {
    let realm = crate::game::harness::realm_with(data());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        level(m, g, 60);
        spells::learn(m, g, crate::game::map::derive::SPELL_DUAL_WIELD);
        equip(m, g, MACE, 15);
        equip(m, g, DAGGER, OFF_HAND);
    })
    .await
    .unwrap();
    (a, g, map)
}

/// Main hand's damage and speed, and the off hand's damage.
fn hands(m: &Map, g: Guid) -> ((u32, u32, u32), (u32, u32)) {
    let f = &m.players[&g].fields;
    (
        (
            f.get_f32(UNIT_FIELD_MINDAMAGE) as u32,
            f.get_f32(UNIT_FIELD_MAXDAMAGE) as u32,
            f.get_u32(UNIT_FIELD_BASEATTACKTIME),
        ),
        (
            f.get_f32(UNIT_FIELD_MINOFFHANDDAMAGE) as u32,
            f.get_f32(UNIT_FIELD_MAXOFFHANDDAMAGE) as u32,
        ),
    )
}

fn restat(m: &mut Map, g: Guid) {
    let data = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().apply_stats(&data);
}

fn ap(m: &Map, g: Guid) -> f32 {
    let f = &m.players[&g].fields;
    (f.get_u32(UNIT_FIELD_ATTACK_POWER) as i32 + f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS) as i32)
        as f32
}

#[tokio::test]
async fn a_disarmed_player_fights_with_fists_by_attack_power_and_keeps_the_off_hand() {
    let (_a, g, map) = armed().await;
    let got = map
        .query(move |m| {
            let armed = hands(m, g);
            aura_of(m, g, DISARM, AURA_MOD_DISARM, 0, 0);
            restat(m, g);
            let disarmed = hands(m, g);
            let power = ap(m, g);
            let flag = m.players[&g]
                .fields
                .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_DISARMED);
            (armed, disarmed, power, flag)
        })
        .await
        .unwrap();
    let (armed, disarmed, power, flag) = got;
    let bonus = power / 14.0;
    assert_eq!(
        (armed.0.0, armed.0.2),
        ((70.0 + bonus * 3.0) as u32, 3000),
        "the mace"
    );
    assert_eq!(
        disarmed.0,
        ((1.0 + bonus * 2.0) as u32, (2.0 + bonus * 2.0) as u32, 2000),
        "fists, at two seconds"
    );
    assert_eq!(disarmed.1, armed.1, "the off hand as it was");
    assert!(flag, "the client is told");
}

#[tokio::test]
async fn the_weapon_comes_back_when_the_last_disarm_goes_or_at_death() {
    let (_a, g, map) = armed().await;
    let got = map
        .query(move |m| {
            let armed = hands(m, g).0;
            aura_of(m, g, DISARM, AURA_MOD_DISARM, 0, 0);
            aura_of(m, g, DISARM_TOO, AURA_MOD_DISARM, 0, 0);
            auras::remove_spell(m, g, DISARM, None);
            restat(m, g);
            let one_left = (
                hands(m, g).0,
                m.players[&g]
                    .fields
                    .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_DISARMED),
            );
            auras::remove_spell(m, g, DISARM_TOO, None);
            restat(m, g);
            let none_left = (
                hands(m, g).0,
                m.players[&g]
                    .fields
                    .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_DISARMED),
            );
            // Disarmed, killed: armed again.
            aura_of(m, g, DISARM, AURA_MOD_DISARM, 0, 0);
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            combat::deal_damage(m, brute, g, 1_000_000);
            restat(m, g);
            let dead = auras::auras(m, g).is_some_and(|a| a.has(AURA_MOD_DISARM));
            (armed, one_left, none_left, dead, hands(m, g).0)
        })
        .await
        .unwrap();
    let (armed, one_left, none_left, dead, after_death) = got;
    assert_eq!(one_left.0.2, 2000, "still disarmed");
    assert!(one_left.1);
    assert_eq!(none_left, (armed, false), "the mace again");
    assert!(!dead, "death takes disarm off");
    assert_eq!(after_death.2, 3000);
}

#[tokio::test]
async fn an_armed_creature_disarmed_hits_for_half_and_an_unarmed_one_as_ever() {
    let (_a, g, map) = armed().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for armed in [true, false] {
                let brute = spawn(m, g, BRUTE, 20, 1.0);
                if armed {
                    m.unit_fields_mut(brute)
                        .unwrap()
                        .set_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY, 1000);
                }
                aura_of(m, brute, DISARM, AURA_MOD_DISARM, 0, 0);
                let f = &mut m.players.get_mut(&g).unwrap().fields;
                f.set_u32(UNIT_FIELD_RESISTANCES, 0);
                f.set_u32(UNIT_FIELD_HEALTH, 100_000);
                f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
                forced_swing(m, brute, g, Outcome::Hit);
                out.push(100_000 - m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [50, 100]);
}

#[tokio::test]
async fn a_disarmed_player_is_refused_weapon_abilities_before_paying_but_may_shoot_and_kick() {
    let (_a, g, map) = armed().await;
    let got = map
        .query(move |m| {
            for s in [WEAPON_STRIKE, SHOT, KICK] {
                spells::learn(m, g, s);
            }
            let brute = spawn(m, g, BRUTE, 20, 2.0);
            let at = Targets {
                unit: Some(brute),
                ..Default::default()
            };
            let check = |m: &mut Map, s: u32| {
                let t = m.realm.data.spells.get(s).unwrap().clone();
                crate::game::tests_p3_spells_kit::fresh(m, g);
                spells::checks::check(m, g, &t, &at, None, false, &Default::default())
            };
            let armed = check(m, WEAPON_STRIKE);
            aura_of(m, g, DISARM, AURA_MOD_DISARM, 0, 0);
            let disarmed = [WEAPON_STRIKE, SHOT, KICK].map(|s| check(&mut *m, s));
            // Cast anyway: nothing is queued for the next swing.
            crate::game::tests_p3_spells_kit::fresh(m, g);
            spells::cast(m, g, WEAPON_STRIKE, at, None);
            let queued = m
                .caster(g)
                .is_some_and(|c| c.next_swing == Some(WEAPON_STRIKE));
            (armed, disarmed, queued)
        })
        .await
        .unwrap();
    assert_eq!(got.0, None, "armed, it may");
    assert_eq!(
        got.1,
        [
            Some(crate::game::map::spells::checks::fail::EQUIPPED_ITEM_CLASS),
            None,
            None
        ]
    );
    assert!(!got.2, "nothing queued");
}
