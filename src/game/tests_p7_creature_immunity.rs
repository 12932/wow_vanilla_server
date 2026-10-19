//! Round 7's creature tests, ported from Thistle Tea's
//! `test/game/core/creature` (docs/parity/matrix.csv): a template's school
//! and mechanic immunities, against what bypasses them and at a DoT's
//! ticks; creatures with no defence against spells; sessile creatures under
//! fear and leaving their fights; unkillable ones and their scripts' floors;
//! wounds that don't slow; and the real combat dummies, water guardians and
//! repair bots.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{
    ATTR_EX2_NO_SCHOOL_IMMUNITIES, ATTR_EX3_ALWAYS_HIT,
    ATTR_EX3_IGNORE_CASTER_AND_TARGET_RESTRICTIONS, ATTR_NO_IMMUNITIES, SpellDuration, SpellEffect,
    SpellTemplate,
};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{
    self, AURA_MOD_FEAR, AURA_MOD_MECHANIC_RESISTANCE, AURA_MOD_STUN, AURA_PERIODIC_DAMAGE,
};
use crate::game::map::{Map, creature_combat, scripts};
use crate::game::tests_guards::hardy;
use crate::game::tests_p3_combat::{BRUTE, aura_of, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::{on, step};
use crate::game::tests_p3_spells_kit::tick_now;

/// A template immune to fire; one immune to stuns; one with no defence
/// against spells; one that never moves; one that can't be killed.
const FIREPROOF: u32 = 4_720;
const STUNPROOF: u32 = 4_721;
const DEFENCELESS: u32 = 4_722;
const SESSILE: u32 = 4_723;
const UNKILLABLE: u32 = 4_724;
const UNSLOWED: u32 = 4_725;
const UNKILLABLE_SPAWN: u32 = 4_724;

const SCORCH: u32 = 94_720;
const SCORCH_UNSTOPPABLE: u32 = 94_721;
const SCORCH_UNRESTRICTED: u32 = 94_722;
const SCORCH_PIERCING: u32 = 94_723;
const BURN: u32 = 94_724;
const STUN_SELF: u32 = 94_725;
const STUN_UNRESTRICTED: u32 = 94_726;
const HORROR: u32 = 94_727;
const RESOLVE: u32 = 94_728;
const TERROR: u32 = 94_729;

const FIRE_MASK: u32 = 1 << 2;
/// A mechanic its target resists whole.
const HORROR_MECHANIC: u32 = 24;

fn magic(id: u32, school: u32, effect: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        school,
        dmg_class: 1,
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 920,
        ..Default::default()
    };
    s.effects[0] = effect;
    s
}

fn effect(effect: u32, aura: u32, points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect,
        aura,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        amplitude: if aura == AURA_PERIODIC_DAMAGE {
            1000
        } else {
            0
        },
        target_a,
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = data();
    let mut add_template = |entry: u32, f: &dyn Fn(&mut CreatureTemplate)| {
        let mut t = template(entry, 7);
        f(&mut t);
        d.creature_templates.insert(entry, t);
    };
    add_template(FIREPROOF, &|t| t.school_immune_mask = FIRE_MASK);
    add_template(STUNPROOF, &|t| t.mechanic_immune_mask = 1 << 11);
    add_template(DEFENCELESS, &|t| t.static_flags1 = 0x8000);
    add_template(SESSILE, &|t| t.static_flags1 = 0x100);
    add_template(UNKILLABLE, &|t| t.static_flags1 = 0x8);
    add_template(UNSLOWED, &|t| {
        t.static_flags2 = 0x40;
        t.rank = 1;
    });
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: UNKILLABLE_SPAWN,
        entries: vec![UNKILLABLE],
        map: 0,
        x: crate::game::tests_p3_combat::START.0 + 4.0,
        y: crate::game::tests_p3_combat::START.1,
        z: crate::game::tests_p3_combat::START.2,
        respawn_min: 60,
        respawn_max: 60,
        ..Default::default()
    });
    d.spells.durations.insert(
        920,
        SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    let mut add = |s: SpellTemplate| {
        d.spells.spells.insert(s.id, s);
    };
    let scorch = |id: u32, attr: (usize, u32)| {
        let mut s = magic(id, 2, effect(2, 0, 20, 6));
        s.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
        s.attributes[attr.0] |= attr.1;
        s
    };
    add(scorch(SCORCH, (0, 0)));
    add(scorch(SCORCH_UNSTOPPABLE, (0, ATTR_NO_IMMUNITIES)));
    add(scorch(
        SCORCH_UNRESTRICTED,
        (3, ATTR_EX3_IGNORE_CASTER_AND_TARGET_RESTRICTIONS),
    ));
    add(scorch(SCORCH_PIERCING, (2, ATTR_EX2_NO_SCHOOL_IMMUNITIES)));
    let mut burn = magic(BURN, 2, effect(6, AURA_PERIODIC_DAMAGE, 20, 6));
    burn.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    add(burn);
    let stun = |id: u32, target_a: u32| {
        let mut s = magic(id, 0, effect(6, AURA_MOD_STUN, 0, target_a));
        s.mechanic = 12;
        s.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
        s
    };
    add(stun(STUN_SELF, 1));
    let mut unrestricted = stun(STUN_UNRESTRICTED, 6);
    unrestricted.attributes[3] |= ATTR_EX3_IGNORE_CASTER_AND_TARGET_RESTRICTIONS;
    add(unrestricted);
    // A shadow blow of the mechanic resisted, no roll of its own spared.
    let mut horror = magic(HORROR, 5, effect(2, 0, 20, 6));
    horror.mechanic = HORROR_MECHANIC;
    add(horror);
    let mut terror = magic(TERROR, 5, effect(6, AURA_MOD_FEAR, 0, 6));
    terror.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    add(terror);
    d
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// Thistle Tea's `CreatureImmunity.school?`: a template's mask is each of
/// its schools, physical, fire and frost of 21, and none of the rest.
#[tokio::test]
async fn a_templates_school_mask_is_immune_to_each_of_its_schools() {
    let mut d = world();
    d.creature_templates
        .get_mut(&FIREPROOF)
        .unwrap()
        .school_immune_mask = 21;
    let (_realm, _a, ga, map) = setup_with(d, Class::Mage).await;
    let out = on(&map, move |m| {
        let g = spawn(m, ga, FIREPROOF, 20, 3.0);
        (0..7)
            .map(|school| creature_combat::school_immune(m, g, 1 << school))
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(out, [true, false, true, false, true, false, false]);
}

/// Thistle Tea's "full bypass attributes permit damage but the aura-only
/// bypass does not", as mangos-classic has it: only a spell with no
/// immunities carries through a template's school; one that pierces school
/// immunities is stopped (it pierces auras alone), and so is one ignoring
/// caster and target restrictions (mangos's `SpellHitResult` checks
/// immunity for it all the same; Thistle Tea lets it through).
#[tokio::test]
async fn only_a_spell_with_no_immunities_carries_through_a_templates_school() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        [
            SCORCH,
            SCORCH_UNSTOPPABLE,
            SCORCH_UNRESTRICTED,
            SCORCH_PIERCING,
        ]
        .map(|spell| {
            let g = spawn(m, ga, FIREPROOF, 20, 3.0);
            let before = health(m, g);
            crate::game::map::spells::cast_triggered(m, ga, spell, Some(g));
            before - health(m, g)
        })
    })
    .await;
    assert_eq!(
        out,
        [0, 20, 0, 0],
        "(plain, no immunities, unrestricted, piercing)"
    );
}

/// Thistle Tea's "periodic protection retains tick clocks and resumes after
/// protection changes" (mangos's `IsImmuneToDamage` at each tick): a burn
/// put on a creature, its template then one immune to fire, ticks for
/// nothing and stays, its ticks counted; its template as it was, the burn
/// goes on burning.
#[tokio::test]
async fn a_burns_ticks_on_a_creature_its_template_makes_immune_burn_nothing_and_resume() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let g = spawn(m, ga, BRUTE, 20, 3.0);
        crate::game::map::spells::cast_triggered(m, ga, BURN, Some(g));
        let ticks = |m: &Map| {
            auras::auras(m, g)
                .and_then(|a| a.holders.iter().find(|h| h.spell == BURN))
                .map(|h| h.effects[0].ticks)
        };
        m.creature_mut(g).unwrap().entry = FIREPROOF;
        let before = health(m, g);
        tick_now(m, g, BURN);
        let protected = (before - health(m, g), ticks(m));
        m.creature_mut(g).unwrap().entry = BRUTE;
        let before = health(m, g);
        tick_now(m, g, BURN);
        (protected, (before - health(m, g), ticks(m)))
    })
    .await;
    assert_eq!(
        out.0,
        (0, Some(1)),
        "nothing, the tick counted, the burn on"
    );
    assert_eq!(out.1, (20, Some(2)), "burning again");
}

/// Thistle Tea's "self casts and restriction-ignoring spells bypass
/// template protection", where mangos-classic differs for the second: a
/// creature immune to stuns stuns itself (its own cast passes its
/// template, mangos's `IsImmuneToSpell(castOnSelf)`), but a stun from
/// another that ignores caster and target restrictions is still kept off
/// (mangos checks a harmful spell's immunity whatever that attribute).
#[tokio::test]
async fn a_creature_stuns_itself_through_its_immunity_and_nothing_else_does() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let own = spawn(m, ga, STUNPROOF, 20, 3.0);
        crate::game::map::spells::cast_triggered(m, own, STUN_SELF, Some(own));
        let other = spawn(m, ga, STUNPROOF, 20, 3.0);
        crate::game::map::spells::cast_triggered(m, ga, STUN_UNRESTRICTED, Some(other));
        let stunned = |m: &Map, g: Guid| m.creature(g).unwrap().auras.has(AURA_MOD_STUN);
        (stunned(m, own), stunned(m, other))
    })
    .await;
    assert_eq!(out, (true, false));
}

/// Thistle Tea's `SpellResist.magic_hit?` with no spell defence
/// (`NO_SPELL_DEFENSE`, `static_flags1` 0x8000): a creature resisting a
/// spell's mechanic whole is missed by it, but one with no defence
/// against spells is hit whatever it resists.
#[tokio::test]
async fn a_creature_with_no_spell_defence_is_hit_whatever_it_resists() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        [BRUTE, DEFENCELESS].map(|entry| {
            let g = spawn(m, ga, entry, 60, 3.0);
            aura_of(
                m,
                g,
                RESOLVE,
                AURA_MOD_MECHANIC_RESISTANCE,
                HORROR_MECHANIC as i32,
                100,
            );
            let before = health(m, g);
            crate::game::map::spells::cast_triggered(m, ga, HORROR, Some(g));
            before - health(m, g)
        })
    })
    .await;
    assert_eq!(out, [0, 20], "(resisted, hit)");
}

/// Thistle Tea's sessile creatures, where mangos-classic differs: a
/// sessile creature's fear lands (Thistle Tea makes it immune), but it
/// stands under it, its AI holding it rooted (mangos's `SESSILE`, as
/// `SetAIImmobilizedState`); and leaving its fight, moved from where it
/// stood, it stays where it is, whole and out of its fight (Thistle Tea's
/// "sessile evasion cleans combat without moving home").
#[tokio::test]
async fn a_sessile_creature_stands_under_fear_and_leaves_its_fight_where_it_stands() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let (g, at) = on(&map, move |m| {
        hardy(m, ga);
        let g = spawn(m, ga, SESSILE, 20, 3.0);
        crate::game::map::ai::attacked(m, g, ga);
        crate::game::map::spells::cast_triggered(m, ga, TERROR, Some(g));
        (g, m.position_of(g).unwrap())
    })
    .await;
    let from = on(&map, |m| m.stats.updates).await;
    crate::game::tests_p3_entity_objects::until(&map, "ten updates", move |m| {
        m.stats.updates >= from + 10
    })
    .await;
    let feared = on(&map, move |m| {
        (
            m.creature(g).unwrap().auras.has(AURA_MOD_FEAR),
            crate::game::map::combat::dist(m.position_of(g).unwrap(), at),
        )
    })
    .await;
    assert!(feared.0, "the fear on it");
    assert!(feared.1 < 0.01, "standing, {} yards off", feared.1);
    let out = on(&map, move |m| {
        auras::remove_spell(m, g, TERROR, Some(ga));
        let c = m.creature_mut(g).unwrap();
        c.position.x += 10.0;
        c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000);
        let moved = c.position;
        crate::game::map::ai::enter_evade(m, g);
        let c = m.creature(g).unwrap();
        (
            crate::game::map::combat::dist(c.position, moved),
            matches!(c.motion, crate::game::map::creature::Motion::Moving(_)),
            c.evading,
            c.threat.is_empty(),
            c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            ),
            c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    })
    .await;
    assert_eq!(
        out,
        (0.0, false, false, true, false, true),
        "(moved, walking, evading, threat gone, in combat, whole)"
    );
}

/// Thistle Tea's "scripts can override the threshold and scripted death
/// still works": an unkillable creature's floor is a script's to raise
/// (command 52) or take away (a floor of 0, and it dies); respawned, its
/// template's floor of one holds again; and a kill kills it whatever its
/// floor.
#[tokio::test]
async fn an_unkillable_creatures_floor_is_a_scripts_and_a_kill_kills_it() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Mage).await;
    let g = crate::game::guid::unit(UNKILLABLE, UNKILLABLE_SPAWN);
    a.sees(g).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let blow = |m: &mut Map, g: Guid| {
            crate::game::map::combat::deal_damage(m, ga, g, 1_000_000);
            health(m, g)
        };
        scripts::execute(m, &step(52, [20, 0, 0, 0]), g, g);
        let raised = blow(m, g);
        scripts::execute(m, &step(52, [0, 0, 0, 0]), g, g);
        let taken = blow(m, g);
        crate::game::map::ai::respawn_now(m, g);
        let back = scripts::creature_of_spawn(m, UNKILLABLE_SPAWN).unwrap();
        let whole = health(m, back) == m.unit_fields(back).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
        let floored = blow(m, back);
        crate::game::map::combat::kill(m, ga, back);
        (raised, taken, whole, floored, health(m, back))
    })
    .await;
    assert_eq!(out, (20, 0, true, 1, 0));
}

/// Thistle Tea's "loads the separate wounded slowdown exemption": a
/// creature whose template says wounds don't slow it (`static_flags2`
/// 0x40), elite or not, runs whole at a sliver of health; one without
/// slows.
#[tokio::test]
async fn a_creature_wounds_dont_slow_runs_whole() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        [BRUTE, UNSLOWED].map(|entry| {
            let g = spawn(m, ga, entry, 20, 3.0);
            let whole = m.creature(g).unwrap().speeds.run;
            crate::game::map::combat::deal_damage(m, ga, g, 99_000);
            m.creature(g).unwrap().speeds.run / whole
        })
    })
    .await;
    assert!((out[0] - 0.5).abs() < 0.01, "slowed: {}", out[0]);
    assert!((out[1] - 1.0).abs() < 0.01, "whole: {}", out[1]);
}

/// Thistle Tea's vmangos flags, on the real templates: the combat dummies
/// (1921, 4952, 5652) can't be killed, don't swing and don't chase, and
/// the two that are immune to players say so in their unit flags; a Minor
/// Water Guardian (3950) has no defence against blows or spells; a Field
/// Repair Bot (14337) doesn't chase but would swing; and Darkmaster
/// Gandling (1853), an elite, isn't slowed by his wounds.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_dummies_guardians_and_bots_have_their_flags() {
    let db = crate::game::harness::real_data("db");
    if !db.join("vmangos.sqlite").exists() {
        return;
    }
    let data = crate::game::harness::load_real(&db);
    let (_realm, _a, ga, map) = setup_with(data, Class::Mage).await;
    let out = on(&map, move |m| {
        use crate::game::map::event_ai::{combat_movement, invincible_floor, melee};
        let dummies = [1921, 4952, 5652].map(|entry| {
            let g = spawn(m, ga, entry, 1, 3.0);
            (
                invincible_floor(m, g),
                melee(m, g),
                combat_movement(m, g),
                m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_FLAGS) & 0x100 != 0,
            )
        });
        let guardian = spawn(m, ga, 3950, 1, 3.0);
        let flags = m.realm.data.creature_template(3950).unwrap().static_flags1;
        let guardian = (
            flags & crate::game::map::combat::STATIC_NO_DEFENSE != 0,
            creature_combat::no_spell_defense(m, guardian),
        );
        let bot = spawn(m, ga, 14_337, 1, 3.0);
        let bot = (combat_movement(m, bot), melee(m, bot));
        let gandling = spawn(m, ga, 1853, 60, 3.0);
        let whole = m.creature(gandling).unwrap().speeds.run;
        crate::game::map::combat::deal_damage(m, ga, gandling, 99_900);
        let gandling = (m.creature(gandling).unwrap().speeds.run - whole).abs() < 0.01;
        (dummies, guardian, bot, gandling)
    })
    .await;
    assert_eq!(
        out.0,
        [
            (1, false, false, false),
            (1, false, false, true),
            (1, false, false, true)
        ],
        "(floor, swings, chases, immune to players)"
    );
    assert_eq!(out.1, (true, true), "no defence, nor against spells");
    assert_eq!(out.2, (false, true), "the bot stands, but would swing");
    assert!(out.3, "Gandling runs whole");
}
