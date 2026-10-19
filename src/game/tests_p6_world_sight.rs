//! Round 6's world tests, ported from Thistle Tea's
//! `test/game/world/visibility_test.exs`, `stealth_visibility_test.exs`
//! and `proximity_test.exs` (docs/parity/matrix.csv): who sees the
//! stealthed and the invisible as detection comes and goes, and as they
//! move; what moving within sight keeps; what a creature notices.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::{Map, MapHandle, ai, auras, grid, visibility};
use crate::game::tests_p3_combat::{self as base, HUMANS, PACK, START, aura_of, setup_with};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};

/// A hostile humanoid that notices no one (it is looked at, not looking).
const SNEAK: u32 = 3720;
/// A hostile one that notices within ten yards.
const WATCHER: u32 = 3721;

fn data() -> GameData {
    let mut d = base::data();
    for (entry, detection) in [(SNEAK, 0.1), (WATCHER, 10.0)] {
        let mut t = base::template(entry, 7);
        t.faction = PACK;
        t.detection_range = detection;
        t.level_min = 5;
        t.level_max = 5;
        d.creature_templates.insert(entry, t);
    }
    d
}

/// `yards` east of the start.
fn east(yards: f32) -> Vector3d {
    Vector3d {
        x: START.0 + yards,
        y: START.1,
        z: START.2,
    }
}

/// A creature of `entry` at level 5, `yards` east of the start, facing
/// west, seen by those near.
fn spawn_at(m: &mut Map, entry: u32, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let t = data.creature_template(entry).cloned().unwrap();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &t,
        5,
        east(yards),
        std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 1000);
    let g = m.add_creature(c.with_base());
    visibility::refresh(m, g);
    g
}

/// Move `g` (a player or a creature) to `yards` east of the start, as
/// walking there would: its cell, and its visibility if it went far
/// enough.
fn put(m: &mut Map, g: Guid, yards: f32) {
    let at = east(yards);
    let (anchor, old) = if let Some(p) = m.players.get_mut(&g) {
        p.movement.position = at;
        p.move_check = None;
        (p.vis_anchor, p.cell)
    } else if let Some(c) = m.creature_mut(g) {
        c.position = at;
        (c.vis_anchor, c.cell)
    } else {
        return;
    };
    let new = grid::cell_of(at.x, at.y);
    if let Some(p) = m.players.get_mut(&g) {
        p.cell = new;
    } else if let Some(c) = m.creature_mut(g) {
        c.cell = new;
    }
    m.grid.relocate(g, old, new);
    visibility::moved(m, g, anchor, at);
}

/// Whether `g` knows `object` now.
async fn knows(map: &MapHandle, g: Guid, object: Guid) -> bool {
    on(map, move |m| m.players[&g].known.contains(&object)).await
}

/// Thistle Tea's `Visibility.can_see?` "client aura cancellation
/// republishes detection and hides a tracked target" and "destroys a
/// hidden tracked unit and recreates it after invisibility ends", and its
/// stealth's "client cancellation removes the bonus, publishes it and
/// destroys the target": a creature going stealthed or invisible is
/// forgotten; Alice's detecting it shows it at once, her cancelling the
/// detection hides it again (mangos's `HandleInvisibilityDetect`,
/// `UpdateVisibilityForOwner`); its stealth or invisibility over, it is
/// seen again.
#[tokio::test]
async fn detection_gained_shows_the_hidden_and_cancelled_hides_them_again() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let sneak = on(&map, |m| spawn_at(m, SNEAK, 5.0)).await;
    a.sees(sneak).await;
    // Stealth past any level's sight but a bump's, and as much detection;
    // invisibility, and its detection.
    for (hide, detect, strength) in [
        (auras::AURA_MOD_STEALTH, auras::AURA_MOD_STEALTH_DETECT, 500),
        (
            auras::AURA_MOD_INVISIBILITY,
            auras::AURA_MOD_INVISIBILITY_DETECTION,
            10,
        ),
    ] {
        on(&map, move |m| aura_of(m, sneak, 96_000, hide, 0, strength)).await;
        a.forgets(sneak).await;
        on(&map, move |m| aura_of(m, ga, 96_001, detect, 0, strength)).await;
        a.sees(sneak).await;
        a.send(CMSG_CANCEL_AURA { id: 96_001 }).await;
        a.forgets(sneak).await;
        assert!(!knows(&map, ga, sneak).await);
        on(&map, move |m| auras::remove_spell(m, sneak, 96_000, None)).await;
        a.sees(sneak).await;
    }
}

/// Thistle Tea's visibility "keeps tracked entities that move between
/// still-visible cells": a player walking across the grid's cells within
/// sight isn't forgotten and told of again; past sight and its margin, he
/// is.
#[tokio::test]
async fn one_walking_within_sight_stays_known() {
    let (realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 2.0, START.1, START.2).await;
    a.sees(gb).await;
    for step in 1..=11 {
        on(&map, move |m| put(m, gb, 8.0 * step as f32)).await;
        harness::an_update(&map).await;
    }
    assert!(knows(&map, ga, gb).await, "88 yards off, in sight");
    assert!(
        !a.sees_within(gb, NONE_WITHIN).await,
        "never forgotten, so never told of again"
    );
    on(&map, move |m| put(m, gb, 140.0)).await;
    a.forgets(gb).await;
}

/// Thistle Tea's stealth "re-checks hidden units near a viewer that moves",
/// "a hidden announcement re-checks its announcer where it now stands" and
/// its proximity's "a stealthed walker keeps announcing every two yards
/// until it arrives": a stealthed creature walking toward Alice is seen as
/// soon as it is within her detection (fifteen yards, in front), looked at
/// every two yards it goes; Alice walking off forgets it, and back, sees
/// it.
#[tokio::test]
async fn a_stealthed_walker_is_seen_where_it_now_stands() {
    let (_realm, mut a, ga, map) = setup_with(data(), Class::Warrior).await;
    let sneak = on(&map, move |m| {
        let g = spawn_at(m, SNEAK, 20.0);
        aura_of(m, g, 96_010, auras::AURA_MOD_STEALTH, 0, 25);
        m.players.get_mut(&ga).unwrap().movement.orientation = 0.0;
        g
    })
    .await;
    harness::an_update(&map).await;
    assert!(!knows(&map, ga, sneak).await, "twenty yards off, unseen");
    for (yards, seen) in [(18.0, false), (16.0, false), (14.0, true)] {
        on(&map, move |m| put(m, sneak, yards)).await;
        harness::an_update(&map).await;
        assert_eq!(knows(&map, ga, sneak).await, seen, "at {yards} yards");
    }
    a.clear();
    // Alice walks west, away: it's behind her sight now.
    on(&map, move |m| put(m, ga, -10.0)).await;
    a.forgets(sneak).await;
    on(&map, move |m| put(m, ga, 0.0)).await;
    a.sees(sneak).await;
}

/// A creature's look for enemies, now.
fn turn(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m.creature_mut(g) {
        c.aggro_check_at = now;
    }
    ai::update_creatures(m);
}

fn attacking(m: &Map, c: Guid, g: Guid) -> bool {
    m.creature(c)
        .is_some_and(|c| c.combat.attacking && c.combat.victim == g)
}

/// Thistle Tea's proximity "uses the creature's published detect range
/// modifier" and "invisibility blocks aggro until the creature has
/// matching detection": a watcher noticing within ten yards (at Alice's
/// level, fourteen) doesn't notice her at eighteen; with eight more yards
/// of detect range (a detect-range aura), it does. Invisible beside it,
/// she goes unnoticed until it detects her kind of invisibility.
#[tokio::test]
async fn a_creature_notices_by_its_detect_range_and_not_the_invisible_it_cant_detect() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = on(&map, move |m| {
        let watcher = spawn_at(m, WATCHER, 18.0);
        turn(m, watcher);
        let far = attacking(m, watcher, ga);
        let reach = ai::attack_distance(m, watcher, ga);
        aura_of(m, watcher, 96_020, auras::AURA_MOD_DETECT_RANGE, 0, 8);
        let wider = ai::attack_distance(m, watcher, ga);
        turn(m, watcher);
        let near = attacking(m, watcher, ga);
        m.remove_creature(watcher);
        crate::game::map::combat::leave_combat(m, ga);

        let other = spawn_at(m, WATCHER, 3.0);
        aura_of(m, ga, 96_021, auras::AURA_MOD_INVISIBILITY, 0, 10);
        turn(m, other);
        let unseen = attacking(m, other, ga);
        aura_of(
            m,
            other,
            96_022,
            auras::AURA_MOD_INVISIBILITY_DETECTION,
            0,
            10,
        );
        turn(m, other);
        let detected = attacking(m, other, ga);
        (far, reach, wider, near, unseen, detected)
    })
    .await;
    assert_eq!(r, (false, 14.0, 22.0, true, false, true));
}

/// An aura of `kind` (misc 0) from `caster` on `target`.
fn aura_from(m: &mut Map, caster: Guid, target: Guid, id: u32, kind: u32) {
    let mut s = crate::game::data::spells::SpellTemplate {
        id,
        name: format!("Test Aura {id}"),
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = crate::game::data::spells::SpellEffect {
        effect: 6,
        aura: kind,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    let _ = auras::apply_spell(m, caster, target, &s, &[(0, 1)], auras::Context::default());
}

/// Thistle Tea's stealth "ownership and marks reveal only the appropriate
/// observer": a stealthed creature Alice has marked (Hunter's Mark's
/// `MOD_STALKED`) she sees; one Bob marked she doesn't; her own pet,
/// stealthed, she sees.
#[tokio::test]
async fn a_mark_shows_the_stealthed_to_its_marker_alone() {
    use crate::game::map::spells::stealth::sees;
    let (realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let r = on(&map, move |m| {
        let sneak = spawn_at(m, SNEAK, 5.0);
        aura_of(m, sneak, 96_030, auras::AURA_MOD_STEALTH, 0, 500);
        let hidden = sees(m, ga, sneak);
        aura_from(m, gb, sneak, 96_031, auras::AURA_MOD_STALKED);
        let bobs = (sees(m, ga, sneak), sees(m, gb, sneak));
        aura_from(m, ga, sneak, 96_032, auras::AURA_MOD_STALKED);
        let hers = sees(m, ga, sneak);
        let pet = spawn_at(m, SNEAK, 6.0);
        m.creature_mut(pet).unwrap().owner = Some(ga);
        aura_of(m, pet, 96_033, auras::AURA_MOD_STEALTH, 0, 500);
        (hidden, bobs, hers, sees(m, ga, pet), sees(m, gb, pet))
    })
    .await;
    assert!(!r.0, "stealthed");
    assert_eq!(r.1, (false, true), "Bob's mark shows it to Bob");
    assert!(r.2, "hers to her");
    assert!(r.3, "her pet she sees");
}

const TRAP: u32 = 96_100;
const OPEN_TRAP: u32 = 96_101;

fn trap_data() -> GameData {
    let mut d = data();
    for (entry, stealthed) in [(TRAP, 1), (OPEN_TRAP, 0)] {
        let mut data = [0u32; 24];
        data[9] = stealthed;
        d.gameobject_templates.insert(
            entry,
            crate::game::data::GameObjectTemplate {
                entry,
                kind: 6,
                display_id: 1000,
                name: format!("Trap {entry}"),
                size: 1.0,
                data,
                ..Default::default()
            },
        );
    }
    d
}

/// Thistle Tea's trap visibility "requires trap detection rather than
/// ordinary stealth or invisibility", "uses unowned trap faction and
/// treats an absent faction as hostile" and "re-checks listed traps
/// against the viewer's current hostility and detection": a stealthed trap
/// of Alice's hides from Mallory, her enemy, whatever she detects but
/// traps (invisibility detection of the third kind); gaining that, she is
/// told of it at once, and losing it, told to forget it. An unowned trap
/// hides from those its faction is hostile to, and from all without a
/// faction; an unstealthed one hides from no one.
#[tokio::test]
async fn a_trap_shows_to_its_owners_enemies_only_through_trap_detection() {
    use crate::game::map::objects::seen_by;
    let realm = harness::realm_with(trap_data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut o = Client::connect(&realm, "O");
    let go = o.enter("Mallory", Race::Human, Class::Warrior).await;
    o.heartbeat(START.0 + 3.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let trap = on(&map, move |m| {
        for x in [ga, go] {
            m.players.get_mut(&x).unwrap().fields.set_flag(
                PLAYER_FLAGS,
                crate::game::map::player::PLAYER_FLAGS_FFA_PVP,
                true,
            );
        }
        crate::game::map::objects::summon(m, Some(ga), TRAP, east(1.0), 0.0, 0, 0, 0, None).unwrap()
    })
    .await;
    a.sees(trap).await;
    harness::an_update(&map).await;
    assert!(!knows(&map, go, trap).await, "hidden from Mallory");
    let through = on(&map, move |m| {
        let mut seen = Vec::new();
        for (id, kind, misc) in [
            (96_110, auras::AURA_MOD_STEALTH_DETECT, 1),
            (96_111, auras::AURA_MOD_INVISIBILITY_DETECTION, 0),
            (96_112, auras::AURA_MOD_INVISIBILITY, 3),
        ] {
            aura_of(m, go, id, kind, misc, 300);
            seen.push(seen_by(m, go, trap));
            auras::remove_spell(m, go, id, None);
        }
        seen
    })
    .await;
    assert_eq!(through, [false; 3], "only trap detection");
    on(&map, move |m| {
        aura_of(
            m,
            go,
            96_113,
            auras::AURA_MOD_INVISIBILITY_DETECTION,
            3,
            300,
        )
    })
    .await;
    o.sees(trap).await;
    o.send(CMSG_CANCEL_AURA { id: 96_113 }).await;
    o.forgets(trap).await;

    let unowned = on(&map, move |m| {
        let at = east(2.0);
        let hidden =
            crate::game::map::objects::summon(m, None, TRAP, at, 0.0, 0, 0, 0, None).unwrap();
        let open =
            crate::game::map::objects::summon(m, None, OPEN_TRAP, at, 0.0, 0, 0, 0, None).unwrap();
        let mut seen = Vec::new();
        for faction in [0, PACK, HUMANS] {
            for t in [hidden, open] {
                m.gameobject_mut(t)
                    .unwrap()
                    .fields
                    .set_u32(GAMEOBJECT_FACTION, faction);
            }
            seen.push((seen_by(m, ga, hidden), seen_by(m, ga, open)));
        }
        seen
    })
    .await;
    assert_eq!(
        unowned,
        [(false, true), (false, true), (true, true)],
        "none, the pack's, the humans'"
    );
}
