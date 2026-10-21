//! Round 12's two calls the user made, as mangos-classic has them: a
//! disarmed creature with a weapon strikes for half (`Creature::
//! GetConditionalTotalPhysicalDamageModifier`), and crowd control takes the
//! place of another of its diminishing returns group, whoever cast it, its
//! returns going on (`Unit::RemoveNoStackAurasDueToAuraHolder`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::combat::Outcome;
use crate::game::map::spells;
use crate::game::tests_p3_aura2::{add_spells, aura_spell};
use crate::game::tests_p3_combat::{BRUTE, forced_swing, setup_with, spawn};

const DISARM: u32 = 97_200;
const STUN: u32 = 97_201;
const STUN_2: u32 = 97_202;
const POLYMORPH: u32 = 97_203;
const POLYMORPH_PIG: u32 = 97_204;
/// A slow of no mechanic: of no group.
const SLOW: u32 = 97_205;
const SLOW_2: u32 = 97_206;

/// A harmful aura at the target of `kind` and `mechanic`, for `ms`.
fn at_target(id: u32, ms: u32, kind: u32, mechanic: u32) -> SpellTemplate {
    let mut s = aura_spell(id, Some(ms), &[(kind, 0, 0)]);
    s.mechanic = mechanic;
    s.effects[0].target_a = 6;
    if kind == AURA_MOD_DECREASE_SPEED {
        s.effects[0].base_points = -51;
    }
    s
}

fn data() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    add_spells(
        &mut d,
        [
            at_target(DISARM, 10_000, AURA_MOD_DISARM, 3),
            at_target(STUN, 8_000, AURA_MOD_STUN, 12),
            at_target(STUN_2, 8_000, AURA_MOD_STUN, 12),
            at_target(POLYMORPH, 10_000, AURA_MOD_CONFUSE, 17),
            at_target(POLYMORPH_PIG, 10_000, AURA_MOD_CONFUSE, 17),
            at_target(SLOW, 10_000, AURA_MOD_DECREASE_SPEED, 0),
            at_target(SLOW_2, 10_000, AURA_MOD_DECREASE_SPEED, 0),
        ],
    );
    d
}

/// The spells of `g`'s auras, and of its aura fields.
fn on(m: &Map, g: Guid) -> (Vec<u32>, Vec<u32>) {
    let held = auras::auras(m, g).map_or_else(Vec::new, |a| {
        a.holders.iter().map(|h| h.spell).collect::<Vec<_>>()
    });
    let shown = m.unit_fields(g).map_or_else(Vec::new, |f| {
        (0..MAX_SLOTS as u16)
            .map(|i| f.get_u32(UNIT_FIELD_AURA + i))
            .filter(|s| *s != 0)
            .collect()
    });
    (held, shown)
}

fn lasts(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .and_then(|h| h.duration)
}

/// A creature with a weapon, disarmed by a player's spell, strikes for
/// half its 100 (mangos-classic's 0.5, Thistle Tea's 0.4); one without
/// strikes for all of it.
#[tokio::test]
async fn a_creature_disarmed_of_its_weapon_by_a_cast_strikes_for_half() {
    let (_realm, _a, g, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for armed in [true, false] {
                let brute = spawn(m, g, BRUTE, 20, 3.0);
                if armed {
                    m.unit_fields_mut(brute)
                        .unwrap()
                        .set_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY, 1000);
                }
                spells::cast_triggered(m, g, DISARM, Some(brute));
                let disarmed = auras::auras(m, brute).is_some_and(|a| a.has(AURA_MOD_DISARM));
                let f = &mut m.players.get_mut(&g).unwrap().fields;
                f.set_u32(UNIT_FIELD_RESISTANCES, 0);
                f.set_u32(UNIT_FIELD_HEALTH, 100_000);
                f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
                forced_swing(m, brute, g, Outcome::Hit);
                out.push((
                    disarmed,
                    100_000 - m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH),
                ));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(got, [(true, 50), (true, 100)]);
}

/// Two players' stuns on a creature: the second, of another spell, takes
/// the first's place, on the bar too, at half its length (the returns go
/// on); the third, the first spell again, the second's at a quarter; the
/// fourth is refused and the third stays.
#[tokio::test]
async fn a_stun_takes_the_place_of_another_whoever_cast_it_and_the_returns_go_on() {
    let (realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            let mut out = Vec::new();
            for (by, spell) in [(ga, STUN), (gb, STUN_2), (ga, STUN), (gb, STUN_2)] {
                spells::cast_triggered(m, by, spell, Some(brute));
                out.push((lasts(m, brute, spell), on(m, brute)));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            (Some(8_000), (vec![STUN], vec![STUN])),
            (Some(4_000), (vec![STUN_2], vec![STUN_2])),
            (Some(2_000), (vec![STUN], vec![STUN])),
            (None, (vec![STUN], vec![STUN])),
        ]
    );
}

/// A polymorph of another spell takes the first's place on a creature, at
/// its whole length: the group is a creature's too, its returns only
/// between players (mangos-classic's `setDiminishGroup` whatever the
/// levels). Two slows, of no group, stand together.
#[tokio::test]
async fn a_polymorph_replaces_another_at_full_length_and_auras_of_no_group_stand_together() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 5.0);
            spells::cast_triggered(m, ga, POLYMORPH, Some(brute));
            spells::cast_triggered(m, ga, POLYMORPH_PIG, Some(brute));
            let sheep = (lasts(m, brute, POLYMORPH_PIG), on(m, brute).0);
            let other = spawn(m, ga, BRUTE, 20, 5.0);
            spells::cast_triggered(m, ga, SLOW, Some(other));
            spells::cast_triggered(m, ga, SLOW_2, Some(other));
            (sheep, on(m, other).0)
        })
        .await
        .unwrap();
    assert_eq!(got.0, (Some(10_000), vec![POLYMORPH_PIG]));
    assert_eq!(got.1, [SLOW, SLOW_2]);
}
