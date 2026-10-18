//! The world's last gaps with Thistle Tea, through the protocol: quests
//! that want a spell cast on a creature (and kills that don't count for
//! them), the Ritual of Summoning's checks, the server's notices (death's
//! wear, speeds backwards, a faction at war for a fight, a language not
//! learned, a meeting stone refused, a banner's state, rest), the tables
//! that shape loot, vendors, reputation, flights and reach, and skinning's
//! knives.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::crafts::LockKey;
use crate::game::data::misc::{FactionDef, MeetingStone};
use crate::game::data::npc::{Quest, TaxiPathNode};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::spells::Targets;
use crate::game::map::spells::checks::fail;
use crate::game::map::{Map, MapKey};

/// Northshire, where humans start.
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const LEGACKI: u32 = 10_978;
const WOLF: u32 = 299;
const CHEER: u32 = 17_166;
const YETI_QUEST: u32 = 5163;
const RITUAL: u32 = 698;
const SLOW: u32 = 900_500;
const STONE: u32 = 178_824;
const BANNER: u32 = 900_600;
const BANNER_LOCK: u32 = 900;
const NEUTRAL_FACTION: u32 = 72;
const NEUTRAL_TEMPLATE: u32 = 40;
const GUARD: u32 = 900_700;
const THE_BEAST: u32 = 10_430;
const SKINNING: u32 = 8613;
const FINKLE: u32 = 16_710;
const LEATHER: u32 = 2318;

fn spell(id: u32, effect: u32, target_a: u32, aura: u32, points: i32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect,
        target_a,
        aura,
        base_points: points,
        ..Default::default()
    };
    s
}

fn creature(entry: u32, name: &str, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_max: 500,
        speed_walk: 0.01,
        speed_run: 0.01,
        detection_range: 0.1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.spells.ranges.insert(1, (0.0, 30.0));
    d.spells.spells.insert(CHEER, spell(CHEER, 3, 25, 0, 0));
    let mut ritual = spell(RITUAL, 50, 47, 0, 0);
    ritual.script_name = "spell_warlock_ritual_of_summoning".into();
    d.spells.spells.insert(RITUAL, ritual);
    d.spells.spells.insert(SLOW, spell(SLOW, 6, 1, 33, -51));
    let mut skinning = spell(SKINNING, 95, 25, 0, 0);
    skinning.totems = [7005, 0];
    d.spells.spells.insert(SKINNING, skinning);
    d.spells.spells.insert(FINKLE, spell(FINKLE, 6, 1, 4, 0));
    // "Are We There, Yeti?" as vmangos has it: cheer at Legacki, then a
    // wolf for good measure.
    d.npc.quests.insert(
        YETI_QUEST,
        Quest {
            id: YETI_QUEST,
            title: "Are We There, Yeti?".into(),
            min_level: 1,
            level: 1,
            required_kills: [
                (LEGACKI as i32, 1),
                (WOLF as i32, 1),
                (WOLF as i32, 1),
                (0, 0),
            ],
            required_spell_casts: [CHEER, 0, 0, 0],
            ..Default::default()
        },
    );
    d.creature_templates
        .insert(LEGACKI, creature(LEGACKI, "Legacki", 35));
    d.creature_templates
        .insert(WOLF, creature(WOLF, "Wolf", 35));
    d.creature_templates
        .insert(GUARD, creature(GUARD, "Bodyguard", NEUTRAL_TEMPLATE));
    let mut beast = creature(THE_BEAST, "The Beast", 35);
    beast.skinning_loot_id = THE_BEAST;
    d.creature_templates.insert(THE_BEAST, beast);
    d.loot.skinning.insert(
        THE_BEAST,
        vec![LootEntry {
            item: LEATHER,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    d.items.insert(
        LEATHER,
        ItemTemplate {
            entry: LEATHER,
            name: "Light Leather".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    for (entry, name) in [(7005, "Skinning Knife"), (12_709, "Finkle's Skinner")] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: name.into(),
                stackable: 1,
                ..Default::default()
            },
        );
    }
    let spawn = |guid, entry, dx: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map: 0,
        x: START.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![
            spawn(1, LEGACKI, 3.0),
            spawn(2, WOLF, -3.0),
            spawn(3, GUARD, 4.0),
            spawn(4, THE_BEAST, -2.0),
        ],
    );
    // A neutral faction the players start out at peace with.
    d.misc.factions.insert(
        NEUTRAL_FACTION,
        FactionDef {
            id: NEUTRAL_FACTION,
            index: 5,
            name: "Booty Bay".into(),
            ..Default::default()
        },
    );
    d.faction_templates.insert(
        NEUTRAL_TEMPLATE,
        FactionTemplate {
            id: NEUTRAL_TEMPLATE,
            faction: NEUTRAL_FACTION,
            ..Default::default()
        },
    );
    // A meeting stone, and a banner taken by attacking it.
    let object = |entry, kind, data0| {
        let mut t = GameObjectTemplate {
            entry,
            kind,
            display_id: 1,
            name: format!("Object {entry}"),
            size: 1.0,
            ..Default::default()
        };
        t.data[0] = data0;
        t
    };
    d.gameobject_templates.insert(STONE, object(STONE, 23, 1));
    d.gameobject_templates
        .insert(BANNER, object(BANNER, 10, BANNER_LOCK));
    d.crafts.locks.insert(
        BANNER_LOCK,
        vec![LockKey {
            kind: crate::game::data::crafts::LOCK_KEY_SKILL,
            index: 17,
            skill: 0,
        }],
    );
    d.misc.meeting_stones.insert(
        STONE,
        MeetingStone {
            min_level: 1,
            max_level: 60,
            area: 1581,
        },
    );
    let near = |guid, entry, dx: f32| GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + dx,
        y: START.1 + 2.0,
        z: START.2,
        state: 1,
        ..Default::default()
    };
    d.gameobject_spawns
        .insert(0, vec![near(30, STONE, 0.0), near(31, BANNER, 1.0)]);
    d
}

fn map0(realm: &Arc<Realm>) -> crate::game::map::MapHandle {
    realm.maps.get(realm, MapKey::world(0))
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map0(realm).query(f).await.expect("the map answers")
}

async fn grouped(realm: &Arc<Realm>, names: &[&str]) -> Vec<Client> {
    let mut out = Vec::new();
    for name in names {
        let mut c = Client::connect(realm, name);
        c.enter(name, Race::Human, Class::Warrior).await;
        out.push(c);
    }
    for i in 1..out.len() {
        let name = names[i].to_owned();
        out[0].send(CMSG_GROUP_INVITE { name }).await;
        let who = &mut out[i];
        who.wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
        who.send(CMSG_GROUP_ACCEPT {}).await;
        who.wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
    }
    out
}

fn counts(m: &Map, g: Guid) -> [u32; 4] {
    m.players[&g]
        .journal
        .log
        .iter()
        .find(|l| l.quest == YETI_QUEST)
        .map_or([0; 4], |l| [l.kills[0], l.kills[1], l.kills[2], l.kills[3]])
}

#[tokio::test]
async fn a_spell_cast_at_a_creature_credits_its_quest_and_a_kill_doesnt() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let legacki = guid::unit(LEGACKI, 1);
    let wolf = guid::unit(WOLF, 2);
    a.sees(legacki).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.spells.push(CHEER);
        p.journal.log.push(crate::game::store::Logged {
            quest: YETI_QUEST,
            ..Default::default()
        });
        crate::game::map::quests::refresh_log(p, &d);
    })
    .await;
    // Killing Legacki isn't cheering at him; a wolf's kill counts once,
    // for its first objective.
    let after_kills = on_map(&realm, move |m| {
        crate::game::map::quests::killed(m, g, legacki);
        crate::game::map::quests::killed(m, g, wolf);
        counts(m, g)
    })
    .await;
    assert_eq!(after_kills, [0, 1, 0, 0]);
    // A cheer for free (a triggered cast) doesn't count; one cast does.
    let triggered = on_map(&realm, move |m| {
        crate::game::map::spells::cast_triggered(m, g, CHEER, Some(legacki));
        counts(m, g)
    })
    .await;
    assert_eq!(triggered, [0, 1, 0, 0]);
    on_map(&realm, move |m| {
        crate::game::map::spells::cast(
            m,
            g,
            CHEER,
            Targets {
                unit: Some(legacki),
                ..Default::default()
            },
            None,
        )
    })
    .await;
    let credit = a
        .wait_for("SMSG_QUESTUPDATE_ADD_KILL", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) if k.creature_id == LEGACKI => {
                Some(k.guid)
            }
            _ => None,
        })
        .await;
    assert_eq!(credit, legacki);
    assert_eq!(on_map(&realm, move |m| counts(m, g)).await, [1, 1, 0, 0]);
}

async fn ritual_check(realm: &Arc<Realm>, g: Guid, target: Guid) -> Option<u8> {
    on_map(realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        if !p.spells.contains(&RITUAL) {
            p.spells.push(RITUAL);
        }
        p.fields.set_guid(UNIT_FIELD_TARGET, target);
        let s = m.realm.data.spells.get(RITUAL).cloned().unwrap();
        let mods = crate::game::map::spells::mods::for_spell(m, g, &s);
        crate::game::map::spells::checks::check(m, g, &s, &Targets::default(), None, false, &mods)
    })
    .await
}

#[tokio::test]
async fn the_ritual_of_summoning_wants_a_party_member_out_of_combat() {
    let realm = harness::realm_with(data());
    let cs = grouped(&realm, &["Alice", "Bob"]).await;
    let mut e = Client::connect(&realm, "E");
    let ge = e.enter("Eve", Race::Human, Class::Warrior).await;
    let (ga, gb) = (cs[0].guid, cs[1].guid);
    assert_eq!(
        ritual_check(&realm, ga, Guid::zero()).await,
        Some(fail::BAD_TARGETS)
    );
    assert_eq!(
        ritual_check(&realm, ga, ga).await,
        Some(fail::BAD_TARGETS),
        "not herself"
    );
    assert_eq!(
        ritual_check(&realm, ga, ge).await,
        Some(fail::BAD_TARGETS),
        "Eve isn't hers"
    );
    assert_eq!(ritual_check(&realm, ga, gb).await, None);
    on_map(&realm, move |m| {
        m.players.get_mut(&gb).unwrap().fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            true,
        );
    })
    .await;
    assert_eq!(
        ritual_check(&realm, ga, gb).await,
        Some(fail::TARGET_AFFECTING_COMBAT)
    );
}

fn frame_of(c: &mut Client, opcode: u16) -> impl std::future::Future<Output = Vec<u8>> + '_ {
    c.wait_frame("a frame", opcode, |_| true)
}

#[tokio::test]
async fn the_server_sends_what_thistle_tea_does() {
    let realm = harness::realm_with(data());
    let mut cs = grouped(&realm, &["Alice", "Bob"]).await;
    let (ga, gb) = (cs[0].guid, cs[1].guid);
    // Rest's start, at login.
    cs[0]
        .wait_for("SMSG_SET_REST_START", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_SET_REST_START(_)).then_some(())
        })
        .await;
    // A banner taken by attacking it starts each viewer afresh.
    let banner = guid::game_object(BANNER, 31);
    cs[0].sees(banner).await;
    let reset = cs[0]
        .wait_for("SMSG_GAMEOBJECT_RESET_STATE", |m| match m {
            ServerOpcodeMessage::SMSG_GAMEOBJECT_RESET_STATE(r) => Some(r.guid),
            _ => None,
        })
        .await;
    assert_eq!(reset, banner);
    // A language she hasn't learned (and, no GM, can't speak).
    on_map(&realm, move |m| {
        m.players.get_mut(&ga).unwrap().gm_level = 0
    })
    .await;
    cs[0]
        .send(CMSG_MESSAGECHAT {
            chat_type: CMSG_MESSAGECHAT_ChatType::Say,
            language: Language::Orcish,
            message: "lok'tar".into(),
        })
        .await;
    let notice = cs[0]
        .wait_for("SMSG_NOTIFICATION", |m| match m {
            ServerOpcodeMessage::SMSG_NOTIFICATION(n) => Some(n.notification.clone()),
            _ => None,
        })
        .await;
    assert_eq!(notice, "You have not learned that language.");
    // Bob isn't the party's leader: the stone won't have him.
    let stone = guid::game_object(STONE, 30);
    cs[1].sees(stone).await;
    cs[1].send(CMSG_MEETINGSTONE_JOIN { guid: stone }).await;
    let reason = cs[1]
        .wait_for("SMSG_MEETINGSTONE_JOINFAILED", |m| match m {
            ServerOpcodeMessage::SMSG_MEETINGSTONE_JOINFAILED(f) => Some(f.reason),
            _ => None,
        })
        .await;
    assert_eq!(reason, MeetingStoneFailure::MeetingstoneFailPartyleader);
    // A slow slows her backwards too.
    on_map(&realm, move |m| {
        let s = m.realm.data.spells.get(SLOW).cloned().unwrap();
        crate::game::map::auras::apply(m, ga, ga, &s, 0, -50);
    })
    .await;
    let back = cs[0]
        .wait_for("SMSG_FORCE_RUN_BACK_SPEED_CHANGE", |m| match m {
            ServerOpcodeMessage::SMSG_FORCE_RUN_BACK_SPEED_CHANGE(s) => Some(s.speed),
            _ => None,
        })
        .await;
    assert!((back - 2.25).abs() < 0.01, "{back}");
    // A neutral faction's guard she fights is at war with her for the
    // fight.
    let guard = guid::unit(GUARD, 3);
    on_map(&realm, move |m| {
        crate::game::map::ai::attacked(m, guard, ga)
    })
    .await;
    let war = frame_of(&mut cs[0], 0x313).await;
    assert_eq!(war, vec![5, 0, 0, 0, 2]);
    on_map(&realm, move |m| {
        crate::game::map::combat::leave_combat(m, ga)
    })
    .await;
    let peace = frame_of(&mut cs[0], 0x313).await;
    assert_eq!(peace, vec![5, 0, 0, 0, 0]);
    // Killed by a creature, Bob's gear wears, and he's told.
    let wolf = guid::unit(WOLF, 2);
    on_map(&realm, move |m| crate::game::map::combat::kill(m, wolf, gb)).await;
    cs[1]
        .wait_for("SMSG_DURABILITY_DAMAGE_DEATH", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_DURABILITY_DAMAGE_DEATH).then_some(())
        })
        .await;
}

#[tokio::test]
async fn any_skinning_knife_serves_and_the_beast_frees_finkle() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let beast = guid::unit(THE_BEAST, 4);
    a.sees(beast).await;
    let missing = on_map(&realm, move |m| {
        let s = m.realm.data.spells.get(SKINNING).cloned().unwrap();
        let p = &m.players[&g];
        crate::game::map::professions::tool_missing(p, &s, 7005)
    })
    .await;
    assert_eq!(missing, Some(fail::EQUIPPED_ITEM), "no knife at all");
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.spells.push(SKINNING);
        crate::game::map::skills::learn_skill(p, 393, 1);
        crate::game::map::skills::set_value(p, 393, 300);
        let _ = crate::game::map::inventory::add_item(m, g, 12_709, 1, false);
        crate::game::map::combat::kill(m, g, beast);
    })
    .await;
    let missing = on_map(&realm, move |m| {
        let s = m.realm.data.spells.get(SKINNING).cloned().unwrap();
        crate::game::map::professions::tool_missing(&m.players[&g], &s, 7005)
    })
    .await;
    assert_eq!(missing, None, "Finkle's Skinner skins");
    on_map(&realm, move |m| {
        crate::game::map::spells::cast(
            m,
            g,
            SKINNING,
            Targets {
                unit: Some(beast),
                ..Default::default()
            },
            None,
        )
    })
    .await;
    let mut freed = false;
    for _ in 0..100 {
        freed = on_map(&realm, move |m| {
            crate::game::map::auras::auras(m, g)
                .is_some_and(|a| a.holders.iter().any(|h| h.spell == FINKLE))
        })
        .await;
        if freed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(freed, "skinning the Beast casts Finkle is Einhorn");
}

#[test]
fn a_flight_is_stitched_where_its_paths_meet() {
    let node = |x: f32| TaxiPathNode {
        map: 0,
        x,
        ..Default::default()
    };
    let a: Vec<TaxiPathNode> = (0..5).map(|i| node(i as f32)).collect();
    let b: Vec<TaxiPathNode> = (0..4).map(|i| node(10.0 + i as f32)).collect();
    let xs = |v: Vec<TaxiPathNode>| v.into_iter().map(|n| n.x).collect::<Vec<f32>>();
    // Named: left at A's third node, B joined at its second.
    let mut named = ahash::AHashMap::new();
    named.insert((1, 2), (2, 1));
    let stitched = crate::game::map::npc::stitch(&named, &[(1, a.clone()), (2, b.clone())]);
    assert_eq!(xs(stitched), vec![0.0, 1.0, 2.0, 11.0, 12.0, 13.0]);
    // Unnamed: A's next to last, B's second.
    let stitched = crate::game::map::npc::stitch(&ahash::AHashMap::new(), &[(1, a), (2, b)]);
    assert_eq!(xs(stitched), vec![0.0, 1.0, 2.0, 3.0, 11.0, 12.0, 13.0]);
}

/// The real tables: forbidden items off vendors and out of loot, flights'
/// transitions, objects' boxes, and the quests that want a spell cast.
#[test]
#[ignore = "extended"]
fn the_real_tables_load() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    // Removed by a patch up to 1.12: Lethargy Root off its 51 vendors,
    // and item 835 out of the loot that has it.
    let dropped = d
        .loot
        .creature
        .values()
        .chain(d.loot.gameobject.values())
        .chain(d.loot.reference.values())
        .flatten()
        .any(|e| e.min >= 0 && e.item == 835);
    let sold = d
        .vendors
        .values()
        .chain(d.vendor_templates.values())
        .flatten()
        .any(|v| v.item == 3777);
    assert!(!dropped && !sold);
    assert!(
        d.npc.taxi_transitions.len() > 300,
        "{}",
        d.npc.taxi_transitions.len()
    );
    assert!(
        d.gameobject_templates
            .values()
            .filter(|t| t.bounds.is_some())
            .count()
            > 1000
    );
    let yeti = &d.npc.quests[&5163];
    assert_eq!(
        (yeti.required_kills[0].0, yeti.required_spell_casts[0]),
        (10_978, 17_166)
    );
    for q in [6124, 6129] {
        assert_eq!(d.npc.quests[&q].required_spell_casts[0], 19_512);
    }
}
