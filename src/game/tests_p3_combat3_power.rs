//! Phase 3's power tests, part two (Thistle Tea's `resources_test.exs`,
//! `power_burn_test.exs`, `power_leech_test.exs` and
//! `power_restoration_test.exs`): rage from damage, what a spell costs and
//! who pays it, power restored, burned and drained, at once and tick by
//! tick.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ATTR_EX3_ALWAYS_HIT, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{self, *};
use crate::game::map::combat;
use crate::game::map::spells::{self, POWER_HEALTH, Targets};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{BRUTE, aura_of, spawn};
use crate::game::tests_p3_combat3_regen::world;
use crate::game::tests_p3_spells_kit::{fresh, pass, tick_now};

/// Mana Burn's like: 200 of `misc`'s power, half of it as shadow damage.
const BURN: u32 = 95_400;
const BURN_ENERGY: u32 = 95_401;
const BURN_RAGE: u32 = 95_402;
const BURN_NONE: u32 = 95_403;
const BURN_SIX: u32 = 95_404;
const BURN_HARMLESS: u32 = 95_405;
/// The same, each second for three.
const BURNING: u32 = 95_410;
const BURNING_100: u32 = 95_411;
const BURNING_HARMLESS: u32 = 95_412;
/// Drain Mana's like: 200 of `misc`'s power, half of it back.
const DRAIN: u32 = 95_420;
const DRAIN_ENERGY: u32 = 95_421;
const DRAIN_SELF: u32 = 95_422;
const DRAIN_ONE: u32 = 95_423;
const DRAIN_BAD: u32 = 95_424;
/// Viper Sting's like: 100 a second for three, half of it back.
const LEECHING: u32 = 95_430;
const LEECHING_ENERGY: u32 = 95_431;
const LEECHING_NONE: u32 = 95_432;
/// Fifty of a power, on the target.
const ENERGIZE: u32 = 95_440;
const ENERGIZE_ENERGY: u32 = 95_441;
const ENERGIZE_LESS: u32 = 95_442;
const ENERGIZE_BAD: u32 = 95_443;
/// Costs: 30 mana; 20% of base health; ten and a tenth of base mana;
/// nothing; 33 health a second of a channel.
const COSTLY: u32 = 95_450;
const BLOOD: u32 = 95_451;
const SHARE: u32 = 95_452;
const FREE: u32 = 95_453;
const BLOOD_CHANNEL: u32 = 95_454;
/// A hundred percent more of the burns' and drains' multiplier.
const CONVERSION: u32 = 95_460;

const SHADOW: u32 = 5;
const FAMILY: u32 = 5;

fn spell(id: u32, effect: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        school: SHADOW,
        dmg_class: 1,
        family_name: FAMILY,
        family_flags: 1,
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 900,
        ..Default::default()
    };
    s.attributes[3] = ATTR_EX3_ALWAYS_HIT;
    s.effects[0] = effect;
    s
}

fn hit(effect: u32, aura: u32, points: i32, misc: i32, mult: f32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect,
        aura,
        base_points: points - 1,
        base_dice: 1,
        die_sides: 1,
        misc_value: misc,
        multiple_value: mult,
        amplitude: if aura != 0 { 1000 } else { 0 },
        target_a,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = world();
    d.spells.durations.insert(
        900,
        SpellDuration {
            base: 3000,
            per_level: 0,
            max: 3000,
        },
    );
    d.spells.durations.insert(
        901,
        SpellDuration {
            base: 5000,
            per_level: 0,
            max: 5000,
        },
    );
    let mut add = |s: SpellTemplate| {
        d.spells.spells.insert(s.id, s);
    };
    for (id, misc, mult) in [
        (BURN, 0, 0.5),
        (BURN_ENERGY, 3, 0.5),
        (BURN_RAGE, 1, 0.5),
        (BURN_NONE, -1, 0.5),
        (BURN_SIX, 5, 0.5),
        (BURN_HARMLESS, 0, 0.0),
    ] {
        add(spell(id, hit(62, 0, 200, misc, mult, 6)));
    }
    for (id, points, mult) in [
        (BURNING, 200, 0.5),
        (BURNING_100, 100, 0.5),
        (BURNING_HARMLESS, 200, 0.0),
    ] {
        add(spell(id, hit(6, AURA_POWER_BURN_MANA, points, 0, mult, 6)));
    }
    for (id, misc, mult, target) in [
        (DRAIN, 0, 0.5, 6),
        (DRAIN_ENERGY, 3, 0.5, 6),
        (DRAIN_SELF, 0, 0.5, 1),
        (DRAIN_ONE, 0, 0.0, 6),
        (DRAIN_BAD, 5, 0.5, 6),
    ] {
        add(spell(id, hit(8, 0, 200, misc, mult, target)));
    }
    for (id, misc, mult) in [
        (LEECHING, 0, 0.5),
        (LEECHING_ENERGY, 3, 0.5),
        (LEECHING_NONE, 0, 0.0),
    ] {
        add(spell(
            id,
            hit(6, AURA_PERIODIC_MANA_LEECH, 100, misc, mult, 6),
        ));
    }
    for (id, misc, points) in [
        (ENERGIZE, 0, 50),
        (ENERGIZE_ENERGY, 3, 50),
        (ENERGIZE_LESS, 0, -50),
        (ENERGIZE_BAD, 5, 50),
    ] {
        let mut s = spell(id, hit(30, 0, points, misc, 0.0, 6));
        s.dmg_class = 0;
        s.attributes[3] = 0;
        add(s);
    }
    let dummy = || hit(3, 0, 1, 0, 0.0, 1);
    let mut costly = spell(COSTLY, dummy());
    costly.mana_cost = 30;
    add(costly);
    let mut blood = spell(BLOOD, dummy());
    blood.power_type = POWER_HEALTH;
    blood.mana_cost_percentage = 20;
    add(blood);
    let mut share = spell(SHARE, dummy());
    share.mana_cost = 10;
    share.mana_cost_percentage = 10;
    add(share);
    add(spell(FREE, dummy()));
    let mut channel = spell(BLOOD_CHANNEL, dummy());
    channel.power_type = POWER_HEALTH;
    channel.mana_per_second = 33;
    channel.attributes[1] = crate::game::data::spells::ATTR_EX_CHANNELED;
    channel.duration_index = 901;
    add(channel);
    let mut conversion = spell(CONVERSION, hit(6, AURA_ADD_PCT_MODIFIER, 100, 27, 0.0, 1));
    conversion.effects[0].item_type = 1;
    conversion.attributes[3] = 0;
    add(conversion);
    d
}

async fn player(class: Class) -> (Client, Guid, MapHandle) {
    let realm = crate::game::harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let race = if class == Class::Druid {
        Race::NightElf
    } else {
        Race::Human
    };
    let g = a.enter("Alice", race, class).await;
    let map = realm.registry.online(g).unwrap().map();
    (a, g, map)
}

/// A creature 5 yards ahead of `g`, of 1000 health and 150 mana, showing
/// `power`.
fn mana_user(m: &mut Map, g: Guid, power: u8) -> Guid {
    let c = spawn(m, g, BRUTE, 60, 5.0);
    let f = m.unit_fields_mut(c).unwrap();
    f.set_u32(UNIT_FIELD_MAXHEALTH, 1000);
    f.set_u32(UNIT_FIELD_HEALTH, 1000);
    f.set_u32(UNIT_FIELD_MAXPOWER1, 150);
    f.set_u32(UNIT_FIELD_POWER1, 150);
    f.set_u32(UNIT_FIELD_MAXPOWER1 + 3, 100);
    f.set_u32(UNIT_FIELD_POWER1 + 3, 60);
    f.set_byte(UNIT_FIELD_BYTES_0, 3, power);
    c
}

/// Health, mana and energy.
fn pools(m: &Map, g: Guid) -> (u32, u32, u32) {
    let f = m.unit_fields(g).unwrap();
    (
        f.get_u32(UNIT_FIELD_HEALTH),
        f.get_u32(UNIT_FIELD_POWER1),
        f.get_u32(UNIT_FIELD_POWER1 + 3),
    )
}

fn set_pools(m: &mut Map, g: Guid, health: u32, mana: (u32, u32)) {
    let f = m.unit_fields_mut(g).unwrap();
    f.set_u32(UNIT_FIELD_HEALTH, health);
    f.set_u32(UNIT_FIELD_MAXPOWER1, mana.1);
    f.set_u32(UNIT_FIELD_POWER1, mana.0);
}

fn trigger(m: &mut Map, caster: Guid, spell: u32, target: Guid) {
    spells::cast_triggered(m, caster, spell, Some(target));
}

fn threat_of(m: &Map, c: Guid, on: Guid) -> f32 {
    m.creature(c)
        .and_then(|c| c.threat.iter().find(|(g, _)| *g == on).map(|t| t.1))
        .unwrap_or(0.0)
}

#[test]
fn rage_from_damage_follows_the_vanilla_conversion() {
    // The conversion at 1, 30 and 60: 7.5, 109.23 and 230.6.
    assert_eq!(combat::rage_for(1000, 1, false), 9999);
    assert_eq!(combat::rage_for(1000, 30, false), 686);
    assert_eq!(combat::rage_for(1000, 60, false), 325);
    // Twelve dealt at 1: twelve rage (Thistle Tea's doubles fall a hair
    // short, to 119).
    assert_eq!(combat::rage_for(12, 1, false), 120);
    // Less at a higher level; a third as much for damage taken.
    assert_eq!(combat::rage_for(200, 60, false), 65);
    assert_eq!(combat::rage_for(200, 60, true), 21);
    // Berserker Rage's 1.3 before rounding down.
    assert_eq!(combat::rage_scaled(200, 60, true, 1.3), 28);
}

#[tokio::test]
async fn rage_comes_from_damage_to_its_cap_and_only_to_those_who_use_it() {
    let (_a, w, map) = player(Class::Warrior).await;
    let got = map
        .query(move |m| {
            let rage = |m: &Map| m.players[&w].fields.get_u32(UNIT_FIELD_POWER1 + 1);
            let set = |m: &mut Map, v: u32| {
                m.players
                    .get_mut(&w)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_FIELD_POWER1 + 1, v);
            };
            crate::game::tests_p3_combat::level(m, w, 60);
            set(m, 0);
            combat::gain_rage(m, w, 200, false);
            let dealt = rage(m);
            set(m, 0);
            combat::gain_rage(m, w, 200, true);
            let taken = rage(m);
            // Berserker Rage: 1.3 times as much from blows taken.
            aura_of(m, w, combat::SPELL_BERSERKER_RAGE, AURA_DUMMY, 0, 0);
            set(m, 0);
            combat::gain_rage(m, w, 200, true);
            let berserk = rage(m);
            auras::remove_spell(m, w, combat::SPELL_BERSERKER_RAGE, None);
            // Never past a hundred.
            set(m, 950);
            combat::gain_rage(m, w, 2000, false);
            (dealt, taken, berserk, rage(m))
        })
        .await
        .unwrap();
    assert_eq!(got, (65, 21, 28, 1000));
    let (_b, mage, map) = player(Class::Mage).await;
    let rage = map
        .query(move |m| {
            combat::gain_rage(m, mage, 200, false);
            combat::gain_rage(m, mage, 200, true);
            m.players[&mage].fields.get_u32(UNIT_FIELD_POWER1 + 1)
        })
        .await
        .unwrap();
    assert_eq!(rage, 0, "a mage gains no rage");
}

#[tokio::test]
async fn a_spell_costs_its_mana_and_starts_the_five_second_rule_unless_free_or_for_a_god() {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for s in [COSTLY, FREE, SHARE] {
                spells::learn(m, g, s);
            }
            let reset = |m: &mut Map| {
                set_pools(m, g, 100, (100, 100));
                m.players.get_mut(&g).unwrap().mana_spent_at = None;
                fresh(m, g);
            };
            let me = Targets {
                unit: Some(g),
                ..Default::default()
            };
            reset(m);
            spells::cast(m, g, COSTLY, me, None);
            out.push((pools(m, g).1, m.players[&g].mana_spent_at.is_some()));
            // Nothing to pay: nothing paid, no rule.
            reset(m);
            spells::cast(m, g, FREE, me, None);
            out.push((pools(m, g).1, m.players[&g].mana_spent_at.is_some()));
            // Ten and a tenth of base mana (80): eighteen.
            reset(m);
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_BASE_MANA, 80);
            spells::cast(m, g, SHARE, me, None);
            out.push((pools(m, g).1, m.players[&g].mana_spent_at.is_some()));
            // A god pays nothing, even what they don't have.
            reset(m);
            m.players.get_mut(&g).unwrap().misc.god = true;
            set_pools(m, g, 100, (10, 100));
            let cast = spells::cast(m, g, COSTLY, me, None);
            m.players.get_mut(&g).unwrap().misc.god = false;
            out.push((pools(m, g).1, m.players[&g].mana_spent_at.is_some()));
            (out, cast)
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        [(70, true), (100, false), (82, true), (10, false)],
        "mana left, and the rule"
    );
    assert!(got.1, "a god casts what they can't pay for");
}

#[tokio::test]
async fn a_cost_in_health_comes_off_health_but_never_kills_and_a_channel_pays_it_each_second() {
    let (_a, g, map) = player(Class::Warrior).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            for s in [BLOOD, BLOOD_CHANNEL] {
                spells::learn(m, g, s);
            }
            let me = Targets {
                unit: Some(g),
                ..Default::default()
            };
            let rage = m.players[&g].fields.get_u32(UNIT_FIELD_POWER1 + 1);
            // A fifth of base health (80): sixteen.
            let f = &mut m.players.get_mut(&g).unwrap().fields;
            f.set_u32(UNIT_FIELD_BASE_HEALTH, 80);
            f.set_u32(UNIT_FIELD_MAXHEALTH, 100);
            f.set_u32(UNIT_FIELD_HEALTH, 100);
            fresh(m, g);
            spells::cast(m, g, BLOOD, me, None);
            out.push(pools(m, g).0);
            // Seventeen left: one.
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 17);
            fresh(m, g);
            spells::cast(m, g, BLOOD, me, None);
            out.push(pools(m, g).0);
            // A god pays none.
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 100);
            m.players.get_mut(&g).unwrap().misc.god = true;
            fresh(m, g);
            spells::cast(m, g, BLOOD, me, None);
            m.players.get_mut(&g).unwrap().misc.god = false;
            out.push(pools(m, g).0);
            // A channel of 33 health a second: 67 after one; at 33 it
            // can't go on.
            fresh(m, g);
            spells::cast(m, g, BLOOD_CHANNEL, me, None);
            let channelling = crate::game::tests_p3_spells_kit::casting(m, g);
            pass(m, 1000);
            out.push(pools(m, g).0);
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 33);
            pass(m, 1000);
            let after = crate::game::tests_p3_spells_kit::casting(m, g);
            out.push(pools(m, g).0);
            let rage_after = m.players[&g].fields.get_u32(UNIT_FIELD_POWER1 + 1);
            (out, channelling, after, rage, rage_after)
        })
        .await
        .unwrap();
    let (health, channelling, after, rage, rage_after) = got;
    assert_eq!(health, [84, 1, 100, 67, 33]);
    assert_eq!(channelling, Some(BLOOD_CHANNEL));
    assert_eq!(after, None, "the channel ends when it can't pay");
    assert_eq!(rage_after, rage, "no rage paid for a cost in health");
}

#[tokio::test]
async fn energizing_fills_any_pool_there_is_to_its_cap_and_logs_the_whole_amount() {
    let (mut a, g, map) = player(Class::Warrior).await;
    let target = map
        .query(move |m| {
            let c = mana_user(m, g, 3);
            set_pools(m, c, 1000, (80, 100));
            c
        })
        .await
        .unwrap();
    a.sees(target).await;
    map.query(move |m| trigger(m, g, ENERGIZE, target))
        .await
        .unwrap();
    // Shown energy, but the mana is filled: to its cap of a hundred.
    let log = a
        .wait_for("the energize log", |msg| match msg {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_SPELLENERGIZELOG(l)
                if l.spell == ENERGIZE =>
            {
                Some(*l)
            }
            _ => None,
        })
        .await;
    assert_eq!(
        (log.victim, log.damage, log.power),
        (target, 50, Power::Mana)
    );
    let got = map
        .query(move |m| {
            let mut out = vec![pools(m, target)];
            // A pool of none: nothing.
            set_pools(m, target, 1000, (0, 0));
            trigger(m, g, ENERGIZE, target);
            out.push(pools(m, target));
            // Less than none, and a power that isn't one: nothing.
            set_pools(m, target, 1000, (80, 100));
            trigger(m, g, ENERGIZE_LESS, target);
            trigger(m, g, ENERGIZE_BAD, target);
            out.push(pools(m, target));
            // The dead get nothing.
            combat::deal_damage(m, g, target, 5000);
            trigger(m, g, ENERGIZE, target);
            out.push(pools(m, target));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [(1000, 100, 60), (1000, 0, 60), (1000, 80, 60), (0, 80, 60)]
    );
}

#[tokio::test]
async fn a_burn_takes_only_the_power_there_is_from_the_pool_shown_and_harms_by_its_multiplier() {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // 150 mana of the 200: 75 shadow damage.
            let c = mana_user(m, g, 0);
            trigger(m, g, BURN, c);
            out.push(pools(m, c));
            // Energy shown: the energy goes, the mana stays.
            let c = mana_user(m, g, 3);
            trigger(m, g, BURN_ENERGY, c);
            out.push(pools(m, c));
            // A power not shown, none, or no power at all: nothing.
            let c = mana_user(m, g, 0);
            for s in [BURN_RAGE, BURN_NONE, BURN_SIX] {
                trigger(m, g, s, c);
            }
            out.push(pools(m, c));
            // None to burn: nothing.
            set_pools(m, c, 1000, (0, 150));
            trigger(m, g, BURN, c);
            out.push(pools(m, c));
            // A multiplier of none burns without harm.
            let c = mana_user(m, g, 0);
            trigger(m, g, BURN_HARMLESS, c);
            out.push(pools(m, c));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            (925, 0, 60),
            (970, 150, 0),
            (1000, 150, 60),
            (1000, 0, 60),
            (1000, 0, 60)
        ]
    );
}

#[tokio::test]
async fn a_burns_harm_is_absorbed_crits_takes_the_conversion_modifiers_and_an_immune_target_keeps_all()
 {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // A shield of 30, then of 100: the mana goes all the same.
            for capacity in [30, 100] {
                let c = mana_user(m, g, 0);
                aura_of(m, c, 95_470, AURA_SCHOOL_ABSORB, 32, capacity);
                trigger(m, g, BURN, c);
                out.push(pools(m, c));
            }
            // Immune to shadow: neither health nor mana goes.
            let c = mana_user(m, g, 0);
            aura_of(m, c, 95_471, AURA_SCHOOL_IMMUNITY, 32, 0);
            trigger(m, g, BURN, c);
            out.push(pools(m, c));
            // Twice the multiplier and a sure crit, but no spell power:
            // 150 times one, and half again.
            crate::game::tests_p3_spells_kit::apply(m, g, g, CONVERSION);
            aura_of(m, g, 95_472, AURA_MOD_DAMAGE_DONE, 32, 1000);
            m.players.get_mut(&g).unwrap().derived.spell_crit[SHADOW as usize] = 100.0;
            let c = mana_user(m, g, 0);
            trigger(m, g, BURN, c);
            out.push(pools(m, c));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [(955, 0, 60), (1000, 0, 60), (1000, 150, 60), (775, 0, 60)]
    );
}

/// `spell`'s aura from `g` on `c`, and its first tick.
fn burn_tick(m: &mut Map, g: Guid, c: Guid, spell: u32) {
    trigger(m, g, spell, c);
    tick_now(m, c, spell);
}

#[tokio::test]
async fn a_burn_over_time_takes_what_mana_there_is_each_tick_absorbed_or_lethal() {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // A hundred a tick: 50 mana left and 50 harm; then the last 50
            // and 25.
            let c = mana_user(m, g, 0);
            burn_tick(m, g, c, BURNING_100);
            out.push(pools(m, c));
            tick_now(m, c, BURNING_100);
            out.push(pools(m, c));
            // Absorbed: the mana goes all the same.
            let c = mana_user(m, g, 0);
            aura_of(m, c, 95_470, AURA_SCHOOL_ABSORB, 32, 100);
            burn_tick(m, g, c, BURNING);
            out.push(pools(m, c));
            // A multiplier of none: mana, no harm.
            let c = mana_user(m, g, 0);
            burn_tick(m, g, c, BURNING_HARMLESS);
            out.push(pools(m, c));
            // Lethal: dead, and the burn is gone.
            let c = mana_user(m, g, 0);
            set_pools(m, c, 50, (150, 150));
            burn_tick(m, g, c, BURNING);
            out.push(pools(m, c));
            let gone = !crate::game::tests_p3_spells_kit::has(m, c, BURNING);
            (out, gone)
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        [
            (950, 50, 60),
            (925, 0, 60),
            (1000, 0, 60),
            (1000, 0, 60),
            (0, 0, 60)
        ]
    );
    assert!(got.1, "the dead lose the burn");
}

#[tokio::test]
async fn a_burn_over_time_spares_a_pool_not_shown_and_the_immune_and_ends_with_its_aura() {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // In cat form: no tick; back to mana, it burns.
            let c = mana_user(m, g, 3);
            burn_tick(m, g, c, BURNING);
            out.push(pools(m, c));
            m.unit_fields_mut(c)
                .unwrap()
                .set_byte(UNIT_FIELD_BYTES_0, 3, 0);
            tick_now(m, c, BURNING);
            out.push(pools(m, c));
            // Immune: the tick is lost.
            let c = mana_user(m, g, 0);
            trigger(m, g, BURNING, c);
            aura_of(m, c, 95_471, AURA_SCHOOL_IMMUNITY, 32, 0);
            tick_now(m, c, BURNING);
            out.push(pools(m, c));
            // Taken off: no more ticks.
            let c = mana_user(m, g, 0);
            trigger(m, g, BURNING, c);
            auras::remove_spell(m, c, BURNING, None);
            tick_now(m, c, BURNING);
            out.push(pools(m, c));
            // Cast again, it starts over: one aura.
            let c = mana_user(m, g, 0);
            trigger(m, g, BURNING, c);
            trigger(m, g, BURNING, c);
            let holders = auras::auras(m, c).map_or(0, |a| {
                a.holders.iter().filter(|h| h.spell == BURNING).count()
            });
            (out, holders)
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        [
            (1000, 150, 60),
            (925, 0, 60),
            (1000, 150, 60),
            (1000, 150, 60)
        ]
    );
    assert_eq!(got.1, 1);
}

#[tokio::test]
async fn a_drain_takes_the_mana_there_is_and_gives_back_its_share_to_a_caster_who_uses_mana() {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            let now = m.now;
            // 150 taken, 75 back; the victim's health and rule untouched.
            let c = mana_user(m, g, 0);
            set_pools(m, g, 100, (0, 1000));
            trigger(m, g, DRAIN, c);
            let rule = m.creature(c).unwrap().engagement.mana_spent_at;
            out.push((pools(m, c), pools(m, g).1));
            // A multiplier of none is one.
            let c = mana_user(m, g, 0);
            set_pools(m, g, 100, (0, 1000));
            trigger(m, g, DRAIN_ONE, c);
            out.push((pools(m, c), pools(m, g).1));
            // Energy, which the caster doesn't use: none back.
            let c = mana_user(m, g, 3);
            set_pools(m, g, 100, (0, 1000));
            trigger(m, g, DRAIN_ENERGY, c);
            out.push((pools(m, c), pools(m, g).1));
            // Oneself: none back.
            set_pools(m, g, 100, (150, 1000));
            trigger(m, g, DRAIN_SELF, g);
            out.push(((0, 0, 0), pools(m, g).1));
            // A power not shown, no power, the immune and the dead keep it.
            set_pools(m, g, 100, (0, 1000));
            let c = mana_user(m, g, 3);
            trigger(m, g, DRAIN, c);
            trigger(m, g, DRAIN_BAD, c);
            out.push((pools(m, c), pools(m, g).1));
            let c = mana_user(m, g, 0);
            aura_of(m, c, 95_471, AURA_SCHOOL_IMMUNITY, 32, 0);
            trigger(m, g, DRAIN, c);
            out.push((pools(m, c), pools(m, g).1));
            let c = mana_user(m, g, 0);
            combat::deal_damage(m, g, c, 5000);
            trigger(m, g, DRAIN, c);
            out.push((pools(m, c), pools(m, g).1));
            (out, rule, now)
        })
        .await
        .unwrap();
    assert_eq!(
        got.0,
        [
            ((1000, 0, 60), 75),
            ((1000, 0, 60), 150),
            ((1000, 150, 0), 0),
            ((0, 0, 0), 0),
            ((1000, 150, 60), 0),
            ((1000, 150, 60), 0),
            ((0, 150, 60), 0),
        ]
    );
    assert_eq!(got.1, None, "draining isn't spending");
}

#[tokio::test]
async fn a_leech_over_time_gives_back_to_its_cap_with_threat_and_nothing_to_one_without_the_pool() {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // 100 taken, 50 back, but 10 fills her: 5 threat.
            let c = mana_user(m, g, 0);
            set_pools(m, g, 100, (140, 150));
            burn_tick(m, g, c, LEECHING);
            out.push((pools(m, c).1, pools(m, g).1, threat_of(m, c, g)));
            // Full: nothing back, no more threat.
            tick_now(m, c, LEECHING);
            out.push((pools(m, c).1, pools(m, g).1, threat_of(m, c, g)));
            // No mana to fill: the victim loses it, the caster gains none.
            let c = mana_user(m, g, 0);
            set_pools(m, g, 100, (0, 0));
            burn_tick(m, g, c, LEECHING);
            out.push((pools(m, c).1, pools(m, g).1, threat_of(m, c, g)));
            // A multiplier of none stays none.
            let c = mana_user(m, g, 0);
            set_pools(m, g, 100, (0, 1000));
            burn_tick(m, g, c, LEECHING_NONE);
            out.push((pools(m, c).1, pools(m, g).1, 0.0));
            // Her modifiers as they are at the tick: twice the half.
            let c = mana_user(m, g, 0);
            trigger(m, g, LEECHING, c);
            crate::game::tests_p3_spells_kit::apply(m, g, g, CONVERSION);
            tick_now(m, c, LEECHING);
            out.push((pools(m, c).1, pools(m, g).1, 0.0));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            (50, 150, 5.0),
            (0, 150, 5.0),
            (50, 0, 0.0),
            (50, 0, 0.0),
            (50, 100, 0.0)
        ]
    );
}

#[tokio::test]
async fn a_leech_over_time_takes_the_pool_shown_skips_a_dead_caster_and_ends_with_its_aura() {
    let (_a, g, map) = player(Class::Druid).await;
    let got = map
        .query(move |m| {
            let mut out = Vec::new();
            // Energy from one showing energy, into her energy.
            let c = mana_user(m, g, 3);
            let f = m.unit_fields_mut(g).unwrap();
            f.set_u32(UNIT_FIELD_POWER1, 0);
            f.set_u32(UNIT_FIELD_MAXPOWER1 + 3, 100);
            f.set_u32(UNIT_FIELD_POWER1 + 3, 0);
            burn_tick(m, g, c, LEECHING_ENERGY);
            out.push((pools(m, c), pools(m, g).2));
            // Shifted out of mana: no tick, and none caught up after.
            let c = mana_user(m, g, 3);
            burn_tick(m, g, c, LEECHING);
            out.push((pools(m, c), 0));
            m.unit_fields_mut(c)
                .unwrap()
                .set_byte(UNIT_FIELD_BYTES_0, 3, 0);
            tick_now(m, c, LEECHING);
            out.push((pools(m, c), 0));
            // Taken off, or its target dead: no more.
            let c = mana_user(m, g, 0);
            trigger(m, g, LEECHING, c);
            auras::remove_spell(m, c, LEECHING, None);
            tick_now(m, c, LEECHING);
            out.push((pools(m, c), 0));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [
            ((1000, 150, 0), 30),
            ((1000, 150, 60), 0),
            ((1000, 50, 60), 0),
            ((1000, 150, 60), 0),
        ]
    );
}

#[tokio::test]
async fn a_leech_with_its_caster_dead_takes_nothing() {
    let (_a, g, map) = player(Class::Mage).await;
    let got = map
        .query(move |m| {
            let c = mana_user(m, g, 0);
            trigger(m, g, LEECHING, c);
            let now = m.now;
            m.players.get_mut(&g).unwrap().death =
                crate::game::map::death::Death::Dead { since: now };
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 0);
            tick_now(m, c, LEECHING);
            let after = pools(m, c);
            m.players.get_mut(&g).unwrap().death = crate::game::map::death::Death::Alive;
            after
        })
        .await
        .unwrap();
    assert_eq!(got, (1000, 150, 60));
}
