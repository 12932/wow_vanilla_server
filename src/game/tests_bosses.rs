//! The encounter mechanics Thistle Tea runs by spell (map/boss_mechanics.rs),
//! on the real world database and spells.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

fn realm() -> Option<Arc<Realm>> {
    let dir = DB.as_path();
    if !dir.join("vmangos.sqlite").exists() {
        return None;
    }
    Some(harness::realm_with(crate::game::harness::load_real(dir)))
}

async fn player(realm: &Arc<Realm>, name: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, name);
    let g = c.enter(name, Race::Human, Class::Warrior).await;
    // Past the intro's camera, back in their own eyes.
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

/// A creature of `entry` at `d` yards east of the start, facing east.
async fn boss(map: &MapHandle, entry: u32, dx: f32, dy: f32) -> Guid {
    map.query(move |m| {
        let at = Vector3d {
            x: START.0 + dx,
            y: START.1 + dy,
            z: START.2,
        };
        let g = crate::game::map::scripts::summon(m, Guid::zero(), entry, at, 0.0, None)
            .expect("the creature");
        // It neither swings nor casts on its own, so those it fights live.
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, g) {
            s.summoner = None;
            s.passive = true;
            s.no_melee = true;
            s.spell_list = Some(0);
        }
        // Its scripts don't run here: what is tested is the spell.
        crate::game::map::scripts::stop_for(m, g);
        g
    })
    .await
    .unwrap()
}

/// The bosses a part summoned go, so the next part starts clear.
fn clear(map: &MapHandle, bosses: Vec<Guid>) {
    map.call(move |m| {
        for b in bosses {
            crate::game::map::scripts::despawn(m, b);
        }
    });
}

fn put(map: &MapHandle, g: Guid, dx: f32, dy: f32) {
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&g) {
            p.movement.position = Vector3d {
                x: START.0 + dx,
                y: START.1 + dy,
                z: START.2,
            };
        }
    });
}

fn has(map: &MapHandle, g: Guid, spell: u32) -> impl std::future::Future<Output = bool> {
    let map = map.clone();
    async move {
        map.query(move |m| crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(spell)))
            .await
            .unwrap()
    }
}

fn cast(map: &MapHandle, caster: Guid, spell: u32, target: Guid) {
    map.call(move |m| crate::game::map::spells::cast_triggered(m, caster, spell, Some(target)));
}

/// Golemagg's Trust, his template's aura, strengthens only the Core
/// Ragers near him, and only while he fights; home from evading, he has
/// it back.
async fn golemaggs_trust_and_separation_anxiety(realm: &Arc<Realm>) {
    let (_a, alice, map) = player(realm, "Gale").await;
    let golemagg = boss(&map, 11988, 5.0, 0.0).await;
    let near = boss(&map, 11672, 8.0, 0.0).await;
    let far = boss(&map, 11672, 45.0, 0.0).await;
    assert!(has(&map, golemagg, 20556).await, "his template's aura");
    // The ragers' own (their template's two seconds of it) lapses.
    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert!(!has(&map, near, 20553).await, "not while he is at peace");
    map.call(move |m| crate::game::map::ai::attacked(m, golemagg, alice));
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert!(has(&map, near, 20553).await, "the rager near him");
    assert!(!has(&map, far, 20553).await, "not the one far off");
    assert!(!has(&map, alice, 20553).await, "nor his enemy");
    map.call(move |m| crate::game::map::ai::enter_evade(m, golemagg));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(has(&map, golemagg, 20556).await, "back once home");
    // Separation Anxiety: Garr's Firesworn far from him grows stronger.
    let garr = boss(&map, 12057, -20.0, 0.0).await;
    let close = boss(&map, 12099, -22.0, 0.0).await;
    let strayed = boss(&map, 12099, -23.0, 2.0).await;
    // Garr marks the adds around him; one strays.
    cast(&map, garr, 23487, garr);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(has(&map, close, 23487).await && has(&map, strayed, 23487).await);
    map.call(move |m| {
        let at = Vector3d {
            x: START.0 - 140.0,
            y: START.1,
            z: START.2,
        };
        crate::game::map::scripts::place(m, strayed, at, 0.0);
    });
    tokio::time::sleep(Duration::from_millis(6200)).await;
    assert!(has(&map, strayed, 23492).await, "the strayed one");
    assert!(!has(&map, close, 23492).await, "not the one by him");
    clear(&map, vec![golemagg, near, far, garr, close, strayed]);
}

/// Baron Geddon's Inferno pulses 500, 500, then 1000.
async fn geddons_inferno_ramps(realm: &Arc<Realm>) {
    let (mut a, alice, map) = player(realm, "Ignis").await;
    let geddon = boss(&map, 12056, 3.0, 0.0).await;
    a.sees(geddon).await;
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&alice) {
            p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 10_000_000);
            p.fields.set_u32(UNIT_FIELD_HEALTH, 10_000_000);
        }
    });
    cast(&map, geddon, 19695, geddon);
    let mut pulses = Vec::new();
    for _ in 0..3 {
        let hit = a
            .wait_for("an Inferno pulse", |m| match m {
                ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d)
                    if d.spell == 19698 && d.target == alice =>
                {
                    Some(d.damage + d.resisted + d.absorbed_damage)
                }
                _ => None,
            })
            .await;
        pulses.push(hit);
    }
    assert_eq!(pulses, vec![500, 500, 1000], "each pulse hotter");
    clear(&map, vec![geddon]);
}

/// The whirl's cone that hits `g`, if one does soon.
async fn whirl_hit(c: &mut Client, g: Guid) -> Option<u32> {
    c.gets(Duration::from_millis(400), move |m| match m {
        ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d)
            if d.target == g && (24820..=24838).contains(&d.spell) =>
        {
            Some(d.spell)
        }
        _ => None,
    })
    .await
}

/// Lord Kazzak's Shadow Bolt Whirl turns: its eighth cone faces him, its
/// second a quarter turn to his left.
async fn kazzaks_whirl_turns(realm: &Arc<Realm>) {
    let (mut a, ahead, map) = player(realm, "Ahead").await;
    let (mut b, left, _) = player(realm, "Left").await;
    let kazzak = boss(&map, 12397, 0.0, 0.0).await;
    put(&map, ahead, 6.0, 0.0);
    put(&map, left, 0.0, 6.0);
    a.sees(kazzak).await;
    b.sees(kazzak).await;
    // The whirl's aura, taken off once it is had, so only the ticks
    // called here turn it.
    cast(&map, kazzak, 24834, kazzak);
    let (h, x) = map
        .query(move |m| {
            let a = crate::game::map::auras::auras(m, kazzak)?;
            let h = a.holders.iter().find(|h| h.spell == 24834)?.clone();
            let x = h.effects.first()?.clone();
            crate::game::map::auras::remove_spell(m, kazzak, 24834, None);
            Some((h, x))
        })
        .await
        .unwrap()
        .expect("the whirl's aura");
    let tick = |ticks: u32| {
        let (map, h, mut x) = (map.clone(), h.clone(), x.clone());
        async move {
            x.ticks = ticks;
            map.call(move |m| {
                crate::game::map::boss_mechanics::periodic_trigger(m, kazzak, &h, &x);
            });
        }
    };
    // The eighth tick (and every eighth after): straight ahead.
    tick(7).await;
    assert_eq!(whirl_hit(&mut a, ahead).await, Some(24820), "ahead");
    assert_eq!(whirl_hit(&mut b, left).await, None, "not to his left");
    // The second: a quarter turn on.
    tick(1).await;
    assert_eq!(whirl_hit(&mut b, left).await, Some(24822), "to his left");
    assert_eq!(whirl_hit(&mut a, ahead).await, None);
    clear(&map, vec![kazzak]);
}

/// The Emerald Dream's dragons: their aura stuns whom they marked, a
/// player dying under the mark's aura keeps the mark, and one dying under
/// Emeriss's leaves a mushroom; Azuregos's Arcane Vacuum pulls a player
/// to him and forgets them.
async fn the_dragons_marks_and_vacuum(realm: &Arc<Realm>) {
    let (mut a, alice, map) = player(realm, "Daria").await;
    let dragon = boss(&map, 14887, 4.0, 0.0).await;
    a.sees(dragon).await;
    cast(&map, alice, 25040, alice);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(has(&map, alice, 25040).await, "marked");
    cast(&map, dragon, 25044, dragon);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(has(&map, alice, 25043).await, "the marked are stunned");
    // Dying under the dragon's mark aura leaves the mark.
    let (_b, bob, _) = player(realm, "Bob").await;
    cast(&map, dragon, 25042, bob);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(has(&map, bob, 25042).await);
    map.call(move |m| crate::game::map::combat::deal_damage(m, dragon, bob, 1_000_000));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(has(&map, bob, 25040).await, "the fallen keep the mark");
    // Dying under Emeriss's aura leaves a Putrid Mushroom where one fell.
    let (_e, emma, _) = player(realm, "Emma").await;
    put(&map, emma, 4.0, 8.0);
    cast(&map, dragon, 24906, emma);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(has(&map, emma, 24906).await);
    map.call(move |m| crate::game::map::combat::deal_damage(m, dragon, emma, 1_000_000));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mushroom = map
        .query(move |m| {
            let at = m.players[&emma].position();
            m.gameobjects.iter().any(|(_, o)| {
                o.entry == 180517 && crate::game::map::combat::dist(o.position, at) < 3.0
            })
        })
        .await
        .unwrap();
    assert!(mushroom, "a mushroom where they fell");
    // Azuregos's Arcane Vacuum.
    let (_c, cara, _) = player(realm, "Cara").await;
    let azuregos = boss(&map, 6109, -30.0, 0.0).await;
    put(&map, cara, -30.0, 25.0);
    map.call(move |m| {
        crate::game::map::ai::add_threat(m, azuregos, cara, 500.0);
    });
    cast(&map, azuregos, 21147, azuregos);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (threat, near) = map
        .query(move |m| {
            let c = m.creature(azuregos).unwrap();
            let threat = c
                .threat
                .iter()
                .find(|(g, _)| *g == cara)
                .map_or(0.0, |(_, t)| *t);
            let near =
                crate::game::map::combat::dist(m.players[&cara].position(), c.position) < 10.0;
            (threat, near)
        })
        .await
        .unwrap();
    assert_eq!(threat, 0.0, "forgotten");
    assert!(near, "pulled to him");
    // Darkmaster Gandling's Shadow Portal sends one off to a room, and
    // he forgets them.
    let (mut d, dan, _) = player(realm, "Dan").await;
    let gandling = boss(&map, 1853, 0.0, -20.0).await;
    put(&map, dan, 2.0, -20.0);
    map.call(move |m| crate::game::map::ai::add_threat(m, gandling, dan, 500.0));
    cast(&map, gandling, 17950, dan);
    d.wait_for("the portal's pull", |m| match m {
        ServerOpcodeMessage::SMSG_TRANSFER_PENDING(_)
        | ServerOpcodeMessage::SMSG_NEW_WORLD(_)
        | ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_) => Some(()),
        _ => None,
    })
    .await;
    let threat = map
        .query(move |m| {
            m.creature(gandling)
                .unwrap()
                .threat
                .iter()
                .find(|(g, _)| *g == dan)
                .map_or(0.0, |(_, t)| *t)
        })
        .await
        .unwrap();
    assert_eq!(threat, 0.0, "Gandling forgets them");
    clear(&map, vec![dragon, azuregos, gandling]);
}

/// A spawn's addon's auras are on it from the first: Battle Stance on a
/// warrior's whose addon names it.
async fn spawn_addon_auras(realm: &Arc<Realm>) {
    let (_a, _, map) = player(realm, "Stance").await;
    let held = map
        .query(|m| {
            let data = m.realm.data.clone();
            m.creatures
                .iter()
                .find(|(_, c)| {
                    c.alive()
                        && c.spawn_guid
                            .and_then(|s| data.creature_addons.get(&s))
                            .is_some_and(|a| a.auras == [7165])
                })
                .map(|(_, c)| c.auras.has_spell(7165))
        })
        .await
        .unwrap();
    assert_eq!(held, Some(true), "Battle Stance from its addon");
}

/// The mechanics on one load of the real world.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_encounter_mechanics() {
    let Some(realm) = realm() else {
        return;
    };
    spawn_addon_auras(&realm).await;
    geddons_inferno_ramps(&realm).await;
    kazzaks_whirl_turns(&realm).await;
    golemaggs_trust_and_separation_anxiety(&realm).await;
    the_dragons_marks_and_vacuum(&realm).await;
}
