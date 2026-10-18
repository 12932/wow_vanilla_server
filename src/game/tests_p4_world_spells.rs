//! Phase 3's fourth round of a spell's flight and hit, ported from
//! Thistle Tea's `test/game/world/entity/effect_resolver`
//! (`spell_launch_test.exs`, `triggered_hit_dbc_test.exs`;
//! docs/parity/matrix.csv): how long a missile flies, and which spells roll
//! the magic hit, against whose avoidance.

use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::map::Map;
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

const LEECH: u32 = 98_410;
const ENERGIZE: u32 = 98_411;

/// Thistle Tea's "periodic leech threat goes to the drained creature
/// without splitting among attackers", "player and creature owners
/// receive grants and leeches" and "direct leech threat routes only to
/// creatures", as mangos-classic's `PeriodicTick`: a mana leech's tick
/// takes from the creature it is on and gives its caster the mana, half
/// of it threat on the drained creature for the caster alone; a dead
/// caster's leech gives nothing; a creature's grant of mana fills a
/// player's pool, a player's a creature's.
#[tokio::test]
async fn a_leech_drains_to_its_living_caster_and_threatens_the_drained() {
    use crate::game::fields::*;
    use crate::game::map::auras::{self, AURA_PERIODIC_MANA_LEECH};
    let mut d = data();
    let mut leech = SpellTemplate {
        id: LEECH,
        name: "Leech".into(),
        dmg_class: 1,
        duration_index: 9,
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    leech.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_PERIODIC_MANA_LEECH,
        amplitude: 1000,
        base_points: 9,
        base_dice: 1,
        multiple_value: 1.0,
        target_a: 6,
        ..Default::default()
    };
    d.spells.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 60_000,
            per_level: 0,
            max: 60_000,
        },
    );
    d.spells.spells.insert(LEECH, leech);
    let mut energize = bolt(ENERGIZE, 1, false);
    energize.effects[0] = SpellEffect {
        effect: 30,
        misc_value: 0,
        base_points: 29,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(ENERGIZE, energize);
    let (_realm, _a, g, map) = setup_with(d, Class::Mage).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, g, BRUTE, 20, 4.0);
        let helper = spawn(m, g, BRUTE, 20, 6.0);
        for c in [brute, helper] {
            let f = m.unit_fields_mut(c).unwrap();
            f.set_byte(UNIT_FIELD_BYTES_0, 3, 0);
            f.set_u32(UNIT_FIELD_MAXPOWER1, 1_000);
            f.set_u32(UNIT_FIELD_POWER1, 500);
        }
        let mana = |m: &Map, u| m.unit_fields(u).unwrap().get_u32(UNIT_FIELD_POWER1);
        let set_mana =
            |m: &mut Map, u, v| m.unit_fields_mut(u).unwrap().set_u32(UNIT_FIELD_POWER1, v);
        set_mana(m, g, 0);
        crate::game::map::ai::attacked(m, brute, g);
        crate::game::map::ai::attacked(m, brute, helper);
        let threat = |m: &Map, by| {
            m.creature(brute)
                .unwrap()
                .threat
                .iter()
                .find(|(t, _)| *t == by)
                .map_or(0.0, |(_, v)| *v)
        };
        let before = (threat(m, g), threat(m, helper));
        let s = m.realm.data.spells.get(LEECH).cloned().unwrap();
        let _ = auras::apply_spell(m, g, brute, &s, &[(0, 10)], auras::Context::default());
        let tick = |m: &mut Map| {
            let now = m.now;
            if let Some(a) = auras::auras_mut(m, brute) {
                for h in a.holders.iter_mut().filter(|h| h.spell == LEECH) {
                    for x in h.effects.iter_mut() {
                        x.next_tick = Some(now);
                    }
                }
            }
            auras::update(m);
        };
        tick(m);
        let drained = (
            mana(m, brute),
            mana(m, g),
            threat(m, g) - before.0,
            threat(m, helper) - before.1,
        );
        crate::game::map::combat::deal_damage(m, brute, g, 1_000_000);
        tick(m);
        let dead = (mana(m, brute), mana(m, g));
        // Grants: a creature's to a player, a player's to a creature.
        crate::game::map::death::resurrect(m, g, 1.0);
        set_mana(m, g, 10);
        set_mana(m, helper, 10);
        crate::game::map::spells::cast_triggered(m, helper, ENERGIZE, Some(g));
        crate::game::map::spells::cast_triggered(m, g, ENERGIZE, Some(helper));
        (drained, dead, (mana(m, g), mana(m, helper)))
    })
    .await;
    let (drained, dead, granted) = out;
    assert_eq!((drained.0, drained.1), (490, 10), "ten taken, ten given");
    assert!(
        (drained.2 - 5.0).abs() < 1e-3,
        "half of it threat for the caster: {}",
        drained.2
    );
    assert_eq!(drained.3, 0.0, "none for another on the list");
    assert_eq!(dead, (490, 10), "a dead caster's leech takes nothing");
    assert_eq!(granted.0, 40, "a creature's grant fills a player");
    assert_eq!(granted.1, 40, "a player's a creature");
}
