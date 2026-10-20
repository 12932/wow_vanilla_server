//! Round 8's creature tests against Thistle Tea's
//! `test/game/core/creature/firework_guy_test.exs` (docs/parity/matrix.csv):
//! the Lunar Festival's firework launchers, vmangos's
//! `npc_pats_firework_guy`.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemSpell;
use crate::game::data::npc::Quest;
use crate::game::data::spells::{SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::{GameData, GameObjectSpawn, GameObjectTemplate, ItemTemplate};
use crate::game::fireworks::{self, CLUSTER_CREDIT, LUCKY, LUNAR_FORTUNE, ROCKET_CREDIT};
use crate::game::guid;
use crate::game::harness::Client;
use crate::game::map::{Map, MapHandle, inventory};
use crate::game::tests_p3_combat::{HUMANS, START, setup_with, template};
use crate::game::tests_p3_entity::on;

fn v(x: f32, y: f32, z: f32) -> Vector3d {
    Vector3d { x, y, z }
}

/// Thistle Tea's `launch/2`: a rocket bursts three yards over its guy for
/// the rocket credit, a large one its large rocket; a cluster fans its
/// color's five about and over him for the cluster credit; the lucky one
/// mixes its large rockets and brings Lunar Fortune; any other creature
/// sends nothing up.
#[test]
fn a_firework_guy_sends_up_a_rocket_or_a_cluster_and_nothing_else_does() {
    let rocket = fireworks::launch(15_882, v(1.0, 2.0, 3.0)).unwrap();
    assert_eq!(rocket.fireworks, vec![(180_851, v(1.0, 2.0, 6.0))]);
    assert_eq!((rocket.credit, rocket.lucky), (ROCKET_CREDIT, false));
    assert_eq!(
        fireworks::launch(15_888, v(1.0, 2.0, 3.0))
            .unwrap()
            .fireworks,
        vec![(180_860, v(1.0, 2.0, 6.0))],
        "a large red rocket"
    );
    let cluster = fireworks::launch(15_873, v(0.0, 0.0, 0.0)).unwrap();
    assert_eq!(cluster.credit, CLUSTER_CREDIT);
    assert_eq!(
        cluster.fireworks,
        vec![
            (180_851, v(0.0, 0.0, 8.0)),
            (180_851, v(3.5, -1.0, 5.0)),
            (180_851, v(0.0, 2.0, 5.0)),
            (180_851, v(0.0, 0.0, 2.0)),
            (180_851, v(-3.5, -1.0, 5.0)),
        ]
    );
    let lucky = fireworks::launch(LUCKY, v(0.0, 0.0, 0.0)).unwrap();
    assert_eq!((lucky.credit, lucky.lucky), (CLUSTER_CREDIT, true));
    assert_eq!(
        lucky.fireworks.iter().map(|f| f.0).collect::<Vec<_>>(),
        [180_861, 180_864, 180_864, 180_861, 180_861]
    );
    assert_eq!(lucky.fireworks.last(), Some(&(180_861, v(0.0, 0.0, 12.0))));
    assert_eq!(fireworks::launch(15_466, v(0.0, 0.0, 0.0)), None);
}

const SMALL_RED_ROCKET: u32 = 21_557;
const SMALL_RED_ROCKET_SPELL: u32 = 26_286;
const RED_GUY: u32 = 15_882;
const LUCKY_CLUSTER: u32 = 21_744;
const LUCKY_CLUSTER_SPELL: u32 = 26_521;
const LAUNCHER: u32 = 180_850;
const ROCKET_FOCUS: u32 = 1351;
const CLUSTER_FOCUS: u32 = 1352;
const LUNAR_FIREWORKS: u32 = 8867;
const RED_ROCKET: u32 = 180_851;

/// A rocket's or a cluster's spell, as the client has them: the launcher
/// worked (86 at target 40), the guy called up there (41 at target 46).
fn firework_spell(id: u32, focus: u32, guy: u32, launcher: u32, d: &mut GameData) {
    let mut s = SpellTemplate {
        id,
        name: format!("Firework {id}"),
        equipped_item_class: -1,
        range_index: 5,
        duration_index: 28,
        requires_spell_focus: focus,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: crate::game::map::objects::EFFECT_ACTIVATE_OBJECT,
        target_a: 40,
        misc_value: 1,
        ..Default::default()
    };
    s.effects[1] = SpellEffect {
        effect: crate::game::map::pets::EFFECT_SUMMON_WILD,
        target_a: 46,
        misc_value: guy as i32,
        base_points: 0,
        base_dice: 1,
        die_sides: 1,
        ..Default::default()
    };
    d.spells.spells.insert(id, s);
    d.spells.script_targets.insert(id, vec![(0, launcher)]);
}

fn item(entry: u32, spell: u32) -> ItemTemplate {
    let mut t = ItemTemplate {
        entry,
        name: format!("Firework {entry}"),
        display_id: 1000,
        stackable: 20,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    t.spells[0] = ItemSpell {
        id: spell,
        trigger: 0,
        charges: -1,
        ..Default::default()
    };
    t
}

fn object(entry: u32, kind: u8, data: &[(usize, u32)]) -> GameObjectTemplate {
    let mut t = GameObjectTemplate {
        entry,
        kind,
        display_id: 1,
        name: format!("Object {entry}"),
        size: 1.0,
        ..Default::default()
    };
    for (i, x) in data {
        t.data[*i] = *x;
    }
    t
}

/// Where the launchers stand: eight yards east of the start.
const AT: (f32, f32, f32) = (START.0 + 8.0, START.1, START.2);

/// A firework launcher and Omen's cluster launcher by the start, their
/// rockets, the Small Red Rocket and the Lucky Rocket Cluster, Lunar
/// Fortune, and the Lunar Fireworks quest.
fn world(launcher: u32) -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.ranges.insert(5, (0.0, 40.0));
    d.spells.radii.insert(9, 20.0);
    for (index, ms) in [(28, 5_000), (30, 1_800_000)] {
        d.spells.durations.insert(
            index,
            SpellDuration {
                base: ms,
                ..Default::default()
            },
        );
    }
    let focus = if launcher == LAUNCHER {
        ROCKET_FOCUS
    } else {
        CLUSTER_FOCUS
    };
    d.gameobject_templates.insert(
        launcher,
        object(
            launcher,
            crate::game::map::objects::GO_SPELL_FOCUS,
            &[(0, focus), (1, 35)],
        ),
    );
    d.gameobject_spawns.insert(
        0,
        vec![GameObjectSpawn {
            guid: 900,
            entry: launcher,
            map: 0,
            x: AT.0,
            y: AT.1,
            z: AT.2,
            state: 1,
            ..Default::default()
        }],
    );
    for rocket in [RED_ROCKET, 180_861, 180_864] {
        d.gameobject_templates
            .insert(rocket, object(rocket, 10, &[(3, 196_608)]));
    }
    firework_spell(
        SMALL_RED_ROCKET_SPELL,
        ROCKET_FOCUS,
        RED_GUY,
        LAUNCHER,
        &mut d,
    );
    firework_spell(
        LUCKY_CLUSTER_SPELL,
        CLUSTER_FOCUS,
        LUCKY,
        crate::game::omen::LAUNCHER,
        &mut d,
    );
    for guy in [RED_GUY, LUCKY] {
        let mut t = template(guy, 10);
        t.faction = HUMANS;
        t.display_ids = [11_686, 0, 0, 0];
        d.creature_templates.insert(guy, t);
    }
    let mut fortune = SpellTemplate {
        id: LUNAR_FORTUNE,
        name: "Lunar Fortune".into(),
        equipped_item_class: -1,
        range_index: 1,
        duration_index: 30,
        ..Default::default()
    };
    fortune.effects[0] = SpellEffect {
        effect: 6,
        aura: 34,
        base_points: 249,
        base_dice: 1,
        die_sides: 1,
        target_a: 22,
        target_b: 8,
        radius_index: 9,
        ..Default::default()
    };
    d.spells.spells.insert(LUNAR_FORTUNE, fortune);
    d.items.insert(
        SMALL_RED_ROCKET,
        item(SMALL_RED_ROCKET, SMALL_RED_ROCKET_SPELL),
    );
    d.items
        .insert(LUCKY_CLUSTER, item(LUCKY_CLUSTER, LUCKY_CLUSTER_SPELL));
    d.npc.quests.insert(
        LUNAR_FIREWORKS,
        Quest {
            id: LUNAR_FIREWORKS,
            title: "Lunar Fireworks".into(),
            min_level: 1,
            level: 1,
            required_kills: [
                (ROCKET_CREDIT as i32, 8),
                (CLUSTER_CREDIT as i32, 2),
                (0, 0),
                (0, 0),
            ],
            ..Default::default()
        },
    );
    d
}

/// Alice on Lunar Fireworks, with a stack of `entry`.
async fn ready(map: &MapHandle, g: Guid, entry: u32) {
    on(map, move |m| {
        inventory::add_item(m, g, entry, 5, false).unwrap();
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.journal.log.push(crate::game::store::Logged {
            quest: LUNAR_FIREWORKS,
            ..Default::default()
        });
        crate::game::map::quests::refresh_log(p, &d);
    })
    .await;
}

/// `CMSG_USE_ITEM` of `entry`, aimed at nothing.
async fn fire(a: &mut Client, map: &MapHandle, g: Guid, entry: u32) {
    let (bag, slot) = on(map, move |m| {
        let p = &m.players[&g];
        let i = p.items.iter().position(|i| i.entry == entry).unwrap();
        (p.items[i].bag, p.items[i].slot)
    })
    .await;
    a.send(CMSG_USE_ITEM {
        bag_index: bag,
        bag_slot: slot,
        spell_index: 0,
        targets: crate::game::map::spells::wire_targets(&Default::default()),
    })
    .await;
}

/// Where a create puts its object.
fn placed(m: &ServerOpcodeMessage) -> Vec<(Guid, Vector3d)> {
    let objects = match m {
        ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
        ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
        _ => return Vec::new(),
    };
    objects
        .iter()
        .filter_map(|o| match &o.update_type {
            Object_UpdateType::CreateObject {
                guid3, movement2, ..
            }
            | Object_UpdateType::CreateObject2 {
                guid3, movement2, ..
            } => match movement2.update_flag.get_living()? {
                MovementBlock_UpdateFlag_Living::Living {
                    living_position, ..
                } => Some((*guid3, *living_position)),
                MovementBlock_UpdateFlag_Living::HasPosition { position, .. } => {
                    Some((*guid3, *position))
                }
            },
            _ => None,
        })
        .collect()
}

/// What `a` is shown created of each of `wanted`'s entries, until it has
/// seen as many of each as asked: by entry, the objects and where.
async fn created(a: &mut Client, wanted: &[(u32, usize)]) -> Vec<(u32, Guid, Vector3d)> {
    let mut out: Vec<(u32, Guid, Vector3d)> = Vec::new();
    let enough = |out: &[(u32, Guid, Vector3d)]| {
        wanted
            .iter()
            .all(|(e, n)| out.iter().filter(|(x, ..)| x == e).count() >= *n)
    };
    while !enough(&out) {
        let more = a
            .wait_for("the creates", |m| {
                let these: Vec<(u32, Guid, Vector3d)> = placed(m)
                    .into_iter()
                    .map(|(g, at)| (guid::entry(g), g, at))
                    .filter(|(e, ..)| wanted.iter().any(|(w, _)| w == e))
                    .collect();
                (!these.is_empty()).then_some(these)
            })
            .await;
        out.extend(more);
    }
    out
}

fn of(seen: &[(u32, Guid, Vector3d)], entry: u32) -> Vec<(Guid, Vector3d)> {
    seen.iter()
        .filter(|(e, ..)| *e == entry)
        .map(|(_, g, at)| (*g, *at))
        .collect()
}

fn near(a: Vector3d, b: Vector3d) -> bool {
    (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.z - b.z).abs() < 0.01
}

/// Thistle Tea's `launch/2` through a launcher (and vmangos's guy): a
/// Small Red Rocket used by a launcher calls its guy up there, not at its
/// user; his rocket bursts three yards over it at once (its create, then
/// its despawn, as vmangos's `go_firework_rocket` sends it off), and its
/// firer has the rocket credit toward Lunar Fireworks.
#[tokio::test]
async fn a_rocket_used_at_a_launcher_bursts_over_it_at_once_for_the_rocket_credit() {
    let (_realm, mut a, g, map) = setup_with(world(LAUNCHER), Class::Warrior).await;
    ready(&map, g, SMALL_RED_ROCKET).await;
    a.clear();
    fire(&mut a, &map, g, SMALL_RED_ROCKET).await;
    // The credit as the guy sends it up, before the update's creates.
    let credit = a
        .wait_for("the rocket credit", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) => Some((
                k.quest_id,
                k.creature_id,
                k.kill_count,
                k.required_kill_count,
            )),
            _ => None,
        })
        .await;
    assert_eq!(credit, (LUNAR_FIREWORKS, ROCKET_CREDIT, 1, 8));
    let launcher = v(AT.0, AT.1, AT.2);
    let seen = created(&mut a, &[(RED_GUY, 1), (RED_ROCKET, 1)]).await;
    let guy = of(&seen, RED_GUY);
    assert!(near(guy[0].1, launcher), "the guy at the launcher: {guy:?}");
    let rocket = of(&seen, RED_ROCKET);
    assert!(
        near(rocket[0].1, v(AT.0, AT.1, AT.2 + 3.0)),
        "three yards over it: {rocket:?}"
    );
    let burst = rocket[0].0;
    a.wait_for("the rocket goes off", |m| match m {
        ServerOpcodeMessage::SMSG_GAMEOBJECT_DESPAWN_ANIM(d) if d.guid == burst => Some(()),
        _ => None,
    })
    .await;
}

/// The lucky cluster at Omen's launcher: five large rockets fanned over it
/// (blue, white, white, blue, blue, the last twelve yards up), the cluster
/// credit, a firework counted toward Omen, and Lunar Fortune on those
/// about it three seconds after.
#[tokio::test]
async fn a_lucky_cluster_at_omens_launcher_counts_for_him_and_brings_lunar_fortune() {
    let (realm, mut a, g, map) =
        setup_with(world(crate::game::omen::LAUNCHER), Class::Warrior).await;
    ready(&map, g, LUCKY_CLUSTER).await;
    a.clear();
    fire(&mut a, &map, g, LUCKY_CLUSTER).await;
    let credit = a
        .wait_for("the cluster credit", |m| match m {
            ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) => {
                Some((k.creature_id, k.kill_count))
            }
            _ => None,
        })
        .await;
    assert_eq!(credit, (CLUSTER_CREDIT, 1));
    let seen = created(&mut a, &[(180_861, 3), (180_864, 2)]).await;
    let (big_blue, big_white) = (of(&seen, 180_861), of(&seen, 180_864));
    assert!(
        big_blue
            .iter()
            .any(|(_, at)| near(*at, v(AT.0, AT.1, AT.2 + 12.0))),
        "the last twelve yards up: {big_blue:?}"
    );
    assert!(
        big_white
            .iter()
            .any(|(_, at)| near(*at, v(AT.0 + 5.25, AT.1 - 1.5, AT.2 + 7.5))),
        "{big_white:?}"
    );
    assert_eq!(realm.social.omen.lock().fireworks, 1, "Omen hears of it");
    // Not yet: three seconds on.
    let fortune = move |m: &Map| {
        crate::game::map::auras::auras(m, g)
            .is_some_and(|x| x.holders.iter().any(|h| h.spell == LUNAR_FORTUNE))
    };
    let (now, pending) = on(&map, move |m| {
        (fortune(m), m.objects.pending_spells.clone())
    })
    .await;
    assert!(!now, "not at once");
    assert!(
        pending.iter().any(|(_, _, s, _)| *s == LUNAR_FORTUNE),
        "{pending:?}"
    );
    on(&map, |m| {
        for p in m.objects.pending_spells.iter_mut() {
            p.0 = p.0.checked_sub(Duration::from_secs(3)).unwrap_or(p.0);
        }
    })
    .await;
    for _ in 0..100 {
        if on(&map, move |m| fortune(m)).await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no Lunar Fortune");
}

/// The real world's fireworks: every guy's rockets and his spell's summon
/// at its launcher (effect 41 at target 46, named by its script targets),
/// and the credit markers Lunar Fireworks asks for.
#[test]
#[ignore = "extended"]
fn the_real_fireworks_call_their_guys_to_the_launchers() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    for (spell, guy, launcher) in [
        (SMALL_RED_ROCKET_SPELL, RED_GUY, LAUNCHER),
        (26_327, 15_873, 180_859),
        (LUCKY_CLUSTER_SPELL, LUCKY, crate::game::omen::LAUNCHER),
    ] {
        let s = d.spells.get(spell).unwrap();
        let e = s.effects[1];
        assert_eq!(
            (e.effect, e.target_a, e.misc_value as u32),
            (crate::game::map::pets::EFFECT_SUMMON_WILD, 46, guy),
            "{spell}"
        );
        assert!(
            d.spells.script_targets[&spell].contains(&(0, launcher)),
            "{spell}"
        );
        assert!(d.creature_template(guy).is_some(), "{guy}");
        let l = fireworks::launch(guy, v(0.0, 0.0, 0.0)).unwrap();
        for (rocket, _) in l.fireworks {
            assert!(d.gameobject_templates.contains_key(&rocket), "{rocket}");
        }
    }
    let q = &d.npc.quests[&LUNAR_FIREWORKS];
    assert_eq!(
        &q.required_kills[..2],
        &[(ROCKET_CREDIT as i32, 8), (CLUSTER_CREDIT as i32, 2)]
    );
}

/// What a launch sends (the guy's create, the rockets' creates and their
/// despawns, the credit, Lunar Fortune) passes the client.
#[tokio::test]
#[ignore = "extended"]
async fn a_launchs_packets_pass_the_client() {
    use crate::game::tests_client_emu::{capture, checker, run_checker};
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let (_realm, mut a, g, map) =
        setup_with(world(crate::game::omen::LAUNCHER), Class::Warrior).await;
    ready(&map, g, LUCKY_CLUSTER).await;
    a.captured();
    fire(&mut a, &map, g, LUCKY_CLUSTER).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    on(&map, |m| {
        for p in m.objects.pending_spells.iter_mut() {
            p.0 = p.0.checked_sub(Duration::from_secs(3)).unwrap_or(p.0);
        }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (frames, counts) = capture(vec![&mut a]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "the creates were captured: {counts:?}"
    );
    run_checker(&check_py, &wow_exe, &frames);
}
