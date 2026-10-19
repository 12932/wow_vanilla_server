//! Round 7, against Thistle Tea's `test/game/core/loot/pickpocket_test.exs`
//! (docs/parity/matrix.csv): what a rogue's stealth and peace come to as
//! they pick a pocket, or are caught at it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::spells::SpellEffect;
use crate::game::data::{GameData, ItemTemplate, LootEntry};
use crate::game::map::{Map, auras, professions, spells};
use crate::game::tests_p3_entity::on;

const PICK: u32 = 997_801;
const STEALTH: u32 = 997_802;
const INVISIBLE: u32 = 997_803;
const COIN: u32 = 997_810;
const POCKETS: u32 = 997_820;

fn pockets(d: &mut GameData) {
    d.spells.ranges.insert(1, (0.0, 30.0));
    if let Some(k) = d.creature_templates.get_mut(&KOBOLD) {
        k.pickpocket_loot_id = POCKETS;
    }
    d.items.insert(
        COIN,
        ItemTemplate {
            entry: COIN,
            name: "Coin".into(),
            display_id: 1000,
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    d.loot.pickpocket.insert(
        POCKETS,
        vec![LootEntry {
            item: COIN,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    // Pick Pocket (921): an ability, cast from stealth, at a unit, whose
    // failure breaks stealth (its attributes).
    let mut pick = bare(PICK, 0);
    pick.attributes[0] = 0x3_0010;
    pick.attributes[1] = 0x20_1020;
    effect(
        &mut pick,
        0,
        SpellEffect {
            effect: professions::EFFECT_PICKPOCKET,
            target_a: 6,
            ..Default::default()
        },
    );
    // Stealth and Lesser Invisibility (1784, 66): broken by an action
    // (their aura interrupt flags), of their dispel types.
    let mut stealth = bare(STEALTH, 0);
    stealth.duration_index = LASTS_LONG;
    stealth.dispel = 5;
    stealth.aura_interrupt_flags = 0x1_3C03;
    effect(&mut stealth, 0, aura(auras::AURA_MOD_STEALTH, 0, 1, 1));
    let mut invisible = bare(INVISIBLE, 0);
    invisible.duration_index = LASTS_LONG;
    invisible.dispel = 6;
    invisible.aura_interrupt_flags = 0x1_3C03;
    effect(
        &mut invisible,
        0,
        aura(auras::AURA_MOD_INVISIBILITY, 0, 100, 1),
    );
    insert(d, [pick, stealth, invisible]);
}

/// The rogue's state after a pick: stealthed, invisible, in combat, and
/// what they loot; and whether the kobold fights them.
fn after(m: &Map, g: Guid, kobold: Guid) -> (bool, bool, bool, Option<Guid>, bool) {
    let in_combat = |u: Guid| {
        m.unit_fields(u).is_some_and(|f| {
            f.has_flag(
                crate::game::fields::UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
        })
    };
    (
        kit::has(m, g, STEALTH),
        kit::has(m, g, INVISIBLE),
        in_combat(g),
        m.players[&g].looting,
        in_combat(kobold) || m.combat(kobold).is_some_and(|c| c.attacking),
    )
}

/// Thistle Tea's `Casting.advance/2` "cast preparation preserves stealth
/// but still interrupts invisibility", "a successful cast opens pockets
/// while preserving stealth and peace" and "a resisted cast breaks stealth
/// and delivers the miss without opening loot": a pick that lands keeps
/// the rogue's stealth (Pick Pocket is allowed while stealthed) but ends
/// their invisibility, opens the pockets and starts no fight; one the mark
/// resists opens nothing, and its failure breaks the rogue's stealth and
/// turns the mark on them (mangos's `CasterHitTargetWithSpell` for
/// `SPELL_ATTR_EX_FAILURE_BREAKS_STEALTH`).
#[tokio::test]
async fn a_picked_pocket_keeps_stealth_and_peace_and_a_caught_pick_breaks_stealth() {
    let realm = fast_realm(pockets);
    let (_c, g, map) = human(&realm, "P7L1", "Picker", Class::Rogue).await;
    let landed = on(&map, move |m| {
        kit::ready(m, g, 60, &[PICK]);
        let kobold = kit::dummy(m, g, 60, 3.0);
        spells::cast_triggered(m, g, STEALTH, Some(g));
        spells::cast_triggered(m, g, INVISIBLE, Some(g));
        let before = (kit::has(m, g, STEALTH), kit::has(m, g, INVISIBLE));
        let cast = kit::cast_fresh(m, g, PICK, kit::at(kobold));
        (before, cast, after(m, g, kobold))
    })
    .await;
    assert_eq!(landed.0, (true, true), "stealthed and invisible");
    assert!(landed.1, "picked");
    let (stealth, invisible, fighting, looting, mark_fights) = landed.2;
    assert!(stealth, "stealth kept");
    assert!(!invisible, "invisibility ended by the cast");
    assert!(looting.is_some(), "the pockets open");
    assert!(!fighting && !mark_fights, "no fight");
    // Against a mark far above them, with the rolls rolled, it is resisted.
    let caught = on(&map, move |m| {
        m.spells.rolls = Some(true);
        for _ in 0..50 {
            if let Some(l) = m.players[&g].looting {
                crate::game::map::loot::release(m, g, l);
            }
            let mark = kit::dummy(m, g, 63, 3.0);
            if let Some(c) = m.creature_mut(mark) {
                c.fields.set_u32(crate::game::fields::UNIT_FIELD_LEVEL, 75);
            }
            spells::cast_triggered(m, g, STEALTH, Some(g));
            kit::cast_fresh(m, g, PICK, kit::at(mark));
            let now = after(m, g, mark);
            if now.3.is_none() {
                m.spells.rolls = None;
                return Some(now);
            }
        }
        None
    })
    .await;
    let (stealth, _, fighting, looting, mark_fights) = caught.expect("a pick resisted");
    assert_eq!(looting, None, "nothing opened");
    assert!(!stealth, "the failure breaks stealth");
    assert!(mark_fights && fighting, "the mark turns on the rogue");
}
