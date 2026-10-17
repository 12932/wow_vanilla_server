//! Phase 3's combat tests, part seven (Thistle Tea's
//! `creature/creature_flags_test.exs`): what a template's static flags
//! make of a creature: its unit and type flags, who may attack it, an
//! unkillable one's last point of health, and a defenseless one's table.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::load::{type_flags, unit_flags};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::{Map, spells};
use crate::game::tests_p3_combat::{self as base, *};

/// Unkillable; without defense.
const UNKILLABLE: u32 = 3720;
const DEFENSELESS: u32 = 3721;
const BURN: u32 = 95_600;

fn data() -> GameData {
    let mut d = base::data();
    for (entry, flags) in [(UNKILLABLE, 0x8), (DEFENSELESS, 0x4000)] {
        let mut t = template(entry, 7);
        t.static_flags1 = flags;
        d.creature_templates.insert(entry, t);
    }
    let mut burn = crate::game::data::spells::SpellTemplate {
        id: BURN,
        name: "Burn".into(),
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 9,
        ..Default::default()
    };
    burn.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura: 3,
        base_points: 149,
        base_dice: 1,
        die_sides: 1,
        amplitude: 1000,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(BURN, burn);
    d.spells.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 3_000,
            ..Default::default()
        },
    );
    d
}

#[test]
fn static_flags_give_the_unit_flags_and_the_clients_type_flags() {
    // Immune to players and creatures, unselectable, swimming, PvP.
    assert_eq!(unit_flags(0x10400260), 0x0200_9300);
    assert_eq!(unit_flags(0), 0);
    for (stat, ty) in [
        (0x10, 0x01),
        (0x0020_0000, 0x02),
        (0x0001_0000, 0x04),
        (0x0080_0000, 0x08),
        (0x0100_0000, 0x10),
        (0x4000_0000, 0x20),
    ] {
        assert_eq!(type_flags(stat, 0), ty, "{stat:#x}");
    }
    assert_eq!(type_flags(0, 0x08), 0x40);
    // The rest have none.
    assert_eq!(type_flags(0x280166, 0), 0x02);
    assert_eq!(type_flags(0x0F, 0x37), 0);
}

#[tokio::test]
async fn immunity_to_players_or_creatures_from_static_flags_keeps_each_side_off() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let target = spawn(m, ga, BRUTE, 20, 3.0);
            let npc = spawn(m, ga, HELPER, 20, 3.0);
            let pet = spawn(m, ga, HELPER, 20, 3.0);
            m.creature_mut(pet).unwrap().owner = Some(ga);
            let mut out = Vec::new();
            for statics in [0, 0x20, 0x40, 0x200] {
                m.unit_fields_mut(target)
                    .unwrap()
                    .set_u32(UNIT_FIELD_FLAGS, unit_flags(statics));
                out.push([
                    combat::can_attack(m, ga, target),
                    combat::can_attack(m, pet, target),
                    combat::can_attack(m, npc, target),
                ]);
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        vec![
            [true, true, true],
            [false, false, true],
            [true, true, false],
            [false, false, false],
        ]
    );
}

#[tokio::test]
async fn an_unkillable_creature_keeps_its_last_point_of_health_against_blows_and_burns() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let health = |m: &Map, g: Guid| m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let unkillable = spawn(m, ga, UNKILLABLE, 20, 3.0);
            let ordinary = spawn(m, ga, BRUTE, 20, 3.0);
            m.unit_fields_mut(unkillable)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 100);
            m.unit_fields_mut(ordinary)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 100);
            combat::deal_damage(m, ga, unkillable, 1_000);
            let first = (
                health(m, unkillable),
                m.creature(unkillable).unwrap().alive(),
            );
            combat::deal_damage(m, ga, unkillable, 1_000);
            let again = health(m, unkillable);
            combat::deal_damage(m, ga, ordinary, 1_000);
            let dies = m.creature(ordinary).unwrap().alive();
            // Burned, it stays at one too.
            // Healed a little first, so the burn's ticks have to be held.
            m.unit_fields_mut(unkillable)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 400);
            spells::cast_triggered(m, ga, BURN, Some(unkillable));
            let burning =
                crate::game::map::auras::auras(m, unkillable).is_some_and(|a| a.has_spell(BURN));
            let now = m.now;
            for _ in 0..3 {
                if let Some(a) = crate::game::map::auras::auras_mut(m, unkillable) {
                    for h in a.holders.iter_mut().filter(|h| h.spell == BURN) {
                        for x in h.effects.iter_mut() {
                            x.next_tick = Some(now);
                        }
                    }
                }
                crate::game::map::auras::update(m);
            }
            let burned = (
                burning,
                health(m, unkillable),
                m.creature(unkillable).unwrap().alive(),
            );
            // A script's kill still kills it.
            combat::kill(m, ga, unkillable);
            (
                first,
                again,
                dies,
                burned,
                m.creature(unkillable).unwrap().alive(),
            )
        })
        .await
        .unwrap();
    assert_eq!(r.0, (1, true));
    assert_eq!(r.1, 1);
    assert!(!r.2);
    assert_eq!(r.3, (true, 1, true));
    assert!(!r.4);
}

#[tokio::test]
async fn a_creature_without_defense_meets_every_blow_with_no_skill() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let (normal, defenseless) = map
        .query(move |m| {
            level(m, ga, 20);
            let normal = spawn(m, ga, BRUTE, 20, 1.0);
            let defenseless = spawn(m, ga, DEFENSELESS, 20, 1.0);
            (
                combat::roll_for(m, ga, normal, Hand::Main).unwrap(),
                combat::roll_for(m, ga, defenseless, Hand::Main).unwrap(),
            )
        })
        .await
        .unwrap();
    assert_eq!(normal.victim_defense, 100);
    assert_eq!(defenseless.victim_defense, 0);
    assert_eq!(combat::roll_outcome(&normal, 0), Outcome::Miss);
    assert_ne!(combat::roll_outcome(&defenseless, 0), Outcome::Miss);
}
