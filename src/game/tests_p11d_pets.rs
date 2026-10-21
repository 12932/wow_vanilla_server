//! Round 11's 11d, pet AI and control (docs/parity/matrix.csv): a pet's
//! autocast reaches as far as its master's range modifiers take it
//! (Thistle Tea's `range_test.exs` "pet target discovery includes inherited
//! range bonuses"); an aggressive pet notices a foe as far as its level
//! lets it, in its sight and not far above it (`ai_environment_test.exs`);
//! a possessed unit's movement isn't told back to the client that moved it
//! (`event_sink/movement_test.exs`, `possession_owner_test.exs`); and a
//! pet's harmful bar spell goes as mangos-classic's `HandlePetAction` has
//! it (`spell_combat_control_dbc_test.exs`, the user's call).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage, ServerOpcodeMessage};
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{self, AURA_ADD_FLAT_MODIFIER, AURA_ADD_PCT_MODIFIER};
use crate::game::map::creature::{Life, Motion};
use crate::game::map::spells::mods;
use crate::game::map::{Map, MapHandle, pets};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{GORE, cast, effect, enter, learn, spell};
use crate::game::tests_pets2::{MIND_CONTROL, bring, control_update, flag_pvp, more};

/// A bolt of 30 yards in a family's spells; its master's five yards more
/// and half as far again for them.
const BOLT: u32 = 97_400;
const FARTHER: u32 = 97_401;
const HALF_AGAIN: u32 = 97_402;
const RANGE_30: u32 = 974;
const FAMILY: u32 = 3;
const EYE_OF_KILROGG: u32 = 126;

/// [`more`], its Eye of Kilrogg a minute long.
fn eyes() -> GameData {
    let mut d = more();
    let eye = d.spells.spells.get_mut(&EYE_OF_KILROGG).unwrap();
    eye.duration_index = LONG;
    d.spells.durations.insert(
        LONG,
        crate::game::data::spells::SpellDuration {
            base: 60_000,
            ..Default::default()
        },
    );
    d
}

const LONG: u32 = 974;

fn world() -> GameData {
    let mut d = crate::game::tests_pets::data();
    d.spells.ranges.insert(RANGE_30, (0.0, 30.0));
    let none = SpellEffect::default();
    let mut bolt = spell(BOLT, "Bolt", [effect(2, 9, 6), none, none]);
    bolt.range_index = RANGE_30;
    bolt.family_name = FAMILY;
    bolt.family_flags = 0x1;
    bolt.recovery_time = 10_000;
    d.spells.spells.insert(BOLT, bolt);
    for (id, kind, amount) in [
        (FARTHER, AURA_ADD_FLAT_MODIFIER, 5),
        (HALF_AGAIN, AURA_ADD_PCT_MODIFIER, 50),
    ] {
        let mut s = spell(
            id,
            "Range",
            [
                SpellEffect {
                    aura: kind,
                    misc_value: mods::OP_RANGE as i32,
                    item_type: 0x1,
                    ..effect(6, amount - 1, 1)
                },
                none,
                none,
            ],
        );
        s.family_name = FAMILY;
        s.duration_index = 0;
        d.spells.spells.insert(id, s);
    }
    d
}

/// The pet thinks once, a second on, free to cast.
fn think(m: &mut Map, pet: Guid) {
    m.now += Duration::from_secs(1);
    if let Some(c) = m.caster_mut(pet) {
        c.casting = None;
        c.gcd_until = None;
    }
    let now = m.now;
    pets::state_mut(m, pet).unwrap().next_think = now;
    crate::game::map::spells::update(m);
    pets::update(m);
}

/// `g` put at `p`, standing.
fn put(m: &mut Map, g: Guid, p: Vector3d) {
    let c = m.creature_mut(g).unwrap();
    let old = c.cell;
    c.position = p;
    c.motion = Motion::Idle { until: None };
    c.cell = crate::game::map::grid::cell_of(p.x, p.y);
    let new = c.cell;
    m.grid.relocate(g, old, new);
}

fn east(p: Vector3d, by: f32) -> Vector3d {
    Vector3d { x: p.x + by, ..p }
}

fn cooling(m: &Map, g: Guid, spell: u32) -> bool {
    m.caster(g)
        .and_then(|c| c.cooldowns.get(&spell))
        .is_some_and(|t| *t > m.now)
}

fn victim(m: &Map, g: Guid) -> Option<Guid> {
    m.combat(g).filter(|c| c.attacking).map(|c| c.victim)
}

fn order(m: &mut Map, owner: Guid, pet: Guid, data: u32, target: Guid) {
    pets::handle(
        m,
        owner,
        ClientOpcodeMessage::CMSG_PET_ACTION(CMSG_PET_ACTION { pet, data, target }),
    );
}

/// Thistle Tea's "pet target discovery includes inherited range bonuses":
/// a hunter's pet casts its autocast bolt of 30 yards at a wolf 45 off
/// once its master has five yards more and half as far again for the
/// bolt's family (52.5), as its cast takes its master's modifiers
/// (mangos's `GetSpellModOwner`); without them, it doesn't from there.
#[tokio::test]
async fn a_pets_autocast_reaches_as_far_as_its_masters_range_bonuses() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            {
                let s = pets::state_mut(m, pet).unwrap();
                s.spells.push(BOLT);
                s.autocast = vec![BOLT];
                s.react = pets::REACT_PASSIVE;
            }
            let here = m.position_of(pet).unwrap();
            let place = |m: &mut Map| {
                put(m, pet, here);
                put(m, wolf, east(here, 45.0));
                pets::attack_target(m, pet, wolf);
            };
            place(m);
            think(m, pet);
            let bare = cooling(m, pet, BOLT);
            for id in [FARTHER, HALF_AGAIN] {
                let s = m.realm.data.spells.get(id).cloned().unwrap();
                let amount = if id == FARTHER { 5 } else { 50 };
                auras::apply(m, gh, gh, &s, 0, amount);
            }
            place(m);
            think(m, pet);
            (bare, cooling(m, pet, BOLT))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (false, true),
        "not from 45 yards bare; from there with its master's bonuses"
    );
}

/// Thistle Tea's "aggressive pets observe and check sight beyond twenty
/// yards", with mangos's `PetAI::MoveInLineOfSight`: an aggressive pet
/// notices a foe within its attack distance at the foe's level (its
/// detection range, 20, and a yard for each level it has over the foe:
/// 39 for a level 20 pet and a level 1 wolf), so one 30 yards off; not
/// one 30 off of level 15 (25 yards), nor one 30 off and 10 above it.
#[tokio::test]
async fn an_aggressive_pet_notices_a_foe_as_far_as_its_level_lets_it() {
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(world()).await;
    let out = map
        .query(move |m| {
            m.creature_mut(pet)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_LEVEL, 20);
            pets::state_mut(m, pet).unwrap().react = pets::REACT_AGGRESSIVE;
            let here = m.players[&gh].position();
            let mut tries = Vec::new();
            for (level, up) in [(15, 0.0), (1, 10.0), (1, 0.0)] {
                crate::game::map::combat::stop_attack(m, pet);
                put(m, pet, here);
                put(
                    m,
                    wolf,
                    Vector3d {
                        z: here.z + up,
                        ..east(here, 30.0)
                    },
                );
                m.creature_mut(wolf)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_FIELD_LEVEL, level);
                think(m, pet);
                tries.push(victim(m, pet));
            }
            tries
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![None, None, Some(wolf)],
        "not the level 15 wolf nor the one above it; the level 1 one"
    );
}

async fn heartbeat_of(c: &mut Client, unit: Guid, within: Duration) -> Option<Vector3d> {
    c.gets(within, |m| match m {
        ServerOpcodeMessage::MSG_MOVE_HEARTBEAT(h) if h.guid == unit => Some(h.info.position),
        _ => None,
    })
    .await
}

async fn until(map: &MapHandle, what: &str, f: impl Fn(&Map) -> bool + Send + Clone + 'static) {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let f = f.clone();
        if on(map, move |m| f(m)).await {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Thistle Tea's "acknowledged possession launches reach observers once
/// and never relaunch the controller", by mangos's `HandleMovementOpcodes`
/// (`SendMessageToAllWhoSeeMeMove(data, _player)`): the warlock's Eye of
/// Kilrogg, moved by her client, is told to one who watches it once, and
/// not back to her; dead, it isn't moved; let go (her client's own
/// mover again), her movement doesn't move it.
#[tokio::test]
async fn a_possessed_creatures_move_reaches_its_watchers_once_and_not_its_controller() {
    let realm = harness::realm_with(eyes());
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    let (mut o, go, _) = enter(&realm, "O", "Onlooker", Race::Human, Class::Warrior).await;
    bring(&map, go, gw, 2.0).await;
    learn(&map, gw, &[EYE_OF_KILROGG]);
    cast(&w, EYE_OF_KILROGG, None).await;
    let eye = w
        .wait_for("control of the eye", |m| match m {
            ServerOpcodeMessage::SMSG_CLIENT_CONTROL_UPDATE(u)
                if u.guid != gw && u.allow_movement =>
            {
                Some(u.guid)
            }
            _ => None,
        })
        .await;
    o.sees(eye).await;
    w.send(CMSG_SET_ACTIVE_MOVER { guid: eye }).await;
    until(&map, "her client moves the eye", move |m| {
        m.players[&gw].owned.mover == Some(eye)
    })
    .await;
    let at = on(&map, move |m| m.position_of(eye).unwrap()).await;
    w.clear();
    o.clear();
    w.heartbeat(at.x + 3.0, at.y, at.z).await;
    let told = heartbeat_of(&mut o, eye, harness::WAIT).await;
    assert_eq!(told.map(|p| p.x), Some(at.x + 3.0), "the onlooker is told");
    assert!(
        heartbeat_of(&mut o, eye, NONE_WITHIN).await.is_none(),
        "once"
    );
    assert!(
        heartbeat_of(&mut w, eye, NONE_WITHIN).await.is_none(),
        "the warlock's client isn't told back"
    );
    // Dead, it isn't moved, nor told.
    on(&map, move |m| {
        let now = m.now;
        m.creature_mut(eye).unwrap().life = Life::Dead {
            corpse_until: now + Duration::from_secs(600),
            respawn_at: now + Duration::from_secs(600),
        };
    })
    .await;
    w.heartbeat(at.x + 6.0, at.y, at.z).await;
    assert!(heartbeat_of(&mut o, eye, NONE_WITHIN).await.is_none());
    let x = on(&map, move |m| m.position_of(eye).map(|p| p.x)).await;
    assert!(x.is_none_or(|x| x == at.x + 3.0), "the dead eye stays");
    // Let go, her movement is her own.
    on(&map, move |m| {
        if let Some(c) = m.creature_mut(eye) {
            c.life = Life::Alive;
        }
        m.players.get_mut(&gw).unwrap().owned.mover = None;
    })
    .await;
    let me = on(&map, move |m| m.players[&gw].position()).await;
    w.heartbeat(me.x + 1.0, me.y, me.z).await;
    until(&map, "she moves", move |m| {
        (m.players[&gw].position().x - (me.x + 1.0)).abs() < 0.01
    })
    .await;
    let x = on(&map, move |m| m.position_of(eye).map(|p| p.x)).await;
    assert!(x.is_none_or(|x| x == at.x + 3.0), "the eye let go stays");
}

/// Thistle Tea's "controller movement updates authoritative position and
/// reaches the victim": a priest's client moving the warrior she holds by
/// Mind Control moves him, and is told to his own client and to one who
/// watches, not back to hers (mangos's `SendMessageToAllWhoSeeMeMove`).
#[tokio::test]
async fn a_possessed_players_move_reaches_him_and_his_watchers_not_his_controller() {
    let realm = harness::realm_with(more());
    let (mut a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (mut b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    let (mut c, gc, _) = enter(&realm, "C", "Watcher", Race::Human, Class::Warrior).await;
    learn(&map, ga, &[MIND_CONTROL]);
    bring(&map, gb, ga, 8.0).await;
    bring(&map, gc, ga, 2.0).await;
    a.sees(gb).await;
    c.sees(gb).await;
    flag_pvp(&map, vec![ga, gb]);
    cast(&a, MIND_CONTROL, Some(gb)).await;
    assert!(!control_update(&mut b, gb).await, "the victim lets go");
    assert!(control_update(&mut a, gb).await, "the priest takes hold");
    a.send(CMSG_SET_ACTIVE_MOVER { guid: gb }).await;
    until(&map, "she moves him", move |m| {
        m.players[&ga].owned.mover == Some(gb)
    })
    .await;
    let at = on(&map, move |m| m.players[&gb].position()).await;
    a.clear();
    b.clear();
    c.clear();
    a.heartbeat(at.x + 2.0, at.y, at.z).await;
    until(&map, "he moved", move |m| {
        (m.players[&gb].position().x - (at.x + 2.0)).abs() < 0.01
    })
    .await;
    for (who, client) in [("his own client", &mut b), ("the watcher", &mut c)] {
        let told = heartbeat_of(client, gb, harness::WAIT).await;
        assert_eq!(told.map(|p| p.x), Some(at.x + 2.0), "{who} is told");
    }
    assert!(
        heartbeat_of(&mut a, gb, NONE_WITHIN).await.is_none(),
        "her client isn't told back"
    );
}

/// The user's call, mangos-classic's `HandlePetAction`, where Thistle
/// Tea's "pet attacks and player openers retain their separate initiation
/// flags" reads the spell's attributes: a pet's harmful bar spell at a
/// wolf in range and sight is cast and starts no attack; at one out of its
/// range, the pet runs at it and attacks it, the spell its opener, cast
/// once in range; with no way there (the wolf high above), its master is
/// told it can't attack, and it stays.
#[tokio::test]
async fn a_pets_harmful_bar_spell_is_cast_in_range_and_an_opener_out_of_it() {
    let (_realm, mut h, gh, map, pet, wolf) = tamed_with(world()).await;
    let gore = pets::button(GORE, pets::ACT_ENABLED);
    let health = |m: &Map, g: Guid| m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH);
    let near = map
        .query(move |m| {
            pets::state_mut(m, pet).unwrap().spells.push(GORE);
            // A wolf that outlasts a few Gores.
            let f = &mut m.creature_mut(wolf).unwrap().fields;
            f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
            f.set_u32(UNIT_FIELD_HEALTH, 10_000);
            for g in [pet, wolf] {
                crate::game::map::combat::leave_combat(m, g);
            }
            let here = m.players[&gh].position();
            put(m, pet, here);
            put(m, wolf, east(here, 10.0));
            let before = health(m, wolf);
            order(m, gh, pet, gore, wolf);
            (health(m, wolf) < before, victim(m, pet))
        })
        .await
        .unwrap();
    assert_eq!(near, (true, None), "in range: gored, no attack");
    let far = map
        .query(move |m| {
            for g in [pet, wolf] {
                crate::game::map::combat::leave_combat(m, g);
            }
            m.caster_mut(pet).unwrap().gcd_until = None;
            let here = m.players[&gh].position();
            put(m, pet, here);
            put(m, wolf, east(here, 45.0));
            let before = health(m, wolf);
            order(m, gh, pet, gore, wolf);
            let sent = (
                victim(m, pet),
                pets::state(m, pet).unwrap().opener.map(|o| o.0),
                health(m, wolf) == before,
            );
            // Brought within its range, it casts its opener.
            put(m, pet, east(here, 20.0));
            think(m, pet);
            (
                sent,
                health(m, wolf) < before,
                pets::state(m, pet).unwrap().opener,
                victim(m, pet),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        far,
        ((Some(wolf), Some(GORE), true), true, None, Some(wolf)),
        "out of range: sent at it with its opener, cast once in range"
    );
    h.clear();
    let high = map
        .query(move |m| {
            for g in [pet, wolf] {
                crate::game::map::combat::leave_combat(m, g);
            }
            m.caster_mut(pet).unwrap().gcd_until = None;
            let here = m.players[&gh].position();
            put(m, pet, here);
            put(
                m,
                wolf,
                Vector3d {
                    z: here.z + 40.0,
                    ..east(here, 10.0)
                },
            );
            order(m, gh, pet, gore, wolf);
            (victim(m, pet), pets::state(m, pet).unwrap().opener)
        })
        .await
        .unwrap();
    assert_eq!(high, (None, None), "no way there: it stays");
    let told = h
        .wait_for("the pet's feedback", |m| match m {
            ServerOpcodeMessage::SMSG_PET_ACTION_FEEDBACK(f) => Some(f.feedback),
            _ => None,
        })
        .await;
    assert_eq!(told, PetFeedback::CantAttackTarget);
}
