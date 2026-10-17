//! Phase 3's creature AI tests, the script commands (Thistle Tea's
//! `ai/script_test.exs` and its neighbours): each command run from a
//! creature beside the player through `scripts::execute`, and what it left.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::BroadcastText;
use crate::game::data::scripts::*;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{AURA_DUMMY, AURA_MOD_ROOT};
use crate::game::map::creature::Motion;
use crate::game::map::{Map, MapHandle, MapKey, ai, event_ai, scripts};
use crate::game::tests_p3_combat::{START, aura_of, faction};

const HUMANS: u32 = 1;
const PACK: u32 = 14;
/// Friends of humans, who answer calls for help.
const FRIENDS: u32 = 35;
const NEUTRAL: u32 = 7;

const HOST: u32 = 4600;
const BUDDY: u32 = 4601;
const THIRD: u32 = 4602;
const FOE: u32 = 4603;
const SON: u32 = 4604;

const DOOR: u32 = 600;
const HIDDEN: u32 = 601;
const BOX: u32 = 602;

const TOKEN: u32 = 22_484;
const GIFT: u32 = 22_048;

/// An aura on its caster, one with a cast time, and one on its target.
const BOON: u32 = 96_300;
const SLOW: u32 = 96_301;
const MARK: u32 = 96_302;

/// The quests: one wanting a kill of entry 11220, one wanting HOST hit by
/// spell 19512, one wanting HOST with no spell, one to fail, one to
/// explore.
const Q_KILL: u32 = 9000;
const Q_CAST: u32 = 9001;
const Q_TALK: u32 = 9002;
const Q_FAIL: u32 = 9003;
const Q_EXPLORE: u32 = 9004;
const KILLED: u32 = 11_220;

/// Generic script 950: its source sits.
const SIT: u32 = 950;

fn step(command: u32, datalong: [u32; 4]) -> ScriptStep {
    ScriptStep {
        command,
        datalong,
        ..Default::default()
    }
}

fn template(entry: u32, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 10,
        level_max: 10,
        faction,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 0.01,
        base_attack_time: 2000,
        detection_range: 0.1,
        ..Default::default()
    }
}

fn at(east: f32, north: f32) -> Vector3d {
    Vector3d {
        x: START.0 + east,
        y: START.1 + north,
        z: START.2,
    }
}

fn aura_spell(id: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_DUMMY,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn quest(id: u32) -> crate::game::data::npc::Quest {
    crate::game::data::npc::Quest {
        id,
        title: format!("Quest {id}"),
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let mut friends = faction(FRIENDS, 0x1, 0x2);
    friends.flags = 0x01;
    d.faction_templates.extend([
        (HUMANS, faction(HUMANS, 0x1, 0x2)),
        (PACK, faction(PACK, 0x2, 0x1)),
        (FRIENDS, friends),
        (
            NEUTRAL,
            FactionTemplate {
                id: NEUTRAL,
                faction: NEUTRAL,
                ..Default::default()
            },
        ),
    ]);
    d.class_level_stats.insert(
        (1, 10),
        ClassLevelStats {
            health: 500,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    for (entry, faction) in [
        (HOST, FRIENDS),
        (BUDDY, FRIENDS),
        (THIRD, FRIENDS),
        (FOE, PACK),
        (SON, FRIENDS),
    ] {
        d.creature_templates.insert(entry, template(entry, faction));
    }
    let spawns = [
        (1, HOST, 3.0, 0.0),
        (2, BUDDY, 6.0, 0.0),
        (3, THIRD, 9.0, 0.0),
        (4, FOE, -6.0, 0.0),
        (5, FOE, -9.0, 0.0),
    ]
    .iter()
    .map(|(g, e, east, north)| {
        let p = at(*east, *north);
        CreatureSpawn {
            guid: *g,
            entries: vec![*e],
            map: 0,
            x: p.x,
            y: p.y,
            z: p.z,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        }
    })
    .collect();
    d.creature_spawns.insert(0, spawns);
    for (entry, kind, name) in [(DOOR, 0, "Door"), (HIDDEN, 3, "Chest"), (BOX, 3, "Box")] {
        d.gameobject_templates.insert(
            entry,
            GameObjectTemplate {
                entry,
                kind,
                display_id: 2670,
                name: name.into(),
                size: 1.0,
                ..Default::default()
            },
        );
    }
    let object = |guid: u32, entry: u32, north: f32, despawned: bool| {
        let p = at(0.0, north);
        GameObjectSpawn {
            guid,
            entry,
            map: 0,
            x: p.x,
            y: p.y,
            z: p.z,
            rotation: [0.0, 0.0, 0.0, 1.0],
            state: 1,
            despawned,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        }
    };
    d.gameobject_spawns.insert(
        0,
        vec![object(1, DOOR, 4.0, false), object(2, HIDDEN, -4.0, true)],
    );
    for (entry, name) in [(TOKEN, "Token"), (GIFT, "Gift")] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                name: name.into(),
                stackable: 20,
                ..Default::default()
            },
        );
    }
    d.spells.ranges.insert(1, (0.0, 30.0));
    d.spells.spells.insert(BOON, aura_spell(BOON));
    let mut slow = aura_spell(SLOW);
    slow.casting_time_index = 5;
    d.spells.cast_times.insert(5, 2000);
    d.spells.spells.insert(SLOW, slow);
    let mut mark = aura_spell(MARK);
    mark.effects[0].target_a = 25;
    d.spells.spells.insert(MARK, mark);
    let mut kill = quest(Q_KILL);
    kill.required_kills[0] = (KILLED as i32, 1);
    let mut cast = quest(Q_CAST);
    cast.required_kills[0] = (HOST as i32, 1);
    cast.required_spell_casts[0] = 19_512;
    let mut talk = quest(Q_TALK);
    talk.required_kills[0] = (HOST as i32, 1);
    let mut explore = quest(Q_EXPLORE);
    explore.special_flags = 0x2;
    // One not yet done, to fail.
    let mut fail = quest(Q_FAIL);
    fail.required_kills[0] = (KILLED as i32, 5);
    for q in [kill, cast, talk, fail, explore] {
        d.npc.quests.insert(q.id, q);
    }
    d.scripts.generic.insert(
        SIT,
        vec![ScriptStep {
            id: SIT,
            ..step(28, [1, 0, 0, 0])
        }],
    );
    d.npc.broadcast_texts.insert(
        104,
        BroadcastText {
            male: "A whisper.".into(),
            chat_type: 4,
            ..Default::default()
        },
    );
    d
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    setup_with(data()).await
}

async fn setup_with(d: GameData) -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let me = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.sees(host()).await;
    while map
        .query(move |m| {
            crate::game::map::combat::dist(m.players[&me].position(), at(0.0, 0.0)) > 0.01
        })
        .await
        .unwrap()
    {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    a.clear();
    (realm, a, me, map)
}

fn host() -> Guid {
    guid::unit(HOST, 1)
}

fn buddy() -> Guid {
    guid::unit(BUDDY, 2)
}

fn third() -> Guid {
    guid::unit(THIRD, 3)
}

fn foe() -> Guid {
    guid::unit(FOE, 4)
}

fn door() -> Guid {
    guid::game_object(DOOR, 1)
}

fn run(m: &mut Map, step: &ScriptStep, source: Guid, target: Guid) -> bool {
    scripts::execute(m, step, source, target)
}

fn stand(m: &Map, g: Guid) -> u8 {
    m.unit_fields(g)
        .map(|f| f.get_byte(UNIT_FIELD_BYTES_1, 0))
        .unwrap_or(0)
}

fn has_aura(m: &Map, g: Guid, spell: u32) -> bool {
    crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn logs(m: &mut Map, g: Guid, quest: u32) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    p.journal.log.push(crate::game::store::Logged {
        quest,
        ..Default::default()
    });
    crate::game::map::quests::refresh_log(p, &data);
}

fn logged(m: &Map, g: Guid, quest: u32) -> crate::game::store::Logged {
    m.players[&g]
        .journal
        .log
        .iter()
        .find(|l| l.quest == quest)
        .cloned()
        .unwrap_or_default()
}

fn fighting(m: &mut Map, g: Guid, victim: Guid) {
    let c = m.creature_mut(g).unwrap();
    c.combat.victim = victim;
    c.combat.attacking = true;
    c.fields.set_flag(
        UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        true,
    );
}

/// The item, quest and credit commands go to the player of the step:
/// items taken (from the player only) and given, a kill credited by
/// entry, a cast by spell against the creature's entry, a quest credit
/// for the creature itself, a quest failed, and an exploration done only
/// within its reach.
#[tokio::test]
async fn the_item_and_quest_commands_go_to_the_player_of_the_step() {
    let (_realm, _a, me, map) = setup().await;
    let host = host();
    let out = map
        .query(move |m| {
            let _ = crate::game::map::inventory::add_item(m, me, TOKEN, 10, false);
            for q in [Q_KILL, Q_CAST, Q_TALK, Q_FAIL, Q_EXPLORE] {
                logs(m, me, q);
            }
            run(m, &step(40, [TOKEN, 8, 0, 0]), host, me);
            run(m, &step(40, [TOKEN, 8, 0, 0]), host, host);
            let tokens = m.players[&me].carried_count_of(TOKEN);
            run(m, &step(17, [GIFT, 1, 0, 0]), host, me);
            let gifts = m.players[&me].carried_count_of(GIFT);
            run(m, &step(8, [KILLED, 0, 0, 0]), host, me);
            run(
                m,
                &step(scripts::C_CAST_CREDIT, [19_512, 0, 0, 0]),
                host,
                me,
            );
            run(m, &step(83, [0; 4]), host, me);
            run(m, &step(70, [Q_FAIL, 0, 0, 0]), host, me);
            // Explored only within ten yards of the creature; out of reach,
            // failed.
            run(m, &step(7, [Q_EXPLORE, 10, 0, 0]), host, me);
            let near = logged(m, me, Q_EXPLORE);
            m.players
                .get_mut(&me)
                .unwrap()
                .journal
                .log
                .retain(|l| l.quest != Q_EXPLORE);
            logs(m, me, Q_EXPLORE);
            run(m, &step(7, [Q_EXPLORE, 1, 0, 0]), host, me);
            let far = logged(m, me, Q_EXPLORE);
            (
                tokens,
                gifts,
                logged(m, me, Q_KILL).kills[0],
                logged(m, me, Q_CAST).kills[0],
                logged(m, me, Q_TALK).kills[0],
                logged(m, me, Q_FAIL).failed,
                (near.explored, near.failed),
                (far.explored, far.failed),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (2, 1, 1, 1, 1, true, (true, false), (false, true)),
        "items, kill, cast and quest credit, a failure, exploration near and far"
    );
}

/// A script's cast: triggered on the caster itself (0x2) with no cast
/// under way, a player's own cast through their casting, and none when the
/// target it names isn't there.
#[tokio::test]
async fn a_scripts_cast_is_triggered_or_its_casters_own_on_whom_it_names() {
    let (_realm, _a, me, map) = setup().await;
    let host = host();
    let out = map
        .query(move |m| {
            let triggered = ScriptStep {
                data_flags: SF_TARGET_SELF,
                ..step(15, [BOON, 2, 0, 0])
            };
            run(m, &triggered, host, Guid::zero());
            let on_host = (
                has_aura(m, host, BOON),
                m.caster(host).is_some_and(|c| c.casting.is_some()),
            );
            run(m, &triggered, me, me);
            let on_me = has_aura(m, me, BOON);
            m.players.get_mut(&me).unwrap().spells.push(SLOW);
            let own = ScriptStep {
                data_flags: SF_TARGET_SELF,
                ..step(15, [SLOW, 0, 0, 0])
            };
            run(m, &own, me, me);
            let casting = m
                .caster(me)
                .and_then(|c| c.casting.as_ref())
                .map(|c| c.spell);
            // At its victim, with none: nothing.
            let at_victim = ScriptStep {
                target_type: 1,
                ..step(15, [BOON, 2, 0, 0])
            };
            run(m, &at_victim, buddy(), Guid::zero());
            (on_host, on_me, casting, has_aura(m, buddy(), BOON))
        })
        .await
        .unwrap();
    assert_eq!(out, ((true, false), true, Some(SLOW), false));
}

/// A script's whisper reaches the player it is for, and no one when the
/// target is a creature or nobody.
#[tokio::test]
async fn a_scripts_whisper_reaches_a_player_and_no_one_else() {
    let (_realm, mut a, me, map) = setup().await;
    let host = host();
    map.query(move |m| {
        let whisper = ScriptStep {
            dataint: [104, 0, 0, 0],
            ..step(0, [0; 4])
        };
        run(m, &whisper, host, buddy());
        run(m, &whisper, host, Guid::zero());
        run(m, &whisper, host, me);
    })
    .await
    .unwrap();
    let whispers = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(c)
            if matches!(
                c.chat_type,
                SMSG_MESSAGECHAT_ChatType::MonsterWhisper { .. }
            ) =>
        {
            Some(c.message.clone())
        }
        _ => None,
    };
    assert_eq!(a.wait_for("the whisper", whispers).await, "A whisper.");
    assert_eq!(
        a.gets(Duration::from_millis(300), whispers).await,
        None,
        "only one"
    );
}

/// A call for help brings its friends within the step's radius into its
/// fight (none with a radius of 0); threat is raised or cut by a share,
/// on one or all; a script event goes to the source's EventAI with the
/// step's target as its cause; an attack needs a victim to start on; and
/// a flee with no one to flee from is none.
#[tokio::test]
async fn help_threat_events_attacks_and_flight() {
    let mut d = data();
    d.npc.broadcast_texts.insert(
        105,
        BroadcastText {
            male: "Heard.".into(),
            chat_type: 1,
            ..Default::default()
        },
    );
    d.scripts.ai_scripts.insert(
        105,
        vec![ScriptStep {
            dataint: [105, 0, 0, 0],
            ..step(0, [0; 4])
        }],
    );
    d.scripts.ai_events.insert(
        HOST,
        vec![AiEvent {
            id: 1,
            creature: HOST,
            kind: EVENT_SCRIPT_EVENT,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [5944, 3, 0, 0],
            actions: [105, 0, 0],
            ..Default::default()
        }],
    );
    let (_realm, mut a, _me, map) = setup_with(d).await;
    let (host, buddy, third, foe) = (host(), buddy(), third(), foe());
    let out = map
        .query(move |m| {
            // No victim: no attack, no flight, no call.
            run(
                m,
                &ScriptStep {
                    target_type: 1,
                    ..step(26, [0; 4])
                },
                host,
                Guid::zero(),
            );
            let idle = m.creature(host).unwrap().combat.attacking;
            run(m, &step(47, [0; 4]), host, Guid::zero());
            let fled = m.creature(host).unwrap().engagement.flee.is_some();
            // With a victim.
            m.creature_mut(host).unwrap().combat.victim = foe;
            run(
                m,
                &ScriptStep {
                    x: 0.0,
                    ..step(50, [0; 4])
                },
                host,
                Guid::zero(),
            );
            let quiet = m.creature(buddy).unwrap().combat.attacking;
            run(
                m,
                &ScriptStep {
                    x: 35.0,
                    ..step(50, [0; 4])
                },
                host,
                Guid::zero(),
            );
            let helped = m.creature(buddy).map(|c| c.combat.victim);
            run(
                m,
                &ScriptStep {
                    target_type: 1,
                    ..step(26, [0; 4])
                },
                host,
                Guid::zero(),
            );
            let attacks = m.creature(host).unwrap().combat.attacking;
            // Threat: a cut on the victim, then on all.
            let c = m.creature_mut(host).unwrap();
            c.threat = vec![(foe, 100.0), (third, 50.0)];
            c.combat.victim = foe;
            run(
                m,
                &ScriptStep {
                    x: -30.0,
                    ..step(29, [1, 0, 0, 0])
                },
                host,
                Guid::zero(),
            );
            let cut = m.creature(host).unwrap().threat.clone();
            run(
                m,
                &ScriptStep {
                    x: -100.0,
                    ..step(29, [8, 0, 0, 0])
                },
                host,
                Guid::zero(),
            );
            let gone = m.creature(host).unwrap().threat.clone();
            // The script event: by its id and data.
            run(m, &step(85, [5944, 4, 0, 0]), host, buddy);
            run(m, &step(85, [5944, 3, 0, 0]), host, buddy);
            ai::enter_evade(m, host);
            ai::enter_evade(m, buddy);
            (idle, fled, quiet, helped, attacks, cut, gone)
        })
        .await
        .unwrap();
    let (idle, fled, quiet, helped, attacks, cut, gone) = out;
    assert!(!idle && !fled, "nothing without a victim");
    assert!(!quiet, "a radius of none calls no one");
    assert_eq!(helped, Some(foe), "its friend called in");
    assert!(attacks, "it attacks its victim");
    assert_eq!(cut, [(foe, 70.0), (third, 50.0)]);
    assert_eq!(gone, [(foe, 0.0), (third, 0.0)]);
    let heard = a
        .wait_for("the event's line", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterYell { target, .. } => Some(*target),
                _ => None,
            },
            _ => None,
        })
        .await;
    assert_eq!(heard, buddy, "at the step's target");
}

/// A morph to a display, back to the native one, to the one it has (no
/// change sent), and none while dead; a mount by an entry with no template
/// leaves the mount as it was.
#[tokio::test]
async fn morphs_and_mounts() {
    let (_realm, _a, _me, map) = setup().await;
    let host = host();
    let out = map
        .query(move |m| {
            let display = |m: &Map| m.unit_fields(host).unwrap().get_u32(UNIT_FIELD_DISPLAYID);
            let dirty = |m: &Map| {
                m.creature(host)
                    .unwrap()
                    .fields
                    .dirty()
                    .get(UNIT_FIELD_DISPLAYID)
            };
            run(m, &step(23, [89, 1, 0, 0]), host, host);
            let morphed = display(m);
            run(m, &step(23, [0, 0, 0, 0]), host, host);
            let native = display(m);
            m.creature_mut(host).unwrap().fields.clear_dirty();
            run(m, &step(23, [native, 1, 0, 0]), host, host);
            let quiet = !dirty(m);
            run(m, &step(23, [99_999, 0, 0, 0]), host, host);
            let unknown = display(m);
            m.unit_fields_mut(host)
                .unwrap()
                .set_u32(UNIT_FIELD_MOUNTDISPLAYID, 2404);
            run(m, &step(24, [14, 0, 0, 0]), host, host);
            let mount = m
                .unit_fields(host)
                .unwrap()
                .get_u32(UNIT_FIELD_MOUNTDISPLAYID);
            ai::die(m, host, host);
            run(m, &step(23, [89, 1, 0, 0]), host, host);
            (morphed, native, quiet, unknown, mount, display(m))
        })
        .await
        .unwrap();
    assert_eq!(out, (89, 903, true, 903, 2404, 903));
}

/// A step's condition is judged with its sides swapped where it says so,
/// and one it can't judge (an item carried by a creature) skips the step.
#[tokio::test]
async fn a_steps_condition_swaps_its_sides_and_an_unknown_one_skips_it() {
    let mut d = data();
    for (id, kind, values, flags) in [
        (30, 16, [BUDDY as i32, 0, 0, 0], CF_SWAP_TARGETS),
        (31, 16, [BUDDY as i32, 0, 0, 0], 0),
        (32, 2, [TOKEN as i32, 1, 0, 0], 0),
    ] {
        d.scripts.conditions.insert(
            id,
            Condition {
                entry: id,
                kind,
                values,
                flags,
            },
        );
    }
    let (_realm, _a, _me, map) = setup_with(d).await;
    let (host, buddy) = (host(), buddy());
    let out = map
        .query(move |m| {
            let sit = |condition| ScriptStep {
                condition,
                ..step(28, [1, 0, 0, 0])
            };
            let mut out = Vec::new();
            for c in [30, 31, 32] {
                m.unit_fields_mut(host)
                    .unwrap()
                    .set_byte(UNIT_FIELD_BYTES_1, 0, 0);
                run(m, &sit(c), host, buddy);
                out.push(stand(m, host));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        [1, 0, 0],
        "swapped: the buddy's entry; not swapped: the host's; unknown"
    );
}

/// A summon lands at the step's place, facing its way, running, for its
/// time and despawn type; with no place, on its summoner, facing its way;
/// and a despawn waits its delay and comes back after the step's time.
#[tokio::test]
async fn a_summon_lands_where_the_step_says_or_on_its_summoner() {
    let (_realm, _a, _me, map) = setup().await;
    let host = host();
    let out = map
        .query(move |m| {
            let p = at(20.0, 20.0);
            let placed = ScriptStep {
                x: p.x,
                y: p.y,
                z: p.z,
                o: 1.5,
                dataint: [1, 0, -1, 3],
                ..step(10, [SON, 30_000, 0, 0])
            };
            let sons = |m: &Map| {
                m.creatures
                    .iter()
                    .filter(|(_, c)| c.entry == SON)
                    .map(|(_, c)| c.guid)
                    .collect::<Vec<_>>()
            };
            run(m, &placed, host, Guid::zero());
            let first = sons(m)[0];
            let c = m.creature(first).unwrap();
            let s = c.scripted.as_deref().unwrap();
            let one = (
                crate::game::map::combat::dist(c.position, p) < 0.01,
                c.orientation,
                s.run,
                s.despawn_after,
                s.despawn_type,
            );
            m.creature_mut(host).unwrap().orientation = 0.5;
            let here = m.creature(host).unwrap().position;
            run(
                m,
                &ScriptStep {
                    o: 2.0,
                    ..step(10, [SON, 0, 0, 0])
                },
                host,
                Guid::zero(),
            );
            let second = sons(m).into_iter().find(|g| *g != first).unwrap();
            let c = m.creature(second).unwrap();
            let two = (
                crate::game::map::combat::dist(c.position, here) < 0.01,
                c.orientation,
            );
            let now = m.now;
            run(m, &step(18, [2000, 30, 0, 0]), host, host);
            let despawn = event_ai::scripted(m, host)
                .and_then(|s| s.script_despawn)
                .map(|(at, back)| (at.duration_since(now), back));
            (one, two, despawn)
        })
        .await
        .unwrap();
    assert_eq!(
        out.0,
        (true, 1.5, true, Some(Duration::from_secs(30)), 3),
        "its place, facing, run, time and type"
    );
    assert_eq!(out.1, (true, 0.5), "on its summoner, facing its way");
    assert_eq!(
        out.2,
        Some((Duration::from_secs(2), Some(Duration::from_secs(30)))),
        "a despawn in two seconds, back in thirty"
    );
}

/// The angle of a facing monster move to a target, if it is one.
fn faces_target(b: &[u8]) -> Option<u64> {
    let at = 1 + b[0].count_ones() as usize + 16;
    (b.get(at) == Some(&3)).then(|| u64::from_le_bytes(b[at + 1..at + 9].try_into().unwrap()))
}

/// Turning to the victim, or to a creature the step names by its spawn,
/// shows the turn to the one it faces.
#[tokio::test]
async fn turn_to_faces_the_victim_or_the_creature_it_names() {
    let (_realm, mut a, me, map) = setup().await;
    let (host, buddy) = (host(), buddy());
    map.query(move |m| {
        m.creature_mut(host).unwrap().combat.victim = me;
        run(
            m,
            &ScriptStep {
                target_type: 1,
                ..step(35, [0; 4])
            },
            host,
            Guid::zero(),
        );
    })
    .await
    .unwrap();
    a.wait_frame("the turn to the victim", ai::SMSG_MONSTER_MOVE, |b| {
        faces_target(b) == Some(me.guid())
    })
    .await;
    map.query(move |m| {
        m.creature_mut(host).unwrap().combat.victim = Guid::zero();
        let to_buddy = ScriptStep {
            target_type: 11,
            target_param1: 2,
            ..step(35, [0; 4])
        };
        run(m, &to_buddy, host, Guid::zero());
    })
    .await
    .unwrap();
    a.wait_frame("the turn to the buddy", ai::SMSG_MONSTER_MOVE, |b| {
        faces_target(b) == Some(buddy.guid())
    })
    .await;
}

/// A sound: to everyone about, or from the source as an object's.
#[tokio::test]
async fn play_sound_plays_or_sounds_from_the_source() {
    let (_realm, mut a, _me, map) = setup().await;
    let host = host();
    map.query(move |m| {
        run(m, &step(16, [6943, 0, 0, 0]), host, Guid::zero());
        run(m, &step(16, [6944, 0x2, 0, 0]), host, Guid::zero());
    })
    .await
    .unwrap();
    a.wait_for("the sound", |m| match m {
        ServerOpcodeMessage::SMSG_PLAY_SOUND(s) if s.sound_id == 6943 => Some(()),
        _ => None,
    })
    .await;
    a.wait_for("the object's sound", |m| match m {
        ServerOpcodeMessage::SMSG_PLAY_OBJECT_SOUND(s) if s.sound_id == 6944 && s.guid == host => {
            Some(())
        }
        _ => None,
    })
    .await;
}

/// A final swap hands the step to the creature it names, on the old
/// source; with that creature missing it is skipped (ending the script
/// with the abort flag); naming the source itself it runs there; and an
/// initial swap with no one to swap with is skipped.
#[tokio::test]
async fn swaps_hand_a_step_to_the_one_it_names() {
    let (_realm, _a, _me, map) = setup().await;
    let (host, buddy) = (host(), buddy());
    let out = map
        .query(move |m| {
            let named = |spawn| ScriptStep {
                target_type: 11,
                target_param1: spawn,
                data_flags: SF_SWAP_FINAL_TARGETS,
                ..step(28, [1, 1, 0, 0])
            };
            // The buddy runs it: its target, the host, sits.
            run(m, &named(2), host, Guid::zero());
            let forwarded = (stand(m, host), stand(m, buddy));
            m.unit_fields_mut(host)
                .unwrap()
                .set_byte(UNIT_FIELD_BYTES_1, 0, 0);
            let missing = run(
                m,
                &ScriptStep {
                    data_flags: SF_SWAP_FINAL_TARGETS | SF_ABORT_ON_FAILURE,
                    ..named(999)
                },
                host,
                Guid::zero(),
            );
            let itself = ScriptStep {
                target_type: 11,
                target_param1: 1,
                data_flags: SF_SWAP_FINAL_TARGETS,
                ..step(28, [1, 0, 0, 0])
            };
            run(m, &itself, host, Guid::zero());
            let local = stand(m, host);
            m.unit_fields_mut(host)
                .unwrap()
                .set_byte(UNIT_FIELD_BYTES_1, 0, 0);
            let initial = ScriptStep {
                data_flags: SF_SWAP_INITIAL_TARGETS,
                ..step(28, [1, 0, 0, 0])
            };
            run(m, &initial, host, Guid::zero());
            (forwarded, missing, local, stand(m, host))
        })
        .await
        .unwrap();
    assert_eq!(out, ((1, 0), false, 1, 0));
}

/// The movement commands: a path from a point after a wait, once round
/// (60, and movement 2); its spawn's movement by the step (67); standing
/// still (movement 0), its walk halted; a wander about where it stands
/// with its home its spawn's (movement 1); nothing while dead; and its
/// home set where it is, at the step's place or back at its spawn (34).
#[tokio::test]
async fn the_movement_commands_set_paths_wanders_stillness_and_home() {
    let mut d = data();
    let point = |point: u32, east: f32| {
        let p = at(east, 10.0);
        Waypoint {
            point,
            x: p.x,
            y: p.y,
            z: p.z,
            ..Default::default()
        }
    };
    d.waypoints_special
        .insert(HOST, vec![point(1, 3.0), point(2, 6.0)]);
    d.waypoints.insert(2, vec![point(1, 6.0), point(2, 9.0)]);
    let (_realm, _a, _me, map) = setup_with(d).await;
    let (host, buddy, third) = (host(), buddy(), third());
    let out = map
        .query(move |m| {
            let now = m.now;
            let mut out = Vec::new();
            run(
                m,
                &ScriptStep {
                    dataint: [0, HOST as i32, 0, 0],
                    ..step(60, [3, 2, 500, 0])
                },
                host,
                Guid::zero(),
            );
            let c = m.creature(host).unwrap();
            let waits = matches!(c.motion, Motion::Idle { until: Some(t) } if t == now + Duration::from_millis(500));
            let route = event_ai::scripted(m, host).and_then(|s| s.route.as_ref().map(|r| r.repeat));
            out.push((c.waypoint, waits, route));
            run(m, &step(20, [2, 0, 2, 0]), buddy, Guid::zero());
            let c = m.creature(buddy).unwrap();
            let route = event_ai::scripted(m, buddy).and_then(|s| s.route.as_ref().map(|r| r.repeat));
            out.push((c.waypoint, c.movement_type == 2, route));
            // Its default movement: a wander of seven yards.
            run(m, &step(67, [1, 0, 7, 0]), third, Guid::zero());
            let c = m.creature(third).unwrap();
            let default = (c.movement_type, c.wander_distance);
            // Walking, then still.
            scripts::move_to(m, third, at(30.0, 0.0), false);
            run(m, &step(20, [0; 4]), third, Guid::zero());
            let c = m.creature(third).unwrap();
            let still = (c.movement_type, matches!(c.motion, Motion::Idle { until: None }));
            // A wander about where it stands, its home its spawn.
            let spawn_home = m.creature(host).unwrap().home;
            scripts::place(m, host, at(20.0, 20.0), 0.0);
            run(m, &ScriptStep { x: 8.5, ..step(20, [1, 1, 0, 0]) }, host, Guid::zero());
            let c = m.creature(host).unwrap();
            let wander = (
                c.movement_type,
                c.wander_distance,
                c.home == spawn_home,
                event_ai::wander_center(c) == at(20.0, 20.0),
            );
            // Its home: where it stands, the step's place, its spawn's.
            run(m, &step(34, [1, 0, 0, 0]), host, Guid::zero());
            let here = m.creature(host).unwrap().home == at(20.0, 20.0);
            let p = at(1.0, 2.0);
            run(m, &ScriptStep { x: p.x, y: p.y, z: p.z, o: 1.0, ..step(34, [0; 4]) }, host, Guid::zero());
            let c = m.creature(host).unwrap();
            let stepped = (c.home == p, c.home_orientation);
            run(m, &step(34, [2, 0, 0, 0]), host, Guid::zero());
            let back = m.creature(host).unwrap().home == spawn_home;
            // Dead, none.
            ai::die(m, buddy, buddy);
            let moved = run(
                m,
                &ScriptStep { data_flags: SF_ABORT_ON_FAILURE, ..step(20, [1, 0, 0, 0]) },
                buddy,
                Guid::zero(),
            );
            (out, default, still, wander, here, stepped, back, moved)
        })
        .await
        .unwrap();
    let (out, default, still, wander, here, stepped, back, moved) = out;
    assert_eq!(
        out,
        [(1, true, Some(false)), (1, true, Some(false))],
        "from point 2 after its wait, once round"
    );
    assert_eq!(default, (1, 7.0));
    assert_eq!(still, (0, true), "still, its walk halted");
    assert_eq!(
        wander,
        (1, 8.5, true, true),
        "about where it stands, its home its spawn's"
    );
    assert!(here && back);
    assert_eq!(stepped, (true, 1.0));
    assert!(!moved, "the dead move nowhere");
}

/// Back home from a script's walk home, it moves as its spawn did: a
/// script's stillness and anchor given up (Thistle Tea's `start_home`).
#[tokio::test]
async fn home_from_a_scripts_walk_a_creature_moves_as_its_spawn_did() {
    let mut d = data();
    if let Some(s) = d
        .creature_spawns
        .get_mut(&0)
        .and_then(|v| v.iter_mut().find(|s| s.guid == 1))
    {
        s.movement_type = 1;
        s.wander_distance = 5.0;
    }
    let (_realm, _a, _me, map) = setup_with(d).await;
    let host = host();
    let before = map
        .query(move |m| {
            run(m, &step(20, [0; 4]), host, Guid::zero());
            scripts::place(m, host, at(8.0, 8.0), 0.0);
            run(m, &step(20, [7, 0, 0, 0]), host, Guid::zero());
            m.creature(host).unwrap().movement_type
        })
        .await
        .unwrap();
    assert_eq!(before, 0, "still on its way");
    let mut back = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        back = map
            .query(move |m| {
                let c = m.creature(host).unwrap();
                event_ai::scripted(m, host)
                    .is_some_and(|s| s.scripted_move.is_none())
                    .then_some((c.movement_type, c.wander_distance))
            })
            .await
            .unwrap();
        if back.is_some() {
            break;
        }
    }
    assert_eq!(back, Some((1, 5.0)), "wandering again, as its spawn does");
}

/// Flags set and cleared on a creature's fields and an object's (4); its
/// sheath (51); a floor of health, flat or a share (52); a gossip menu
/// until it respawns (84); and hidden and shown (207).
#[tokio::test]
async fn flags_sheath_invincibility_gossip_and_concealment() {
    let (_realm, mut a, _me, map) = setup().await;
    let (host, door) = (host(), door());
    let out = map
        .query(move |m| {
            let f = m.unit_fields_mut(host).unwrap();
            f.set_u32(UNIT_FIELD_FLAGS, 0x100);
            f.set_u32(UNIT_NPC_FLAGS, 0x3);
            run(
                m,
                &step(4, [UNIT_FIELD_FLAGS as u32, 0x200, 1, 0]),
                host,
                Guid::zero(),
            );
            run(
                m,
                &step(4, [UNIT_NPC_FLAGS as u32, 0x2, 2, 0]),
                host,
                Guid::zero(),
            );
            let f = m.unit_fields(host).unwrap();
            let unit = (f.get_u32(UNIT_FIELD_FLAGS), f.get_u32(UNIT_NPC_FLAGS));
            m.gameobject_mut(door)
                .unwrap()
                .fields
                .set_u32(GAMEOBJECT_FLAGS, 0x10);
            run(
                m,
                &step(4, [GAMEOBJECT_FLAGS as u32, 0x10, 2, 0]),
                door,
                Guid::zero(),
            );
            let object = m.gameobject(door).unwrap().fields.get_u32(GAMEOBJECT_FLAGS);
            run(m, &step(51, [1, 0, 0, 0]), host, Guid::zero());
            let sheath = m.unit_fields(host).unwrap().get_byte(UNIT_FIELD_BYTES_2, 0);
            run(m, &step(52, [25, 1, 0, 0]), host, Guid::zero());
            let share = event_ai::scripted(m, host).and_then(|s| s.invincible_hp);
            run(m, &step(52, [10, 0, 0, 0]), host, Guid::zero());
            let flat = event_ai::scripted(m, host).and_then(|s| s.invincible_hp);
            run(m, &step(84, [5711, 0, 0, 0]), host, Guid::zero());
            let menu = event_ai::scripted(m, host).and_then(|s| s.gossip_menu);
            run(
                m,
                &step(scripts::C_SET_CONCEALED, [1, 0, 0, 0]),
                buddy(),
                Guid::zero(),
            );
            (unit, object, sheath, share, flat, menu)
        })
        .await
        .unwrap();
    assert_eq!(out, ((0x300, 0x1), 0, 1, Some(125), Some(10), Some(5711)));
    a.forgets(buddy()).await;
    map.query(|m| {
        run(
            m,
            &step(scripts::C_SET_CONCEALED, [0; 4]),
            buddy(),
            Guid::zero(),
        );
    })
    .await
    .unwrap();
    a.sees(buddy()).await;
    let respawned = map
        .query(move |m| {
            ai::die(m, host, host);
            ai::respawn_now(m, host);
            event_ai::scripted(m, host).and_then(|s| s.gossip_menu)
        })
        .await
        .unwrap();
    assert_eq!(respawned, None, "its menu gone with its respawn");
}

/// The object commands: its state (80); the nearest of an entry used by
/// a creature's step, as its owner (13 at target type 13); a spell an
/// object casts at the one using it (15); a spawn brought in (82) and a
/// creature's spawn brought back (91); and an object summoned for a
/// while, at its summoner where the step leaves a coordinate 0 (76).
#[tokio::test]
async fn the_object_commands() {
    let (_realm, _a, me, map) = setup().await;
    let (host, door) = (host(), door());
    let out = map
        .query(move |m| {
            run(m, &step(80, [2, 0, 0, 0]), door, Guid::zero());
            let state = m.gameobject(door).unwrap().fields.get_u32(GAMEOBJECT_STATE);
            run(m, &step(80, [1, 0, 0, 0]), door, Guid::zero());
            let used = ScriptStep {
                target_type: 13,
                target_param1: DOOR,
                target_param2: 10,
                ..step(13, [0; 4])
            };
            run(m, &used, host, Guid::zero());
            let opened = m.gameobject(door).unwrap().fields.get_u32(GAMEOBJECT_STATE);
            let cast = run(m, &step(15, [MARK, 0, 0, 0]), door, me);
            let nothing = run(
                m,
                &ScriptStep {
                    data_flags: SF_ABORT_ON_FAILURE,
                    ..step(15, [0; 4])
                },
                door,
                me,
            );
            run(m, &step(82, [2, 0, 0, 0]), host, Guid::zero());
            let hidden = scripts::object_of_spawn(m, 2).is_some();
            m.remove_creature(buddy());
            run(m, &step(91, [2, 0, 0, 0]), host, Guid::zero());
            let back = scripts::creature_of_spawn(m, 2).is_some();
            m.creature_mut(host).unwrap().orientation = 0.7;
            let here = m.creature(host).unwrap().position;
            let now = m.now;
            run(
                m,
                &ScriptStep {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    ..step(76, [BOX, 300, 0, 0])
                },
                host,
                Guid::zero(),
            );
            let boxed = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == BOX)
                .map(|(_, o)| (o.guid, o.position, o.orientation));
            let (box_guid, box_at, box_o) = boxed.unwrap();
            let gone_at = m
                .object_resets
                .iter()
                .find(|(_, g, _)| *g == box_guid)
                .map(|(t, _, _)| t.duration_since(now));
            (
                state,
                opened,
                (cast, has_aura(m, me, MARK), nothing),
                hidden,
                back,
                (crate::game::map::combat::dist(box_at, here) < 0.01, box_o),
                gone_at,
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, 2, "its state");
    assert_eq!(out.1, 0, "the door the host found, opened");
    assert_eq!(
        out.2,
        (true, true, false),
        "cast at its user; no spell, no cast"
    );
    assert!(
        out.3 && out.4,
        "the object brought in, the creature brought back"
    );
    assert_eq!(out.5, (true, 0.7), "at its summoner, facing its way");
    assert_eq!(out.6, Some(Duration::from_secs(300)));
}
