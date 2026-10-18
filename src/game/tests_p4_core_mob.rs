//! Phase 3, round 4, against Thistle Tea's `entity/mob_test.exs`: what a
//! spawned creature is built with (its weapons out and its auras shown as
//! they are, its model's gender, its class's health), and what a respawn
//! keeps (its template's immunities and its blows' school).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::combat::{self, Outcome};
use crate::game::map::{Map, ai, auras, creature_combat, derive};
use crate::game::tests_p3_combat::*;

const SPAWN: u32 = 7_001;

/// The brute spawned two yards east of the start: of a display that is a
/// woman's, immune to stuns (12) and fire, striking with holy.
fn world() -> GameData {
    let mut d = data();
    let t = d.creature_templates.get_mut(&BRUTE).unwrap();
    t.mechanic_immune_mask = 1 << 11;
    t.school_immune_mask = 4;
    t.damage_school = 1;
    d.display_info.insert(
        903,
        DisplayInfo {
            scale: 1.0,
            gender: Some(1),
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: SPAWN,
            entries: vec![BRUTE],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        }],
    );
    d
}

/// What a respawn must keep: its bytes, its gender, its health, its
/// immunities.
fn built(m: &Map, g: Guid) -> ([u8; 4], u8, u32, bool, bool) {
    let f = &m.creature(g).unwrap().fields;
    (
        f.get_u32(UNIT_FIELD_BYTES_2).to_le_bytes(),
        f.get_byte(UNIT_FIELD_BYTES_0, 2),
        f.get_u32(UNIT_FIELD_MAXHEALTH),
        creature_combat::mechanic_immune(m, g, 12),
        creature_combat::school_immune(m, g, 4),
    )
}

/// The school (its index) a swing's log says.
fn school_of(b: &[u8]) -> u32 {
    let mut at = 4;
    for _ in 0..2 {
        at += 1 + b[at].count_ones() as usize;
    }
    u32::from_le_bytes(b[at + 5..at + 9].try_into().unwrap())
}

#[tokio::test]
async fn a_spawned_creature_has_its_weapons_out_its_auras_shown_its_models_gender_and_keeps_them_through_a_respawn()
 {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let brute = guid::unit(BRUTE, SPAWN);
    a.sees(brute).await;
    let r = map
        .query(move |m| {
            level(m, g, 60);
            let first = built(m, brute);
            // Its class's health at its level, through a stat recompute and
            // an aura's coming and going.
            derive::unit(m, brute);
            aura_of(m, brute, 97_901, auras::AURA_MOD_INCREASE_HEALTH, 0, 100);
            derive::unit(m, brute);
            let buffed = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            auras::remove_spell(m, brute, 97_901, None);
            derive::unit(m, brute);
            let after = built(m, brute);
            forced_swing(m, brute, g, Outcome::Hit);
            combat::deal_damage(m, g, brute, 10_000_000);
            ai::respawn_now(m, brute);
            (first, buffed, after, built(m, brute))
        })
        .await
        .unwrap();
    let (first, buffed, after, respawned) = r;
    assert_eq!(first.0, [1, 0x10, 0, 0], "weapons out, auras as they are");
    assert_eq!(first.1, 1, "the model's gender");
    assert_eq!(first.2, 5_000, "the class's health at 20");
    assert!(first.3 && first.4, "immune to stuns and fire");
    assert_eq!(buffed, 5_100);
    assert_eq!(after, first);
    assert_eq!(respawned, first);
    let frame = a
        .wait_frame("the brute's blow", combat::SMSG_ATTACKERSTATEUPDATE, |_| {
            true
        })
        .await;
    assert_eq!(school_of(&frame), 1, "a holy blow");
}

#[tokio::test]
async fn the_first_to_tag_a_creature_has_its_credit_though_another_kills_it() {
    let (realm, _a, alice, map) = setup(Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let bob = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    let r = map
        .query(move |m| {
            for g in [alice, bob] {
                level(m, g, 18);
            }
            let brute = spawn(m, bob, BRUTE, 20, 2.0);
            let xp = |m: &Map, g: Guid| m.players[&g].fields.get_u32(PLAYER_XP);
            let before = (xp(m, alice), xp(m, bob));
            forced_swing(m, bob, brute, Outcome::Miss);
            let tapped = m.creature(brute).unwrap().tapped_by;
            combat::deal_damage(m, alice, brute, 10_000_000);
            (tapped, before, (xp(m, alice), xp(m, bob)))
        })
        .await
        .unwrap();
    let (tapped, before, after) = r;
    assert_eq!(tapped, Some(bob), "a miss tags it too");
    assert_eq!(after.0, before.0, "Alice's blow, no credit");
    assert!(after.1 > before.1, "Bob's credit: {before:?} {after:?}");
}
