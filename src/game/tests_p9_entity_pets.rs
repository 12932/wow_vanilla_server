//! Round 9's entity tests of pets and summons, ported from Thistle Tea's
//! `test/game/world/entity/mob/creature_pet_owner_test.exs`,
//! `player_test.exs` and `event_sink_test.exs` (docs/parity/matrix.csv): a
//! new pet's health after its passives, a level row of no armor, one
//! create of a pet, a timed summon at a relog, and Feed Pet.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{ATTR_PASSIVE, SpellEffect};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, auras, pets, spells};
use crate::game::tests_pets::{
    BOAR, FIREBOLT, IMP, SUMMON_IMP, TAME_BEAST, WOLF, cast, data, effect, enter, learn, pet_bar,
    place, spell,
};

/// Imp Toughness, made up: a passive of a hundred health more.
const TOUGHNESS: u32 = 990_970;
const SUMMON_WOLF: u32 = 990_971;
const FEED: u32 = 990_972;
const FEED_EFFECT: u32 = 990_973;
const MEAT: u32 = 990_974;

fn world() -> GameData {
    let mut d = data();
    let none = SpellEffect::default();
    let mut tough = effect(6, 99, 1);
    tough.aura = auras::AURA_MOD_INCREASE_HEALTH;
    let mut toughness = spell(TOUGHNESS, "Toughness", [tough, none, none]);
    toughness.attributes[0] |= ATTR_PASSIVE;
    toughness.duration_index = 21;
    d.spells.spells.insert(TOUGHNESS, toughness);
    d.pets.create_spells.insert(IMP, vec![FIREBOLT, TOUGHNESS]);
    // A guardian for a minute.
    let mut guard = effect(pets::EFFECT_SUMMON_GUARDIAN, 0, 1);
    guard.misc_value = WOLF as i32;
    let mut summon_wolf = spell(SUMMON_WOLF, "Summon Wolf", [guard, none, none]);
    summon_wolf.duration_index = 4;
    d.spells.spells.insert(SUMMON_WOLF, summon_wolf);
    // Feed Pet and its effect, happiness over a minute.
    let mut feed = effect(pets::EFFECT_FEED_PET, 0, 1);
    feed.trigger_spell = FEED_EFFECT;
    d.spells
        .spells
        .insert(FEED, spell(FEED, "Feed Pet", [feed, none, none]));
    let mut happy = effect(6, 0, 1);
    happy.aura = 24;
    happy.misc_value = 4;
    happy.amplitude = 1000;
    let mut fed = spell(FEED_EFFECT, "Feed Pet Effect", [happy, none, none]);
    fed.duration_index = 4;
    d.spells.spells.insert(FEED_EFFECT, fed);
    d.items.insert(
        MEAT,
        ItemTemplate {
            entry: MEAT,
            name: "Tough Jerky".into(),
            display_id: 1000,
            stackable: 20,
            item_level: 5,
            food_type: 1,
            ..Default::default()
        },
    );
    d
}

/// A warlock in the world, knowing Summon Imp and Summon Wolf.
async fn warlock(realm: &std::sync::Arc<crate::game::Realm>) -> (Client, Guid, MapHandle) {
    let (w, g, map) = enter(realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, g, &[SUMMON_IMP, SUMMON_WOLF]);
    harness::an_update(&map).await;
    (w, g, map)
}

/// A warlock's imp out: its guid.
async fn imp_out(w: &mut Client) -> Guid {
    cast(w, SUMMON_IMP, None).await;
    let imp = pet_bar(w).await;
    assert!(!imp.is_zero(), "the imp's bar");
    imp
}

fn vitals(m: &Map, g: Guid) -> (u32, u32, u32) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_u32(UNIT_FIELD_HEALTH),
        f.get_u32(UNIT_FIELD_MAXHEALTH),
        f.get_u32(UNIT_FIELD_RESISTANCES),
    )
}

/// A new pet comes at its full health with what its passives give it: an
/// imp of a level row's 60 and a passive's hundred more is at 160 of 160
/// (Thistle Tea's "retains template casting delays and fills resources
/// after pet passives"; mangos-zero's `EffectSummonPet` fills its health
/// after `InitPetCreateSpells`). wvs filled it before the passives, at 60
/// of 160.
#[tokio::test]
async fn a_new_pet_is_at_full_health_with_its_passives() {
    let realm = harness::realm_with(world());
    let (mut w, _g, map) = warlock(&realm).await;
    let imp = imp_out(&mut w).await;
    let (health, max, _) = map.query(move |m| vitals(m, imp)).await.unwrap();
    assert_eq!((health, max), (160, 160));
}

/// A demon's level row of no armor leaves it the armor of its class and
/// level; a row that has some gives it that (mangos-zero's
/// `InitStatsForLevel`, `if (pInfo->armor > 0)` for a summoned pet;
/// Thistle Tea's "pet level rows override canonical inputs while zero
/// damage and armor retain template values"). wvs gave it none.
#[tokio::test]
async fn a_demons_level_row_of_no_armor_leaves_it_its_own() {
    let mut d = world();
    if let Some(l) = d.pets.levels.get_mut(&(IMP, 2)) {
        l.armor = 50;
    }
    let realm = harness::realm_with(d);
    let (mut w, g, map) = warlock(&realm).await;
    let imp = imp_out(&mut w).await;
    let unarmored = map.query(move |m| vitals(m, imp).2).await.unwrap();
    map.query(move |m| crate::game::tests_p3_combat::level(m, g, 2))
        .await
        .unwrap();
    let armored = map
        .query(move |m| {
            pets::testing::summon_demon(m, g, IMP, SUMMON_IMP);
            let second = pets::pet_of(m, g).unwrap();
            assert_ne!(second, imp);
            vitals(m, second).2
        })
        .await
        .unwrap();
    // The imp's class's 20 at level 1; the row's 50 at 2; each with its
    // agility's 20, a point each.
    assert_eq!((unarmored, armored), (40, 70));
}

/// A summoned pet is created once for its owner: its create through
/// visibility is the one its bar comes after, and nothing creates it
/// again (Thistle Tea's "reuses a pet create already sent by visibility
/// before attachment").
#[tokio::test]
async fn a_summoned_pet_is_created_once_for_its_owner() {
    let realm = harness::realm_with(world());
    let (mut w, _g, map) = warlock(&realm).await;
    let imp = imp_out(&mut w).await;
    for _ in 0..10 {
        harness::an_update(&map).await;
    }
    let creates = w
        .captured()
        .iter()
        .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
        .filter(|m| harness::creates(m).contains(&imp))
        .count();
    assert_eq!(creates, 1);
}

/// A timed summon (a guardian, for a minute) doesn't come back with its
/// owner at their next login, where their demon does (Thistle Tea's "does
/// not restore a timed summon after suspension").
#[tokio::test]
async fn a_timed_summon_isnt_back_at_the_next_login() {
    let realm = harness::realm_with(world());
    let (mut w, g, map) = warlock(&realm).await;
    let imp = imp_out(&mut w).await;
    cast(&w, SUMMON_WOLF, None).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    let wolf = loop {
        let wolf = map
            .query(move |m| m.players[&g].owned.guardians.first().copied())
            .await
            .unwrap();
        if let Some(wolf) = wolf {
            break wolf;
        }
        assert!(tokio::time::Instant::now() < deadline, "no wolf");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    w.disconnect();
    harness::offline(&realm, g).await;
    let mut back = Client::connect(&realm, "W");
    back.login(g).await;
    let pet = pet_bar(&mut back).await;
    harness::an_update(&map).await;
    let (guardians, wolves) = map
        .query(move |m| {
            (
                m.players[&g].owned.guardians.len(),
                m.creatures
                    .iter()
                    .filter(|(_, c)| c.entry == WOLF && c.owner == Some(g))
                    .count(),
            )
        })
        .await
        .unwrap();
    assert!(!pet.is_zero() && pet != imp, "the demon is back, anew");
    assert_eq!((guardians, wolves), (0, 0), "the wolf isn't: {wolf}");
}

/// Feed Pet eats the food it is cast on and puts its effect (the spell its
/// effect triggers) on the pet, at the food's worth for the pet's level
/// (Thistle Tea's "feed-pet events preserve the DBC trigger and item
/// target for the player boundary").
#[tokio::test]
async fn feed_pet_eats_its_food_and_gives_the_pet_its_effect() {
    let realm = harness::realm_with(world());
    let (mut h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, FEED]);
    let boar = place(&map, gh, BOAR, 0x73_0001, 4.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    assert!(!pet.is_zero());
    let r = map
        .query(move |m| {
            crate::game::map::inventory::add_item(m, gh, MEAT, 2, false).unwrap();
            let p = &m.players[&gh];
            let meat = p.items.iter().find(|i| i.entry == MEAT).unwrap().guid;
            let cast = spells::cast(
                m,
                gh,
                FEED,
                spells::Targets {
                    item: Some(meat),
                    ..Default::default()
                },
                None,
            );
            let left = m.players[&gh].count_of(MEAT);
            let effect = auras::auras(m, pet)
                .and_then(|a| a.holders.iter().find(|h| h.spell == FEED_EFFECT).cloned())
                .map(|h| (h.caster, h.effects.first().map(|x| x.amount)));
            (cast, left, effect)
        })
        .await
        .unwrap();
    // A level-1 pet, food of level 5: 35,000.
    assert_eq!(r, (true, 1, Some((gh, Some(35_000)))));
}

/// Two warlocks side by side each summon an Eye of Kilrogg (a possessed
/// summon): two eyes, each its own caster's, possessed by them and moved
/// by them alone; no rule of a summon's nearness keeps the second from
/// coming (Thistle Tea's "possessed spell summons retain separate owners
/// without script proximity limits").
#[tokio::test]
async fn two_warlocks_eyes_are_each_their_own() {
    const EYE_OF_KILROGG: u32 = 126;
    const EYE: u32 = 4277;
    let realm = harness::realm_with(crate::game::tests_pets2::more());
    let (mut w1, g1, map) = enter(&realm, "W1", "Warlock", Race::Human, Class::Warlock).await;
    let (mut w2, g2, _) = enter(&realm, "W2", "Warlockb", Race::Human, Class::Warlock).await;
    for g in [g1, g2] {
        learn(&map, g, &[EYE_OF_KILROGG]);
    }
    let mut eyes = Vec::new();
    for (w, g) in [(&mut w1, g1), (&mut w2, g2)] {
        cast(w, EYE_OF_KILROGG, None).await;
        let eye = w
            .wait_for("control of the eye", |m| match m {
                ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u)
                    if u.guid != g && u.allow_movement =>
                {
                    Some(u.guid)
                }
                _ => None,
            })
            .await;
        eyes.push(eye);
    }
    let (e1, e2) = (eyes[0], eyes[1]);
    let r = map
        .query(move |m| {
            [(e1, g1), (e2, g2)].map(|(eye, g)| {
                let c = m.creature(eye);
                (
                    crate::game::guid::entry(eye) == EYE,
                    c.and_then(|c| c.owner) == Some(g),
                    pets::state(m, eye).map(|s| (s.kind, s.owner))
                        == Some((pets::Kind::Possessed, g)),
                )
            })
        })
        .await
        .unwrap();
    assert_ne!(e1, e2, "two eyes, each given to its own caster's client");
    assert_eq!(r, [(true, true, true); 2]);
}
