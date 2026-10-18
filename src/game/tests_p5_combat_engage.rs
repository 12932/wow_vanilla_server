//! Phase 3's fifth round of combat tests, a creature's engagement (Thistle
//! Tea's `engagement_test.exs`): a pull stands it up, an evade clears the
//! fight and refuses another until home, and death keeps the tap.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, ai, combat};
use crate::game::tests_p3_combat::*;

/// What a creature's fight holds: its threat list's size, its tapper, its
/// selection, its combat flag, whether it attacks, and its victim.
fn fight(m: &Map, g: Guid) -> (usize, Option<Guid>, Guid, bool, bool) {
    let c = m.creature(g).unwrap();
    (
        c.threat.len(),
        c.tapped_by,
        c.fields.get_guid(UNIT_FIELD_TARGET),
        c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
        c.combat.attacking,
    )
}

/// Thistle Tea's "a resting creature stands up and stops its idle emote
/// when it is pulled", "clears victim, threat, tap, casting-facing memory,
/// and combat projection together", "refuses renewed combat until
/// return-home movement finishes" and "releases combat identity while
/// preserving corpse tap ownership": a sleeping creature at its emote
/// stands and stops it when pulled; an evade clears its threat, its tap,
/// its selection and its combat flag, and while it goes home a blow
/// starts no fight; a death keeps the tap for the loot.
#[tokio::test]
async fn a_pull_stands_a_creature_up_an_evade_clears_the_fight_and_death_keeps_the_tap() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            let c = m.creature_mut(brute).unwrap();
            c.fields.set_byte(UNIT_FIELD_BYTES_1, 0, 3);
            c.fields.set_u32(UNIT_NPC_EMOTESTATE, 69);
            ai::attacked(m, brute, ga);
            let c = m.creature(brute).unwrap();
            let pulled = (
                c.fields.get_byte(UNIT_FIELD_BYTES_1, 0),
                c.fields.get_u32(UNIT_NPC_EMOTESTATE),
            );
            let engaged = fight(m, brute);
            ai::enter_evade(m, brute);
            let evaded = fight(m, brute);
            let evading = m.creature(brute).unwrap().evading;
            ai::attacked(m, brute, ga);
            let refused = fight(m, brute);
            // Another, fought and killed.
            let other = spawn(m, ga, BRUTE, 20, 3.0);
            ai::attacked(m, other, ga);
            combat::deal_damage(m, ga, other, 1_000_000);
            let c = m.creature(other).unwrap();
            let dead = (
                c.tapped_by,
                c.threat.len(),
                c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
            );
            (pulled, engaged, evaded, evading, refused, dead)
        })
        .await
        .unwrap();
    let (pulled, engaged, evaded, evading, refused, dead) = r;
    assert_eq!(pulled, (0, 0), "stood up, the emote stopped");
    assert_eq!(engaged, (1, Some(ga), ga, true, true), "in the fight");
    assert_eq!(evaded, (0, None, Guid::zero(), false, false), "all cleared");
    assert!(evading, "going home");
    assert_eq!(refused, evaded, "no fight on the way home");
    assert_eq!(dead, (Some(ga), 0, false), "the tap kept for the loot");
}
