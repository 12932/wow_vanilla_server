//! Round 9's entity tests on the real world (extended), ported from Thistle
//! Tea's `effect_resolver/*_dbc_test.exs`, `wild_trap_dbc_test.exs`,
//! `player_breathing_maps_test.exs` and `ai_environment_test.exs`
//! (docs/parity/matrix.csv): cones, an area's rolls and its one cast's
//! end, Arcane Missiles' channel and missiles, the dead and a trigger,
//! Frost Trap's areas, a breath under water, and a script's random move.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, auras, scripts};
use crate::game::tests_p3_spells_kit as kit;

const ARCANE_EXPLOSION: u32 = 1449;
const ARCANE_MISSILES: u32 = 5143;
const ARCANE_MISSILE: u32 = 7268;
const FROSTBITE: u32 = 12_494;
const RELENTLESS_STRIKES_EFFECT: u32 = 14_181;
const FROST_TRAP: u32 = 13_809;
const FROST_TRAP_AURA: u32 = 13_810;
/// Made up: spells of all schools miss the one it is on.
const WARD: u32 = 990_990;

/// `g` warded: every spell at it misses, as the fixed rolls of the tests
/// have a chance this small (a hundred points off it).
fn ward(m: &mut Map, g: Guid) {
    let s = kit::made_up(
        WARD,
        6,
        auras::AURA_MOD_ATTACKER_SPELL_HIT_CHANCE,
        127,
        -100,
    );
    kit::apply_template(m, g, g, &s);
}

fn stacks(m: &Map, g: Guid, spell: u32) -> u32 {
    kit::holder(m, g, spell).map_or(0, |h| h.stacks)
}

/// The `SMSG_SPELL_GO`s of `spell` `c` is told of soon, by their hits.
async fn goes(c: &mut Client, spell: u32) -> Vec<Vec<Guid>> {
    let mut out = Vec::new();
    while let Some(hits) = c
        .gets(Duration::from_millis(300), |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => Some(g.hits.clone()),
            _ => None,
        })
        .await
    {
        out.push(hits);
    }
    out
}

/// The `SMSG_SPELLLOGMISS`es of `spell` `c` is told of soon: whom, and why
/// (2 resisted).
async fn misses(c: &mut Client, spell: u32) -> Vec<(u64, u8)> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    while tokio::time::Instant::now() < deadline {
        let left = deadline - tokio::time::Instant::now();
        let got = tokio::time::timeout(
            left,
            c.wait_frame(
                "SMSG_SPELLLOGMISS",
                spells::outcome::SMSG_SPELLLOGMISS,
                |b| b[..4] == spell.to_le_bytes(),
            ),
        )
        .await;
        let Ok(b) = got else {
            break;
        };
        let target = u64::from_le_bytes(b[17..25].try_into().unwrap());
        out.push((target, b[25]));
    }
    out
}

/// Lava Breath and Sand Blast breathe on the enemies in their cone before
/// the caster (60 degrees, no cone of their own in vmangos's table), not on
/// the enemy selected behind: of eight about the caster, the one ahead
/// (Thistle Tea's "Lava Breath and Sand Blast use their cone instead of the
/// selected enemy").
#[tokio::test]
#[ignore = "extended"]
async fn lava_breath_and_sand_blast_breathe_on_their_cone_not_the_selected() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, g, map) = kit::player(&realm, "P9ECONE", "Coner", Race::Human, Class::Mage).await;
    let r = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            m.players.get_mut(&g).unwrap().movement.orientation = 0.0;
            let here = m.players[&g].position();
            let ring: Vec<Guid> = (0..8)
                .map(|i| {
                    let d = kit::dummy(m, g, 60, 1.0);
                    let a = i as f32 * std::f32::consts::FRAC_PI_4;
                    let at = Vector3d {
                        x: here.x + 5.0 * a.cos(),
                        y: here.y + 5.0 * a.sin(),
                        z: here.z,
                    };
                    scripts::place(m, d, at, 0.0);
                    d
                })
                .collect();
            [19_272u32, 26_102].map(|id| {
                let s = m.realm.data.spells.get(id).unwrap().clone();
                let mods = spells::mods::for_spell(m, g, &s);
                let behind = Targets {
                    unit: Some(ring[4]),
                    ..Default::default()
                };
                let hit = spells::targets::effect_targets(m, g, &s, 0, &behind, &mods);
                // Of the ring: the world's own creatures about may be in
                // the cone too.
                let which: Vec<usize> = hit
                    .iter()
                    .filter_map(|h| ring.iter().position(|r| r == h))
                    .collect();
                (m.realm.data.spells.cones.get(&id).copied(), which)
            })
        })
        .await
        .unwrap();
    // The one ahead of the eight, at 0 degrees; the next are 45 off.
    assert_eq!(r, [(None, vec![0]), (None, vec![0])]);
}

/// Arcane Explosion rolls each enemy it reaches for itself: one warded
/// against spells resists it (`SMSG_SPELLLOGMISS`, 2), the other takes its
/// damage; both are in its one `SMSG_SPELL_GO`, as 1.12 lists every target
/// there (mangos-zero's `WriteSpellGoTargets`). Its cast ends once, however
/// many it lands on: Unstable Power loses one stack (Thistle Tea's "area
/// triggers roll each target and keep separate launch lists" and "an area
/// trigger emits one completion across multiple recipients").
#[tokio::test]
#[ignore = "extended"]
async fn arcane_explosion_rolls_each_enemy_and_ends_its_cast_once() {
    use crate::game::map::trinkets::{UNSTABLE_POWER, UNSTABLE_POWER_BONUS};
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (mut c, g, map) =
        kit::player(&realm, "P9EAREA", "Exploder", Race::Human, Class::Mage).await;
    let (hit, warded) = map
        .query(move |m| {
            kit::ready(m, g, 60, &[ARCANE_EXPLOSION]);
            (kit::dummy(m, g, 60, 3.0), kit::dummy(m, g, 60, 4.0))
        })
        .await
        .unwrap();
    harness::an_update(&map).await;
    c.clear();
    let r = map
        .query(move |m| {
            ward(m, warded);
            spells::cast_triggered(m, g, UNSTABLE_POWER, Some(g));
            let full = stacks(m, g, UNSTABLE_POWER_BONUS);
            let before = (kit::health(m, hit), kit::health(m, warded));
            let began = kit::cast_fresh(m, g, ARCANE_EXPLOSION, Targets::default());
            let after = (kit::health(m, hit), kit::health(m, warded));
            (
                began,
                full,
                stacks(m, g, UNSTABLE_POWER_BONUS),
                after.0 < before.0,
                after.1 == before.1,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, 12, 11, true, true));
    let go = goes(&mut c, ARCANE_EXPLOSION).await;
    assert_eq!(go.len(), 1, "one go: {go:?}");
    assert!(go[0].contains(&hit) && go[0].contains(&warded), "{go:?}");
    assert_eq!(
        misses(&mut c, ARCANE_EXPLOSION).await,
        vec![(warded.guid(), 2)]
    );
}

/// Arcane Missiles' channel isn't rolled at the enemy it is aimed at (a
/// spell of no damage class): at one warded against spells it goes on,
/// its channel object that enemy, its go naming the caster and the enemy
/// and no miss; each missile is rolled, and resisted (Thistle Tea's
/// "caster channels retain their selected enemy without a wrapper hit
/// roll"; mangos-zero's `SpellHitResult`, `SPELL_DAMAGE_CLASS_NONE`).
#[tokio::test]
#[ignore = "extended"]
async fn arcane_missiles_channel_isnt_rolled_but_its_missiles_are() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (mut c, g, map) = kit::player(&realm, "P9EAM1", "Missiler", Race::Human, Class::Mage).await;
    let foe = map
        .query(move |m| {
            kit::ready(m, g, 60, &[ARCANE_MISSILES]);
            kit::dummy(m, g, 60, 10.0)
        })
        .await
        .unwrap();
    harness::an_update(&map).await;
    c.clear();
    let r = map
        .query(move |m| {
            ward(m, foe);
            let began = kit::cast_fresh(m, g, ARCANE_MISSILES, kit::at(foe));
            let channel = m.players[&g].fields.get_guid(UNIT_FIELD_CHANNEL_OBJECT);
            let hp = kit::health(m, foe);
            kit::tick_now(m, g, ARCANE_MISSILES);
            (
                began,
                channel == foe,
                kit::casting(m, g),
                kit::health(m, foe) == hp,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, true, Some(ARCANE_MISSILES), true));
    let go = goes(&mut c, ARCANE_MISSILES).await;
    assert_eq!(go.len(), 1, "{go:?}");
    assert!(go[0].contains(&g) && go[0].contains(&foe), "{go:?}");
    assert!(
        misses(&mut c, ARCANE_MISSILES).await.is_empty(),
        "the channel isn't missed"
    );
    assert_eq!(misses(&mut c, ARCANE_MISSILE).await, vec![(foe.guid(), 2)]);
}

/// Arcane Missiles' missiles go at the enemy the channel was aimed at,
/// whatever its caster selects meanwhile; the channel cancelled, its aura
/// is gone and no missile comes after (Thistle Tea's "channel ticks retain
/// caster hit inputs and the selected target after channel cleanup" and
/// "periodic holders retain the source snapshot and stop triggering after
/// removal").
#[tokio::test]
#[ignore = "extended"]
async fn arcane_missiles_go_at_their_channels_enemy_and_stop_with_it() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, g, map) = kit::player(&realm, "P9EAM2", "Missiles", Race::Human, Class::Mage).await;
    let r = map
        .query(move |m| {
            kit::ready(m, g, 60, &[ARCANE_MISSILES]);
            let foe = kit::dummy(m, g, 60, 10.0);
            let other = kit::dummy(m, g, 60, 12.0);
            let began = kit::cast_fresh(m, g, ARCANE_MISSILES, kit::at(foe));
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_TARGET, other);
            let hp = |m: &Map| (kit::health(m, foe), kit::health(m, other));
            let before = hp(m);
            kit::tick_now(m, g, ARCANE_MISSILES);
            let ticked = hp(m);
            spells::cancel(m, g);
            let held = kit::has(m, g, ARCANE_MISSILES);
            kit::tick_now(m, g, ARCANE_MISSILES);
            kit::pass(m, 1_100);
            (
                began,
                ticked.0 < before.0,
                ticked.1 == before.1,
                held,
                hp(m) == ticked,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, true, true, false, true));
}

/// A dead victim takes no harmful trigger (Frostbite's root), where a
/// trigger whose effect is on its caster (Relentless Strikes' energy)
/// still gives it; a dead caster's gives nothing. Frostbite on the living
/// roots for its five seconds (Thistle Tea's "dead victims reject target
/// debuffs while caster-targeted refunds still resolve" and "triggered
/// Chilled snapshots Frostbite and sends its root request through the
/// caster": `tests_p8_loader_rogue::frostbite_comes_of_chilled_alone`
/// has Chilled set it off).
#[tokio::test]
#[ignore = "extended"]
async fn the_dead_take_no_harmful_trigger_but_a_casters_own_lands() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, g, map) = kit::player(&realm, "P9EDEAD", "Refunder", Race::Human, Class::Rogue).await;
    let r = map
        .query(move |m| {
            kit::ready(m, g, 60, &[]);
            let foe = kit::dummy(m, g, 60, 5.0);
            let dead = kit::dummy(m, g, 60, 6.0);
            crate::game::map::combat::deal_damage(m, g, dead, 2_000_000);
            let is_dead = m.unit_view(dead).is_some_and(|u| !u.alive);
            kit::set_power(m, g, 3, 0);
            spells::cast_triggered(m, g, RELENTLESS_STRIKES_EFFECT, Some(dead));
            let refund = kit::power(m, g, 3);
            spells::cast_triggered(m, g, FROSTBITE, Some(dead));
            let dead_rooted = kit::has(m, dead, FROSTBITE);
            spells::cast_triggered(m, g, FROSTBITE, Some(foe));
            let rooted = auras::auras(m, foe).is_some_and(|a| a.has(auras::AURA_MOD_ROOT));
            kit::pass(m, 4_900);
            let still = kit::has(m, foe, FROSTBITE);
            kit::pass(m, 200);
            let gone = !kit::has(m, foe, FROSTBITE);
            crate::game::map::death::kill(m, g);
            kit::set_power(m, g, 3, 0);
            spells::cast_triggered(m, g, RELENTLESS_STRIKES_EFFECT, Some(foe));
            let from_the_dead = kit::power(m, g, 3);
            (
                is_dead,
                refund,
                dead_rooted,
                rooted,
                still,
                gone,
                from_the_dead,
            )
        })
        .await
        .unwrap();
    assert_eq!(r, (true, 25, false, true, true, true, 0));
}

/// A hunter's Frost Trap, sprung, makes its aura's two areas as its hunter,
/// ten yards across each, ticking: one an effect (Thistle Tea's "owned
/// Frost Trap creates ticking areas with complete caster snapshots";
/// mangos's persistent area aura of each effect).
#[tokio::test]
#[ignore = "extended"]
async fn a_frost_trap_sprung_makes_its_two_areas_as_its_hunter() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_c, h, map) = kit::player(&realm, "P9ETRAP", "Trapper", Race::Dwarf, Class::Hunter).await;
    let r = map
        .query(move |m| {
            kit::ready(m, h, 60, &[FROST_TRAP]);
            let laid = kit::cast_fresh(m, h, FROST_TRAP, Targets::default());
            let trap = m
                .objects
                .traps
                .iter()
                .find(|(_, t)| t.owner == Some(h))
                .map(|(g, _)| *g)
                .expect("the trap");
            let foe = kit::dummy(m, h, 60, 2.0);
            let now = m.now;
            m.objects.traps.get_mut(&trap).unwrap().ready_at = now;
            crate::game::map::objects::used_by_script(m, foe, trap);
            let mut areas: Vec<(Guid, usize, f32)> = m
                .spells
                .areas
                .iter()
                .filter(|a| a.spell == FROST_TRAP_AURA)
                .map(|a| (a.caster, a.index, a.radius))
                .collect();
            areas.sort_by_key(|a| a.1);
            let objects = m
                .spells
                .dynamic
                .values()
                .filter(|d| d.spell == FROST_TRAP_AURA && d.caster == h)
                .count();
            (laid, areas, objects)
        })
        .await
        .unwrap();
    assert!(r.0, "laid");
    assert_eq!(r.1, vec![(h, 0, 10.0), (h, 1, 10.0)]);
    assert_eq!(r.2, 2, "an object for each area");
}

/// A player standing still under water whose Water Breathing runs out
/// starts holding their breath at the next look, no move of theirs needed:
/// a minute's breath, told to the client (Thistle Tea's "samples liquid
/// for a stationary protected player before aura expiration").
#[tokio::test]
#[ignore = "extended"]
async fn a_still_player_under_water_holds_their_breath_once_water_breathing_ends() {
    use crate::game::data::spells::SpellDuration;
    const BREATHING: u32 = 990_991;
    let terrain = harness::real_data("terrain");
    if crate::game::map::terrain::Terrain::open(&terrain, 0).is_none() {
        return;
    }
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.durations.insert(
        9,
        SpellDuration {
            base: 1_000,
            ..Default::default()
        },
    );
    let mut s = kit::made_up(BREATHING, 6, auras::AURA_WATER_BREATHING, 0, 1);
    s.duration_index = 9;
    d.spells.spells.insert(BREATHING, s);
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(terrain);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let mut c = Client::connect(&realm, "P9EBREATH");
    let g = c.enter("Diver", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    let look = move |m: &mut Map| {
        if let Some(p) = m.players.get_mut(&g) {
            p.misc.env_checked_at = None;
        }
        crate::game::map::environment::update(m);
        m.players[&g].misc.breath.map(|b| b.remaining)
    };
    let r = map
        .query(move |m| {
            let p = m.players.get_mut(&g).unwrap();
            p.movement.position = Vector3d {
                x: -9500.0,
                y: -220.0,
                z: 55.47,
            };
            let s = m.realm.data.spells.get(BREATHING).unwrap().clone();
            kit::apply_template(m, g, g, &s);
            let protected = look(m);
            m.now += Duration::from_millis(1_100);
            auras::update(m);
            let lapsed = kit::has(m, g, BREATHING);
            let held = look(m);
            (protected, lapsed, held, m.players[&g].position())
        })
        .await
        .unwrap();
    assert_eq!(r.0, None, "no breath held under Water Breathing");
    assert!(!r.1, "its time is up");
    assert_eq!(r.2, Some(Duration::from_secs(60)), "a minute's breath");
    assert!((r.3.z - 55.47).abs() < 0.01, "standing still: {:?}", r.3);
    let told = c
        .wait_for("SMSG_START_MIRROR_TIMER", |m| match m {
            ServerOpcodeMessage::SMSG_START_MIRROR_TIMER(t) if t.timer == TimerType::Breath => {
                Some(t.duration)
            }
            _ => None,
        })
        .await;
    assert_eq!(told, 60_000);
}

/// A script's move to a random point about a place (step 3, datalong 3)
/// goes to a point of the navmesh within its radius, on the ground about
/// the place: in the Barrens, within five yards of the step's point and
/// between 90 and 98 high, each time (Thistle Tea's "prepares random
/// movement regions for EventAI and explicit script continuations": an
/// EventAI action's step and a script's are the same step in wvs).
#[tokio::test]
#[ignore = "extended"]
async fn a_scripts_random_move_lands_on_the_ground_within_its_radius() {
    const ROAMER: u32 = 3799;
    let bake = harness::real_data("nav");
    if !bake.join("Kalimdor.map").exists() {
        return;
    }
    let anchor = Vector3d {
        x: -288.089,
        y: -1874.42,
        z: 92.743,
    };
    let mut d = crate::game::tests_p3_combat::data();
    d.creature_templates
        .insert(ROAMER, crate::game::tests_p3_combat::template(ROAMER, 1));
    d.creature_spawns.insert(
        1,
        vec![crate::game::data::CreatureSpawn {
            guid: 79_390,
            entries: vec![ROAMER],
            map: 1,
            x: anchor.x,
            y: anchor.y,
            z: anchor.z,
            respawn_min: 300,
            respawn_max: 300,
            ..Default::default()
        }],
    );
    let mut cfg = harness::test_config();
    cfg.nav_dir = Some(bake);
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = crate::game::Realm::new(cfg, store, startup, d, None);
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(1));
    let points = map
        .query(move |m| {
            let g = scripts::creature_of_spawn(m, 79_390).expect("the roamer");
            let step = crate::game::data::scripts::ScriptStep {
                command: 3,
                datalong: [3, 0, 0, 0],
                x: anchor.x,
                y: anchor.y,
                z: anchor.z,
                o: 5.0,
                ..Default::default()
            };
            let mut points = Vec::new();
            for _ in 0..20 {
                scripts::place(m, g, anchor, 0.0);
                scripts::execute(m, &step, g, g);
                if let crate::game::map::creature::Motion::Moving(w) =
                    &m.creature(g).unwrap().motion
                {
                    points.push(w.destination());
                }
            }
            (m.nav.is_some(), points)
        })
        .await
        .unwrap();
    let (navigated, points) = points;
    assert!(navigated, "the map has its navmesh");
    assert!(points.len() >= 15, "it moves: {points:?}");
    for p in &points {
        let flat = ((p.x - anchor.x).powi(2) + (p.y - anchor.y).powi(2)).sqrt();
        assert!(flat <= 5.0 + 0.01, "within five yards: {p:?}");
        assert!(p.z > 90.0 && p.z < 98.0, "on the ground: {p:?}");
    }
}
