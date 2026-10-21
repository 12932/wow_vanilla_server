//! Players the server moves, and what's left of creature combat, through
//! the protocol: fear running a player with its client's control taken,
//! confusion's stagger, a creature's charm moving its player, civilians'
//! calls for the guards, creatures' mana in a fight, a dungeon's boss
//! fighting the whole dungeon, and Lord Kazzak's marks.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::map::combat::dist;
use crate::game::map::creature_combat;
use crate::game::map::player_motion::{self, Mode};
use crate::game::map::procs::{self, Event};
use crate::game::map::{Map, MapHandle, MapKey};

pub(crate) const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
pub(crate) const DUMMY: (u32, u32) = (3200, 40);
const PEASANT: (u32, u32) = (3201, 41);
const GUARD: (u32, u32) = (3202, 42);
const MAGE: (u32, u32) = (3203, 43);
const KAZZAK: (u32, u32) = (3204, 44);
const BOSS: u32 = 3205;

/// The town's side, and the human players'.
const TOWN: u32 = 29;
const HUMAN: u32 = 1;

pub(crate) const SCARE: u32 = 92_001;
const DAZE: u32 = 92_002;
const TWISTED_REFLECTION: u32 = 21063;
const TWISTED_REFLECTION_HEAL: u32 = 21064;
const MARK_OF_KAZZAK: u32 = 21056;
const MARK_OF_KAZZAK_EXPLOSION: u32 = 21058;

fn effect(effect: u32, base_points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect,
        base_points,
        base_dice: 1,
        die_sides: 1,
        target_a,
        ..Default::default()
    }
}

fn aura(kind: u32, base_points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        aura: kind,
        ..effect(6, base_points, target_a)
    }
}

fn spell(id: u32, name: &str, effects: [SpellEffect; 3]) -> SpellTemplate {
    SpellTemplate {
        id,
        name: name.into(),
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 9,
        effects,
        ..Default::default()
    }
}

fn creature(entry: u32, name: &str) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 20,
        level_max: 20,
        faction: 8,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        detection_range: 20.0,
        ..Default::default()
    }
}

pub(crate) fn data() -> GameData {
    let mut d = GameData::empty();
    d.player_level_stats.insert((1, 1, 1), [23, 20, 22, 20, 21]);
    d.player_class_stats.insert((1, 1), (60, 0));
    for class in [1, 8] {
        d.class_level_stats.insert(
            (class, 20),
            ClassLevelStats {
                health: 10_000,
                melee_damage: 1.0,
                mana: 1000,
                intellect: 100,
                spirit: 40,
                ..Default::default()
            },
        );
    }
    // The town hates the humans, who are of no side with the dummies.
    d.faction_templates.insert(
        TOWN,
        FactionTemplate {
            id: TOWN,
            faction: 76,
            ours: 0x4,
            friend_mask: 0x4,
            enemy_mask: 0x3,
            ..Default::default()
        },
    );
    d.faction_templates.insert(
        HUMAN,
        FactionTemplate {
            id: HUMAN,
            faction: 1,
            ours: 0x3,
            friend_mask: 0x2,
            enemy_mask: 0xC,
            ..Default::default()
        },
    );
    d.creature_templates
        .insert(DUMMY.0, creature(DUMMY.0, "Dummy"));
    let mut peasant = creature(PEASANT.0, "Peon");
    peasant.faction = TOWN;
    peasant.civilian = true;
    // It fights back, but starts no fight: it calls the guards.
    peasant.flags_extra = 0x2;
    peasant.static_flags1 = 0x0800_0000;
    peasant.detection_range = 10.0;
    d.creature_templates.insert(PEASANT.0, peasant);
    let mut guard = creature(GUARD.0, "Grunt");
    guard.faction = TOWN;
    // It waits to be called.
    guard.flags_extra = 0x400 | 0x2;
    guard.civilian = true;
    d.creature_templates.insert(GUARD.0, guard);
    let mut mage = creature(MAGE.0, "Conjurer");
    mage.unit_class = 8;
    d.creature_templates.insert(MAGE.0, mage);
    d.creature_templates
        .insert(KAZZAK.0, creature(KAZZAK.0, "Lord Kazzak"));
    let mut boss = creature(BOSS, "Rhahk'Zor");
    boss.rank = 3;
    boss.static_flags2 = 0x2;
    d.creature_templates.insert(BOSS, boss);
    let spawn = |guid, entry, map, x: f32, y: f32, z: f32| CreatureSpawn {
        guid,
        entries: vec![entry],
        map,
        x,
        y,
        z,
        respawn_min: 1,
        respawn_max: 1,
        ..Default::default()
    };
    let near =
        |guid, entry, dx: f32, dy: f32| spawn(guid, entry, 0, START.0 + dx, START.1 + dy, START.2);
    d.creature_spawns.insert(
        0,
        vec![
            near(DUMMY.1, DUMMY.0, 2.0, 0.0),
            near(PEASANT.1, PEASANT.0, -2.0, 25.0),
            near(GUARD.1, GUARD.0, -2.0, 40.0),
            near(MAGE.1, MAGE.0, 2.0, -15.0),
            near(KAZZAK.1, KAZZAK.0, -2.0, -15.0),
        ],
    );
    d.creature_spawns
        .insert(36, vec![spawn(79_000, BOSS, 36, 3.0, 0.0, 0.0)]);
    d.maps.insert(
        36,
        MapInfo {
            id: 36,
            internal_name: "DeadminesInstance".into(),
            name: "The Deadmines".into(),
            instance_type: 1,
            player_limit: 0,
        },
    );
    let s = &mut d.spells;
    s.ranges.insert(1, (0.0, 30.0));
    s.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 30_000,
            ..Default::default()
        },
    );
    let none = SpellEffect::default();
    let mut scare = spell(SCARE, "Scare", [aura(7, 0, 6), none, none]);
    scare.school = 5;
    let daze = spell(DAZE, "Daze", [aura(5, 0, 6), none, none]);
    let mut reflection = spell(
        TWISTED_REFLECTION,
        "Twisted Reflection",
        [aura(4, 0, 6), none, none],
    );
    reflection.proc_flags = procs::PROC_TAKEN_MELEE | procs::PROC_TAKEN_NEGATIVE_SPELL;
    reflection.proc_chance = 100;
    let heal = spell(
        TWISTED_REFLECTION_HEAL,
        "Twisted Reflection",
        [effect(10, 24_999, 6), none, none],
    );
    let mut mark = spell(
        MARK_OF_KAZZAK,
        "Mark of Kazzak",
        [aura(64, 499, 6), none, none],
    );
    mark.effects[0].amplitude = 1000;
    mark.effects[0].multiple_value = 1.0;
    let mut explosion = spell(
        MARK_OF_KAZZAK_EXPLOSION,
        "Mark of Kazzak",
        [effect(2, 49, 1), none, none],
    );
    explosion.school = 5;
    for t in [scare, daze, reflection, heal, mark, explosion] {
        s.spells.insert(t.id, t);
    }
    d
}

async fn setup() -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    setup_as(Class::Warrior).await
}

/// [`setup`], with a player of `class`.
async fn setup_as(class: Class) -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, class).await;
    a.sees(unit(DUMMY)).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, map)
}

pub(crate) fn unit(c: (u32, u32)) -> Guid {
    guid::unit(c.0, c.1)
}

fn start() -> Vector3d {
    Vector3d {
        x: START.0,
        y: START.1,
        z: START.2,
    }
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

pub(crate) fn apply(m: &mut Map, caster: Guid, target: Guid, spell: u32, amount: i32) {
    let data = m.realm.data.clone();
    auras::apply_spell(
        m,
        caster,
        target,
        data.spells.get(spell).unwrap(),
        &[(0, amount)],
        Default::default(),
    )
    .unwrap();
}

/// The player's next walk: its mode, and where it goes.
fn next_walk(m: &mut Map, g: Guid) -> Option<(Mode, Vector3d, bool)> {
    let now = m.now;
    if let Some(d) = m.players.get_mut(&g).and_then(|p| p.driven.as_mut()) {
        d.walk = None;
        d.next_move_at = now;
    }
    player_motion::update(m);
    let d = m.players.get(&g)?.driven.as_ref()?;
    let w = d.walk.as_ref()?;
    Some((d.mode, w.destination(), w.run))
}

pub(crate) fn control(m: &ServerOpcodeMessage, g: Guid) -> Option<bool> {
    match m {
        ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(c) if c.guid == g => Some(c.allow_movement),
        _ => None,
    }
}

#[tokio::test]
async fn fear_takes_a_players_control_and_runs_them() {
    let (_realm, mut a, ga, map) = setup().await;
    let dummy = unit(DUMMY);
    map.query(move |m| apply(m, dummy, ga, SCARE, 0)).await;
    let allowed = a.wait_for("control taken", |m| control(m, ga)).await;
    assert!(!allowed);
    // Its own client is told where it runs.
    a.wait_for("the run", |m| match m {
        ServerOpcodeMessage::SMSG_MONSTER_MOVE(mv)
            if mv.guid == ga
                && !matches!(mv.move_type, SMSG_MONSTER_MOVE_MonsterMoveType::Stop) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    // What its client says of where it is goes unheeded.
    let elsewhere = (START.0 + 30.0, START.1, START.2);
    a.heartbeat(elsewhere.0, elsewhere.1, elsewhere.2).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (at, driven) = map
        .query(move |m| {
            let p = &m.players[&ga];
            (p.position(), p.driven.is_some())
        })
        .await
        .unwrap();
    assert!(driven);
    assert!(dist(at, start()) > 0.5, "it ran: {at:?}");
    let told = Vector3d {
        x: elsewhere.0,
        y: elsewhere.1,
        z: elsewhere.2,
    };
    assert!(dist(at, told) > 1.0, "not where its client said");
    // The fear ends: control comes back.
    map.query(move |m| auras::remove_spell(m, ga, SCARE, None))
        .await;
    let allowed = a.wait_for("control back", |m| control(m, ga)).await;
    assert!(allowed);
    let driven = map
        .query(move |m| m.players[&ga].driven.is_some())
        .await
        .unwrap();
    assert!(!driven);
}

#[tokio::test]
async fn a_feared_players_runs_go_further_the_nearer_the_frightener() {
    let mut rng = rand::rng();
    let at = start();
    for _ in 0..50 {
        let near = dist(player_motion::fear_destination(at, 2.0, &mut rng), at);
        assert!((10.0..=25.01).contains(&near), "{near}");
        let far = dist(player_motion::fear_destination(at, 40.0, &mut rng), at);
        assert!((4.0..=10.01).contains(&far), "{far}");
    }
}

#[tokio::test]
async fn confusion_staggers_a_player_about_where_it_began() {
    let (_realm, mut a, ga, map) = setup().await;
    let dummy = unit(DUMMY);
    let walks = map
        .query(move |m| {
            apply(m, dummy, ga, DAZE, 0);
            (0..20).map(|_| next_walk(m, ga)).collect::<Vec<_>>()
        })
        .await
        .unwrap();
    assert!(!a.wait_for("control taken", |m| control(m, ga)).await);
    for w in walks {
        let (mode, to, run) = w.expect("a stagger");
        assert!(matches!(mode, Mode::Confusion { .. }));
        assert!(!run, "it walks");
        assert!(
            dist(to, start()) <= player_motion::CONFUSION_RADIUS + 0.01,
            "{to:?}"
        );
    }
}

#[tokio::test]
async fn a_player_a_creature_charms_follows_it_and_closes_on_its_foes() {
    let (realm, mut a, ga, map) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let dummy = unit(DUMMY);
    let (follow, master_at, chase, bob_at) = map
        .query(move |m| {
            let master_at = Vector3d {
                x: START.0 + 15.0,
                ..start()
            };
            m.creature_mut(dummy).unwrap().position = master_at;
            crate::game::map::pets::charmed(m, dummy, ga, false);
            let follow = next_walk(m, ga);
            // A foe ten yards the other way.
            let bob_at = Vector3d {
                x: START.0 - 10.0,
                ..start()
            };
            m.players.get_mut(&gb).unwrap().movement.position = bob_at;
            let p = m.players.get_mut(&ga).unwrap();
            p.combat.attacking = true;
            p.combat.victim = gb;
            let chase = next_walk(m, ga);
            (follow, master_at, chase, bob_at)
        })
        .await
        .unwrap();
    assert!(!a.wait_for("control taken", |m| control(m, ga)).await);
    let (mode, to, run) = follow.expect("following");
    assert_eq!(mode, Mode::Charm);
    assert!(run);
    assert!(dist(to, master_at) <= 3.5, "beside its charmer: {to:?}");
    let (_, to, _) = chase.expect("chasing");
    assert!(dist(to, bob_at) <= 5.0, "at its foe: {to:?}");
}

#[tokio::test]
async fn a_creature_regains_mana_in_a_fight_unless_it_cast_lately() {
    let (_realm, _a, ga, map) = setup().await;
    let mage = unit(MAGE);
    let (per_tick, after, held) = map
        .query(move |m| {
            let now = m.now;
            crate::game::map::ai::attacked(m, mage, ga);
            let set = |m: &mut Map, mana: u32| {
                let c = m.creature_mut(mage).unwrap();
                c.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
                c.fields.set_u32(UNIT_FIELD_POWER1, mana);
                c.engagement.regen_at = Some(now);
            };
            set(m, 100);
            creature_combat::regenerate(m);
            let mana = |m: &Map| m.unit_fields(mage).unwrap().get_u32(UNIT_FIELD_POWER1);
            let after = mana(m);
            set(m, 100);
            m.creature_mut(mage).unwrap().engagement.mana_spent_at = Some(now);
            creature_combat::regenerate(m);
            (
                creature_combat::combat_mana_per_tick(m, mage),
                after,
                mana(m),
            )
        })
        .await
        .unwrap();
    // (spirit / 4 + 12.5) / 2 + 0.6 √intellect / 5, for five seconds.
    assert_eq!(per_tick, 62);
    assert_eq!(after, 162);
    assert_eq!(held, 100, "the five-second rule");
}

#[tokio::test]
async fn a_dungeon_boss_fights_everyone_in_its_dungeon() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
        _ => None,
    })
    .await;
    for c in [&mut a, &mut b] {
        let g = c.guid;
        realm.registry.online(g).unwrap().map().call(move |m| {
            m.teleport(
                g,
                MapKey::world(36),
                Vector3d {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                },
                0.0,
            )
        });
        c.wait_for("SMSG_NEW_WORLD", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
        })
        .await;
        c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
        c.sees(g).await;
    }
    let dungeon = realm.registry.online(ga).unwrap().map();
    let boss = guid::unit(BOSS, 79_000);
    a.sees(boss).await;
    let in_fight = move |m: &Map, g: Guid| {
        m.players[&g].fields.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        ) && m
            .creature(boss)
            .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == g))
    };
    let (both, back) = dungeon
        .query(move |m| {
            crate::game::map::ai::attacked(m, boss, ga);
            let both = in_fight(m, ga) && in_fight(m, gb);
            // Bob slips out; the next pulse draws him back in.
            crate::game::map::ai::forget(m, gb);
            crate::game::map::combat::leave_combat(m, gb);
            let now = m.now;
            m.creature_mut(boss).unwrap().engagement.zone_pulse_at = Some(now);
            creature_combat::update(m);
            (both, in_fight(m, gb))
        })
        .await
        .unwrap();
    assert!(both, "Bob is in the fight too");
    assert!(back, "and drawn back in");
}

#[tokio::test]
async fn lord_kazzaks_marks_heal_him_and_explode() {
    // A mage: the mark drains only a pool its bearer shows.
    let (_realm, _a, ga, map) = setup_as(Class::Mage).await;
    let kazzak = unit(KAZZAK);
    let (healed, marked, exploded) = map
        .query(move |m| {
            m.creature_mut(kazzak)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 1000);
            apply(m, kazzak, ga, TWISTED_REFLECTION, 0);
            // His blow on the reflected heals him.
            procs::trigger(
                m,
                Event {
                    actor: kazzak,
                    target: ga,
                    actor_flags: procs::PROC_DONE_MELEE,
                    target_flags: procs::PROC_TAKEN_MELEE,
                    ex: procs::EX_NORMAL_HIT,
                    damage: 10,
                    school: 1,
                    ..Default::default()
                },
            );
            let healed = health(m, kazzak);
            // The mark drains her mana dry, and goes off.
            apply(m, kazzak, ga, MARK_OF_KAZZAK, 499);
            let now = m.now;
            let p = m.players.get_mut(&ga).unwrap();
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
            p.fields.set_u32(UNIT_FIELD_POWER1, 300);
            let before = p.fields.get_u32(UNIT_FIELD_HEALTH);
            for h in p
                .auras
                .holders
                .iter_mut()
                .filter(|h| h.spell == MARK_OF_KAZZAK)
            {
                for x in h.effects.iter_mut() {
                    x.next_tick = Some(now);
                }
            }
            auras::update(m);
            let marked = m.players[&ga]
                .auras
                .holders
                .iter()
                .any(|h| h.spell == MARK_OF_KAZZAK);
            (healed, marked, health(m, ga) < before)
        })
        .await
        .unwrap();
    assert!(healed > 1000, "{healed}");
    assert!(!marked, "the mark is gone");
    assert!(exploded);
}
