//! Phase 4's professions and engineering, Thistle Tea's disenchanting,
//! engineering, Explosive Guardian, gathering, profession and Death Ray
//! tests ported: who may disenchant and when; the Universal Remote's, the
//! Mind Control Cap's, the Goblin Bomb Dispenser's and the Net-o-Matic's
//! outcomes, once a use and not on the dead; the Explosive Sheep's and the
//! Goblin Bomb's one blast, and a dispenser's malfunction (on the real
//! data); a gathering skill's rise; unlearning; and the Gnomish Death
//! Ray's charge, from its first tick to its last.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, MapKey, auras, item_scripts, loot, professions, skills};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
/// Two creatures by the start, to aim at.
const DUMMY: u32 = 3300;
const OTHER: u32 = 3301;

const DEATH_RAY: u32 = 13278;
const DEATH_RAY_HURT: u32 = 13493;
const DEATH_RAY_BLAST: u32 = 13279;
const REMOTE: u32 = 8344;
const CAP: u32 = 13180;
const DISPENSER: u32 = 23134;
const NET: u32 = 13120;
/// The remote's: control, malfunction, and the machine enraged (its own).
const REMOTE_OUTCOMES: [u32; 3] = [8345, 8346, 8599];
const MIND_CONTROL: u32 = 13181;
const MALFUNCTION: u32 = 13261;
const SUMMON_BOMB: u32 = 13258;
/// The net's: its user netted, a net with a backfire, the target netted.
const NET_OUTCOMES: [u32; 3] = [16566, 13119, 13099];
const SHIFT: u32 = 93_100;
const IMMUNE: u32 = 93_101;
const OTHER_CHANNEL: u32 = 93_102;
const DISENCHANT: u32 = 93_103;

const BRACER: u32 = 61_100;
const DUST: u32 = 61_101;
const ENCHANTING: u32 = 333;
const MINING: u32 = 186;
const HERBALISM: u32 = 182;

fn effect(effect: u32, base_points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect,
        base_points,
        base_dice: 1,
        die_sides: 1,
        target_a,
        ..Default::default()
    }
}

fn aura(kind: u32, base_points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        aura: kind,
        ..effect(6, base_points, target_a)
    }
}

fn spell(id: u32, effects: [SpellEffect; 3]) -> SpellTemplate {
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 9,
        effects,
        ..Default::default()
    }
}

fn creature(entry: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 1,
        level_max: 1,
        faction: 8,
        unit_class: 1,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        ..Default::default()
    }
}

/// The Death Ray as the client's data has it (its channel and its hurt
/// four seconds, a tick a second), the gadgets with a dummy effect at
/// each of their first two indices, their outcomes markers; a bracer to
/// disenchant.
fn data() -> GameData {
    let mut d = GameData::empty();
    d.player_level_stats.insert((1, 1, 1), [23, 20, 22, 20, 21]);
    d.player_class_stats.insert((1, 1), (10_000, 0));
    d.class_level_stats.insert(
        (1, 1),
        ClassLevelStats {
            health: 5000,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    for (entry, n, dy) in [(DUMMY, 50, 0.0), (OTHER, 51, 1.0)] {
        d.creature_templates.insert(entry, creature(entry));
        d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
            guid: n,
            entries: vec![entry],
            map: 0,
            x: START.0 + 2.0,
            y: START.1 + dy,
            z: START.2,
            respawn_min: 1,
            respawn_max: 1,
            ..Default::default()
        });
    }
    let s = &mut d.spells;
    s.ranges.insert(1, (0.0, 30.0));
    for (index, base) in [(9, 30_000), (35, 4_000)] {
        s.durations.insert(
            index,
            crate::game::data::spells::SpellDuration {
                base,
                ..Default::default()
            },
        );
    }
    let none = SpellEffect::default();
    let mut all = Vec::new();
    for id in [DEATH_RAY, OTHER_CHANNEL] {
        let mut channel = spell(id, [aura(4, 0, 1), effect(3, 0, 6), none]);
        channel.duration_index = 35;
        channel.attributes[1] = crate::game::data::spells::ATTR_EX_CHANNELED;
        all.push(channel);
    }
    let mut hurt = spell(DEATH_RAY_HURT, [aura(3, 149, 1), none, none]);
    hurt.effects[0].amplitude = 1000;
    hurt.duration_index = 35;
    hurt.school = 2;
    all.push(hurt);
    // Sure to land, so that what it does is what was charged.
    let mut blast = spell(DEATH_RAY_BLAST, [effect(2, 599, 6), none, none]);
    blast.school = 2;
    blast.attributes[3] = crate::game::data::spells::ATTR_EX3_ALWAYS_HIT;
    all.push(blast);
    for id in [REMOTE, CAP, NET] {
        all.push(spell(id, [effect(3, 0, 6), effect(3, 0, 6), none]));
    }
    all.push(spell(DISPENSER, [effect(3, 0, 1), none, none]));
    let marker = |id, target| spell(id, [aura(4, 0, target), none, none]);
    for id in REMOTE_OUTCOMES[..2].iter().chain(&NET_OUTCOMES) {
        all.push(marker(*id, 6));
    }
    all.push(marker(MIND_CONTROL, 6));
    for id in [REMOTE_OUTCOMES[2], MALFUNCTION, SUMMON_BOMB] {
        all.push(marker(id, 1));
    }
    let mut shift = spell(SHIFT, [aura(auras::AURA_MOD_SHAPESHIFT, 0, 1), none, none]);
    shift.effects[0].misc_value = 1;
    all.push(shift);
    let mut immune = spell(
        IMMUNE,
        [aura(auras::AURA_SCHOOL_IMMUNITY, 0, 1), none, none],
    );
    immune.effects[0].misc_value = 127;
    all.push(immune);
    all.push(spell(
        DISENCHANT,
        [effect(professions::EFFECT_DISENCHANT, 0, 1), none, none],
    ));
    for t in all {
        s.spells.insert(t.id, t);
    }
    for entry in [BRACER, DUST] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class: if entry == BRACER { 4 } else { 7 },
                name: format!("Item {entry}"),
                display_id: 1000,
                stackable: if entry == BRACER { 1 } else { 20 },
                allowable_class: -1,
                allowable_race: -1,
                disenchant_id: if entry == BRACER { 30 } else { 0 },
                ..Default::default()
            },
        );
    }
    d.loot.disenchant.insert(
        30,
        vec![LootEntry {
            item: DUST,
            chance: 100.0,
            min: 1,
            max: 1,
            ..Default::default()
        }],
    );
    d
}

async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(dummy()).await;
    a.heartbeat(START.0, START.1, START.2).await;
    (realm, a, ga)
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

fn dummy() -> Guid {
    guid::unit(DUMMY, 50)
}

fn other() -> Guid {
    guid::unit(OTHER, 51)
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn holder(m: &Map, g: Guid, spell: u32) -> Option<auras::Holder> {
    auras::auras(m, g).and_then(|a| a.holders.iter().find(|h| h.spell == spell).cloned())
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).unwrap().clone()
}

/// Move the map's clock on by `ms` and run what time drives in spells and
/// auras, as an update does.
fn pass(m: &mut Map, ms: u64) {
    m.now += Duration::from_millis(ms);
    spells::update(m);
    auras::update(m);
}

/// Which of `spells` are on `g`, and whose each is.
fn markers(m: &Map, g: Guid, spells: &[u32]) -> Vec<(u32, Guid)> {
    spells
        .iter()
        .filter_map(|s| holder(m, g, *s).map(|h| (*s, h.caster)))
        .collect()
}

fn clear(m: &mut Map, gs: &[Guid], spells: &[u32]) {
    for g in gs {
        for s in spells {
            auras::remove_spell(m, *g, *s, None);
        }
    }
}

// The Gnomish Death Ray.

/// `g` at full health, never to crit, aiming at `aim`.
fn steady(m: &mut Map, g: Guid, aim: Guid) {
    let p = m.players.get_mut(&g).unwrap();
    p.derived.spell_crit = [0.0; 7];
    let max = p.fields.get_u32(UNIT_FIELD_MAXHEALTH);
    p.fields.set_u32(UNIT_FIELD_HEALTH, max);
    p.fields.set_guid(UNIT_FIELD_TARGET, aim);
}

/// What `g` is short of full health.
fn hurt(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| {
        f.get_u32(UNIT_FIELD_MAXHEALTH) - f.get_u32(UNIT_FIELD_HEALTH)
    })
}

/// `g` channels the Death Ray at `aim`: the hurt a second it costs.
fn channel(m: &mut Map, g: Guid, aim: Guid) -> u32 {
    begin(m, g, aim);
    holder(m, g, DEATH_RAY_HURT).expect("its hurt").effects[0].amount as u32
}

/// `g` begins to channel the Death Ray at `aim`.
fn begin(m: &mut Map, g: Guid, aim: Guid) {
    if let Some(c) = m.caster_mut(g) {
        c.casting = None;
        c.gcd_until = None;
        c.cooldowns.clear();
    }
    let t = Targets {
        unit: Some(aim),
        ..Default::default()
    };
    let s = template(m, DEATH_RAY);
    let mods = spells::mods::for_spell(m, g, &s);
    let why = spells::checks::check(m, g, &s, &t, None, false, &mods);
    assert!(spells::cast(m, g, DEATH_RAY, t, None), "cast: {why:?}");
    assert_eq!(
        m.caster(g)
            .and_then(|c| c.casting.as_ref())
            .map(|c| c.spell),
        Some(DEATH_RAY),
        "channelling"
    );
}

/// Thistle Tea's "delayed delivery preserves the final tick before the
/// channel ends": the hurt's four ticks all count, the last as the
/// channel runs its course; the whole of it on the target as both end.
#[tokio::test]
async fn the_death_rays_last_tick_counts_as_its_channel_ends() {
    let (realm, _a, ga) = setup().await;
    let target = dummy();
    let (amount, hurt, lost, after) = on_map(&realm, move |m| {
        steady(m, ga, target);
        let amount = channel(m, ga, target);
        let full = health(m, target);
        for _ in 0..4 {
            pass(m, 1000);
        }
        let after = (
            m.caster(ga).and_then(|c| c.casting.as_ref()).is_some(),
            holder(m, ga, DEATH_RAY_HURT).is_some(),
        );
        (amount, hurt(m, ga), full - health(m, target), after)
    })
    .await;
    assert!((100..=500).contains(&amount), "{amount}");
    assert_eq!(hurt, 4 * amount, "four ticks, the last too");
    assert!(lost >= hurt, "the four on the target: {lost} of {hurt}");
    assert_eq!(after, (false, false), "the channel and its hurt over");
}

/// Thistle Tea's "interruption stops charging and releases only the
/// partial charge at expiry": interrupted after a tick, its user hurts no
/// more, and the hurt, run out, lets loose that tick's worth.
#[tokio::test]
async fn an_interrupted_death_ray_charges_no_more_and_lets_its_part_loose() {
    let (realm, _a, ga) = setup().await;
    let target = dummy();
    let (amount, hurt_at_interrupt, hurt_after, kept, lost_before, lost) =
        on_map(&realm, move |m| {
            steady(m, ga, target);
            let amount = channel(m, ga, target);
            let full = health(m, target);
            pass(m, 1000);
            pass(m, 500);
            spells::interrupt(m, ga);
            let at_interrupt = hurt(m, ga);
            let kept = holder(m, ga, DEATH_RAY_HURT).is_some();
            pass(m, 500);
            pass(m, 1000);
            let lost_before = full - health(m, target);
            pass(m, 1000);
            (
                amount,
                at_interrupt,
                hurt(m, ga),
                kept,
                lost_before,
                full - health(m, target),
            )
        })
        .await;
    assert_eq!(hurt_at_interrupt, amount, "a tick");
    assert!(kept, "the hurt stays on");
    assert_eq!(hurt_after, amount, "no more once interrupted");
    assert_eq!(lost_before, 0, "nothing loosed before it runs out");
    assert!(lost >= amount, "the one tick's worth: {lost} of {amount}");
}

/// Thistle Tea's "a different spell and an unstarted channel do not
/// charge": the hurt alone, or under another channel, costs nothing and
/// lets nothing loose.
#[tokio::test]
async fn the_death_rays_hurt_charges_only_under_its_channel() {
    let (realm, _a, ga) = setup().await;
    let target = dummy();
    let out = on_map(&realm, move |m| {
        let mut out = Vec::new();
        for other_channel in [false, true] {
            steady(m, ga, target);
            if other_channel {
                if let Some(c) = m.caster_mut(ga) {
                    c.casting = None;
                    c.gcd_until = None;
                }
                assert!(spells::cast(
                    m,
                    ga,
                    OTHER_CHANNEL,
                    Targets {
                        unit: Some(target),
                        ..Default::default()
                    },
                    None
                ));
            }
            let full = health(m, target);
            spells::cast_triggered(m, ga, DEATH_RAY_HURT, Some(ga));
            assert!(holder(m, ga, DEATH_RAY_HURT).is_some());
            for _ in 0..4 {
                pass(m, 1000);
            }
            out.push((
                hurt(m, ga),
                full - health(m, target),
                holder(m, ga, DEATH_RAY_HURT).is_some(),
            ));
        }
        out
    })
    .await;
    assert_eq!(out, vec![(0, 0, false), (0, 0, false)]);
}

/// Thistle Tea's "expiry uses the current selection and needs a target":
/// the charge goes at whoever its user has selected as it runs out, and
/// nowhere with no one selected.
#[tokio::test]
async fn the_death_ray_lets_loose_at_its_users_target_of_the_moment() {
    let (realm, mut a, ga) = setup().await;
    a.sees(other()).await;
    let (first, second, none) = on_map(&realm, move |m| {
        let mut out = Vec::new();
        for aim in [other(), Guid::zero()] {
            steady(m, ga, dummy());
            channel(m, ga, dummy());
            let full = (health(m, dummy()), health(m, other()));
            pass(m, 1000);
            m.players
                .get_mut(&ga)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_TARGET, aim);
            for _ in 0..3 {
                pass(m, 1000);
            }
            out.push((
                full.0 - health(m, dummy()),
                full.1 - health(m, other()),
                m.players[&ga].gadgets.death_ray,
            ));
        }
        (out[0], out[1], out[1].0 + out[1].1)
    })
    .await;
    assert_eq!(first.0, 0, "not at the first aim");
    assert!(first.1 > 0, "at the target of the moment");
    assert_eq!(second.2, 0, "the charge spent");
    assert_eq!(none, 0, "no one selected, nothing loosed");
}

/// Thistle Tea's "school immunity prevents both self damage and
/// accumulation": immune, its user takes nothing, and nothing is loosed.
#[tokio::test]
async fn an_immune_death_ray_user_takes_and_charges_nothing() {
    let (realm, _a, ga) = setup().await;
    let target = dummy();
    let (hurt, lost) = on_map(&realm, move |m| {
        steady(m, ga, target);
        spells::cast_triggered(m, ga, IMMUNE, Some(ga));
        let full = health(m, target);
        begin(m, ga, target);
        for _ in 0..4 {
            pass(m, 1000);
        }
        (hurt(m, ga), full - health(m, target))
    })
    .await;
    assert_eq!((hurt, lost), (0, 0));
}

/// Thistle Tea's "death and manual removal discard the charge" and
/// "lethal charging damage cannot fire a discharge": its user dead (by
/// another's hand or the ray's own tick) or the hurt taken off, nothing
/// is loosed, then or later.
#[tokio::test]
async fn the_death_rays_charge_is_lost_with_its_user_or_its_hurt() {
    for how in ["killed", "removed", "lethal"] {
        let (realm, _a, ga) = setup().await;
        let target = dummy();
        let out = on_map(&realm, move |m| {
            steady(m, ga, target);
            let full = health(m, target);
            channel(m, ga, target);
            if how == "lethal" {
                m.players
                    .get_mut(&ga)
                    .unwrap()
                    .fields
                    .set_u32(UNIT_FIELD_HEALTH, 1);
            }
            pass(m, 1000);
            match how {
                "killed" => crate::game::map::combat::kill(m, target, ga),
                "removed" => auras::remove_spell(m, ga, DEATH_RAY_HURT, None),
                _ => {}
            }
            for _ in 0..4 {
                pass(m, 1000);
            }
            (
                full - health(m, target),
                holder(m, ga, DEATH_RAY_HURT).is_some(),
                m.players[&ga].gadgets.death_ray,
                m.players[&ga].death == crate::game::map::death::Death::Alive,
            )
        })
        .await;
        let alive = how == "removed";
        assert_eq!(out, (0, false, 0, alive), "{how}");
    }
}

/// Thistle Tea's "refresh starts a fresh charge": a channel begun again
/// charges from nothing, and lets loose its own ticks alone.
#[tokio::test]
async fn a_death_ray_begun_again_charges_afresh() {
    let (realm, _a, ga) = setup().await;
    let target = dummy();
    let (first, second, charges, lost) = on_map(&realm, move |m| {
        steady(m, ga, target);
        let first = channel(m, ga, target);
        pass(m, 1000);
        let charged = m.players[&ga].gadgets.death_ray;
        spells::interrupt(m, ga);
        let second = channel(m, ga, target);
        let at_start = m.players[&ga].gadgets.death_ray;
        let full = health(m, target);
        for _ in 0..3 {
            pass(m, 1000);
        }
        let before_last = m.players[&ga].gadgets.death_ray;
        pass(m, 1000);
        (
            first,
            second,
            (charged, at_start, before_last),
            full - health(m, target),
        )
    })
    .await;
    assert_eq!(charges.0, first, "the first channel's tick");
    assert_eq!(charges.1, 0, "begun again from nothing");
    assert_eq!(charges.2, 3 * second, "its own ticks alone");
    assert!(lost >= 4 * second, "its own four: {lost} of {second}");
}

// The control devices, the dispenser and the net.

/// Each use of a gadget at index 0, `n` times: the markers of `outcomes`
/// it left on the user and on the target, counted by spell and caster.
fn uses(
    m: &mut Map,
    g: Guid,
    target: Guid,
    (gadget, index): (u32, usize),
    item: Option<Guid>,
    outcomes: &[u32],
    n: usize,
) -> Vec<(u32, Guid, Guid)> {
    let s = template(m, gadget);
    let mut seen = Vec::new();
    for _ in 0..n {
        item_scripts::effect(m, g, target, &s, index, item);
        let on: &[Guid] = if g == target { &[g] } else { &[g, target] };
        for on in on.iter().copied() {
            for (spell, caster) in markers(m, on, outcomes) {
                seen.push((spell, caster, on));
            }
        }
        clear(m, &[g, target], outcomes);
    }
    seen
}

fn count(seen: &[(u32, Guid, Guid)], what: (u32, Guid, Guid)) -> usize {
    seen.iter().filter(|s| **s == what).count()
}

/// Thistle Tea's "the remote selects possession, rooting, or a
/// target-owned enrage with equal weights": one of three each use, a
/// third each, the enrage the machine's own on itself.
#[tokio::test]
async fn the_universal_remote_controls_roots_or_enrages_a_third_each() {
    let (realm, _a, ga) = setup().await;
    let t = dummy();
    let item = guid::item(1);
    let seen = on_map(&realm, move |m| {
        uses(m, ga, t, (REMOTE, 0), Some(item), &REMOTE_OUTCOMES, 150)
    })
    .await;
    assert_eq!(seen.len(), 150, "one outcome a use: {seen:?}");
    for what in [
        (REMOTE_OUTCOMES[0], ga, t),
        (REMOTE_OUTCOMES[1], ga, t),
        (REMOTE_OUTCOMES[2], t, t),
    ] {
        let n = count(&seen, what);
        assert!((25..=75).contains(&n), "{what:?}: {n} of 150");
    }
}

/// Thistle Tea's "the cap preserves failures and reversed control as
/// distinct outcomes": one in six nothing, one in six the target controls
/// its user, four in six the user controls the target; shapeshifted, the
/// reversal is nothing instead.
#[tokio::test]
async fn the_mind_control_cap_fails_turns_or_controls() {
    let (realm, _a, ga) = setup().await;
    let t = dummy();
    let item = guid::item(1);
    let (plain, shifted, form) = on_map(&realm, move |m| {
        let plain = uses(m, ga, t, (CAP, 0), Some(item), &[MIND_CONTROL], 240);
        spells::cast_triggered(m, ga, SHIFT, Some(ga));
        let form = auras::form(m, ga);
        let shifted = uses(m, ga, t, (CAP, 0), Some(item), &[MIND_CONTROL], 240);
        (plain, shifted, form)
    })
    .await;
    assert_ne!(form, 0, "shapeshifted");
    let control = count(&plain, (MIND_CONTROL, ga, t));
    let reversed = count(&plain, (MIND_CONTROL, t, ga));
    let nothing = 240 - control - reversed;
    assert_eq!(control + reversed, plain.len());
    assert!((120..=200).contains(&control), "four in six: {control}");
    assert!((15..=70).contains(&reversed), "one in six: {reversed}");
    assert!((15..=70).contains(&nothing), "one in six: {nothing}");
    assert_eq!(count(&shifted, (MIND_CONTROL, t, ga)), 0, "never turned");
    let control = count(&shifted, (MIND_CONTROL, ga, t));
    assert!(
        (120..=200).contains(&control),
        "still four in six: {control}"
    );
}

/// Thistle Tea's "control device effects execute only once and ignore
/// dead recipients": the remote and the cap do nothing from their second
/// dummy effect, nor at a dead target.
#[tokio::test]
async fn the_control_devices_work_once_a_use_and_not_on_the_dead() {
    let (realm, _a, ga) = setup().await;
    let t = dummy();
    let item = guid::item(1);
    let all: Vec<u32> = REMOTE_OUTCOMES
        .iter()
        .copied()
        .chain([MIND_CONTROL])
        .collect();
    let (second, dead) = on_map(&realm, move |m| {
        let mut second = Vec::new();
        for gadget in [REMOTE, CAP] {
            second.extend(uses(m, ga, t, (gadget, 1), Some(item), &all, 30));
        }
        crate::game::map::combat::kill(m, ga, t);
        let mut dead = Vec::new();
        for gadget in [REMOTE, CAP] {
            dead.extend(uses(m, ga, t, (gadget, 0), Some(item), &all, 30));
        }
        (second, dead)
    })
    .await;
    assert_eq!(second, vec![], "the second effect does nothing");
    assert_eq!(dead, vec![], "nothing on the dead");
}

/// Thistle Tea's "the dispenser selects a summon or malfunction only for
/// an item cast": a bomb nine uses in ten, a malfunction the tenth, both
/// on its user; nothing cast without the item, nor at another.
#[tokio::test]
async fn the_goblin_bomb_dispenser_summons_or_malfunctions_from_its_item() {
    let (realm, _a, ga) = setup().await;
    let item = guid::item(1);
    let t = dummy();
    let outcomes = [MALFUNCTION, SUMMON_BOMB];
    let (used, no_item, at_another) = on_map(&realm, move |m| {
        (
            uses(m, ga, ga, (DISPENSER, 0), Some(item), &outcomes, 200),
            uses(m, ga, ga, (DISPENSER, 0), None, &outcomes, 20),
            uses(m, ga, t, (DISPENSER, 0), Some(item), &outcomes, 20),
        )
    })
    .await;
    assert_eq!(used.len(), 200, "one a use");
    let bombs = count(&used, (SUMMON_BOMB, ga, ga));
    let malfunctions = count(&used, (MALFUNCTION, ga, ga));
    assert_eq!(bombs + malfunctions, 200);
    assert!(
        (5..=40).contains(&malfunctions),
        "one in ten: {malfunctions}"
    );
    assert_eq!((no_item, at_another), (vec![], vec![]));
}

/// Thistle Tea's "a net requests one caster-owned outcome from its first
/// dummy effect": its user's at the target, its user netted one in ten, a
/// net with a backfire one in ten, the target netted the rest; nothing
/// from its second.
#[tokio::test]
async fn the_net_o_matic_nets_from_its_first_effect_alone() {
    let (realm, _a, ga) = setup().await;
    let t = dummy();
    let item = guid::item(1);
    let (first, second) = on_map(&realm, move |m| {
        (
            uses(m, ga, t, (NET, 0), Some(item), &NET_OUTCOMES, 200),
            uses(m, ga, t, (NET, 1), Some(item), &NET_OUTCOMES, 20),
        )
    })
    .await;
    assert_eq!(first.len(), 200, "one a use");
    let [own, backfire, netted] = NET_OUTCOMES.map(|s| count(&first, (s, ga, t)));
    assert_eq!(own + backfire + netted, 200, "all the user's at the target");
    assert!((5..=40).contains(&own), "one in ten: {own}");
    assert!((5..=40).contains(&backfire), "one in ten: {backfire}");
    assert_eq!(second, vec![]);
}

// Disenchanting.

/// Thistle Tea's "requires a living enchanter without unclaimed
/// materials" and "validates item-targeted disenchant through shared cast
/// admission": an enchanter, alive, at an item that may be disenchanted;
/// not with a disenchanted item's dust still in its window, which would
/// be lost, the item kept.
#[tokio::test]
async fn a_disenchant_wants_a_living_enchanter_with_no_window_open() {
    const LOW_CASTLEVEL: u8 = 0x2c;
    const CASTER_DEAD: u8 = 0x13;
    const ALREADY_OPEN: u8 = 0x06;
    const CANT_BE_DISENCHANTED: u8 = 0x0c;
    let (realm, mut a, ga) = setup().await;
    let out = on_map(&realm, move |m| {
        let why = |m: &mut Map, item: Guid| {
            let s = template(m, DISENCHANT);
            let mods = spells::mods::for_spell(m, ga, &s);
            let t = Targets {
                item: Some(item),
                ..Default::default()
            };
            spells::checks::check(m, ga, &s, &t, None, false, &mods)
        };
        crate::game::map::inventory::add_item(m, ga, BRACER, 2, false).unwrap();
        crate::game::map::inventory::add_item(m, ga, DUST, 1, false).unwrap();
        let p = &m.players[&ga];
        let bracers: Vec<Guid> = p
            .items
            .iter()
            .filter(|i| i.entry == BRACER)
            .map(|i| i.guid)
            .collect();
        let dust = p.items.iter().find(|i| i.entry == DUST).unwrap().guid;
        let unskilled = why(m, bracers[0]);
        let p = m.players.get_mut(&ga).unwrap();
        skills::learn_skill(p, ENCHANTING, 1);
        let ready = why(m, bracers[0]);
        let not_one = why(m, dust);
        crate::game::map::combat::kill(m, dummy(), ga);
        let dead = why(m, bracers[0]);
        crate::game::map::death::resurrect(m, ga, 1.0);
        (unskilled, ready, not_one, dead, bracers)
    })
    .await;
    assert_eq!(out.0, Some(LOW_CASTLEVEL), "no enchanter");
    assert_eq!(out.1, None);
    assert_eq!(out.2, Some(CANT_BE_DISENCHANTED), "dust");
    assert_eq!(out.3, Some(CASTER_DEAD));
    let bracers = [out.4[0], out.4[1]];
    let disenchant = move |m: &mut Map, item: Guid| {
        let s = template(m, DISENCHANT);
        let t = Targets {
            item: Some(item),
            ..Default::default()
        };
        professions::execute(m, ga, &s, &t, None);
    };
    // One disenchanted, its window open: not another.
    a.clear();
    let (window, kept, dust_left) = on_map(&realm, move |m| {
        disenchant(m, bracers[1]);
        let window = m.players[&ga].looting;
        disenchant(m, bracers[0]);
        let kept = m.players[&ga].item_by_guid(bracers[0]).is_some();
        let dust_left = window.and_then(|w| loot::loot_ref(m, w).map(|l| l.items.len()));
        (window, kept, dust_left)
    })
    .await;
    assert_eq!(window, Some(bracers[1]), "the first one's window");
    assert_eq!(cast_failure(&mut a, DISENCHANT).await, ALREADY_OPEN);
    assert!(kept, "the second bracer kept");
    assert_eq!(dust_left, Some(1), "the first one's dust still there");
    // Its window closed: the second goes.
    let gone = on_map(&realm, move |m| {
        loot::release(m, ga, bracers[1]);
        disenchant(m, bracers[0]);
        m.players[&ga].item_by_guid(bracers[0]).is_none()
    })
    .await;
    assert!(gone);
}

/// The failure `c` was told of for `spell` (`SMSG_CAST_RESULT`'s reason).
async fn cast_failure(c: &mut Client, spell: u32) -> u8 {
    let frame = c
        .wait_frame("a failed SMSG_CAST_RESULT", spells::SMSG_CAST_RESULT, |f| {
            f[..4] == spell.to_le_bytes() && f[4] == 2
        })
        .await;
    frame[5]
}

// Gathering and unlearning.

/// Thistle Tea's "preserves trained rank and slot while respecting caps and
/// failed rolls": a gathering skill's rise is one point, its rank's most
/// and its slot kept; none at the rank's most, none for a skill not known,
/// none from a failed roll.
#[tokio::test]
async fn a_gathering_skill_rises_a_point_within_its_rank() {
    let (realm, _a, ga) = setup().await;
    let out = on_map(&realm, move |m| {
        let slot = |m: &Map| {
            (0..128u16).find(|i| {
                m.players[&ga].fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == MINING
            })
        };
        let state = |m: &Map| {
            let p = &m.players[&ga];
            (
                skills::value(p, MINING),
                skills::max_value(p, MINING),
                slot(m),
            )
        };
        skills::learn_skill(m.players.get_mut(&ga).unwrap(), MINING, 1);
        let trained = state(m);
        let rose = professions::try_raise(m, ga, MINING, 100.0);
        let after = state(m);
        let failed = professions::try_raise(m, ga, MINING, 0.0);
        let after_failed = state(m);
        skills::set_value(m.players.get_mut(&ga).unwrap(), MINING, 75);
        let capped = professions::try_raise(m, ga, MINING, 100.0);
        let at_cap = state(m);
        let unknown = professions::try_raise(m, ga, HERBALISM, 100.0);
        let herbalism = skills::value(&m.players[&ga], HERBALISM);
        (
            trained,
            rose,
            after,
            failed,
            after_failed,
            capped,
            at_cap,
            unknown,
            herbalism,
        )
    })
    .await;
    let slot = out.0.2;
    assert!(slot.is_some());
    assert_eq!(out.0, (Some(1), Some(75), slot), "trained: 1 of 75");
    assert!(out.1);
    assert_eq!(out.2, (Some(2), Some(75), slot), "a point, the rank kept");
    assert!(!out.3, "a failed roll");
    assert_eq!(out.4, out.2);
    assert!(!out.5, "at the rank's most");
    assert_eq!(out.6, (Some(75), Some(75), slot));
    assert!(!out.7, "not known");
    assert_eq!(out.8, None);
}

/// Thistle Tea's "rejects an unknown skill": unlearning a profession not
/// known changes nothing; one known goes, and the other stays.
#[tokio::test]
async fn unlearning_a_profession_not_known_changes_nothing() {
    const SKINNING: u32 = 393;
    let (realm, a, ga) = setup().await;
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        skills::learn_skill(p, MINING, 1);
        skills::set_value(p, MINING, 40);
        skills::learn_skill(p, SKINNING, 1);
    })
    .await;
    // Herbalism, not known, then skinning: once skinning is gone, the
    // first was handled.
    a.send(CMSG_UNLEARN_SKILL {
        skill: Skill::Herbalism,
    })
    .await;
    a.send(CMSG_UNLEARN_SKILL {
        skill: Skill::Skinning,
    })
    .await;
    let gone = harness_wait(&realm, move |m| {
        skills::value(&m.players[&ga], SKINNING).is_none()
    })
    .await;
    assert!(gone, "skinning forgotten");
    let (mining, herbalism) = on_map(&realm, move |m| {
        let p = &m.players[&ga];
        (skills::value(p, MINING), skills::value(p, HERBALISM))
    })
    .await;
    assert_eq!(mining, Some(40), "mining stays");
    assert_eq!(herbalism, None, "no herbalism made");
}

/// Whether `f` comes true on the map within a second.
async fn harness_wait(
    realm: &Arc<Realm>,
    f: impl Fn(&Map) -> bool + Send + Sync + 'static,
) -> bool {
    let f = Arc::new(f);
    for _ in 0..100 {
        let f = f.clone();
        if on_map(realm, move |m| f(m)).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}
