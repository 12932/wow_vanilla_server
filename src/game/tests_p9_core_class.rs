//! Round 9, the classes on made-up data: Thistle Tea's `hunter_test.exs`
//! (Feed Pet's amount), `shaman_test.exs` (Flametongue's formula) and
//! `self_resurrection_test.exs` (a forged Reincarnation).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras::AURA_PERIODIC_ENERGIZE;
use crate::game::map::death::{self, Death};
use crate::game::map::{Map, MapHandle, pets};

const FEED_PET: u32 = 97_801;
const FEED_EFFECT: u32 = 97_802;
const FOOD: u32 = 97_803;

/// Thistle Tea's "overrides the DBC trigger aura amount without changing
/// unrelated effects": Feed Pet's trigger puts on the pet the food's
/// benefit for its level, 35,000 of happiness for food of its level, not
/// the trigger's own 9,999 (and no roll); the trigger's other effects are
/// none of Feed Pet's (the real trigger, 1539, has only the energize).
#[tokio::test]
async fn feed_pet_gives_the_pet_the_foods_benefit_for_its_level() {
    let mut d = crate::game::tests_pets::data();
    let mut feed = SpellTemplate {
        id: FEED_PET,
        name: "Feed Pet".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    feed.effects[0] = SpellEffect {
        effect: pets::EFFECT_FEED_PET,
        trigger_spell: FEED_EFFECT,
        target_a: 1,
        ..Default::default()
    };
    let mut effect = SpellTemplate {
        id: FEED_EFFECT,
        name: "Feed Pet Effect".into(),
        equipped_item_class: -1,
        duration_index: 9_781,
        ..Default::default()
    };
    effect.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_PERIODIC_ENERGIZE,
        misc_value: 4,
        base_points: 9_998,
        base_dice: 1,
        die_sides: 1,
        amplitude: 1_000,
        target_a: 1,
        ..Default::default()
    };
    d.spells.durations.insert(
        9_781,
        SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    for s in [feed, effect] {
        d.spells.spells.insert(s.id, s);
    }
    d.items.insert(
        FOOD,
        crate::game::data::ItemTemplate {
            entry: FOOD,
            name: "Meat".into(),
            stackable: 20,
            item_level: 60,
            food_type: 1,
            ..Default::default()
        },
    );
    let (_realm, _h, gh, map, pet, _wolf) = crate::game::tests_p5_ai_pets::tamed_with(d).await;
    let held = map
        .query(move |m| {
            if let Some(s) = m.creature_mut(pet).and_then(|c| c.pet.as_deref_mut()) {
                s.food_mask = 1;
            }
            crate::game::map::inventory::add_item(m, gh, FOOD, 2, false).expect("room");
            let food = m.players[&gh]
                .items
                .iter()
                .find(|i| i.entry == FOOD)
                .map(|i| i.guid);
            let s = m.realm.data.spells.get(FEED_PET).cloned().unwrap();
            let t = crate::game::map::spells::Targets {
                item: food,
                ..Default::default()
            };
            pets::before_effects(m, gh, &s, &t);
            let h = crate::game::map::auras::auras(m, pet)
                .and_then(|a| a.holders.iter().find(|h| h.spell == FEED_EFFECT).cloned());
            (
                h.map(|h| (h.caster, h.effects.len(), h.effects[0].amount)),
                m.players[&gh].count_of(FOOD),
            )
        })
        .await
        .unwrap();
    assert_eq!(held.0, Some((gh, 1, 35_000)), "the food's benefit, no roll");
    assert_eq!(held.1, 1, "a food eaten");
}

/// Thistle Tea's "matches the VMangos weapon-speed and fire-power
/// coefficient": Flametongue's attack is the proc's points and 3.85 times
/// fire power, a hundredth a second of the weapon's speed, rounded.
#[test]
fn flametongue_goes_by_fire_power_and_the_weapons_speed() {
    use crate::game::map::spells::class_scripts::flametongue_damage;
    assert_eq!(flametongue_damage(325, 100.0, 2000), 14);
    assert_eq!(flametongue_damage(325, 0.0, 4000), 13);
}

const REINCARNATION: u32 = 21_169;
const REINCARNATION_PASSIVE: u32 = 20_608;
const ANKH: u32 = 17_030;

fn reincarnation_world() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut s = SpellTemplate {
        id: REINCARNATION,
        name: "Reincarnation".into(),
        category: 1161,
        category_recovery_time: 3_600_000,
        equipped_item_class: -1,
        ..Default::default()
    };
    s.reagents[0] = (ANKH, 1);
    s.effects[0] = SpellEffect {
        effect: 94,
        base_points: 19,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(REINCARNATION, s);
    d.items.insert(
        ANKH,
        crate::game::data::ItemTemplate {
            entry: ANKH,
            name: "Ankh".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    d
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's "rejects a forged Reincarnation offer without learning
/// it": a dead player offered Reincarnation without knowing it, though
/// carrying its Ankh, stays dead and keeps the Ankh.
#[tokio::test]
async fn reincarnation_offered_to_one_who_doesnt_know_it_does_nothing() {
    let realm = harness::realm_with(reincarnation_world());
    let mut a = Client::connect(&realm, "A");
    let ga: Guid = a.enter("Alice", Race::Human, Class::Warrior).await;
    let key = realm.registry.online(ga).expect("online").map().key;
    let map = realm.maps.get(&realm, key);
    on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, ANKH, 1, false).unwrap();
        death::kill(m, ga);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(PLAYER_SELF_RES_SPELL, REINCARNATION);
    })
    .await;
    a.clear();
    a.send(CMSG_SELF_RES {}).await;
    handled(&mut a).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (dead, ankhs, knows) = on(&map, move |m| {
        let p = &m.players[&ga];
        (
            matches!(p.death, Death::Dead { .. }),
            p.count_of(ANKH),
            p.spells.contains(&REINCARNATION_PASSIVE),
        )
    })
    .await;
    assert!(dead, "still dead");
    assert_eq!((ankhs, knows), (1, false), "the Ankh kept, nothing learned");
}
