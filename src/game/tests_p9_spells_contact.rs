//! Round 9, against Thistle Tea's tests of what a spell's contact does
//! where they are 1.12's (`spell_contact_test.exs` and
//! `spell_locations_test.exs` of the world, `combat_test.exs` and
//! `casting_test.exs` of the core): when a pet's spell, a tick, an immune
//! Sap, a failure that breaks stealth, a missile, a proc or a channel
//! puts a caster and its target in the fight, flags them or unhides the
//! caster; a focus wanted before a script's creature, and a script's
//! place; and Feed Pet's trigger on the pet.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, pass};
use super::tests_p6_wspell::{BEAST, DRAGONKIN, RANGE_30, face_east, off, put, sides, unit_at};
use crate::game::Realm;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{
    ATTR_EX_CHANNELED, ATTR_EX_FAILURE_BREAKS_STEALTH, ATTR_EX_NO_THREAT,
    ATTR_EX2_NO_INITIAL_THREAT, ATTR_EX3_PVP_ENABLING, SpellEffect, SpellTemplate,
};
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{
    self, AURA_MOD_DECREASE_SPEED, AURA_MOD_STEALTH, AURA_MOD_STUN, AURA_PERIODIC_DAMAGE,
    AURA_PERIODIC_ENERGIZE, AURA_SCHOOL_IMMUNITY,
};
use crate::game::map::player::{UNIT_FLAG_IN_COMBAT, UNIT_FLAG_PVP};
use crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP;
use crate::game::map::spells::{self, Targets, checks, fail, mods, stealth, targets};
use crate::game::map::{Map, MapHandle, inventory, pets};

/// A pet's bolt of 20 fire, and its burn of 20 fire a second.
const PET_BOLT: u32 = 997_320;
const PET_BURN: u32 = 997_321;
/// Harmful auras of no damage: a Sap (the rogues' family, flag 0x80); a
/// slow whose failure breaks stealth and which makes no threat; a slow,
/// as a spell and as a missile of 20 yards a second; a mark of no threat
/// that is PvP-enabling (Hunter's Mark's shape).
const SAP: u32 = 997_322;
const LOUD: u32 = 997_323;
const SLOW: u32 = 997_324;
const SLOW_MISSILE: u32 = 997_325;
const MARK: u32 = 997_326;
/// A proc's 20 fire.
const PROC_FIRE: u32 = 997_327;
/// An immunity to every school; a stealth a player of its level sees
/// through close up, and one no one does, both of stealth's dispel type.
const WARD: u32 = 997_328;
const SHADOWED: u32 = 997_329;
const HIDDEN: u32 = 997_330;
/// A channel of 20 a second that makes no threat as it lands.
const DRAIN: u32 = 997_331;
/// A dummy at a script's creature near the caster, wanting a focus and
/// not; a heal at any unit, then a script's place; damage at an enemy,
/// then that place; a persistent area at it.
const FOCUSED: u32 = 997_332;
const SCRIPTED: u32 = 997_333;
const ANY_UNIT_THERE: u32 = 997_334;
const HARM_THERE: u32 = 997_335;
const AREA_THERE: u32 = 997_336;
/// Feed Pet, the happiness over time it triggers, and a pet's food.
const FEED: u32 = 997_337;
const FED: u32 = 997_338;
const FOOD: u32 = 997_339;
/// A tameable creature of no type.
const PET_BEAST: u32 = 9_973;
/// The spell focus Thistle Tea's test wants (an anvil's).
const FOCUS: u32 = 1351;
const R10: u32 = 13;
/// Stealth's dispel type.
const DISPEL_STEALTH: u32 = 5;
/// A food of a pet's first kind (meat), of item level 60.
const MEAT: u32 = 1;

fn harmful(id: u32, kind: u32) -> SpellTemplate {
    let mut s = bare(id, 6);
    s.duration_index = LASTS_5S;
    effect(&mut s, 0, aura(kind, 0, -50, 6));
    s
}

fn lasting(id: u32, kind: u32, misc: i32, amount: i32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    effect(&mut s, 0, aura(kind, misc, amount, 1));
    s
}

fn fire(id: u32) -> SpellTemplate {
    let mut s = bare(id, 2);
    effect(&mut s, 0, damage(20));
    s
}

fn burn(id: u32) -> SpellTemplate {
    let mut s = bare(id, 2);
    s.duration_index = LASTS_5S;
    effect(
        &mut s,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 20, 6)
        },
    );
    s
}

fn data(d: &mut GameData) {
    d.creature_templates.insert(
        PET_BEAST,
        CreatureTemplate {
            entry: PET_BEAST,
            name: "Typeless Beast".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 60,
            level_max: 60,
            faction: 7,
            type_flags: 1,
            ..Default::default()
        },
    );
    d.items.insert(
        FOOD,
        ItemTemplate {
            entry: FOOD,
            name: "Made-up Meat".into(),
            stackable: 20,
            food_type: MEAT,
            item_level: 60,
            ..Default::default()
        },
    );
    d.spells.radii.insert(R10, 10.0);
    d.spells.ranges.insert(RANGE_30, (0.0, 30.0));
    let mut sap = harmful(SAP, AURA_MOD_STUN);
    sap.school = 0;
    sap.family_name = 8;
    sap.family_flags = 0x80;
    let mut loud = harmful(LOUD, AURA_MOD_DECREASE_SPEED);
    loud.attributes[1] |= ATTR_EX_FAILURE_BREAKS_STEALTH | ATTR_EX_NO_THREAT;
    let mut slow = harmful(SLOW, AURA_MOD_DECREASE_SPEED);
    slow.range_index = RANGE_30;
    let mut missile = slow.clone();
    missile.id = SLOW_MISSILE;
    missile.speed = 20.0;
    let mut mark = harmful(MARK, AURA_MOD_DECREASE_SPEED);
    mark.attributes[1] |= ATTR_EX_NO_THREAT;
    mark.attributes[3] |= ATTR_EX3_PVP_ENABLING;
    let stealth = |id: u32, amount: i32| {
        let mut s = lasting(id, AURA_MOD_STEALTH, 0, amount);
        s.dispel = DISPEL_STEALTH;
        s
    };
    let mut drain = burn(DRAIN);
    drain.attributes[1] |= ATTR_EX_CHANNELED;
    drain.attributes[2] |= ATTR_EX2_NO_INITIAL_THREAT;
    let scripted = |id: u32, focus: u32| {
        let mut s = bare(id, 0);
        s.range_index = RANGE_30;
        s.mana_cost = 10;
        s.requires_spell_focus = focus;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: spells::EFFECT_DUMMY,
                target_a: 38,
                ..Default::default()
            },
        );
        s
    };
    let there = |id: u32, e: SpellEffect| {
        let mut s = bare(id, 0);
        s.range_index = RANGE_30;
        s.duration_index = LASTS_5S;
        effect(&mut s, 0, e);
        s
    };
    let any_unit = there(
        ANY_UNIT_THERE,
        SpellEffect {
            effect: 10,
            base_points: 9,
            base_dice: 1,
            target_a: 25,
            target_b: 46,
            ..Default::default()
        },
    );
    let harm = there(
        HARM_THERE,
        SpellEffect {
            target_b: 46,
            ..damage(10)
        },
    );
    let area = there(
        AREA_THERE,
        SpellEffect {
            effect: 27,
            radius_index: R10,
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 10, 46)
        },
    );
    for spell in [FOCUSED, SCRIPTED, ANY_UNIT_THERE, HARM_THERE, AREA_THERE] {
        d.spells.script_targets.insert(spell, vec![(1, BEAST)]);
    }
    let mut feed = bare(FEED, 0);
    effect(
        &mut feed,
        0,
        SpellEffect {
            effect: pets::EFFECT_FEED_PET,
            trigger_spell: FED,
            ..Default::default()
        },
    );
    let mut fed = bare(FED, 0);
    fed.duration_index = LASTS_15S;
    effect(
        &mut fed,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_ENERGIZE, 4, 1, 5)
        },
    );
    insert(
        d,
        [
            fire(PET_BOLT),
            burn(PET_BURN),
            sap,
            loud,
            slow,
            missile,
            mark,
            fire(PROC_FIRE),
            lasting(WARD, AURA_SCHOOL_IMMUNITY, 127, 0),
            stealth(SHADOWED, 1),
            stealth(HIDDEN, 1_000),
            drain,
            scripted(FOCUSED, FOCUS),
            scripted(SCRIPTED, 0),
            any_unit,
            harm,
            area,
            feed,
            fed,
        ],
    );
}

/// [`data`], with the sides and the creature types of a script's units.
fn placed(d: &mut GameData) {
    data(d);
    sides(d);
}

/// A level-60 dwarf hunter facing east, knowing `learn`, with a pet of no
/// creature type two yards ahead.
async fn hunter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    learn: &'static [u32],
) -> (Client, Guid, MapHandle, Guid) {
    let mut c = Client::connect(realm, account);
    let h = c.enter(name, Race::Dwarf, Class::Hunter).await;
    let map = world(realm);
    let pet = map
        .query(move |m| {
            kit::ready(m, h, 60, learn);
            face_east(m, h);
            pets::give_pet(m, h, PET_BEAST, 60).expect("a pet");
            let pet = pets::pet_of(m, h).expect("the pet out");
            let to = off(m, h, 2.0, 0.0);
            put(m, pet, to);
            pet
        })
        .await
        .unwrap();
    (c, h, map, pet)
}

fn fighting(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
}

fn flagged(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP))
}

fn contested(m: &Map, g: Guid) -> bool {
    m.players[&g]
        .fields
        .has_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP)
}

/// Whether `creature` has `by` on its threat list: in the fight with them.
fn engaged(m: &Map, creature: Guid, by: Guid) -> bool {
    m.creature(creature)
        .is_some_and(|c| c.threat.iter().any(|(g, _)| *g == by))
}

/// `b` three yards east of `a`, facing each other.
fn face_off(m: &mut Map, a: Guid, b: Guid) {
    let to = off(m, a, 3.0, 0.0);
    put(m, b, to);
    m.players.get_mut(&a).unwrap().movement.orientation = 0.0;
    m.players.get_mut(&b).unwrap().movement.orientation = std::f32::consts::PI;
}

fn flag(m: &mut Map, g: Guid) {
    m.unit_fields_mut(g)
        .unwrap()
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
}

/// Everyone unflagged, out of the fight and uncontested.
fn reset(m: &mut Map, players: &[Guid]) {
    for &g in players {
        let p = m.players.get_mut(&g).unwrap();
        p.pvp = Default::default();
        p.combat = Default::default();
        p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, false);
        p.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
        p.fields
            .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP, false);
    }
}

/// Why `caster` can't cast `spell` at no one now, fresh.
fn why(m: &mut Map, caster: Guid, spell: u32) -> Option<u8> {
    kit::fresh(m, caster);
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    checks::check(m, caster, &s, &Targets::default(), None, false, &mods)
}

/// The units the first effect of `spell` lands on, cast at `t`.
fn aimed(m: &Map, caster: Guid, spell: u32, t: &Targets) -> Vec<Guid> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    targets::effect_targets(m, caster, &s, 0, t, &mods)
}

/// A pet's tick of an ordinary burn hurts a creature but puts no one in
/// the fight: not the pet, its master or the creature (mangos-classic's
/// `DealDamage`, "DOTs don't put in combat"). Its direct spell damage
/// puts the pet and its master in the fight, and the creature on the
/// pet (Thistle Tea's "direct pet damage reports caster contact while
/// ordinary periodic damage does not"). A channel's tick is left out:
/// in wvs it doesn't put its caster in the fight, in Thistle Tea it does.
#[tokio::test]
async fn a_pets_direct_spell_draws_it_and_its_master_in_and_a_dot_tick_doesnt() {
    let realm = fast_realm(data);
    let (_c, h, map, pet) = hunter(&realm, "P9CT1", "Contacter", &[]).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            let data = m.realm.data.clone();
            let kobold = kit::dummy(m, h, 60, 5.0);
            let burn = data.spells.get(PET_BURN).cloned().unwrap();
            kit::apply_template(m, pet, kobold, &burn);
            let before = kit::health(m, kobold);
            kit::tick_now(m, kobold, PET_BURN);
            let ticked = before - kit::health(m, kobold);
            let after_tick = (fighting(m, pet), fighting(m, h), fighting(m, kobold));
            let other = kit::dummy(m, h, 60, 5.0);
            let bolt = data.spells.get(PET_BOLT).cloned().unwrap();
            spells::spell_damage(m, pet, other, &bolt, 20);
            let on_pet = m
                .combat(other)
                .is_some_and(|c| c.attacking && c.victim == pet);
            (
                ticked,
                after_tick,
                (fighting(m, pet), fighting(m, h), on_pet),
            )
        })
        .await
        .unwrap();
    let (ticked, after_tick, after_bolt) = out;
    assert_eq!(ticked, 20, "the tick hurts");
    assert_eq!(after_tick, (false, false, false), "pet, master, creature");
    assert_eq!(after_bolt, (true, true, true), "pet, master, on the pet");
}

/// An immune Sap from a stealthed rogue at a flagged player of the other
/// side who sees her puts both in the fight, flags her, and leaves her
/// stealthed: a failure only breaks stealth for a spell that says so
/// (Thistle Tea's "a detected immune Sap engages players and alerts the
/// defensive pet"; mangos-classic's `CasterHitTargetWithSpell`). The
/// target's defensive pet isn't told in wvs, so that is left out.
#[tokio::test]
async fn an_immune_sap_seen_by_a_flagged_player_fights_and_flags_and_keeps_stealth() {
    let realm = fast_realm(data);
    let (_r, rogue, map) = kit::player(&realm, "P9CT2", "Sapper", Race::Human, Class::Rogue).await;
    let (_t, target, _) =
        kit::player(&realm, "P9CT3", "Sapped", Race::Undead, Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, rogue, 60, &[SAP]);
            kit::ready(m, target, 60, &[]);
            face_off(m, rogue, target);
            reset(m, &[rogue, target]);
            flag(m, target);
            kit::apply(m, target, target, WARD);
            kit::apply(m, rogue, rogue, SHADOWED);
            let seen = stealth::sees(m, target, rogue);
            let cast = cast_fresh(m, rogue, SAP, at(target));
            (
                (seen, cast, kit::has(m, target, SAP)),
                (fighting(m, rogue), fighting(m, target)),
                flagged(m, rogue),
                kit::has(m, rogue, SHADOWED),
            )
        })
        .await
        .unwrap();
    let (cast, fight, flagged, stealthed) = out;
    assert_eq!(cast, (true, true, false), "seen, cast, kept off");
    assert_eq!(fight, (true, true), "the rogue and her target");
    assert!(flagged, "the rogue flagged");
    assert!(stealthed, "still stealthed");
}

/// A spell whose failure breaks stealth and which makes no threat, kept
/// off a creature that doesn't see through its caster's stealth, still
/// provokes it: the creature fights the caster, the caster is in the
/// fight and out of stealth (Thistle Tea's "failure-breaks-stealth
/// provokes an undetecting creature" and "failure-breaks-stealth
/// overrides concealment and no-threat"; mangos-classic's
/// `CasterHitTargetWithSpell`, `RemoveAurasWithDispelType`).
#[tokio::test]
async fn a_failure_that_breaks_stealth_provokes_the_unseeing_and_unhides_its_caster() {
    let realm = fast_realm(data);
    let (_c, a, map) = kit::player(&realm, "P9CT4", "Caught", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[LOUD]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, kobold, kobold, WARD);
            kit::apply(m, a, a, HIDDEN);
            let unseen = !stealth::sees(m, kobold, a);
            let cast = cast_fresh(m, a, LOUD, at(kobold));
            (
                (unseen, cast, kit::has(m, kobold, LOUD)),
                engaged(m, kobold, a),
                fighting(m, a),
                kit::has(m, a, HIDDEN),
            )
        })
        .await
        .unwrap();
    let (cast, engaged, fighting, hidden) = out;
    assert_eq!(cast, (true, true, false), "unseen, cast, kept off");
    assert!(engaged, "the creature fights its caster");
    assert!(fighting, "the caster in the fight");
    assert!(!hidden, "out of stealth");
}

/// A missile kept off a creature engages it by whether it sees its
/// caster as the missile lands: cast seen and hidden in flight, it
/// doesn't; cast hidden and seen in flight, it does. An instant engages
/// as it is cast, whatever its caster does after (Thistle Tea's "a
/// projectile uses current concealment while an instant uses its launch
/// snapshot").
#[tokio::test]
async fn a_missile_reads_its_casters_stealth_as_it_lands_an_instant_as_it_leaves() {
    let realm = fast_realm(data);
    let (_c, a, map) = kit::player(&realm, "P9CT5", "Shooter", Race::Human, Class::Rogue).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            m.spells.travel = Some(true);
            kit::ready(m, a, 60, &[SLOW, SLOW_MISSILE]);
            // Whether a warded kobold ten yards off is engaged as the
            // spell leaves and once it has landed, its caster hidden or
            // not as it leaves and the other way after.
            let shot = |m: &mut Map, spell: u32, hidden: bool| {
                auras::remove_spell(m, a, HIDDEN, None);
                let kobold = kit::dummy(m, a, 60, 10.0);
                kit::apply(m, kobold, kobold, WARD);
                if hidden {
                    kit::apply(m, a, a, HIDDEN);
                }
                let cast = cast_fresh(m, a, spell, at(kobold));
                let leaving = engaged(m, kobold, a);
                if hidden {
                    auras::remove_spell(m, a, HIDDEN, None);
                } else {
                    kit::apply(m, a, a, HIDDEN);
                }
                pass(m, 600);
                (cast, leaving, engaged(m, kobold, a))
            };
            (
                shot(m, SLOW_MISSILE, false),
                shot(m, SLOW_MISSILE, true),
                shot(m, SLOW, false),
            )
        })
        .await
        .unwrap();
    let (seen_then_hidden, hidden_then_seen, instant) = out;
    assert_eq!(
        seen_then_hidden,
        (true, false, false),
        "a missile, hidden in flight"
    );
    assert_eq!(
        hidden_then_seen,
        (true, false, true),
        "a missile, seen in flight"
    );
    assert_eq!(instant, (true, true, true), "an instant, hidden after");
}

/// A proc's instant 20 fire hurts a creature but starts no fight: the
/// creature isn't in combat, attacks no one and is no one's tap, and the
/// player isn't in combat (Thistle Tea's "instant proc damage lands
/// without starting combat for either owner"; mangos-classic's
/// `DealDamage` puts both in combat for direct damage).
#[tokio::test]
async fn a_procs_damage_hurts_without_a_fight_for_either() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9CT6", "Procced", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let before = kit::health(m, kobold);
            spells::cast_triggered(m, a, PROC_FIRE, Some(kobold));
            let c = m.creature(kobold).unwrap();
            (
                before - kit::health(m, kobold),
                fighting(m, kobold),
                c.combat.attacking,
                c.tapped_by,
                fighting(m, a),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (20, false, false, None, false));
}

/// A channel of no initial threat, cast at a creature, puts no one in
/// the fight as it lands; its first tick, killing the creature, gives
/// its caster the kill (Thistle Tea's "a channeled first damage tick
/// enters combat and preserves lethal-tick credit", its lethal half;
/// `kill_once`'s tapper). A first tick that doesn't kill puts no one in
/// the fight in wvs, nor taps the creature, so that half is left out.
#[tokio::test]
async fn a_channels_killing_tick_gives_its_caster_the_kill() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P9CT7", "Drainer", Class::Warlock).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[DRAIN]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let cast = cast_fresh(m, a, DRAIN, at(kobold));
            let landed = (
                kit::casting(m, a),
                kit::has(m, kobold, DRAIN),
                engaged(m, kobold, a),
                fighting(m, kobold),
                fighting(m, a),
                m.creature(kobold).and_then(|c| c.tapped_by),
            );
            kit::set_health(m, kobold, 1);
            pass(m, 1_000);
            let c = m.creature(kobold).unwrap();
            (cast, landed, c.alive(), c.tapped_by)
        })
        .await
        .unwrap();
    let (cast, landed, alive, tapped_by) = out;
    assert!(cast);
    assert_eq!(
        landed,
        (Some(DRAIN), true, false, false, false, None),
        "channelling, on it, no fight, no tap"
    );
    assert!(!alive, "killed by its tick");
    assert_eq!(tapped_by, Some(a), "the caster's kill");
}

/// A spell wanting a focus and a script's creature near its caster, with
/// neither there, is refused for the focus: no cast, its mana kept. The
/// same spell wanting no focus is refused for the creature (Thistle
/// Tea's "reports a missing focus before a missing scripted
/// destination"; mangos-classic's `CheckCast`, the focus before
/// `CheckScriptTargeting`). Thistle Tea's is a script's place (46), which
/// a cast in wvs doesn't want, so the order is shown with a script's
/// creature (38).
#[tokio::test]
async fn a_focus_is_wanted_before_a_scripts_creature() {
    let realm = fast_realm(placed);
    let (_c, a, map) = human(&realm, "P9CT8", "Smith", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[FOCUSED, SCRIPTED]);
            face_east(m, a);
            let focus = why(m, a, FOCUSED);
            let script = why(m, a, SCRIPTED);
            let mana = kit::power(m, a, 0);
            let cast = cast_fresh(m, a, FOCUSED, Targets::default());
            (
                focus,
                script,
                (cast, mana - kit::power(m, a, 0), kit::casting(m, a)),
            )
        })
        .await
        .unwrap();
    let (focus, script, cast) = out;
    assert_eq!(focus, Some(fail::REQUIRES_SPELL_FOCUS), "the focus first");
    assert_eq!(script, Some(fail::BAD_TARGETS), "then the creature");
    assert_eq!(cast, (false, 0, None), "no cast, no mana");
}

/// An effect at any unit with a script's place second lands on the unit
/// aimed at, not the script's creature nearer; a harmful one at an enemy
/// with that place second, aimed at no one, is refused for its implicit
/// target (Thistle Tea's "secondary locations preserve explicit unit
/// targeting").
#[tokio::test]
async fn a_script_place_keeps_the_unit_aimed_at() {
    let realm = fast_realm(placed);
    let (_c, a, map) = human(&realm, "P9CT9", "Aimer", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[ANY_UNIT_THERE, HARM_THERE]);
            face_east(m, a);
            let beast = unit_at(m, a, BEAST, 2.0, 0.0);
            let dragon = unit_at(m, a, DRAGONKIN, 5.0, 0.0);
            let aimed_at = aimed(m, a, ANY_UNIT_THERE, &at(dragon));
            (beast, dragon, aimed_at, why(m, a, HARM_THERE))
        })
        .await
        .unwrap();
    let (beast, dragon, aimed_at, harm) = out;
    assert_eq!(aimed_at, vec![dragon], "the unit aimed at");
    assert!(!aimed_at.contains(&beast), "not the script's creature");
    assert_eq!(harm, Some(fail::BAD_IMPLICIT_TARGETS), "harmful, at no one");
}

/// A persistent area at a script's place lands on no unit there, not the
/// script's creature, which a spell at the script's creature near the
/// caster finds (Thistle Tea's "persistent areas do not deliver their
/// effect to the location creature").
#[tokio::test]
async fn a_script_places_area_names_no_unit() {
    let realm = fast_realm(placed);
    let (_c, a, map) = human(&realm, "P9CTA", "Arealist", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            face_east(m, a);
            let beast = unit_at(m, a, BEAST, 2.0, 0.0);
            let none = Targets::default();
            (
                beast,
                aimed(m, a, SCRIPTED, &none),
                aimed(m, a, AREA_THERE, &none),
            )
        })
        .await
        .unwrap();
    let (beast, near, area) = out;
    assert_eq!(near, vec![beast], "the script's creature is there");
    assert!(area.is_empty(), "the area names no unit: {area:?}");
}

/// A PvP-enabling spell of no threat kept off a flagged player of the
/// other side who sees its caster contests the caster without a fight
/// for either; unseen, it contests no one (Thistle Tea's "combat-free PvP
/// enabling requires detection on a miss"). Unseen, wvs still flags the
/// caster, as any spell reaching a flagged player does (mangos-classic's
/// `FillTargetMap`), where Thistle Tea flags no one: that is left out.
#[tokio::test]
async fn a_missed_pvp_enabling_spell_flags_only_when_seen() {
    let realm = fast_realm(data);
    let (_a, alice, map) = kit::player(&realm, "P9CTB", "Marker", Race::Dwarf, Class::Hunter).await;
    let (_g, grom, _) = kit::player(&realm, "P9CTC", "Marked", Race::Undead, Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, alice, 60, &[MARK]);
            kit::ready(m, grom, 60, &[]);
            face_off(m, alice, grom);
            kit::apply(m, grom, grom, WARD);
            let mark = |m: &mut Map, hidden: bool| {
                reset(m, &[alice, grom]);
                flag(m, grom);
                auras::remove_spell(m, alice, HIDDEN, None);
                if hidden {
                    kit::apply(m, alice, alice, HIDDEN);
                }
                let seen = stealth::sees(m, grom, alice);
                let cast = cast_fresh(m, alice, MARK, at(grom));
                (
                    (seen, cast, kit::has(m, grom, MARK)),
                    contested(m, alice),
                    (fighting(m, alice), fighting(m, grom)),
                    flagged(m, alice),
                )
            };
            (mark(m, false), mark(m, true))
        })
        .await
        .unwrap();
    let (seen, unseen) = out;
    assert_eq!(seen.0, (true, true, false), "seen, cast, kept off");
    assert!(seen.1, "seen: contested");
    assert_eq!(seen.2, (false, false), "seen: no fight");
    assert!(seen.3, "seen: flagged");
    assert_eq!(unseen.0, (false, true, false), "unseen, cast, kept off");
    assert!(!unseen.1, "unseen: not contested");
    assert_eq!(unseen.2, (false, false), "unseen: no fight");
}

/// Feed Pet, cast at a food its pet eats, takes one of it and puts its
/// trigger from the DBC on the pet, from the hunter, of the food's
/// benefit to a pet of its level (35,000 for food of the pet's level;
/// Thistle Tea's "queues the DBC trigger spell for the selected food item
/// and active pet"; mangos-classic's `EffectFeedPet`). Thistle Tea's
/// range of ten yards to the pet isn't checked in wvs.
#[tokio::test]
async fn feed_pet_puts_its_trigger_on_the_pet_by_the_food() {
    let realm = fast_realm(data);
    let (_c, h, map, pet) = hunter(&realm, "P9CTD", "Feeder", &[FEED]).await;
    let out = map
        .query(move |m| {
            if let Some(s) = m.creature_mut(pet).and_then(|c| c.pet.as_mut()) {
                s.food_mask = 1 << (MEAT - 1);
            }
            let given = inventory::add_item(m, h, FOOD, 2, false).is_ok();
            let food = m.players[&h]
                .items
                .iter()
                .find(|i| i.entry == FOOD)
                .map(|i| i.guid);
            let cast = cast_fresh(
                m,
                h,
                FEED,
                Targets {
                    item: food,
                    ..Default::default()
                },
            );
            let fed =
                kit::holder(m, pet, FED).map(|x| (x.caster, x.effects.first().map(|e| e.amount)));
            (
                (given, food.is_some(), cast),
                fed,
                m.players[&h].carried_count_of(FOOD),
            )
        })
        .await
        .unwrap();
    let (cast, fed, left) = out;
    assert_eq!(cast, (true, true, true), "given, found, cast");
    assert_eq!(fed, Some((h, Some(35_000))), "on the pet, from the hunter");
    assert_eq!(left, 1, "one eaten");
}
