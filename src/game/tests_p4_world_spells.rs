//! Phase 3's fourth round of a spell's flight and hit, ported from
//! Thistle Tea's `test/game/world/entity/effect_resolver`
//! (`spell_launch_test.exs`, `triggered_hit_dbc_test.exs`;
//! docs/parity/matrix.csv): how long a missile flies, and which spells roll
//! the magic hit, against whose avoidance.

use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::auras::{
    AURA_MOD_AOE_AVOIDANCE, AURA_MOD_RESIST_CHANCE, AURA_REFLECT_SPELLS,
};
use crate::game::map::spells::mods::Mods;
use crate::game::map::spells::outcome::{Hit, roll_hit};
use crate::game::tests_p3_combat::{BRUTE, aura_of, data, setup_with, spawn};
use crate::game::tests_p4_world::on;

/// Thistle Tea's "point-blank missiles use the minimum flight distance and
/// self casts remain immediate" (`SpellLaunch.delay`): a missile flies its
/// distance at its speed, as from five yards when nearer; to one not in
/// the map, it doesn't fly.
#[tokio::test]
async fn a_missile_flies_its_distance_at_its_speed_from_five_yards() {
    let (_realm, _a, g, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let near = spawn(m, g, BRUTE, 20, 1.0);
        let far = spawn(m, g, BRUTE, 20, 40.0);
        let gone = crate::game::guid::unit(BRUTE, 0x7F_FFFF);
        use crate::game::map::spells::flight_ms;
        (
            flight_ms(m, g, near, 20.0),
            flight_ms(m, g, far, 20.0),
            flight_ms(m, g, gone, 20.0),
        )
    })
    .await;
    assert_eq!(out.0, 250, "point blank: five yards at twenty a second");
    assert!((1_995..=2_005).contains(&out.1), "forty yards: {}", out.1);
    assert_eq!(out.2, 0, "nowhere to fly to");
}

const WARD: u32 = 98_400;
const MIRROR: u32 = 98_401;
const SHIELD: u32 = 98_402;

/// A harmful spell of `school` and damage class at a unit, or about the
/// caster with `area`.
fn bolt(id: u32, dmg_class: u32, area: bool) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        school: 2,
        dmg_class,
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        target_a: if area { 22 } else { 6 },
        target_b: if area { 15 } else { 0 },
        ..Default::default()
    };
    s
}

/// Thistle Tea's "beneficial, unclassified and weapon spells bypass the
/// magic roll", "a saved resist still allows reflection to resolve first"
/// and "area triggers use each target's current avoidance", as
/// mangos-classic's `SpellHitResult`: against one who resists all fire, a
/// magic bolt is resisted and one of no damage class lands (it was rolled
/// as magic); a helpful spell lands; a reflection goes first; and an area's
/// spell is resisted by one who avoids areas, not by another, nor is a bolt.
#[tokio::test]
async fn which_spells_roll_the_magic_hit_and_against_what() {
    let (_realm, _a, g, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let warded = spawn(m, g, BRUTE, 20, 4.0);
        let plain = spawn(m, g, BRUTE, 20, 5.0);
        aura_of(m, warded, WARD, AURA_MOD_RESIST_CHANCE, 0x7F, 10_000);
        let none = Mods::default();
        let magic = bolt(1, 1, false);
        let unclassed = bolt(2, 0, false);
        let mut helpful = bolt(3, 1, false);
        helpful.effects[0].effect = 10;
        helpful.effects[0].target_a = 21;
        let resisted = (
            roll_hit(m, g, warded, &magic, &none),
            roll_hit(m, g, warded, &unclassed, &none),
            roll_hit(m, g, warded, &helpful, &none),
        );
        aura_of(m, warded, MIRROR, AURA_REFLECT_SPELLS, 0, 100);
        let reflected = roll_hit(m, g, warded, &magic, &none);
        aura_of(m, plain, SHIELD, AURA_MOD_AOE_AVOIDANCE, 0, 10_000);
        let wave = bolt(4, 1, true);
        let avoided = (
            roll_hit(m, g, plain, &wave, &none),
            roll_hit(m, g, plain, &magic, &none),
        );
        (resisted, reflected, avoided)
    })
    .await;
    assert_eq!(
        out.0,
        (Hit::Resist, Hit::Hit, Hit::Hit),
        "magic, none, helpful"
    );
    assert_eq!(out.1, Hit::Reflect, "a reflection before the resist");
    assert_eq!(
        out.2,
        (Hit::Resist, Hit::Hit),
        "an area avoided, a bolt not"
    );
}
