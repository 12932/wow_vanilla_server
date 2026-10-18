//! Phase 3's creature tests, ported from Thistle Tea's
//! `test/game/core/creature` (docs/parity/matrix.csv): the core hounds'
//! linked feign death.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Class;

use crate::game::data::GameData;
use crate::game::data::scripts::ScriptStep;
use crate::game::fields::*;
use crate::game::map::{Map, event_ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, aura_of, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;

/// A core hound, unkillable as its template has it; and another kind of
/// hound.
const HOUND: u32 = 3_790;
const OTHER_HOUND: u32 = 3_791;
const PACIFY: u32 = 97_900;

fn hounds() -> GameData {
    let mut d = data();
    let mut hound = template(HOUND, 1);
    hound.static_flags1 |= crate::game::map::combat::STATIC_UNKILLABLE;
    d.creature_templates.insert(HOUND, hound);
    d.creature_templates
        .insert(OTHER_HOUND, template(OTHER_HOUND, 1));
    d
}

/// Alice, beyond a pack's harm.
fn hardy(m: &mut Map, g: Guid) {
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000_000);
    f.set_u32(UNIT_FIELD_HEALTH, 100_000_000);
}

/// A hound's linked feign death: up after `delay_ms` if a hound fights on
/// within 30 yards.
fn link(m: &mut Map, g: Guid, delay_ms: u32) {
    let feign = ScriptStep {
        dataint: [0, 0, 0, 0],
        ..crate::game::tests_p3_entity::step(
            scripts::C_LINKED_FEIGN_DEATH,
            [delay_ms, HOUND, 30, 0],
        )
    };
    scripts::execute(m, &feign, g, Guid::zero());
}

/// Alive, health, stand state, the fallen flag, down.
fn state(m: &Map, g: Guid) -> (bool, u32, u8, bool, bool) {
    let c = m.creature(g).unwrap();
    (
        c.alive(),
        c.fields.get_u32(UNIT_FIELD_HEALTH),
        c.fields.get_byte(UNIT_FIELD_BYTES_1, 0),
        c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & 0x20 != 0,
        event_ai::feigning(m, g),
    )
}

/// Thistle Tea's `LinkedFeignDeath.prevent`: a killing blow, exact or
/// overkill, leaves the hound down at one health, its template's floor of
/// one notwithstanding; it keeps its foe's threat and tap, but its auras
/// go; and down, no more harm, healing or regeneration touches it. A blow
/// short of lethal leaves it fighting, and a creature without the link
/// dies of the same.
#[tokio::test]
async fn a_fallen_hound_takes_the_killing_blow_and_nothing_more() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let h = spawn(m, ga, HOUND, 20, 3.0);
        link(m, h, 60_000);
        crate::game::map::ai::attacked(m, h, ga);
        aura_of(m, h, PACIFY, crate::game::map::auras::AURA_MOD_PACIFY, 0, 0);
        // Short of lethal: hurt, not down.
        crate::game::map::combat::deal_damage(m, ga, h, 99_999);
        let hurt = state(m, h);
        // Exactly lethal.
        crate::game::map::combat::deal_damage(m, ga, h, 1);
        let down = state(m, h);
        let c = m.creature(h).unwrap();
        let kept = (
            c.threat.iter().any(|(t, _)| *t == ga),
            c.tapped_by,
            c.auras.holders.is_empty(),
            c.combat.attacking,
        );
        // More harm, a heal, regeneration: still one.
        crate::game::map::combat::deal_damage(m, ga, h, 1_000);
        crate::game::map::spells::heal(m, ga, h, 0, 1_000, false);
        let now = m.now;
        m.creature_mut(h).unwrap().engagement.regen_at = Some(now);
        crate::game::map::creature_combat::regenerate(m);
        let after = state(m, h);
        // Overkill on a second.
        let h2 = spawn(m, ga, HOUND, 20, 4.0);
        link(m, h2, 60_000);
        crate::game::map::ai::attacked(m, h2, ga);
        crate::game::map::combat::deal_damage(m, ga, h2, 10_000_000);
        let overkill = state(m, h2);
        // No link: dead.
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        crate::game::map::ai::attacked(m, brute, ga);
        crate::game::map::combat::deal_damage(m, ga, brute, 10_000_000);
        let brute = m.creature(brute).is_none_or(|c| !c.alive());
        (hurt, down, kept, after, overkill, brute)
    })
    .await;
    assert_eq!(out.0, (true, 1, 0, false, false), "hurt, still fighting");
    assert_eq!(out.1, (true, 1, 7, true, true), "down at one, fallen");
    assert_eq!(out.2.0, true, "its foe's threat kept");
    assert_eq!(out.2.1, Some(ga), "and its tap");
    assert!(out.2.2, "its auras gone");
    assert!(!out.2.3, "no longer swinging");
    assert_eq!(out.3, out.1, "nothing more touches it");
    assert_eq!(out.4, (true, 1, 7, true, true), "overkill the same");
    assert!(out.5, "the brute, unlinked, dies");
}

/// Thistle Tea's `LinkedFeignDeath.tick`: a hound down, with no hound of
/// its kind fighting on within the radius, dies at its time; one out of
/// reach or of another kind doesn't count. A script's kill takes no
/// notice of the link.
#[tokio::test]
async fn a_fallen_hound_dies_without_a_fighting_kin_in_reach() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let (h, far, other, killed) = on(&map, move |m| {
        hardy(m, ga);
        let h = spawn(m, ga, HOUND, 20, 3.0);
        let far = spawn(m, ga, HOUND, 20, 45.0);
        let other = spawn(m, ga, OTHER_HOUND, 20, 4.0);
        let killed = spawn(m, ga, HOUND, 20, 5.0);
        for g in [h, far, other, killed] {
            link(m, g, 300);
            crate::game::map::ai::attacked(m, g, ga);
        }
        // The far one held where it is, out of reach.
        aura_of(
            m,
            far,
            PACIFY + 1,
            crate::game::map::auras::AURA_MOD_ROOT,
            0,
            0,
        );
        crate::game::map::combat::deal_damage(m, ga, h, 10_000_000);
        crate::game::map::combat::kill(m, ga, killed);
        (h, far, other, killed)
    })
    .await;
    let killed = on(&map, move |m| m.creature(killed).is_none_or(|c| !c.alive())).await;
    assert!(killed, "a script's kill kills");
    until(&map, "the fallen hound dies", move |m| {
        m.creature(h).is_none_or(|c| !c.alive())
    })
    .await;
    let others = on(&map, move |m| {
        [far, other].map(|g| m.creature(g).is_some_and(|c| c.alive()))
    })
    .await;
    assert_eq!(
        others,
        [true, true],
        "the far hound and the other kind fight on"
    );
}

/// Thistle Tea's `LinkedFeignDeath.reset`: a fallen hound whose foe gives
/// up the fight gets up whole, out of it.
#[tokio::test]
async fn a_fallen_hound_left_alone_gets_up_whole() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let h = on(&map, move |m| {
        let h = spawn(m, ga, HOUND, 20, 3.0);
        link(m, h, 60_000);
        crate::game::map::ai::attacked(m, h, ga);
        crate::game::map::combat::deal_damage(m, ga, h, 10_000_000);
        crate::game::map::ai::enter_evade(m, h);
        h
    })
    .await;
    until(&map, "up again", move |m| {
        let c = m.creature(h).unwrap();
        !event_ai::feigning(m, h)
            && c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH)
    })
    .await;
    let s = on(&map, move |m| state(m, h)).await;
    assert_eq!(
        (s.0, s.2, s.3, s.4),
        (true, 0, false, false),
        "standing, whole"
    );
}
