//! Round 7's creature tests, ported from Thistle Tea's
//! `test/game/core/creature` (docs/parity/matrix.csv): a script's weapons
//! slot by slot, a pet's react state over its template's, a passive order
//! ending its attack, the real scripts that set a react state, and a
//! civilian calling guards only on its victim.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::pets::{
    self, ACT_COMMAND, ACT_REACTION, COMMAND_ATTACK, REACT_AGGRESSIVE, REACT_PASSIVE,
};
use crate::game::map::{Map, MapHandle, scripts};
use crate::game::tests_guards::hardy;
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::{on, step};
use crate::game::tests_p3_entity_objects::until;

/// `n` of the map's updates from now.
async fn updates(map: &MapHandle, n: u64) {
    let from = on(map, |m| m.stats.updates).await;
    until(map, "the updates", move |m| m.stats.updates >= from + n).await;
}

const SWORD: u32 = 5_470;
const SHIELD: u32 = 5_471;
const BOW: u32 = 5_472;
const BLADE: u32 = 5_473;
const CHARM: u32 = 94_770;
/// A brute that looks about it, for the charm.
const LOOKOUT: u32 = 94_771;

/// Thistle Tea's `ScriptEquipment.apply` and `reset`: a script's weapon in
/// a slot shows its display and its item's class, subclass, material,
/// inventory type and sheath; a slot left keeps its own, one emptied shows
/// nothing; and a reset puts back what the creature spawned with.
#[tokio::test]
async fn a_scripts_weapons_change_keep_or_empty_each_slot_and_a_reset_brings_back_the_spawns() {
    let mut d = data();
    for (entry, display, class, subclass, inventory, sheath) in [
        (SWORD, 5_480, 2, 7, 13, 3),
        (SHIELD, 5_481, 4, 6, 14, 4),
        (BOW, 5_482, 2, 2, 15, 0),
        (BLADE, 5_483, 2, 7, 13, 3),
    ] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                display_id: display,
                class,
                subclass,
                material: 1,
                inventory_type: inventory,
                sheath,
                ..Default::default()
            },
        );
    }
    d.creature_equipment.insert(76, [SWORD, SHIELD, BOW]);
    d.creature_templates.get_mut(&BRUTE).unwrap().equipment_id = 76;
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        let g = spawn(m, ga, BRUTE, 20, 3.0);
        let items = data_items(m);
        m.creature_mut(g)
            .unwrap()
            .set_equipment(&items, [SWORD, SHIELD, BOW]);
        let read = |m: &Map| {
            let f = m.unit_fields(g).unwrap();
            (
                [0, 1, 2].map(|i| f.get_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + i)),
                [0, 1, 2, 3, 4, 5].map(|i| f.get_u32(UNIT_VIRTUAL_ITEM_INFO + i)),
            )
        };
        let before = read(m);
        let change = ScriptStep {
            dataint: [BLADE as i32, -1, 0, 0],
            ..step(19, [0; 4])
        };
        scripts::execute(m, &change, g, g);
        let changed = read(m);
        scripts::execute(m, &step(19, [1, 0, 0, 0]), g, g);
        (before, changed, read(m))
    })
    .await;
    let (before, changed, reset) = out;
    assert_eq!(changed.0, [5_483, 5_481, 0], "changed, kept, emptied");
    assert_eq!(
        changed.1[0].to_le_bytes(),
        [2, 7, 1, 13],
        "class, subclass, material, inventory type"
    );
    assert_eq!(changed.1[1].to_le_bytes(), [3, 0, 0, 0], "its sheath");
    assert_eq!(changed.1[2..4], before.1[2..4], "the shield's as it was");
    assert_eq!(changed.1[4..6], [0, 0], "the ranged slot's nothing");
    assert_eq!(reset, before, "back as it spawned");
}

fn data_items(m: &Map) -> std::sync::Arc<GameData> {
    m.realm.data.clone()
}

async fn charmed() -> (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    Guid,
    MapHandle,
    Guid,
    Guid,
) {
    let mut d = data();
    d.spells.spells.insert(
        CHARM,
        crate::game::tests_p3_entity::spell(
            CHARM,
            &[crate::game::tests_p3_entity::aura_effect(
                crate::game::map::auras::AURA_MOD_CHARM,
                0,
                0,
                6,
            )],
        ),
    );
    // A brute that looks about it (detection range 20), to charm.
    let mut lookout = d.creature_templates[&BRUTE].clone();
    lookout.entry = LOOKOUT;
    lookout.detection_range = 20.0;
    d.creature_templates.insert(LOOKOUT, lookout);
    let (realm, a, ga, map) = setup_with(d, Class::Warrior).await;
    let (pet, foe) = on(&map, move |m| {
        hardy(m, ga);
        let pet = spawn(m, ga, LOOKOUT, 20, 3.0);
        let charm = m.realm.data.spells.get(CHARM).cloned().unwrap();
        let _ = crate::game::map::auras::apply_spell(
            m,
            ga,
            pet,
            &charm,
            &[(0, 0)],
            crate::game::map::auras::Context::default(),
        );
        let foe = spawn(m, ga, BRUTE, 20, 6.0);
        (pet, foe)
    })
    .await;
    (realm, a, ga, map, pet, foe)
}

fn attacking(m: &Map, pet: Guid) -> Option<Guid> {
    m.combat(pet).filter(|c| c.attacking).map(|c| c.victim)
}

/// Thistle Tea's "uses the pet's reaction instead of its creature
/// template": a charmed creature, aggressive by its template, is defensive
/// by its own say: an enemy beside it is let be; made aggressive, it goes
/// for it.
#[tokio::test]
async fn a_pets_own_react_state_rules_over_its_templates() {
    let (_realm, a, _ga, map, pet, foe) = charmed().await;
    let react = on(&map, move |m| pets::state(m, pet).unwrap().react).await;
    assert_eq!(react, pets::REACT_DEFENSIVE);
    updates(&map, 20).await;
    assert_eq!(on(&map, move |m| attacking(m, pet)).await, None, "let be");
    a.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(REACT_AGGRESSIVE), ACT_REACTION),
        target: Guid::zero(),
    })
    .await;
    until(&map, "it goes for the enemy", move |m| {
        attacking(m, pet) == Some(foe)
    })
    .await;
}

/// Thistle Tea's "updates the pet owner once and preserves explicit attack
/// commands", where mangos-classic differs for the owner: a pet told to
/// attack and then made passive stops its attack and stays out of it
/// (mangos's `ACT_REACTION`'s `AttackStop` for passive, as Thistle Tea);
/// its master isn't told of the new state (mangos sends no
/// `SMSG_PET_MODE` for it: the client changed its own bar).
#[tokio::test]
async fn a_passive_order_ends_a_pets_attack_and_tells_its_master_nothing() {
    let (_realm, mut a, _ga, map, pet, foe) = charmed().await;
    a.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: foe,
    })
    .await;
    until(&map, "the attack", move |m| attacking(m, pet) == Some(foe)).await;
    a.clear();
    a.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(REACT_PASSIVE), ACT_REACTION),
        target: Guid::zero(),
    })
    .await;
    until(&map, "the attack stopped", move |m| {
        attacking(m, pet).is_none()
    })
    .await;
    updates(&map, 20).await;
    assert_eq!(
        on(&map, move |m| attacking(m, pet)).await,
        None,
        "out of it"
    );
    assert!(
        !crate::game::tests_p3_entity::gets_frame(&mut a, 0x17A, |_| true).await,
        "no SMSG_PET_MODE"
    );
}

/// Thistle Tea's "loads passive Thornling and defensive Timbermaw Ancestor
/// scripts": the real Thornling's EventAI script 1436201 makes it passive,
/// the Timbermaw Ancestor's 1572002 defensive.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_thornling_turns_passive_and_the_timbermaw_ancestor_defensive() {
    use crate::game::map::assistance::{Reaction, reaction};
    let db = crate::game::harness::real_data("db");
    if !db.join("vmangos.sqlite").exists() {
        return;
    }
    let data = crate::game::harness::load_real(&db);
    let (_realm, _a, ga, map) = setup_with(data, Class::Warrior).await;
    let out = on(&map, move |m| {
        [(1_436_201, 14_362), (1_572_002, 15_720)].map(|(script, entry)| {
            let steps = m.realm.data.scripts.ai_scripts.get(&script).cloned();
            let react = steps
                .as_ref()
                .and_then(|s| s.iter().find(|x| x.command == 59))
                .cloned();
            let g = spawn(m, ga, entry, 30, 3.0);
            let before = reaction(m, g);
            if let Some(step) = &react {
                scripts::execute(m, step, g, g);
            }
            (react.map(|s| s.datalong[0]), before, reaction(m, g))
        })
    })
    .await;
    assert_eq!(out[0].0, Some(0));
    assert_eq!(out[0].2, Reaction::Passive, "{:?}", out[0]);
    assert_eq!(out[1].0, Some(1));
    assert_eq!(out[1].2, Reaction::Defensive, "{:?}", out[1]);
}

/// Thistle Tea's "watches only its victim while fighting", as mangos's
/// `AttackStart` has it: a civilian in a fight calls guards on its victim
/// alone, another enemy beside it called on by none.
#[tokio::test]
async fn a_fighting_civilian_calls_guards_on_its_victim_alone() {
    use crate::game::tests_guards::{CITY_GUARD, CIVILIAN, STORMWIND, called, in_town, unit};
    let (realm, mut a, ga, map) =
        setup_with(crate::game::tests_guards::world(), Class::Warrior).await;
    let civ = unit(CIVILIAN);
    a.sees(civ).await;
    let (_b, gb) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    on(&map, move |m| {
        hardy(m, ga);
        hardy(m, gb);
        in_town(m, civ, STORMWIND);
        crate::game::map::ai::attacked(m, civ, ga);
    })
    .await;
    updates(&map, 20).await;
    let victims = on(&map, move |m| {
        called(m, CITY_GUARD)
            .iter()
            .map(|g| m.creature(*g).unwrap().combat.victim)
            .collect::<Vec<_>>()
    })
    .await;
    assert_eq!(victims, [ga], "one guard, on Alice; none on Bob");
}
