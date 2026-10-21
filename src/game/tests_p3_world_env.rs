//! Phase 3's environment against Thistle Tea's tests: the breath's
//! arithmetic (its recovery, a late check, a longer or shorter breath),
//! drowning, who the water, the deep sea and lava spare, Water Breathing,
//! and the world's damage: resisted at the victim's own level, absorbed by
//! the right shields, past them for a fall, drowning and exhaustion, never
//! to the immune, breaking stealth and sitting, and by a spell with no
//! bonus.

use std::sync::Arc;
use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras::*;
use crate::game::map::environment::{self, BREATH};
use crate::game::map::{Map, MapKey};

/// An aura spell of `kind` and `amount` on oneself, of `misc`, for
/// `duration_index` (0: for good).
fn aura_spell(d: &mut GameData, id: u32, kind: u32, amount: i32, misc: i32) {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        base_points: amount - 1,
        base_dice: 1,
        misc_value: misc,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        id,
        SpellTemplate {
            id,
            name: format!("Aura {id}"),
            effects,
            equipped_item_class: -1,
            ..Default::default()
        },
    );
}

const WATER_BREATHING: u32 = 90_082;
const LONG_BREATH: u32 = 90_155;
const PHYSICAL_IMMUNITY: u32 = 90_039;
const FIRE_IMMUNITY: u32 = 90_040;
const FIRE_SCHOOL_IMMUNITY: u32 = 90_041;
const FIRE_WARD: u32 = 90_069;
const PHYSICAL_WARD: u32 = 90_070;
const NATURE_WARD: u32 = 90_071;
const EVERY_WARD: u32 = 90_072;
const MANA_SHIELD: u32 = 90_097;
const STEALTH: u32 = 90_016;
const FEIGN: u32 = 90_066;
const UNTOUCHED: u32 = 90_087;
const FIRE_PIERCING: u32 = 90_123;
const CAMPFIRE: u32 = 90_007;

fn world() -> GameData {
    let mut d = GameData::empty();
    aura_spell(&mut d, WATER_BREATHING, AURA_WATER_BREATHING, 1, 0);
    aura_spell(&mut d, LONG_BREATH, AURA_MOD_WATER_BREATHING, 300, 0);
    aura_spell(&mut d, PHYSICAL_IMMUNITY, AURA_SCHOOL_IMMUNITY, 1, 1);
    aura_spell(&mut d, FIRE_IMMUNITY, AURA_DAMAGE_IMMUNITY, 1, 4);
    aura_spell(&mut d, FIRE_SCHOOL_IMMUNITY, AURA_SCHOOL_IMMUNITY, 1, 4);
    aura_spell(&mut d, FIRE_WARD, AURA_SCHOOL_ABSORB, 150, 4);
    aura_spell(&mut d, PHYSICAL_WARD, AURA_SCHOOL_ABSORB, 100, 1);
    aura_spell(&mut d, NATURE_WARD, AURA_SCHOOL_ABSORB, 100, 8);
    aura_spell(&mut d, EVERY_WARD, AURA_SCHOOL_ABSORB, 500, 127);
    aura_spell(&mut d, MANA_SHIELD, AURA_MANA_SHIELD, 100, 4);
    d.spells.spells.get_mut(&MANA_SHIELD).unwrap().effects[0].multiple_value = 2.0;
    aura_spell(&mut d, STEALTH, AURA_MOD_STEALTH, 1, 0);
    aura_spell(&mut d, FEIGN, AURA_FEIGN_DEATH, 1, 0);
    d.spells
        .spells
        .get_mut(&FEIGN)
        .unwrap()
        .aura_interrupt_flags = 15_420;
    aura_spell(&mut d, UNTOUCHED, AURA_MOD_DAMAGE_PERCENT_TAKEN, -100, 4);
    aura_spell(&mut d, FIRE_PIERCING, AURA_MOD_TARGET_RESISTANCE, -300, 4);
    aura_spell(
        &mut d,
        crate::game::map::spells::redemption::FORM,
        AURA_DUMMY,
        1,
        0,
    );
    // A campfire's: a hundred fire to those by it.
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: crate::game::map::spells::EFFECT_ENVIRONMENTAL_DAMAGE,
        base_points: 99,
        base_dice: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(
        CAMPFIRE,
        SpellTemplate {
            id: CAMPFIRE,
            name: "Fire".into(),
            school: 2,
            effects,
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    d
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

async fn alice(realm: &Arc<Realm>, class: Class) -> (Client, Guid) {
    let mut a = Client::connect(realm, "A");
    let g = a.enter("Alice", Race::Human, class).await;
    (a, g)
}

fn health(m: &Map, g: Guid) -> u32 {
    m.players[&g].fields.get_u32(UNIT_FIELD_HEALTH)
}

/// `g` at 1,000 health of 1,000, at sixty.
fn hardy(m: &mut Map, g: Guid) {
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
    f.set_u32(UNIT_FIELD_HEALTH, 1000);
    f.set_u32(UNIT_FIELD_LEVEL, 60);
}

fn put_on(m: &mut Map, g: Guid, spell: u32) {
    crate::game::map::spells::cast_triggered(m, g, spell, Some(g));
}

fn breath(m: &Map, g: Guid) -> Option<environment::Mirror> {
    m.players[&g].misc.breath
}

#[tokio::test]
async fn breath_comes_back_ten_times_as_fast_keeps_its_reserve_and_a_late_check_drowns_once() {
    let realm = harness::realm_with(world());
    let (mut a, g) = alice(&realm, Class::Warrior).await;
    a.clear();
    let (surfaced, redived, gone, late) = on_map(&realm, move |m| {
        hardy(m, g);
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        environment::breathe(m, g, true, BREATH, true, t0);
        environment::breathe(m, g, true, BREATH, true, at(30));
        // Up for air, in the water still: thirty seconds left, filling.
        environment::breathe(m, g, false, BREATH, true, at(30));
        let surfaced = breath(m, g).map(|b| (b.remaining, b.draining));
        // A second up is ten under: forty seconds when she dives again.
        environment::breathe(m, g, false, BREATH, true, at(31));
        environment::breathe(m, g, true, BREATH, true, at(31));
        let redived = breath(m, g).map(|b| (b.remaining, b.draining));
        // Two seconds up fill it, and it's gone.
        environment::breathe(m, g, false, BREATH, true, at(31));
        environment::breathe(m, g, false, BREATH, true, at(33));
        let gone = breath(m, g).is_none();
        // Under two minutes at one check: one pulse, the next two seconds
        // on.
        let t1 = at(100);
        environment::breathe(m, g, true, BREATH, true, t1);
        environment::breathe(m, g, true, BREATH, true, t1 + Duration::from_secs(120));
        let late = (
            health(m, g),
            breath(m, g).and_then(|b| b.next_damage.map(|t| t - t1)),
        );
        (surfaced, redived, gone, late)
    })
    .await;
    assert_eq!(surfaced, Some((Duration::from_secs(30), false)));
    assert_eq!(redived, Some((Duration::from_secs(40), true)));
    assert!(gone, "a full breath goes");
    // A fifth of 1,000, and up to the level more, once.
    assert!((741..=800).contains(&late.0), "{}", late.0);
    assert_eq!(late.1, Some(Duration::from_secs(122)));
    // The client was told the filling's scale (10), then the timer's end.
    a.wait_for("the filling", |m| match m {
        ServerOpcodeMessage::SMSG_START_MIRROR_TIMER(t) => {
            (t.timer == TimerType::Breath && t.scale == 10 && t.time_remaining == 30_000)
                .then_some(())
        }
        _ => None,
    })
    .await;
    a.wait_for("the end", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_STOP_MIRROR_TIMER(t) if t.timer == TimerType::Breath)
            .then_some(())
    })
    .await;
}

#[tokio::test]
async fn a_longer_breath_or_its_end_keeps_the_time_already_spent_under() {
    let realm = harness::realm_with(world());
    let (mut a, g) = alice(&realm, Class::Warrior).await;
    a.clear();
    let (extended, ended, lengthened) = on_map(&realm, move |m| {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // Four minutes' breath; ten seconds in, its aura ends: a minute,
        // less the ten.
        environment::breathe(m, g, true, BREATH * 4, true, t0);
        let extended = breath(m, g).map(|b| b.duration);
        environment::breathe(m, g, true, BREATH, true, at(10));
        let ended = breath(m, g).map(|b| (b.duration, b.remaining));
        // A minute's, ten seconds in, made four: the ten kept.
        m.players.get_mut(&g).unwrap().misc.breath = None;
        environment::breathe(m, g, true, BREATH, true, at(20));
        environment::breathe(m, g, true, BREATH * 4, true, at(30));
        let lengthened = breath(m, g).map(|b| (b.duration, b.remaining));
        (extended, ended, lengthened)
    })
    .await;
    assert_eq!(extended, Some(Duration::from_secs(240)));
    assert_eq!(
        ended,
        Some((Duration::from_secs(60), Duration::from_secs(50)))
    );
    assert_eq!(
        lengthened,
        Some((Duration::from_secs(240), Duration::from_secs(230)))
    );
    // The client is told the minute's timer when the aura ends.
    a.wait_for("the shorter timer", |m| match m {
        ServerOpcodeMessage::SMSG_START_MIRROR_TIMER(t) => {
            (t.duration == 60_000 && t.time_remaining == 50_000).then_some(())
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn drowning_kills_once_and_the_breath_stops_and_the_immune_dont_drown() {
    let realm = harness::realm_with(world());
    let (mut a, g) = alice(&realm, Class::Warrior).await;
    // Immune to the physical: no harm, nothing logged.
    a.clear();
    let spared = on_map(&realm, move |m| {
        hardy(m, g);
        put_on(m, g, PHYSICAL_IMMUNITY);
        let t0 = Instant::now();
        environment::breathe(m, g, true, BREATH, true, t0);
        environment::breathe(m, g, true, BREATH, true, t0 + BREATH);
        remove_spell(m, g, PHYSICAL_IMMUNITY, None);
        m.players.get_mut(&g).unwrap().misc.breath = None;
        health(m, g)
    })
    .await;
    assert_eq!(spared, 1000);
    let logged = a
        .gets(Duration::from_millis(200), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(_)).then_some(())
        })
        .await;
    assert!(logged.is_none(), "no log for the immune");
    // At a hundred health, the first pulse drowns her; the breath stops.
    let dead = on_map(&realm, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 100);
        let t0 = Instant::now();
        environment::breathe(m, g, true, BREATH, true, t0);
        environment::breathe(m, g, true, BREATH, true, t0 + BREATH);
        m.players[&g].death
    })
    .await;
    assert!(
        matches!(dead, crate::game::map::death::Death::Dead { .. }),
        "{dead:?}"
    );
    a.wait_for("the drowning", |m| match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => {
            (d.damage_type == EnvironmentalDamageType::Drowning).then_some(())
        }
        _ => None,
    })
    .await;
    a.wait_for("the breath stopped", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_STOP_MIRROR_TIMER(t) if t.timer == TimerType::Breath)
            .then_some(())
    })
    .await;
    assert!(on_map(&realm, move |m| breath(m, g).is_none()).await);
}

// The real terrain's water and lava.

static TERRAIN: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("terrain"));
/// Out at sea west of Stranglethorn, deep; nearer the coast, ocean but not
/// deep, five yards under; Searing Gorge's lava; Goldshire's dry road.
const HIGH_SEA: (f32, f32, f32) = (-15000.0, 1024.0, -5.0);
const SHALLOWS: (f32, f32, f32) = (-15000.0, 64.0, -5.0);
const SEARING_GORGE: (f32, f32, f32) = (-6488.0, -976.0, 334.0);
const GOLDSHIRE: (f32, f32, f32) = (-9460.0, 62.0, 56.0);

fn terrain_realm() -> Option<Arc<Realm>> {
    crate::game::map::terrain::Terrain::open(TERRAIN.as_path(), 0)?;
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(TERRAIN.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Some(Realm::new(cfg, store, startup, world(), None))
}

fn place(m: &mut Map, g: Guid, (x, y, z): (f32, f32, f32)) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position = Vector3d { x, y, z };
    p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
    p.fields.set_u32(UNIT_FIELD_HEALTH, 100_000);
}

/// The environment's check, as the update makes it, `after` on.
fn check(m: &mut Map, g: Guid, after: Duration) {
    m.now += after;
    if let Some(p) = m.players.get_mut(&g) {
        p.misc.env_checked_at = None;
    }
    environment::update(m);
}

/// A change to a player, on the map.
type Change = fn(&mut Map, Guid);

/// The ways a player is spared, and their undoing.
const SPARED: [(&str, Change, Change); 5] = [
    (
        "dead",
        |m, g| crate::game::map::death::kill(m, g),
        |m, g| {
            crate::game::map::death::resurrect(m, g, 1.0);
        },
    ),
    (
        "a ghost",
        |m, g| {
            crate::game::map::death::kill(m, g);
            crate::game::map::death::release(m, g);
        },
        |m, g| {
            crate::game::map::death::resurrect(m, g, 1.0);
        },
    ),
    (
        "a god",
        |m, g| m.players.get_mut(&g).unwrap().misc.god = true,
        |m, g| m.players.get_mut(&g).unwrap().misc.god = false,
    ),
    (
        "a spirit",
        |m, g| put_on(m, g, crate::game::map::spells::redemption::FORM),
        |m, g| remove_spell(m, g, crate::game::map::spells::redemption::FORM, None),
    ),
    (
        "flying",
        |m, g| {
            let pos = m.players[&g].position();
            let far = Vector3d {
                x: pos.x + 1000.0,
                ..pos
            };
            m.players.get_mut(&g).unwrap().flight = Some(crate::game::map::npc::Flight {
                walk: crate::game::map::creature::Walk::new(
                    vec![pos, far],
                    1.0,
                    Instant::now(),
                    0,
                    true,
                ),
                destination: 0,
                mount: 0,
            });
        },
        |m, g| m.players.get_mut(&g).unwrap().flight = None,
    ),
];

#[tokio::test]
#[ignore = "extended"]
async fn the_dead_a_ghost_a_god_a_spirit_and_a_flyer_neither_drown_nor_tire_nor_burn() {
    let Some(realm) = terrain_realm() else {
        return;
    };
    let (_a, g) = alice(&realm, Class::Warrior).await;
    for (what, spare, undo) in SPARED {
        let (breath, fatigue, burnt) = on_map(&realm, move |m| {
            // Under the shallows, five yards: the breath runs; then spared.
            place(m, g, SHALLOWS);
            check(m, g, Duration::ZERO);
            assert!(breath(m, g).is_some(), "{what}: under, a breath");
            spare(m, g);
            check(m, g, Duration::from_secs(1));
            let no_breath = breath(m, g).is_none();
            undo(m, g);
            // The deep sea: tired; then spared, a minute and more.
            place(m, g, HIGH_SEA);
            check(m, g, Duration::ZERO);
            assert!(
                m.players[&g].misc.fatigue.is_some(),
                "{what}: the deep sea tires"
            );
            spare(m, g);
            check(m, g, Duration::from_secs(61));
            let ghost = matches!(
                m.players[&g].death,
                crate::game::map::death::Death::Ghost { .. }
            );
            // A ghost tires still (and is sent to a graveyard); the others
            // don't.
            let no_fatigue = ghost || m.players[&g].misc.fatigue.is_none();
            undo(m, g);
            m.players.get_mut(&g).unwrap().misc.fatigue = None;
            m.players.get_mut(&g).unwrap().misc.breath = None;
            // Lava: spared, never burnt.
            place(m, g, SEARING_GORGE);
            spare(m, g);
            let spared_at = health(m, g);
            check(m, g, Duration::ZERO);
            check(m, g, Duration::from_secs(2));
            check(m, g, Duration::from_secs(2));
            let burnt = health(m, g) < spared_at;
            undo(m, g);
            m.players.get_mut(&g).unwrap().misc.lava = None;
            place(m, g, GOLDSHIRE);
            check(m, g, Duration::from_secs(1));
            (no_breath, no_fatigue, burnt)
        })
        .await;
        assert!(breath, "{what}: the breath stops");
        assert!(fatigue, "{what}: the fatigue stops");
        assert!(!burnt, "{what}: no burn");
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn water_breathing_refills_the_breath_under_water_and_its_end_drowns_again() {
    let Some(realm) = terrain_realm() else {
        return;
    };
    let (_a, g) = alice(&realm, Class::Warrior).await;
    let (refilled, hurt_after) = on_map(&realm, move |m| {
        place(m, g, SHALLOWS);
        check(m, g, Duration::ZERO);
        check(m, g, Duration::from_secs(59));
        // A second left, Water Breathing: it fills ten times as fast.
        put_on(m, g, WATER_BREATHING);
        check(m, g, Duration::ZERO);
        check(m, g, Duration::from_secs(1));
        let refilled = breath(m, g).map(|b| (b.remaining, b.draining));
        // It ends: eleven seconds, and drowning again.
        remove_spell(m, g, WATER_BREATHING, None);
        check(m, g, Duration::ZERO);
        check(m, g, Duration::from_millis(11_100));
        (refilled, health(m, g))
    })
    .await;
    assert_eq!(refilled, Some((Duration::from_secs(11), false)));
    assert!(hurt_after < 100_000, "drowning again");
    // Water Breathing from the first: no breath timer at all, and none on
    // fatigue in the deep sea either.
    let (no_breath, tired) = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().misc.breath = None;
        place(m, g, HIGH_SEA);
        put_on(m, g, WATER_BREATHING);
        check(m, g, Duration::ZERO);
        let no_breath = breath(m, g).is_none();
        check(m, g, Duration::from_secs(61));
        (no_breath, health(m, g) < 100_000)
    })
    .await;
    assert!(no_breath);
    assert!(tired, "Water Breathing is no help in the deep sea");
}

// The world's damage.

fn log(
    c: &mut Client,
) -> impl std::future::Future<Output = (EnvironmentalDamageType, u32, u32, u32)> + '_ {
    c.wait_for("the log", |m| match m {
        ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(d) => {
            Some((d.damage_type, d.damage, d.absorb, d.resist))
        }
        _ => None,
    })
}

#[tokio::test]
async fn the_worlds_fire_is_resisted_at_the_victims_own_level_then_absorbed_and_gives_no_rage() {
    let realm = harness::realm_with(world());
    let (mut a, g) = alice(&realm, Class::Warrior).await;
    a.clear();
    // A hundred fire resistance: a quarter's chance at sixty, the cap at
    // twenty; a middling roll resists a quarter, then three.
    let by_level = on_map(&realm, move |m| {
        hardy(m, g);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_RESISTANCES + 2, 100);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 5_000);
        let at_sixty = 1000 - health(m, g);
        hardy(m, g);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_LEVEL, 20);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 5_000);
        let at_twenty = 1000 - health(m, g);
        // Her own penetration takes her resistance off.
        hardy(m, g);
        put_on(m, g, FIRE_PIERCING);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 0);
        let pierced = 1000 - health(m, g);
        remove_spell(m, g, FIRE_PIERCING, None);
        (at_sixty, at_twenty, pierced)
    })
    .await;
    assert_eq!(by_level, (75, 25, 100));
    // Three hundred: three quarters of 200 resisted, then 30 of a ward
    // absorbed; 20 taken, as logged. No rage, no fight.
    a.clear();
    let (taken, rage, fighting) = on_map(&realm, move |m| {
        hardy(m, g);
        put_on(m, g, FIRE_WARD);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_RESISTANCES + 2, 300);
        let a = m.players.get_mut(&g).unwrap();
        for h in a.auras.holders.iter_mut().filter(|h| h.spell == FIRE_WARD) {
            for x in h.effects.iter_mut() {
                x.left = 30;
            }
        }
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 200, 0);
        let p = &m.players[&g];
        (
            1000 - health(m, g),
            p.fields.get_u32(UNIT_FIELD_POWER1 + 1),
            p.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            ),
        )
    })
    .await;
    assert_eq!(taken, 20);
    assert_eq!(
        log(&mut a).await,
        (EnvironmentalDamageType::Fire, 20, 30, 150)
    );
    assert_eq!((rage, fighting), (0, false));
}

#[tokio::test]
async fn shields_soak_the_worlds_harm_by_their_school_and_a_fall_drowning_and_exhaustion_go_past() {
    let realm = harness::realm_with(world());
    let (mut a, g) = alice(&realm, Class::Mage).await;
    a.clear();
    // A 150 fire ward: a hundred soaked, then fifty and it's gone.
    let pulses = on_map(&realm, move |m| {
        hardy(m, g);
        put_on(m, g, FIRE_WARD);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 9_999);
        let first = health(m, g);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 9_999);
        (
            first,
            health(m, g),
            m.players[&g].auras.has_spell(FIRE_WARD),
        )
    })
    .await;
    assert_eq!(pulses, (1000, 950, false));
    assert_eq!(
        log(&mut a).await,
        (EnvironmentalDamageType::Fire, 0, 100, 0)
    );
    assert_eq!(
        log(&mut a).await,
        (EnvironmentalDamageType::Fire, 50, 50, 0)
    );
    // A physical ward doesn't soak fire; Mana Shield does, two mana a
    // point.
    let (physical, mana_shielded, mana) = on_map(&realm, move |m| {
        hardy(m, g);
        put_on(m, g, PHYSICAL_WARD);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 9_999);
        let physical = (health(m, g), m.players[&g].auras.has_spell(PHYSICAL_WARD));
        remove_spell(m, g, PHYSICAL_WARD, None);
        hardy(m, g);
        let f = &mut m.players.get_mut(&g).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXPOWER1, 100);
        f.set_u32(UNIT_FIELD_POWER1, 100);
        put_on(m, g, MANA_SHIELD);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 9_999);
        (
            physical,
            health(m, g),
            m.players[&g].fields.get_u32(UNIT_FIELD_POWER1),
        )
    })
    .await;
    assert_eq!(physical, (900, true));
    assert_eq!((mana_shielded, mana), (950, 0));
    // Lava is fire's; slime nature's.
    let (lava, slime_past_fire, slime) = on_map(&realm, move |m| {
        hardy(m, g);
        put_on(m, g, FIRE_WARD);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Lava, 100, 9_999);
        let lava = health(m, g);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Slime, 100, 9_999);
        let slime_past_fire = health(m, g);
        remove_spell(m, g, FIRE_WARD, None);
        hardy(m, g);
        put_on(m, g, NATURE_WARD);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Slime, 100, 9_999);
        (lava, slime_past_fire, health(m, g))
    })
    .await;
    assert_eq!((lava, slime_past_fire, slime), (1000, 900, 1000));
    // A fall, drowning and exhaustion: past a ward of every school and any
    // resistance, the ward untouched.
    a.clear();
    let physical = on_map(&realm, move |m| {
        hardy(m, g);
        put_on(m, g, EVERY_WARD);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_RESISTANCES, 300);
        let mut out = Vec::new();
        for kind in [
            EnvironmentalDamageType::Fall,
            EnvironmentalDamageType::Drowning,
            EnvironmentalDamageType::Exhausted,
        ] {
            hardy(m, g);
            environment::damage_rolled(m, g, kind, 100, 0);
            out.push(health(m, g));
        }
        (out, m.players[&g].auras.has_spell(EVERY_WARD))
    })
    .await;
    assert_eq!(physical, (vec![900, 900, 900], true));
    for kind in [
        EnvironmentalDamageType::Fall,
        EnvironmentalDamageType::Drowning,
        EnvironmentalDamageType::Exhausted,
    ] {
        assert_eq!(log(&mut a).await, (kind, 100, 0, 0));
    }
    // Damage taken modifiers don't touch it.
    let untouched = on_map(&realm, move |m| {
        hardy(m, g);
        remove_spell(m, g, EVERY_WARD, None);
        put_on(m, g, UNTOUCHED);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 9_999);
        health(m, g)
    })
    .await;
    assert_eq!(untouched, 900);
}

#[tokio::test]
async fn the_immune_the_dead_a_ghost_and_a_god_take_none_of_it_and_hear_nothing() {
    let realm = harness::realm_with(world());
    let (mut a, g) = alice(&realm, Class::Warrior).await;
    type Way = (fn(&mut Map, Guid), fn(&mut Map, Guid));
    let ways: [Way; 5] = [
        (
            |m, g| put_on(m, g, FIRE_IMMUNITY),
            |m, g| remove_spell(m, g, FIRE_IMMUNITY, None),
        ),
        (
            |m, g| put_on(m, g, FIRE_SCHOOL_IMMUNITY),
            |m, g| remove_spell(m, g, FIRE_SCHOOL_IMMUNITY, None),
        ),
        (
            |m, g| crate::game::map::death::kill(m, g),
            |m, g| {
                crate::game::map::death::resurrect(m, g, 1.0);
            },
        ),
        (
            |m, g| {
                crate::game::map::death::kill(m, g);
                crate::game::map::death::release(m, g);
            },
            |m, g| {
                crate::game::map::death::resurrect(m, g, 1.0);
            },
        ),
        (
            |m, g| m.players.get_mut(&g).unwrap().misc.god = true,
            |m, g| m.players.get_mut(&g).unwrap().misc.god = false,
        ),
    ];
    for (i, (spare, undo)) in ways.into_iter().enumerate() {
        a.clear();
        let before_after = on_map(&realm, move |m| {
            hardy(m, g);
            spare(m, g);
            let before = health(m, g);
            environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 9_999);
            let after = health(m, g);
            undo(m, g);
            (before, after)
        })
        .await;
        assert_eq!(before_after.0, before_after.1, "way {i}");
        let logged = a
            .gets(Duration::from_millis(150), |m| {
                matches!(m, ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(_)).then_some(())
            })
            .await;
        assert!(logged.is_none(), "way {i}: nothing logged");
    }
}

#[tokio::test]
async fn fire_all_soaked_still_breaks_stealth_and_sitting_but_not_feign_death() {
    let realm = harness::realm_with(world());
    let (mut a, g) = alice(&realm, Class::Rogue).await;
    let (stealthed, feigned, stand, hp) = on_map(&realm, move |m| {
        hardy(m, g);
        put_on(m, g, FIRE_WARD);
        put_on(m, g, STEALTH);
        put_on(m, g, FEIGN);
        m.players.get_mut(&g).unwrap().set_stand_state(1);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 100, 9_999);
        let p = &m.players[&g];
        (
            p.auras.has_spell(STEALTH),
            p.auras.has_spell(FEIGN),
            p.stand_state(),
            health(m, g),
        )
    })
    .await;
    assert_eq!((stealthed, feigned, stand, hp), (false, true, 0, 1000));
    a.wait_for("she stands", |m| match m {
        ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(s) => {
            (s.state == UnitStandState::Stand).then_some(())
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn a_lethal_burn_kills_once_and_wears_no_gear() {
    let realm = harness::realm_with(world());
    let (mut a, g) = alice(&realm, Class::Warrior).await;
    a.clear();
    let (dead, again) = on_map(&realm, move |m| {
        hardy(m, g);
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 1100, 9_999);
        let dead = m.players[&g].death;
        environment::damage_rolled(m, g, EnvironmentalDamageType::Fire, 1100, 9_999);
        (dead, health(m, g))
    })
    .await;
    assert!(matches!(dead, crate::game::map::death::Death::Dead { .. }));
    assert_eq!(again, 0);
    // The world's fire wears nothing at death: mangos's
    // `EnvironmentalDamage` deals it as her own harm (question #1).
    let (mut logs, mut worn) = (0, 0);
    while let Some(m) = a
        .gets(Duration::from_millis(100), |m| match m {
            ServerOpcodeMessage::SMSG_ENVIRONMENTAL_DAMAGE_LOG(_) => Some(true),
            ServerOpcodeMessage::SMSG_DURABILITY_DAMAGE_DEATH => Some(false),
            _ => None,
        })
        .await
    {
        if m {
            logs += 1;
        } else {
            worn += 1;
        }
    }
    assert_eq!(logs, 1, "the second burn finds her dead");
    assert_eq!(worn, 0, "no wear");
}

#[tokio::test]
async fn a_campfires_spell_burns_a_player_as_the_world_does_and_only_soaks_a_creatures_shield() {
    let mut d = world();
    super::tests_pvp2::npc(&mut d, 90_500, "Kobold", 14);
    let realm = harness::realm_with(d);
    let (mut a, g) = alice(&realm, Class::Warrior).await;
    a.clear();
    // A creature's campfire spell at Alice, with spell power to spare: a
    // hundred, no more, as the world's fire; no fight.
    let (taken, fighting, kobold_health, ward) = on_map(&realm, move |m| {
        hardy(m, g);
        let pos = m.players[&g].position();
        let spawn = crate::game::data::CreatureSpawn {
            guid: 0x7D_0000,
            entries: vec![90_500],
            map: 0,
            x: pos.x + 2.0,
            y: pos.y,
            z: pos.z,
            ..Default::default()
        };
        let data = m.realm.data.clone();
        let c = crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())
            .unwrap();
        let kobold = m.add_creature(c);
        crate::game::map::spells::cast_triggered(m, kobold, CAMPFIRE, Some(g));
        let taken = 1000 - health(m, g);
        let fighting = m.players[&g].fields.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        );
        // At the kobold, warded: the ward soaks it, and it loses nothing.
        put_on(m, kobold, FIRE_WARD);
        let before = m
            .creature(kobold)
            .unwrap()
            .fields
            .get_u32(UNIT_FIELD_HEALTH);
        crate::game::map::spells::cast_triggered(m, g, CAMPFIRE, Some(kobold));
        let after = m
            .creature(kobold)
            .unwrap()
            .fields
            .get_u32(UNIT_FIELD_HEALTH);
        let ward = crate::game::map::auras::auras(m, kobold).and_then(|a| {
            a.holders
                .iter()
                .find(|h| h.spell == FIRE_WARD)
                .map(|h| h.effects[0].left)
        });
        (taken, fighting, (before, after), ward)
    })
    .await;
    assert_eq!((taken, fighting), (100, false));
    assert_eq!(
        log(&mut a).await,
        (EnvironmentalDamageType::Fire, 100, 0, 0)
    );
    assert_eq!(
        kobold_health.0, kobold_health.1,
        "a creature loses no health"
    );
    assert_eq!(ward, Some(50), "its ward soaked a hundred");
}

#[test]
#[ignore = "extended"]
fn the_real_models_heights_put_a_humans_head_two_yards_up_and_a_gnomes_one() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let height = |display: u32| d.display_info[&display].height;
    assert!(
        (height(49) - 2.031).abs() < 0.01,
        "a human man: {}",
        height(49)
    );
    assert!((height(51) - 2.361).abs() < 0.01, "an orc: {}", height(51));
    assert!(
        (height(1563) - 1.056).abs() < 0.01,
        "a gnome: {}",
        height(1563)
    );
}

#[tokio::test]
#[ignore = "extended"]
async fn the_waterline_is_the_models_height_at_its_scale() {
    let Some(_) = terrain_realm() else {
        return;
    };
    // Alice (a human woman, display 50) a yard and a half under: her head
    // is under at a yard's height, not at two, nor at one made two by her
    // scale.
    let mut d = world();
    d.display_info.insert(
        50,
        crate::game::data::DisplayInfo {
            scale: 1.0,
            height: 1.0,
            ..Default::default()
        },
    );
    let mut cfg = harness::test_config();
    cfg.terrain_dir = Some(TERRAIN.clone());
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    let realm = Realm::new(cfg, store, startup, d, None);
    let (_a, g) = alice(&realm, Class::Warrior).await;
    let under = on_map(&realm, move |m| {
        let surface = environment::liquid_at(
            m,
            Vector3d {
                x: SHALLOWS.0,
                y: SHALLOWS.1,
                z: SHALLOWS.2,
            },
        )
        .expect("the shallows")
        .surface;
        let mut out = Vec::new();
        // Display 50 a yard high; display 0 unknown, two yards.
        for (display, scale) in [(50, 1.0), (0, 1.0), (50, 2.0)] {
            m.players.get_mut(&g).unwrap().misc.breath = None;
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_u32(UNIT_FIELD_DISPLAYID, display);
            p.fields.set_f32(OBJECT_FIELD_SCALE_X, scale);
            place(m, g, (SHALLOWS.0, SHALLOWS.1, surface - 1.5));
            check(m, g, Duration::ZERO);
            out.push(breath(m, g).is_some());
        }
        out
    })
    .await;
    assert_eq!(
        under,
        vec![true, false, false],
        "a yard; two (unknown); a yard at twice the scale"
    );
}

const SLIME: u32 = crate::game::map::terrain::SPELL_NAXX_SLIME;
const OTHER_LIQUID: u32 = 90_200;

/// Naxxramas's slime as Thistle Tea's test has it: a tenth of every stat,
/// and a hundred nature every two seconds; and another liquid's spell.
fn slimy() -> GameData {
    let mut d = world();
    for id in [SLIME, OTHER_LIQUID] {
        let mut effects = [SpellEffect::default(); 3];
        effects[0] = SpellEffect {
            effect: 6,
            aura: AURA_MOD_TOTAL_STAT_PERCENTAGE,
            base_points: -91,
            base_dice: 1,
            misc_value: -1,
            target_a: 1,
            ..Default::default()
        };
        effects[1] = SpellEffect {
            effect: 6,
            aura: AURA_PERIODIC_DAMAGE,
            base_points: 99,
            base_dice: 1,
            amplitude: 2000,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(
            id,
            SpellTemplate {
                id,
                name: format!("Liquid {id}"),
                school: 3,
                effects,
                equipped_item_class: -1,
                ..Default::default()
            },
        );
    }
    d
}

fn liquid_holder(m: &Map, g: Guid, spell: u32) -> Option<(Option<Instant>, Option<Instant>)> {
    m.players[&g]
        .auras
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .map(|h| (h.expires, h.effects.iter().find_map(|a| a.next_tick)))
}

#[tokio::test]
async fn a_liquids_spell_goes_on_once_comes_back_while_in_it_and_changes_with_the_liquid() {
    let realm = harness::realm_with(slimy());
    let (_a, g) = alice(&realm, Class::Warrior).await;
    let (once, restored, changed, dead) = on_map(&realm, move |m| {
        hardy(m, g);
        environment::liquid_spell(m, g, Some(SLIME));
        let first = liquid_holder(m, g, SLIME);
        // In it still: the same aura, its ticks where they were.
        environment::liquid_spell(m, g, Some(SLIME));
        let once = first.is_some() && liquid_holder(m, g, SLIME) == first;
        // Taken off while in it: it comes back.
        remove_spell(m, g, SLIME, None);
        environment::liquid_spell(m, g, Some(SLIME));
        let restored = liquid_holder(m, g, SLIME).is_some();
        // Another liquid: its spell, the slime's gone.
        environment::liquid_spell(m, g, Some(OTHER_LIQUID));
        let changed = (
            liquid_holder(m, g, SLIME).is_none(),
            liquid_holder(m, g, OTHER_LIQUID).is_some(),
        );
        // Dead in it: the update gives the dead no liquid spell.
        crate::game::map::death::kill(m, g);
        environment::liquid_spell(m, g, None);
        let dead = (
            liquid_holder(m, g, OTHER_LIQUID).is_none(),
            m.players[&g].misc.liquid_spell,
        );
        (once, restored, changed, dead)
    })
    .await;
    assert!(once, "on once, its ticks kept");
    assert!(restored);
    assert_eq!(changed, (true, true));
    assert_eq!(dead, (true, None));
}

#[tokio::test]
async fn a_liquids_tick_that_kills_leaves_no_spell_behind() {
    let realm = harness::realm_with(slimy());
    let (_a, g) = alice(&realm, Class::Warrior).await;
    // Standing still in the slime at fifty health, its environment checks
    // held off: the tick kills her.
    on_map(&realm, move |m| {
        hardy(m, g);
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 50);
        m.players.get_mut(&g).unwrap().misc.env_checked_at =
            Some(Instant::now() + Duration::from_secs(3600));
        environment::liquid_spell(m, g, Some(SLIME));
        let h = m.players.get_mut(&g).unwrap();
        for holder in h.auras.holders.iter_mut().filter(|x| x.spell == SLIME) {
            for a in holder.effects.iter_mut() {
                if a.next_tick.is_some() {
                    a.next_tick = Some(Instant::now());
                }
            }
        }
    })
    .await;
    let mut dead = false;
    for _ in 0..100 {
        dead = on_map(&realm, move |m| {
            m.players[&g].death != crate::game::map::death::Death::Alive
        })
        .await;
        if dead {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(dead, "the slime's tick kills");
    let (aura, tracked) = on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().misc.env_checked_at = None;
        environment::update(m);
        (
            liquid_holder(m, g, SLIME).is_some(),
            m.players[&g].misc.liquid_spell,
        )
    })
    .await;
    assert_eq!((aura, tracked), (false, None));
}

const PROWL_SPEED: u32 = 90_300;
const CAT_FORM: u32 = 90_301;

#[tokio::test]
async fn an_outdoor_passive_comes_back_outdoors_only_in_its_form_alive_and_known() {
    let mut d = world();
    aura_spell(&mut d, PROWL_SPEED, AURA_MOD_INCREASE_SPEED, 30, 0);
    let s = d.spells.spells.get_mut(&PROWL_SPEED).unwrap();
    s.attributes[0] =
        crate::game::data::spells::ATTR_PASSIVE | crate::game::data::spells::ATTR_ONLY_OUTDOORS;
    s.stances = 1;
    aura_spell(&mut d, CAT_FORM, AURA_MOD_SHAPESHIFT, 1, 1);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::NightElf, Class::Druid).await;
    let map = realm
        .maps
        .get(&realm, realm.registry.online(g).unwrap().map().key);
    let passive = move |m: &mut Map| m.players[&g].auras.has_spell(PROWL_SPEED);
    let reconcile = |outside: Option<bool>| {
        move |m: &mut Map| {
            match outside {
                Some(o) => m.spells.outdoors_override.insert(g, o),
                None => m.spells.outdoors_override.remove(&g),
            };
            crate::game::map::spells::environment::reconcile(m, g, Instant::now());
            m.players[&g].auras.has_spell(PROWL_SPEED)
        }
    };
    map.query(move |m| m.players.get_mut(&g).unwrap().spells.push(PROWL_SPEED))
        .await;
    // Out of her form, outdoors: not on.
    assert_eq!(
        map.query(reconcile(Some(true))).await,
        Some(false),
        "not in cat form"
    );
    // In cat form outdoors: on; indoors: off; out again: back.
    map.query(move |m| put_on(m, g, CAT_FORM)).await;
    assert_eq!(
        map.query(reconcile(Some(true))).await,
        Some(true),
        "a cat outdoors"
    );
    assert_eq!(
        map.query(reconcile(Some(false))).await,
        Some(false),
        "indoors"
    );
    assert_eq!(
        map.query(reconcile(Some(true))).await,
        Some(true),
        "outdoors again"
    );
    // Nothing known of where she is: what she has stays.
    assert_eq!(
        map.query(reconcile(None)).await,
        Some(true),
        "unknown: kept"
    );
    // Dead, or a ghost: not back.
    map.query(move |m| {
        crate::game::map::death::kill(m, g);
    })
    .await;
    assert_eq!(map.query(reconcile(Some(true))).await, Some(false), "dead");
    map.query(move |m| crate::game::map::death::release(m, g))
        .await;
    assert_eq!(
        map.query(reconcile(Some(true))).await,
        Some(false),
        "a ghost"
    );
    // Alive, a cat, outdoors, but unlearned: not back.
    map.query(move |m| {
        crate::game::map::death::resurrect(m, g, 1.0);
        m.players
            .get_mut(&g)
            .unwrap()
            .spells
            .retain(|s| *s != PROWL_SPEED);
        put_on(m, g, CAT_FORM);
    })
    .await;
    assert_eq!(
        map.query(reconcile(Some(true))).await,
        Some(false),
        "unlearned"
    );
    let _ = map.query(passive).await;
}

#[test]
fn lava_is_touched_in_it_or_a_hair_over_it_and_only_lava_burns() {
    use crate::game::map::terrain::*;
    let lava = Liquid {
        flags: LIQUID_MAGMA,
        surface: 0.0,
        floor: -10.0,
        entry: None,
    };
    assert!(lava.magma() && lava.touching(0.08), "walking on it");
    assert!(!lava.touching(0.1), "a tenth over, jumping");
    assert!(lava.touching(-3.0));
    for flags in [LIQUID_OCEAN, LIQUID_SLIME, LIQUID_WATER] {
        assert!(!Liquid { flags, ..lava }.magma(), "{flags}");
    }
}

#[tokio::test]
async fn lavas_grace_comes_back_ten_times_as_fast_and_is_kept_when_one_steps_in_again() {
    let realm = harness::realm_with(world());
    let (_a, g) = alice(&realm, Class::Warrior).await;
    let steps = on_map(&realm, move |m| {
        let f = &mut m.players.get_mut(&g).unwrap().fields;
        f.set_u32(UNIT_FIELD_MAXHEALTH, 3000);
        f.set_u32(UNIT_FIELD_HEALTH, 3000);
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut out = Vec::new();
        environment::lava(m, g, Some(true), t0);
        environment::lava(m, g, Some(true), at(999));
        out.push(health(m, g));
        environment::lava(m, g, Some(true), at(1000));
        out.push(health(m, g));
        // A jump: off it, filling; down again fifty milliseconds on, half
        // a second's grace.
        environment::lava(m, g, Some(false), at(1000));
        environment::lava(m, g, Some(true), at(1050));
        let grace = m.players[&g].misc.lava.map(|l| l.remaining);
        environment::lava(m, g, Some(true), at(1549));
        out.push(health(m, g));
        environment::lava(m, g, Some(true), at(1550));
        out.push(health(m, g));
        // Out of any liquid: the timer's gone, and nothing burns.
        environment::lava(m, g, None, at(3000));
        let gone = m.players[&g].misc.lava.is_none();
        environment::lava(m, g, None, at(5000));
        out.push(health(m, g));
        (out, grace, gone)
    })
    .await;
    let (h, grace, gone) = steps;
    assert_eq!(h[0], 3000, "the second's grace");
    assert!((2390..=2395).contains(&h[1]), "{}", h[1]);
    assert_eq!(grace, Some(Duration::from_millis(500)));
    assert_eq!(h[2], h[1], "half a second's grace again");
    assert!(
        h[3] < h[2] && h[2] - h[3] >= 605 && h[2] - h[3] <= 610,
        "{h:?}"
    );
    assert!(gone);
    assert_eq!(h[4], h[3]);
}

#[tokio::test]
#[ignore = "extended"]
async fn the_real_environmental_fires_are_fire_and_burn_without_a_fight() {
    let Some(real) = crate::game::data::load::real_data() else {
        return;
    };
    let ids = [7897, 7902, 12_796, 20_533, 20_676, 21_650, 23_485, 29_115];
    for id in ids {
        let s = real.spells.get(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(s.school, 2, "{id}: fire");
        let kinds: Vec<u32> = s
            .effects
            .iter()
            .map(|e| e.effect)
            .filter(|e| *e != 0)
            .collect();
        assert_eq!(
            kinds,
            vec![crate::game::map::spells::EFFECT_ENVIRONMENTAL_DAMAGE],
            "{id}"
        );
    }
    let mut d = world();
    super::tests_p3_world_bg::copy_spell(&mut d, real, 7897);
    super::tests_pvp2::npc(&mut d, 90_500, "Kobold", 14);
    let realm = harness::realm_with(d);
    let (mut a, g) = alice(&realm, Class::Warrior).await;
    a.clear();
    let fighting = on_map(&realm, move |m| {
        hardy(m, g);
        let pos = m.players[&g].position();
        let spawn = crate::game::data::CreatureSpawn {
            guid: 0x7D_0001,
            entries: vec![90_500],
            map: 0,
            x: pos.x + 2.0,
            y: pos.y,
            z: pos.z,
            ..Default::default()
        };
        let data = m.realm.data.clone();
        let c = crate::game::map::creature::Creature::from_spawn(&data, &spawn, &mut rand::rng())
            .unwrap();
        let fire = m.add_creature(c);
        crate::game::map::spells::cast_triggered(m, fire, 7897, Some(g));
        m.players[&g].fields.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    })
    .await;
    assert!(!fighting, "no fight");
    let (kind, ..) = log(&mut a).await;
    assert_eq!(kind, EnvironmentalDamageType::Fire);
}
