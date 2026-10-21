//! Round 11j's pet tests: what a hunter learns of a pet, a stored pet's
//! spells and a demon's return, and a talent's pet aura (Thistle Tea's
//! `pet_discovery_test`, `summon_test`, `state_test` and `talents_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::pets::PetAbility;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::MapHandle;
use crate::game::map::pets::{self, Kind};
use crate::game::tests_p3_entity::on;
use crate::game::tests_pets::{self as tp, GORE, TRAIN_GORE};

const THICK_HIDE: u32 = 9201;
const TRAIN_THICK_HIDE: u32 = 9202;
/// A boar's attack, which nothing teaches.
const TUSK: u32 = 9203;

/// The pets' world, with a boar's family passive (+5 armor) its hunter
/// learns to teach, and Gore, if `gore_innate`, a spell of the wild boar's
/// it comes with too.
fn world(gore_innate: bool) -> GameData {
    let mut d = tp::data();
    if gore_innate {
        // What the wild boar casts, it keeps tamed.
        d.pets.create_spells.insert(tp::BOAR, vec![GORE]);
    }
    d.pets.abilities.get_mut(&211).unwrap().push(PetAbility {
        spell: THICK_HIDE,
        skill: 211,
        acquire: 2,
        ..Default::default()
    });
    d.pets.teaching.insert(THICK_HIDE, TRAIN_THICK_HIDE);
    let none = Default::default();
    let mut hide = tp::effect(6, 5, 1);
    hide.aura = 22;
    hide.misc_value = 1;
    let mut passive = tp::spell(THICK_HIDE, "Thick Hide", [hide, none, none]);
    passive.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    let mut teach = tp::effect(36, 0, 5);
    teach.trigger_spell = THICK_HIDE;
    let recipe = tp::spell(TRAIN_THICK_HIDE, "Thick Hide", [teach, none, none]);
    let tusk = tp::spell(TUSK, "Tusk", [tp::effect(2, 4, 6), none, none]);
    d.spells.spells.insert(THICK_HIDE, passive);
    d.spells.spells.insert(TRAIN_THICK_HIDE, recipe);
    d.spells.spells.insert(TUSK, tusk);
    d
}

async fn tamed(
    d: GameData,
) -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
) {
    let realm = crate::game::harness::realm_with(d);
    let (mut h, g, map) = tp::enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    tp::learn(&map, g, &[tp::TAME_BEAST, tp::CALL_PET, tp::DISMISS_PET]);
    let boar = tp::place(&map, g, tp::BOAR, 0x70_0001, 4.0).await;
    h.sees(boar).await;
    tp::cast(&h, tp::TAME_BEAST, Some(boar)).await;
    let pet = tp::pet_bar(&mut h).await;
    assert!(!pet.is_zero(), "the tamed boar");
    (realm, h, g, map, pet)
}

/// Thistle Tea's "learns innate passive recipes on attachment without
/// teaching active recipes": a boar tamed with its family's passive and
/// an active spell of its own teaches its hunter the passive's recipe, not
/// the active one's (mangos's `InitPetCreateSpells` learns the passives'
/// alone); the boar knows both.
#[tokio::test]
async fn a_tamed_pet_teaches_its_passives_recipe_not_its_active_ones() {
    let (_realm, _h, g, map, pet) = tamed(world(true)).await;
    let known = on(&map, move |m| {
        let s = pets::state(m, pet).unwrap();
        let p = &m.players[&g];
        (
            s.spells.contains(&THICK_HIDE),
            s.spells.contains(&GORE),
            p.spells.contains(&TRAIN_THICK_HIDE),
            p.spells.contains(&TRAIN_GORE),
        )
    })
    .await;
    assert_eq!(known, (true, true, true, false));
}

/// Thistle Tea's "learns the recipe once, not the pet attack, and retains
/// it after dismissal": a pet's use of Gore teaches its hunter Gore's
/// recipe once, told once; uses after teach nothing more; its attack,
/// which no recipe teaches, teaches nothing; dismissed, the recipe stays.
#[tokio::test]
async fn a_pets_use_teaches_its_recipe_once_and_it_is_kept_after_dismissal() {
    let (_realm, mut h, g, map, pet) = tamed(world(false)).await;
    let before = on(&map, move |m| m.players[&g].spells.len()).await;
    // Its attack, used and used: nothing learnt.
    on(&map, move |m| {
        m.spells.rolls = Some(true);
        let tusk = m.realm.data.spells.get(TUSK).cloned().unwrap();
        for _ in 0..300 {
            pets::before_effects(m, pet, &tusk, &Default::default());
        }
    })
    .await;
    assert_eq!(on(&map, move |m| m.players[&g].spells.len()).await, before);
    h.clear();
    let learnt = on(&map, move |m| {
        if let Some(s) = pets::state_mut(m, pet) {
            s.spells.push(GORE);
        }
        let gore = m.realm.data.spells.get(GORE).cloned().unwrap();
        let mut uses = 0;
        while uses < 2000 && !m.players[&g].spells.contains(&TRAIN_GORE) {
            pets::before_effects(m, pet, &gore, &Default::default());
            uses += 1;
        }
        let known = m.players[&g].spells.contains(&TRAIN_GORE);
        // And used as often again.
        for _ in 0..500 {
            pets::before_effects(m, pet, &gore, &Default::default());
        }
        let count = m.players[&g]
            .spells
            .iter()
            .filter(|s| **s == TRAIN_GORE)
            .count();
        (known, count)
    })
    .await;
    assert_eq!(learnt, (true, 1), "learnt, once");
    h.wait_for("SMSG_LEARNED_SPELL", |m| match m {
        ServerOpcodeMessage::SMSG_LEARNED_SPELL(l) if l.id == TRAIN_GORE => Some(()),
        _ => None,
    })
    .await;
    let again = h
        .gets(Duration::from_millis(200), |m| match m {
            ServerOpcodeMessage::SMSG_LEARNED_SPELL(l) if l.id == TRAIN_GORE => Some(()),
            _ => None,
        })
        .await;
    assert!(again.is_none(), "told once");
    tp::cast(&h, tp::DISMISS_PET, None).await;
    assert_eq!(tp::pet_bar(&mut h).await, Guid::zero(), "dismissed");
    assert!(on(&map, move |m| m.players[&g].spells.contains(&TRAIN_GORE)).await);
}

/// Log `c` out and in again, its pet's fight over first.
async fn relog(c: &mut Client, g: Guid, map: &MapHandle) {
    on(map, move |m| {
        crate::game::map::combat::leave_combat(m, g);
        if let Some(pet) = m.players[&g].owned.pet {
            crate::game::map::combat::leave_combat(m, pet);
        }
    })
    .await;
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    c.login(g).await;
}

fn armor(m: &crate::game::map::Map, pet: Guid) -> u32 {
    m.creature(pet)
        .map_or(0, |c| c.fields.get_u32(UNIT_FIELD_RESISTANCES))
}

/// Thistle Tea's "restores only family passives after untraining and
/// retains the reset price history": a boar trained in Gore, untrained
/// (its family's passive kept), and back after its hunter's logout knows
/// its family's passive alone, wears it (its armor as before), and the
/// untraining's price and time are its hunter's still (mangos's
/// `m_resetTalentsCost` and `m_resetTalentsTime` saved with the pet).
#[tokio::test]
async fn an_untrained_pet_comes_back_with_its_familys_passive_alone_and_the_price_kept() {
    let (_realm, mut h, g, map, pet) = tamed(world(false)).await;
    let (worn, hide) = on(&map, move |m| {
        if let Some(s) = pets::state_mut(m, pet) {
            s.spells.push(GORE);
        }
        let p = m.players.get_mut(&g).unwrap();
        p.set_money(100_000);
        (
            armor(m, pet),
            crate::game::map::auras::auras(m, pet).is_some_and(|a| a.has_spell(THICK_HIDE)),
        )
    })
    .await;
    assert!(hide, "Thick Hide worn");
    h.send(CMSG_PET_UNLEARN { pet }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut h).await;
    let (spells, price) = on(&map, move |m| {
        let p = &m.players[&g];
        (
            pets::state(m, pet).map(|s| s.spells.clone()),
            (p.row.state.pets.untrain_cost, p.row.state.pets.untrain_at),
        )
    })
    .await;
    assert_eq!(spells, Some(vec![THICK_HIDE]), "untrained");
    assert!(price.0 > 0 && price.1 > 0, "{price:?}");
    relog(&mut h, g, &map).await;
    let back = tp::pet_bar(&mut h).await;
    assert!(!back.is_zero(), "back after the logout");
    let mut seen = None;
    for _ in 0..100 {
        seen = on(&map, move |m| {
            let p = &m.players[&g];
            Some((
                pets::state(m, back)?.spells.clone(),
                crate::game::map::auras::auras(m, back)?.has_spell(THICK_HIDE),
                armor(m, back),
                (p.row.state.pets.untrain_cost, p.row.state.pets.untrain_at),
            ))
        })
        .await;
        if seen.as_ref().is_some_and(|s| s.1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(seen, Some((vec![THICK_HIDE], true, worn, price)));
}

/// Thistle Tea's "replaces the active reference with stable restore
/// state": a warlock's imp out at her logout is kept as her demon (kind,
/// entry, number, out), and is summoned again at her login.
#[tokio::test]
async fn a_warlocks_demon_out_at_logout_is_kept_and_back_at_login() {
    let realm = crate::game::harness::realm_with(tp::data());
    let (mut w, g, map) = tp::enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    tp::learn(&map, g, &[tp::SUMMON_IMP]);
    tp::cast(&w, tp::SUMMON_IMP, None).await;
    let imp = tp::pet_bar(&mut w).await;
    assert!(!imp.is_zero(), "the imp");
    let number = on(&map, move |m| {
        m.creature(imp)
            .map(|c| c.fields.get_u32(UNIT_FIELD_PETNUMBER))
    })
    .await;
    relog(&mut w, g, &map).await;
    let kept = on(&map, move |m| {
        let s = &m.players[&g].row.state.pets;
        (
            s.active,
            s.current.as_ref().map(|r| (r.kind, r.entry, r.number)),
        )
    })
    .await;
    assert_eq!(
        kept,
        (
            true,
            // A demon's kind.
            Some((2, tp::IMP, number.unwrap()))
        )
    );
    let back = tp::pet_bar(&mut w).await;
    assert!(!back.is_zero(), "summoned again");
    let (kind, entry, n) = on(&map, move |m| {
        (
            tp::pet_kind(m, back),
            m.creature(back).map(|c| c.entry),
            m.creature(back)
                .map(|c| c.fields.get_u32(UNIT_FIELD_PETNUMBER)),
        )
    })
    .await;
    assert_eq!((kind, entry, n), (Some(Kind::Demon), Some(tp::IMP), number));
}

const BOND: u32 = 97_300;
const BOND_1: u32 = 97_301;
const BOND_2: u32 = 97_302;
const BOND_IMP_1: u32 = 97_303;
const BOND_IMP_2: u32 = 97_304;
const WARLOCK_TAB: u32 = 302;

/// Thistle Tea's "replaces a superseded talent's pet aura link": a
/// warlock's talent whose first rank gives her imp an aura
/// (`spell_pet_auras`), learnt to its second, takes the first's aura off
/// the imp and gives it the second's (`inherit::sync`, within a second).
/// The aura's caster is the warlock (TT and mangos make it the pet's own),
/// which her client doesn't see.
#[tokio::test]
async fn a_talents_next_rank_swaps_its_pet_aura() {
    use crate::game::data::spells::TalentInfo;
    let mut d = tp::data();
    let none = Default::default();
    for (rank, aura, amount) in [(BOND_1, BOND_IMP_1, -10), (BOND_2, BOND_IMP_2, -20)] {
        let mut dummy = tp::effect(6, 0, 1);
        dummy.aura = 4;
        let mut talent = tp::spell(rank, "Demonic Bond", [dummy, none, none]);
        talent.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
        d.spells.spells.insert(rank, talent);
        let mut threat = tp::effect(6, amount - 1, 1);
        threat.aura = 10;
        threat.misc_value = 127;
        d.spells
            .spells
            .insert(aura, tp::spell(aura, "Demonic Bond", [threat, none, none]));
        d.pets.owner_auras.push((rank, tp::IMP, aura));
    }
    d.spells.talents.insert(
        BOND,
        TalentInfo {
            id: BOND,
            tab: WARLOCK_TAB,
            row: 0,
            column: 0,
            ranks: [BOND_1, BOND_2, 0, 0, 0],
            depends_on: 0,
            depends_rank: 0,
        },
    );
    d.spells.talent_tab_classes.insert(WARLOCK_TAB, 0x100);
    let realm = crate::game::harness::realm_with(d);
    let (mut w, g, map) = tp::enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    tp::learn(&map, g, &[tp::SUMMON_IMP]);
    on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        crate::game::map::spells::learn_talent(m, g, BOND, 0);
    })
    .await;
    tp::cast(&w, tp::SUMMON_IMP, None).await;
    let imp = tp::pet_bar(&mut w).await;
    let has = move |m: &crate::game::map::Map, spell: u32| {
        crate::game::map::auras::auras(m, imp).is_some_and(|a| a.has_spell(spell))
    };
    let mut first = false;
    for _ in 0..150 {
        first = on(&map, move |m| has(m, BOND_IMP_1)).await;
        if first {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(first, "the first rank's aura on the imp");
    on(&map, move |m| {
        crate::game::map::spells::learn_talent(m, g, BOND, 1);
    })
    .await;
    let mut swapped = (true, false, true);
    for _ in 0..150 {
        swapped = on(&map, move |m| {
            (
                has(m, BOND_IMP_1),
                has(m, BOND_IMP_2),
                m.players[&g].spells.contains(&BOND_1),
            )
        })
        .await;
        if swapped == (false, true, false) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(swapped, (false, true, false), "the second rank's alone");
}
