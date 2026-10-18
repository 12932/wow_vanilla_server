//! Phase 3's creature tests, ported from Thistle Tea's
//! `test/game/core/creature` (docs/parity/matrix.csv): the core hounds'
//! linked feign death.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Class;

use crate::game::data::GameData;
use crate::game::data::scripts::ScriptStep;
use crate::game::fields::*;
use crate::game::map::{Map, event_ai, scripts};
use crate::game::tests_p3_combat::{BRUTE, aura_of, data, setup_with, spawn, template};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_entity_objects::until;

/// A core hound, unkillable as its template has it; and another kind of
/// hound.
const HOUND: u32 = 3_790;
const OTHER_HOUND: u32 = 3_791;
const PACIFY: u32 = 97_900;

fn hounds() -> GameData {
    let mut d = data();
    let mut hound = template(HOUND, 1);
    hound.static_flags1 |= crate::game::map::combat::STATIC_UNKILLABLE;
    d.creature_templates.insert(HOUND, hound);
    d.creature_templates
        .insert(OTHER_HOUND, template(OTHER_HOUND, 1));
    d
}

/// Alice, beyond a pack's harm.
fn hardy(m: &mut Map, g: Guid) {
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000_000);
    f.set_u32(UNIT_FIELD_HEALTH, 100_000_000);
}

/// A hound's linked feign death: up after `delay_ms` if a hound fights on
/// within 30 yards.
fn link(m: &mut Map, g: Guid, delay_ms: u32) {
    let feign = ScriptStep {
        dataint: [0, 0, 0, 0],
        ..crate::game::tests_p3_entity::step(
            scripts::C_LINKED_FEIGN_DEATH,
            [delay_ms, HOUND, 30, 0],
        )
    };
    scripts::execute(m, &feign, g, Guid::zero());
}

/// Alive, health, stand state, the fallen flag, down.
fn state(m: &Map, g: Guid) -> (bool, u32, u8, bool, bool) {
    let c = m.creature(g).unwrap();
    (
        c.alive(),
        c.fields.get_u32(UNIT_FIELD_HEALTH),
        c.fields.get_byte(UNIT_FIELD_BYTES_1, 0),
        c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & 0x20 != 0,
        event_ai::feigning(m, g),
    )
}

/// Thistle Tea's `LinkedFeignDeath.prevent`: a killing blow, exact or
/// overkill, leaves the hound down at one health, its template's floor of
/// one notwithstanding; it keeps its foe's threat and tap, but its auras
/// go; and down, no more harm, healing or regeneration touches it. A blow
/// short of lethal leaves it fighting, and a creature without the link
/// dies of the same.
#[tokio::test]
async fn a_fallen_hound_takes_the_killing_blow_and_nothing_more() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let h = spawn(m, ga, HOUND, 20, 3.0);
        link(m, h, 60_000);
        crate::game::map::ai::attacked(m, h, ga);
        aura_of(m, h, PACIFY, crate::game::map::auras::AURA_MOD_PACIFY, 0, 0);
        // Short of lethal: hurt, not down.
        crate::game::map::combat::deal_damage(m, ga, h, 99_999);
        let hurt = state(m, h);
        // Exactly lethal.
        crate::game::map::combat::deal_damage(m, ga, h, 1);
        let down = state(m, h);
        let c = m.creature(h).unwrap();
        let kept = (
            c.threat.iter().any(|(t, _)| *t == ga),
            c.tapped_by,
            c.auras.holders.is_empty(),
            c.combat.attacking,
        );
        // More harm, a heal, regeneration: still one.
        crate::game::map::combat::deal_damage(m, ga, h, 1_000);
        crate::game::map::spells::heal(m, ga, h, 0, 1_000, false);
        let now = m.now;
        m.creature_mut(h).unwrap().engagement.regen_at = Some(now);
        crate::game::map::creature_combat::regenerate(m);
        let after = state(m, h);
        // Overkill on a second.
        let h2 = spawn(m, ga, HOUND, 20, 4.0);
        link(m, h2, 60_000);
        crate::game::map::ai::attacked(m, h2, ga);
        crate::game::map::combat::deal_damage(m, ga, h2, 10_000_000);
        let overkill = state(m, h2);
        // No link: dead.
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        crate::game::map::ai::attacked(m, brute, ga);
        crate::game::map::combat::deal_damage(m, ga, brute, 10_000_000);
        let brute = m.creature(brute).is_none_or(|c| !c.alive());
        (hurt, down, kept, after, overkill, brute)
    })
    .await;
    assert_eq!(out.0, (true, 1, 0, false, false), "hurt, still fighting");
    assert_eq!(out.1, (true, 1, 7, true, true), "down at one, fallen");
    assert_eq!(out.2.0, true, "its foe's threat kept");
    assert_eq!(out.2.1, Some(ga), "and its tap");
    assert!(out.2.2, "its auras gone");
    assert!(!out.2.3, "no longer swinging");
    assert_eq!(out.3, out.1, "nothing more touches it");
    assert_eq!(out.4, (true, 1, 7, true, true), "overkill the same");
    assert!(out.5, "the brute, unlinked, dies");
}

/// Thistle Tea's `LinkedFeignDeath.tick`: a hound down, with no hound of
/// its kind fighting on within the radius, dies at its time; one out of
/// reach or of another kind doesn't count. A script's kill takes no
/// notice of the link.
#[tokio::test]
async fn a_fallen_hound_dies_without_a_fighting_kin_in_reach() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let (h, far, other, killed) = on(&map, move |m| {
        hardy(m, ga);
        let h = spawn(m, ga, HOUND, 20, 3.0);
        let far = spawn(m, ga, HOUND, 20, 45.0);
        let other = spawn(m, ga, OTHER_HOUND, 20, 4.0);
        let killed = spawn(m, ga, HOUND, 20, 5.0);
        for g in [h, far, other, killed] {
            link(m, g, 300);
            crate::game::map::ai::attacked(m, g, ga);
        }
        // The far one held where it is, out of reach.
        aura_of(
            m,
            far,
            PACIFY + 1,
            crate::game::map::auras::AURA_MOD_ROOT,
            0,
            0,
        );
        crate::game::map::combat::deal_damage(m, ga, h, 10_000_000);
        crate::game::map::combat::kill(m, ga, killed);
        (h, far, other, killed)
    })
    .await;
    let killed = on(&map, move |m| m.creature(killed).is_none_or(|c| !c.alive())).await;
    assert!(killed, "a script's kill kills");
    until(&map, "the fallen hound dies", move |m| {
        m.creature(h).is_none_or(|c| !c.alive())
    })
    .await;
    let others = on(&map, move |m| {
        [far, other].map(|g| m.creature(g).is_some_and(|c| c.alive()))
    })
    .await;
    assert_eq!(
        others,
        [true, true],
        "the far hound and the other kind fight on"
    );
}

/// Thistle Tea's `LinkedFeignDeath.reset`: a fallen hound whose foe gives
/// up the fight gets up whole, out of it.
#[tokio::test]
async fn a_fallen_hound_left_alone_gets_up_whole() {
    let (_realm, _a, ga, map) = setup_with(hounds(), Class::Warrior).await;
    let h = on(&map, move |m| {
        let h = spawn(m, ga, HOUND, 20, 3.0);
        link(m, h, 60_000);
        crate::game::map::ai::attacked(m, h, ga);
        crate::game::map::combat::deal_damage(m, ga, h, 10_000_000);
        crate::game::map::ai::enter_evade(m, h);
        h
    })
    .await;
    until(&map, "up again", move |m| {
        let c = m.creature(h).unwrap();
        !event_ai::feigning(m, h)
            && c.fields.get_u32(UNIT_FIELD_HEALTH) == c.fields.get_u32(UNIT_FIELD_MAXHEALTH)
    })
    .await;
    let s = on(&map, move |m| state(m, h)).await;
    assert_eq!(
        (s.0, s.2, s.3, s.4),
        (true, 0, false, false),
        "standing, whole"
    );
}

/// A world of brutes spawned in a row beside Alice, spawn `guid` linked to
/// its master's spawn with `flags` for each of `links` (vmangos's
/// `creature_linking`).
fn linked(links: &[(u32, u32, u32)]) -> GameData {
    let mut d = data();
    let mut guids: Vec<u32> = links.iter().flat_map(|(s, m, _)| [*s, *m]).collect();
    guids.sort_unstable();
    guids.dedup();
    for (i, guid) in guids.into_iter().enumerate() {
        d.creature_spawns
            .entry(0)
            .or_default()
            .push(crate::game::data::CreatureSpawn {
                guid,
                entries: vec![BRUTE],
                map: 0,
                x: crate::game::tests_p3_combat::START.0 + 2.0 + i as f32,
                y: crate::game::tests_p3_combat::START.1,
                z: crate::game::tests_p3_combat::START.2,
                respawn_min: 300,
                respawn_max: 300,
                ..Default::default()
            });
    }
    for (slave, master, flags) in links {
        d.scripts.links.insert(*slave, (*master, *flags));
    }
    d
}

fn of(spawn: u32) -> Guid {
    crate::game::guid::unit(BRUTE, spawn)
}

fn alive(m: &Map, spawn: u32) -> bool {
    m.creature(of(spawn)).is_some_and(|c| c.alive())
}

fn fighting(m: &Map, spawn: u32) -> bool {
    m.creature(of(spawn))
        .is_some_and(|c| c.alive() && c.combat.attacking)
}

fn die(m: &mut Map, killer: Guid, spawn: u32) {
    crate::game::map::combat::deal_damage(m, killer, of(spawn), 10_000_000);
}

/// Thistle Tea's `CreatureLink.slave_command` and `master_command` on
/// aggro: a slave flagged to joins its master's fight, once (one already
/// fighting or dead is left be), and one not flagged doesn't; a slave
/// flagged to pulls its master in, and its master's own aggro doesn't pull
/// it.
#[tokio::test]
async fn linked_slaves_and_masters_join_each_others_fights_as_flagged() {
    use crate::game::data::scripts::{LINK_AGGRO_ON_AGGRO, LINK_TO_AGGRO_ON_AGGRO};
    // 100 leads 101 (joins it), 102 (pulls it in), 103 (neither) and 104
    // (joins it, but dead).
    let d = linked(&[
        (101, 100, LINK_AGGRO_ON_AGGRO),
        (102, 100, LINK_TO_AGGRO_ON_AGGRO),
        (103, 100, 0),
        (104, 100, LINK_AGGRO_ON_AGGRO),
        (111, 110, LINK_TO_AGGRO_ON_AGGRO),
    ]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(111)).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        die(m, ga, 104);
        crate::game::map::ai::attacked(m, of(100), ga);
        let first = [101, 102, 103, 104].map(|s| fighting(m, s));
        // The slave that pulls its master in, pulled.
        crate::game::map::ai::attacked(m, of(111), ga);
        (first, fighting(m, 110))
    })
    .await;
    assert_eq!(
        out.0,
        [true, false, false, false],
        "only the slave flagged to join, alive"
    );
    assert!(out.1, "the slave pulls its master in");
}

/// Thistle Tea's `CreatureLink.slave_command` on a master's evade, respawn,
/// despawn and death: its evade despawns, evades, or raises its slaves as
/// each is flagged; a slave's evade raises its dead master when flagged to;
/// its respawn raises its dead slaves or takes away its living ones; its
/// despawn takes along the slaves flagged to go with it; and its death
/// passes nothing on, as in vmangos.
#[tokio::test]
async fn a_masters_evade_respawn_despawn_and_death_pass_to_its_slaves_by_flag() {
    use crate::game::data::scripts::*;
    let d = linked(&[
        (121, 120, LINK_DESPAWN_ON_EVADE),
        (122, 120, LINK_EVADE_ON_EVADE | LINK_AGGRO_ON_AGGRO),
        (123, 120, LINK_RESPAWN_ON_EVADE),
        (124, 120, LINK_EVADE_ON_EVADE),
        (131, 130, LINK_RESPAWN_ON_RESPAWN),
        (132, 130, LINK_DESPAWN_ON_RESPAWN),
        (133, 130, LINK_DESPAWN_ON_DESPAWN),
        (141, 140, LINK_TO_RESPAWN_ON_EVADE),
        (151, 150, 0x10 | 0x20 | 0x40),
    ]);
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(151)).await;
    let evade = on(&map, move |m| {
        hardy(m, ga);
        die(m, ga, 123);
        crate::game::map::ai::attacked(m, of(120), ga);
        let joined = fighting(m, 122);
        crate::game::map::ai::enter_evade(m, of(120));
        let c = |s| m.creature(of(s)).map(|c| (c.alive(), c.evading));
        (joined, c(121), c(122), alive(m, 123), c(124))
    })
    .await;
    assert!(evade.0, "the evading slave was in the fight");
    assert_eq!(evade.1.map(|c| c.0), Some(false), "despawned");
    assert_eq!(evade.2, Some((true, true)), "evades with it");
    assert!(evade.3, "the dead slave raised");
    assert_eq!(
        evade.4,
        Some((true, false)),
        "one not fighting doesn't evade"
    );
    let respawn = on(&map, move |m| {
        die(m, ga, 131);
        die(m, ga, 130);
        crate::game::map::ai::respawn_now(m, of(130));
        let back = (alive(m, 130), alive(m, 131), alive(m, 132), alive(m, 133));
        crate::game::map::scripts::despawn(m, of(130));
        (back, alive(m, 133))
    })
    .await;
    assert_eq!(
        respawn.0,
        (true, true, false, true),
        "its dead slave raised, the other taken away"
    );
    assert!(!respawn.1, "its despawn takes the third along");
    let other = on(&map, move |m| {
        die(m, ga, 140);
        crate::game::map::ai::attacked(m, of(141), ga);
        crate::game::map::ai::enter_evade(m, of(141));
        let raised = alive(m, 140);
        die(m, ga, 150);
        let c = m.creature(of(151)).unwrap();
        (raised, c.alive(), c.combat.attacking, c.evading)
    })
    .await;
    assert!(other.0, "the slave's evade raises its master");
    assert_eq!(
        (other.1, other.2, other.3),
        (true, false, false),
        "the master's death passes nothing on"
    );
}

/// Thistle Tea's `CreatureLink.spawn_allowed?`: a slave that can't spawn
/// while its master is dead, or alive, follows its master's life, the
/// first flag deciding with both; with its master unknown, or with
/// neither flag, it may.
#[tokio::test]
async fn a_linked_slave_spawns_as_its_masters_life_allows() {
    use crate::game::data::scripts::*;
    let (dead, alive_) = (LINK_CANT_SPAWN_IF_BOSS_DEAD, LINK_CANT_SPAWN_IF_BOSS_ALIVE);
    let mut d = linked(&[
        (161, 160, dead),
        (162, 160, alive_),
        (163, 160, dead | alive_),
        (164, 160, LINK_AGGRO_ON_AGGRO),
    ]);
    d.scripts.links.insert(165, (999, dead));
    let (_realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    a.sees(of(160)).await;
    let out = on(&map, move |m| {
        hardy(m, ga);
        let when_alive =
            [161, 162, 163, 164, 165].map(|s| crate::game::map::creature_groups::may_spawn(m, s));
        die(m, ga, 160);
        let when_dead =
            [161, 162, 163, 164, 165].map(|s| crate::game::map::creature_groups::may_spawn(m, s));
        (when_alive, when_dead)
    })
    .await;
    assert_eq!(out.0, [true, false, true, true, true], "the master alive");
    assert_eq!(out.1, [false, true, false, true, true], "the master dead");
}
