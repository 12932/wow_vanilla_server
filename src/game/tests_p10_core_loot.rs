//! Round 10, against Thistle Tea's `loot/loot_session_test.exs`
//! (`visible?/2`): a corpse whose only drop is a quest item sparkles for
//! one it fell to who is on the quest, not for one who isn't, nor for
//! round robin's other members, as mangos's per-viewer `Loot::CanLoot`.

use std::collections::BTreeMap;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::{GameData, LootEntry};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::loot::DYNFLAG_LOOTABLE;
use crate::game::map::{Map, MapHandle};
use crate::game::tests_bugs6::fields;
use crate::game::tests_pets::{BOAR, data, enter, place};

const NOTE: u32 = 63_500;
const QUEST: u32 = 6_350;
const LOOT_ID: u32 = 6_350;

fn world() -> GameData {
    let mut d = data();
    d.items.insert(
        NOTE,
        ItemTemplate {
            entry: NOTE,
            name: "Note".into(),
            display_id: 1000,
            stackable: 1,
            ..Default::default()
        },
    );
    d.loot.creature.insert(
        LOOT_ID,
        vec![LootEntry {
            item: NOTE,
            chance: -100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    let boar = d.creature_templates.get_mut(&BOAR).unwrap();
    boar.loot_id = LOOT_ID;
    (boar.gold_min, boar.gold_max) = (0, 0);
    d.npc.quests.insert(
        QUEST,
        crate::game::data::npc::Quest {
            id: QUEST,
            title: "Notes".into(),
            min_level: 1,
            level: 1,
            required_items: [(NOTE, 1), (0, 0), (0, 0), (0, 0)],
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

async fn group(leader: &mut Client, member: &mut Client, name: &str, method: GroupLootSetting) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    member
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
    let master = leader.guid;
    leader
        .send(CMSG_LOOT_METHOD {
            loot_setting: method,
            loot_master: master,
            loot_threshold: ItemQuality::Uncommon,
        })
        .await;
    leader
        .wait_for("the loot method", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => l
                .group_not_empty
                .as_ref()
                .filter(|g| g.loot_setting == method)
                .map(|_| ()),
            _ => None,
        })
        .await;
}

/// The dynamic flags of the next values block `c` is told of `of` with
/// them.
async fn flags_of(c: &mut Client, of: Guid) -> u32 {
    let f: BTreeMap<u16, u32> = c
        .wait_for("its dynamic flags", |m| {
            harness::values(m)
                .into_iter()
                .filter(|(g, _)| *g == of)
                .map(|(_, mask)| fields(&mask))
                .find(|f| f.contains_key(&UNIT_DYNAMIC_FLAGS))
        })
        .await;
    f[&UNIT_DYNAMIC_FLAGS]
}

/// Ari on the quest and Cy not, grouped by `method`, kill boars of the
/// quest's drop alone, one after another: whether each sparkles for Ari
/// and for Cy.
async fn sparkles(method: GroupLootSetting, boars: u32) -> Vec<(bool, bool)> {
    let realm = harness::realm_with(world());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    let (mut c, _gc, _) = enter(&realm, "C", "Cy", Race::Dwarf, Class::Hunter).await;
    group(&mut a, &mut c, "Cy", method).await;
    on(&map, move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .journal
            .add(crate::game::store::Logged {
                quest: QUEST,
                ..Default::default()
            });
    })
    .await;
    let mut out = Vec::new();
    for n in 0..boars {
        let boar = place(&map, ga, BOAR, 0x71_0031 + n, 6.0).await;
        a.sees(boar).await;
        c.sees(boar).await;
        a.clear();
        c.clear();
        let raw = on(&map, move |m| {
            crate::game::map::combat::kill(m, ga, boar);
            m.creature(boar).unwrap().fields.get_u32(UNIT_DYNAMIC_FLAGS) & DYNFLAG_LOOTABLE != 0
        })
        .await;
        assert!(raw, "the drop fell for Ari");
        let ari = flags_of(&mut a, boar).await & DYNFLAG_LOOTABLE != 0;
        let cy = flags_of(&mut c, boar).await & DYNFLAG_LOOTABLE != 0;
        out.push((ari, cy));
    }
    out
}

#[tokio::test]
async fn a_quest_drop_alone_sparkles_for_the_one_on_the_quest_only() {
    // Free for all: Ari's, not Cy's.
    assert_eq!(
        sparkles(GroupLootSetting::FreeForAll, 1).await,
        vec![(true, false)]
    );
}

#[tokio::test]
async fn round_robins_looter_off_the_quest_sees_no_quest_drop_nor_does_the_other() {
    // Round robin: the first is Ari's to loot, and she is on the quest;
    // the second Cy's, who isn't, and Ari isn't its looter: nobody's.
    assert_eq!(
        sparkles(GroupLootSetting::RoundRobin, 2).await,
        vec![(true, false), (false, false)]
    );
}
