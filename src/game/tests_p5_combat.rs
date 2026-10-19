//! Phase 3's fifth round of combat tests (docs/parity/matrix.csv's gaps in
//! Thistle Tea's `test/game/core/combat`): swing timers on a weapon change.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
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

/// Food on `g`: an aura that standing ends.
fn food(m: &mut Map, g: Guid, id: u32) {
    let mut effects = [crate::game::data::spells::SpellEffect::default(); 3];
    effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura: crate::game::map::auras::AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    let s = crate::game::data::spells::SpellTemplate {
        id,
        name: "Food".into(),
        equipped_item_class: -1,
        aura_interrupt_flags: crate::game::map::auras::INTERRUPT_NOT_SEATED,
        duration_index: 0,
        effects,
        ..Default::default()
    };
    let _ = crate::game::map::auras::apply_spell(
        m,
        g,
        g,
        &s,
        &[(0, 0)],
        crate::game::map::auras::Context::default(),
    );
}

fn posture(m: &Map, g: Guid) -> (u8, bool) {
    let p = &m.players[&g];
    let fed = crate::game::map::auras::auras(m, g)
        .is_some_and(|a| a.holders.iter().any(|h| h.spell == 90_510));
    (p.stand_state(), fed)
}

/// Thistle Tea's "direct damage stands all seated poses and removes
/// sitting-only auras once" and "periodic damage ... retain their
/// posture", as mangos's `DealDamage` has it since 1.5: a player sitting,
/// sleeping or kneeling who is struck (a swing that misses too, or a
/// spell) stands, told so, and their food ends; a periodic tick leaves
/// them as they were; god mode, nothing. (Food sits a player down, and
/// sleeping ends it.)
#[tokio::test]
async fn a_seated_player_struck_stands_and_their_food_ends_but_a_tick_leaves_them_seated() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let brute = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 2.0);
            let c = m.creature_mut(brute).unwrap();
            c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 0.0);
            c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 0.0);
            brute
        })
        .await
        .unwrap();
    for (state, outcome) in [
        (UnitStandState::Sit, combat::Outcome::Miss),
        (UnitStandState::Sleep, combat::Outcome::Hit),
        (UnitStandState::Kneel, combat::Outcome::Dodge),
    ] {
        // The food sits them; then the posture: kneeling keeps the meal,
        // lying down to sleep ends it (mangos's `IsSeatedState`).
        map.query(move |m| food(m, ga, 90_510)).await.unwrap();
        a.send(CMSG_STANDSTATECHANGE {
            animation_state: state,
        })
        .await;
        a.wait_for("the posture", |m| match m {
            ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(u) if u.state == state => Some(()),
            _ => None,
        })
        .await;
        let (seated, struck) = map
            .query(move |m| {
                let seated = posture(m, ga);
                forced_swing(m, brute, ga, outcome);
                (seated, posture(m, ga))
            })
            .await
            .unwrap();
        let fed = state != UnitStandState::Sleep;
        assert_eq!(seated, (state.as_int(), fed), "{state:?} after food");
        assert_eq!(struck, (0, false), "{state:?} struck: {outcome:?}");
        a.wait_for("standing", |m| match m {
            ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(u) if u.state == UnitStandState::Stand => {
                Some(())
            }
            _ => None,
        })
        .await;
    }
    // A tick of a periodic aura: still seated, still fed; a spell's blow
    // stands them.
    a.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Sit,
    })
    .await;
    a.wait_for("sitting", |m| match m {
        ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(u) if u.state == UnitStandState::Sit => {
            Some(())
        }
        _ => None,
    })
    .await;
    let (ticked, spell) = map
        .query(move |m| {
            food(m, ga, 90_510);
            combat::deal_periodic_damage(m, brute, ga, 5, 1, None, false);
            let ticked = posture(m, ga);
            let s = m.realm.data.spells.get(STRIKE).cloned().unwrap();
            crate::game::map::spells::spell_damage(m, brute, ga, &s, 5);
            (ticked, posture(m, ga))
        })
        .await
        .unwrap();
    assert_eq!(ticked, (1, true), "a tick");
    assert_eq!(spell, (0, false), "a spell");
}

/// Thistle Tea's "seated physical abilities crit before avoidance" and
/// "cannot-crit ... retain precedence over seated crits": an ability on a
/// seated player crits ahead of the dodge it would have rolled, unless it
/// can't crit; a player with no chance at all to crit doesn't (mangos's
/// `CanCrit`, a chance of at least 0.005%), where Thistle Tea crits anyway.
#[test]
fn an_ability_on_a_seated_player_crits_before_avoidance_unless_it_cant_crit() {
    let seated = combat::Roll {
        victim_is_player: true,
        standing: false,
        dodge: 100.0,
        ..even(60)
    };
    // The roll would land in the dodge.
    assert_eq!(
        combat::roll_special(&seated, 9_999, 9_999),
        combat::Outcome::Crit
    );
    let cant = combat::Roll {
        can_crit: false,
        ..seated
    };
    assert_ne!(combat::roll_special(&cant, 9_999, 0), combat::Outcome::Crit);
    let no_chance = combat::Roll {
        crit: 0.0,
        ..seated
    };
    assert_ne!(
        combat::roll_special(&no_chance, 9_999, 0),
        combat::Outcome::Crit
    );
    // A creature's base crit stands in: it crits a seated player.
    let creature = combat::Roll {
        attacker_is_player: false,
        crit: 0.0,
        ..seated
    };
    assert_eq!(
        combat::roll_special(&creature, 9_999, 9_999),
        combat::Outcome::Crit
    );
    let standing = combat::Roll {
        standing: true,
        ..seated
    };
    assert_eq!(
        combat::roll_special(&standing, 9_999, 9_999),
        combat::Outcome::Dodge
    );
}
