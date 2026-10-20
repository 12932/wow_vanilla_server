//! Round 9's world tests of Thistle Tea's loot, escort and pet loaders,
//! ported from `test/game/world/loot/` and `test/game/world/loader/`: a
//! drop behind a server variable gone from a loot rolled once the
//! variable changed, a drop behind a spawn's guid only from that spawn,
//! an escort walking its own points from point 0, a pet's first spells
//! with no teaching recipe, and an ability of two families' lines taught
//! once, its next rank for the difference.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::pets::{Family, PetAbility};
use crate::game::data::scripts::Condition;
use crate::game::data::spells::SpellEffect;
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::conditions::{Truth, check};
use crate::game::map::creature::Motion;
use crate::game::map::scripts::{creature_of_spawn, set_server_variable};
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapKey, combat, escorts, loot, pets};
use crate::game::tests_p3_combat::{START, spawn};
use crate::game::tests_pets::{self, effect};

// Loot behind conditions.

/// A creature that drops both gated items; its two spawns.
const LOOTER: u32 = 98_130;
const SPAWNED: u32 = 98_131;
const OTHER_SPAWN: u32 = 98_132;
/// An item behind the server variable, one behind the spawn's guid.
const BY_VARIABLE: u32 = 98_133;
const BY_SPAWN: u32 = 98_134;
/// The conditions: server variable `VARIABLE` at 0 (11); the source of
/// spawn `SPAWNED` (52).
const VARIABLE_AT_ZERO: u32 = 98_135;
const OF_THE_SPAWN: u32 = 98_136;
/// A server variable no other test sets.
const VARIABLE: u32 = 98_137;

fn loot_world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for item in [BY_VARIABLE, BY_SPAWN] {
        d.items.insert(
            item,
            ItemTemplate {
                entry: item,
                name: format!("Item {item}"),
                display_id: 1000,
                stackable: 1,
                allowable_class: -1,
                allowable_race: -1,
                buy_count: 1,
                ..Default::default()
            },
        );
    }
    let mut t = crate::game::tests_p3_combat::template(LOOTER, 1);
    t.loot_id = LOOTER;
    d.creature_templates.insert(LOOTER, t);
    d.loot.creature.insert(
        LOOTER,
        [(BY_VARIABLE, VARIABLE_AT_ZERO), (BY_SPAWN, OF_THE_SPAWN)]
            .map(|(item, condition)| LootEntry {
                item,
                chance: 100.0,
                min: 1,
                max: 1,
                condition,
                ..Default::default()
            })
            .to_vec(),
    );
    for (entry, kind, values) in [
        (VARIABLE_AT_ZERO, 11, [VARIABLE as i32, 0, 0, 0]),
        (OF_THE_SPAWN, 52, [SPAWNED as i32, 0, 0, 0]),
    ] {
        d.scripts.conditions.insert(
            entry,
            Condition {
                entry,
                kind,
                values,
                flags: 0,
            },
        );
    }
    d.creature_spawns.insert(
        0,
        [(SPAWNED, 2.0), (OTHER_SPAWN, -2.0)]
            .map(|(guid, dy)| CreatureSpawn {
                guid,
                entries: vec![LOOTER],
                map: 0,
                x: START.0 + 2.0,
                y: START.1 + dy,
                z: START.2,
                ..Default::default()
            })
            .to_vec(),
    );
    d
}

/// The items of the loot window `c` is shown on `corpse`, opening it.
async fn shown(c: &mut Client, corpse: Guid) -> Vec<u32> {
    c.send(CMSG_LOOT { guid: corpse }).await;
    let b = c
        .wait_frame("SMSG_LOOT_RESPONSE", loot::SMSG_LOOT_RESPONSE, |b| {
            b[0..8] == corpse.guid().to_le_bytes()
        })
        .await;
    let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    (0..usize::from(b[13]))
        .map(|n| u32_at(14 + n * 22 + 1))
        .collect()
}

/// Thistle Tea's `ActorFactory` "refresh global conditions for both local
/// and remote loot actors" and "loot sources use the owner's
/// authoritative spawn DB guid": a drop behind server variable 98137 at 0
/// (vmangos's condition 11) shows while the variable is 0, and not from a
/// creature killed once it is 1; a drop behind spawn 98131's guid
/// (condition 52) shows from that spawn's creature, not from another
/// spawn's, nor from a summoned one of the same entry, which has no spawn,
/// so the condition can't be judged. wvs judges a slot's condition as the
/// window opens; Thistle Tea's loot context built before the change still
/// saying met is its own snapshot.
#[tokio::test]
async fn a_drop_follows_its_server_variable_and_its_sources_spawn() {
    let realm = harness::realm_with(loot_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (spawned, other, summoned, unknown) = map
        .query(move |m| {
            let spawned = creature_of_spawn(m, SPAWNED).expect("spawn 98131");
            let other = creature_of_spawn(m, OTHER_SPAWN).expect("spawn 98132");
            let summoned = spawn(m, ga, LOOTER, 20, 2.0);
            let unknown = check(m, OF_THE_SPAWN, Some(summoned), Some(ga));
            for c in [spawned, summoned] {
                combat::kill(m, ga, c);
            }
            (spawned, other, summoned, unknown)
        })
        .await
        .unwrap();
    assert_eq!(unknown, Truth::Unknown, "a summoned creature has no spawn");
    assert_eq!(shown(&mut a, spawned).await, [BY_VARIABLE, BY_SPAWN]);
    assert_eq!(shown(&mut a, summoned).await, [BY_VARIABLE]);
    map.query(move |m| {
        set_server_variable(VARIABLE, 1);
        combat::kill(m, ga, other);
    })
    .await;
    assert_eq!(
        shown(&mut a, other).await,
        Vec::<u32>::new(),
        "the variable at 1; another spawn"
    );
}

// An escort's own path.

/// Waking Legends (8447): Keeper Remulos walks points of his own, no
/// `script_waypoint` rows, and is credited at point 9.
const WAKING_LEGENDS: u32 = 8447;
const REMULOS: u32 = 11_832;
const REMULOS_SPAWN: u32 = 98_140;

fn moonglade() -> GameData {
    let mut d = GameData::empty();
    d.maps.insert(
        1,
        MapInfo {
            id: 1,
            internal_name: "Kalimdor".into(),
            name: "Kalimdor".into(),
            instance_type: 0,
            player_limit: 0,
        },
    );
    let (x, y, z, _) = escorts::escort(WAKING_LEGENDS).unwrap().path.unwrap()[0];
    let mut t = crate::game::tests_p4_items_quests::template(REMULOS, "Keeper Remulos");
    t.npc_flags = 0x2;
    d.creature_templates.insert(REMULOS, t);
    d.creature_spawns.insert(
        1,
        vec![CreatureSpawn {
            guid: REMULOS_SPAWN,
            entries: vec![REMULOS],
            map: 1,
            x: x - 4.0,
            y,
            z,
            respawn_min: 60,
            respawn_max: 60,
            ..Default::default()
        }],
    );
    d.npc.quests.insert(
        WAKING_LEGENDS,
        crate::game::data::npc::Quest {
            id: WAKING_LEGENDS,
            title: "Waking Legends".into(),
            min_level: 1,
            level: 1,
            special_flags: 2,
            ..Default::default()
        },
    );
    d.npc.creature_starts.insert(REMULOS, vec![WAKING_LEGENDS]);
    d
}

/// Where `c` is walking to, if it walks.
fn bound_for(m: &Map, c: Guid) -> Option<Vector3d> {
    match &m.creature(c)?.motion {
        Motion::Moving(w) => w.points.last().copied(),
        Motion::Idle { .. } => None,
    }
}

/// Thistle Tea's `QuestEscort` loader "walks the escort's own path in
/// place of its script_waypoint rows": Keeper Remulos, with no
/// `script_waypoint` rows, sets out on Waking Legends along his own
/// points, numbered from 0: bound for point 0 first, then each in turn,
/// waiting at each as long as it says (still there a millisecond before);
/// the quest explored at point 9, the tenth, and not before; done past
/// the last.
#[tokio::test]
async fn an_escort_walks_its_own_points_from_point_zero() {
    let realm = harness::realm_with(moonglade());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let path = escorts::escort(WAKING_LEGENDS).unwrap().path.unwrap();
    let (x, y, z, _) = path[0];
    let from = realm.registry.online(g).unwrap().map();
    from.call(move |m| m.teleport(g, MapKey::world(1), Vector3d { x: x - 3.0, y, z }, 0.0));
    a.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    a.sees(g).await;
    let map = realm.registry.online(g).unwrap().map();
    let walked = map
        .query(move |m| {
            let remulos = creature_of_spawn(m, REMULOS_SPAWN).expect("Remulos");
            crate::game::map::quests::accept(m, g, remulos, WAKING_LEGENDS);
            let escort = |m: &mut Map| {
                m.escorts
                    .iter_mut()
                    .find(|e| e.quest == WAKING_LEGENDS)
                    .map(|e| e.point())
            };
            m.escorts.iter_mut().for_each(|e| e.skip_start());
            escorts::update(m);
            let explored = |m: &Map| m.players[&g].journal.log.iter().any(|l| l.explored);
            let mut walked = Vec::new();
            for &(x, y, z, wait) in path {
                let at = Vector3d { x, y, z };
                let (bound, to) = (escort(m), bound_for(m, remulos));
                let c = m.creature_mut(remulos).unwrap();
                c.position = at;
                c.motion = Motion::Idle { until: None };
                m.players.get_mut(&g).unwrap().movement.position = at;
                escorts::update(m);
                let credited = explored(m);
                let mut waited = true;
                if wait > 0 {
                    m.now += Duration::from_millis(u64::from(wait - 1));
                    escorts::update(m);
                    waited = escort(m) == bound && bound_for(m, remulos).is_none();
                    m.now += Duration::from_millis(1);
                }
                escorts::update(m);
                let near = to.is_some_and(|t| (t.x - x).abs() < 0.5 && (t.y - y).abs() < 0.5);
                walked.push((bound, near, waited, credited));
            }
            escorts::update(m);
            (walked, escort(m).is_none(), explored(m))
        })
        .await
        .unwrap();
    let (walked, over, explored) = walked;
    let expected: Vec<_> = (0..path.len() as u32)
        .map(|k| (Some(Some(k)), true, true, k == 9))
        .collect();
    assert_eq!(walked, expected, "(bound for, toward it, waited, explored)");
    assert!(over && explored, "done past the last point, credited");
}

// A pet's spells.

const SNAP: u32 = 98_150;
const SNAP_2: u32 = 98_151;
/// A pet skill line's row of a spell the data hasn't, free, learned on
/// taming.
const PHANTOM: u32 = 98_152;
const SNAPPER: u32 = 98_153;
const NOTHING_AT_ALL: u32 = 98_154;
const GROWL: u32 = 2649;
/// The wolves' and the cats' families.
const WOLVES: u32 = 1;
const CATS: u32 = 2;
const BOARS: u32 = 5;

fn spell(id: u32, name: &str, level: u32) -> crate::game::data::spells::SpellTemplate {
    let mut s = tests_pets::spell(
        id,
        name,
        [
            effect(2, 3, 6),
            SpellEffect::default(),
            SpellEffect::default(),
        ],
    );
    s.base_level = level;
    s
}

/// Thistle Tea's `PetSpells.build/4` "keeps abilities without a recipe and
/// gives empty beasts no spells": a creature whose first spell, Snap, is
/// in no pet skill line and has no teaching knows it as a pet; one with no
/// spells knows none; a hunter's tame of it, of a family with no passives,
/// knows only Growl, as wvs gives every hunter's pet Growl's first rank.
#[test]
fn a_pets_first_spell_stays_without_a_teaching_and_an_empty_beast_has_none() {
    let mut d = GameData::empty();
    d.spells.spells.insert(SNAP, spell(SNAP, "Snap", 1));
    d.spells.spells.insert(GROWL, spell(GROWL, "Growl", 1));
    d.pets.spell_lists.insert(1, vec![SNAP]);
    d.pets.creature_spell_list.insert(SNAPPER, 1);
    d.pets.families.insert(
        WOLVES,
        Family {
            id: WOLVES,
            skills: [208, 270],
            ..Default::default()
        },
    );
    assert_eq!(pets::spellbook(&d, SNAPPER, 10, false, 0), [SNAP]);
    assert!(pets::spellbook(&d, NOTHING_AT_ALL, 10, false, 0).is_empty());
    assert_eq!(
        pets::spellbook(&d, NOTHING_AT_ALL, 10, true, WOLVES),
        [GROWL]
    );
}

/// The teaching of `spell`.
const fn teaching(spell: u32) -> u32 {
    spell + 100_000
}

/// Snap in the wolves' line (208) and the cats' (209), five points, its
/// second rank in the wolves', ten; and a free row of a spell there isn't
/// in every pet's line (270), each with a teaching.
fn training_world() -> GameData {
    let mut d = tests_pets::data();
    for (id, skills) in [(WOLVES, 208), (CATS, 209)] {
        d.pets.families.insert(
            id,
            Family {
                id,
                skills: [skills, 270],
                food_mask: 1,
                ..Default::default()
            },
        );
    }
    d.spells.spells.insert(SNAP, spell(SNAP, "Snap", 1));
    d.spells.spells.insert(SNAP_2, spell(SNAP_2, "Snap", 8));
    let row = |spell, skill, cost, superseded_by| PetAbility {
        spell,
        skill,
        cost,
        superseded_by,
        ..Default::default()
    };
    d.pets.abilities.insert(
        208,
        vec![row(SNAP, 208, 5, SNAP_2), row(SNAP_2, 208, 10, 0)],
    );
    d.pets
        .abilities
        .insert(209, vec![row(SNAP, 209, 5, SNAP_2)]);
    d.pets.abilities.insert(
        270,
        vec![PetAbility {
            acquire: 2,
            ..row(PHANTOM, 270, 0, 0)
        }],
    );
    for id in [SNAP, SNAP_2, PHANTOM] {
        let mut e = effect(36, 0, 5);
        e.trigger_spell = id;
        d.spells.spells.insert(
            teaching(id),
            tests_pets::spell(
                teaching(id),
                "Teaching",
                [e, SpellEffect::default(), SpellEffect::default()],
            ),
        );
        d.pets.teaching.insert(id, teaching(id));
    }
    d
}

/// Why the hunter's pet, of `family`, can't be taught `spell` now, if it
/// can't.
fn refused(m: &mut Map, hunter: Guid, pet: Guid, family: u32, spell: u32) -> Option<u8> {
    pets::state_mut(m, pet).unwrap().family = family;
    let t = m.realm.data.spells.get(teaching(spell)).cloned().unwrap();
    pets::testing::train_check(m, hunter, &t)
}

/// Thistle Tea's `PetTraining.build/2` "derives rank chains from skill
/// successors and merges family eligibility": Snap, in the wolves' line
/// and the cats', may be taught a wolf and a cat, not a boar; a wolf with
/// twenty points taught it pays its five once, though it is in two lines;
/// Snap's next rank takes its place for the five more of its ten; a free
/// row of a spell there isn't is neither taught nor learned on taming.
/// wvs ties ranks by `spell_chain` or by name, not by `superseded_by`;
/// ranks share their name throughout vanilla's pet lines.
#[tokio::test]
async fn an_ability_of_two_lines_is_taught_either_family_once_and_its_next_rank_for_the_rest() {
    use wow_world_messages::vanilla::SpellCastResult as R;
    let (_realm, _h, gh, map, pet, _wolf) =
        crate::game::tests_p5_ai_pets::tamed_with(training_world()).await;
    let out = map
        .query(move |m| {
            pets::state_mut(m, pet).unwrap().training_points = 20;
            let may = [WOLVES, CATS, BOARS].map(|f| refused(m, gh, pet, f, SNAP));
            pets::state_mut(m, pet).unwrap().family = WOLVES;
            let mut taught = Vec::new();
            for spell in [SNAP, SNAP_2] {
                let t = m.realm.data.spells.get(teaching(spell)).cloned().unwrap();
                pets::effect(m, gh, pet, &t, 0, 0, true, &Targets::default());
                let s = pets::state(m, pet).unwrap();
                taught.push((
                    s.training_points,
                    s.spells.contains(&SNAP),
                    s.spells.contains(&SNAP_2),
                ));
            }
            let phantom = refused(m, gh, pet, WOLVES, PHANTOM);
            let passives = pets::family_passives(&m.realm.data, WOLVES);
            (may, taught, phantom, passives)
        })
        .await
        .unwrap();
    let (may, taught, phantom, passives) = out;
    assert_eq!(may, [None, None, Some(R::BadTargets.as_int())]);
    assert_eq!(
        taught,
        [(15, true, false), (10, false, true)],
        "five for Snap; five more for its second rank, in its place"
    );
    assert_eq!(phantom, Some(R::NotKnown.as_int()));
    assert!(passives.is_empty(), "{passives:?}");
}
