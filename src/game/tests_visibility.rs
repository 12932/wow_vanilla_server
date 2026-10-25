//! Visibility's quick path, its chunks across threads and its budget
//! (`map/visibility.rs`): each decides as the refresh before them did,
//! one object at a time, every pair looked at; and what changes who sees
//! whom without moving (stealth, a ghost's shape, a game master's sight, a
//! flight, a teleport, a view from elsewhere) is seen at once, though the
//! pairs it changes are well inside sight, where the quick path skips.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;

use crate::game::RealmConfig;
use crate::game::harness;
use crate::game::map::vis_bench::{CENTRE, Crowd, move_by, move_to};
use crate::game::map::{Map, auras, death, visibility};

/// A tokio runtime for the realm, and the crowd in it.
fn crowd_with(cfg: RealmConfig, n: usize, radius: f32) -> (tokio::runtime::Runtime, Crowd) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    let c = {
        let _in = rt.enter();
        Crowd::with_config(cfg, n, radius)
    };
    (rt, c)
}

fn config(threads: usize) -> RealmConfig {
    RealmConfig {
        visibility_threads: threads,
        ..harness::test_config()
    }
}

/// A small, fixed generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// In [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// An invisibility of a kind no one detects.
const HIDE: u32 = 95_400;

fn hide(m: &mut Map, g: Guid) {
    crate::game::tests_p3_combat::aura_of(m, g, HIDE, auras::AURA_MOD_INVISIBILITY, 9, 1);
}

/// `g` of the other side: its invisibility hides it from the crowd (a
/// side sees its own stealthed).
fn horde(m: &mut Map, g: Guid) {
    m.players.get_mut(&g).unwrap().team = crate::game::registry::Team::Horde;
}

fn unhide(m: &mut Map, g: Guid) {
    auras::remove_spell(m, g, HIDE, None);
}

fn ghost(m: &mut Map, g: Guid, on: bool) {
    let now = m.now;
    let p = m.players.get_mut(&g).unwrap();
    p.death = if on {
        death::Death::Ghost { since: now }
    } else {
        death::Death::Alive
    };
    // As the release and the resurrection mark them.
    m.visibility_dirty.insert(g);
}

fn gm(m: &mut Map, g: Guid, on: bool) {
    use crate::game::fields::PLAYER_FLAGS;
    let p = m.players.get_mut(&g).unwrap();
    p.fields
        .set_flag(PLAYER_FLAGS, crate::game::map::player::PLAYER_FLAGS_GM, on);
    // As `.gm` marks them.
    m.visibility_dirty.insert(g);
}

/// `g` sent `yards` east at once, as a teleport within the map does.
fn teleport(m: &mut Map, g: Guid, yards: f32) {
    let mut at = m.players[&g].position();
    at.x += yards;
    move_to(m, g, at);
    m.visibility_dirty.insert(g);
}

fn view(m: &mut Map, g: Guid, at: Option<Vector3d>) {
    m.players.get_mut(&g).unwrap().misc.view = at;
    // As a cinematic's camera and a far sight mark them.
    m.visibility_dirty.insert(g);
}

/// One of the walk's changes of state, the same in every crowd.
fn change(m: &mut Map, g: Guid, what: usize) {
    match what {
        0 => hide(m, g),
        1 => unhide(m, g),
        2 => ghost(m, g, true),
        3 => ghost(m, g, false),
        4 => gm(m, g, true),
        5 => gm(m, g, false),
        6 => teleport(m, g, 260.0),
        7 => teleport(m, g, -260.0),
        8 => {
            let at = m.players[&g].position();
            view(
                m,
                g,
                Some(Vector3d {
                    x: at.x + 90.0,
                    ..at
                }),
            );
        }
        _ => view(m, g, None),
    }
}

/// `n` players on a disc of `radius` walking about: each a heading and a
/// speed (up to `speed` yards an update), turning now and then, a few
/// changing state each update. The same walk in each crowd, an update at a
/// time; after each, `check` compares them.
fn walk(
    crowds: &mut [Crowd],
    seed: u64,
    updates: usize,
    speed: f32,
    changes: bool,
    mut check: impl FnMut(usize, &mut [Crowd]),
) {
    let n = crowds[0].guids.len();
    for c in crowds.iter_mut() {
        for i in (0..n).step_by(5) {
            let g = c.guids[i];
            horde(&mut c.map, g);
        }
    }
    let mut rng = Rng(seed);
    let mut heading: Vec<(f32, f32)> = (0..n)
        .map(|_| (rng.unit() * std::f32::consts::TAU, rng.unit() * speed))
        .collect();
    for u in 0..updates {
        for (i, h) in heading.iter_mut().enumerate() {
            if rng.unit() < 0.05 {
                *h = (rng.unit() * std::f32::consts::TAU, rng.unit() * speed);
            }
            let (dx, dy) = (h.0.cos() * h.1, h.0.sin() * h.1);
            for c in crowds.iter_mut() {
                let g = c.guids[i];
                move_by(&mut c.map, g, dx, dy);
            }
        }
        if changes {
            for _ in 0..3 {
                let (i, what) = (rng.below(n), rng.below(10));
                for c in crowds.iter_mut() {
                    let g = c.guids[i];
                    change(&mut c.map, g, what);
                }
            }
        }
        for c in crowds.iter_mut() {
            visibility::refresh_dirty(&mut c.map);
        }
        check(u, crowds);
        for c in crowds.iter_mut() {
            c.drain();
        }
    }
}

/// The quick path, a walk with every change of state in it: after every
/// update each player knows what they knew when each refresh was worked
/// out alone and whole.
#[test]
fn the_quick_path_decides_as_a_whole_refresh_does() {
    for seed in [7, 1_234_567, 99_999_989] {
        let (_rt, quick) = crowd_with(config(1), 120, 110.0);
        let (_rt2, mut whole) = crowd_with(config(1), 120, 110.0);
        whole.map.visibility_dirty.one_by_one = true;
        let _in = _rt.enter();
        let mut crowds = [quick, whole];
        assert_eq!(crowds[0].knowledge(), crowds[1].knowledge());
        walk(&mut crowds, seed, 200, 1.5, true, |u, c| {
            assert_eq!(
                c[0].knowledge(),
                c[1].knowledge(),
                "seed {seed}, update {u}: the quick path decided otherwise"
            );
        });
        let v = &crowds[0].map.visibility_dirty;
        assert!(
            v.skipped > 5_000,
            "the quick path skipped: {} {}",
            v.skipped,
            v.looked
        );
    }
}

/// The quick path skips: in a crowd all in sight of each other, moving,
/// most pairs aren't looked at, and what each knows is still everyone.
#[test]
fn a_crowd_in_sight_of_itself_knows_itself() {
    let (_rt, c) = crowd_with(config(1), 200, 30.0);
    let _in = _rt.enter();
    let mut crowds = [c];
    walk(&mut crowds, 42, 60, 0.3, false, |_, c| {
        for (g, known) in c[0].knowledge() {
            assert_eq!(known.len(), 199, "{g} knows everyone");
        }
    });
    let v = &crowds[0].map.visibility_dirty;
    assert!(
        v.skipped > 20 * v.looked,
        "most skipped: {} {}",
        v.skipped,
        v.looked
    );
}

/// The chunks' decisions worked out across threads are the ones worked
/// out on the map's thread: the same creates, in the same packets, and the
/// same forgets, to each observer, every update, a crowd in step and at
/// random.
#[test]
fn decisions_across_threads_are_the_ones_on_the_maps() {
    let (_rt, serial) = crowd_with(config(1), 300, 60.0);
    let (_rt2, parallel) = crowd_with(config(4), 300, 60.0);
    let _in = _rt.enter();
    let mut crowds = [serial, parallel];
    assert_eq!(crowds[0].knowledge(), crowds[1].knowledge());
    for _ in 0..4 {
        for c in crowds.iter_mut() {
            c.shift(1, 0, 9.0);
            visibility::refresh_dirty(&mut c.map);
        }
        assert_eq!(crowds[0].sent(), crowds[1].sent());
        assert_eq!(crowds[0].knowledge(), crowds[1].knowledge());
        for c in crowds.iter_mut() {
            c.drain();
        }
    }
    walk(&mut crowds, 5, 80, 3.0, true, |u, c| {
        let (a, b) = c.split_at_mut(1);
        assert_eq!(a[0].sent(), b[0].sent(), "update {u}");
        assert_eq!(a[0].knowledge(), b[0].knowledge(), "update {u}");
    });
}

/// Over the budget, an update does a chunk, oldest first, and leaves the
/// rest to the next: everything is worked out in the end, as it would
/// have been at once.
#[test]
fn refreshes_over_the_budget_wait_for_the_next_update_oldest_first() {
    let budgeted = RealmConfig {
        visibility_budget: Duration::ZERO,
        ..config(1)
    };
    let (_rt, mut c) = crowd_with(budgeted, 400, 60.0);
    let (_rt2, mut at_once) = crowd_with(config(1), 400, 60.0);
    let _in = _rt.enter();
    let order: Vec<Guid> = c.guids.iter().rev().copied().collect();
    for crowd in [&mut c, &mut at_once] {
        for &g in &order {
            move_by(&mut crowd.map, g, 9.0, 0.0);
        }
    }
    let mut left = Vec::new();
    for _ in 0..4 {
        visibility::refresh_dirty(&mut c.map);
        left.push(c.map.visibility_dirty.len());
        c.drain();
    }
    assert_eq!(left, [272, 144, 16, 0], "a chunk of 128 an update");
    visibility::refresh_dirty(&mut at_once.map);
    assert!(at_once.map.visibility_dirty.is_empty());
    assert_eq!(c.knowledge(), at_once.knowledge());

    // Oldest first: the first marked are the first worked out.
    for crowd in [&mut c] {
        for &g in &order {
            move_by(&mut crowd.map, g, -9.0, 0.0);
        }
    }
    let anchor = |c: &Crowd, g: Guid| c.map.players[&g].vis_anchor.x;
    let before: Vec<f32> = order.iter().map(|&g| anchor(&c, g)).collect();
    visibility::refresh_dirty(&mut c.map);
    let moved: Vec<bool> = order
        .iter()
        .zip(&before)
        .map(|(&g, b)| anchor(&c, g) != *b)
        .collect();
    assert!(moved[..128].iter().all(|m| *m), "the oldest 128 first");
    assert!(moved[128..].iter().all(|m| !*m), "and only them");
}

/// A crowd all in sight of each other, settled, and one of them.
fn settled() -> (tokio::runtime::Runtime, Crowd, Guid) {
    let (rt, mut c) = crowd_with(config(1), 60, 25.0);
    {
        let _in = rt.enter();
        for _ in 0..2 {
            c.shift(1, 0, 9.0);
            c.refresh();
        }
    }
    let g = c.guids[7];
    (rt, c, g)
}

fn knows(c: &Crowd, observer: Guid, object: Guid) -> bool {
    c.map.players[&observer].known.contains(&object)
}

/// Everyone but `g` knows `g`, and `g` knows everyone: or not.
fn all_know(c: &Crowd, g: Guid) -> (bool, bool) {
    let others = || c.guids.iter().copied().filter(move |&o| o != g);
    (
        others().all(|o| knows(c, o, g)),
        others().all(|o| knows(c, g, o)),
    )
}

fn none_know(c: &Crowd, g: Guid) -> (bool, bool) {
    let others = || c.guids.iter().copied().filter(move |&o| o != g);
    (
        others().all(|o| !knows(c, o, g)),
        others().all(|o| !knows(c, g, o)),
    )
}

/// One gone invisible among a crowd moving about it is forgotten at once
/// by all, though each is well inside the others' sight; seen again as it
/// ends.
#[test]
fn invisibility_parts_a_pair_the_quick_path_would_skip() {
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    horde(&mut c.map, g);
    hide(&mut c.map, g);
    assert!(none_know(&c, g).0, "forgotten at once");
    c.drain();
    // The others move on: none sees it again.
    c.shift(1, 0, 9.0);
    c.refresh();
    assert!(none_know(&c, g).0);
    unhide(&mut c.map, g);
    assert_eq!(all_know(&c, g), (true, true), "seen again at once");
}

/// A ghost is forgotten by the living about it (it has no corpse here to
/// see by), and sees none of them; risen, both again.
#[test]
fn a_ghost_parts_a_pair_the_quick_path_would_skip() {
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    ghost(&mut c.map, g, true);
    c.shift(1, 0, 9.0);
    c.refresh();
    assert_eq!(none_know(&c, g), (true, true));
    ghost(&mut c.map, g, false);
    c.shift(1, 0, 9.0);
    c.refresh();
    assert_eq!(all_know(&c, g), (true, true));
}

/// A game master sees one invisible among the crowd from `.gm on`, not
/// from their next steps; and no longer from `.gm off`.
#[test]
fn a_game_masters_sight_changes_at_once() {
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    let hidden = c.guids[3];
    horde(&mut c.map, hidden);
    hide(&mut c.map, hidden);
    assert!(!knows(&c, g, hidden));
    gm(&mut c.map, g, true);
    c.refresh();
    assert!(knows(&c, g, hidden), "seen from .gm on");
    gm(&mut c.map, g, false);
    c.refresh();
    assert!(!knows(&c, g, hidden), "forgotten from .gm off");
}

/// A flight is noticed at the flier's first move (nothing marks it as it
/// takes off): it sees as far as a flier does at once, and as a walker
/// does again once landed.
#[test]
fn a_flight_is_seen_at_its_first_move_and_its_landing() {
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    let far = *c.guids.last().unwrap();
    teleport(&mut c.map, far, 250.0);
    c.refresh();
    assert!(!knows(&c, g, far));
    let now = c.map.now;
    let at = c.map.players[&g].position();
    c.map.players.get_mut(&g).unwrap().flight = Some(crate::game::map::npc::Flight {
        walk: crate::game::map::creature::Walk::new(vec![at, at], 30.0, now, 1, true),
        destination: 2,
        mount: 0,
    });
    move_by(&mut c.map, g, 1.0, 0.0);
    assert_eq!(
        c.map.visibility_dirty.kind(&g),
        Some(visibility::Kind::Full),
        "taken off: worked out whole"
    );
    c.refresh();
    assert!(knows(&c, g, far), "a flier sees 400 yards");
    c.map.players.get_mut(&g).unwrap().flight = None;
    move_by(&mut c.map, g, 40.0, 0.0);
    c.refresh();
    assert!(!knows(&c, g, far), "landed, as far as a walker");
    for &o in c.guids.iter().filter(|&&o| o != g && o != far) {
        assert!(knows(&c, g, o) && knows(&c, o, g), "the crowd still");
    }
}

/// A teleport out of the crowd parts it both ways, one back introduces it
/// both ways.
#[test]
fn a_teleport_parts_and_introduces_both_ways() {
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    teleport(&mut c.map, g, 500.0);
    c.refresh();
    assert_eq!(none_know(&c, g), (true, true));
    teleport(&mut c.map, g, -500.0);
    c.refresh();
    assert_eq!(all_know(&c, g), (true, true));
}

/// A view from elsewhere (a cinematic's camera, a far sight) knows what is
/// near it, not what is near the player; and the crowd again once it ends.
#[test]
fn a_view_from_elsewhere_sees_from_there() {
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    let far = *c.guids.last().unwrap();
    teleport(&mut c.map, far, 300.0);
    c.refresh();
    let at = c.map.players[&far].position();
    view(&mut c.map, g, Some(at));
    c.refresh();
    assert!(knows(&c, g, far));
    let crowd = c.guids[1];
    assert!(!knows(&c, g, crowd), "the crowd is 300 yards off the view");
    view(&mut c.map, g, None);
    c.refresh();
    assert!(knows(&c, g, crowd) && !knows(&c, g, far));
}

/// One that goes further than [`visibility::DRIFT`] before its refresh
/// (a charge) is no longer taken as plain by the others, and is worked
/// out whole.
#[test]
fn one_gone_past_its_drift_is_worked_out_whole() {
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    let plain = |c: &Crowd| {
        let cell = c.map.players[&g].cell;
        c.map.grid.entry(g, cell).unwrap().plain
    };
    assert!(plain(&c));
    move_by(&mut c.map, g, 9.0, 0.0);
    assert_eq!(
        c.map.visibility_dirty.kind(&g),
        Some(visibility::Kind::Moved)
    );
    move_by(&mut c.map, g, 9.0, 0.0);
    assert!(!plain(&c), "past its drift");
    assert_eq!(
        c.map.visibility_dirty.kind(&g),
        Some(visibility::Kind::Full)
    );
    c.refresh();
    assert!(plain(&c), "plain again from its refresh");
    let cell = c.map.players[&g].cell;
    let e = *c.map.grid.entry(g, cell).unwrap();
    let at = CENTRE;
    assert!((e.ax - c.map.players[&g].position().x).abs() < 0.01 && e.ax != at.0);
}

/// What a client has, from what it was sent: the objects it was told of
/// and not told to forget, and what it was sent that it couldn't take (a
/// create of one it has, a forget or values of one it hasn't: its own
/// player it always has).
#[derive(Debug, Default, Clone, PartialEq)]
struct ClientView {
    me: Guid,
    has: std::collections::BTreeSet<Guid>,
    wrong: Vec<String>,
}

/// Each client's frames since the last look, in order, onto its view.
fn replay(c: &mut Crowd, views: &mut [ClientView]) {
    use wow_world_messages::vanilla::Object_UpdateType;
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    for (i, v) in views.iter_mut().enumerate() {
        for f in c.take_frames(i) {
            let Ok(m) = ServerOpcodeMessage::read_unencrypted(&f[..]) else {
                v.wrong.push("a frame wow_messages can't read".into());
                continue;
            };
            let objects = match &m {
                ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
                ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
                _ => continue,
            };
            for o in objects {
                match &o.update_type {
                    Object_UpdateType::OutOfRangeObjects { guids } => {
                        for g in guids {
                            if !v.has.remove(g) {
                                v.wrong.push(format!("told to forget {g:?}, unknown"));
                            }
                        }
                    }
                    Object_UpdateType::CreateObject { guid3, .. }
                    | Object_UpdateType::CreateObject2 { guid3, .. } => {
                        if !v.has.insert(*guid3) {
                            v.wrong.push(format!("told of {guid3:?} again"));
                        }
                    }
                    Object_UpdateType::Values { guid1, .. }
                        if *guid1 != v.me && !v.has.contains(guid1) =>
                    {
                        v.wrong.push(format!("values of {guid1:?}, unknown"));
                    }
                    _ => {}
                }
            }
        }
    }
}

/// A crowd's creates shared among it (`visibility::Crowds`) leave each
/// client with what it had when each create went into its own batch: over
/// a walk with every change of state in it (an invisibility on and off, a
/// ghost, a game master's sight, a teleport out and back, a view from
/// elsewhere), each client is told of what its player knows, each object
/// once, and of nothing about an object before it has been told of it, the
/// same in both, update by update.
#[test]
fn shared_creates_leave_each_client_as_one_by_one() {
    for seed in [3, 77_777, 4_242_421] {
        let (_rt, shared) = crowd_with(config(4), 160, 90.0);
        let (_rt2, mut alone) = crowd_with(config(4), 160, 90.0);
        let _in = _rt.enter();
        alone.creates_one_by_one(true);
        let mut crowds = [shared, alone];
        // What making the crowd sent was dropped: each client has what its
        // player knows.
        let mut views: [Vec<ClientView>; 2] = [Vec::new(), Vec::new()];
        for (c, v) in crowds.iter_mut().zip(views.iter_mut()) {
            c.drain();
            c.map.visibility_dirty.shared_creates = 0;
            *v = c
                .guids
                .iter()
                .map(|&g| ClientView {
                    me: g,
                    has: c.map.players[&g].known.iter().copied().collect(),
                    wrong: Vec::new(),
                })
                .collect();
        }
        let mut told = 0usize;
        walk(&mut crowds, seed, 60, 4.0, true, |u, c| {
            for (c, v) in c.iter_mut().zip(views.iter_mut()) {
                c.end_update();
                replay(c, v);
                for (i, &g) in c.guids.iter().enumerate() {
                    assert!(
                        v[i].wrong.is_empty(),
                        "seed {seed}, update {u}: {g:?}'s client: {:?}",
                        v[i].wrong
                    );
                    let known: std::collections::BTreeSet<Guid> =
                        c.map.players[&g].known.iter().copied().collect();
                    assert_eq!(v[i].has, known, "seed {seed}, update {u}: {g:?}'s client");
                }
            }
            assert_eq!(views[0], views[1], "seed {seed}, update {u}");
            told += views[0].iter().map(|v| v.has.len()).sum::<usize>();
        });
        assert!(told > 0);
        let [shared, alone] = &crowds;
        assert!(
            shared.map.visibility_dirty.shared_creates > 1_000,
            "shared: {}",
            shared.map.visibility_dirty.shared_creates
        );
        assert_eq!(alone.map.visibility_dirty.shared_creates, 0);
    }
}

/// One coming out of invisibility among a crowd is told to it in a packet
/// shared among it, no client held for it; and what the map sends about it
/// at once after reaches each of them after its create.
#[test]
fn a_crowds_shared_create_comes_before_what_names_its_object() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    use wow_world_messages::vanilla::{Emote, SMSG_EMOTE};
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    horde(&mut c.map, g);
    hide(&mut c.map, g);
    c.end_update();
    c.drain();
    let before = c.map.visibility_dirty.shared_creates;
    unhide(&mut c.map, g);
    assert_eq!(all_know(&c, g), (true, true), "seen again at once");
    assert_eq!(
        c.map.visibility_dirty.shared_creates - before,
        c.guids.len() as u64 - 1,
        "every create shared"
    );
    assert!(
        c.map.holding.iter().all(|(o, _)| *o == g),
        "no one but itself held for it"
    );
    let frame = crate::game::conn::encode(&SMSG_EMOTE {
        emote: Emote::OneshotWave,
        guid: g,
    })
    .expect("encoded");
    c.map.send_to_observers(g, &frame, false);
    c.end_update();
    for (i, &o) in c.guids.clone().iter().enumerate() {
        if o == g {
            continue;
        }
        let mut created = None;
        let mut waved = None;
        for (k, f) in c.take_frames(i).iter().enumerate() {
            match ServerOpcodeMessage::read_unencrypted(&f[..]) {
                Ok(m @ ServerOpcodeMessage::SMSG_UPDATE_OBJECT(_))
                | Ok(m @ ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(_)) => {
                    if harness::creates(&m).contains(&g) {
                        created.get_or_insert(k);
                    }
                }
                Ok(ServerOpcodeMessage::SMSG_EMOTE(e)) if e.guid == g => {
                    waved.get_or_insert(k);
                }
                _ => {}
            }
        }
        let (Some(created), Some(waved)) = (created, waved) else {
            panic!("{o:?}: created {created:?}, waved {waved:?}");
        };
        assert!(created < waved, "{o:?}: the create first");
    }
}

/// A spell's area put down among a crowd is seen by it at once, its create
/// shared among it, no client held for it (it was every client near it,
/// until the event's end or the update's visibility); and each is told of
/// it once.
#[test]
fn a_spell_area_is_seen_at_once_its_create_shared() {
    use crate::game::map::spells::dynamic;
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let (rt, mut c, g) = settled();
    let _in = rt.enter();
    c.end_update();
    c.drain();
    let at = c.map.players[&g].position();
    let area = dynamic::spawn(&mut c.map, g, 10, dynamic::KIND_AREA_SPELL, at, 8.0, Some(8000));
    for &o in &c.guids {
        assert!(knows(&c, o, area), "{o:?} sees it");
    }
    assert!(c.map.holding.is_empty(), "no one held for it");
    assert!(c.map.fresh.is_empty(), "nothing waits for the update");
    c.end_update();
    for i in 0..c.guids.len() {
        let told = c
            .take_frames(i)
            .iter()
            .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
            .flat_map(|m| harness::creates(&m))
            .filter(|o| *o == area)
            .count();
        assert_eq!(told, 1, "client {i} told of it once");
    }
}

/// Past the budget a chunk stops after the refresh at hand (the update's
/// first done whatever it costs), and the rest wait for the next update,
/// first in line, as they were: everything is worked out in the end, as it
/// would have been at once, and each update does at least one.
#[test]
fn a_chunk_stops_where_the_budget_runs_out() {
    let budgeted = RealmConfig {
        visibility_budget: Duration::from_nanos(1),
        ..config(1)
    };
    let (_rt, mut c) = crowd_with(budgeted, 90, 60.0);
    let (_rt2, mut at_once) = crowd_with(config(1), 90, 60.0);
    let _in = _rt.enter();
    for crowd in [&mut c, &mut at_once] {
        crowd.shift(1, 0, 9.0);
    }
    let mut left = vec![c.map.visibility_dirty.len()];
    while !c.map.visibility_dirty.is_empty() {
        visibility::refresh_dirty(&mut c.map);
        left.push(c.map.visibility_dirty.len());
        c.drain();
        assert!(left.len() < 200, "never done: {left:?}");
    }
    assert!(
        left.windows(2).all(|w| w[1] < w[0]),
        "at least one an update: {left:?}"
    );
    assert!(left.len() > 4, "spread over updates: {left:?}");
    visibility::refresh_dirty(&mut at_once.map);
    assert_eq!(c.knowledge(), at_once.knowledge());
}
