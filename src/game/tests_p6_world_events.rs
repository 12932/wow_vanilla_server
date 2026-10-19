//! Round 6's world tests, ported from Thistle Tea's
//! `test/game/world/system/` (docs/parity/matrix.csv): spawn pools'
//! picks, the Scourge invasion called off, the elemental invasion's
//! hours, server variables.

use ahash::AHashSet;

use crate::game::data::scripts::{Pool, ScriptData};
use crate::game::harness;
use crate::game::map::pools::{Member, Selection};

/// Script data with `pools`, each spawn in its pool and each child's
/// mother set.
fn pools(list: Vec<Pool>) -> ScriptData {
    let mut s = ScriptData::default();
    for p in list {
        for (g, _) in &p.creatures {
            s.creature_pool.insert(*g, p.entry);
        }
        for (g, _) in &p.objects {
            s.object_pool.insert(*g, p.entry);
        }
        s.pools.insert(p.entry, p);
    }
    let mothers: Vec<(u32, u32)> = s
        .pools
        .values()
        .flat_map(|p| p.pools.iter().map(move |(c, _)| (*c, p.entry)))
        .collect();
    for (child, mother) in mothers {
        if let Some(c) = s.pools.get_mut(&child) {
            c.mother = mother;
        }
    }
    s
}

/// Every spawn under `root`, through its pools of pools.
fn under(s: &ScriptData, root: u32) -> AHashSet<Member> {
    let root_of = |mut p: u32| {
        for _ in 0..8 {
            match s.pools.get(&p) {
                Some(x) if x.mother != 0 && s.pools.contains_key(&x.mother) => p = x.mother,
                _ => break,
            }
        }
        p
    };
    s.creature_pool
        .iter()
        .filter(|(_, p)| root_of(**p) == root)
        .map(|(g, _)| Member::Creature(*g))
        .chain(
            s.object_pool
                .iter()
                .filter(|(_, p)| root_of(**p) == root)
                .map(|(g, _)| Member::Object(*g)),
        )
        .collect()
}

/// Thistle Tea's `Selection.initialize` "fills a multi-member limit without
/// duplicates": a pool of four objects with a limit of three puts out
/// three of them, each once, whichever three.
#[test]
fn a_pool_fills_its_limit_with_different_members() {
    let s = pools(vec![Pool {
        entry: 1,
        max_limit: 3,
        objects: (10..=13).map(|g| (g, 0.0)).collect(),
        ..Default::default()
    }]);
    let avail = under(&s, 1);
    let mut seen = AHashSet::new();
    for _ in 0..50 {
        let sel = Selection::initialize(&s, 1, &avail, &mut rand::rng());
        assert_eq!(sel.leaves.len(), 3, "{:?}", sel.leaves);
        assert!(sel.leaves.is_subset(&avail));
        seen.extend(sel.leaves);
    }
    assert_eq!(seen.len(), 4, "any three of the four");
}

/// Thistle Tea's `Selection.initialize` "expands selected child pools": a
/// pool of one pool puts out that pool's picks, up to its own limit.
#[test]
fn a_pool_of_pools_puts_out_its_picks_picks() {
    let s = pools(vec![
        Pool {
            entry: 1,
            max_limit: 1,
            pools: vec![(2, 0.0)],
            ..Default::default()
        },
        Pool {
            entry: 2,
            max_limit: 2,
            creatures: vec![(20, 0.0), (21, 0.0), (22, 0.0)],
            ..Default::default()
        },
    ]);
    let avail = under(&s, 1);
    assert_eq!(avail.len(), 3);
    for _ in 0..20 {
        let sel = Selection::initialize(&s, 1, &avail, &mut rand::rng());
        assert_eq!(sel.leaves.len(), 2, "the child's limit: {:?}", sel.leaves);
        assert!(sel.leaves.is_subset(&avail));
    }
}

/// The real world's pools, as vmangos ships them.
fn real_scripts() -> ScriptData {
    let dir = harness::real_data("db");
    harness::load_real(&dir).scripts
}

/// Thistle Tea's pool catalog "expands Hogger's template pool to its five
/// spawn rows", "selects a configured number of resource nodes" and
/// "expands a selected dungeon pack child pool": Hogger's pool (1270, of
/// one) has his five spawns (from `pool_creature_template`); pool 4303
/// puts out its 37 nodes; pool 4294, a pool of packs, one pack of three or
/// four.
#[test]
#[ignore = "extended"]
fn the_real_pools_pick_as_vmangos_has_them() {
    let s = real_scripts();
    let mut hogger: Vec<u32> = s
        .creature_pool
        .iter()
        .filter(|(_, p)| **p == 1270)
        .map(|(g, _)| *g)
        .collect();
    hogger.sort_unstable();
    assert_eq!(hogger, [80_531, 80_731, 81_027, 81_028, 81_029]);
    assert_eq!(s.pools[&1270].max_limit, 1);
    let one = Selection::initialize(&s, 1270, &under(&s, 1270), &mut rand::rng());
    assert_eq!(one.leaves.len(), 1, "one Hogger");

    let nodes = Selection::initialize(&s, 4303, &under(&s, 4303), &mut rand::rng());
    assert_eq!(nodes.leaves.len(), 37);

    let packs = &s.pools[&4294];
    assert!(!packs.pools.is_empty(), "a pool of packs");
    for _ in 0..10 {
        let sel = Selection::initialize(&s, 4294, &under(&s, 4294), &mut rand::rng());
        assert!((3..=4).contains(&sel.leaves.len()), "{:?}", sel.leaves);
        let mothers: AHashSet<u32> = sel
            .leaves
            .iter()
            .filter_map(|m| match m {
                Member::Creature(g) => s.creature_pool.get(g).copied(),
                Member::Object(g) => s.object_pool.get(g).copied(),
                Member::Pool(_) => None,
            })
            .collect();
        assert_eq!(mothers.len(), 1, "one pack: {mothers:?}");
    }
}

/// Thistle Tea's Scourge invasion "starts called off", "calls the Scourge
/// off and clears the map" and "shows the invasion only while it runs": at
/// the start none of its events is on and no one is told its states; on,
/// they are; called off, its events end, its Mouths go and its states are
/// told gone, and those coming in aren't told them.
#[tokio::test]
async fn the_scourge_invasion_starts_off_and_called_off_leaves_nothing() {
    use crate::game::invasion::{self, INVASION, MOUTH, ZONES};
    use crate::game::map::world_states::global;
    let mut d = crate::game::data::GameData::empty();
    let mut mouth = crate::game::tests_p3_combat::template(MOUTH, 7);
    mouth.name = "Mouth of Kel'Thuzad".into();
    d.creature_templates.insert(MOUTH, mouth);
    let realm = harness::realm_with(d);
    // Someone in each continent, for the Mouths to stand in.
    let mut a = harness::Client::connect(&realm, "A");
    a.enter(
        "Alice",
        wow_world_messages::vanilla::Race::Human,
        wow_world_messages::vanilla::Class::Warrior,
    )
    .await;
    let mut b = harness::Client::connect(&realm, "B");
    b.enter(
        "Grom",
        wow_world_messages::vanilla::Race::Orc,
        wow_world_messages::vanilla::Class::Warrior,
    )
    .await;
    let states = |realm: &crate::game::Realm| {
        global(realm)
            .iter()
            .filter(|s| s.state == 2219 || ZONES.iter().any(|z| z.invaded_state == s.state))
            .map(|s| (s.state, s.value))
            .collect::<Vec<_>>()
    };
    let driven_on = |realm: &crate::game::Realm| {
        realm
            .social
            .driven_events
            .lock()
            .iter()
            .filter(|(e, on)| **on && (**e == INVASION || (90..=99).contains(*e)))
            .map(|(e, _)| *e)
            .collect::<Vec<_>>()
    };
    assert!(!realm.social.invasion.lock().enabled, "starts off");
    invasion::sweep(&realm);
    assert_eq!(driven_on(&realm), Vec::<u32>::new());
    assert_eq!(states(&realm), [], "no one told of it");

    invasion::enable(&realm);
    assert!(driven_on(&realm).contains(&INVASION));
    let on = states(&realm);
    assert!(on.contains(&(2219, 0)), "{on:?}");
    assert!(on.iter().filter(|(s, _)| *s != 2219).all(|(_, v)| *v == 1));
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while realm.social.invasion.lock().mouths.len() < ZONES.len() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the Mouths never came"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    a.clear();
    invasion::disable(&realm);
    assert_eq!(driven_on(&realm), Vec::<u32>::new(), "its events end");
    assert_eq!(states(&realm), [], "left out for those coming in");
    let told = a
        .wait_for("the invaded state told gone", |m| match m {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_UPDATE_WORLD_STATE(
                w,
            ) if w.state.state == ZONES[0].invaded_state => Some(w.state.value),
            _ => None,
        })
        .await;
    assert_eq!(told, 0);
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while !realm.social.invasion.lock().mouths.is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "the Mouths stayed");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Thistle Tea's elemental invasion's hourly clock "brings each lord out
/// at the boss stage": begun, four hours on every element is at its
/// lord's stage, its rift's event and its lord's on.
#[test]
fn the_elemental_invasions_hours_bring_out_its_lords() {
    use crate::game::elemental::{ELEMENTS, INVASION, sweep};
    let realm = harness::realm();
    sweep(&realm);
    assert_eq!(realm.social.elemental.lock().stages, [1; 4], "begun");
    for _ in 0..4 {
        realm.social.elemental.lock().next_hour = 0;
        sweep(&realm);
    }
    assert_eq!(
        realm.social.elemental.lock().stages,
        [5; 4],
        "the lords' stage"
    );
    let d = realm.social.driven_events.lock();
    assert_eq!(d.get(&INVASION), Some(&true));
    for e in &ELEMENTS {
        assert_eq!(
            (d.get(&e.rift_event), d.get(&e.boss_event)),
            (Some(&true), Some(&true)),
            "rift {}",
            e.rift_event
        );
    }
    drop(d);
    // A fifth hour doesn't take them past it.
    realm.social.elemental.lock().next_hour = 0;
    sweep(&realm);
    assert_eq!(realm.social.elemental.lock().stages, [5; 4]);
}

/// Thistle Tea's server variables "retain unsigned values": a script's
/// server-wide variable holds any u32 and the last set; one never set is 0.
#[test]
fn a_server_variable_holds_any_u32() {
    use crate::game::map::scripts::{server_variable, set_server_variable};
    assert_eq!(server_variable(96_200), 0);
    set_server_variable(96_200, u32::MAX);
    assert_eq!(server_variable(96_200), u32::MAX);
    set_server_variable(96_200, 6);
    assert_eq!(server_variable(96_200), 6);
}
