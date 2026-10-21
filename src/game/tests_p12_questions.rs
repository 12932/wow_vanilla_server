//! The open questions settled by mangos-classic (round 12): each test
//! pins what mangos-classic does where wvs did otherwise, the question's
//! number in docs/DECISIONS.md by its test.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_entity::{bob, on, spell};

// #12: the Cenarion Favor in the Gates of Ahn'Qiraj.

/// mangos-classic's `OutdoorPvPMgr::GetScriptOfAffectedZone` gives
/// Silithus's script the Gates of Ahn'Qiraj (zone 3478) with the Ruins and
/// the Temple, so the side that won the Cenarion Favor keeps it there, on
/// Kalimdor; Thistle Tea's zone list (and wvs's) left it out.
#[tokio::test]
async fn the_cenarion_favor_holds_in_the_gates_of_ahnqiraj() {
    use crate::game::tests_p3_world_pvp::{silithus, tick};
    let realm = harness::realm_with(silithus());
    let mut c = Client::connect(&realm, "Alice");
    let g = c.enter("Alice", Race::NightElf, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    assert_eq!(key.map, 1);
    let map = realm.maps.get(&realm, key);
    map.query(move |m| {
        m.outdoor
            .as_mut()
            .unwrap()
            .silithus
            .as_mut()
            .unwrap()
            .controller = Some(0);
        m.players.get_mut(&g).unwrap().zone = 3478;
    })
    .await;
    let mut has = false;
    for _ in 0..30 {
        tick(&map).await;
        has = map
            .query(move |m| m.players[&g].auras.has_spell(30754))
            .await
            .unwrap();
        if has {
            break;
        }
    }
    assert!(has, "the Alliance's favor at the Gates");
}

// #23: an untapped creature's soul shard.

const DRAIN: u32 = 912_300;
const SHARD: u32 = 6265;

fn drain_world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut s = spell(
        DRAIN,
        &[SpellEffect {
            effect: 6,
            aura: auras::AURA_CHANNEL_DEATH_ITEM,
            base_points: 0,
            base_dice: 1,
            target_a: 25,
            item_type: u64::from(SHARD),
            ..Default::default()
        }],
    );
    s.family_name = 5;
    d.spells.spells.insert(DRAIN, s);
    d.items.insert(
        SHARD,
        ItemTemplate {
            entry: SHARD,
            name: "Soul Shard".into(),
            display_id: 1000,
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            buy_count: 1,
            ..Default::default()
        },
    );
    d
}

/// mangos-classic's `Aura::HandleChannelDeathItem` gives a soul shard only
/// for a creature `IsTappedBy` the warlock (their tap or their group's):
/// an untapped one is the killing blow's, so a stranger's blow leaves the
/// warlock none. wvs counted no one's tap as everyone's.
#[tokio::test]
async fn an_untapped_creature_killed_by_a_stranger_leaves_the_warlock_no_shard() {
    use crate::game::map::spells::outcome::death_items;
    let (realm, _a, ga, map) = setup_with(drain_world(), Class::Warlock).await;
    let (_b, gb) = bob(&realm, ga).await;
    let got = on(&map, move |m| {
        let wolf = spawn(m, ga, BRUTE, 1, 5.0);
        let s = m.realm.data.spells.get(DRAIN).cloned().unwrap();
        auras::apply_spell(m, ga, wolf, &s, &[(0, 1)], Default::default()).unwrap();
        m.creature_mut(wolf).unwrap().tapped_by = None;
        let hp = m.creature(wolf).unwrap().fields.get_u32(UNIT_FIELD_HEALTH);
        let by_stranger = death_items(m, wolf, gb, hp);
        let by_her = death_items(m, wolf, ga, hp);
        (by_stranger, by_her)
    })
    .await;
    assert_eq!(got.0, Vec::<(Guid, u32, u32)>::new(), "Bob's blow taps it");
    assert_eq!(got.1, vec![(ga, SHARD, 1)], "her own blow, hers");
}
