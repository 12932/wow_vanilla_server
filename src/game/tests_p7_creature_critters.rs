//! Round 7's creature tests, ported from Thistle Tea's
//! `test/game/core/creature/critter_test.exs` (docs/parity/matrix.csv): what
//! sends a critter running and what doesn't (a burning ground's first tick,
//! a warding ground, a resisted spell, a heal), a DoT's ticks lengthening
//! its escape, a hold on fleeing, and its pauses between runs.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{
    ATTR_DEBUFF, ATTR_EX_NO_THREAT, ATTR_EX2_NO_INITIAL_THREAT, ATTR_EX3_ALWAYS_HIT, SpellDuration,
    SpellEffect, SpellTemplate,
};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{
    self, AURA_DISPEL_IMMUNITY, AURA_MOD_MECHANIC_RESISTANCE, AURA_MOD_ROOT, AURA_PERIODIC_DAMAGE,
    AURA_PREVENTS_FLEEING, UNIT_FLAG_FLEEING,
};
use crate::game::map::creature::Motion;
use crate::game::tests_guards::hardy;
use crate::game::tests_p3_combat::{PACK, aura_of, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;
use crate::game::tests_p3_spells_kit::tick_now;

const CRITTER: u32 = 4_730;
const BURNING_GROUND: u32 = 94_730;
const WARDING_GROUND: u32 = 94_731;
const BURN: u32 = 94_732;
const SNARE: u32 = 94_733;
const MEND: u32 = 94_734;
const HOLD: u32 = 94_735;
const RESOLVE: u32 = 94_736;
const FIVE_YARDS: u32 = 930;
/// A mechanic the critter resists whole.
const SNARE_MECHANIC: u32 = 7;

fn spell(id: u32, effect: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        school: 2,
        dmg_class: 1,
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 930,
        ..Default::default()
    };
    s.effects[0] = effect;
    s
}

fn world() -> GameData {
    let mut d = data();
    let mut critter = template(CRITTER, 8);
    critter.faction = PACK;
    d.creature_templates.insert(CRITTER, critter);
    d.spells.radii.insert(FIVE_YARDS, 5.0);
    d.spells.durations.insert(
        930,
        SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    let ground = |aura: u32, points: i32, misc: i32| SpellEffect {
        effect: 27,
        aura,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        misc_value: misc,
        amplitude: 1_000,
        radius_index: FIVE_YARDS,
        target_a: 28,
        ..Default::default()
    };
    let mut add = |s: SpellTemplate| {
        d.spells.spells.insert(s.id, s);
    };
    add(spell(BURNING_GROUND, ground(AURA_PERIODIC_DAMAGE, 1, 0)));
    // A flare's ground: a dispel immunity on the enemies in it, of no
    // threat.
    let mut ward = spell(WARDING_GROUND, ground(AURA_DISPEL_IMMUNITY, 0, 5));
    ward.attributes[0] |= ATTR_DEBUFF;
    ward.attributes[1] |= ATTR_EX_NO_THREAT;
    ward.attributes[2] |= ATTR_EX2_NO_INITIAL_THREAT;
    add(ward);
    let aura = |aura: u32, points: i32| SpellEffect {
        effect: 6,
        aura,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        amplitude: if aura == AURA_PERIODIC_DAMAGE {
            1_000
        } else {
            0
        },
        target_a: 6,
        ..Default::default()
    };
    let mut burn = spell(BURN, aura(AURA_PERIODIC_DAMAGE, 1));
    burn.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    add(burn);
    let mut snare = spell(SNARE, aura(AURA_MOD_ROOT, 0));
    snare.mechanic = SNARE_MECHANIC;
    add(snare);
    let mut mend = spell(
        MEND,
        SpellEffect {
            effect: 10,
            base_points: 1,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    mend.dmg_class = 0;
    add(mend);
    d
}

/// A critter's escape's end, its run's, and its fleeing flag.
fn escape(m: &Map, g: Guid) -> (Option<Instant>, Option<Instant>, bool) {
    let c = m.creature(g).unwrap();
    (
        c.engagement.escape_at,
        c.engagement.flee.map(|f| f.until),
        c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_FLEEING),
    )
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// A blow on `g` by `by`.
fn hit(m: &mut Map, by: Guid, g: Guid) {
    crate::game::map::combat::deal_damage(m, by, g, 10);
    crate::game::map::ai::attacked(m, g, by);
}

/// Thistle Tea's "ground damage triggers escape when the first tick lands"
/// and "ground dispel immunity refreshes do not trigger escape or threat":
/// a critter in a burning ground runs once its first tick burns it, not
/// before; one in a ground that only wards it, its aura on it again and
/// again, doesn't run, isn't in a fight and holds no threat.
#[tokio::test]
async fn a_critter_runs_from_a_burning_grounds_first_tick_not_from_a_warding_ground() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let (burnt, warded, before) = on(&map, move |m| {
        hardy(m, ga);
        let burnt = spawn(m, ga, CRITTER, 20, 3.0);
        let warded = spawn(m, ga, CRITTER, 20, 20.0);
        for (g, s) in [(burnt, BURNING_GROUND), (warded, WARDING_GROUND)] {
            let at = m.position_of(g).unwrap();
            crate::game::map::spells::cast_triggered_at(m, ga, s, at);
        }
        (burnt, warded, escape(m, burnt).0)
    })
    .await;
    assert!(before.is_none(), "not running before a tick lands");
    // Two ticks of each, at once.
    for _ in 0..2 {
        on(&map, |m| {
            let now = m.now;
            for a in m.spells.areas.iter_mut() {
                a.next_tick = now;
            }
        })
        .await;
        crate::game::harness::an_update(&map).await;
    }
    let burning = on(&map, move |m| (health(m, burnt), escape(m, burnt))).await;
    assert!(burning.0 < 100_000, "burnt");
    assert!(burning.1.0.is_some() && burning.1.2, "running from it");
    let out = on(&map, move |m| {
        let c = m.creature(warded).unwrap();
        (
            auras::auras(m, warded).is_some_and(|a| a.has_spell(WARDING_GROUND)),
            escape(m, warded),
            c.threat.is_empty(),
            c.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            ),
        )
    })
    .await;
    assert!(out.0, "the ward on it");
    assert_eq!(out.1, (None, None, false), "not running");
    assert!(out.2 && !out.3, "no threat, no fight");
}

/// Thistle Tea's "periodic damage refreshes the owner-held escape
/// deadline": a burn put on a critter starts its escape; a tick later its
/// escape is longer, its run as it was.
#[tokio::test]
async fn a_burns_ticks_lengthen_a_critters_escape_not_its_run() {
    let (_realm, a, ga, map) = setup_with(world(), Class::Mage).await;
    crate::game::tests_pets::learn(&map, ga, &[BURN]);
    let g = on(&map, move |m| {
        hardy(m, ga);
        spawn(m, ga, CRITTER, 20, 3.0)
    })
    .await;
    crate::game::tests_pets::cast(&a, BURN, Some(g)).await;
    until(&map, "the burn on it", move |m| {
        auras::auras(m, g).is_some_and(|a| a.has_spell(BURN))
    })
    .await;
    let first = on(&map, move |m| escape(m, g)).await;
    assert!(first.0.is_some() && first.1.is_some(), "running");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let later = on(&map, move |m| {
        let before = health(m, g);
        tick_now(m, g, BURN);
        (before - health(m, g), escape(m, g))
    })
    .await;
    assert_eq!(later.0, 1, "a tick");
    assert!(later.1.0 > first.0, "the escape lengthened");
    assert_eq!(later.1.1, first.1, "the run as it was");
}

/// Thistle Tea's "prevent fleeing suppresses movement until removed"
/// (mangos's `SetFleeing` refused, and `HandlePreventFleeing` putting it
/// back): a critter held from fleeing, struck, has its escape but stands,
/// showing no flight; let go, it runs.
#[tokio::test]
async fn a_critter_held_from_fleeing_stands_struck_and_runs_once_let_go() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let (g, at, struck) = on(&map, move |m| {
        hardy(m, ga);
        let g = spawn(m, ga, CRITTER, 20, 3.0);
        aura_of(m, g, HOLD, AURA_PREVENTS_FLEEING, 0, 0);
        hit(m, ga, g);
        (g, m.position_of(g).unwrap(), escape(m, g))
    })
    .await;
    assert!(struck.0.is_some(), "its escape begun");
    assert!(!struck.2, "but not fleeing");
    let from = on(&map, |m| m.stats.updates).await;
    until(&map, "ten updates", move |m| m.stats.updates >= from + 10).await;
    let held = on(&map, move |m| {
        let c = m.creature(g).unwrap();
        (
            crate::game::map::combat::dist(c.position, at),
            matches!(c.motion, Motion::Moving(_)),
        )
    })
    .await;
    assert!(held.0 < 0.01 && !held.1, "standing: {held:?}");
    let flag = on(&map, move |m| {
        auras::remove_spell(m, g, HOLD, Some(g));
        escape(m, g).2
    })
    .await;
    assert!(flag, "fleeing, let go");
    until(&map, "it runs", move |m| {
        crate::game::map::combat::dist(m.position_of(g).unwrap(), at) > 0.5
    })
    .await;
}

/// Thistle Tea's "resisted and helpful spells do not trigger escape", where
/// mangos-classic has a failed hostile spell an attack all the same
/// (`CasterHitTargetWithSpell` on a miss): a critter resisting a snare runs
/// from it; one healed doesn't.
#[tokio::test]
async fn a_resisted_snare_sends_a_critter_running_and_a_heal_doesnt() {
    let (_realm, a, ga, map) = setup_with(world(), Class::Mage).await;
    crate::game::tests_pets::learn(&map, ga, &[SNARE]);
    let snared = on(&map, move |m| {
        hardy(m, ga);
        let snared = spawn(m, ga, CRITTER, 20, 3.0);
        aura_of(
            m,
            snared,
            RESOLVE,
            AURA_MOD_MECHANIC_RESISTANCE,
            SNARE_MECHANIC as i32,
            100,
        );
        snared
    })
    .await;
    crate::game::tests_pets::cast(&a, SNARE, Some(snared)).await;
    until(&map, "the snare resisted", move |m| {
        escape(m, snared).0.is_some()
    })
    .await;
    let out = on(&map, move |m| {
        let healed = spawn(m, ga, CRITTER, 20, 4.0);
        m.unit_fields_mut(healed)
            .unwrap()
            .set_u32(UNIT_FIELD_HEALTH, 50_000);
        crate::game::map::spells::cast_triggered(m, ga, MEND, Some(healed));
        (
            m.creature(snared).unwrap().auras.has(AURA_MOD_ROOT),
            escape(m, snared).0.is_some(),
            health(m, healed) > 50_000,
            escape(m, healed),
        )
    })
    .await;
    assert_eq!(out.0, false, "the snare resisted");
    assert!(out.1, "running all the same");
    assert!(out.2, "healed");
    assert_eq!(out.3, (None, None, false), "not running");
}

/// Thistle Tea's "completed runs pause before another destination is
/// requested": a critter's next run is asked for no sooner than 800 ms
/// after the one under way ends.
#[tokio::test]
async fn a_critter_pauses_between_its_runs() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let g = on(&map, move |m| {
        hardy(m, ga);
        let g = spawn(m, ga, CRITTER, 20, 3.0);
        hit(m, ga, g);
        g
    })
    .await;
    until(&map, "it runs", move |m| {
        matches!(m.creature(g).unwrap().motion, Motion::Moving(_))
    })
    .await;
    let pause = on(&map, move |m| {
        let c = m.creature(g).unwrap();
        let Motion::Moving(w) = &c.motion else {
            return None;
        };
        let ends = w.started + w.duration;
        c.engagement.next_move_at.map(|t| t.duration_since(ends))
    })
    .await
    .expect("its next move timed");
    assert!(pause >= Duration::from_millis(800), "{pause:?}");
}
