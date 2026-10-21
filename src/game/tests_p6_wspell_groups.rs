//! Phase 3, round 6, against Thistle Tea's `raid_spell_target_test.exs`
//! and the group cases of `spell_target_resolver_test.exs`: whom a raid's,
//! a party's and a party member's spells land on, their pets with them,
//! and a pet's spells at its master.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, health, set_health};
use super::tests_p6_wspell::{RANGE_30, creature, go, off, put, sides, unit_at};
use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellRank, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_ADD_PCT_MODIFIER, AURA_MOD_STAT, AURA_PERIODIC_TRIGGER_SPELL};
use crate::game::map::pets::Kind;
use crate::game::map::spells::{Targets, mods};
use crate::game::social::groups;

/// A beast to be a pet, of a warrior's class.
const PET: u32 = 99_610;
/// Radii: 5, 30 and 100 yards; forty yards' range.
const R5: u32 = 991;
const R30: u32 = 992;
const R100: u32 = 993;
const RANGE_40: u32 = 952;

/// Damage about the caster's raid in 5 yards (Plague's), of the mage's
/// family; a fifth more radius for it.
const RAID_HARM: u32 = 991_100;
const WIDER: u32 = 991_101;
/// A buff of the party about the caster; of the raid's class; of the
/// party about the one aimed at; of a party member; one of level 48.
const PARTY_BUFF: u32 = 991_102;
const CLASS_BUFF: u32 = 991_103;
const TARGET_PARTY: u32 = 991_104;
const PARTY_ONLY: u32 = 991_105;
const HIGH_RANK: u32 = 991_106;
/// A pet's aura at its master and one at itself; damage at an enemy with
/// an aura at it and the master; a threat change at an enemy and an aura at
/// the caster.
const MASTER_AND_SELF: u32 = 991_107;
const ENEMY_AND_MASTER: u32 = 991_108;
const ENEMY_AND_SELF: u32 = 991_109;
/// Plague: every three seconds, its effect about the one it is on.
const PLAGUE: u32 = 991_110;
const PLAGUE_EFFECT: u32 = 991_111;

fn buff(target: u32) -> SpellEffect {
    aura(AURA_MOD_STAT, 0, 1, target)
}

fn data(d: &mut GameData) {
    sides(d);
    let mut pet = creature(PET, 1, 11);
    pet.unit_class = 1;
    d.creature_templates.insert(PET, pet);
    d.spells.radii.insert(R5, 5.0);
    d.spells.radii.insert(R30, 30.0);
    d.spells.radii.insert(R100, 100.0);
    d.spells.ranges.insert(RANGE_40, (0.0, 40.0));
    let one = |id: u32, range: u32, e: SpellEffect| {
        let mut s = bare(id, 0);
        s.range_index = range;
        s.duration_index = LASTS_LONG;
        effect(&mut s, 0, e);
        s
    };
    let mut raid = one(
        RAID_HARM,
        RANGE_30,
        SpellEffect {
            target_a: 56,
            radius_index: R5,
            ..damage(10)
        },
    );
    raid.family_name = 3;
    raid.family_flags = 0x40;
    let mut wider = one(
        WIDER,
        1,
        SpellEffect {
            item_type: 0x40,
            ..aura(AURA_ADD_PCT_MODIFIER, mods::OP_RADIUS as i32, 20, 1)
        },
    );
    wider.family_name = 3;
    let around = |id: u32, target: u32, radius: u32| {
        one(
            id,
            RANGE_40,
            SpellEffect {
                radius_index: radius,
                ..buff(target)
            },
        )
    };
    let mut high = around(HIGH_RANK, 37, R30);
    high.spell_level = 48;
    d.spells.ranks.insert(
        HIGH_RANK,
        SpellRank {
            prev: 0,
            first: HIGH_RANK,
            rank: 1,
            req: 0,
        },
    );
    let mut master_and_self = one(MASTER_AND_SELF, RANGE_30, buff(27));
    effect(&mut master_and_self, 1, buff(1));
    let mut enemy_and_master = one(ENEMY_AND_MASTER, RANGE_30, damage(10));
    effect(
        &mut enemy_and_master,
        1,
        SpellEffect {
            target_b: 27,
            ..aura(AURA_MOD_STAT, 0, -1, 6)
        },
    );
    let mut enemy_and_self = one(
        ENEMY_AND_SELF,
        RANGE_30,
        SpellEffect {
            effect: 63,
            base_points: 9,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    effect(&mut enemy_and_self, 1, buff(1));
    let mut plague = one(
        PLAGUE,
        RANGE_30,
        SpellEffect {
            amplitude: 3_000,
            trigger_spell: PLAGUE_EFFECT,
            ..aura(AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 6)
        },
    );
    plague.attributes[0] |= crate::game::data::spells::ATTR_DEBUFF;
    let mut effect_of = one(
        PLAGUE_EFFECT,
        RANGE_30,
        SpellEffect {
            target_a: 56,
            radius_index: R5,
            ..damage(100)
        },
    );
    effect_of.school = 3;
    let spells: Vec<SpellTemplate> = vec![
        raid,
        wider,
        around(PARTY_BUFF, 20, R30),
        around(CLASS_BUFF, 61, R100),
        around(TARGET_PARTY, 37, R30),
        one(PARTY_ONLY, RANGE_30, buff(35)),
        high,
        master_and_self,
        enemy_and_master,
        enemy_and_self,
        plague,
        effect_of,
    ];
    insert(d, spells);
}

/// `owner`'s pet of a warrior's class, `dx` yards east of `origin`.
fn pet_for(m: &mut Map, owner: Guid, origin: Guid, dx: f32) -> Guid {
    let to = off(m, origin, dx, 0.0);
    let pet = crate::game::map::pets::testing::spawn(m, owner, PET, Kind::Hunter, 60, to, 0.0);
    m.players.get_mut(&owner).unwrap().owned.pet = Some(pet);
    if let Some(f) = m.unit_fields_mut(pet) {
        f.set_u32(UNIT_FIELD_MAXHEALTH, 5_000);
        f.set_u32(UNIT_FIELD_HEALTH, 5_000);
    }
    pet
}

/// `members` joined to `leader`'s group, a raid if `raid`.
fn group(realm: &Arc<Realm>, leader: Guid, members: &[Guid], raid: bool) {
    let lo = realm.registry.online(leader).unwrap();
    for m in members {
        let mo = realm.registry.online(*m).unwrap();
        assert!(groups::join_directly(realm, &lo, &mo));
    }
    if raid {
        groups::convert_to_raid(realm, &lo);
    }
}

fn subgroup(realm: &Arc<Realm>, leader: Guid, name: &str, sub: u8) {
    let lo = realm.registry.online(leader).unwrap();
    groups::change_subgroup(realm, &lo, name, sub);
}

fn sorted(mut v: Vec<Guid>) -> Vec<Guid> {
    v.sort_by_key(|g| g.guid());
    v
}

/// Whom `spell` from `caster` at `t` lands on, as the resolver has it.
fn landing(m: &mut Map, caster: Guid, spell: u32, t: Targets) -> Vec<Guid> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    let v: Vec<Guid> = (0..3)
        .filter(|i| s.effects[*i].effect != 0)
        .flat_map(|i| {
            crate::game::map::spells::targets::effect_targets(m, caster, &s, i, &t, &mods)
        })
        .collect();
    let mut v = sorted(v);
    v.dedup();
    v
}

/// `a` and `b` in a duel, fighting.
fn duel(m: &mut Map, a: Guid, b: Guid) {
    use crate::game::map::duel::{Duel, Phase};
    let arbiter = Guid::new(0xF110_0000_0000_0001);
    for (me, them, initiator) in [(a, b, true), (b, a, false)] {
        m.players.get_mut(&me).unwrap().duel = Some(Duel {
            opponent: them,
            flag: arbiter,
            phase: Phase::Fighting,
            initiator,
            out_of_bounds_since: None,
            started: None,
        });
    }
}

/// A harmful raid spell about its carrier lands on its raid across the
/// subgroups and on their pets in its radius, not on the carrier nor one
/// outside the raid; a pet's, on its master's raid but not on itself; an
/// ungrouped carrier's, on its pet, and its pet's on it; the carrier's
/// radius modifiers widen it (Thistle Tea's `RaidSpellTarget`; mangos's
/// `FillRaidOrPartyTargets` with pets, the caster only for a helpful
/// spell).
#[tokio::test]
async fn a_raid_spell_crosses_subgroups_with_pets_and_spares_its_carrier() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P6WG", "Carrier", Class::Mage).await;
    let (_d, b, _) = human(&realm, "P6WH", "Member", Class::Mage).await;
    let (_e, outsider, _) = human(&realm, "P6WI", "Outsider", Class::Mage).await;
    group(&realm, a, &[b], true);
    subgroup(&realm, a, "Member", 1);
    let first = map
        .query(move |m| {
            kit::ready(m, a, 60, &[RAID_HARM]);
            let (to_b, to_o) = (off(m, a, 3.0, 0.0), off(m, a, 2.0, 0.0));
            put(m, b, to_b);
            put(m, outsider, to_o);
            let pet = pet_for(m, b, a, 2.0);
            assert!(cast_fresh(m, a, RAID_HARM, Targets::default()));
            pet
        })
        .await
        .unwrap();
    let pet = first;
    assert_eq!(
        sorted(go(&mut c, RAID_HARM).await),
        sorted(vec![b, pet]),
        "across subgroups, with the pet, not the carrier nor the outsider"
    );
    let out = map
        .query(move |m| {
            let mut out = Vec::new();
            let far = off(m, a, 20.0, 0.0);
            put(m, b, far);
            out.push(landing(m, a, RAID_HARM, Targets::default()));
            let past = off(m, a, 5.01, 0.0);
            put(m, pet, past);
            out.push(landing(m, a, RAID_HARM, Targets::default()));
            // Back, the member 5.5 yards off: a fifth more radius reaches.
            let (to_b, to_p) = (off(m, a, 5.5, 0.0), off(m, a, 2.0, 0.0));
            put(m, b, to_b);
            put(m, pet, to_p);
            out.push(landing(m, a, RAID_HARM, Targets::default()));
            kit::apply(m, a, a, WIDER);
            out.push(landing(m, a, RAID_HARM, Targets::default()));
            crate::game::map::auras::remove_spell(m, a, WIDER, None);
            out.push(landing(m, a, RAID_HARM, Targets::default()));
            // The member dead, then their pet; the member back, in a duel.
            let near = off(m, a, 3.0, 0.0);
            put(m, b, near);
            set_health(m, b, 0);
            crate::game::map::death::kill(m, b);
            out.push(landing(m, a, RAID_HARM, Targets::default()));
            crate::game::map::combat::kill(m, a, pet);
            out.push(landing(m, a, RAID_HARM, Targets::default()));
            out
        })
        .await
        .unwrap();
    assert_eq!(out[0], [pet], "the member out of reach");
    assert_eq!(out[1], Vec::<Guid>::new(), "the pet out of reach");
    assert_eq!(out[2], sorted(vec![pet]), "5.5 yards beyond 5");
    assert_eq!(out[3], sorted(vec![b, pet]), "a fifth more radius");
    assert_eq!(out[4], sorted(vec![pet]), "and back");
    assert_eq!(out[5], [pet], "not the dead member");
    assert_eq!(out[6], Vec::<Guid>::new(), "nor the dead pet");
}

/// A pet's raid spell is its master's: on the master and the raid, not
/// the pet; an ungrouped carrier's on its pet and its pet's on it; a
/// raid member in a duel with the carrier, and their pet, are spared.
#[tokio::test]
async fn a_pets_raid_spell_is_its_masters_and_a_duel_spares_the_other_side() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6WJ", "Owner", Class::Mage).await;
    let (_d, b, _) = human(&realm, "P6WK", "Raider", Class::Mage).await;
    group(&realm, a, &[b], true);
    let out = map
        .query(move |m| {
            let near = off(m, a, 3.0, 0.0);
            put(m, b, near);
            let pet = pet_for(m, a, a, 2.0);
            let mine = landing(m, pet, RAID_HARM, Targets::default());
            let b_pet = pet_for(m, b, a, 1.0);
            duel(m, a, b);
            let dueling = landing(m, a, RAID_HARM, Targets::default());
            for g in [a, b] {
                m.players.get_mut(&g).unwrap().duel = None;
            }
            let after = landing(m, a, RAID_HARM, Targets::default());
            (pet, b_pet, mine, dueling, after)
        })
        .await
        .unwrap();
    let (pet, b_pet, mine, dueling, after) = out;
    assert_eq!(mine, sorted(vec![a, b]), "the master and the raid");
    assert_eq!(dueling, [pet], "not the duel's other side or their pet");
    assert_eq!(after, sorted(vec![b, pet, b_pet]), "after the duel");
    let lb = realm.registry.online(b).unwrap();
    groups::leave(&realm, &lb);
    let alone = map
        .query(move |m| {
            (
                landing(m, a, RAID_HARM, Targets::default()),
                landing(m, pet, RAID_HARM, Targets::default()),
            )
        })
        .await
        .unwrap();
    assert_eq!(alone, (vec![pet], vec![a]), "alone: one another");
}

/// Plague on a carrier ticks its effect about the carrier: on the
/// carrier's raid in its radius, not the carrier, cast by the carrier
/// (mangos's `Aura::TriggerSpell`, whose trigger of no unit target is its
/// bearer's, round 11's 11b; Thistle Tea's periodic raid triggers have it
/// Plague's caster's); gone, it ticks no more (mangos's
/// `PickTargetsForSpellTrigger`).
#[tokio::test]
async fn plague_ticks_about_its_carrier_on_the_carriers_raid() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6WL", "Plagued", Class::Mage).await;
    let (mut d, b, _) = human(&realm, "P6WM", "Nearby", Class::Mage).await;
    group(&realm, a, &[b], true);
    let out = map
        .query(move |m| {
            let near = off(m, a, 3.0, 0.0);
            put(m, b, near);
            for g in [a, b] {
                m.unit_fields_mut(g)
                    .unwrap()
                    .set_u32(UNIT_FIELD_MAXHEALTH, 5_000);
                set_health(m, g, 5_000);
            }
            let anubisath = unit_at(m, a, crate::game::tests_p6_wspell::BEAST, 10.0, 0.0);
            kit::apply(m, anubisath, a, PLAGUE);
            let before = (health(m, a), health(m, b));
            kit::tick_now(m, a, PLAGUE);
            let after = (health(m, a), health(m, b));
            crate::game::map::auras::remove_spell(m, a, PLAGUE, None);
            kit::pass(m, 6_000);
            let later = health(m, b);
            (before, after, anubisath, later)
        })
        .await
        .unwrap();
    let (before, after, _anubisath, later) = out;
    let attacker = d
        .wait_for("Plague's damage", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(l)
                if l.spell == PLAGUE_EFFECT && l.target == b =>
            {
                Some(l.attacker)
            }
            _ => None,
        })
        .await;
    assert_eq!(after.0, before.0, "the carrier spared");
    assert_eq!(after.1, before.1 - 100, "the raid member hit");
    assert_eq!(attacker, a, "by its carrier");
    assert_eq!(later, after.1, "gone, no more ticks");
}

/// A party's buff about its caster is its subgroup's and their pets'; the
/// class buff crosses the raid, to the class of the one aimed at; a pet's
/// party buff is its master's party's, itself among them (Thistle Tea's
/// party buffs; mangos's `FillRaidOrPartyTargets` and
/// `TARGET_UNIT_RAID_AND_CLASS`).
#[tokio::test]
async fn a_party_buff_is_the_subgroups_with_pets_and_a_class_buff_the_raids() {
    let realm = fast_realm(data);
    let (mut c, leader, map) = human(&realm, "P6WN", "Leader", Class::Mage).await;
    let (_d, moved, _) = human(&realm, "P6WO", "Moved", Class::Mage).await;
    let (_e, other, _) = human(&realm, "P6WP", "Other", Class::Warrior).await;
    let (_f, stranger, _) = human(&realm, "P6WQ", "Stranger", Class::Mage).await;
    group(&realm, leader, &[moved, other], true);
    subgroup(&realm, leader, "Moved", 1);
    let pet = map
        .query(move |m| {
            kit::ready(m, leader, 60, &[PARTY_BUFF, CLASS_BUFF]);
            let pet = pet_for(m, moved, leader, 2.0);
            assert!(cast_fresh(m, leader, PARTY_BUFF, Targets::default()));
            assert!(cast_fresh(m, leader, CLASS_BUFF, at(moved)));
            pet
        })
        .await
        .unwrap();
    assert_eq!(
        sorted(go(&mut c, PARTY_BUFF).await),
        sorted(vec![leader, other]),
        "the subgroup"
    );
    assert_eq!(
        sorted(go(&mut c, CLASS_BUFF).await),
        sorted(vec![leader, moved]),
        "the raid's mages, not the warrior's-class pet"
    );
    subgroup(&realm, leader, "Moved", 0);
    let out = map
        .query(move |m| {
            let together = landing(m, leader, PARTY_BUFF, Targets::default());
            let my_pet = pet_for(m, stranger, leader, 2.5);
            let theirs = landing(m, stranger, PARTY_BUFF, Targets::default());
            let pets = landing(m, my_pet, PARTY_BUFF, Targets::default());
            crate::game::map::combat::kill(m, leader, my_pet);
            let dead = landing(m, stranger, PARTY_BUFF, Targets::default());
            (together, my_pet, theirs, pets, dead)
        })
        .await
        .unwrap();
    let (together, my_pet, theirs, pets, dead) = out;
    assert_eq!(
        together,
        sorted(vec![leader, moved, other, pet]),
        "one subgroup, the pet with its master"
    );
    assert_eq!(
        theirs,
        sorted(vec![stranger, my_pet]),
        "alone: with the pet"
    );
    assert_eq!(
        pets,
        sorted(vec![stranger, my_pet]),
        "the pet's: its master's"
    );
    assert_eq!(dead, [stranger], "not a dead pet");
}

/// A party buff aimed at someone is the party of that one (a pet's, its
/// master's) about them, with their pets; one of another subgroup's, theirs
/// alone; an ungrouped friend's, with their pet. A rank too high passes
/// over the low but not the one aimed at (Thistle Tea's targeted party
/// buffs; mangos's `TARGET_UNIT_FRIEND_AND_PARTY`).
#[tokio::test]
async fn a_party_buff_at_someone_is_their_subgroups_about_them_with_pets() {
    let realm = fast_realm(data);
    let (_c, caster, map) = human(&realm, "P6WR", "Caster", Class::Mage).await;
    let (_d, selected, _) = human(&realm, "P6WS", "Selected", Class::Mage).await;
    let (_e, nearby, _) = human(&realm, "P6WT", "Nearby", Class::Mage).await;
    let (_f, other, _) = human(&realm, "P6WU", "Othergroup", Class::Mage).await;
    let (_g, friend, _) = human(&realm, "P6WV", "Friend", Class::Mage).await;
    group(&realm, caster, &[selected, nearby, other], true);
    subgroup(&realm, caster, "Selected", 1);
    subgroup(&realm, caster, "Nearby", 1);
    let out = map
        .query(move |m| {
            kit::ready(m, caster, 60, &[TARGET_PARTY, HIGH_RANK]);
            for (g, dx) in [(selected, 30.0), (nearby, 35.0), (other, 32.0)] {
                let to = off(m, caster, dx, 0.0);
                put(m, g, to);
            }
            let to = off(m, caster, 0.0, -60.0);
            put(m, friend, to);
            let pet = pet_for(m, selected, caster, 34.0);
            let low_pet = pet_for(m, nearby, caster, 36.0);
            let friends_pet = pet_for(m, friend, friend, 2.0);
            let mut out = vec![
                landing(m, caster, TARGET_PARTY, at(selected)),
                landing(m, caster, TARGET_PARTY, at(pet)),
                landing(m, caster, TARGET_PARTY, at(other)),
                landing(m, caster, TARGET_PARTY, at(friend)),
            ];
            let data = m.realm.data.clone();
            for (g, level) in [(selected, 50u8), (nearby, 37), (other, 50)] {
                crate::game::map::stats::set_level(m.players.get_mut(&g).unwrap(), &data, level);
            }
            for (g, level) in [(pet, 40u32), (low_pet, 37)] {
                m.unit_fields_mut(g)
                    .unwrap()
                    .set_u32(UNIT_FIELD_LEVEL, level);
            }
            kit::fresh(m, caster);
            assert!(cast_fresh(m, caster, HIGH_RANK, at(selected)));
            let ranked = [selected, nearby, pet, low_pet]
                .into_iter()
                .filter(|g| kit::has(m, *g, HIGH_RANK))
                .collect::<Vec<_>>();
            out.push(sorted(ranked));
            (pet, low_pet, friends_pet, out)
        })
        .await
        .unwrap();
    let (pet, low_pet, friends_pet, out) = out;
    let party = sorted(vec![selected, nearby, pet, low_pet]);
    assert_eq!(out[0], party, "the selected one's subgroup and pets");
    assert_eq!(out[1], party, "aimed at a pet, its master's");
    assert_eq!(out[2], [other], "another subgroup's, about them");
    assert_eq!(
        out[3],
        sorted(vec![friend, friends_pet]),
        "a friend alone, with their pet"
    );
    assert_eq!(
        out[4],
        sorted(vec![selected, pet]),
        "a high rank passes over the low"
    );
}

/// A party member's spell lands on one of the caster's master's group or
/// their pets, or its own pet, a pet's on its master; never on the caster
/// itself, a stranger, or the dead (Thistle Tea's `party_unit_allowed?`;
/// mangos's `TARGET_UNIT_PARTY`).
#[tokio::test]
async fn a_party_members_spell_is_for_the_group_and_pets_never_the_caster() {
    let realm = fast_realm(data);
    let (mut c, owner, map) = human(&realm, "P6WW", "Owner", Class::Mage).await;
    let (_d, member, _) = human(&realm, "P6WX", "Member", Class::Mage).await;
    let (_e, stranger, _) = human(&realm, "P6WY", "Stranger", Class::Mage).await;
    group(&realm, owner, &[member], false);
    let out = map
        .query(move |m| {
            kit::ready(m, owner, 60, &[PARTY_ONLY]);
            let pet = pet_for(m, owner, owner, 1.0);
            let member_pet = pet_for(m, member, owner, 1.5);
            let cast = |m: &mut Map, caster: Guid, target: Guid| {
                cast_fresh(m, caster, PARTY_ONLY, at(target))
            };
            let mut out = vec![
                cast(m, owner, pet),
                cast(m, owner, member),
                cast(m, owner, member_pet),
                cast(m, owner, owner),
                cast(m, owner, stranger),
                cast(m, pet, owner),
                cast(m, pet, pet),
                cast(m, pet, stranger),
            ];
            crate::game::map::death::kill(m, member);
            out.push(cast(m, owner, member));
            (pet, member_pet, out)
        })
        .await
        .unwrap();
    let (pet, member_pet, out) = out;
    assert_eq!(
        out,
        [true, true, true, false, false, true, false, false, false],
        "own pet, member, member's pet; not self or a stranger; the pet's at its master only; not the dead"
    );
    for expected in [pet, member, member_pet] {
        assert_eq!(go(&mut c, PARTY_ONLY).await, [expected]);
    }
}

/// A pet's spell at its master lands on the master, and its effect at
/// itself on itself, whatever is aimed at; a player's, on itself alone. A
/// spell at an enemy with its master too lands on both; one at an enemy
/// and at its caster, on both (Thistle Tea's mixed recipients; mangos's
/// `SetTargetMap` of each effect).
#[tokio::test]
async fn mixed_effects_land_on_each_ones_own() {
    let realm = fast_realm(data);
    let (_c, owner, map) = human(&realm, "P6WZ", "Master", Class::Mage).await;
    let (_d, stranger, _) = human(&realm, "P6XA", "Passerby", Class::Mage).await;
    let out = map
        .query(move |m| {
            let pet = pet_for(m, owner, owner, 1.0);
            let enemy = unit_at(m, owner, crate::game::tests_p6_wspell::BEAST, 3.0, 0.0);
            (
                pet,
                enemy,
                [
                    landing(m, pet, MASTER_AND_SELF, Targets::default()),
                    landing(m, pet, MASTER_AND_SELF, at(stranger)),
                    landing(m, owner, MASTER_AND_SELF, Targets::default()),
                    landing(m, pet, ENEMY_AND_MASTER, at(enemy)),
                    landing(m, owner, ENEMY_AND_SELF, at(enemy)),
                ],
            )
        })
        .await
        .unwrap();
    let (pet, enemy, out) = out;
    assert_eq!(out[0], sorted(vec![owner, pet]), "the master and itself");
    assert_eq!(out[1], sorted(vec![owner, pet]), "whatever is aimed at");
    assert_eq!(out[2], [owner], "a player's: itself");
    assert_eq!(
        out[3],
        sorted(vec![owner, enemy]),
        "the enemy and the master"
    );
    assert_eq!(
        out[4],
        sorted(vec![owner, enemy]),
        "the enemy and the caster"
    );
}
