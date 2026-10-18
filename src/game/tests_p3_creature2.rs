//! Phase 3's creature tests, ported from Thistle Tea's
//! `test/game/core/creature` (docs/parity/matrix.csv): the core hounds'
//! linked feign death.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Class;

use crate::game::data::GameData;
use crate::game::data::scripts::ScriptStep;
use crate::game::fields::*;
use crate::game::map::{Map, event_ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, aura_of, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;

/// A core hound, unkillable as its template has it; and another kind of
/// hound.
const HOUND: u32 = 3_790;
const OTHER_HOUND: u32 = 3_791;
const PACIFY: u32 = 97_900;

fn hounds() -> GameData {
    let mut d = data();
    let mut hound = template(HOUND, 1);
    hound.static_flags1 |= crate::game::map::combat::STATIC_UNKILLABLE;
    d.creature_templates.insert(HOUND, hound);
    d.creature_templates
        .insert(OTHER_HOUND, template(OTHER_HOUND, 1));
    d
}

/// Alice, beyond a pack's harm.
fn hardy(m: &mut Map, g: Guid) {
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000_000);
    f.set_u32(UNIT_FIELD_HEALTH, 100_000_000);
}

/// A hound's linked feign death: up after `delay_ms` if a hound fights on
/// within 30 yards.
fn link(m: &mut Map, g: Guid, delay_ms: u32) {
    let feign = ScriptStep {
        dataint: [0, 0, 0, 0],
        ..crate::game::tests_p3_entity::step(
            scripts::C_LINKED_FEIGN_DEATH,
            [delay_ms, HOUND, 30, 0],
        )
    };
    scripts::execute(m, &feign, g, Guid::zero());
}

/// Alive, health, stand state, the fallen flag, down.
fn state(m: &Map, g: Guid) -> (bool, u32, u8, bool, bool) {
    let c = m.creature(g).unwrap();
    (
        c.alive(),
        c.fields.get_u32(UNIT_FIELD_HEALTH),
        c.fields.get_byte(UNIT_FIELD_BYTES_1, 0),
        c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & 0x20 != 0,
        event_ai::feigning(m, g),
    )
}

/// Thistle Tea's `LinkedFeignDeath.prevent`: a killing blow, exact or
/// overkill, leaves the hound down at one health, its template's floor of
/// one notwithstanding; it keeps its foe's threat and tap, but its auras
/// go; and down, no more harm, healing or regeneration touches it. A blow
/// short of lethal leaves it fighting, and a creature without the link
/// dies of the same.
#[tokio::test]
async fn a_fallen_hound_takes_the_killing_blow_and_nothing_more() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let h = spawn(m, ga, HOUND, 20, 3.0);
        link(m, h, 60_000);
        crate::game::map::ai::attacked(m, h, ga);
        aura_of(m, h, PACIFY, crate::game::map::auras::AURA_MOD_PACIFY, 0, 0);
        // Short of lethal: hurt, not down.
        crate::game::map::combat::deal_damage(m, ga, h, 99_999);
        let hurt = state(m, h);
        // Exactly lethal.
        crate::game::map::combat::deal_damage(m, ga, h, 1);
        let down = state(m, h);
        let c = m.creature(h).unwrap();
        let kept = (
            c.threat.iter().any(|(t, _)| *t == ga),
            c.tapped_by,
            c.auras.holders.is_empty(),
            c.combat.attacking,
        );
        // More harm, a heal, regeneration: still one.
        crate::game::map::combat::deal_damage(m, ga, h, 1_000);
        crate::game::map::spells::heal(m, ga, h, 0, 1_000, false);
        let now = m.now;
        m.creature_mut(h).unwrap().engagement.regen_at = Some(now);
        crate::game::map::creature_combat::regenerate(m);
        let after = state(m, h);
        // Overkill on a second.
        let h2 = spawn(m, ga, HOUND, 20, 4.0);
        link(m, h2, 60_000);
        crate::game::map::ai::attacked(m, h2, ga);
        crate::game::map::combat::deal_damage(m, ga, h2, 10_000_000);
        let overkill = state(m, h2);
        // No link: dead.
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        crate::game::map::ai::attacked(m, brute, ga);
        crate::game::map::combat::deal_damage(m, ga, brute, 10_000_000);
        let brute = m.creature(brute).is_none_or(|c| !c.alive());
        (hurt, down, kept, after, overkill, brute)
    })
    .await;
    assert_eq!(out.0, (true, 1, 0, false, false), "hurt, still fighting");
    assert_eq!(out.1, (true, 1, 7, true, true), "down at one, fallen");
    assert!(out.2.0, "its foe's threat kept");
    assert_eq!(out.2.1, Some(ga), "and its tap");
    assert!(out.2.2, "its auras gone");
    assert!(!out.2.3, "no longer swinging");
    assert_eq!(out.3, out.1, "nothing more touches it");
    assert_eq!(out.4, (true, 1, 7, true, true), "overkill the same");
    assert!(out.5, "the brute, unlinked, dies");
}

/// Thistle Tea's `LinkedFeignDeath.tick`: a hound down, with no hound of
/// its kind fighting on within the radius, dies at its time; one out of
/// reach or of another kind doesn't count. A script's kill takes no
/// notice of the link.
#[tokio::test]
async fn a_fallen_hound_dies_without_a_fighting_kin_in_reach() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let (h, far, other, killed) = on(&map, move |m| {
        hardy(m, ga);
        let h = spawn(m, ga, HOUND, 20, 3.0);
        let far = spawn(m, ga, HOUND, 20, 45.0);
        let other = spawn(m, ga, OTHER_HOUND, 20, 4.0);
        let killed = spawn(m, ga, HOUND, 20, 5.0);
        for g in [h, far, other, killed] {
            link(m, g, 300);
            crate::game::map::ai::attacked(m, g, ga);
        }
        // The far one held where it is, out of reach.
        aura_of(
            m,
            far,
            PACIFY + 1,
            crate::game::map::auras::AURA_MOD_ROOT,
            0,
            0,
        );
        crate::game::map::combat::deal_damage(m, ga, h, 10_000_000);
        crate::game::map::combat::kill(m, ga, killed);
        (h, far, other, killed)
    })
    .await;
    let killed = on(&map, move |m| m.creature(killed).is_none_or(|c| !c.alive())).await;
    assert!(killed, "a script's kill kills");
    until(&map, "the fallen hound dies", move |m| {
        m.creature(h).is_none_or(|c| !c.alive())
    })
    .await;
    let others = on(&map, move |m| {
        [far, other].map(|g| m.creature(g).is_some_and(|c| c.alive()))
    })
    .await;
    assert_eq!(
        others,
        [true, true],
        "the far hound and the other kind fight on"
    );
}

/// Thistle Tea's `LinkedFeignDeath.reset`: a fallen hound whose foe gives
/// up the fight gets up whole, out of it.
#[tokio::test]
async fn a_fallen_hound_left_alone_gets_up_whole() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let h = on(&map, move |m| {
        let h = spawn(m, ga, HOUND, 20, 3.0);
        link(m, h, 60_000);
        crate::game::map::ai::attacked(m, h, ga);
        crate::game::map::combat::deal_damage(m, ga, h, 10_000_000);
        crate::game::map::ai::enter_evade(m, h);
        h
    })
    .await;
    until(&map, "up again", move |m| {
        let c = m.creature(h).unwrap();
        !event_ai::feigning(m, h)
            && c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH)
    })
    .await;
    let s = on(&map, move |m| state(m, h)).await;
    assert_eq!(
        (s.0, s.2, s.3, s.4),
        (true, 0, false, false),
        "standing, whole"
    );
}

/// A world of brutes spawned in a row beside Alice, spawn `guid` linked to
/// its master's spawn with `flags` for each of `links` (vmangos's
/// `creature_linking`).
fn linked(links: &[(u32, u32, u32)]) -> GameData {
    let mut d = data();
    let mut guids: Vec<u32> = links.iter().flat_map(|(s, m, _)| [*s, *m]).collect();
    guids.sort_unstable();
    guids.dedup();
    for (i, guid) in guids.into_iter().enumerate() {
        d.creature_spawns
            .entry(0)
            .or_default()
            .push(crate::game::data::CreatureSpawn {
                guid,
                entries: vec![BRUTE],
                map: 0,
                x: crate::game::tests_p3_combat::START.0 + 2.0 + i as f32,
                y: crate::game::tests_p3_combat::START.1,
                z: crate::game::tests_p3_combat::START.2,
                respawn_min: 300,
                respawn_max: 300,
                ..Default::default()
            });
    }
    for (slave, master, flags) in links {
        d.scripts.links.insert(*slave, (*master, *flags));
    }
    d
}

fn of(spawn: u32) -> Guid {
    crate::game::guid::unit(BRUTE, spawn)
}

fn alive(m: &Map, spawn: u32) -> bool {
    m.creature(of(spawn)).is_some_and(|c| c.alive())
}

fn fighting(m: &Map, spawn: u32) -> bool {
    m.creature(of(spawn))
        .is_some_and(|c| c.alive() && c.combat.attacking)
}

fn die(m: &mut Map, killer: Guid, spawn: u32) {
    crate::game::map::combat::deal_damage(m, killer, of(spawn), 10_000_000);
}

/// Thistle Tea's `CreatureLink.slave_command` and `master_command` on
/// aggro: a slave flagged to joins its master's fight, once (one already
/// fighting or dead is left be), and one not flagged doesn't; a slave
/// flagged to pulls its master in, and its master's own aggro doesn't pull
/// it.
#[tokio::test]
async fn linked_slaves_and_masters_join_each_others_fights_as_flagged() {
    use crate::game::data::scripts::{LINK_AGGRO_ON_AGGRO, LINK_TO_AGGRO_ON_AGGRO};
    // 100 leads 101 (joins it), 102 (pulls it in), 103 (neither) and 104
    // (joins it, but dead).
    let d = linked(&[
        (101, 100, LINK_AGGRO_ON_AGGRO),
        (102, 100, LINK_TO_AGGRO_ON_AGGRO),
        (103, 100, 0),
        (104, 100, LINK_AGGRO_ON_AGGRO),
        (111, 110, LINK_TO_AGGRO_ON_AGGRO),
    ]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(111)).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        die(m, ga, 104);
        crate::game::map::ai::attacked(m, of(100), ga);
        let first = [101, 102, 103, 104].map(|s| fighting(m, s));
        // The slave that pulls its master in, pulled.
        crate::game::map::ai::attacked(m, of(111), ga);
        (first, fighting(m, 110))
    })
    .await;
    assert_eq!(
        out.0,
        [true, false, false, false],
        "only the slave flagged to join, alive"
    );
    assert!(out.1, "the slave pulls its master in");
}

/// Thistle Tea's `CreatureLink.slave_command` on a master's evade, respawn,
/// despawn and death: its evade despawns, evades, or raises its slaves as
/// each is flagged; a slave's evade raises its dead master when flagged to;
/// its respawn raises its dead slaves or takes away its living ones; its
/// despawn takes along the slaves flagged to go with it; and its death
/// passes nothing on, as in vmangos.
#[tokio::test]
async fn a_masters_evade_respawn_despawn_and_death_pass_to_its_slaves_by_flag() {
    use crate::game::data::scripts::*;
    let d = linked(&[
        (121, 120, LINK_DESPAWN_ON_EVADE),
        (122, 120, LINK_EVADE_ON_EVADE | LINK_AGGRO_ON_AGGRO),
        (123, 120, LINK_RESPAWN_ON_EVADE),
        (124, 120, LINK_EVADE_ON_EVADE),
        (131, 130, LINK_RESPAWN_ON_RESPAWN),
        (132, 130, LINK_DESPAWN_ON_RESPAWN),
        (133, 130, LINK_DESPAWN_ON_DESPAWN),
        (141, 140, LINK_TO_RESPAWN_ON_EVADE),
        (151, 150, 0x10 | 0x20 | 0x40),
    ]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(151)).await;
    let evade = on(&map, move |m| {
        hardy(m, ga);
        die(m, ga, 123);
        crate::game::map::ai::attacked(m, of(120), ga);
        let joined = fighting(m, 122);
        crate::game::map::ai::enter_evade(m, of(120));
        let c = |s| m.creature(of(s)).map(|c| (c.alive(), c.evading));
        (joined, c(121), c(122), alive(m, 123), c(124))
    })
    .await;
    assert!(evade.0, "the evading slave was in the fight");
    assert_eq!(evade.1.map(|c| c.0), Some(false), "despawned");
    assert_eq!(evade.2, Some((true, true)), "evades with it");
    assert!(evade.3, "the dead slave raised");
    assert_eq!(
        evade.4,
        Some((true, false)),
        "one not fighting doesn't evade"
    );
    let respawn = on(&map, move |m| {
        die(m, ga, 131);
        die(m, ga, 130);
        crate::game::map::ai::respawn_now(m, of(130));
        let back = (alive(m, 130), alive(m, 131), alive(m, 132), alive(m, 133));
        crate::game::map::scripts::despawn(m, of(130));
        (back, alive(m, 133))
    })
    .await;
    assert_eq!(
        respawn.0,
        (true, true, false, true),
        "its dead slave raised, the other taken away"
    );
    assert!(!respawn.1, "its despawn takes the third along");
    let other = on(&map, move |m| {
        die(m, ga, 140);
        crate::game::map::ai::attacked(m, of(141), ga);
        crate::game::map::ai::enter_evade(m, of(141));
        let raised = alive(m, 140);
        die(m, ga, 150);
        let c = m.creature(of(151)).unwrap();
        (raised, c.alive(), c.combat.attacking, c.evading)
    })
    .await;
    assert!(other.0, "the slave's evade raises its master");
    assert_eq!(
        (other.1, other.2, other.3),
        (true, false, false),
        "the master's death passes nothing on"
    );
}

/// Thistle Tea's `CreatureLink.spawn_allowed?`: a slave that can't spawn
/// while its master is dead, or alive, follows its master's life, the
/// first flag deciding with both; with its master unknown, or with
/// neither flag, it may.
#[tokio::test]
async fn a_linked_slave_spawns_as_its_masters_life_allows() {
    use crate::game::data::scripts::*;
    let (dead, alive_) = (LINK_CANT_SPAWN_IF_BOSS_DEAD, LINK_CANT_SPAWN_IF_BOSS_ALIVE);
    let mut d = linked(&[
        (161, 160, dead),
        (162, 160, alive_),
        (163, 160, dead | alive_),
        (164, 160, LINK_AGGRO_ON_AGGRO),
    ]);
    d.scripts.links.insert(165, (999, dead));
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(160)).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let when_alive =
            [161, 162, 163, 164, 165].map(|s| crate::game::map::creature_groups::may_spawn(m, s));
        die(m, ga, 160);
        let when_dead =
            [161, 162, 163, 164, 165].map(|s| crate::game::map::creature_groups::may_spawn(m, s));
        (when_alive, when_dead)
    })
    .await;
    assert_eq!(out.0, [true, false, true, true, true], "the master alive");
    assert_eq!(out.1, [false, true, false, true, true], "the master dead");
}

/// Thistle Tea's `CreatureGroups` holding a slave: one that can't spawn
/// while its master lies dead is held, and comes in once its master is
/// back; one that can't while its master lives comes in once it dies.
#[tokio::test]
async fn a_held_slave_comes_in_once_its_master_allows() {
    use crate::game::data::scripts::*;
    let d = linked(&[
        (171, 170, LINK_CANT_SPAWN_IF_BOSS_DEAD),
        (173, 172, LINK_CANT_SPAWN_IF_BOSS_ALIVE),
    ]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(170)).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        // The second's slave was held at the start, its master alive.
        let held_at_start = alive(m, 173);
        die(m, ga, 170);
        die(m, ga, 171);
        crate::game::map::ai::respawn_now(m, of(171));
        let held = alive(m, 171);
        crate::game::map::ai::respawn_now(m, of(170));
        let let_in = alive(m, 171);
        // The second's slave, once its master is dead.
        crate::game::map::ai::respawn_now(m, of(173));
        die(m, ga, 172);
        (held_at_start, held, let_in, alive(m, 173))
    })
    .await;
    assert_eq!(
        out,
        (false, false, true, true),
        "held, then let in as its master allows"
    );
}

/// A world of brutes in groups of `groups` (leader spawn, members, every
/// member's flags), in a row beside Alice; a brute yells "member" when it
/// hears a member died, and "leader" when its leader did.
fn grouped(groups: &[(u32, &[u32], u32)]) -> GameData {
    use crate::game::data::scripts::*;
    let mut d = linked(&[]);
    let mut i = 0;
    for (leader, members, flags) in groups {
        for guid in std::iter::once(leader).chain(members.iter()) {
            d.creature_spawns
                .entry(0)
                .or_default()
                .push(crate::game::data::CreatureSpawn {
                    guid: *guid,
                    entries: vec![BRUTE],
                    map: 0,
                    x: crate::game::tests_p3_combat::START.0 + 2.0 + i as f32,
                    y: crate::game::tests_p3_combat::START.1,
                    z: crate::game::tests_p3_combat::START.2,
                    respawn_min: 300,
                    respawn_max: 300,
                    ..Default::default()
                });
            i += 1;
            d.scripts.group_of.insert(*guid, *leader);
        }
        d.scripts.groups.insert(
            *leader,
            std::iter::once(GroupMember {
                guid: *leader,
                ..Default::default()
            })
            .chain(members.iter().map(|m| GroupMember {
                guid: *m,
                flags: *flags,
                ..Default::default()
            }))
            .collect(),
        );
    }
    for (id, line, of_leader) in [(1u32, "member", 0), (2, "leader", 1)] {
        d.npc.broadcast_texts.insert(
            900 + id,
            crate::game::data::npc::BroadcastText {
                male: line.into(),
                chat_type: 1,
                ..Default::default()
            },
        );
        d.scripts.ai_scripts.insert(
            id,
            vec![ScriptStep {
                id,
                command: 0,
                dataint: [900 + id as i32, 0, 0, 0],
                ..Default::default()
            }],
        );
        d.scripts.ai_events.entry(BRUTE).or_default().push(AiEvent {
            id,
            creature: BRUTE,
            kind: EVENT_GROUP_MEMBER_DIED,
            chance: 100,
            flags: EF_REPEATABLE,
            params: [0, of_leader, 0, 0],
            actions: [id, 0, 0],
            ..Default::default()
        });
    }
    d
}

/// Who yelled what, sorted: (line, the yeller's spawn).
async fn yells(c: &mut crate::game::harness::Client) -> Vec<(String, u32)> {
    use wow_world_messages::vanilla::SMSG_MESSAGECHAT_ChatType;
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let mut out = Vec::new();
    while let Some(y) = c
        .gets(std::time::Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) => match &c.chat_type {
                SMSG_MESSAGECHAT_ChatType::MonsterYell { sender1, .. } => {
                    Some((c.message.clone(), sender1.guid() as u32 & 0xFF_FFFF))
                }
                _ => None,
            },
            _ => None,
        })
        .await
    {
        out.push(y);
    }
    out.sort();
    out
}

/// Thistle Tea's `CreatureGroup.actions` on a death: a group that informs
/// its leader tells it of a member's death, and one that informs its
/// members tells the rest of anyone's, the leader's as the leader's; the
/// dead hear nothing.
#[tokio::test]
async fn a_groups_death_notices_go_as_its_flags_say() {
    use crate::game::data::scripts::*;
    let d = grouped(&[
        (210, &[211, 212], GROUP_INFORM_LEADER_ON_MEMBER_DIED),
        (220, &[221, 222], GROUP_INFORM_MEMBERS_ON_ANY_DIED),
        (230, &[231, 232], 0),
    ]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(232)).await;
    a.clear();
    on(&map, move |m| {
        hardy(m, ga);
        die(m, ga, 211);
    })
    .await;
    assert_eq!(
        yells(&mut a).await,
        [("member".into(), 210)],
        "the leader told"
    );
    on(&map, move |m| die(m, ga, 221)).await;
    assert_eq!(
        yells(&mut a).await,
        [("member".into(), 222)],
        "the other member told, not the leader"
    );
    on(&map, move |m| die(m, ga, 220)).await;
    assert_eq!(
        yells(&mut a).await,
        [("leader".into(), 222)],
        "the leader's death, as the leader's, to the living"
    );
    on(&map, move |m| die(m, ga, 231)).await;
    assert_eq!(yells(&mut a).await, [], "no flags, no notices");
}

/// Thistle Tea's `CreatureGroup.actions` on an evade and a respawn, and
/// `dead?`: a member evading takes its leader, fighting, with it when they
/// evade together, and its dead come back when the leader evades (or when
/// any does, with the other flag); a member back brings its dead back with
/// it when they respawn together; and the group is dead to one of it when
/// the others here are. One that fights together does.
#[tokio::test]
async fn a_groups_evade_respawn_and_death_go_together_as_flagged() {
    use crate::game::data::scripts::*;
    let d = grouped(&[
        (
            240,
            &[241, 242],
            GROUP_EVADE_TOGETHER | GROUP_RESPAWN_ON_LEADER_EVADE,
        ),
        (250, &[251, 252], GROUP_RESPAWN_ON_LEADER_EVADE),
        (260, &[261, 262], GROUP_RESPAWN_ON_EVADE),
        (270, &[271, 272], GROUP_RESPAWN_TOGETHER),
        (280, &[281, 282], 0),
        (300, &[301], GROUP_AGGRO_TOGETHER),
    ]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(301)).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let evade = |m: &mut Map, s| crate::game::map::ai::enter_evade(m, of(s));
        // Evading together, and the leader alive: its dead back.
        die(m, ga, 242);
        crate::game::map::ai::attacked(m, of(240), ga);
        crate::game::map::ai::attacked(m, of(241), ga);
        evade(m, 241);
        let together = (
            m.creature(of(240)).is_some_and(|c| c.evading),
            alive(m, 242),
        );
        // Only on the leader's evade.
        die(m, ga, 252);
        crate::game::map::ai::attacked(m, of(251), ga);
        evade(m, 251);
        let member = alive(m, 252);
        crate::game::map::ai::attacked(m, of(250), ga);
        evade(m, 250);
        let leader = alive(m, 252);
        // On any.
        die(m, ga, 262);
        crate::game::map::ai::attacked(m, of(261), ga);
        evade(m, 261);
        let any = alive(m, 262);
        // Respawning together.
        die(m, ga, 270);
        die(m, ga, 271);
        crate::game::map::ai::respawn_now(m, of(271));
        let respawn = alive(m, 270);
        // Even a living member made new brings its dead.
        die(m, ga, 272);
        crate::game::map::ai::respawn_forced(m, of(270), true);
        let respawn = (respawn, alive(m, 272));
        // Fighting together.
        crate::game::map::ai::attacked(m, of(300), ga);
        let aggro = fighting(m, 301);
        // The group dead to one of it.
        let all = |m: &Map, s| crate::game::map::creature_groups::all_dead(m, of(s));
        let before = all(m, 280);
        die(m, ga, 281);
        die(m, ga, 282);
        let dead = (before, all(m, 280), all(m, 281));
        (together, (member, leader), any, respawn, dead, aggro)
    })
    .await;
    assert_eq!(
        out.0,
        (true, true),
        "the leader evades with it, its dead back"
    );
    assert_eq!(out.1, (false, true), "back on the leader's evade only");
    assert!(out.2, "back on any evade");
    assert_eq!(out.3, (true, true), "back with a member respawning");
    assert!(out.5, "the group fights together");
    assert_eq!(
        out.4,
        (false, true, false),
        "dead to the one left, not to one with it alive"
    );
}

/// Thistle Tea's `CreatureGroup.on_death` and `CreatureGroups.formation`:
/// a patrolling formation's leader dead, a survivor leads it on along the
/// leader's path, and another when that one dies; none left, none leads;
/// and the leader back, it leads again, the survivor behind it.
#[tokio::test]
async fn a_formation_is_led_on_by_its_survivors_until_its_leader_is_back() {
    use crate::game::data::scripts::*;
    let mut d = grouped(&[(290, &[291, 292], GROUP_FORMATION)]);
    let (x, y, z) = crate::game::tests_p3_combat::START;
    d.waypoints.insert(
        290,
        (0..2)
            .map(|i| crate::game::data::Waypoint {
                point: i,
                x: x + 2.0 + 10.0 * i as f32,
                y,
                z,
                ..Default::default()
            })
            .collect(),
    );
    if let Some(s) = d
        .creature_spawns
        .get_mut(&0)
        .and_then(|v| v.iter_mut().find(|s| s.guid == 290))
    {
        s.movement_type = 2;
    }
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(292)).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let acting = |m: &Map| m.acting_leaders.get(&290).copied();
        die(m, ga, 290);
        let first = (
            acting(m),
            crate::game::map::creature_groups::following(m, of(291)),
        );
        die(m, ga, 291);
        let second = acting(m);
        crate::game::map::ai::respawn_now(m, of(290));
        let back = (
            acting(m),
            crate::game::map::creature_groups::following(m, of(292)),
        );
        crate::game::map::ai::respawn_now(m, of(291));
        for s in [291, 292, 290] {
            die(m, ga, s);
        }
        (first, second, back, acting(m))
    })
    .await;
    assert_eq!(out.0, (Some(of(291)), false), "the first survivor leads");
    assert_eq!(out.1, Some(of(292)), "then the next");
    assert_eq!(
        out.2,
        (None, true),
        "the leader back, the survivor follows again"
    );
    assert_eq!(out.3, None, "none left, none leads");
}

const SHEEP: u32 = 3_792;

/// Critters; one spawned 3 yards from Alice.
fn critters() -> GameData {
    let mut d = data();
    let mut sheep = template(SHEEP, 8);
    sheep.faction = crate::game::tests_p3_combat::PACK;
    d.creature_templates.insert(SHEEP, sheep);
    let (x, y, z) = crate::game::tests_p3_combat::START;
    d.creature_spawns
        .entry(0)
        .or_default()
        .push(crate::game::data::CreatureSpawn {
            guid: 400,
            entries: vec![SHEEP],
            map: 0,
            x: x + 3.0,
            y,
            z,
            respawn_min: 300,
            respawn_max: 300,
            ..Default::default()
        });
    d
}

fn spawned_sheep() -> Guid {
    crate::game::guid::unit(SHEEP, 400)
}

/// Escape's end, the flee's (from whom, until), and the fleeing flag.
fn escape(
    m: &Map,
    g: Guid,
) -> (
    Option<std::time::Instant>,
    Option<(Guid, std::time::Instant)>,
    bool,
) {
    let c = m.creature(g).unwrap();
    (
        c.engagement.escape_at,
        c.engagement.flee.map(|f| (f.from, f.until)),
        c.fields
            .has_flag(UNIT_FIELD_FLAGS, crate::game::map::auras::UNIT_FLAG_FLEEING),
    )
}

/// A hit on `g` by `by`, as a blow lands.
fn hit(m: &mut Map, by: Guid, g: Guid, damage: u32) {
    crate::game::map::combat::deal_damage(m, by, g, damage);
    if m.creature(g).is_some_and(|c| c.alive()) {
        crate::game::map::ai::attacked(m, g, by);
    }
}

fn flee_flag(m: &Map, g: Guid) -> bool {
    escape(m, g).2
}

/// Thistle Tea's `Critter.react` through `Entity.take_damage`: a critter
/// struck runs from the one who struck it, for thirty seconds; a later
/// blow from another lengthens the escape but leaves the run as it was. A
/// killing blow ends the run and keeps its tap, and it comes back clean. An
/// ordinary creature, or a critter someone owns, doesn't run.
#[tokio::test]
async fn a_critter_runs_from_its_first_foe_and_later_blows_only_lengthen_its_escape() {
    let (realm, mut a, ga, map) = setup_with(critters(), Class::Warrior).await;
    let (_b, gb) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    a.sees(spawned_sheep()).await;
    let (s, first) = on(&map, move |m| {
        let s = spawned_sheep();
        hit(m, ga, s, 10);
        (s, escape(m, s))
    })
    .await;
    let (at, (from, until), fleeing) = (first.0.unwrap(), first.1.unwrap(), first.2);
    assert_eq!(from, ga, "from Alice");
    assert!(fleeing, "fleeing");
    let target = on(&map, move |m| {
        m.creature(s).unwrap().fields.get_guid(UNIT_FIELD_TARGET)
    })
    .await;
    assert!(target.is_zero(), "with no one selected");
    assert_eq!(until, at, "running as long as its escape");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let second = on(&map, move |m| {
        hit(m, gb, s, 10);
        escape(m, s)
    })
    .await;
    assert!(second.0.unwrap() > at, "the escape lengthened");
    assert_eq!(second.1, Some((ga, until)), "the run as it was");
    let dead = on(&map, move |m| {
        hit(m, ga, s, 10_000_000);
        let c = m.creature(s).unwrap();
        (c.alive(), c.tapped_by, c.threat.is_empty(), escape(m, s))
    })
    .await;
    assert_eq!(
        (dead.0, dead.1, dead.2, dead.3.1, dead.3.2),
        (false, Some(ga), true, None, false),
        "dead: its run over, its tap kept"
    );
    let back = on(&map, move |m| {
        crate::game::map::ai::respawn_now(m, s);
        m.creature(s)
            .map(|c| (c.alive(), c.engagement.escape_at.is_none(), flee_flag(m, s)))
    })
    .await;
    assert_eq!(back, Some((true, true, false)), "back clean");
    let others = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 4.0);
        let owned = spawn(m, ga, SHEEP, 20, 5.0);
        m.creature_mut(owned).unwrap().owner = Some(ga);
        hit(m, gb, brute, 10);
        hit(m, gb, owned, 10);
        [brute, owned].map(|g| m.creature(g).unwrap().engagement.escape_at.is_none())
    })
    .await;
    assert_eq!(others, [true, true], "neither runs");
}

const CURSE: u32 = 97_910;
const PREVENT_FLEEING: u32 = 97_911;

/// Thistle Tea's critter `SpellEffect.receive`: a harmful debuff cast on a
/// critter sends it running though it takes no harm.
#[tokio::test]
async fn a_cursed_critter_runs_unharmed() {
    let mut d = critters();
    let mut curse = crate::game::tests_p3_entity::spell(
        CURSE,
        &[crate::game::tests_p3_entity::aura_effect(
            crate::game::map::auras::AURA_MOD_ATTACK_POWER,
            -10,
            0,
            6,
        )],
    );
    curse.range_index = 1;
    curse.attributes[0] |= crate::game::data::spells::ATTR_DEBUFF;
    d.spells.spells.insert(CURSE, curse);
    let (_realm, a, ga, map) = setup_with(d, Class::Warrior).await;
    crate::game::tests_pets::learn(&map, ga, &[CURSE]);
    let s = on(&map, move |m| spawn(m, ga, SHEEP, 20, 3.0)).await;
    crate::game::tests_pets::cast(&a, CURSE, Some(s)).await;
    until(&map, "the curse on it", move |m| {
        crate::game::map::auras::auras(m, s).is_some_and(|a| a.has_spell(CURSE))
    })
    .await;
    let out = on(&map, move |m| {
        let c = m.creature(s).unwrap();
        (
            c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            escape(m, s),
        )
    })
    .await;
    assert!(out.0, "unharmed");
    assert!(out.1.0.is_some() && out.1.2, "and running");
}

/// Thistle Tea's `BehaviorRunner.tick` on a critter's escape: when its run
/// is over it waits out a lengthened escape without fighting; rooted, it
/// doesn't run, but its escape ends all the same; and its escape over, it
/// forgets the fight, sheds what was put on it, heals, loses its tap and
/// goes home.
#[tokio::test]
async fn an_escaped_critter_waits_its_escape_out_then_heals_forgets_and_goes_home() {
    let (_realm, _a, ga, map) = setup_with(critters(), Class::Warrior).await;
    let s = on(&map, move |m| {
        let s = spawn(m, ga, SHEEP, 20, 3.0);
        hit(m, ga, s, 10);
        // Its run over, its escape not.
        let now = m.now;
        let c = m.creature_mut(s).unwrap();
        c.engagement.escape_at = Some(now + std::time::Duration::from_secs(30));
        if let Some(f) = c.engagement.flee.as_mut() {
            f.until = now;
        }
        s
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let waiting = on(&map, move |m| {
        let c = m.creature(s).unwrap();
        (
            c.engagement.escape_at.is_some(),
            !c.fields.get_guid(UNIT_FIELD_TARGET).is_zero(),
            c.evading,
            flee_flag(m, s),
            m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH)
                == m.players[&ga].fields.get_u32(UNIT_FIELD_MAXHEALTH),
        )
    })
    .await;
    assert_eq!(
        waiting,
        (true, false, false, false, true),
        "waiting its escape out, not fighting"
    );
    // Rooted; then its escape over.
    on(&map, move |m| {
        aura_of(
            m,
            s,
            PREVENT_FLEEING,
            crate::game::map::auras::AURA_MOD_ROOT,
            0,
            0,
        );
        let now = m.now;
        m.creature_mut(s).unwrap().engagement.escape_at = Some(now);
    })
    .await;
    until(&map, "home again", move |m| {
        let c = m.creature(s).unwrap();
        c.engagement.escape_at.is_none() && !c.evading
    })
    .await;
    let home = on(&map, move |m| {
        let c = m.creature(s).unwrap();
        (
            c.threat.is_empty(),
            c.tapped_by,
            c.auras.holders.is_empty(),
            c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            flee_flag(m, s),
        )
    })
    .await;
    assert_eq!(
        home,
        (true, None, true, true, false),
        "the fight forgotten, the root shed, healed, untapped"
    );
}

/// An aura of resistance, `positive` or not, `timed` (ten seconds) or
/// not; `kept` marked to stay over an evade.
fn ward(
    id: u32,
    positive: bool,
    timed: bool,
    kept: bool,
) -> crate::game::data::spells::SpellTemplate {
    let mut s = crate::game::tests_p3_entity::spell(
        id,
        &[crate::game::tests_p3_entity::aura_effect(
            crate::game::map::auras::AURA_MOD_RESISTANCE,
            if positive { 10 } else { -10 },
            1,
            1,
        )],
    );
    if !positive {
        s.attributes[0] |= crate::game::data::spells::ATTR_DEBUFF;
    }
    if timed {
        s.duration_index = 9;
    }
    if kept {
        s.custom_flags |= crate::game::data::spells::CUSTOM_NOT_REMOVED_ON_EVADE;
    }
    s
}

/// Thistle Tea's `Aura.remove_on_evade`: an evade keeps a player's timed
/// buffs and what is marked to stay, and sheds the rest (a player's buff
/// without an end, a debuff, the creature's own buff); a creature whose
/// template says so keeps every buff.
#[tokio::test]
async fn an_evade_sheds_all_but_a_players_timed_buffs_and_the_exceptions() {
    let mut d = critters();
    d.spells.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 10_000,
            ..Default::default()
        },
    );
    for (id, positive, timed, kept) in [
        (97_920, true, true, false),
        (97_921, true, false, false),
        (97_922, false, true, false),
        (97_923, true, true, false),
        (97_924, false, false, true),
        (97_925, true, false, false),
    ] {
        d.spells.spells.insert(id, ward(id, positive, timed, kept));
    }
    let mut keeper = template(BRUTE + 50, 7);
    keeper.flags_extra |= 0x1000;
    d.creature_templates.insert(BRUTE + 50, keeper);
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        let put = |m: &mut Map, by: Guid, on: Guid, id: u32| {
            let t = m.realm.data.spells.get(id).cloned().unwrap();
            crate::game::map::auras::apply(
                m,
                by,
                on,
                &t,
                0,
                if t.attributes[0] & crate::game::data::spells::ATTR_DEBUFF != 0 {
                    -10
                } else {
                    10
                },
            );
        };
        let held = |m: &Map, g: Guid| {
            let mut v: Vec<u32> = m
                .creature(g)
                .unwrap()
                .auras
                .holders
                .iter()
                .map(|h| h.spell)
                .collect();
            v.sort_unstable();
            v
        };
        let brute = spawn(m, ga, BRUTE, 20, 4.0);
        for id in [97_920, 97_921, 97_922, 97_924] {
            put(m, ga, brute, id);
        }
        put(m, brute, brute, 97_923);
        let keeper = spawn(m, ga, BRUTE + 50, 20, 5.0);
        put(m, keeper, keeper, 97_925);
        put(m, ga, keeper, 97_922);
        let before = (held(m, brute), held(m, keeper));
        for g in [brute, keeper] {
            crate::game::map::ai::attacked(m, g, ga);
            crate::game::map::ai::enter_evade(m, g);
        }
        (before, held(m, brute), held(m, keeper))
    })
    .await;
    assert_eq!(out.0.0.len(), 5, "all five on it: {:?}", out.0);
    assert_eq!(
        out.1,
        [97_920, 97_924],
        "the player's timed buff and the exception"
    );
    assert_eq!(out.2, [97_925], "the keeper keeps its own buff");
}

/// A town hating humans; its peon, that calls the guards and starts no
/// fight; a peon of the humans' side that calls them; a town brute that
/// calls them and fights on sight (though it notices no one); and the
/// town's guard, waiting to be called.
const TOWN: u32 = 29;
const PEON: u32 = 3_793;
const FRIENDLY_PEON: u32 = 3_794;
const TOWN_BRUTE: u32 = 3_795;
const TOWN_GUARD: u32 = 3_796;
const CALLS_GUARDS: u32 = 0x0800_0000;

fn town() -> GameData {
    let mut d = data();
    d.faction_templates
        .insert(TOWN, crate::game::tests_p3_combat::faction(TOWN, 0x4, 0x1));
    for (entry, faction, extra, calls, x, y) in [
        (PEON, TOWN, 0x2, true, 4.0, 0.0),
        (
            FRIENDLY_PEON,
            crate::game::tests_p3_combat::HUMANS,
            0x2,
            true,
            0.0,
            4.0,
        ),
        (TOWN_BRUTE, TOWN, 0, true, 0.0, -4.0),
        (TOWN_GUARD, TOWN, 0x400 | 0x2, false, -15.0, 0.0),
    ] {
        let mut t = template(entry, 7);
        t.faction = faction;
        t.flags_extra = extra;
        t.civilian = extra & 0x2 != 0;
        if calls {
            t.static_flags1 |= CALLS_GUARDS;
        }
        t.detection_range = if entry == TOWN_BRUTE { 0.1 } else { 10.0 };
        d.creature_templates.insert(entry, t);
        let (sx, sy, sz) = crate::game::tests_p3_combat::START;
        d.creature_spawns
            .entry(0)
            .or_default()
            .push(crate::game::data::CreatureSpawn {
                guid: entry,
                entries: vec![entry],
                map: 0,
                x: sx + x,
                y: sy + y,
                z: sz,
                respawn_min: 300,
                respawn_max: 300,
                ..Default::default()
            });
    }
    d
}

fn town_unit(entry: u32) -> Guid {
    crate::game::guid::unit(entry, entry)
}

/// Thistle Tea's `GuardCall.watch_range`, `answered` and `sees?`: a peon
/// seeing an enemy calls the guards once; answered, it holds; with a guard
/// it summoned alive it waits, and the guard dead it may call again; back
/// from the dead it may too. A peon of the player's own side doesn't call,
/// and one that fights on sight leaves it to its aggro until it fights.
#[tokio::test]
async fn a_peons_call_waits_on_its_answer_and_only_enemies_rouse_it() {
    use crate::game::map::creature_combat::GuardCall;
    let (_realm, mut a, ga, map) = setup_with(town(), Class::Warrior).await;
    let (peon, guard) = (town_unit(PEON), town_unit(TOWN_GUARD));
    a.sees(guard).await;
    on(&map, move |m| hardy(m, ga)).await;
    until(&map, "the guard roused", move |m| {
        m.creature(guard)
            .is_some_and(|c| c.combat.attacking && c.combat.victim == ga)
    })
    .await;
    let calls = on(&map, move |m| {
        [PEON, FRIENDLY_PEON, TOWN_BRUTE]
            .map(|e| m.creature(town_unit(e)).unwrap().engagement.guard_call)
    })
    .await;
    assert_eq!(
        calls,
        [Some(GuardCall::Held), None, None],
        "only the peon of the town, that starts no fight, called"
    );
    // Answered, it holds: the guard sent home isn't called again.
    let look = move |m: &mut Map| {
        for e in [PEON, FRIENDLY_PEON, TOWN_BRUTE] {
            m.creature_mut(town_unit(e)).unwrap().engagement.watch_at = None;
        }
    };
    on(&map, move |m| {
        crate::game::map::ai::enter_evade(m, guard);
        look(m);
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !on(&map, move |m| m.creature(guard).unwrap().combat.attacking).await,
        "held: no second call"
    );
    // A guard of its own alive, it waits; that guard dead, it calls.
    let summoned = on(&map, move |m| {
        let g = spawn(m, ga, TOWN_GUARD, 20, 30.0);
        m.creature_mut(peon).unwrap().engagement.guard_call = Some(GuardCall::Summoned(g));
        crate::game::map::ai::quiet_death(m, guard);
        crate::game::map::ai::respawn_now(m, guard);
        look(m);
        g
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !on(&map, move |m| m.creature(guard).unwrap().combat.attacking).await,
        "its guard alive, it waits"
    );
    on(&map, move |m| {
        crate::game::map::ai::quiet_death(m, summoned);
        look(m);
    })
    .await;
    until(&map, "called again", move |m| {
        m.creature(guard).is_some_and(|c| c.combat.attacking)
    })
    .await;
    // Back from the dead, it may call again.
    let back = on(&map, move |m| {
        crate::game::map::ai::quiet_death(m, peon);
        crate::game::map::ai::respawn_now(m, peon);
        m.creature(peon).map(|c| c.engagement.guard_call)
    })
    .await;
    assert_eq!(back, Some(None), "its call forgotten");
    // The brute in a fight calls.
    on(&map, move |m| {
        crate::game::map::ai::enter_evade(m, guard);
        crate::game::map::ai::attacked(m, town_unit(TOWN_BRUTE), ga);
    })
    .await;
    let brute = on(&map, move |m| {
        m.creature(town_unit(TOWN_BRUTE))
            .unwrap()
            .engagement
            .guard_call
    })
    .await;
    assert_eq!(
        brute,
        Some(GuardCall::Held),
        "the brute calls once it fights"
    );
}

/// Thistle Tea's `GuardPost.recharge`: a post regains a charge for each
/// whole minute and keeps the remainder toward the next; and never holds
/// more than ten.
#[test]
fn a_guard_post_regains_whole_minutes_and_at_most_ten_charges() {
    use crate::game::map::guard_posts::{Post, take_from};
    let t0 = std::time::Instant::now();
    let secs = std::time::Duration::from_secs;
    let mut post = Post {
        charges: 3,
        cooldown_until: None,
        recharged_at: t0,
    };
    assert!(take_from(&mut post, t0 + secs(150)));
    assert_eq!(post.charges, 4, "two minutes regained, one spent");
    assert_eq!(post.recharged_at, t0 + secs(120), "the half minute kept");
    let mut full = Post {
        charges: 8,
        cooldown_until: None,
        recharged_at: t0,
    };
    assert!(take_from(&mut full, t0 + secs(900)));
    assert_eq!(full.charges, 9, "ten at most, one spent");
    assert_eq!(full.recharged_at, t0 + secs(900));
}

/// Thistle Tea's `TemporaryFaction`: a faction a script set (command 22)
/// gives way to the old one at the fight's end when its flags say so; over
/// a respawn it is gone when its flags say, and kept otherwise.
#[tokio::test]
async fn a_scripts_faction_comes_off_at_the_fights_end_or_the_respawn_as_flagged() {
    use crate::game::map::event_ai::{FACTION_RESTORE_COMBAT_STOP, FACTION_RESTORE_RESPAWN};
    let d = linked(&[(311, 310, 0), (313, 312, 0)]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(313)).await;
    let set = |m: &mut Map, s: u32, faction: u32, flags: u32| {
        let step = crate::game::tests_p3_entity::step(22, [faction, flags, 0, 0]);
        scripts::execute(m, &step, of(s), Guid::zero());
    };
    let faction = |m: &Map, s: u32| {
        m.creature(of(s))
            .unwrap()
            .fields
            .get_u32(UNIT_FIELD_FACTIONTEMPLATE)
    };
    let out = on(&map, move |m| {
        hardy(m, ga);
        let pack = faction(m, 310);
        let humans = crate::game::tests_p3_combat::HUMANS;
        // Off when the fight stops.
        set(m, 310, humans, FACTION_RESTORE_COMBAT_STOP);
        let set_to = faction(m, 310);
        crate::game::map::ai::attacked(m, of(310), ga);
        crate::game::map::ai::enter_evade(m, of(310));
        let after_fight = faction(m, 310);
        // Over a respawn: gone with its flag, kept without.
        set(m, 311, humans, FACTION_RESTORE_RESPAWN);
        set(m, 312, humans, 0);
        for s in [311, 312] {
            die(m, ga, s);
            crate::game::map::ai::respawn_now(m, of(s));
        }
        (pack, set_to, after_fight, faction(m, 311), faction(m, 312))
    })
    .await;
    let humans = crate::game::tests_p3_combat::HUMANS;
    assert_eq!(out.1, humans, "set");
    assert_eq!(out.2, out.0, "back at the fight's end");
    assert_eq!(out.3, out.0, "gone with the respawn");
    assert_eq!(out.4, humans, "kept over the respawn");
}

/// Thistle Tea's `SummonDespawn`, every type a summon may have (vmangos's
/// `TempSummonType`s, and Thistle Tea's eleventh): whether its time runs
/// from the summoning; what its time coming does to it alive and calm, alive
/// and fighting, and dead; what its death does; and whether a fight's end
/// starts its time again.
#[tokio::test]
async fn a_summons_despawn_type_decides_its_time_its_death_and_its_corpse() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let rows = on(&map, move |m| {
        hardy(m, ga);
        let after = std::time::Duration::from_secs(60);
        let summoner = spawn(m, ga, BRUTE, 20, 10.0);
        let mut rows = Vec::new();
        for kind in 1..=11 {
            let new = |m: &mut Map| {
                let at = m.creature(summoner).unwrap().position;
                scripts::summon(m, summoner, BRUTE, at, 0.0, Some((after, kind))).unwrap()
            };
            let due = |m: &mut Map, g: Guid| {
                let now = m.now;
                if let Some(s) = event_ai::scripted_mut(m, g) {
                    s.despawn_at = Some(now);
                }
                event_ai::update_despawns(m);
            };
            // 'G' gone, 'C' a corpse, 'A' alive, 'W' waiting, 'R' its time
            // started again.
            let what = |m: &Map, g: Guid| match m.creature(g) {
                None => 'G',
                Some(c) if !c.alive() => {
                    if event_ai::scripted(m, g)
                        .is_some_and(|s| s.despawn_at.is_some_and(|t| t > m.now))
                    {
                        'R'
                    } else {
                        'C'
                    }
                }
                Some(_) => {
                    if event_ai::scripted(m, g)
                        .is_some_and(|s| s.despawn_at.is_some_and(|t| t > m.now))
                    {
                        'W'
                    } else {
                        'A'
                    }
                }
            };
            let timed = {
                let g = new(m);
                let t = event_ai::scripted(m, g).is_some_and(|s| s.despawn_at.is_some());
                due(m, g);
                (t, what(m, g))
            };
            let fighting = {
                let g = new(m);
                crate::game::map::ai::attacked(m, g, ga);
                due(m, g);
                what(m, g)
            };
            let (death, dead_due) = {
                let g = new(m);
                if let Some(s) = event_ai::scripted_mut(m, g) {
                    s.despawn_at = None;
                }
                crate::game::map::combat::deal_damage(m, ga, g, 10_000_000);
                let death = what(m, g);
                if m.creature(g).is_some() {
                    due(m, g);
                }
                (death, what(m, g))
            };
            let restarts = {
                let g = new(m);
                crate::game::map::ai::attacked(m, g, ga);
                if let Some(s) = event_ai::scripted_mut(m, g) {
                    s.despawn_at = None;
                }
                event_ai::combat_stopped(m, g, false);
                event_ai::scripted(m, g).is_some_and(|s| s.despawn_at.is_some())
            };
            rows.push((kind, timed.0, timed.1, fighting, death, dead_due, restarts));
        }
        rows
    })
    .await;
    let expected = [
        // Timed from the summoning; due calm, fighting; at death; due dead;
        // started again after a fight.
        (1, true, 'G', 'W', 'C', 'C', true),
        (2, true, 'G', 'W', 'G', 'G', true),
        (3, true, 'G', 'G', 'C', 'G', false),
        (4, true, 'G', 'W', 'R', 'G', true),
        (5, false, 'A', 'A', 'G', 'G', false),
        (6, false, 'A', 'A', 'R', 'G', false),
        (7, false, 'A', 'A', 'C', 'C', false),
        (8, false, 'A', 'A', 'C', 'C', false),
        (9, true, 'G', 'G', 'C', 'G', false),
        (10, true, 'G', 'G', 'G', 'G', false),
        (11, false, 'C', 'W', 'C', 'C', false),
    ];
    for (row, want) in rows.iter().zip(expected) {
        assert_eq!(*row, want, "despawn type {}", want.0);
    }
}

const SNARE: u32 = 97_930;

/// Thistle Tea's `MovementStats.recompute` and `Entity.take_damage` on
/// wounds: a creature's wounds and a snare both slow it, one on the other;
/// healed, only the snare slows it, and the snare gone, it runs whole. Its
/// health regenerated, its wounds slow it no more; nor dead, nor back from
/// the dead.
#[tokio::test]
async fn wounds_slow_a_creature_with_its_snares_and_go_with_its_health() {
    let d = linked(&[(321, 320, 0)]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(321)).await;
    let out = on(&map, move |m| {
        let g = of(320);
        let whole = m.creature(g).unwrap().speeds.run;
        let set_health = |m: &mut Map, g: Guid, pct: u32| {
            let c = m.creature_mut(g).unwrap();
            let max = c.fields.get_u32(UNIT_FIELD_MAXHEALTH);
            c.fields.set_u32(UNIT_FIELD_HEALTH, max * pct / 100);
            crate::game::map::combat::sync_aura_state(m, g);
        };
        let run = |m: &Map, g: Guid| m.creature(g).unwrap().speeds.run / whole;
        set_health(m, g, 5);
        aura_of(
            m,
            g,
            SNARE,
            crate::game::map::auras::AURA_MOD_DECREASE_SPEED,
            0,
            -50,
        );
        let both = run(m, g);
        set_health(m, g, 100);
        let snared = run(m, g);
        crate::game::map::auras::remove_spell(m, g, SNARE, None);
        let free = run(m, g);
        // Regenerated out of a fight.
        let h = of(321);
        set_health(m, h, 5);
        let wounded = run(m, h);
        let now = m.now;
        for _ in 0..2 {
            m.creature_mut(h).unwrap().engagement.regen_at = Some(now);
            crate::game::map::creature_combat::regenerate(m);
        }
        let regenerated = run(m, h);
        // Dead, and back.
        set_health(m, h, 5);
        die(m, ga, 321);
        let dead = run(m, h);
        crate::game::map::ai::respawn_now(m, h);
        (both, snared, free, wounded, regenerated, dead, run(m, h))
    })
    .await;
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(near(out.0, 0.25), "wounds and the snare: {}", out.0);
    assert!(near(out.1, 0.5), "the snare alone: {}", out.1);
    assert!(near(out.2, 1.0), "whole: {}", out.2);
    assert!(near(out.3, 0.5), "wounded: {}", out.3);
    assert!(near(out.4, 1.0), "regenerated: {}", out.4);
    assert!(near(out.5, 1.0), "dead: {}", out.5);
    assert!(near(out.6, 1.0), "back: {}", out.6);
}

/// Thistle Tea's `Posture`: a spawn's addon sets how it rests, rides and
/// holds its weapons, and its template's mount is ridden unless the addon
/// names another or none (0; -1 is the template's); one with no addon
/// stands with its weapons out (mangos's `InitEntry`). Home from a fight,
/// it takes its posture back.
#[tokio::test]
async fn a_spawns_addon_sets_its_posture_and_it_takes_it_back_at_home() {
    let mut d = linked(&[(331, 330, 0), (333, 332, 0), (335, 334, 0)]);
    let mounted = BRUTE + 60;
    let mut t = template(mounted, 7);
    t.mount_display_id = 14_575;
    d.creature_templates.insert(mounted, t);
    for s in d.creature_spawns.get_mut(&0).unwrap().iter_mut() {
        if [331, 332, 333].contains(&s.guid) {
            s.entries = vec![mounted];
        }
    }
    let addon = |stand, sheath, emote, mount| crate::game::data::CreatureAddon {
        stand_state: stand,
        sheath_state: sheath,
        emote_state: emote,
        mount_display_id: mount,
        ..Default::default()
    };
    d.creature_addons.insert(330, addon(3, 0, 69, 2_410));
    d.creature_addons.insert(331, addon(0, 1, 0, -1));
    d.creature_addons.insert(332, addon(0, 1, 0, 0));
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let mounted_of = move |s: u32| crate::game::guid::unit(mounted, s);
    a.sees(of(330)).await;
    let out = on(&map, move |m| {
        let posture = |m: &Map, g: Guid| {
            let f = &m.creature(g).unwrap().fields;
            (
                f.get_byte(UNIT_FIELD_BYTES_1, 0),
                f.get_byte(UNIT_FIELD_BYTES_2, 0),
                f.get_u32(UNIT_NPC_EMOTESTATE),
                f.get_u32(UNIT_FIELD_MOUNTDISPLAYID),
            )
        };
        let rested = posture(m, of(330));
        let rides = [331, 332, 333].map(|s| posture(m, mounted_of(s)).3);
        let bare = posture(m, of(334));
        // Roused by a fight, then home.
        crate::game::map::ai::attacked(m, of(330), ga);
        let roused = posture(m, of(330));
        crate::game::map::ai::enter_evade(m, of(330));
        (rested, rides, bare, roused)
    })
    .await;
    assert_eq!(out.0, (3, 0, 69, 2_410), "its addon's posture");
    assert_eq!(
        out.1,
        [14_575, 0, 14_575],
        "the template's mount, none, the template's"
    );
    assert_eq!(out.2, (0, 1, 0, 0), "no addon: standing, weapons out");
    assert_ne!(out.3, out.0, "a fight stands it up");
    until(&map, "home and rested", move |m| {
        let c = m.creature(of(330)).unwrap();
        !c.evading
            && c.fields.get_byte(UNIT_FIELD_BYTES_1, 0) == 3
            && c.fields.get_u32(UNIT_NPC_EMOTESTATE) == 69
    })
    .await;
}
