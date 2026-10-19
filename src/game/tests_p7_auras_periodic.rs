//! Round 7's periodic aura tests, ported from Thistle Tea's
//! `periodic_sequence_test.exs` and `periodic_death_test.exs`
//! (docs/parity/matrix.csv): a pulse's spell, its cadence through a
//! refresh, and what death does to ticks.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{
    ATTR_EX3_DEATH_PERSISTENT, CUSTOM_POSITIVE, SpellDuration, SpellEffect, SpellTemplate,
};
use crate::game::fields::*;
use crate::game::map::{Map, auras, combat};
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_spells_kit::pass;

/// A pulse every five seconds casting `MARK` on its unit, and the mark.
const PULSE: u32 = 97_100;
const MARK: u32 = 97_101;
/// A tick that kills, with a heal and mana in the same tick.
const DOOM: u32 = 97_110;
/// Ticks of damage, healing, mana and energy that last through death,
/// two seconds long.
const LASTING: [u32; 4] = [97_120, 97_121, 97_122, 97_123];
const TWO_SECONDS: u32 = 970;

fn effect(aura: u32, misc: i32, amplitude: u32, trigger: u32) -> SpellEffect {
    SpellEffect {
        effect: 6,
        aura,
        misc_value: misc,
        amplitude,
        trigger_spell: trigger,
        target_a: 1,
        ..Default::default()
    }
}

fn spell(id: u32, effects: &[SpellEffect]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        ..Default::default()
    };
    for (i, e) in effects.iter().enumerate() {
        s.effects[i] = *e;
    }
    s
}

fn data() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.durations.insert(
        TWO_SECONDS,
        SpellDuration {
            base: 2000,
            per_level: 0,
            max: 2000,
        },
    );
    let mut mark = spell(MARK, &[effect(auras::AURA_DUMMY, 0, 0, 0)]);
    mark.custom_flags = CUSTOM_POSITIVE;
    let mut pulse = spell(
        PULSE,
        &[effect(auras::AURA_PERIODIC_TRIGGER_SPELL, 0, 5000, MARK)],
    );
    pulse.custom_flags = CUSTOM_POSITIVE;
    let doom = spell(
        DOOM,
        &[
            effect(auras::AURA_PERIODIC_DAMAGE, 0, 1000, 0),
            effect(auras::AURA_PERIODIC_HEAL, 0, 1000, 0),
            effect(auras::AURA_OBS_MOD_MANA, 0, 1000, 0),
        ],
    );
    let kinds = [
        (auras::AURA_PERIODIC_DAMAGE, 0),
        (auras::AURA_PERIODIC_HEAL, 0),
        (auras::AURA_OBS_MOD_MANA, 0),
        (auras::AURA_PERIODIC_ENERGIZE, 0),
    ];
    for (id, (kind, misc)) in LASTING.into_iter().zip(kinds) {
        let mut s = spell(id, &[effect(kind, misc, 1000, 0)]);
        s.attributes[3] = ATTR_EX3_DEATH_PERSISTENT;
        s.duration_index = TWO_SECONDS;
        d.spells.spells.insert(id, s);
    }
    for s in [mark, pulse, doom] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn put(m: &mut Map, caster: Guid, target: Guid, id: u32, amount: i32, ctx: auras::Context) {
    let s = m.realm.data.spells.spells[&id].clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].aura != 0)
        .map(|i| (i, amount))
        .collect();
    auras::apply_spell(m, caster, target, &s, &effects, ctx).expect("put on");
}

fn has(m: &Map, g: Guid, id: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(id))
}

/// Whether `g` was marked since last asked (the mark taken off).
fn marked(m: &mut Map, g: Guid) -> bool {
    let was = has(m, g, MARK);
    auras::remove_where(m, g, |h| h.spell == MARK);
    was
}

fn pulse_effect(m: &Map, g: Guid) -> Option<auras::Aura> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == PULSE)?
        .effects
        .first()
        .cloned()
}

/// Thistle Tea's `PeriodicSequence` "ordinary triggers keep their
/// configured spell" and "removal expiry and death stop the sequence": a
/// pulse casts its own spell at each tick, every five seconds, and none
/// once it is taken off, once it has run out (its last tick at its end
/// had), or once its unit died.
#[tokio::test]
async fn a_pulse_casts_its_spell_each_tick_until_it_goes() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        let mut seen = Vec::new();
        put(m, ga, ga, PULSE, 0, auras::Context::default());
        for ms in [4999, 1, 5000] {
            pass(m, ms);
            seen.push(marked(m, ga));
        }
        // Taken off.
        auras::remove_where(m, ga, |h| h.spell == PULSE);
        pass(m, 10_000);
        seen.push(marked(m, ga));
        // Five seconds long: its tick at its end, then none.
        let five = auras::Context {
            duration: Some(Some(5000)),
            ..Default::default()
        };
        put(m, ga, ga, PULSE, 0, five);
        pass(m, 5000);
        seen.push(marked(m, ga));
        seen.push(has(m, ga, PULSE));
        pass(m, 10_000);
        seen.push(marked(m, ga));
        // On a creature that dies.
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        put(m, brute, brute, PULSE, 0, auras::Context::default());
        combat::kill(m, ga, brute);
        let gone = !has(m, brute, PULSE);
        pass(m, 10_000);
        (seen, gone, has(m, brute, MARK))
    })
    .await;
    let (seen, gone, dead_marked) = out;
    assert_eq!(
        seen,
        [false, true, true, false, true, false, false],
        "(4.999 s, 5 s, 10 s, off, at its end, still on, after)"
    );
    assert!(gone && !dead_marked, "death stops it");
}

/// Thistle Tea's `PeriodicSequence` "refresh restarts the sequence and
/// interval": a pulse put on again starts its count of ticks over (a
/// sequence, Shadow Bolt Whirl's, from its first) and its next tick five
/// seconds from then, none before.
#[tokio::test]
async fn a_pulse_put_on_again_starts_its_count_and_interval_over() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let out = on(&map, move |m| {
        put(m, ga, ga, PULSE, 0, auras::Context::default());
        pass(m, 5000);
        pass(m, 5000);
        marked(m, ga);
        let before = pulse_effect(m, ga).map(|x| x.ticks);
        pass(m, 1000);
        put(m, ga, ga, PULSE, 0, auras::Context::default());
        let now = m.now;
        let after = pulse_effect(m, ga).map(|x| (x.ticks, x.next_tick.map(|t| t - now)));
        pass(m, 4000);
        let early = marked(m, ga);
        pass(m, 1000);
        (before, after, early, marked(m, ga))
    })
    .await;
    let (before, after, early, on_time) = out;
    assert_eq!(before, Some(2));
    assert_eq!(after, Some((0, Some(Duration::from_secs(5)))));
    assert!(!early && on_time, "five seconds from the refresh");
}

/// Thistle Tea's `PeriodicDeath` "advances every retained resource aura
/// without affecting corpses": ticks that last through death (damage,
/// healing, mana, energy) keep their cadence on a corpse and do nothing to
/// it, and run out when they would have.
#[tokio::test]
async fn ticks_that_outlast_death_keep_time_and_leave_the_corpse_be() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 100);
        let start = m.now;
        for id in LASTING {
            put(m, brute, ga, id, 10, auras::Context::default());
        }
        combat::kill(m, brute, ga);
        let mana = m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1);
        pass(m, 1000);
        let f = &m.players[&ga].fields;
        let after = (f.get_u32(UNIT_FIELD_HEALTH), f.get_u32(UNIT_FIELD_POWER1));
        let next: Vec<Option<Duration>> = auras::auras(m, ga)
            .map(|a| {
                a.holders
                    .iter()
                    .filter(|h| LASTING.contains(&h.spell))
                    .map(|h| h.effects[0].next_tick.map(|t| t - start))
                    .collect()
            })
            .unwrap_or_default();
        pass(m, 1000);
        let left = LASTING.iter().filter(|id| has(m, ga, **id)).count();
        (mana, after, next, left)
    })
    .await;
    let (mana, after, next, left) = out;
    assert_eq!(after, (0, mana), "the corpse untouched");
    assert_eq!(
        next,
        vec![Some(Duration::from_secs(2)); 4],
        "each on its cadence"
    );
    assert_eq!(left, 0, "run out at two seconds");
}

/// Thistle Tea's `PeriodicDeath` "lethal damage cannot be undone by a
/// later effect in the same tick": a tick that kills leaves its heal and
/// mana in the same tick undone, and its aura goes with the death.
#[tokio::test]
async fn a_tick_that_kills_heals_nothing_after_it() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Mage).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 100);
        let mana = m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1);
        put(m, brute, ga, DOOM, 1_000_000, auras::Context::default());
        pass(m, 1000);
        let f = &m.players[&ga].fields;
        (
            mana,
            f.get_u32(UNIT_FIELD_HEALTH),
            f.get_u32(UNIT_FIELD_POWER1),
            has(m, ga, DOOM),
        )
    })
    .await;
    let (mana, health, after, on_her) = out;
    assert_eq!((health, after, on_her), (0, mana, false));
}

/// Thistle Tea's `GolemaggsTrust` "dead and foreign casters never refresh
/// the buff": Golemagg's Trust pulses the Core Ragers' buff only from
/// Golemagg's own aura, while he lives and fights; another's on him, or
/// his on him dead, gives nothing.
#[tokio::test]
async fn golemaggs_trust_pulses_only_from_his_own_aura_while_he_lives() {
    const GOLEMAGG: u32 = 11988;
    const CORE_RAGER: u32 = 11672;
    const TRUST: u32 = 20556;
    const TRUST_BUFF: u32 = 20553;
    let mut d = data();
    for entry in [GOLEMAGG, CORE_RAGER] {
        d.creature_templates
            .insert(entry, crate::game::tests_p3_combat::template(entry, 7));
    }
    let mut trust = spell(
        TRUST,
        &[effect(
            auras::AURA_PERIODIC_TRIGGER_SPELL,
            0,
            1000,
            TRUST_BUFF,
        )],
    );
    trust.custom_flags = CUSTOM_POSITIVE;
    let mut buff = spell(TRUST_BUFF, &[effect(auras::AURA_MOD_DAMAGE_DONE, 1, 0, 0)]);
    buff.custom_flags = CUSTOM_POSITIVE;
    for s in [trust, buff] {
        d.spells.spells.insert(s.id, s);
    }
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        let golemagg = spawn(m, ga, GOLEMAGG, 63, 5.0);
        let rager = spawn(m, ga, CORE_RAGER, 60, 8.0);
        m.unit_fields_mut(golemagg).unwrap().set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            true,
        );
        let pulse = |m: &mut Map, caster: Guid| {
            auras::remove_where(m, rager, |h| h.spell == TRUST_BUFF);
            put(m, caster, golemagg, TRUST, 0, auras::Context::default());
            pass(m, 1000);
            let buffed = has(m, rager, TRUST_BUFF);
            auras::remove_where(m, golemagg, |h| h.spell == TRUST);
            buffed
        };
        let foreign = pulse(m, ga);
        let own = pulse(m, golemagg);
        put(m, golemagg, golemagg, TRUST, 0, auras::Context::default());
        combat::kill(m, ga, golemagg);
        auras::remove_where(m, rager, |h| h.spell == TRUST_BUFF);
        pass(m, 1000);
        (foreign, own, has(m, rager, TRUST_BUFF))
    })
    .await;
    assert_eq!(out, (false, true, false), "(another's, his own, his dead)");
}

/// Thistle Tea's `WorldBossScript` "leaves players without mana marked":
/// Mark of Kazzak on a warrior (rage, no mana to drain) drains nothing
/// and never goes off; the mark stays.
#[tokio::test]
async fn mark_of_kazzak_leaves_a_warrior_marked_and_unexploded() {
    const MARK_OF_KAZZAK: u32 = 21056;
    const EXPLOSION: u32 = 21058;
    let mut d = data();
    let mut mark = spell(
        MARK_OF_KAZZAK,
        &[effect(auras::AURA_PERIODIC_MANA_LEECH, 0, 2000, 0)],
    );
    mark.effects[0].multiple_value = 1.0;
    let mut explosion = spell(EXPLOSION, &[]);
    explosion.effects[0] = SpellEffect {
        effect: 2,
        base_points: 49,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    for s in [mark, explosion] {
        d.spells.spells.insert(s.id, s);
    }
    let (_realm, _a, ga, map) = setup_with(d, Class::Warrior).await;
    let out = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let health = m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH);
        put(m, brute, ga, MARK_OF_KAZZAK, 250, auras::Context::default());
        pass(m, 2000);
        pass(m, 2000);
        (
            has(m, ga, MARK_OF_KAZZAK),
            m.players[&ga].fields.get_u32(UNIT_FIELD_HEALTH) == health,
        )
    })
    .await;
    assert_eq!(out, (true, true), "(still marked, not exploded)");
}
