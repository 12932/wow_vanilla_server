//! Round 10, against Thistle Tea's `stats_test.exs`,
//! `stats/attack_power_test.exs` and `stats/resistances_test.exs`: the
//! creature stat model. A creature's stats are its class's at its level
//! through its auras, and their change moves its health (ten a point of
//! stamina), its mana (fifteen a point of intellect, with mana already),
//! its armor (its agility, two a point for a hunter's pet) and its attack
//! power (its class's formula, unclamped, against its seed); a pet's
//! attack power is its strength's as it is.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::creature::Creature;
use crate::game::map::pets::{Kind, PetState};
use crate::game::map::{Map, auras, derive};
use crate::game::tests_p3_combat::*;

const STAT_UP: u32 = 98_101;
const AGILITY_UP: u32 = 98_102;
const ALL_UP: u32 = 98_103;
const AP_DOWN: u32 = 98_104;
const RAP_UP: u32 = 98_105;
const AP_UP: u32 = 98_106;
const AP_SAPPED: u32 = 98_107;

/// What a creature starts with: its class and level, strength to spirit,
/// melee and ranged attack power and damage, health, mana and armor.
struct Seed {
    class: u8,
    stats: [u32; 5],
    ap: u32,
    rap: u32,
    damage: f32,
    ranged: f32,
    health: u32,
    mana: u32,
    armor: u32,
}

const WARRIOR: Seed = Seed {
    class: 1,
    stats: [100, 50, 10, 10, 10],
    ap: 200,
    rap: 100,
    damage: 100.0,
    ranged: 50.0,
    health: 1_000,
    mana: 0,
    armor: 100,
};

fn place(m: &mut Map, g: Guid, s: &Seed, pet: Option<Kind>) -> Guid {
    let mut t = template(BRUTE, 7);
    t.unit_class = s.class;
    let pos = m.players[&g].position();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let at = Vector3d {
        x: pos.x + 3.0,
        ..pos
    };
    let mut c = Creature::new(guid::unit(BRUTE, counter | 0x80_0000), &t, 60, at, 0.0);
    let f = &mut c.fields;
    for (i, v) in s.stats.iter().enumerate() {
        f.set_u32(UNIT_FIELD_STAT0 + i as u16, *v);
    }
    f.set_u32(UNIT_FIELD_ATTACK_POWER, s.ap);
    f.set_u32(UNIT_FIELD_RANGED_ATTACK_POWER, s.rap);
    f.set_f32(UNIT_FIELD_MINDAMAGE, s.damage);
    f.set_f32(UNIT_FIELD_MAXDAMAGE, s.damage);
    f.set_f32(UNIT_FIELD_MINRANGEDDAMAGE, s.ranged);
    f.set_f32(UNIT_FIELD_MAXRANGEDDAMAGE, s.ranged);
    f.set_u32(UNIT_FIELD_MAXHEALTH, s.health);
    f.set_u32(UNIT_FIELD_HEALTH, s.health);
    f.set_u32(UNIT_FIELD_MAXPOWER1, s.mana);
    f.set_u32(UNIT_FIELD_POWER1, s.mana);
    f.set_u32(UNIT_FIELD_RESISTANCES, s.armor);
    if let Some(kind) = pet {
        c.pet = Some(Box::new(PetState::new(kind, g, m.now)));
    }
    let c = c.with_base();
    m.add_creature(c)
}

/// Its attack power and ranged attack power, the fields' melee and ranged
/// least damage, health, mana and armor.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sheet {
    ap: u32,
    rap: u32,
    min: f32,
    ranged_min: f32,
    health: u32,
    mana: u32,
    armor: u32,
}

fn sheet(m: &Map, c: Guid) -> Sheet {
    let d = m.creature(c).unwrap().derived;
    let f = m.unit_fields(c).unwrap();
    Sheet {
        ap: d.attack_power,
        rap: d.ranged_attack_power,
        min: (f.get_f32(UNIT_FIELD_MINDAMAGE) * 100.0).round() / 100.0,
        ranged_min: (f.get_f32(UNIT_FIELD_MINRANGEDDAMAGE) * 100.0).round() / 100.0,
        health: f.get_u32(UNIT_FIELD_MAXHEALTH),
        mana: f.get_u32(UNIT_FIELD_MAXPOWER1),
        armor: f.get_u32(UNIT_FIELD_RESISTANCES),
    }
}

/// The creature under these test auras, `(id, kind, misc, amount)`, and
/// without them again.
fn under(m: &mut Map, c: Guid, list: &[(u32, u32, i32, i32)]) -> (Sheet, Sheet) {
    for &(id, kind, misc, amount) in list {
        aura_of(m, c, id, kind, misc, amount);
    }
    derive::unit(m, c);
    let on = sheet(m, c);
    for &(id, ..) in list {
        auras::remove_spell(m, c, id, None);
    }
    derive::unit(m, c);
    (on, sheet(m, c))
}

#[tokio::test]
async fn a_creatures_stats_change_moves_its_attack_power_health_mana_and_armor() {
    let (_realm, _a, g, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = place(m, g, &WARRIOR, None);
            let seed = sheet(m, brute);
            // Ten strength and twenty agility: a warrior's 2 a point of
            // strength, a point of agility its ranged; its damage by the
            // ratio, its health as it was.
            let stats = under(
                m,
                brute,
                &[
                    (STAT_UP, auras::AURA_MOD_STAT, 0, 10),
                    (AGILITY_UP, auras::AURA_MOD_STAT, 1, 20),
                ],
            );
            // A hundred less melee and a hundred more ranged: each by its
            // own ratio.
            let powers = under(
                m,
                brute,
                &[
                    (AP_DOWN, auras::AURA_MOD_ATTACK_POWER, 0, -100),
                    (RAP_UP, auras::AURA_MOD_RANGED_ATTACK_POWER, 0, 100),
                ],
            );
            // Ten of every stat: a hundred health, no mana for a warrior
            // without it, ten armor.
            let all = under(m, brute, &[(ALL_UP, auras::AURA_MOD_STAT, -1, 10)]);
            // A mage of five strength and agility, ten of each: its
            // formula's change, not clamped at nothing (-5 to 5).
            let mage = place(
                m,
                g,
                &Seed {
                    class: 8,
                    stats: [5, 5, 10, 10, 10],
                    mana: 500,
                    ..WARRIOR
                },
                None,
            );
            let mage_seed = sheet(m, mage);
            let mage_up = under(
                m,
                mage,
                &[
                    (STAT_UP, auras::AURA_MOD_STAT, 0, 10),
                    (AGILITY_UP, auras::AURA_MOD_STAT, 1, 10),
                ],
            );
            let mage_all = under(m, mage, &[(ALL_UP, auras::AURA_MOD_STAT, -1, 10)]);
            (seed, stats, powers, all, mage_seed, mage_up, mage_all)
        })
        .await
        .unwrap();
    let (seed, stats, powers, all, mage_seed, mage_up, mage_all) = r;
    // Its agility's 50 armor on its 100.
    assert_eq!(
        seed,
        Sheet {
            ap: 200,
            rap: 100,
            min: 100.0,
            ranged_min: 50.0,
            health: 1_000,
            mana: 0,
            armor: 150,
        }
    );
    // (7 x 200 + 3 x 220) / 2000 of 100; ranged (700 + 360) / 1000 of 50.
    assert_eq!(
        stats.0,
        Sheet {
            ap: 220,
            rap: 120,
            min: 103.0,
            ranged_min: 53.0,
            armor: 170,
            ..seed
        }
    );
    assert_eq!(stats.1, seed);
    assert_eq!(
        powers.0,
        Sheet {
            ap: 100,
            rap: 200,
            min: 85.0,
            ranged_min: 65.0,
            ..seed
        }
    );
    assert_eq!(powers.1, seed);
    assert_eq!(all.0.health, 1_100);
    assert_eq!(all.0.mana, 0);
    assert_eq!(all.0.armor, 160);
    assert_eq!(all.0.ap, 220);
    assert_eq!(all.1, seed);
    assert_eq!((mage_seed.ap, mage_seed.rap), (200, 100));
    assert_eq!((mage_up.0.ap, mage_up.0.rap), (210, 110));
    assert_eq!(mage_up.1, mage_seed);
    // A creature with mana takes fifteen a point of intellect.
    assert_eq!(mage_all.0.mana, 650);
    assert_eq!(mage_all.0.health, 1_100);
    assert_eq!(mage_all.1, mage_seed);
}

#[tokio::test]
async fn a_pets_attack_power_is_its_strengths_and_a_hunters_pets_damage_follows_it() {
    let (_realm, _a, g, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            // A hunter's pet of 100 strength: 180 attack power, its 100
            // damage made 125 by its happiness.
            let pet = Seed {
                ap: 180,
                damage: 125.0,
                ..WARRIOR
            };
            let beast = place(m, g, &pet, Some(Kind::Hunter));
            let beast_seed = sheet(m, beast);
            let mut beast_ap = Vec::new();
            for amount in [180, -500] {
                beast_ap.push(
                    under(
                        m,
                        beast,
                        &[(AP_UP, auras::AURA_MOD_ATTACK_POWER, 0, amount)],
                    )
                    .0,
                );
            }
            let beast_strong = under(m, beast, &[(STAT_UP, auras::AURA_MOD_STAT, 0, 10)]).0;
            // A demon and an imp: their strength's attack power moves, but
            // not their damage; a flat change does.
            let mut demons = Vec::new();
            for imp in [false, true] {
                let d = place(
                    m,
                    g,
                    &Seed {
                        damage: 100.0,
                        ..WARRIOR
                    },
                    Some(Kind::Demon),
                );
                if imp {
                    m.creature_mut(d).unwrap().entry = 416;
                }
                derive::unit(m, d);
                let seed = sheet(m, d);
                let strong = under(m, d, &[(STAT_UP, auras::AURA_MOD_STAT, 0, 10)]).0;
                let sapped = under(
                    m,
                    d,
                    &[
                        (STAT_UP, auras::AURA_MOD_STAT, 0, 10),
                        (AP_SAPPED, auras::AURA_MOD_ATTACK_POWER, 0, -50),
                    ],
                )
                .0;
                demons.push((seed, strong, sapped));
            }
            (beast_seed, beast_ap, beast_strong, demons)
        })
        .await
        .unwrap();
    let (beast_seed, beast_ap, beast_strong, demons) = r;
    assert_eq!((beast_seed.ap, beast_seed.min), (180, 125.0));
    // Two armor a point of its agility's 50.
    assert_eq!(beast_seed.armor, 200);
    // No ranged attack power, as a pet has none.
    assert_eq!(beast_seed.rap, 0);
    assert_eq!((beast_ap[0].ap, beast_ap[0].min), (360, 162.5));
    assert_eq!((beast_ap[1].ap, beast_ap[1].min), (0, 87.5));
    // Ten strength: twenty attack power, (1260 + 600) / 1800 of 125.
    assert_eq!(beast_strong.ap, 200);
    assert_eq!(beast_strong.min, 129.17);
    let (demon, imp) = (demons[0], demons[1]);
    assert_eq!((demon.0.ap, demon.0.min), (180, 100.0));
    assert_eq!(demon.0.armor, 150);
    assert_eq!((demon.1.ap, demon.1.min), (200, 100.0));
    assert_eq!((demon.2.ap, demon.2.min), (150, 92.5));
    assert_eq!((imp.0.ap, imp.0.min), (90, 100.0));
    assert_eq!((imp.1.ap, imp.1.min), (100, 100.0));
    assert_eq!((imp.2.ap, imp.2.min), (50, 85.0));
}

const SURE: u32 = 98_110;
const NEVER_CRIT: u32 = 98_111;
const ALWAYS_CRIT: u32 = 98_112;
const VERSUS_UNDEAD: u32 = 98_113;
const RANGED_VERSUS_UNDEAD: u32 = 98_114;
const MARKED: u32 = 98_115;
/// A normalized blow at 150% of the weapon; a shot of the ranged weapon's.
const NORMALIZED_STRIKE: u32 = 98_116;
const SHOT: u32 = 98_117;

fn weapon_spells(d: &mut GameData) {
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    for (id, dmg_class) in [(NORMALIZED_STRIKE, 2), (SHOT, 3)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Blow {id}"),
            equipped_item_class: -1,
            range_index: 1,
            dmg_class,
            ..Default::default()
        };
        if id == NORMALIZED_STRIKE {
            s.effects[0] = SpellEffect {
                effect: 121,
                base_points: -1,
                target_a: 6,
                ..Default::default()
            };
            s.effects[1] = SpellEffect {
                effect: 31,
                base_points: 149,
                base_dice: 1,
                target_a: 6,
                ..Default::default()
            };
        } else {
            s.effects[0] = SpellEffect {
                effect: 58,
                base_points: -1,
                target_a: 6,
                ..Default::default()
            };
        }
        d.spells.spells.insert(id, s);
    }
}

/// An undead of its own.
const UNDEAD: u32 = 3_710;

/// An undead or a beast to strike, still and armorless.
fn victim(m: &mut Map, g: Guid, entry: u32) -> Guid {
    let t = m.realm.data.creature_template(entry).cloned().unwrap();
    let pos = m.players[&g].position();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let at = Vector3d {
        x: pos.x + 2.0,
        ..pos
    };
    let mut c = Creature::new(guid::unit(entry, counter | 0x80_0000), &t, 60, at, 0.0);
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 100_000);
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, 0);
    let flags = c.fields.get_u32(UNIT_FIELD_FLAGS);
    c.fields.set_u32(UNIT_FIELD_FLAGS, flags | 0x0004_0000);
    let c = c.with_base();
    m.add_creature(c)
}

/// What `spell` takes off `target`, from a full health.
fn blow(m: &mut Map, g: Guid, target: Guid, spell: u32) -> u32 {
    let f = m.unit_fields_mut(target).unwrap();
    f.set_u32(UNIT_FIELD_HEALTH, 100_000);
    crate::game::map::spells::cast_triggered(m, g, spell, Some(target));
    100_000 - m.unit_fields(target).unwrap().get_u32(UNIT_FIELD_HEALTH)
}

/// A weapon spell takes the caster's attack power against the target's
/// creature type (auras 102 and 131) at the weapon's normalized speed,
/// after the weapon's percentage and before the crit, and Hunter's Mark's
/// with it on a shot, once (Thistle Tea's `TargetAttackPower`, mangos's
/// `MeleeDamageBonusDone`).
#[tokio::test]
async fn a_weapon_spell_takes_attack_power_against_its_targets_type_after_the_weapons_share() {
    let mut d = data();
    weapon_spells(&mut d);
    d.creature_templates.insert(UNDEAD, template(UNDEAD, 6));
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            crate::game::tests_p3_combat::level(m, g, 60);
            // A sword (2.4 normalized) of no damage of its own.
            equip(m, g, SWORD, 15);
            aura_of(m, g, SURE, auras::AURA_MOD_HIT_CHANCE, 0, 100);
            aura_of(m, g, NEVER_CRIT, auras::AURA_MOD_CRIT_PERCENT, 0, -100);
            derive::unit(m, g);
            let undead = victim(m, g, UNDEAD);
            let beast = victim(m, g, BEAST);
            let plain = blow(m, g, undead, NORMALIZED_STRIKE);
            aura_of(
                m,
                g,
                VERSUS_UNDEAD,
                auras::AURA_MOD_MELEE_ATTACK_POWER_VERSUS,
                32,
                140,
            );
            derive::unit(m, g);
            let against = blow(m, g, undead, NORMALIZED_STRIKE);
            let on_beast = blow(m, g, beast, NORMALIZED_STRIKE);
            auras::remove_spell(m, g, NEVER_CRIT, None);
            aura_of(m, g, ALWAYS_CRIT, auras::AURA_MOD_CRIT_PERCENT, 0, 100);
            derive::unit(m, g);
            let crit = blow(m, g, undead, NORMALIZED_STRIKE);
            auras::remove_spell(m, g, ALWAYS_CRIT, None);
            aura_of(m, g, NEVER_CRIT, auras::AURA_MOD_CRIT_PERCENT, 0, -100);
            // A shot of nothing at 2.8 seconds: the ranged aura's 140 and
            // the mark's 70, not the melee aura's.
            aura_of(
                m,
                g,
                VERSUS_UNDEAD + 100,
                auras::AURA_MOD_MELEE_ATTACK_POWER_VERSUS,
                32,
                999,
            );
            aura_of(
                m,
                g,
                RANGED_VERSUS_UNDEAD,
                auras::AURA_MOD_RANGED_ATTACK_POWER_VERSUS,
                32,
                140,
            );
            derive::unit(m, g);
            let f = m.unit_fields_mut(g).unwrap();
            f.set_f32(UNIT_FIELD_MINRANGEDDAMAGE, 0.0);
            f.set_f32(UNIT_FIELD_MAXRANGEDDAMAGE, 0.0);
            f.set_u32(UNIT_FIELD_RANGEDATTACKTIME, 2_800);
            for t in [undead, beast] {
                aura_of(
                    m,
                    t,
                    MARKED,
                    auras::AURA_RANGED_ATTACK_POWER_ATTACKER_BONUS,
                    0,
                    70,
                );
            }
            let marked = blow(m, g, undead, SHOT);
            let marked_beast = blow(m, g, beast, SHOT);
            auras::remove_spell(m, undead, MARKED, None);
            let unmarked = blow(m, g, undead, SHOT);
            let ap = m.players[&g].derived.attack_power;
            (
                ap,
                plain,
                against,
                on_beast,
                crit,
                marked,
                marked_beast,
                unmarked,
            )
        })
        .await
        .unwrap();
    let (ap, plain, against, on_beast, crit, marked, marked_beast, unmarked) = r;
    // Her own: her attack power at 2.4, by 150%.
    assert_eq!(plain, (ap as f32 / 14.0 * 2.4 * 1.5) as u32, "{r:?}");
    // 140 at 2.4 is 24 more, not by the 150%; not on a beast.
    assert_eq!(against, plain + 24, "{r:?}");
    assert_eq!(on_beast, plain, "{r:?}");
    assert_eq!(crit, against * 2, "{r:?}");
    // (140 + 70) / 14 x 2.8; the mark's alone on a beast; the type's alone.
    assert_eq!((marked, marked_beast, unmarked), (42, 14, 28), "{r:?}");
}
