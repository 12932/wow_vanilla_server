//! Phase 3's fifth round of combat tests (docs/parity/matrix.csv's gaps in
//! Thistle Tea's `test/game/core/combat`): swing timers on a weapon change.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::AURA_MOD_HASTE;
use crate::game::map::combat;
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{self as base, *};

/// A slow axe, a quick dagger, a bow and a helm.
const AXE: u32 = 63_501;
const DAGGER: u32 = 63_502;
const BOW: u32 = 63_503;
const HELM: u32 = 63_504;

pub(crate) fn data() -> GameData {
    let mut d = base::data();
    for (entry, class, subclass, inventory_type, delay) in [
        (AXE, 2, 0, 13, 3400),
        (DAGGER, 2, 15, 13, 1500),
        (BOW, 2, 2, 15, 3000),
        (HELM, 4, 1, 1, 0),
    ] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class,
                subclass,
                name: format!("Item {entry}"),
                display_id: 1000,
                inventory_type,
                delay,
                stackable: 1,
                ..Default::default()
            },
        );
    }
    d
}

/// Give `g` one of `entry`, in the pack; where it is.
pub(crate) fn give(m: &mut Map, g: Guid, entry: u32) -> (u8, u8) {
    crate::game::map::inventory::add_item(m, g, entry, 1, false).unwrap();
    let p = &m.players[&g];
    let i = p.items.iter().rposition(|i| i.entry == entry).unwrap();
    (p.items[i].bag, p.items[i].slot)
}

/// A harmless brute `g` is fighting, and both hands ready to swing.
fn fight(m: &mut Map, g: Guid) -> Guid {
    let brute = spawn(m, g, BRUTE, 20, 3.0);
    let c = m.creature_mut(brute).unwrap();
    c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 0.0);
    c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 0.0);
    combat::engage(m, g, brute);
    ready(m, g, brute);
    brute
}

fn ready(m: &mut Map, g: Guid, victim: Guid) {
    let now = m.now;
    let c = m.combat_mut(g).unwrap();
    c.victim = victim;
    c.attacking = true;
    c.next_swing = Some(now);
    c.next_offhand = Some(now + Duration::from_millis(700));
}

fn swap(m: &mut Map, g: Guid, from: (u8, u8), slot: u8) {
    crate::game::map::inventory::move_item(m, g, from, (BAG_PLAYER, slot));
    // The weapon-switch lock would refuse the next change.
    if let Some(c) = m.caster_mut(g) {
        c.weapon_change_until = None;
    }
}

/// Thistle Tea's "a mainhand swap restarts its full new period and
/// retains the other hand", "another instance of the same weapon also
/// resets a ready swing", "the reset uses the hasted new period" and
/// "stopping melee attack cannot bypass the restarted weapon timer", as
/// mangos's `Player::EquipItem` has them: a weapon put in the main hand in
/// a fight puts its swing a whole period of the new weapon away, hasted,
/// and leaves the off hand's; one in the off hand restarts only it;
/// stopping and starting the attack again doesn't swing sooner.
#[tokio::test]
async fn a_weapon_put_in_a_hand_in_a_fight_starts_that_hands_swing_over() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            equip(m, ga, SWORD, SLOT_OFF_HAND);
            let brute = fight(m, ga);
            let axe = give(m, ga, AXE);
            swap(m, ga, axe, SLOT_MAIN_HAND);
            let axe_in = waits(m, ga);
            // The sword it took the place of, again.
            ready(m, ga, brute);
            let sword = m.players[&ga]
                .items
                .iter()
                .find(|i| i.entry == SWORD && !(i.bag == BAG_PLAYER && i.slot < 19))
                .map(|i| (i.bag, i.slot))
                .unwrap();
            swap(m, ga, sword, SLOT_MAIN_HAND);
            let sword_in = waits(m, ga);
            // Hasted by half again: two thirds of the period.
            aura_of(m, ga, 90_500, AURA_MOD_HASTE, 0, 50);
            crate::game::map::auras::recalc(m, ga);
            ready(m, ga, brute);
            let axe = m.players[&ga]
                .items
                .iter()
                .find(|i| i.entry == AXE)
                .map(|i| (i.bag, i.slot))
                .unwrap();
            swap(m, ga, axe, SLOT_MAIN_HAND);
            let hasted = waits(m, ga);
            // Stopped and started again: the swing waits still.
            combat::stop_attack(m, ga);
            combat::start_attack(m, ga, brute);
            let restarted = waits(m, ga);
            crate::game::map::auras::remove_spell(m, ga, 90_500, None);
            // A dagger in the off hand: only it starts over.
            ready(m, ga, brute);
            let dagger = give(m, ga, DAGGER);
            swap(m, ga, dagger, SLOT_OFF_HAND);
            let off = waits(m, ga);
            (axe_in, sword_in, hasted, restarted, off)
        })
        .await
        .unwrap();
    let (axe_in, sword_in, hasted, restarted, off) = r;
    assert_eq!(axe_in, (Some(3400), Some(700)), "the axe's whole period");
    assert_eq!(sword_in.0, Some(2000), "the same weapon again resets it");
    assert_eq!(hasted.0, Some(2266), "hasted");
    assert_eq!(restarted.0, Some(2266), "no sooner for stopping");
    assert_eq!(off, (Some(0), Some(1500)), "the off hand alone");
}

/// Thistle Tea's "peaceful, dead and non-weapon changes preserve timers"
/// and "ordinary haste and slow recomputation leaves an in-progress swing
/// intact": out of a fight a weapon change restarts nothing, nor does a
/// helm in one, nor a weapon taken off; a haste aura put on mid-swing
/// leaves the swing's deadline where it was.
#[tokio::test]
async fn a_change_out_of_a_fight_or_of_armor_or_haste_leaves_the_swing_alone() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            // Out of a fight: the timers stay as they were.
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            ready(m, ga, brute);
            let axe = give(m, ga, AXE);
            swap(m, ga, axe, SLOT_MAIN_HAND);
            let peaceful = waits(m, ga);
            let brute = fight(m, ga);
            let helm = give(m, ga, HELM);
            swap(m, ga, helm, 0);
            let armor = waits(m, ga);
            // The axe taken off into the pack.
            let free = (BAG_PLAYER, 30);
            crate::game::map::inventory::move_item(m, ga, (BAG_PLAYER, SLOT_MAIN_HAND), free);
            let off = waits(m, ga);
            ready(m, ga, brute);
            let later = m.now + Duration::from_millis(1200);
            if let Some(c) = m.combat_mut(ga) {
                c.next_swing = Some(later);
            }
            aura_of(m, ga, 90_501, AURA_MOD_HASTE, 0, 100);
            crate::game::map::auras::recalc(m, ga);
            let hasted = waits(m, ga);
            (peaceful, armor, off, hasted)
        })
        .await
        .unwrap();
    let (peaceful, armor, off, hasted) = r;
    assert_eq!(peaceful.0, Some(0), "out of a fight");
    assert_eq!(armor.0, Some(0), "a helm");
    assert_eq!(off.0, Some(0), "a weapon taken off");
    assert_eq!(hasted.0, Some(1200), "haste mid-swing");
}
