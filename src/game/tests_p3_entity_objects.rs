//! Phase 3's game object tests, ported from Thistle Tea's
//! `test/game/world/entity/game_object*` (docs/parity/matrix.csv): rituals
//! and their helpers.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::SpellDuration;
use crate::game::data::{GameData, GameObjectTemplate};
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras::AURA_DUMMY;
use crate::game::map::objects::{self, GO_RITUAL};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::{aura_effect, on, spell};

/// An altar of `required` helpers, its completion and animation spells,
/// persistent or not, for a group or anyone.
const ALTAR: u32 = 97_000;
const TEN_ALTAR: u32 = 97_001;
const OPEN_ALTAR: u32 = 97_002;
const PLACED_ALTAR: u32 = 97_003;
/// The completion's mark: a dummy aura on whoever casts it.
const DONE: u32 = 97_010;
/// The helpers' channel: ten seconds.
const CHANNEL: u32 = 97_011;

pub(crate) fn object(
    entry: u32,
    kind: u8,
    name: &str,
    data: &[(usize, u32)],
) -> GameObjectTemplate {
    let mut t = GameObjectTemplate {
        entry,
        kind,
        display_id: 1,
        name: name.into(),
        size: 1.0,
        ..Default::default()
    };
    for (i, v) in data {
        t.data[*i] = *v;
    }
    t
}

fn altar(entry: u32, required: u32, persistent: bool, grouped: bool) -> GameObjectTemplate {
    object(
        entry,
        GO_RITUAL,
        "Altar",
        &[
            (0, required),
            (1, DONE),
            (2, CHANNEL),
            (3, u32::from(persistent)),
            (6, u32::from(grouped)),
        ],
    )
}

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.durations.insert(
        9,
        SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    d.spells
        .spells
        .insert(DONE, spell(DONE, &[aura_effect(AURA_DUMMY, 1, 0, 1)]));
    let mut channel = spell(CHANNEL, &[]);
    channel.duration_index = 9;
    channel.attributes[1] = crate::game::data::spells::ATTR_EX_CHANNELED;
    d.spells.spells.insert(CHANNEL, channel);
    for t in [
        altar(ALTAR, 3, false, true),
        altar(TEN_ALTAR, 10, false, false),
        altar(OPEN_ALTAR, 2, false, false),
        altar(PLACED_ALTAR, 3, true, true),
    ] {
        d.gameobject_templates.insert(t.entry, t);
    }
    d
}

/// Players by `names`, each in the world at the start, the first leading
/// a group of the rest.
async fn grouped(realm: &Arc<Realm>, names: &[&str], group: bool) -> Vec<Client> {
    let mut out = Vec::new();
    for name in names {
        let mut c = Client::connect(realm, name);
        c.enter(name, Race::Human, Class::Warrior).await;
        out.push(c);
    }
    for i in 1..out.len() {
        if !group {
            break;
        }
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

fn here() -> Vector3d {
    Vector3d {
        x: START.0 + 2.0,
        y: START.1,
        z: START.2,
    }
}

/// An altar of `entry` beside the start, `owner`'s or no one's.
fn raise(m: &mut Map, entry: u32, owner: Option<Guid>) -> Guid {
    objects::summon(m, owner, entry, here(), 0.0, 0, 0, 0, None).expect("the altar")
}

fn users(m: &Map, altar: Guid) -> Vec<Guid> {
    m.objects
        .rituals
        .get(&altar)
        .map(|r| r.users.clone())
        .unwrap_or_default()
}

/// How many of the completion's mark `g` holds.
fn done(m: &Map, g: Guid) -> usize {
    m.players.get(&g).map_or(0, |p| {
        p.auras.holders.iter().filter(|h| h.spell == DONE).count()
    })
}

/// Thistle Tea's "ten-player altars wait for ten distinct participants":
/// each helper counts once, however often they use it, and the tenth
/// completes it.
#[tokio::test]
async fn a_ten_player_altar_waits_for_ten_distinct_helpers() {
    let realm = harness::realm_with(world());
    let map: MapHandle = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let altar = raise(m, TEN_ALTAR, None);
        let helpers: Vec<Guid> = (1..=10).map(|n| guid::player(900_000 + n)).collect();
        let mut seen = Vec::new();
        for h in &helpers[..9] {
            objects::use_object(m, *h, altar);
            objects::use_object(m, *h, altar);
            seen.push((users(m, altar).len(), m.objects.rituals[&altar].completed));
        }
        objects::use_object(m, helpers[9], altar);
        let gone = m.gameobject(altar).is_none();
        (seen, gone)
    })
    .await;
    for (i, (n, completed)) in out.0.iter().enumerate() {
        assert_eq!(*n, i + 1, "a repeat counts for nothing");
        assert!(!completed, "not done with {n}");
    }
    assert!(out.1, "the tenth completes it, and it goes");
}

/// Thistle Tea's "completes after the required unique grouped
/// participants", "ignores the owner and ungrouped helpers" and "allows
/// ungrouped helpers when the template does not require grouping": an
/// owner's ritual of three takes its owner's group alone, its owner and a
/// repeat counting for nothing, and its owner casts its spell; one for
/// anyone takes a stranger.
#[tokio::test]
async fn an_owned_ritual_takes_its_owners_group_once_each() {
    let realm = harness::realm_with(world());
    let mut cs = grouped(&realm, &["Alice", "Bob", "Cid"], true).await;
    let mut eve = Client::connect(&realm, "E");
    let ge = eve.enter("Eve", Race::Human, Class::Warrior).await;
    let (ga, gb, gc) = (cs[0].guid, cs[1].guid, cs[2].guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let altar = raise(m, ALTAR, Some(ga));
        let mut seen = Vec::new();
        objects::use_object(m, ga, altar);
        seen.push(users(m, altar));
        objects::use_object(m, ge, altar);
        seen.push(users(m, altar));
        objects::use_object(m, gb, altar);
        objects::use_object(m, gb, altar);
        seen.push(users(m, altar));
        objects::use_object(m, gc, altar);
        let completed = m.gameobject(altar).is_none();
        // One for anyone: Eve, a stranger, makes two.
        let open = raise(m, OPEN_ALTAR, Some(ga));
        objects::use_object(m, ge, open);
        (
            seen,
            completed,
            done(m, ga),
            m.gameobject(open).is_none(),
            done(m, ga),
        )
    })
    .await;
    assert_eq!(out.0[0], vec![ga], "the owner's own use counts for nothing");
    assert_eq!(out.0[1], vec![ga], "Eve isn't of the group");
    assert_eq!(out.0[2], vec![ga, gb], "Bob once");
    assert!(out.1, "Cid makes three: done");
    assert_eq!(out.2, 1, "the owner cast its spell");
    assert!(out.3, "a stranger completes one for anyone");
    assert_eq!(out.4, 1, "its spell on the owner too, refreshed");
    cs.clear();
}

/// Thistle Tea's "a placed ritual groups its helpers with the first
/// participant", "the first participant casts the completion spell once
/// enough have gathered", "clears a completed placed ritual for its next
/// use", "the next participant leads a placed ritual its participants all
/// left" and "an altar without a despawn timer uses its spell's channel
/// duration": no one's altar is led by its first helper, whose group alone
/// may join; done, the first casts its spell and it is ready again, empty;
/// left by everyone, the next to come leads it; and each helper channels
/// the animation's ten seconds.
#[tokio::test]
async fn a_placed_altar_is_led_by_its_first_helper_and_ready_again_once_done() {
    let realm = harness::realm_with(world());
    let mut cs = grouped(&realm, &["Alice", "Bob", "Cid"], true).await;
    let mut eve = Client::connect(&realm, "E");
    let ge = eve.enter("Eve", Race::Human, Class::Warrior).await;
    let (ga, gb, gc) = (cs[0].guid, cs[1].guid, cs[2].guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let altar = on(&map, move |m| raise(m, PLACED_ALTAR, None)).await;
    on(&map, move |m| objects::use_object(m, ga, altar)).await;
    let channel = cs[0]
        .wait_for("Alice's channel", |m| match m {
            ServerOpcodeMessage::MSG_CHANNEL_START(c) if c.spell == CHANNEL => Some(c.duration),
            _ => None,
        })
        .await;
    assert_eq!(channel, 10_000, "the animation's own duration");
    let out = on(&map, move |m| {
        let first = m.objects.rituals[&altar].first;
        objects::use_object(m, ge, altar);
        let after_eve = users(m, altar);
        objects::use_object(m, gb, altar);
        objects::use_object(m, gc, altar);
        let r = m.objects.rituals[&altar].clone();
        (
            first,
            after_eve,
            done(m, ga),
            done(m, gb) + done(m, gc),
            r.users.is_empty() && r.first.is_none() && !r.completed,
            m.gameobject(altar).is_some(),
        )
    })
    .await;
    assert_eq!(out.0, Some(ga), "Alice leads it");
    assert_eq!(out.1, vec![ga], "Eve isn't of her group");
    assert_eq!(out.2, 1, "Alice cast its spell, once");
    assert_eq!(out.3, 0, "the others didn't");
    assert!(out.4, "done, it is clear for the next use");
    assert!(out.5, "and stays");
    // Alice alone at it, then gone: Eve, of no group of hers, leads it.
    let out = on(&map, move |m| {
        objects::use_object(m, ga, altar);
        crate::game::map::spells::interrupt(m, ga);
        let left = m.objects.rituals[&altar].first;
        objects::use_object(m, ge, altar);
        (left, m.objects.rituals[&altar].first, users(m, altar))
    })
    .await;
    assert_eq!(out.0, None, "no one leads an altar all have left");
    assert_eq!(out.1, Some(ge), "Eve does now");
    assert_eq!(out.2, vec![ge]);
    cs.clear();
}

/// Thistle Tea's "a persistent ritual can complete again once too few
/// participants remain": an owner's ritual that stays, done, is open again
/// when a helper leaves, and done again by another.
#[tokio::test]
async fn a_lasting_ritual_left_short_is_done_again() {
    let mut d = world();
    d.gameobject_templates
        .insert(ALTAR, altar(ALTAR, 2, true, false));
    let realm = harness::realm_with(d);
    let mut cs = grouped(&realm, &["Alice", "Bob", "Cid"], false).await;
    let (ga, gb, gc) = (cs[0].guid, cs[1].guid, cs[2].guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let altar = raise(m, ALTAR, Some(ga));
        objects::use_object(m, gb, altar);
        let first = m.objects.rituals[&altar].completed;
        // Bob still channels at it; he leaves.
        let now = m.now;
        if let Some(c) = m.caster_mut(gb) {
            c.casting = Some(crate::game::map::spells::Casting {
                spell: CHANNEL,
                targets: crate::game::map::spells::Targets {
                    object: Some(altar),
                    ..Default::default()
                },
                ends: now + std::time::Duration::from_secs(10),
                item: None,
                pushbacks: 0,
                channel_until: Some(now + std::time::Duration::from_secs(10)),
                duration: 10_000,
            });
        }
        crate::game::map::spells::interrupt(m, gb);
        let reopened = !m.objects.rituals[&altar].completed;
        objects::use_object(m, gc, altar);
        (
            first,
            reopened,
            m.objects.rituals[&altar].completed,
            done(m, ga),
        )
    })
    .await;
    assert!(out.0, "Bob makes two: done");
    assert!(out.1, "short of a helper, it is open again");
    assert!(out.2, "and Cid does it again");
    assert_eq!(out.3, 1, "its spell on the owner each time");
    cs.clear();
}

/// An object to summon, a chest with a linked trap, a trap of no limit
/// and one of two springs.
const POST: u32 = 97_020;
const CHEST: u32 = 97_021;
const CHEST_TRAP: u32 = 97_022;
const TRAP: u32 = 97_023;
const TWO_TRAP: u32 = 97_024;
const BUTTON: u32 = 97_025;
const BUTTON_TRAP: u32 = 97_026;

fn objects_world() -> GameData {
    let mut d = world();
    d.faction_templates.insert(
        NEUTRAL,
        crate::game::tests_p3_combat::faction(NEUTRAL, 0x8, 0),
    );
    d.spells
        .spells
        .insert(MARK, spell(MARK, &[aura_effect(AURA_DUMMY, 1, 0, 6)]));
    let mut sheep = crate::game::tests_p3_combat::template(SHEEP, 1);
    sheep.faction = NEUTRAL;
    d.creature_templates.insert(SHEEP, sheep);
    for t in [
        object(POST, 5, "Post", &[]),
        object(CHEST, 3, "Chest", &[(7, CHEST_TRAP)]),
        object(CHEST_TRAP, 6, "Chest Trap", &[]),
        // A trap of five yards with no limit, two seconds between springs.
        object(TRAP, 6, "Trap", &[(2, 5), (3, MARK), (4, 0), (5, 2)]),
        object(
            TWO_TRAP,
            6,
            "Two Trap",
            &[(2, 5), (3, MARK), (4, 2), (5, 1)],
        ),
        object(BUTTON, 1, "Button", &[(3, BUTTON_TRAP)]),
        object(BUTTON_TRAP, 6, "Button Trap", &[(2, 0), (3, DONE)]),
    ] {
        d.gameobject_templates.insert(t.entry, t);
    }
    d
}

const NEUTRAL: u32 = 41;
/// A trap's mark: a dummy aura on whom it springs on.
const MARK: u32 = 97_012;
const SHEEP: u32 = 3720;

fn summoned_alive(m: &Map, g: Guid) -> bool {
    m.gameobject(g).is_some()
}

/// Thistle Tea's `GameObjectSummons`: a summon in a slot replaces only
/// its owner's own in that slot, and its linked object with it; an
/// owner's objects stay while it lies dead and go with its corpse; a
/// player's go as they log out or leave the map, while one owned by no
/// one stays and goes at its own time; and each is marked its creator's.
#[tokio::test]
async fn summoned_objects_go_with_their_slot_their_owner_or_their_time() {
    let realm = harness::realm_with(objects_world());
    let mut cs = grouped(&realm, &["Alice", "Bob"], false).await;
    let (ga, gb) = (cs[0].guid, cs[1].guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let at = here();
        let s = |m: &mut Map, owner, entry, slot| {
            objects::summon(m, owner, entry, at, 0.0, 0, 0, slot, None).unwrap()
        };
        let first = s(m, Some(ga), CHEST, 1);
        let linked = m
            .objects
            .summoned
            .iter()
            .find(|x| x.guid == first)
            .map(|x| x.linked.clone())
            .unwrap();
        let two = s(m, Some(ga), POST, 2);
        let none = s(m, Some(ga), POST, 0);
        let bobs = s(m, Some(gb), POST, 1);
        let again = s(m, Some(ga), POST, 1);
        let creators = [two, bobs].map(|g| {
            m.gameobject(g)
                .unwrap()
                .fields
                .get_guid(crate::game::fields::OBJECT_FIELD_CREATED_BY)
        });
        (
            first,
            linked,
            [first, two, none, bobs, again].map(|g| summoned_alive(m, g)),
            creators,
        )
    })
    .await;
    let (first, linked, alive, creators) = out;
    assert_eq!(linked.len(), 1, "the chest came with its trap");
    assert_eq!(
        alive,
        [false, true, true, true, true],
        "only Alice's slot 1"
    );
    let linked = linked[0];
    let gone = on(&map, move |m| m.gameobject(linked).is_none()).await;
    assert!(gone, "its linked trap went with it");
    assert_eq!(creators, [ga, gb], "each its creator's");
    let _ = first;
    // A creature's object: kept while it lies dead, gone with its corpse.
    let out = on(&map, move |m| {
        let owner = crate::game::tests_p3_combat::spawn(m, ga, SHEEP, 20, 6.0);
        let post = objects::summon(m, Some(owner), POST, here(), 0.0, 0, 0, 0, None).unwrap();
        let creator = m
            .gameobject(post)
            .unwrap()
            .fields
            .get_guid(crate::game::fields::OBJECT_FIELD_CREATED_BY);
        crate::game::map::combat::deal_damage(m, ga, owner, 1_000_000);
        objects::update(m);
        let kept = summoned_alive(m, post);
        m.remove_creature(owner);
        objects::update(m);
        (creator == owner, kept, summoned_alive(m, post))
    })
    .await;
    assert_eq!(
        out,
        (true, true, false),
        "the creature's, kept by its corpse, gone with it"
    );
    // Alice's owned post and an ownerless one with a time: she logs out.
    let (owned, wild) = on(&map, move |m| {
        let owned = objects::summon(m, Some(ga), POST, here(), 0.0, 0, 0, 3, None).unwrap();
        let wild = objects::summon(m, None, POST, here(), 0.0, 60_000, 0, 0, None).unwrap();
        (owned, wild)
    })
    .await;
    let alice = cs.remove(0);
    alice.send(CMSG_LOGOUT_REQUEST {}).await;
    crate::game::tests_p3_entity_objects::until(&map, "Alice's post gone", move |m| {
        m.gameobject(owned).is_none()
    })
    .await;
    let out = on(&map, move |m| {
        let stays = summoned_alive(m, wild);
        let past = m.now - std::time::Duration::from_millis(1);
        if let Some(x) = m.objects.summoned.iter_mut().find(|x| x.guid == wild) {
            x.expires = Some(past);
        }
        objects::update(m);
        (stays, summoned_alive(m, wild))
    })
    .await;
    assert_eq!(out, (true, false), "no one's stays, and goes at its time");
    // Bob's post goes as he leaves the map.
    let bobs = on(&map, move |m| {
        objects::summon(m, Some(gb), POST, here(), 0.0, 0, 0, 0, None).unwrap()
    })
    .await;
    cs[0].say(".go xyz 1629 -4373 31 1").await;
    until(&map, "Bob's post gone", move |m| {
        m.gameobject(bobs).is_none()
    })
    .await;
}

/// Wait until `f` holds on the map.
pub(crate) async fn until(
    map: &MapHandle,
    what: &str,
    f: impl Fn(&Map) -> bool + Send + Sync + Clone + 'static,
) {
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    loop {
        let f = f.clone();
        if on(map, move |m| f(m)).await {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Look about the traps now.
fn look(m: &mut Map) {
    m.objects.next_trap_look = None;
    objects::update(m);
}

/// Thistle Tea's trap `consume/1`, `ready?/2` and `target/1`: an owner's
/// trap passes over a neutral creature at rest but springs on it fighting;
/// with no limit it springs again and again, but not before its cooldown;
/// one of two springs is spent and goes; and a button springs its nearest
/// linked trap, not the other.
#[tokio::test]
async fn a_trap_springs_on_a_foe_as_its_charges_and_cooldown_allow() {
    let mut d = objects_world();
    d.gameobject_spawns.entry(0).or_default().extend([
        crate::game::data::GameObjectSpawn {
            guid: 71,
            entry: BUTTON,
            map: 0,
            x: START.0 - 20.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        },
        crate::game::data::GameObjectSpawn {
            guid: 72,
            entry: BUTTON_TRAP,
            map: 0,
            x: START.0 - 19.5,
            y: START.1,
            z: START.2,
            ..Default::default()
        },
        crate::game::data::GameObjectSpawn {
            guid: 73,
            entry: BUTTON_TRAP,
            map: 0,
            x: START.0 - 18.0,
            y: START.1,
            z: START.2,
            ..Default::default()
        },
    ]);
    let realm = harness::realm_with(d);
    let cs = grouped(&realm, &["Alice"], false).await;
    let ga = cs[0].guid;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let sheep = crate::game::tests_p3_combat::spawn(m, ga, SHEEP, 20, 2.0);
        let at = m.creature(sheep).unwrap().position;
        let trap = objects::summon(m, Some(ga), TRAP, at, 0.0, 0, 0, 0, None).unwrap();
        let marks = |m: &Map| {
            crate::game::map::auras::auras(m, sheep)
                .map_or(0, |a| a.holders.iter().filter(|h| h.spell == MARK).count())
        };
        look(m);
        let idle = marks(m);
        // In a fight, with Alice.
        crate::game::map::combat::engage(m, ga, sheep);
        look(m);
        let fighting = marks(m);
        crate::game::map::auras::remove_spell(m, sheep, MARK, None);
        look(m);
        let cooling = marks(m);
        m.objects.traps.get_mut(&trap).unwrap().ready_at = m.now;
        look(m);
        let again = (
            marks(m),
            m.objects.traps[&trap].ready_at - m.now == std::time::Duration::from_secs(2),
            m.gameobject(trap).is_some(),
        );
        objects::despawn(m, trap);
        // Two springs, and it's gone.
        crate::game::map::auras::remove_spell(m, sheep, MARK, None);
        let two = objects::summon(m, Some(ga), TWO_TRAP, at, 0.0, 0, 0, 0, None).unwrap();
        let mut springs = Vec::new();
        for _ in 0..2 {
            look(m);
            springs.push((marks(m), m.gameobject(two).is_some()));
            crate::game::map::auras::remove_spell(m, sheep, MARK, None);
            if let Some(t) = m.objects.traps.get_mut(&two) {
                t.ready_at = m.now;
            }
        }
        // Gone before it swings: its hundred a blow would kill Alice, at
        // level 1, if the map's update came between this and her button,
        // and a dead Alice takes no mark.
        m.remove_creature(sheep);
        (idle, fighting, cooling, again, springs)
    })
    .await;
    assert_eq!(out.0, 0, "a neutral sheep at rest is passed over");
    assert_eq!(out.1, 1, "fighting, it springs the trap");
    assert_eq!(out.2, 0, "not again before its cooldown");
    assert_eq!(
        out.3,
        (1, true, true),
        "after it, again, its two seconds again, with no limit"
    );
    assert_eq!(out.4, vec![(1, true), (1, false)], "two springs, then gone");
    // The button: Alice by it, the nearer of its traps sprung on her.
    let (near, far) = (
        crate::game::guid::game_object(BUTTON_TRAP, 72),
        crate::game::guid::game_object(BUTTON_TRAP, 73),
    );
    let button = crate::game::guid::game_object(BUTTON, 71);
    let out = on(&map, move |m| {
        let before = [near, far].map(|g| m.objects.traps.get(&g).map(|t| t.ready_at));
        objects::use_object(m, ga, button);
        let after = [near, far].map(|g| m.objects.traps.get(&g).map(|t| t.ready_at));
        (before, after, done(m, ga))
    })
    .await;
    assert_ne!(out.0[0], out.1[0], "the nearer trap sprang");
    assert_eq!(out.0[1], out.1[1], "the farther didn't");
    assert_eq!(out.2, 1, "on Alice");
}

/// Spells that raise an object (effect 50) and whose cooldown waits for
/// it: a post, and an altar of two.
const RAISE_POST: u32 = 97_030;
const RAISE_ALTAR: u32 = 97_031;
const DOOM_ALTAR: u32 = 97_032;
const SMSG_COOLDOWN_EVENT: u16 = 0x135;
const SMSG_CLEAR_COOLDOWN: u16 = 0x1DE;

fn deferred_world() -> GameData {
    let mut d = objects_world();
    d.gameobject_templates
        .insert(DOOM_ALTAR, altar(DOOM_ALTAR, 2, false, false));
    for (id, entry) in [(RAISE_POST, POST), (RAISE_ALTAR, DOOM_ALTAR)] {
        let mut s = spell(
            id,
            &[crate::game::data::spells::SpellEffect {
                effect: objects::EFFECT_TRANS_DOOR,
                misc_value: entry as i32,
                target_a: 1,
                ..Default::default()
            }],
        );
        s.recovery_time = 60_000;
        s.attributes[0] = 0x0200_0000;
        d.spells.spells.insert(id, s);
    }
    d
}

/// Thistle Tea's deferred cooldowns: an object's spell whose cooldown
/// waits for the object starts it as the object goes, told once; a ritual
/// done starts it; and one taken away undone lets it go, the button
/// cleared, with no cooldown at all.
#[tokio::test]
async fn a_cooldown_that_waits_for_its_object_starts_as_it_goes() {
    let realm = harness::realm_with(deferred_world());
    let mut cs = grouped(&realm, &["Alice", "Bob"], false).await;
    let (ga, gb) = (cs[0].guid, cs[1].guid);
    let map = realm.maps.get(&realm, MapKey::world(0));
    let raised = move |m: &Map, spell: u32| {
        m.objects
            .summoned
            .iter()
            .find(|s| s.owner == Some(ga) && s.spell == spell)
            .map(|s| s.guid)
    };
    let state = move |m: &Map, spell: u32| {
        let c = m.caster(ga).unwrap();
        (
            c.pending_cooldowns.contains(&spell),
            c.cooldowns.get(&spell).is_some_and(|t| *t > m.now),
        )
    };
    // The post: waiting while it stands; started, and told once, as it goes.
    let out = on(&map, move |m| {
        crate::game::map::spells::cast(m, ga, RAISE_POST, Default::default(), None);
        let post = raised(m, RAISE_POST).unwrap();
        let waiting = state(m, RAISE_POST);
        objects::despawn(m, post);
        let gone = state(m, RAISE_POST);
        crate::game::map::spells::cooldowns::release(m, ga, RAISE_POST, true);
        (waiting, gone)
    })
    .await;
    assert_eq!(out, ((true, false), (false, true)), "waiting, then started");
    let events = {
        let a = &mut cs[0];
        a.wait_frame("SMSG_COOLDOWN_EVENT", SMSG_COOLDOWN_EVENT, |f| {
            f[..4] == RAISE_POST.to_le_bytes()
        })
        .await;
        crate::game::tests_p3_entity::gets_frame(a, SMSG_COOLDOWN_EVENT, |_| true).await
    };
    assert!(!events, "told once");
    // The altar taken away undone: no cooldown, the button let go.
    cs[0].clear();
    let undone = on(&map, move |m| {
        crate::game::map::spells::cast(m, ga, RAISE_ALTAR, Default::default(), None);
        let altar = raised(m, RAISE_ALTAR).unwrap();
        objects::despawn(m, altar);
        state(m, RAISE_ALTAR)
    })
    .await;
    assert_eq!(undone, (false, false), "no cooldown for a ritual undone");
    let cleared = cs[0]
        .wait_frame("SMSG_CLEAR_COOLDOWN", SMSG_CLEAR_COOLDOWN, |f| {
            f[..4] == RAISE_ALTAR.to_le_bytes()
        })
        .await;
    assert_eq!(cleared[4..12], ga.guid().to_le_bytes());
    // Raised again and done with Bob's help: started.
    let done = on(&map, move |m| {
        crate::game::map::spells::cast(m, ga, RAISE_ALTAR, Default::default(), None);
        let altar = raised(m, RAISE_ALTAR).unwrap();
        objects::use_object(m, gb, altar);
        (m.gameobject(altar).is_none(), state(m, RAISE_ALTAR))
    })
    .await;
    assert_eq!(done, (true, (false, true)), "done, gone and started");
}

const LOOT_CHEST: u32 = 97_040;
const TREASURE: u32 = 97_041;

fn chest_world() -> GameData {
    let mut d = objects_world();
    d.gameobject_templates
        .insert(LOOT_CHEST, object(LOOT_CHEST, 3, "Chest", &[(1, 900)]));
    d.loot.gameobject.insert(
        900,
        vec![crate::game::data::LootEntry {
            item: TREASURE,
            chance: 100.0,
            group: 0,
            min: 1,
            max: 1,
            condition: 0,
        }],
    );
    d.items.insert(
        TREASURE,
        crate::game::data::items::ItemTemplate {
            entry: TREASURE,
            name: "Treasure".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    d
}

/// Thistle Tea's chest `reserve_item`, `view` and `Chest.release`: an item
/// taken from a chest is had once, a second take of its slot giving
/// nothing; emptied and let go, a summoned chest goes, with no respawn to
/// come, and gone, it gives no loot.
#[tokio::test]
async fn a_chests_item_is_had_once_and_an_emptied_summoned_chest_goes_for_good() {
    let realm = harness::realm_with(chest_world());
    let mut cs = grouped(&realm, &["Alice"], false).await;
    let ga = cs[0].guid;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let chest = on(&map, move |m| {
        objects::summon(m, None, LOOT_CHEST, here(), 0.0, 0, 0, 0, None).unwrap()
    })
    .await;
    let a = &mut cs[0];
    a.sees(chest).await;
    a.send(CMSG_GAMEOBJ_USE { guid: chest }).await;
    a.wait_frame("SMSG_LOOT_RESPONSE", 0x160, |_| true).await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.send(CMSG_LOOT_RELEASE { guid: chest }).await;
    crate::game::tests_p3_entity_objects::until(&map, "the chest gone", move |m| {
        m.gameobject(chest).is_none()
    })
    .await;
    let (count, respawns) = on(&map, move |m| {
        let count: u32 = m.players[&ga]
            .items
            .iter()
            .filter(|i| i.entry == TREASURE)
            .map(|i| i.count())
            .sum();
        (count, m.objects.respawns.len())
    })
    .await;
    assert_eq!(count, 1, "one treasure, not two");
    assert_eq!(respawns, 0, "a summoned chest comes back no more");
    a.clear();
    a.send(CMSG_GAMEOBJ_USE { guid: chest }).await;
    assert!(
        !crate::game::tests_p3_entity::gets_frame(a, 0x160, |_| true).await,
        "gone, no loot"
    );
}
