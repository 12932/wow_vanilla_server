//! Round 12e: an area spell's line of sight, as mangos-classic's
//! `Spell::CheckTarget` draws it for each unit an area takes: from the
//! caster for an area about it (Arcane Explosion's, Frost Nova's) or a
//! cone, from the destination for one at a place (Flamestrike's), and not
//! at all for a spell that ignores it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, cast_fresh};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX2_IGNORE_LOS, SpellEffect};
use crate::game::harness::Client;
use crate::game::map::Map;
use crate::game::map::spells::Targets;

/// A burst about the caster (22, 15), one at a place (16), a cone (24), a
/// burst that ignores sight, and a heal of friends about the caster (30).
const BURST: u32 = 997_501;
const PLACE: u32 = 997_502;
const CONE: u32 = 997_503;
const BLIND_BURST: u32 = 997_504;
const MEND: u32 = 997_505;
const R20: u32 = 990_501;
const RANGE_40: u32 = 990_502;

fn spells(d: &mut GameData) {
    d.spells.radii.insert(R20, 20.0);
    d.spells.ranges.insert(RANGE_40, (0.0, 40.0));
    let area = |id: u32, a: u32, b: u32| {
        let mut s = bare(id, 6);
        effect(
            &mut s,
            0,
            SpellEffect {
                target_a: a,
                target_b: b,
                radius_index: R20,
                ..damage(50)
            },
        );
        s
    };
    let mut place = area(PLACE, 16, 0);
    place.range_index = RANGE_40;
    let mut cone = area(CONE, 24, 0);
    d.spells.cones.insert(CONE, 180);
    cone.range_index = 1;
    let mut blind = area(BLIND_BURST, 22, 15);
    blind.attributes[2] |= ATTR_EX2_IGNORE_LOS;
    let mut mend = bare(MEND, 1);
    effect(
        &mut mend,
        0,
        SpellEffect {
            effect: 10,
            base_points: 99,
            base_dice: 1,
            target_a: 22,
            target_b: 30,
            radius_index: R20,
            ..Default::default()
        },
    );
    insert(d, [area(BURST, 22, 15), place, cone, blind, mend]);
}

/// A wall across the line from `from` to `to`, halfway, 10 yards wide.
fn wall(m: &mut Map, from: Vector3d, to: Vector3d) {
    wall_of(m, from, to, 5.0);
}

/// A wall across the line from `from` to `to`, halfway, `half` yards to
/// each side.
fn wall_of(m: &mut Map, from: Vector3d, to: Vector3d, half: f32) {
    let mid = Vector3d {
        x: (from.x + to.x) / 2.0,
        y: (from.y + to.y) / 2.0,
        z: from.z,
    };
    let across = (to.y - from.y).atan2(to.x - from.x) + std::f32::consts::FRAC_PI_2;
    let end = |k: f32| Vector3d {
        x: mid.x + k * across.cos(),
        y: mid.y + k * across.sin(),
        z: mid.z,
    };
    m.spells.walls.push((end(-half), end(half)));
}

/// The hits of the next `SMSG_SPELL_GO` of `spell`.
async fn hits(c: &mut Client, spell: u32) -> Vec<Guid> {
    c.wait_for("SMSG_SPELL_GO", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => Some(g.hits.clone()),
        _ => None,
    })
    .await
}

/// Two kobolds 8 yards from a mage, one in front and one behind, a wall
/// between the mage and the one behind: a burst about the mage hits the
/// one it sees and not the one behind the wall, and its go lists only
/// that one; a burst that ignores sight hits both. A cone of 180 degrees
/// at the one in front, a wall put before it, hits nothing. A strike at a
/// place beside the one behind, the wall between the mage and both, hits
/// it (sight is from the place, not the caster), and with a wall between
/// the place and it, nothing. Before, every one of them hit every kobold
/// in reach, walls or not.
#[tokio::test]
async fn an_areas_units_are_those_its_anchor_sees() {
    let realm = fast_realm(spells);
    let (mut c, a, map) = human(&realm, "P12ES", "Seer", Class::Mage).await;
    let (front, back) = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[BURST, PLACE, CONE, BLIND_BURST, MEND]);
            let front = kit::dummy(m, a, 60, 8.0);
            let back = kit::dummy(m, a, 60, -8.0);
            let me = m.position_of(a).unwrap();
            let behind = m.position_of(back).unwrap();
            wall(m, me, behind);
            (front, back)
        })
        .await
        .unwrap();
    c.sees(front).await;
    c.sees(back).await;
    c.clear();
    let hurt = |m: &Map, g: Guid| kit::health(m, g) < 1_000_000;
    let (front_hurt, back_hurt) = map
        .query(move |m| {
            cast_fresh(m, a, BURST, Targets::default());
            (hurt(m, front), hurt(m, back))
        })
        .await
        .unwrap();
    assert!(front_hurt, "the burst hits the kobold the mage sees");
    assert!(!back_hurt, "but not the one behind the wall");
    assert_eq!(hits(&mut c, BURST).await, vec![front], "and its go says so");
    let back_hurt = map
        .query(move |m| {
            cast_fresh(m, a, BLIND_BURST, Targets::default());
            hurt(m, back)
        })
        .await
        .unwrap();
    assert!(back_hurt, "a burst that ignores sight hits through it");
    // A cone, the front one walled off: nothing.
    let (cone_front, place_back, walled_back) = map
        .query(move |m| {
            for g in [front, back] {
                kit::set_health(m, g, 1_000_000);
            }
            let me = m.position_of(a).unwrap();
            let ahead = m.position_of(front).unwrap();
            wall(m, me, ahead);
            cast_fresh(m, a, CONE, Targets::default());
            let cone_front = hurt(m, front);
            // A strike at a place 12 yards to the side of the one behind,
            // which the mage sees past the wall's end: the kobold is seen
            // from the place, though not from the mage.
            let behind = m.position_of(back).unwrap();
            let (fx, fy) = ((behind.x - me.x) / 8.0, (behind.y - me.y) / 8.0);
            let past = Vector3d {
                x: behind.x - fy * 12.0,
                y: behind.y + fx * 12.0,
                z: behind.z,
            };
            let at = Targets {
                dest: Some(past),
                ..Default::default()
            };
            cast_fresh(m, a, PLACE, at);
            let place_back = hurt(m, back);
            kit::set_health(m, back, 1_000_000);
            wall_of(m, past, behind, 2.0);
            cast_fresh(m, a, PLACE, at);
            (cone_front, place_back, hurt(m, back))
        })
        .await
        .unwrap();
    assert!(!cone_front, "a cone doesn't hit through a wall");
    assert!(place_back, "a strike at a place sees from the place");
    assert!(!walled_back, "and not through a wall there");
}

/// A heal of friends about the caster heals a friend it sees and not one
/// behind a wall, the caster itself whatever the walls.
#[tokio::test]
async fn a_heal_about_the_caster_doesnt_go_through_walls() {
    let realm = fast_realm(spells);
    let (_c, a, map) = human(&realm, "P12EH", "Mender", Class::Priest).await;
    let (_d, b, _) = human(&realm, "P12EI", "Seen", Class::Warrior).await;
    let (_e, x, _) = human(&realm, "P12EJ", "Hidden", Class::Warrior).await;
    let healed = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MEND]);
            let me = m.position_of(a).unwrap();
            let at = |d: f32| Vector3d {
                x: me.x + d,
                y: me.y,
                z: me.z,
            };
            m.players.get_mut(&b).unwrap().movement.position = at(6.0);
            m.players.get_mut(&x).unwrap().movement.position = at(-6.0);
            wall(m, me, at(-6.0));
            for g in [a, b, x] {
                kit::set_health(m, g, 10);
            }
            cast_fresh(m, a, MEND, Targets::default());
            [a, b, x].map(|g| kit::health(m, g) > 10)
        })
        .await
        .unwrap();
    assert_eq!(healed, [true, true, false], "the caster and the one seen");
}
