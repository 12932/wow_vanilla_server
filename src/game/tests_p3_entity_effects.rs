//! Phase 3's spell effect tests, ported from Thistle Tea's
//! `test/game/world/entity/effect_resolver` (docs/parity/matrix.csv): a
//! meteor's damage shared among those it falls on.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, AURA_SCHOOL_IMMUNITY};
use crate::game::map::spells;
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::{aura_effect, on, spell};

/// 302 fire to the enemies within ten yards of where it falls, shared;
/// the same with a second effect of 90 at its target; the same, not
/// shared; and fire proofing.
const METEOR: u32 = 99_000;
const METEOR_AND_BOLT: u32 = 99_001;
const FIREBALL_RAIN: u32 = 99_002;
const LIMITED_METEOR: u32 = 99_003;
const SLOW_METEOR: u32 = 99_004;
const FIRE_PROOF: u32 = 99_005;

fn meteor(id: u32, shared: bool) -> crate::game::data::spells::SpellTemplate {
    let mut s = spell(
        id,
        &[SpellEffect {
            effect: 2,
            base_points: 301,
            base_dice: 1,
            die_sides: 1,
            target_a: 16,
            radius_index: 13,
            ..Default::default()
        }],
    );
    s.school = 2;
    if shared {
        s.script_name = "spell_meteor".into();
    }
    s
}

fn world() -> GameData {
    let mut d = data();
    d.spells.radii.insert(13, 10.0);
    let mut both = meteor(METEOR_AND_BOLT, true);
    both.effects[1] = SpellEffect {
        effect: 2,
        base_points: 89,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut limited = meteor(LIMITED_METEOR, true);
    limited.max_affected_targets = 2;
    let mut slow = meteor(SLOW_METEOR, true);
    slow.speed = 20.0;
    for s in [
        meteor(METEOR, true),
        both,
        meteor(FIREBALL_RAIN, false),
        limited,
        slow,
        spell(
            FIRE_PROOF,
            &[aura_effect(AURA_SCHOOL_IMMUNITY, 1, 1 << 2, 1)],
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

const FULL: u32 = 100_000;

fn lost(m: &Map, g: Guid) -> u32 {
    FULL - m
        .unit_fields(g)
        .map_or(FULL, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn heal(m: &mut Map, gs: &[Guid]) {
    for g in gs {
        if let Some(f) = m.unit_fields_mut(*g) {
            f.set_u32(UNIT_FIELD_HEALTH, FULL);
        }
    }
}

/// Thistle Tea's `SharedDamage`: a meteor's damage is divided among the
/// living it falls on within its radius, each share truncated; a target
/// limit is applied before the count; another effect's own target isn't
/// divided; ordinary area damage isn't shared; a recipient immune to it
/// keeps its share from the others; and the count is the launch's, though
/// recipients go before it lands.
#[tokio::test]
async fn a_meteors_damage_is_shared_among_those_it_falls_on() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        let brutes: Vec<Guid> = (0..3)
            .map(|i| spawn(m, ga, BRUTE, 20, 10.0 + i as f32))
            .collect();
        let dead = spawn(m, ga, BRUTE, 20, 9.0);
        crate::game::map::combat::deal_damage(m, ga, dead, 10_000_000);
        let far = spawn(m, ga, BRUTE, 20, 40.0);
        let at = m.position_of(brutes[1]).unwrap();
        let mut seen = Vec::new();
        let mut fall = |m: &mut Map, id: u32, unit: Option<Guid>| {
            heal(m, &brutes);
            heal(m, &[far]);
            spells::cast_triggered_with(
                m,
                ga,
                id,
                spells::Targets {
                    unit,
                    dest: Some(at),
                    ..Default::default()
                },
            );
            seen.push((
                brutes.iter().map(|g| lost(m, *g)).collect::<Vec<_>>(),
                lost(m, far),
            ));
        };
        fall(m, METEOR, None);
        fall(m, LIMITED_METEOR, None);
        fall(m, METEOR_AND_BOLT, Some(brutes[0]));
        fall(m, FIREBALL_RAIN, None);
        // The first proof against fire: the others' shares stay a third.
        let proof = m.realm.data.spells.get(FIRE_PROOF).cloned().unwrap();
        auras::apply(m, brutes[0], brutes[0], &proof, 0, 1);
        fall(m, METEOR, None);
        auras::remove_spell(m, brutes[0], FIRE_PROOF, None);
        seen
    })
    .await;
    assert_eq!(
        out[0],
        (vec![100, 100, 100], 0),
        "a third each, truncated; the dead and the far don't count"
    );
    let limited: Vec<u32> = out[1].0.iter().copied().filter(|d| *d > 0).collect();
    assert_eq!(limited, vec![151, 151], "two of the three, a half each");
    assert_eq!(
        out[2].0,
        vec![190, 100, 100],
        "the second effect's target takes it whole besides"
    );
    assert_eq!(
        out[3].0,
        vec![302, 302, 302],
        "ordinary area damage is whole"
    );
    assert_eq!(
        out[4].0,
        vec![0, 100, 100],
        "the proofed one keeps its share"
    );
}

/// Thistle Tea's "retains counts when recipients disappear after launch":
/// a meteor on its way keeps the count it was launched with, though one of
/// those it was falling on has gone when it lands.
#[tokio::test]
async fn a_meteor_on_its_way_keeps_its_count() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Mage).await;
    let out = on(&map, move |m| {
        let brutes: Vec<Guid> = (0..3)
            .map(|i| spawn(m, ga, BRUTE, 20, 10.0 + i as f32))
            .collect();
        let at = m.position_of(brutes[1]).unwrap();
        m.spells.travel = Some(true);
        spells::cast_triggered_with(
            m,
            ga,
            SLOW_METEOR,
            spells::Targets {
                dest: Some(at),
                ..Default::default()
            },
        );
        let flying = m.spells.in_flight.len();
        m.remove_creature(brutes[2]);
        m.now += Duration::from_secs(5);
        spells::update(m);
        m.spells.travel = None;
        (flying, [brutes[0], brutes[1]].map(|g| lost(m, g)))
    })
    .await;
    assert!(out.0 >= 1, "it flies");
    assert_eq!(out.1, [100, 100], "still a third each");
}
