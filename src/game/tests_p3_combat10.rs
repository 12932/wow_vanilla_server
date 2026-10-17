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
/// The most injured, at a one-percent chance and an eight-second repeat.
const UNLIKELY: u32 = 704;

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
    d.spells.creature_spells.insert(
        UNLIKELY,
        vec![CreatureSpell {
            spell: MENDER,
            probability: 1,
            target: 17,
            delay_repeat: (8_000, 8_000),
            ..Default::default()
        }],
    );
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

#[tokio::test]
async fn a_lists_cast_flags_skip_an_aura_already_there_and_a_melee_only_spell_at_itself() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let healer = hurt(m, ga, HEALER, 5.0, 100, true);
            let other = hurt(m, ga, BRUTE, 6.0, 100, true);
            let allow = |m: &Map, flags: u32, target: Guid| {
                lists::flags_allow(m, healer, flags, MENDER, target)
            };
            let bare = allow(m, lists::CF_AURA_NOT_PRESENT, other);
            aura_of(m, other, MENDER, crate::game::map::auras::AURA_DUMMY, 0, 1);
            let present = allow(m, lists::CF_AURA_NOT_PRESENT, other);
            // Melee-only, at itself: never in its own melee.
            let own = allow(
                m,
                lists::CF_AURA_NOT_PRESENT | lists::CF_ONLY_IN_MELEE,
                healer,
            );
            (bare, present, own)
        })
        .await
        .unwrap();
    assert_eq!(r, (true, false, false));
}

#[tokio::test]
async fn a_list_entrys_failed_chance_starts_its_repeat_timer_and_casts_nothing() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let healer = hurt(m, ga, HEALER, 5.0, 10, true);
            let now = m.now;
            // A one-percent chance: of ten tries, those that fail heal
            // nothing and wait the eight seconds.
            let mut failed = Vec::new();
            for _ in 0..10 {
                m.unit_fields_mut(healer)
                    .unwrap()
                    .set_u32(UNIT_FIELD_HEALTH, 100);
                let cast = healed(m, healer, UNLIKELY, &[healer]).is_some();
                let ready = m
                    .creature(healer)
                    .unwrap()
                    .caster
                    .spell_list
                    .as_ref()
                    .unwrap()
                    .ready_at[0];
                if !cast {
                    failed.push(ready == now + std::time::Duration::from_secs(8));
                }
            }
            failed
        })
        .await
        .unwrap();
    assert!(!r.is_empty());
    assert!(r.iter().all(|x| *x), "{r:?}");
}
