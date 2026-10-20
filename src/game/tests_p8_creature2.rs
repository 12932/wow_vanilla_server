//! Round 8's creature tests against Thistle Tea's `test/game/core/creature`
//! (docs/parity/matrix.csv): a core hound dead for good gone at once, a
//! group's leader row, an NPC's cast at a unit immune to it, a hunter's
//! pet on its own spells, and a critter with nowhere to run.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::pets::CharmSpell;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::creature::{Life, Motion};
use crate::game::map::player_motion::{NO_PATH, NO_POINT};
use crate::game::map::{Map, auras, pets, scripts};
use crate::game::tests_guards::hardy;
use crate::game::tests_p3_combat::{BRUTE, HELPER, PACK, START, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;

const HOUND: u32 = 3_790;

/// A core hound's spawn three yards east of the start, and two of a
/// group's.
fn hounds() -> GameData {
    let mut d = data();
    let mut hound = template(HOUND, 1);
    hound.static_flags1 |= crate::game::map::combat::STATIC_UNKILLABLE;
    d.creature_templates.insert(HOUND, hound);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 7_900,
            entries: vec![HOUND],
            map: 0,
            x: START.0 + 3.0,
            y: START.1,
            z: START.2,
            orientation: std::f32::consts::PI,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        }],
    );
    d
}

/// Thistle Tea's "dies and despawns when no fighting hound survives"
/// (vmangos's core hound's `Kill_Self`: a kill, then `ForcedDespawn`): a
/// hound down with no kin fighting on dies at its time and is gone at
/// once, no corpse left to see; its spawn comes back on its own time.
#[tokio::test]
async fn a_fallen_hound_with_no_kin_fighting_dies_and_is_gone_at_once() {
    let (_realm, mut a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let h = guid::unit(HOUND, 7_900);
    a.sees(h).await;
    on(&map, move |m| {
        hardy(m, ga);
        let feign = ScriptStep {
            dataint: [0, 0, 0, 0],
            ..crate::game::tests_p3_entity::step(scripts::C_LINKED_FEIGN_DEATH, [300, HOUND, 30, 0])
        };
        scripts::execute(m, &feign, h, Guid::zero());
        crate::game::map::ai::attacked(m, h, ga);
        crate::game::map::combat::deal_damage(m, ga, h, 10_000_000);
    })
    .await;
    a.forgets(h).await;
    let after = on(&map, move |m| {
        let c = m.creature(h).unwrap();
        let back = match c.life {
            Life::Dead { respawn_at, .. } => Some(respawn_at.duration_since(m.now)),
            Life::Alive => None,
        };
        (c.alive(), c.despawned, c.combat.attacking, back)
    })
    .await;
    assert_eq!(
        (after.0, after.1, after.2),
        (false, true, false),
        "dead and gone"
    );
    let back = after.3.expect("its respawn");
    assert!(
        back > Duration::from_secs(590) && back <= Duration::from_secs(600),
        "on its spawn's time: {back:?}"
    );
}

/// Thistle Tea's `CreatureGroup.add/3` (vmangos's `AddMember`): the
/// leader's own row in `creature_groups` makes the group but adds no
/// member and none of its flags; a member's flags are the group's, and
/// stay its after that member leaves.
#[tokio::test]
async fn a_groups_leader_row_adds_no_flags_and_a_leavers_stay() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE creature_groups (leader_guid, member_guid, dist, angle, flags);
         INSERT INTO creature_groups VALUES (1, 1, 0, 0, 255);
         INSERT INTO creature_groups VALUES (1, 2, 3.0, 1.5, 0);
         INSERT INTO creature_groups VALUES (4, 4, 0, 0, 0);
         INSERT INTO creature_groups VALUES (4, 5, 3.0, 1.5, 2);
         INSERT INTO creature_groups VALUES (4, 6, 3.0, 3.0, 0);",
    )
    .unwrap();
    let mut d = data();
    crate::game::data::scripts_load::load(&db, &mut d);
    assert_eq!(
        d.scripts.groups[&1]
            .iter()
            .map(|m| (m.guid, m.flags))
            .collect::<Vec<_>>(),
        [(2, 0)],
        "the leader's row is no member"
    );
    assert_eq!(
        [1, 2, 4, 5, 6].map(|s| d.scripts.group_of.get(&s).copied()),
        [Some(1), Some(1), Some(4), Some(4), Some(4)]
    );
    let at = |guid: u32, dx: f32| CreatureSpawn {
        guid,
        entries: vec![BRUTE],
        map: 0,
        x: START.0 + 20.0 + dx,
        y: START.1,
        z: START.2,
        respawn_min: 600,
        respawn_max: 600,
        ..Default::default()
    };
    d.creature_spawns.insert(
        0,
        vec![at(1, 0.0), at(2, 2.0), at(4, 4.0), at(5, 6.0), at(6, 8.0)],
    );
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let g = |s: u32| guid::unit(BRUTE, s);
    let fighting = move |m: &Map, s: u32| m.creature(g(s)).is_some_and(|c| c.combat.attacking);
    let first = on(&map, move |m| {
        hardy(m, ga);
        // The first group: its leader's 255 is no aggro together.
        crate::game::map::ai::attacked(m, g(2), ga);
        // The second: its member 5's aggro together stays the group's
        // after it leaves.
        crate::game::map::creature_groups::leave(m, g(5));
        crate::game::map::ai::attacked(m, g(6), ga);
        [1, 2, 4, 5, 6].map(|s| fighting(m, s))
    })
    .await;
    assert_eq!(
        first,
        [false, true, true, false, true],
        "the first leader stays out; the second joins in, the leaver not"
    );
}

const BLAST: u32 = 95_800;

/// Thistle Tea's `MobSpells.attempt_commanded_cast/5`, "any-unit NPC casts
/// receive immunity from immutable observations": a creature's cast of
/// an any-unit (target 25) spell at a unit it may attack begins; at one
/// immune to creatures (`UNIT_FLAG_IMMUNE_TO_NPC`) its checks refuse it,
/// a bad target, and it doesn't begin.
#[tokio::test]
async fn a_creatures_any_unit_cast_at_one_immune_to_it_is_refused() {
    let mut d = data();
    d.spells.cast_times.insert(5, 1_500);
    let mut s = SpellTemplate {
        id: BLAST,
        name: "Blast".into(),
        range_index: 1,
        casting_time_index: 5,
        equipped_item_class: -1,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 1,
        target_a: 25,
        ..Default::default()
    };
    d.spells.spells.insert(BLAST, s);
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 10.0);
        let helper = spawn(m, ga, HELPER, 20, 5.0);
        let s = m.realm.data.spells.get(BLAST).cloned().unwrap();
        let at = crate::game::map::spells::Targets {
            unit: Some(helper),
            ..Default::default()
        };
        let check = |m: &mut Map| {
            let mods = crate::game::map::spells::mods::for_spell(m, brute, &s);
            crate::game::map::spells::checks::check(m, brute, &s, &at, None, false, &mods)
        };
        let casting = |m: &Map| {
            m.caster(brute)
                .and_then(|c| c.casting.as_ref().map(|x| x.spell))
        };
        let f = &mut m.creature_mut(helper).unwrap().fields;
        f.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::combat::UNIT_FLAG_IMMUNE_TO_NPC,
            true,
        );
        let refused = (
            check(m),
            scripts::cast(m, brute, helper, BLAST, 0),
            casting(m),
        );
        let f = &mut m.creature_mut(helper).unwrap().fields;
        f.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::combat::UNIT_FLAG_IMMUNE_TO_NPC,
            false,
        );
        let begun = (
            check(m),
            scripts::cast(m, brute, helper, BLAST, 0),
            casting(m),
        );
        (refused, begun)
    })
    .await;
    assert_eq!(
        out.0,
        (
            Some(crate::game::map::spells::checks::fail::BAD_TARGETS),
            false,
            None
        ),
        "refused at one immune to creatures"
    );
    assert_eq!(out.1, (None, true, Some(BLAST)), "begun at one it may hit");
}

const BITE: u32 = 95_810;
const THRALL_ONLY: u32 = 95_811;

/// Thistle Tea's "keeps pets and possessed pets on their own spellbooks":
/// a hunter's pet tamed from a beast with charm abilities has its own
/// spells, not those; and seen through by Eyes of the Beast, it keeps
/// them, a hunter's pet still.
#[tokio::test]
async fn a_hunters_pet_keeps_its_own_spells_and_under_eyes_of_the_beast_too() {
    use crate::game::tests_pets::{self, BOAR};
    let mut d = tests_pets::data();
    let bite = tests_pets::spell(
        BITE,
        "Bite",
        [
            tests_pets::effect(2, 9, 6),
            SpellEffect::default(),
            SpellEffect::default(),
        ],
    );
    d.spells.spells.insert(BITE, bite.clone());
    let mut thrall = bite;
    thrall.id = THRALL_ONLY;
    thrall.name = "Thrall's Blow".into();
    d.spells.spells.insert(THRALL_ONLY, thrall);
    d.pets.create_spells.insert(BOAR, vec![BITE]);
    d.pets.charm_spells.insert(
        BOAR,
        vec![CharmSpell {
            slot: 0,
            spell: THRALL_ONLY,
            availability: 100.0,
            cooldown_min_ms: 0,
            cooldown_max_ms: 0,
        }],
    );
    let (_realm, _h, gh, map, pet, _wolf) = crate::game::tests_p5_ai_pets::tamed_with(d).await;
    let state = move |m: &Map| {
        let s = pets::state(m, pet).unwrap();
        (s.kind, s.spells.clone(), s.bar)
    };
    let tamed = on(&map, move |m| state(m)).await;
    assert_eq!(tamed.0, pets::Kind::Hunter);
    assert!(tamed.1.contains(&BITE), "its own: {:?}", tamed.1);
    assert!(
        !tamed.1.contains(&THRALL_ONLY),
        "not a charm's: {:?}",
        tamed.1
    );
    let looked = on(&map, move |m| {
        let mut eyes = tests_pets::spell(
            95_812,
            "Eyes of the Beast",
            [
                tests_pets::effect(6, 0, 5),
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        );
        eyes.effects[0].aura = auras::AURA_MOD_POSSESS_PET;
        let _ = auras::apply_spell(m, gh, pet, &eyes, &[(0, 1)], auras::Context::default());
        pets::set_mover(m, gh, pet);
        let possessed = m.creature(pet).is_some_and(|c| {
            c.fields
                .has_flag(UNIT_FIELD_FLAGS, pets::UNIT_FLAG_POSSESSED)
        });
        (possessed, state(m))
    })
    .await;
    assert!(looked.0, "seen through");
    assert_eq!(looked.1, tamed, "its spells and bar as they were");
}

const CRITTER: u32 = 4_731;

fn critters() -> GameData {
    let mut d = data();
    let mut critter = template(CRITTER, 8);
    critter.faction = PACK;
    d.creature_templates.insert(CRITTER, critter);
    d
}

type Struck = (
    std::sync::Arc<crate::game::Realm>,
    crate::game::harness::Client,
    crate::game::map::MapHandle,
    Guid,
    Guid,
);

/// A critter struck by Alice, the navmesh failing it as `fault` says:
/// running from her, with nowhere to go.
async fn struck(fault: &'static str) -> Struck {
    let (realm, a, ga, map) = setup_with(critters(), Class::Mage).await;
    let g = on(&map, move |m| {
        hardy(m, ga);
        m.faults.arm(fault, u32::MAX);
        let g = spawn(m, ga, CRITTER, 20, 3.0);
        crate::game::map::combat::deal_damage(m, ga, g, 10);
        crate::game::map::ai::attacked(m, g, ga);
        g
    })
    .await;
    (realm, a, map, ga, g)
}

/// Its walk, its next try's wait from now, whom it selects, and whether
/// it is still running for its life.
fn stand(m: &Map, g: Guid) -> (bool, Option<Duration>, Guid, bool) {
    let c = m.creature(g).unwrap();
    (
        matches!(c.motion, Motion::Moving(_)),
        c.engagement
            .next_move_at
            .map(|t| t.saturating_duration_since(m.now)),
        c.fields.get_guid(UNIT_FIELD_TARGET),
        c.engagement.escape_at.is_some(),
    )
}

/// Thistle Tea's "failed paths use a bounded retry": a critter running
/// whose path the navmesh refuses stands and asks again a moment on (0.8
/// to 1.5 s), and runs once there is a way.
#[tokio::test]
async fn a_critter_refused_a_path_tries_again_a_moment_on() {
    let (_realm, _a, map, _ga, g) = struck(NO_PATH).await;
    until(&map, "the path refused", |m| m.faults.checked(NO_PATH) >= 1).await;
    let first = std::time::Instant::now();
    let (walking, wait, _, escaping) = on(&map, move |m| stand(m, g)).await;
    assert!(!walking && escaping, "standing, still escaping");
    // Its wait, less what the map has run since it was refused.
    let wait = wait.expect("its next try");
    assert!(
        wait > Duration::from_millis(500) && wait <= Duration::from_millis(1_500),
        "{wait:?}"
    );
    // Asked again, refused again, no sooner than its wait.
    until(&map, "asked again", |m| m.faults.checked(NO_PATH) >= 2).await;
    let between = first.elapsed();
    assert!(between >= Duration::from_millis(600), "{between:?}");
    on(&map, |m| m.faults.disarm(NO_PATH)).await;
    until(&map, "it runs once there's a way", move |m| {
        matches!(m.creature(g).unwrap().motion, Motion::Moving(_))
    })
    .await;
}

/// Thistle Tea's "missing terrain points pause without navigation or
/// retaliation": a critter with no point of the navmesh to run to stands
/// a second or so before it looks again, selecting no one and striking no
/// one, its escape still under way.
#[tokio::test]
async fn a_critter_with_no_point_to_run_to_stands_a_while_and_strikes_no_one() {
    let (_realm, _a, map, ga, g) = struck(NO_POINT).await;
    until(&map, "no point", |m| m.faults.checked(NO_POINT) >= 1).await;
    let (walking, wait, target, escaping) = on(&map, move |m| stand(m, g)).await;
    assert!(!walking && escaping, "standing, still escaping");
    // Its wait, less what the map has run since.
    let wait = wait.expect("its next look");
    assert!(
        wait > Duration::from_millis(500) && wait <= Duration::from_millis(1_500),
        "{wait:?}"
    );
    assert_eq!(target, Guid::zero(), "no one selected");
    assert_eq!(
        on(&map, move |m| m.faults.checked(NO_PATH)).await,
        0,
        "no way asked for"
    );
    let health = on(&map, move |m| {
        m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH)
    })
    .await;
    assert_eq!(health, 100_000_000, "Alice unharmed");
}

/// Thistle Tea's `land_corpse/3`, "falls to the highest surface below the
/// corpse with matching acceleration": a flyer's corpse falling twenty
/// yards takes a fall's time, its spline falling (0x2, with run mode
/// 0x100 as every move is sent), and is where gravity has it as it goes:
/// half a second down, 2.41 yards lower; on the ground at its end.
#[test]
fn a_corpses_fall_goes_under_gravity_to_the_ground() {
    use crate::game::map::creature::Walk;
    use crate::game::map::creature_motion::fall_time;
    let (from, to) = (
        Vector3d {
            x: 0.0,
            y: 0.0,
            z: 30.0,
        },
        Vector3d {
            x: 0.0,
            y: 0.0,
            z: 10.0,
        },
    );
    let time = fall_time(20.0);
    let t0 = std::time::Instant::now();
    let mut w = Walk::new(vec![from, to], 20.0 / time, t0, 1, false);
    w.falling = true;
    assert!((w.duration.as_secs_f32() - time).abs() < 0.001);
    assert_eq!(w.monster_move_flags(), 0x102);
    let half = w.position_at(t0 + Duration::from_millis(500)).0;
    assert!((half.z - 27.588_612).abs() < 0.001, "{half:?}");
    assert_eq!((half.x, half.y), (0.0, 0.0));
    let end = w.position_at(t0 + w.duration).0;
    assert!((end.z - 10.0).abs() < 0.001, "{end:?}");
    assert!(w.done(t0 + w.duration));
}

/// The real navmesh: a flyer killed in the air falls where the map has
/// it, a client coming into view mid-fall told of it falling (a create
/// with its falling spline), and what they are sent passes the client.
#[tokio::test]
#[ignore = "extended"]
async fn a_flyer_falling_dead_passes_the_client() {
    use crate::game::tests_client_emu::{capture, checker, run_checker};
    let bake = crate::game::harness::real_data("nav");
    if !bake.join("Azeroth.map").exists() {
        return;
    }
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    const BAT: u32 = 3_795;
    const HIGH: (f32, f32, f32) = (-9460.0, 62.0, 120.0);
    let mut d = data();
    let mut bat = template(BAT, 1);
    bat.inhabit_type = 4;
    d.creature_templates.insert(BAT, bat);
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: 7_950,
            entries: vec![BAT],
            map: 0,
            x: HIGH.0,
            y: HIGH.1,
            z: HIGH.2,
            respawn_min: 600,
            respawn_max: 600,
            ..Default::default()
        }],
    );
    let mut cfg = crate::game::harness::test_config();
    cfg.nav_dir = Some(bake);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let bat = guid::unit(BAT, 7_950);
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(HIGH.0 + 5.0, HIGH.1, 66.0).await;
    a.sees(bat).await;
    let ground = on(&map, move |m| {
        let ground = m.nav.as_mut().unwrap().ground(HIGH.0, HIGH.1, 80.0);
        crate::game::map::combat::deal_damage(m, bat, bat, 1_000_000);
        ground
    })
    .await
    .expect("Goldshire's ground");
    // Mid-fall: lower than it died, not down yet; and Bob comes.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let mid = on(&map, move |m| m.creature(bat).unwrap().position.z).await;
    assert!(
        mid < HIGH.2 - 1.0 && mid > ground + 1.0,
        "{mid} over {ground}"
    );
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(HIGH.0 - 5.0, HIGH.1, 66.0).await;
    b.sees(bat).await;
    let falls = crate::game::map::creature_motion::fall_time(HIGH.2 - ground);
    tokio::time::sleep(Duration::from_secs_f32(falls)).await;
    let landed = on(&map, move |m| m.creature(bat).unwrap().position.z).await;
    assert!((landed - ground).abs() < 0.1, "{landed} over {ground}");
    let (frames, counts) = capture(vec![&mut a, &mut b]);
    assert!(
        counts.get(&0x0DD).copied().unwrap_or(0) > 0,
        "the fall: {counts:?}"
    );
    run_checker(&check_py, &wow_exe, &frames);
}
