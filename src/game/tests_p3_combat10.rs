//! Phase 3's AI tests (Thistle Tea's `ai/bt/mob/spells_test.exs`, its
//! friendly-injured heal targeting): whom a creature's spell list heals,
//! the most hurt friend near in a fight, past the entry's threshold.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{CreatureSpell, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::spells::lists;
use crate::game::tests_p3_combat::{self as base, *};

const HEALER: u32 = 3740;
const MENDER: u32 = 95_800;
/// Its lists: the most injured friend; the same but never itself; one
/// with its own radius and threshold; one whose threshold is out of range.
const ANY: u32 = 700;
const EXCEPT_SELF: u32 = 701;
const NEAR_AND_HURT: u32 = 702;
const BAD_THRESHOLD: u32 = 703;

fn data() -> GameData {
    let mut d = base::data();
    d.creature_templates.insert(HEALER, template(HEALER, 7));
    let mut heal = SpellTemplate {
        id: MENDER,
        name: "Mend".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    heal.effects[0] = SpellEffect {
        effect: 10,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 21,
        ..Default::default()
    };
    d.spells.spells.insert(MENDER, heal);
    for (list, target, radius, threshold) in [
        (ANY, 17, 0, 0),
        (EXCEPT_SELF, 18, 0, 0),
        (NEAR_AND_HURT, 17, 5, 80),
        (BAD_THRESHOLD, 17, 0, 150),
    ] {
        d.spells.creature_spells.insert(
            list,
            vec![CreatureSpell {
                spell: MENDER,
                probability: 100,
                target,
                target_param1: radius,
                target_param2: threshold,
                ..Default::default()
            }],
        );
    }
    d
}

/// A creature of `entry` `yards` before `g` at `pct` of its health, in a
/// fight or not.
fn hurt(m: &mut Map, g: Guid, entry: u32, yards: f32, pct: u32, fighting: bool) -> Guid {
    let c = spawn(m, g, entry, 20, yards);
    let f = m.unit_fields_mut(c).unwrap();
    f.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
    f.set_u32(UNIT_FIELD_HEALTH, pct * 10);
    f.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, fighting);
    c
}

/// Whom `healer` heals with `list`, if anyone: the one whose health rose.
fn healed(m: &mut Map, healer: Guid, list: u32, among: &[Guid]) -> Option<Guid> {
    let before: Vec<u32> = among
        .iter()
        .map(|g| m.unit_fields(*g).unwrap().get_u32(UNIT_FIELD_HEALTH))
        .collect();
    let c = m.creature_mut(healer).unwrap();
    c.scripted.get_or_insert_with(Default::default).spell_list = Some(list);
    c.caster.spell_list = None;
    lists::tick(m, healer, Guid::zero());
    among
        .iter()
        .zip(before)
        .find(|(g, b)| m.unit_fields(**g).unwrap().get_u32(UNIT_FIELD_HEALTH) > *b)
        .map(|(g, _)| *g)
}

#[tokio::test]
async fn a_creatures_heal_goes_to_the_most_injured_friend_in_a_fight_itself_among_them() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let healer = hurt(m, ga, HEALER, 5.0, 90, true);
            let lightly = hurt(m, ga, BRUTE, 6.0, 40, true);
            let badly = hurt(m, ga, BRUTE, 7.0, 20, true);
            // The worse of its two hurt friends.
            let worst = healed(m, healer, ANY, &[healer, lightly, badly]);
            // Hurt worst of all, itself.
            m.unit_fields_mut(healer)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 100);
            let own = healed(m, healer, ANY, &[healer, lightly, badly]);
            // Never itself, with the list that says so.
            m.unit_fields_mut(healer)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 100);
            let except = healed(m, healer, EXCEPT_SELF, &[healer, lightly, badly]);
            (
                worst == Some(badly),
                own == Some(healer),
                except.is_some_and(|g| g != healer),
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, true, true));
}

#[tokio::test]
async fn a_creatures_heal_passes_over_the_lightly_hurt_the_idle_and_its_enemies() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let healer = hurt(m, ga, HEALER, 5.0, 100, true);
            // Hurt less than half: not worth it.
            let scratched = hurt(m, ga, BRUTE, 6.0, 60, true);
            let none = healed(m, healer, ANY, &[scratched]);
            // Hurt badly but out of the fight; and an injured enemy.
            let idle = hurt(m, ga, BRUTE, 6.0, 10, false);
            let enemy = hurt(m, ga, HELPER, 6.0, 10, true);
            let skipped = healed(m, healer, ANY, &[idle, enemy]);
            // A threshold past a hundred falls back to half.
            let bad = healed(m, healer, BAD_THRESHOLD, &[scratched]);
            (none, skipped, bad)
        })
        .await
        .unwrap();
    assert_eq!(r, (None, None, None));
}

#[tokio::test]
async fn a_creatures_heal_looks_within_its_entrys_radius_for_one_past_its_threshold() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let healer = hurt(m, ga, HEALER, 5.0, 100, true);
            // Within five yards but only half hurt; beyond them, worse.
            let near = hurt(m, ga, BRUTE, 7.0, 50, true);
            let far = hurt(m, ga, BRUTE, 15.0, 5, true);
            let first = healed(m, healer, NEAR_AND_HURT, &[near, far]);
            m.unit_fields_mut(near)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 150);
            let second = healed(m, healer, NEAR_AND_HURT, &[near, far]);
            (first, second == Some(near))
        })
        .await
        .unwrap();
    assert_eq!(r, (None, true));
}
