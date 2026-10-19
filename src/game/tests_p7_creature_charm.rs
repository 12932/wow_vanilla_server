//! Round 7's creature tests, ported from Thistle Tea's
//! `test/game/core/creature/charm_spells_test.exs` (docs/parity/matrix.csv):
//! the abilities a charmed or possessed creature offers, rolled slot by slot
//! from `creature_charm_spells` with their slots' cooldowns, never a charm
//! nor a passive on its bar, and where its spells go when no one is named.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::pets::CharmSpell;
use crate::game::data::spells::{ATTR_EX3_ALWAYS_HIT, CreatureSpell, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{AURA_MOD_CHARM, AURA_MOD_POSSESS, AURA_MOD_RESISTANCE};
use crate::game::map::pets::{self, ACT_COMMAND, COMMAND_ATTACK, Kind};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_guards::hardy;
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::{aura_effect, on, spell};
use crate::game::tests_p3_entity_objects::until;

/// A creature with charm abilities, and one without.
const THRALL: u32 = 4_740;
const PLAIN: u32 = 4_741;
/// Its slots' spells: a blow, a ward, a charm, a possession and a passive;
/// and the spell of its own list.
const BLOW: u32 = 94_740;
const WARD: u32 = 94_741;
const CHARM: u32 = 94_742;
const POSSESS: u32 = 94_743;
const PASSIVE: u32 = 94_744;
const OWN: u32 = 94_745;
const OWN_LIST: u32 = 4_740;

fn option(slot: u32, spell: u32, availability: f32, cooldown: (u32, u32)) -> CharmSpell {
    CharmSpell {
        slot,
        spell,
        availability,
        cooldown_min_ms: cooldown.0,
        cooldown_max_ms: cooldown.1,
    }
}

fn world() -> GameData {
    let mut d = data();
    for entry in [THRALL, PLAIN] {
        let mut t = template(entry, 7);
        t.spell_list_id = OWN_LIST;
        d.creature_templates.insert(entry, t);
    }
    d.spells.creature_spells.insert(
        OWN_LIST,
        vec![CreatureSpell {
            spell: OWN,
            probability: 100,
            ..Default::default()
        }],
    );
    let blow = crate::game::data::spells::SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut add = |s: SpellTemplate| {
        d.spells.spells.insert(s.id, s);
    };
    let mut b = spell(BLOW, &[blow]);
    b.dmg_class = 1;
    b.recovery_time = 2_000;
    b.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    add(b);
    add(spell(WARD, &[aura_effect(AURA_MOD_RESISTANCE, 5, 1, 1)]));
    add(spell(CHARM, &[aura_effect(AURA_MOD_CHARM, 0, 0, 6)]));
    add(spell(POSSESS, &[aura_effect(AURA_MOD_POSSESS, 0, 0, 6)]));
    let mut passive = spell(PASSIVE, &[aura_effect(AURA_MOD_RESISTANCE, 5, 1, 1)]);
    passive.attributes[0] |= 0x40;
    add(passive);
    add(spell(OWN, &[blow]));
    d.pets.charm_spells.insert(
        THRALL,
        vec![
            option(0, BLOW, 100.0, (9_000, 12_000)),
            option(1, CHARM, 50.0, (0, 0)),
            option(1, POSSESS, 50.0, (0, 0)),
            option(2, WARD, 100.0, (10_000, 10_000)),
            option(3, PASSIVE, 100.0, (0, 0)),
        ],
    );
    d
}

/// The spells on a bar.
fn bar_spells(bar: &[u32; 10]) -> Vec<u32> {
    bar.iter()
        .filter(|b| !matches!(*b >> 24, 0x06 | 0x07))
        .map(|b| b & 0x00FF_FFFF)
        .filter(|s| *s != 0)
        .collect()
}

/// Thistle Tea's `CharmSpells.pick`: a slot's roll walks its options'
/// running availability, the one whose share takes it picked, none past
/// them all.
#[test]
fn a_slots_roll_walks_its_options_running_availability() {
    let options = [option(0, 10, 22.0, (0, 0)), option(0, 11, 78.0, (0, 0))];
    let pick = |roll: f32| pets::pick_slot(options.iter(), roll).map(|o| o.spell);
    assert_eq!(pick(22.0), Some(10));
    assert_eq!(pick(22.5), Some(11));
    assert_eq!(pick(100.0), Some(11));
    let one = [option(0, 10, 45.0, (0, 0))];
    assert_eq!(pets::pick_slot(one.iter(), 60.0).map(|o| o.spell), None);
    assert_eq!(pets::pick_slot([].iter(), 10.0).map(|o| o.spell), None);
}

type Charmed = (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    MapHandle,
    Guid,
);

async fn charmed(possess: bool) -> Charmed {
    let (realm, a, ga, map) = setup_with(world(), Class::Warrior).await;
    let g = on(&map, move |m| {
        hardy(m, ga);
        let g = spawn(m, ga, THRALL, 20, 3.0);
        pets::charmed(m, ga, g, possess);
        g
    })
    .await;
    (realm, a, ga, map, g)
}

/// Thistle Tea's `CharmSpells.select`, `attach` and `control_spells`: a
/// charmed or possessed creature offers one spell a slot, in their order,
/// each recovering over its slot's time; its own list's spell isn't among
/// them, nor a charm or a possession; and its bar holds no passive.
#[tokio::test]
async fn a_controlled_creature_offers_a_spell_a_slot_with_its_slots_cooldown_and_no_charm() {
    for possess in [false, true] {
        let (_realm, _a, _ga, map, g) = charmed(possess).await;
        let (kind, spells, cooldowns, bar) = on(&map, move |m| {
            let s = pets::state(m, g).unwrap();
            (s.kind, s.spells.clone(), s.charm_cooldowns.clone(), s.bar)
        })
        .await;
        let expected = if possess {
            Kind::Possessed
        } else {
            Kind::Charmed
        };
        assert_eq!(kind, expected);
        assert_eq!(spells, [BLOW, WARD, PASSIVE], "slot 1 charms or possesses");
        assert_eq!(cooldowns.len(), 3);
        assert_eq!(cooldowns[0].0, BLOW);
        assert!((9_000..=12_000).contains(&cooldowns[0].1), "{cooldowns:?}");
        assert_eq!(cooldowns[1], (WARD, 10_000));
        assert_eq!(bar_spells(&bar), [BLOW, WARD], "on its bar, no passive");
    }
}

/// Thistle Tea's "adds the picked spells to the spellbook, overriding the
/// creature's own copy": the blow the creature is told to strike with
/// recovers over its slot's time, not its own two seconds.
#[tokio::test]
async fn a_charm_spell_recovers_over_its_slots_time() {
    let (_realm, a, ga, map, g) = charmed(false).await;
    let (victim, rolled) = on(&map, move |m| {
        let victim = spawn(m, ga, BRUTE, 20, 2.0);
        let rolled = pets::state(m, g).unwrap().charm_cooldowns[0].1;
        (victim, rolled)
    })
    .await;
    a.send(CMSG_PET_ACTION {
        pet: g,
        data: pets::button(BLOW, 0xC1),
        target: victim,
    })
    .await;
    until(&map, "the blow's cooldown", move |m| {
        m.caster(g).is_some_and(|c| c.cooldowns.contains_key(&BLOW))
    })
    .await;
    let left = on(&map, move |m| {
        let now = m.now;
        m.caster(g).unwrap().cooldowns[&BLOW].saturating_duration_since(now)
    })
    .await;
    assert!(
        left > Duration::from_millis(u64::from(rolled) - 500)
            && left <= Duration::from_millis(u64::from(rolled)),
        "{left:?} of {rolled} ms"
    );
}

/// Thistle Tea's `CharmSpells.entries` (and the pet bar's casts): told to
/// use its spells with no one named, a charmed creature strikes the one it
/// is attacking and wards itself.
#[tokio::test]
async fn a_charmed_creatures_blow_goes_to_its_victim_and_its_ward_to_itself() {
    let (_realm, a, ga, map, g) = charmed(false).await;
    let victim = on(&map, move |m| spawn(m, ga, BRUTE, 20, 2.0)).await;
    a.send(CMSG_PET_ACTION {
        pet: g,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: victim,
    })
    .await;
    until(&map, "it attacks", move |m| {
        m.combat(g)
            .is_some_and(|c| c.attacking && c.victim == victim)
    })
    .await;
    for s in [BLOW, WARD] {
        a.send(CMSG_PET_ACTION {
            pet: g,
            data: pets::button(s, 0xC1),
            target: Guid::zero(),
        })
        .await;
    }
    until(&map, "the ward on it", move |m| {
        crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(WARD))
    })
    .await;
    let struck = on(&map, move |m| {
        m.unit_fields(victim).unwrap().get_u32(UNIT_FIELD_HEALTH) < 100_000
    })
    .await;
    assert!(struck, "the blow on its victim");
}

/// Thistle Tea's "offers nothing without charm data", where mangos-classic
/// differs: a creature with no charm abilities offers its own spell list,
/// as mangos's `GetCharmSpells` gives a charmed creature its list's spells.
#[tokio::test]
async fn a_creature_without_charm_abilities_offers_its_own_list() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let spells = on(&map, move |m: &mut Map| {
        hardy(m, ga);
        let g = spawn(m, ga, PLAIN, 20, 3.0);
        pets::charmed(m, ga, g, false);
        pets::state(m, g).unwrap().spells.clone()
    })
    .await;
    assert_eq!(spells, [OWN]);
}
