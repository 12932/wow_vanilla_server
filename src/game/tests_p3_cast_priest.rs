//! Phase 3's priest spells, on the real data, as Thistle Tea's
//! `priest_spells_test` has them: Fear Ward, Weakened Soul and the shield,
//! a shackle broken by a tick, Devouring Plague's leech, Resurrection, the
//! charges of Inner Fire and Shadowguard, and Abolish Disease's ticks.

use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells::checks::fail;
use crate::game::map::spells::{self, mods};
use crate::game::map::{Map, auras, combat};

const FEAR_WARD: u32 = 6346;
const PSYCHIC_SCREAM: u32 = 8122;
const WEAKENED_SOUL: u32 = 6788;
const SHIELD: u32 = 17;
const SHACKLE: u32 = 9484;
const PAIN: u32 = 589;
const PLAGUE: u32 = 2944;
const RESURRECTION: u32 = 2006;
const INNER_FIRE: u32 = 588;
const SHADOWGUARD: u32 = 18137;
const ABOLISH_DISEASE: u32 = 552;

/// The kobold `k` swings at `g` once, a plain hit.
fn swing_at(m: &mut Map, k: wow_world_messages::Guid, g: wow_world_messages::Guid) {
    let now = m.now;
    if let Some(c) = m.combat_mut(k) {
        c.next_swing = Some(now);
    }
    combat::FORCED_OUTCOME.with(|f| f.set(Some(combat::Outcome::Hit)));
    // Starting the attack swings at once.
    combat::start_attack(m, k, g);
    combat::FORCED_OUTCOME.with(|f| f.set(None));
    combat::stop_attack(m, k);
}

fn charges(m: &Map, g: wow_world_messages::Guid, spell: u32) -> Option<u32> {
    holder(m, g, spell).map(|h| h.charges)
}

/// Fear Ward keeps off a fear and is spent; Weakened Soul is a debuff on
/// its own priest that can't be cancelled, and keeps a shield off before
/// it costs anything; a shackle breaks at a tick of damage; Devouring
/// Plague heals its priest what each tick takes; Inner Fire spends a
/// charge a blow and Shadowguard strikes back for one, and goes with its
/// last; an aura without charges is left alone; Abolish Disease ticks its
/// cure (Thistle Tea's priest tests).
#[tokio::test]
#[ignore = "extended"]
async fn the_priests_wards_and_charges_and_ticks_do_what_they_say() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (c, priest, map) = player(&realm, "P3R1", "Priestone", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(
                m,
                priest,
                60,
                &[FEAR_WARD, SHIELD, PLAGUE, INNER_FIRE, ABOLISH_DISEASE],
            );
            let k = dummy(m, priest, 60, 2.0);
            // Fear Ward, and a kobold's Psychic Scream.
            cast_fresh(m, priest, FEAR_WARD, Default::default());
            let warded = has(m, priest, FEAR_WARD);
            cast_fresh(m, k, PSYCHIC_SCREAM, Default::default());
            let ward = (
                warded,
                has(m, priest, PSYCHIC_SCREAM),
                has(m, priest, FEAR_WARD),
            );
            auras::remove_spell(m, priest, PSYCHIC_SCREAM, None);
            // Weakened Soul: no shield, before the cost.
            apply(m, priest, priest, WEAKENED_SOUL);
            let soul = holder(m, priest, WEAKENED_SOUL).map(|h| h.positive);
            let mana = power(m, priest, 0);
            fresh(m, priest);
            let s = m.realm.data.spells.get(SHIELD).cloned().unwrap();
            let mods = mods::for_spell(m, priest, &s);
            let refused = spells::checks::check(m, priest, &s, &at(priest), None, false, &mods);
            let shield = (refused, power(m, priest, 0) == mana);
            // A shackle, then a tick of pain.
            apply(m, priest, k, SHACKLE);
            apply(m, priest, k, PAIN);
            let shackled = has(m, k, SHACKLE);
            tick_now(m, k, PAIN);
            let shackle = (
                shackled,
                has(m, k, SHACKLE),
                has(m, k, PAIN),
                health(m, k) < 1_000_000,
            );
            auras::remove_spell(m, k, PAIN, None);
            // Devouring Plague's tick.
            set_health(m, priest, 100);
            apply(m, priest, k, PLAGUE);
            let (kh, ph) = (health(m, k), health(m, priest));
            tick_now(m, k, PLAGUE);
            let plague = (kh - health(m, k), health(m, priest) - ph);
            auras::remove_spell(m, k, PLAGUE, None);
            set_health(m, priest, 1_000);
            // Inner Fire's twenty charges, a blow each.
            cast_fresh(m, priest, INNER_FIRE, Default::default());
            let fire_before = charges(m, priest, INNER_FIRE);
            swing_at(m, k, priest);
            let fire_after = charges(m, priest, INNER_FIRE);
            auras::remove_spell(m, priest, INNER_FIRE, None);
            // Shadowguard: three charges, a bolt for each blow.
            apply(m, priest, priest, SHADOWGUARD);
            let guard_before = charges(m, priest, SHADOWGUARD);
            let kh = health(m, k);
            swing_at(m, k, priest);
            let guard = (
                guard_before,
                charges(m, priest, SHADOWGUARD),
                health(m, k) < kh,
            );
            let mut left = Vec::new();
            for _ in 0..2 {
                m.now += Duration::from_secs(4);
                swing_at(m, k, priest);
                left.push(charges(m, priest, SHADOWGUARD));
            }
            let _ = left;
            let guard_gone = has(m, priest, SHADOWGUARD);
            // Abolish Disease ticks its cure, and stays.
            cast_fresh(m, priest, ABOLISH_DISEASE, Default::default());
            let abolish = holder(m, priest, ABOLISH_DISEASE).map(|h| {
                h.effects
                    .iter()
                    .any(|x| x.period == Some(Duration::from_secs(5)))
            });
            (
                ward,
                soul,
                shield,
                shackle,
                plague,
                (fire_before, fire_after),
                guard,
                guard_gone,
                abolish,
            )
        })
        .await
        .unwrap();
    let (ward, soul, shield, shackle, plague, fire, guard, guard_gone, abolish) = out;
    assert_eq!(
        ward,
        (true, false, false),
        "the ward keeps the fear off, and is spent"
    );
    assert_eq!(soul, Some(false), "Weakened Soul is a debuff on its priest");
    assert_eq!(
        shield,
        (Some(fail::TARGET_AURASTATE), true),
        "no shield, nothing paid"
    );
    assert_eq!(
        shackle,
        (true, false, true, true),
        "the tick breaks the shackle"
    );
    assert!(plague.0 > 0, "the plague's tick hurts");
    assert_eq!(plague.0, plague.1, "and heals its priest as much");
    assert_eq!(fire, (Some(20), Some(19)), "Inner Fire's charge a blow");
    assert_eq!(guard.0, Some(3));
    assert_eq!(guard.1, Some(2), "Shadowguard's charge");
    assert!(guard.2, "and its bolt on the kobold");
    assert!(!guard_gone, "gone with its last");
    assert_eq!(
        abolish,
        Some(true),
        "Abolish Disease ticks every five seconds"
    );
    // Weakened Soul can't be cancelled.
    c.send(CMSG_CANCEL_AURA { id: WEAKENED_SOUL }).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let still = map
        .query(move |m| has(m, priest, WEAKENED_SOUL))
        .await
        .unwrap();
    assert!(still, "Weakened Soul isn't cancelled");
}

/// Resurrection: refused on the living (`TARGET_NOT_DEAD`), offered to the
/// dead (Thistle Tea's `resurrect_new`, mangos's checks).
#[tokio::test]
#[ignore = "extended"]
async fn resurrection_is_refused_on_the_living_and_offered_to_the_dead() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, priest, map) = player(&realm, "P3R2", "Priesttwo", Race::Human, Class::Priest).await;
    let (mut d, dead, _) = player(&realm, "P3R3", "Deadone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, priest, 60, &[RESURRECTION]);
            // The two side by side.
            let at_priest = m.players[&priest].position();
            if let Some(p) = m.players.get_mut(&dead) {
                p.movement.position = Vector3d {
                    x: at_priest.x + 3.0,
                    ..at_priest
                };
            }
            let s = m.realm.data.spells.get(RESURRECTION).cloned().unwrap();
            let mods = mods::for_spell(m, priest, &s);
            let living = spells::checks::check(m, priest, &s, &at(dead), None, false, &mods);
            crate::game::map::death::kill(m, dead);
            let dead_check = spells::checks::check(m, priest, &s, &at(dead), None, false, &mods);
            let cast = cast_fresh(m, priest, RESURRECTION, at(dead));
            (living, dead_check, cast)
        })
        .await
        .unwrap();
    assert_eq!(out.0, Some(fail::TARGET_NOT_DEAD), "not on the living");
    assert_eq!(out.1, None, "on the dead");
    assert!(out.2, "cast");
    // Its ten-second cast, then the offer.
    map.query(move |m| pass(m, 10_500)).await;
    d.wait_for("SMSG_RESURRECT_REQUEST", |m| match m {
        ServerOpcodeMessage::SMSG_RESURRECT_REQUEST(r) if r.guid == priest => Some(()),
        _ => None,
    })
    .await;
}
