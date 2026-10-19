//! Round 7, against Thistle Tea's `test/game/core/entity_test.exs` and its
//! neighbours (docs/parity/matrix.csv): what a player's death takes of
//! what they control, and what a blow they live through leaves.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::spells::SpellEffect;
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::harness;
use crate::game::map::{Map, MapHandle, auras, combat, pets, spells};
use crate::game::tests_p3_entity::on;

/// A totem in the first slot and its creature, and a charm of a creature
/// (Enslave Demon's aura) that lasts.
const TOTEM_SPELL: u32 = 997_701;
const TOTEM: u32 = 997_750;
const CHARM: u32 = 997_702;

fn controls(d: &mut GameData) {
    d.creature_templates.insert(
        TOTEM,
        CreatureTemplate {
            entry: TOTEM,
            name: "Made-up Totem".into(),
            display_ids: [4587, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 60,
            level_max: 60,
            faction: 35,
            ..Default::default()
        },
    );
    let mut totem = bare(TOTEM_SPELL, 3);
    totem.duration_index = LASTS_LONG;
    effect(
        &mut totem,
        0,
        SpellEffect {
            effect: 87,
            misc_value: TOTEM as i32,
            base_points: 99,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut charm = bare(CHARM, 5);
    charm.duration_index = LASTS_LONG;
    effect(&mut charm, 0, aura(auras::AURA_MOD_CHARM, 0, 0, 6));
    insert(d, [totem, charm]);
}

/// Run the map's updates until `f` holds.
async fn until(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&Map) -> bool + Send + Sync + Clone + 'static,
) {
    crate::game::tests_p3_entity_objects::until(map, what, f).await;
}

/// What `g` controls: its totems and its charm, and whether the charmed
/// creature still serves it.
fn controlled(m: &Map, g: Guid, kobold: Guid) -> (usize, Option<Guid>, bool) {
    let o = &m.players[&g].owned;
    let serves = pets::state(m, kobold).is_some_and(|s| s.owner == g);
    (o.totems.iter().flatten().count(), o.charm, serves)
}

/// Thistle Tea's `take_damage_with_absorb/4` "clears all player totem
/// slots only on lethal damage" and "queues a charm release when a player
/// dies while controlling a unit": a blow a shaman lives through leaves
/// their totems standing and their charmed creature theirs; the killing
/// blow takes the totems down and lets the creature go, as mangos's
/// `SetDeathState(JUST_DIED)` (`UnsummonAllTotems`, `BreakCharmOutgoing`).
#[tokio::test]
async fn a_blow_lived_through_keeps_totems_and_a_charm_and_death_ends_both() {
    let realm = fast_realm(controls);
    let (_c, a, map) = human(&realm, "P7C1", "Totemic", Class::Warrior).await;
    let kobold = on(&map, move |m| {
        kit::ready(m, a, 60, &[]);
        spells::cast_triggered(m, a, TOTEM_SPELL, Some(a));
        let kobold = kit::dummy(m, a, 60, 5.0);
        kit::apply(m, a, kobold, CHARM);
        kobold
    })
    .await;
    let first = on(&map, move |m| controlled(m, a, kobold)).await;
    assert_eq!(
        first,
        (1, Some(kobold), true),
        "a totem up, a creature charmed"
    );
    let other = on(&map, move |m| kit::dummy(m, a, 60, 8.0)).await;
    on(&map, move |m| combat::deal_damage(m, other, a, 10)).await;
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    let hurt = on(&map, move |m| {
        (
            controlled(m, a, kobold),
            m.players[&a].death == crate::game::map::death::Death::Alive,
        )
    })
    .await;
    assert_eq!(hurt, (first, true), "a blow lived through takes nothing");
    on(&map, move |m| {
        let health = kit::health(m, a);
        combat::deal_damage(m, other, a, health);
    })
    .await;
    until(&map, "the totem down and the creature let go", move |m| {
        controlled(m, a, kobold) == (0, None, false)
    })
    .await;
}

/// Thistle Tea's `take_damage_with_absorb/4` "does not dismiss the pet on
/// non-lethal damage": a hunter's pet stays out through a blow its master
/// lives through (its going at the master's death is
/// `tests_p3_creature2::a_hunters_pet_keeps_its_mood_growth_stance_and_autocast_put_away_and_goes_at_its_masters_death`).
#[tokio::test]
async fn a_hunters_pet_stays_out_through_a_blow_its_master_lives_through() {
    let (_realm, _h, gh, map, pet, wolf) = crate::game::tests_pets::tamed().await;
    on(&map, move |m| combat::deal_damage(m, wolf, gh, 5)).await;
    for _ in 0..3 {
        harness::an_update(&map).await;
    }
    let out = on(&map, move |m| {
        (
            m.players[&gh].owned.pet,
            m.players[&gh].owned.unsummoned,
            m.creature(pet).is_some(),
        )
    })
    .await;
    assert_eq!(out, (Some(pet), false, true), "the pet stays out");
}

/// The pet's attack: whether it attacks, whom it targets, and whether it
/// is in combat.
fn pet_fight(m: &Map, pet: Guid) -> (bool, Guid, bool) {
    let c = m.creature(pet).unwrap();
    (
        m.combat(pet).is_some_and(|c| c.attacking),
        c.fields.get_guid(crate::game::fields::UNIT_FIELD_TARGET),
        c.fields.has_flag(
            crate::game::fields::UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        ),
    )
}

/// Thistle Tea's pet commands "follow stops attacking while retaining
/// recent combat contact" and "passive immediately stops attacking while
/// retaining recent combat contact": told to follow, or made passive, a
/// pet at its foe stops its attack and lets go of its target, still in
/// combat (mangos's `HandlePetAction`: `AttackStop(true, true)`, the
/// target cleared and the cast interrupted, the combat left to time out).
#[tokio::test]
async fn a_pet_told_to_follow_or_made_passive_stops_attacking_but_stays_in_combat() {
    use crate::game::map::pets::{ACT_COMMAND, ACT_REACTION, COMMAND_ATTACK, COMMAND_FOLLOW};
    let (_realm, h, _gh, map, pet, wolf) = crate::game::tests_pets::tamed().await;
    // The wolf outlasts the test.
    on(&map, move |m| {
        let f = m.unit_fields_mut(wolf).unwrap();
        f.set_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH, 1_000_000);
        f.set_u32(crate::game::fields::UNIT_FIELD_HEALTH, 1_000_000);
    })
    .await;
    for stop in [
        pets::button(u32::from(COMMAND_FOLLOW), ACT_COMMAND),
        pets::button(u32::from(pets::REACT_PASSIVE), ACT_REACTION),
    ] {
        h.send(CMSG_PET_ACTION {
            pet,
            data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
            target: wolf,
        })
        .await;
        until(&map, "the pet at the wolf, in combat", move |m| {
            pet_fight(m, pet) == (true, wolf, true)
        })
        .await;
        h.send(CMSG_PET_ACTION {
            pet,
            data: stop,
            target: Guid::zero(),
        })
        .await;
        until(&map, "its attack stopped", move |m| !pet_fight(m, pet).0).await;
        let after = on(&map, move |m| pet_fight(m, pet)).await;
        assert_eq!(
            after,
            (false, Guid::zero(), true),
            "{stop:#x}: no target, in combat"
        );
        // Aggressive again for the next round.
        on(&map, move |m| {
            if let Some(s) = pets::state_mut(m, pet) {
                s.react = pets::REACT_DEFENSIVE;
            }
        })
        .await;
    }
}

/// A made-up Feign Death (5384's aura), of five seconds.
const FEIGN: u32 = 997_703;

fn feign(d: &mut GameData) {
    let mut s = bare(FEIGN, 0);
    s.duration_index = LASTS_5S;
    effect(&mut s, 0, aura(auras::AURA_FEIGN_DEATH, 0, 0, 1));
    insert(d, [s]);
}

/// Thistle Tea's `reset/1` "Feign Death replacement preserves the pose and
/// the final removal restores standing": Feign Death cast again over
/// itself keeps the hunter lying (the dead dynamic flag, standing), and
/// its removal, or its running out, lets them up: the flag off, still
/// standing, no other dynamic flag touched.
#[tokio::test]
async fn feign_death_cast_again_keeps_the_pose_and_its_end_lets_them_up() {
    use crate::game::fields::UNIT_DYNAMIC_FLAGS;
    let realm = fast_realm(feign);
    let (_c, g, map) = human(&realm, "P7C3", "Feigner", Class::Warrior).await;
    let pose = |m: &Map, g: Guid| {
        let p = &m.players[&g];
        (
            kit::has(m, g, FEIGN),
            p.fields.get_u32(UNIT_DYNAMIC_FLAGS),
            p.stand_state(),
        )
    };
    let out = on(&map, move |m| {
        kit::ready(m, g, 60, &[]);
        if let Some(p) = m.players.get_mut(&g) {
            p.fields.set_u32(UNIT_DYNAMIC_FLAGS, 0x08);
        }
        spells::cast_triggered(m, g, FEIGN, Some(g));
        let lying = pose(m, g);
        kit::pass(m, 1_000);
        spells::cast_triggered(m, g, FEIGN, Some(g));
        let again = pose(m, g);
        auras::remove_spell(m, g, FEIGN, None);
        let removed = pose(m, g);
        spells::cast_triggered(m, g, FEIGN, Some(g));
        kit::pass(m, 6_000);
        (lying, again, removed, pose(m, g))
    })
    .await;
    let (lying, again, removed, expired) = out;
    assert_eq!(lying, (true, 0x28, 0), "lying as the dead");
    assert_eq!(again, lying, "cast again, the same pose");
    assert_eq!(removed, (false, 0x08, 0), "removed: up");
    assert_eq!(expired, (false, 0x08, 0), "run out: up");
}

/// Thistle Tea's `SpellSkills.learn/2` "preserves earned progress and
/// stable slots when learning ranks in any order": Expert Mining (step 3)
/// learned over Apprentice keeps the sixty points earned and the slot, to
/// a cap of 225; Apprentice learned after (a login reads the spells back
/// in any order) changes nothing; nor is Enchanting touched.
#[tokio::test]
async fn a_professions_ranks_in_any_order_keep_its_progress_slot_and_highest_cap() {
    use crate::game::fields::PLAYER_SKILL_INFO_1_1;
    use crate::game::map::skills;
    const MINING: u32 = 186;
    const ENCHANTING: u32 = 333;
    let realm = fast_realm(|_| {});
    let (_c, g, map) = human(&realm, "P7C4", "Miner", Class::Warrior).await;
    let out = on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let slot = |p: &crate::game::map::player::Player, skill: u32| {
            (0..128u16).find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == skill)
        };
        let state = |p: &crate::game::map::player::Player, skill: u32| {
            (
                skills::value(p, skill),
                skills::max_value(p, skill),
                slot(p, skill),
            )
        };
        skills::learn_skill(p, ENCHANTING, 1);
        skills::learn_skill(p, MINING, 1);
        skills::set_value(p, MINING, 60);
        let (enchanting, before) = (state(p, ENCHANTING), state(p, MINING));
        skills::learn_skill(p, MINING, 3);
        let expert = state(p, MINING);
        skills::learn_skill(p, MINING, 1);
        (
            enchanting,
            before,
            expert,
            state(p, MINING),
            state(p, ENCHANTING),
        )
    })
    .await;
    let (enchanting, before, expert, after, enchanting_after) = out;
    assert_eq!(before.0, Some(60));
    assert_eq!(
        expert,
        (Some(60), Some(225), before.2),
        "progress and slot kept, to 225"
    );
    assert_eq!(after, expert, "the lower rank after changes nothing");
    assert_eq!(enchanting_after, enchanting, "nor another skill");
}
