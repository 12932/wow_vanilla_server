//! Phase 3's combat tests: combo points (Thistle Tea's
//! `combo_points_test.exs`): Premeditation's held points, made for good by
//! a later builder and taken back at the hold's end but not at its
//! cancelling; and who a builder gives points to, when.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ATTR_EX3_ALWAYS_HIT, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::map::auras::{self, AURA_MOD_DODGE_PERCENT, AURA_RETAIN_COMBO_POINTS};
use crate::game::map::spells;
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, aura_of, spawn};
use crate::game::tests_p3_spells_kit::pass;

/// A point; two, held ten seconds (Premeditation); a blow that gives a
/// point.
const BUILDER: u32 = 95_950;
const PREMEDITATION: u32 = 95_951;
const STRIKE: u32 = 95_952;

fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.durations.insert(
        950,
        SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    let points = |n: i32| SpellEffect {
        effect: 80,
        base_points: n - 1,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    let spell = |id: u32| SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    let mut builder = spell(BUILDER);
    builder.effects[0] = points(1);
    builder.attributes[3] = ATTR_EX3_ALWAYS_HIT;
    let mut premeditation = spell(PREMEDITATION);
    premeditation.duration_index = 950;
    premeditation.attributes[3] = ATTR_EX3_ALWAYS_HIT;
    premeditation.effects[0] = points(2);
    premeditation.effects[1] = SpellEffect {
        effect: 6,
        aura: AURA_RETAIN_COMBO_POINTS,
        base_points: 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let mut strike = spell(STRIKE);
    strike.dmg_class = 2;
    strike.effects[0] = SpellEffect {
        effect: 2,
        base_points: 0,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    strike.effects[1] = points(1);
    for s in [builder, premeditation, strike] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

async fn rogue() -> (crate::game::harness::Client, Guid, MapHandle) {
    let realm = crate::game::harness::realm_with(data());
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Rogue).await;
    let map = realm.registry.online(g).unwrap().map();
    (a, g, map)
}

/// The points and their target, and whether they are held.
fn points(m: &Map, g: Guid) -> (u8, bool) {
    let c = m.caster(g).unwrap();
    let held = auras::auras(m, g).is_some_and(|a| a.has(AURA_RETAIN_COMBO_POINTS));
    (c.combo_points, held)
}

fn give(m: &mut Map, g: Guid, target: Guid, n: u8) {
    crate::game::map::spells::add_combo_points(m, g, target, n);
}

#[tokio::test]
async fn premeditations_points_come_off_at_its_end_unless_a_builder_made_them_good_or_it_was_cancelled()
 {
    let (_a, g, map) = rogue().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            let target = spawn(m, g, BRUTE, 20, 3.0);
            // One, then two held: three; at the end, the one.
            give(m, g, target, 1);
            spells::cast_triggered(m, g, PREMEDITATION, Some(target));
            out.push(points(m, g));
            pass(m, 9_999);
            out.push(points(m, g));
            pass(m, 1);
            out.push(points(m, g));
            // Four, and two held: five; the end takes the two.
            give(m, g, target, 3);
            spells::cast_triggered(m, g, PREMEDITATION, Some(target));
            out.push(points(m, g));
            pass(m, 10_000);
            out.push(points(m, g));
            // Four, two held, then a builder: the hold goes, five kept.
            give(m, g, target, 1);
            spells::cast_triggered(m, g, PREMEDITATION, Some(target));
            spells::cast_triggered(m, g, BUILDER, Some(target));
            out.push(points(m, g));
            pass(m, 10_000);
            out.push(points(m, g));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            (3, true),
            (3, true),
            (1, false),
            (5, true),
            (3, false),
            (5, false),
            (5, false)
        ]
    );
}

#[tokio::test]
async fn a_cancelled_hold_keeps_its_points() {
    let (_a, g, map) = rogue().await;
    let got = map
        .query(move |m| {
            let target = spawn(m, g, BRUTE, 20, 3.0);
            spells::cast_triggered(m, g, PREMEDITATION, Some(target));
            auras::remove_spell(m, g, PREMEDITATION, None);
            pass(m, 10_000);
            points(m, g)
        })
        .await
        .unwrap();
    assert_eq!(got, (2, false));
}

#[tokio::test]
async fn a_builder_gives_its_caster_points_on_a_target_it_lands_on_and_a_creature_none() {
    let (_a, g, map) = rogue().await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // On a live target: the caster's, on that target.
            let target = spawn(m, g, BRUTE, 20, 3.0);
            spells::cast_triggered(m, g, BUILDER, Some(target));
            let c = m.caster(g).unwrap();
            out.push((c.combo_points, c.combo_target == target));
            // At the dead: refused, none.
            let dead = spawn(m, g, BRUTE, 20, 3.0);
            crate::game::map::combat::deal_damage(m, g, dead, 1_000_000);
            crate::game::map::ai::forget(m, g);
            spells::learn(m, g, BUILDER);
            crate::game::tests_p3_spells_kit::cast_fresh(
                m,
                g,
                BUILDER,
                crate::game::tests_p3_spells_kit::at(dead),
            );
            let c = m.caster(g).unwrap();
            out.push((c.combo_points, c.combo_target == dead));
            // A blow dodged: none; landed, one.
            let dodger = spawn(m, g, BRUTE, 1, 1.0);
            aura_of(m, dodger, 95_960, AURA_MOD_DODGE_PERCENT, 0, 10_000);
            crate::game::map::derive::creature(m.creature_mut(dodger).unwrap());
            spells::cast_triggered(m, g, STRIKE, Some(dodger));
            let c = m.caster(g).unwrap();
            out.push((c.combo_points, c.combo_target == dodger));
            let open = spawn(m, g, BRUTE, 1, 1.0);
            spells::cast_triggered(m, g, STRIKE, Some(open));
            let c = m.caster(g).unwrap();
            out.push((c.combo_points, c.combo_target == open));
            // A creature's builder gives no player points.
            let brute = spawn(m, g, BRUTE, 20, 3.0);
            spells::cast_triggered(m, brute, BUILDER, Some(g));
            let c = m.caster(g).unwrap();
            out.push((c.combo_points, c.combo_target == open));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [(1, true), (1, false), (1, false), (1, true), (1, true)]
    );
}
