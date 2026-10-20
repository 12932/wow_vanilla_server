//! Round 6's combat tests, what each hand's weapon brings (Thistle Tea's
//! `weapon_bonuses_test.exs` and `combat_skills_test.exs`): each hand's
//! hit, skill and crit, the auras held to a weapon, stacked damage, and
//! what trains.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::{ItemSpell, ItemTemplate};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::map::{Map, derive, skills};
use crate::game::tests_p3_combat::{self as base, *};

const DAGGER: u32 = 63_620;
const BOW: u32 = 63_621;
const POLE: u32 = 63_622;
/// A dagger with an equip spell of two crit for melee weapons.
const KEEN_DAGGER: u32 = 63_623;
const KEEN: u32 = 91_200;
const SLOT_RANGED: u8 = 17;

const SWORDS: u32 = 43;
const DAGGERS: u32 = 173;
const BOWS: u32 = 45;
const UNARMED: u32 = 162;

fn item(entry: u32, subclass: u32, inventory_type: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class: 2,
        subclass,
        name: format!("Weapon {entry}"),
        display_id: 1000,
        inventory_type,
        delay: 2000,
        stackable: 1,
        max_durability: 100,
        damages: [(100.0, 100.0, 0); 5],
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = base::data();
    let mut keen = item(KEEN_DAGGER, 15, 13);
    keen.spells[0] = ItemSpell {
        id: KEEN,
        trigger: 1,
        ..Default::default()
    };
    for t in [
        item(DAGGER, 15, 13),
        item(BOW, 2, 15),
        item(POLE, 20, 17),
        keen,
    ] {
        d.items.insert(t.entry, t);
    }
    // Critical Weapon +2's restriction: the melee weapons.
    let mut s = SpellTemplate {
        id: KEEN,
        name: "Keen".into(),
        equipped_item_class: 2,
        equipped_item_subclass_mask: 42_483,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MOD_CRIT_PERCENT,
        base_points: 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(KEEN, s);
    d
}

/// An aura of `kind` holding `amount`, for the weapons of `subclass` if
/// one is given; `stacks` up to.
fn weapon_aura(
    m: &mut Map,
    g: Guid,
    id: u32,
    kind: u32,
    misc: i32,
    amount: i32,
    subclass: Option<u32>,
) {
    let mut s = SpellTemplate {
        id,
        name: format!("Aura {id}"),
        equipped_item_class: subclass.map_or(-1, |_| 2),
        equipped_item_subclass_mask: subclass.map_or(0, |c| 1 << c),
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        misc_value: misc,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let _ = auras::apply_spell(m, g, g, &s, &[(0, amount)], auras::Context::default());
    derive::unit(m, g);
}

fn set_stacks(m: &mut Map, g: Guid, spell: u32, stacks: u32) {
    if let Some(h) =
        auras::auras_mut(m, g).and_then(|a| a.holders.iter_mut().find(|h| h.spell == spell))
    {
        h.stacks = stacks;
    }
    derive::unit(m, g);
}

fn drop_aura(m: &mut Map, g: Guid, spell: u32) {
    auras::remove_spell(m, g, spell, None);
    derive::unit(m, g);
}

/// A warrior of 60 who dual wields a sword and a dagger, with a bow.
fn armed(m: &mut Map, g: Guid) {
    level(m, g, 60);
    m.players
        .get_mut(&g)
        .unwrap()
        .spells
        .push(derive::SPELL_DUAL_WIELD);
    equip(m, g, SWORD, SLOT_MAIN_HAND);
    equip(m, g, DAGGER, SLOT_OFF_HAND);
    equip(m, g, BOW, SLOT_RANGED);
    let p = m.players.get_mut(&g).unwrap();
    for s in [SWORDS, DAGGERS, BOWS, UNARMED] {
        skills::learn_skill(p, s, 1);
        skills::set_value(p, s, 300);
    }
    derive::unit(m, g);
}

fn roll(m: &Map, g: Guid, victim: Guid, hand: Hand) -> combat::Roll {
    combat::roll_for(m, g, victim, hand).expect("a roll")
}

fn skill_value(m: &Map, g: Guid, skill: u32) -> Option<u32> {
    skills::value(&m.players[&g], skill)
}

fn crit_field(m: &Map, g: Guid) -> (f32, f32) {
    let f = &m.players[&g].fields;
    (
        f.get_f32(PLAYER_CRIT_PERCENTAGE),
        f.get_f32(PLAYER_RANGED_CRIT_PERCENTAGE),
    )
}

/// Thistle Tea's "hit follows the attacking hand in white attacks and spell
/// snapshots", "offhand scope source does not substitute for the attacking
/// weapon requirements", "hit talents retain equipped identity through
/// disarm, breakage, and forms" and "spell snapshots use canonical crit
/// inputs even if client fields are stale", as mangos's
/// `UpdateWeaponHitChances` has them: each hand's hit is the auras for no
/// weapon and those its own weapon fits, the weapon in the hand whether
/// disarmed, broken or in a form; no weapon, none of those. The table
/// reads the chances worked out, not the fields the client is shown.
#[tokio::test]
async fn each_hand_hits_by_the_auras_its_own_weapon_fits() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            armed(m, ga);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            weapon_aura(m, ga, 91_210, AURA_MOD_HIT_CHANCE, 0, 5, Some(15));
            weapon_aura(m, ga, 91_211, AURA_MOD_HIT_CHANCE, 0, 3, Some(2));
            weapon_aura(m, ga, 91_212, AURA_MOD_HIT_CHANCE, 0, 1, None);
            let hands = [Hand::Main, Hand::Off, Hand::Ranged].map(|h| roll(m, ga, brute, h).hit);
            drop_aura(m, ga, 91_210);
            drop_aura(m, ga, 91_211);
            drop_aura(m, ga, 91_212);
            // Five for swords: disarmed, broken, in a stance, and gone.
            weapon_aura(m, ga, 91_213, AURA_MOD_HIT_CHANCE, 0, 5, Some(7));
            weapon_aura(m, ga, 91_214, AURA_MOD_DISARM, 0, 0, None);
            let disarmed = roll(m, ga, brute, Hand::Main).hit;
            drop_aura(m, ga, 91_214);
            let sword = m.players[&ga]
                .items
                .iter()
                .position(|i| i.entry == SWORD)
                .unwrap();
            let p = m.players.get_mut(&ga).unwrap();
            p.items[sword].fields.set_u32(ITEM_FIELD_MAXDURABILITY, 100);
            p.items[sword].fields.set_u32(ITEM_FIELD_DURABILITY, 0);
            derive::unit(m, ga);
            let broken = roll(m, ga, brute, Hand::Main).hit;
            let p = m.players.get_mut(&ga).unwrap();
            p.items[sword].fields.set_u32(ITEM_FIELD_DURABILITY, 100);
            p.fields.set_byte(UNIT_FIELD_BYTES_1, 2, 17);
            derive::unit(m, ga);
            let stance = roll(m, ga, brute, Hand::Main).hit;
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_byte(UNIT_FIELD_BYTES_1, 2, 0);
            m.players.get_mut(&ga).unwrap().items.remove(sword);
            derive::unit(m, ga);
            let unarmed = roll(m, ga, brute, Hand::Main).hit;
            // The crit the table takes isn't the client's field.
            weapon_aura(m, ga, 91_215, AURA_MOD_CRIT_PERCENT, 0, 10, None);
            let crit = roll(m, ga, brute, Hand::Main).crit;
            let f = &mut m.players.get_mut(&ga).unwrap().fields;
            f.set_f32(PLAYER_CRIT_PERCENTAGE, 99.0);
            f.set_f32(PLAYER_RANGED_CRIT_PERCENTAGE, 99.0);
            let stale = (
                roll(m, ga, brute, Hand::Main).crit,
                roll(m, ga, brute, Hand::Ranged).crit,
            );
            (hands, [disarmed, broken, stance, unarmed], crit, stale)
        })
        .await
        .unwrap();
    let (hands, sword, crit, stale) = r;
    assert_eq!(
        hands,
        [1.0, 6.0, 4.0],
        "the sword's, the dagger's, the bow's"
    );
    assert_eq!(
        sword,
        [5.0, 5.0, 5.0, 0.0],
        "disarmed, broken, in a stance; gone"
    );
    assert!(
        crit < 99.0 && stale.0 == crit && stale.1 < 99.0,
        "{crit} {stale:?}"
    );
}

/// Thistle Tea's "combines both aura bonuses with the skill for each hand",
/// "zero effective skill is retained by the attack table", "weapon skill
/// and skill bonuses update displayed and outgoing crit" and "missing ranged
/// weapons and low skills clamp displayed crit to zero", as mangos's
/// `GetWeaponSkillValue` and `UpdateCritPercentage` have them: each hand
/// swings with its own weapon's skill and both kinds of skill bonus, a
/// negative one too, to no less than 0, which the table takes as it is; a
/// skill short of its cap takes 0.04 a point off the crit shown and
/// rolled; no ranged weapon is no ranged skill, and the crit shown no
/// less than 0.
#[tokio::test]
async fn each_hand_swings_with_its_own_weapons_skill_and_its_bonuses() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            armed(m, ga);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            let p = m.players.get_mut(&ga).unwrap();
            for s in [SWORDS, DAGGERS, BOWS] {
                skills::set_value(p, s, 1);
            }
            weapon_aura(m, ga, 91_220, AURA_MOD_SKILL, SWORDS as i32, 3, None);
            weapon_aura(m, ga, 91_221, AURA_MOD_SKILL_TALENT, SWORDS as i32, 5, None);
            let skills =
                [Hand::Main, Hand::Off, Hand::Ranged].map(|h| roll(m, ga, brute, h).attacker_skill);
            drop_aura(m, ga, 91_220);
            drop_aura(m, ga, 91_221);
            weapon_aura(m, ga, 91_222, AURA_MOD_SKILL, SWORDS as i32, -10, None);
            let zero = roll(m, ga, brute, Hand::Main).attacker_skill;
            drop_aura(m, ga, 91_222);
            // Crit by skill, over ten of crit that keeps it off 0.
            weapon_aura(m, ga, 91_223, AURA_MOD_CRIT_PERCENT, 0, 10, None);
            let p = m.players.get_mut(&ga).unwrap();
            skills::set_value(p, SWORDS, 300);
            skills::set_value(p, BOWS, 300);
            derive::unit(m, ga);
            let full = crit_field(m, ga);
            let p = m.players.get_mut(&ga).unwrap();
            skills::set_value(p, SWORDS, 250);
            skills::set_value(p, BOWS, 275);
            weapon_aura(m, ga, 91_220, AURA_MOD_SKILL, SWORDS as i32, 3, None);
            weapon_aura(m, ga, 91_221, AURA_MOD_SKILL_TALENT, SWORDS as i32, 2, None);
            let short = crit_field(m, ga);
            let rolled = roll(m, ga, brute, Hand::Main).crit;
            let p = m.players.get_mut(&ga).unwrap();
            skills::set_value(p, SWORDS, 251);
            derive::unit(m, ga);
            let trained = crit_field(m, ga).0;
            // No bow, and a sword skill of 1 without the ten.
            drop_aura(m, ga, 91_223);
            let p = m.players.get_mut(&ga).unwrap();
            p.items.retain(|i| i.entry != BOW);
            skills::set_value(p, SWORDS, 1);
            derive::unit(m, ga);
            let bare = (
                crit_field(m, ga),
                roll(m, ga, brute, Hand::Ranged).attacker_skill,
            );
            (skills, zero, (full, short, rolled, trained), bare)
        })
        .await
        .unwrap();
    let (skills, zero, (full, short, rolled, trained), bare) = r;
    assert_eq!(
        skills,
        [9, 1, 1],
        "the sword's with its bonuses, the dagger's, the bow's"
    );
    assert_eq!(zero, 0, "no less than 0, and taken so");
    let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
    assert!(close(full.0 - short.0, 1.8), "45 short: {full:?} {short:?}");
    assert!(close(full.1 - short.1, 1.0), "25 short");
    assert!(close(rolled, short.0), "the crit rolled is the one shown");
    assert!(close(trained - short.0, 0.04), "a point more");
    assert_eq!(bare, ((0.0, 0.0), 0), "no less than 0, and no bow no skill");
}

/// Thistle Tea's "uses maximum player defense against players and current
/// defense against creatures" and "includes both defense bonuses with
/// either skill basis", as mangos's `GetWeaponSkillValue` and
/// `GetDefenseSkillValue` have them: against a player, a player's weapon
/// skill and defense are at least their caps, bonuses on top; against a
/// creature, as they are.
#[tokio::test]
async fn against_a_player_a_players_skills_are_at_least_their_caps() {
    let (realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 2.0, START.1, START.2).await;
    let r = map
        .query(move |m| {
            armed(m, ga);
            armed(m, gb);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            let p = m.players.get_mut(&ga).unwrap();
            skills::set_value(p, SWORDS, 200);
            let p = m.players.get_mut(&gb).unwrap();
            skills::learn_skill(p, derive::SKILL_DEFENSE, 1);
            skills::set_value(p, derive::SKILL_DEFENSE, 100);
            weapon_aura(m, ga, 91_230, AURA_MOD_SKILL, SWORDS as i32, 3, None);
            weapon_aura(
                m,
                gb,
                91_231,
                AURA_MOD_SKILL,
                derive::SKILL_DEFENSE as i32,
                2,
                None,
            );
            weapon_aura(
                m,
                gb,
                91_232,
                AURA_MOD_SKILL_TALENT,
                derive::SKILL_DEFENSE as i32,
                4,
                None,
            );
            let pvp = roll(m, ga, gb, Hand::Main);
            let pve = roll(m, ga, brute, Hand::Main);
            let struck = roll(m, brute, gb, Hand::Main);
            (
                (pvp.attacker_skill, pvp.victim_defense),
                pve.attacker_skill,
                struck.victim_defense,
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, (303, 306), "the caps, with the bonuses");
    assert_eq!(r.1, 203, "against a creature, as it is");
    assert_eq!(r.2, 106, "a creature's blow meets the defense as it is");
}

/// Thistle Tea's "crit enchants apply once from their own weapon", as
/// mangos's `_ApplyWeaponDependentAuraCritMod` has it: a crit aura an
/// off-hand dagger gives, for melee weapons, counts for the melee hands
/// (the off hand's crit is the main hand's, `GetCritChance`) and not the
/// bow; broken, its aura goes, and so does its crit.
#[tokio::test]
async fn a_weapons_own_crit_aura_counts_for_the_hands_it_fits_until_it_breaks() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            armed(m, ga);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            weapon_aura(m, ga, 91_240, AURA_MOD_CRIT_PERCENT, 0, 10, None);
            let crits =
                |m: &Map| [Hand::Main, Hand::Off, Hand::Ranged].map(|h| roll(m, ga, brute, h).crit);
            let plain = crits(m);
            equip(m, ga, KEEN_DAGGER, SLOT_OFF_HAND);
            crate::game::map::inventory::sync_gear(m, ga);
            derive::unit(m, ga);
            let keen = crits(m);
            let p = m.players.get_mut(&ga).unwrap();
            let i = p.items.iter().position(|i| i.entry == KEEN_DAGGER).unwrap();
            p.items[i].fields.set_u32(ITEM_FIELD_MAXDURABILITY, 100);
            p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, 0);
            crate::game::map::inventory::sync_gear(m, ga);
            derive::unit(m, ga);
            (plain, keen, crits(m))
        })
        .await
        .unwrap();
    let (plain, keen, broken) = r;
    let gained: Vec<f32> = keen.iter().zip(plain).map(|(k, p)| k - p).collect();
    assert_eq!(gained, [2.0, 2.0, 0.0], "the melee hands, not the bow");
    assert_eq!(broken, plain, "broken, gone");
}

/// Thistle Tea's "stacked damage follows application, cap, dispel, and
/// expiry" and "stacked reductions clamp to zero and independent bonuses
/// multiply": a percentage of damage done counts by its stacks, to its
/// cap, for the weapons and the spells alike; a stack dispelled is a stack
/// less, the aura gone none; percentages multiply each other, each no
/// less than nothing, and one for swords leaves the dagger.
#[tokio::test]
async fn damage_done_counts_by_its_stacks_and_percentages_multiply() {
    const STACKING: u32 = 91_250;
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            armed(m, ga);
            let damage = |m: &Map| {
                let f = &m.players[&ga].fields;
                (
                    f.get_f32(UNIT_FIELD_MINDAMAGE),
                    f.get_f32(UNIT_FIELD_MINOFFHANDDAMAGE),
                    f.get_f32(PLAYER_FIELD_MOD_DAMAGE_DONE_PCT + 2),
                )
            };
            let (main, off, _) = damage(m);
            let ratio = |m: &Map| {
                let (a, b, c) = damage(m);
                (
                    (a / main * 100.0).round() / 100.0,
                    (b / off * 100.0).round() / 100.0,
                    (c * 100.0).round() / 100.0,
                )
            };
            let mut s = SpellTemplate {
                id: STACKING,
                name: "Stacking".into(),
                equipped_item_class: -1,
                stack_amount: 3,
                dispel: 1,
                ..Default::default()
            };
            s.effects[0] = SpellEffect {
                effect: 6,
                aura: AURA_MOD_DAMAGE_PERCENT_DONE,
                misc_value: 127,
                base_points: 19,
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            };
            let mut stacked = Vec::new();
            for _ in 0..4 {
                let _ = auras::apply_spell(m, ga, ga, &s, &[(0, 20)], auras::Context::default());
                derive::unit(m, ga);
                stacked.push(ratio(m));
            }
            auras::remove_stack(m, ga, STACKING);
            derive::unit(m, ga);
            let dispelled = ratio(m);
            drop_aura(m, ga, STACKING);
            let gone = ratio(m);
            // Twenty less for swords, three times; a quarter more, twice.
            weapon_aura(m, ga, 91_251, AURA_MOD_DAMAGE_PERCENT_DONE, 1, -20, Some(7));
            set_stacks(m, ga, 91_251, 3);
            weapon_aura(m, ga, 91_252, AURA_MOD_DAMAGE_PERCENT_DONE, 1, 25, None);
            set_stacks(m, ga, 91_252, 2);
            let multiplied = ratio(m);
            set_stacks(m, ga, 91_251, 10);
            let floored = ratio(m);
            (stacked, dispelled, gone, multiplied, floored)
        })
        .await
        .unwrap();
    let (stacked, dispelled, gone, multiplied, floored) = r;
    assert_eq!(
        stacked,
        [
            (1.2, 1.2, 1.2),
            (1.4, 1.4, 1.4),
            (1.6, 1.6, 1.6),
            (1.6, 1.6, 1.6)
        ],
        "a fifth a stack, to three"
    );
    assert_eq!(dispelled, (1.4, 1.4, 1.4), "a stack dispelled");
    assert_eq!(gone, (1.0, 1.0, 1.0));
    assert_eq!(
        (multiplied.0, multiplied.1),
        (0.6, 1.5),
        "0.4 x 1.5; the dagger 1.5"
    );
    assert_eq!((floored.0, floored.1), (0.0, 1.5), "no less than nothing");
}

/// Thistle Tea's "loaded Growth applies each stack to physical attacks
/// only": Growth (24086) is a quarter more physical damage a stack, to
/// ten, for both hands and not a spell's.
#[tokio::test]
#[ignore = "extended"]
async fn growth_gives_each_stack_to_physical_blows_only() {
    const GROWTH: u32 = 24_086;
    let mut data = crate::game::harness::load_real(&crate::game::harness::real_data("db"));
    for t in [item(DAGGER, 15, 13), item(BOW, 2, 15)] {
        data.items.insert(t.entry, t);
    }
    data.items.insert(SWORD, item(SWORD, 7, 13));
    let s = data.spells.get(GROWTH).cloned().expect("Growth");
    assert_eq!(s.stack_amount, 10);
    let realm = crate::game::harness::realm_with(data);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let r = map
        .query(move |m| {
            armed(m, ga);
            let damage = |m: &Map| {
                let f = &m.players[&ga].fields;
                (
                    f.get_f32(UNIT_FIELD_MINDAMAGE),
                    f.get_f32(UNIT_FIELD_MINOFFHANDDAMAGE),
                    f.get_f32(PLAYER_FIELD_MOD_DAMAGE_DONE_PCT + 2),
                )
            };
            let before = damage(m);
            for _ in 0..3 {
                auras::apply(m, ga, ga, &s, 0, 25);
            }
            derive::unit(m, ga);
            let after = damage(m);
            (
                (after.0 / before.0 * 100.0).round() / 100.0,
                (after.1 / before.1 * 100.0).round() / 100.0,
                after.2,
            )
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (1.75, 1.75, 1.0),
        "three quarters more, both hands; no spell's"
    );
}

/// Thistle Tea's "fishing poles and empty offhand or ranged slots cannot
/// train" and "stances, stealth, shadowform and moonkin can train weapons",
/// as mangos's `UpdateWeaponSkill` and `IsNoWeaponShapeShift` have them: a
/// blow with a fishing pole trains nothing, an empty off hand nothing; a
/// sword trains in a stance, in Stealth, in Shadowform, in Moonkin Form and
/// in Ghost Wolf (its form doesn't put the weapon away), not in cat form.
#[tokio::test]
async fn a_fishing_pole_trains_nothing_and_only_a_form_without_weapons_stops_training() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            armed(m, ga);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            let p = m.players.get_mut(&ga).unwrap();
            for s in [SWORDS, DAGGERS, UNARMED] {
                skills::set_value(p, s, 1);
            }
            p.items.retain(|i| i.entry != DAGGER);
            let swing = |m: &mut Map, form: u8| {
                m.players
                    .get_mut(&ga)
                    .unwrap()
                    .fields
                    .set_byte(UNIT_FIELD_BYTES_1, 2, form);
                let before = skill_value(m, ga, SWORDS);
                forced_swing(m, ga, brute, Outcome::Hit);
                skill_value(m, ga, SWORDS).zip(before).map(|(a, b)| a - b)
            };
            let forms: Vec<Option<u32>> = [0, 17, 18, 19, 30, 28, 31, 16, 1]
                .map(|f| swing(m, f))
                .to_vec();
            swing(m, 0);
            equip(m, ga, POLE, SLOT_MAIN_HAND);
            derive::unit(m, ga);
            let before = [SWORDS, DAGGERS, UNARMED].map(|s| skill_value(m, ga, s));
            forced_swing(m, ga, brute, Outcome::Hit);
            let pole = [SWORDS, DAGGERS, UNARMED].map(|s| skill_value(m, ga, s));
            (forms, before == pole)
        })
        .await
        .unwrap();
    let (forms, untrained) = r;
    assert_eq!(
        forms,
        [
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(1),
            Some(0)
        ],
        "every form but cat trains the sword"
    );
    assert!(untrained, "a fishing pole trains nothing");
}

/// Found by the Growth test on the real world, where a warrior is always in
/// a stance: a stance, Stealth or any form that keeps the weapons in hand
/// dual wields and parries, as mangos's `CanUseEquippedWeapon` and
/// `CanParryInCombat` have it (wvs did both only out of every form); cat
/// form, which puts them away, neither.
#[tokio::test]
async fn in_a_stance_or_stealth_a_player_dual_wields_and_parries() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            armed(m, ga);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_PARRY);
            [0u8, 17, 18, 19, 30, 1].map(|form| {
                m.players
                    .get_mut(&ga)
                    .unwrap()
                    .fields
                    .set_byte(UNIT_FIELD_BYTES_1, 2, form);
                derive::unit(m, ga);
                let p = &m.players[&ga];
                (
                    p.derived.dual_wield,
                    p.derived.parry > 0.0,
                    p.fields.get_f32(UNIT_FIELD_MINOFFHANDDAMAGE) > 0.0,
                )
            })
        })
        .await
        .unwrap();
    let both = (true, true, true);
    assert_eq!(
        r,
        [both, both, both, both, both, (false, false, false)],
        "out of form, in each stance and Stealth; not in cat form"
    );
}

/// Thistle Tea's "the killing blow can train but late attacks on the
/// corpse cannot" and "typed feedback reaches the registered player owner
/// and publishes progress": a blow that kills trains the weapon, and the
/// client is sent the skill's new value; a swing at the corpse trains
/// nothing.
#[tokio::test]
async fn the_killing_blow_trains_and_the_client_sees_it_and_the_corpse_trains_nothing() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let slot = map
        .query(move |m| {
            armed(m, ga);
            let p = m.players.get_mut(&ga).unwrap();
            skills::set_value(p, SWORDS, 1);
            skills::known(p).iter().position(|s| *s == SWORDS)
        })
        .await
        .unwrap()
        .expect("swords") as u16;
    a.clear();
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            if let Some(c) = m.creature_mut(brute) {
                c.fields.set_u32(UNIT_FIELD_HEALTH, 1);
            }
            forced_swing(m, ga, brute, Outcome::Hit);
            let killed = (
                m.unit_view(brute).is_some_and(|u| u.alive),
                skill_value(m, ga, SWORDS),
            );
            forced_swing(m, ga, brute, Outcome::Hit);
            (killed, skill_value(m, ga, SWORDS))
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        ((false, Some(2)), Some(2)),
        "trained by the kill, not by the corpse"
    );
    // The new value, whenever it comes: an update between the two queries
    // sends the 1 set in the first, after the clear.
    let field = PLAYER_SKILL_INFO_1_1 + slot * 3 + 1;
    let seen = a
        .wait_for("the skill's new value", |m| {
            crate::game::harness::values(m)
                .into_iter()
                .filter(|(g, _)| *g == ga)
                .find_map(|(_, mask)| crate::game::tests_bugs6::fields(&mask).get(&field).copied())
                .filter(|v| v & 0xFFFF == 2)
        })
        .await;
    assert_eq!(seen & 0xFFFF, 2);
}

/// Thistle Tea's "loaded dagger and ranged talents retain their equipment
/// requirements": Dagger Specialization (13807) is five crit with a dagger
/// in the main hand, not a sword; Lethal Shots (19431) five ranged crit
/// with a bow; Precision (13845) five hit for the melee hands, not the bow.
#[tokio::test]
#[ignore = "extended"]
async fn the_weapon_talents_count_for_the_weapons_they_name() {
    let mut data = crate::game::harness::load_real(&crate::game::harness::real_data("db"));
    for t in [item(DAGGER, 15, 13), item(BOW, 2, 15), item(SWORD, 7, 13)] {
        data.items.insert(t.entry, t);
    }
    let talents: Vec<SpellTemplate> = [13_807, 19_431, 13_845]
        .map(|id| data.spells.get(id).cloned().expect("the talent"))
        .to_vec();
    let realm = crate::game::harness::realm_with(data);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let r = map
        .query(move |m| {
            armed(m, ga);
            let brute = spawn(m, ga, 6, 60, 3.0);
            let read = |m: &Map| {
                let main = roll(m, ga, brute, Hand::Main);
                let off = roll(m, ga, brute, Hand::Off);
                let ranged = roll(m, ga, brute, Hand::Ranged);
                ([main.crit, ranged.crit], [main.hit, off.hit, ranged.hit])
            };
            let bare = read(m);
            equip(m, ga, DAGGER, SLOT_MAIN_HAND);
            derive::unit(m, ga);
            let bare_dagger = read(m);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            for s in &talents {
                auras::apply(m, ga, ga, s, 0, 5);
            }
            derive::unit(m, ga);
            let talented = read(m);
            equip(m, ga, DAGGER, SLOT_MAIN_HAND);
            derive::unit(m, ga);
            (bare, talented, bare_dagger, read(m))
        })
        .await
        .unwrap();
    let (bare, talented, bare_dagger, dagger) = r;
    let gained = |a: [f32; 2], b: [f32; 2]| [a[0] - b[0], a[1] - b[1]];
    assert_eq!(
        gained(talented.0, bare.0),
        [0.0, 5.0],
        "the sword none, the bow five"
    );
    assert_eq!(talented.1, [5.0, 5.0, 0.0], "Precision for the melee hands");
    assert_eq!(
        gained(dagger.0, bare_dagger.0)[0],
        5.0,
        "a dagger in the main hand"
    );
}
