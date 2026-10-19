//! Round 6's player tests of pets, ported from Thistle Tea's
//! `test/game/world/entity/player/talent_reset_test.exs` and others
//! (docs/parity/matrix.csv): what a talent reset does to a pet.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::harness::{self, Client};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;
use crate::game::tests_pets::{
    BOAR, CALL_PET, DISMISS_PET, SUMMON_IMP, TAME_BEAST, cast, enter, learn, pet_bar, place,
};

/// A talent of one rank for hunters and warlocks; a trainer for each; a
/// soul shard that the imp's summoning takes.
const TALENT_RANK: u32 = 96_300;
const HUNTERS: u32 = 9_406;
const WARLOCKS: u32 = 9_407;
const SOUL_SHARD: u32 = 6_265;

fn world() -> GameData {
    let mut d = crate::game::tests_pets::data();
    d.spells.talents.insert(
        1,
        crate::game::data::spells::TalentInfo {
            id: 1,
            tab: 161,
            ranks: [TALENT_RANK, 0, 0, 0, 0],
            ..Default::default()
        },
    );
    d.spells.talent_tab_classes.insert(161, 0x4 | 0x100);
    let mut rank = crate::game::tests_p3_entity::spell(TALENT_RANK, &[]);
    rank.attributes[0] = 0x40;
    d.spells.spells.insert(TALENT_RANK, rank);
    for (entry, class) in [(HUNTERS, 3), (WARLOCKS, 9)] {
        d.creature_templates.insert(
            entry,
            crate::game::data::CreatureTemplate {
                faction: 0,
                npc_flags: crate::game::map::npc::NPC_FLAG_TRAINER,
                trainer_class: class,
                ..crate::game::tests_p3_combat::template(entry, 7)
            },
        );
    }
    d.items.insert(
        SOUL_SHARD,
        crate::game::data::ItemTemplate {
            entry: SOUL_SHARD,
            name: "Soul Shard".into(),
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    d.spells.spells.get_mut(&SUMMON_IMP).unwrap().reagents[0] = (SOUL_SHARD, 1);
    d
}

/// `g` at level 20 with the talent, and a trainer of `entry` beside them,
/// seen.
async fn talented(c: &mut Client, map: &crate::game::map::MapHandle, entry: u32) -> Guid {
    let g = c.guid;
    let trainer = on(map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 20);
        crate::game::map::spells::learn_talent(m, g, 1, 0);
        m.players.get_mut(&g).unwrap().set_money(100_000);
        let t = crate::game::tests_p6_player::npc_beside(m, g, 0, 2.0);
        let data = m.realm.data.clone();
        let c = m.creature_mut(t).unwrap();
        c.entry = entry;
        let tpl = data.creature_template(entry).unwrap();
        c.fields
            .set_u32(crate::game::fields::UNIT_NPC_FLAGS, tpl.npc_flags);
        c.fields
            .set_u32(crate::game::fields::OBJECT_FIELD_ENTRY, entry);
        t
    })
    .await;
    c.sees(trainer).await;
    let has = on(map, move |m| m.players[&g].spells.contains(&TALENT_RANK)).await;
    assert!(has, "the talent learned");
    trainer
}

/// Thistle Tea's `TalentReset.complete` "suspends a hunter pet with its
/// final progression and removes its process and controls": a hunter's
/// talent reset puts the pet away (mangos's `RemovePet`), its bar gone,
/// to be called again as it was.
#[tokio::test]
async fn a_talent_reset_puts_a_hunters_pet_away_to_be_called_again() {
    let realm = harness::realm_with(world());
    let (mut h, gh, map) = enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    learn(&map, gh, &[TAME_BEAST, CALL_PET, DISMISS_PET]);
    let boar = place(&map, gh, BOAR, 0x70_0001, 4.0).await;
    h.sees(boar).await;
    cast(&h, TAME_BEAST, Some(boar)).await;
    let pet = pet_bar(&mut h).await;
    assert!(!pet.is_zero(), "tamed");
    let trainer = talented(&mut h, &map, HUNTERS).await;
    let number = on(&map, move |m| {
        m.players[&gh]
            .row
            .state
            .pets
            .current
            .as_ref()
            .map(|r| r.number)
    })
    .await;
    h.clear();
    h.send(MSG_TALENT_WIPE_CONFIRM_Client {
        wiping_npc: trainer,
    })
    .await;
    assert!(pet_bar(&mut h).await.is_zero(), "the bar gone");
    let (out, kept) = on(&map, move |m| {
        let p = &m.players[&gh];
        (
            p.owned.pet.is_some(),
            p.row.state.pets.current.as_ref().map(|r| r.number),
        )
    })
    .await;
    assert!(!out, "put away");
    assert_eq!(kept, number, "kept to be called");
    learn(&map, gh, &[CALL_PET]);
    cast(&h, CALL_PET, None).await;
    assert!(!pet_bar(&mut h).await.is_zero(), "called again");
}

/// Thistle Tea's `TalentReset.complete` "dismisses a summoned demon and
/// refunds its reagent without restoring it on login": a warlock's talent
/// reset sends the demon away and gives back the soul shard its summoning
/// took (mangos's `RemovePet(PET_SAVE_REAGENTS)`); it isn't back at a login.
#[tokio::test]
async fn a_talent_reset_sends_a_demon_away_and_gives_back_its_shard() {
    let realm = harness::realm_with(world());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    on(&map, move |m| {
        // No GM: she pays her reagents.
        m.players.get_mut(&gw).unwrap().gm = false;
        crate::game::map::inventory::add_item(m, gw, SOUL_SHARD, 1, false).unwrap();
    })
    .await;
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    assert!(!imp.is_zero(), "the imp");
    let shards = on(&map, move |m| m.players[&gw].count_of(SOUL_SHARD)).await;
    assert_eq!(shards, 0, "the shard taken");
    let trainer = talented(&mut w, &map, WARLOCKS).await;
    w.clear();
    w.send(MSG_TALENT_WIPE_CONFIRM_Client {
        wiping_npc: trainer,
    })
    .await;
    assert!(pet_bar(&mut w).await.is_zero(), "the bar gone");
    handled(&mut w).await;
    let (out, record, shards) = on(&map, move |m| {
        let p = &m.players[&gw];
        (
            p.owned.pet.is_some(),
            p.row.state.pets.current.is_some(),
            p.count_of(SOUL_SHARD),
        )
    })
    .await;
    assert_eq!(
        (out, record, shards),
        (false, false, 1),
        "gone, the shard back"
    );
    w.send(CMSG_LOGOUT_REQUEST {}).await;
    w.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE
        )
        .then_some(())
    })
    .await;
    w.clear();
    w.login(gw).await;
    handled(&mut w).await;
    let map = realm.registry.online(gw).unwrap().map();
    let back = on(&map, move |m| m.players[&gw].owned.pet.is_some()).await;
    assert!(!back, "not back at a login");
}
