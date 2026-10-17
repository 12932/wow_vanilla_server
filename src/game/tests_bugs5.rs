//! Bugs found while mapping wvs's spells against Thistle Tea's, the fifth
//! batch: a helpful proc's damage that missed, the Warsong flag taken up by
//! the immune, the mounted and the stunned, activation-time modifiers that
//! changed no aura's ticks, a percentage of mana that made no threat, proc
//! cooldowns and charges spent on procs that came to nothing, and Nightfall
//! on a killing tick.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::{Map, procs};

const HOLY_SHIELD: u32 = 20925;

/// A block taken from `attacker`, for `g`'s procs.
fn block(m: &mut Map, attacker: Guid, g: Guid) {
    procs::trigger(
        m,
        procs::Event {
            actor: attacker,
            target: g,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE,
            ex: procs::EX_BLOCK | procs::EX_NORMAL_HIT,
            damage: 10,
            school: 1,
            ..Default::default()
        },
    );
}

/// Holy Shield's holy damage, from a helpful aura, can't miss, as Thistle
/// Tea's `ProcDamage.hit?` and mangos's `SpellHitResult` (with the aura's
/// own spell) have it: a low paladin's blocks against a creature fifty
/// levels above them strike back every time, where a spell of theirs would
/// land one time in five.
#[tokio::test]
async fn holy_shields_damage_never_misses() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, paladin, map) = player(&realm, "B5HS", "Shieldone", Race::Human, Class::Paladin).await;
    let hits = map
        .query(move |m| {
            ready(m, paladin, 10, &[]);
            let ogre = dummy(m, paladin, 60, 3.0);
            m.spells.rolls = Some(true);
            let mut hits = 0;
            for _ in 0..40 {
                // A fresh shield each time: its four charges.
                apply(m, paladin, paladin, HOLY_SHIELD);
                let before = health(m, ogre);
                block(m, ogre, paladin);
                if health(m, ogre) < before {
                    hits += 1;
                }
            }
            m.spells.rolls = None;
            hits
        })
        .await
        .unwrap();
    assert_eq!(hits, 40, "every block strikes back");
}
