//! Phase 3's auras on the real data, Thistle Tea's `AuraTest` ported (the
//! class scripts and stacking rules): Vampiric Embrace, Magic Absorption,
//! Illumination and Furor; ranks, exclusive kinds and casters replacing
//! or standing beside each other; Blink's immunities; Ignite shared.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::procs::{self, Event};
use crate::game::map::spells;
use crate::game::map::{Map, auras};

const VAMPIRIC_EMBRACE: u32 = 15286;
const SHADOW_WORD_PAIN: u32 = 589;
const FIREBALL: u32 = 133;
const MAGIC_ABSORPTION: u32 = 29447;
const FROSTBOLT: u32 = 116;
const ILLUMINATION: u32 = 20210;
const HOLY_LIGHT: u32 = 635;
const FUROR: u32 = 17061;
const CAT_FORM: u32 = 768;
const ARCANE_INTELLECT: u32 = 1459;
const ARCANE_INTELLECT_2: u32 = 1460;
const FROST_ARMOR: u32 = 168;
const MAGE_ARMOR: u32 = 6117;
const SUNDER_ARMOR: u32 = 7386;
const SERPENT_STING: u32 = 1978;
const VIPER_STING: u32 = 3034;
const BLINK: u32 = 1953;
const FROST_NOVA: u32 = 122;
const HAMMER_OF_JUSTICE: u32 = 853;
const IGNITE_TALENT: u32 = 12848;
const IGNITE: u32 = 12654;

/// A made-up ward against every school's spells.
const RESIST_ALL: u32 = 96_300;

/// The holders of `g`: each spell and caster, in order.
fn holders(m: &Map, g: Guid) -> Vec<(u32, Guid)> {
    auras::auras(m, g)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| !h.passive)
                .map(|h| (h.spell, h.caster))
                .collect()
        })
        .unwrap_or_default()
}

/// `id` from `caster` on `target`: whether it went on.
fn try_apply(m: &mut Map, caster: Guid, target: Guid, id: u32) -> Result<(), auras::Refused> {
    let data = m.realm.data.clone();
    let s = data.spells.get(id).unwrap().clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, 60)))
        .collect();
    auras::apply_spell(m, caster, target, &s, &effects, auras::Context::default())
}

/// Shadow damage `damage` from `actor`'s Shadow Word: Pain (or `spell`)
/// taken by `victim` in a tick.
fn taken(actor: Guid, victim: Guid, spell: u32, school: u32, damage: u32) -> Event {
    Event {
        actor,
        target: victim,
        target_flags: procs::PROC_TAKEN_PERIODIC,
        ex: procs::EX_NORMAL_HIT,
        spell: Some(spell),
        damage,
        school,
        ..Default::default()
    }
}

/// Thistle Tea's Vampiric Embrace: the priest heals a fifth of the shadow
/// damage they deal its target (10 of 50, 100 of 500), from a Shadow Word:
/// Pain's tick too; not another's shadow damage, nor their own fire.
#[tokio::test]
#[ignore = "extended"]
async fn vampiric_embrace_heals_a_fifth_of_the_priests_shadow_damage() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, priest, map) = player(&realm, "P3Q1", "Embraceone", Race::Human, Class::Priest).await;
    let (_d, other, _) = player(&realm, "P3Q2", "Embracetwo", Race::Human, Class::Priest).await;
    let got = map
        .query(move |m| {
            ready(m, priest, 60, &[]);
            ready(m, other, 60, &[]);
            let kobold = dummy(m, priest, 60, 5.0);
            apply(m, priest, kobold, VAMPIRIC_EMBRACE);
            let healed = |m: &mut Map, e: Event| {
                set_health(m, priest, 100);
                procs::trigger(m, e);
                health(m, priest) - 100
            };
            let fifty = healed(m, taken(priest, kobold, SHADOW_WORD_PAIN, 32, 50));
            let five_hundred = healed(m, taken(priest, kobold, SHADOW_WORD_PAIN, 32, 500));
            let others = healed(m, taken(other, kobold, SHADOW_WORD_PAIN, 32, 100));
            let fire = healed(m, taken(priest, kobold, FIREBALL, 4, 100));
            // A real tick of Shadow Word: Pain.
            apply(m, priest, kobold, SHADOW_WORD_PAIN);
            set_health(m, priest, 100);
            let before = health(m, kobold);
            tick_now(m, kobold, SHADOW_WORD_PAIN);
            let dealt = before - health(m, kobold);
            let tick = health(m, priest) - 100;
            (fifty, five_hundred, others, fire, dealt, tick)
        })
        .await
        .unwrap();
    let (fifty, five_hundred, others, fire, dealt, tick) = got;
    assert_eq!((fifty, five_hundred), (10, 100));
    assert_eq!((others, fire), (0, 0), "not another's, nor fire");
    assert!(dealt > 0);
    assert_eq!(tick, dealt / 5, "a fifth of the tick's {dealt}");
}

/// Thistle Tea's Magic Absorption: a spell resisted gives back 5% of the
/// mage's mana, at most once a second.
#[tokio::test]
#[ignore = "extended"]
async fn magic_absorption_gives_mana_for_a_resisted_spell_once_a_second() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let ward = made_up(RESIST_ALL, 6, auras::AURA_MOD_RESIST_CHANCE, 0x7F, 1_000);
        d.spells.spells.insert(RESIST_ALL, ward);
    });
    let (_c, mage, map) = player(&realm, "P3Q3", "Absorbone", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            apply(m, mage, mage, MAGIC_ABSORPTION);
            apply(m, mage, mage, RESIST_ALL);
            let kobold = dummy(m, mage, 60, 5.0);
            let gained = |m: &mut Map, after: u64| {
                m.now += Duration::from_millis(after);
                set_power(m, mage, 0, 100);
                spells::cast_triggered(m, kobold, FROSTBOLT, Some(mage));
                power(m, mage, 0) - 100
            };
            let max = m
                .unit_fields(mage)
                .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXPOWER1));
            (max / 20, (gained(m, 0), gained(m, 999), gained(m, 1)))
        })
        .await
        .unwrap();
    let (share, gains) = got;
    assert!(share > 0);
    assert_eq!(gains, (share, 0, share), "5% of the mana, once a second");
}

/// Thistle Tea's Illumination: a crit Holy Light gives back its base mana
/// cost.
#[tokio::test]
#[ignore = "extended"]
async fn illumination_gives_back_a_crit_heals_base_cost() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&ILLUMINATION) {
            s.proc_chance = 100;
        }
    });
    let (_c, paladin, map) = player(&realm, "P3Q4", "Lightone", Race::Human, Class::Paladin).await;
    let got = map
        .query(move |m| {
            ready(m, paladin, 60, &[]);
            apply(m, paladin, paladin, ILLUMINATION);
            let cost = m.realm.data.spells.get(HOLY_LIGHT).unwrap().mana_cost;
            let gained = |m: &mut Map, ex: u32| {
                set_power(m, paladin, 0, 100);
                procs::trigger(
                    m,
                    Event {
                        actor: paladin,
                        target: paladin,
                        actor_flags: procs::PROC_DONE_POSITIVE_SPELL,
                        ex,
                        spell: Some(HOLY_LIGHT),
                        school: 2,
                        ..Default::default()
                    },
                );
                power(m, paladin, 0) - 100
            };
            (
                cost,
                gained(m, procs::EX_NORMAL_HIT),
                gained(m, procs::EX_CRITICAL_HIT),
            )
        })
        .await
        .unwrap();
    assert_eq!(got.0, 35);
    assert_eq!((got.1, got.2), (0, 35));
}

/// Thistle Tea's Furor: at its full rank a druid going into cat form has
/// its 40 energy.
#[tokio::test]
#[ignore = "extended"]
async fn furor_gives_energy_going_into_cat_form() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, druid, map) = player(&realm, "P3Q5", "Furorone", Race::NightElf, Class::Druid).await;
    let got = map
        .query(move |m| {
            ready(m, druid, 60, &[]);
            apply(m, druid, druid, FUROR);
            apply(m, druid, druid, CAT_FORM);
            (auras::form(m, druid), power(m, druid, 3))
        })
        .await
        .unwrap();
    assert_eq!(got, (auras::FORM_CAT, 40));
}

/// Thistle Tea's ranks and exclusive kinds: Arcane Intellect's second rank
/// takes the first's place, and the first doesn't go on under the second;
/// Mage Armor takes Frost Armor's place, another chain of the same kind.
#[tokio::test]
#[ignore = "extended"]
async fn a_higher_rank_replaces_a_lower_and_an_armor_another() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3Q6", "Rankone", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            try_apply(m, mage, mage, ARCANE_INTELLECT).unwrap();
            try_apply(m, mage, mage, ARCANE_INTELLECT_2).unwrap();
            let up = holders(m, mage);
            let down = try_apply(m, mage, mage, ARCANE_INTELLECT);
            let after_down = holders(m, mage);
            auras::remove_where(m, mage, |h| !h.passive);
            try_apply(m, mage, mage, FROST_ARMOR).unwrap();
            try_apply(m, mage, mage, MAGE_ARMOR).unwrap();
            (up, down, after_down, holders(m, mage))
        })
        .await
        .unwrap();
    assert_eq!(got.0, [(ARCANE_INTELLECT_2, mage)]);
    assert_eq!(got.1, Err(auras::Refused::Bounced));
    assert_eq!(got.2, [(ARCANE_INTELLECT_2, mage)]);
    assert_eq!(got.3, [(MAGE_ARMOR, mage)]);
}

/// Thistle Tea's casters: a plain buff from a second caster takes the
/// first's place; two priests' Shadow Word: Pains stand side by side;
/// Sunder Armor's stacks are anyone's, three from two warriors; and a
/// hunter's stings replace each other, another hunter's beside them.
#[tokio::test]
#[ignore = "extended"]
async fn casters_replace_stand_beside_or_share_by_the_spell() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, a, map) = player(&realm, "P3Q7", "Casterone", Race::Human, Class::Mage).await;
    let (_d, b, _) = player(&realm, "P3Q8", "Castertwo", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, a, 60, &[]);
            ready(m, b, 60, &[]);
            let kobold = dummy(m, a, 60, 5.0);
            try_apply(m, a, kobold, ARCANE_INTELLECT).unwrap();
            try_apply(m, b, kobold, ARCANE_INTELLECT).unwrap();
            let buff = holders(m, kobold);
            auras::remove_where(m, kobold, |_| true);
            try_apply(m, a, kobold, SHADOW_WORD_PAIN).unwrap();
            try_apply(m, b, kobold, SHADOW_WORD_PAIN).unwrap();
            let dots = holders(m, kobold);
            auras::remove_where(m, kobold, |_| true);
            for c in [a, a, b] {
                try_apply(m, c, kobold, SUNDER_ARMOR).unwrap();
            }
            let sunder = auras::auras(m, kobold).map(|x| {
                x.holders
                    .iter()
                    .map(|h| (h.spell, h.stacks))
                    .collect::<Vec<_>>()
            });
            auras::remove_where(m, kobold, |_| true);
            try_apply(m, a, kobold, SERPENT_STING).unwrap();
            try_apply(m, a, kobold, VIPER_STING).unwrap();
            try_apply(m, b, kobold, SERPENT_STING).unwrap();
            (buff, dots, sunder, holders(m, kobold))
        })
        .await
        .unwrap();
    assert_eq!(got.0, [(ARCANE_INTELLECT, b)], "the second caster's buff");
    assert_eq!(
        got.1,
        [(SHADOW_WORD_PAIN, a), (SHADOW_WORD_PAIN, b)],
        "both DoTs"
    );
    assert_eq!(got.2, Some(vec![(SUNDER_ARMOR, 3)]), "one stack of three");
    assert_eq!(
        got.3,
        [(VIPER_STING, a), (SERPENT_STING, b)],
        "a sting a hunter"
    );
}

/// Thistle Tea's Blink: its immunities to stuns and roots take the root
/// and the stun already on the mage, and leave Blink's own.
#[tokio::test]
#[ignore = "extended"]
async fn blink_takes_the_roots_and_stuns_on_the_mage() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3Q9", "Blinkone", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            apply(m, kobold, mage, FROST_NOVA);
            apply(m, kobold, mage, HAMMER_OF_JUSTICE);
            let before = holders(m, mage).len();
            apply(m, mage, mage, BLINK);
            let held = auras::auras(m, mage)
                .is_some_and(|a| a.has(auras::AURA_MOD_ROOT) || a.has(auras::AURA_MOD_STUN));
            (before, held, holders(m, mage))
        })
        .await
        .unwrap();
    assert_eq!(got.0, 2);
    assert!(!got.1, "free");
    assert_eq!(got.2, [(BLINK, mage)]);
}

/// Thistle Tea's Ignite: one burn on a target, whoever crits. A second
/// mage's fire crit adds its share to the first mage's burn, a stack more,
/// and the burn stays the first mage's on its first rhythm; each tick
/// deals the shares summed, not again for each stack.
#[tokio::test]
#[ignore = "extended"]
async fn ignite_is_one_burn_the_first_mages_whoever_crits() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, a, map) = player(&realm, "P3Q10", "Igniteone", Race::Human, Class::Mage).await;
    let (_d, b, _) = player(&realm, "P3Q11", "Ignitetwo", Race::Human, Class::Mage).await;
    let got = map
        .query(move |m| {
            ready(m, a, 60, &[]);
            ready(m, b, 60, &[]);
            apply(m, a, a, IGNITE_TALENT);
            apply(m, b, b, IGNITE_TALENT);
            let kobold = dummy(m, a, 60, 5.0);
            let crit = |m: &mut Map, mage: Guid| {
                procs::trigger(
                    m,
                    Event {
                        actor: mage,
                        target: kobold,
                        actor_flags: procs::PROC_DONE_NEGATIVE_SPELL,
                        ex: procs::EX_CRITICAL_HIT,
                        spell: Some(FIREBALL),
                        damage: 100,
                        school: 4,
                        ..Default::default()
                    },
                );
            };
            crit(m, a);
            let first_tick = holder(m, kobold, IGNITE)
                .and_then(|h| h.effects[0].next_tick)
                .unwrap();
            m.now += Duration::from_millis(500);
            crit(m, b);
            let h = holder(m, kobold, IGNITE).unwrap();
            let burns = holders(m, kobold).len();
            let before = health(m, kobold);
            tick_now(m, kobold, IGNITE);
            let dealt = before - health(m, kobold);
            (
                burns,
                h.caster == a,
                h.stacks,
                h.effects[0].amount,
                h.effects[0].next_tick == Some(first_tick),
                dealt,
            )
        })
        .await
        .unwrap();
    let (burns, firsts, stacks, amount, rhythm, dealt) = got;
    assert_eq!(burns, 1, "one burn");
    assert!(firsts, "the first mage's");
    assert_eq!((stacks, amount), (2, 40));
    assert!(rhythm, "on its first rhythm");
    assert_eq!(dealt, 40);
}

const BESTIAL_WRATH: u32 = 19574;
const BESTIAL_WRATH_IMMUNITIES: [u32; 4] = [24395, 24396, 24397, 26592];

/// Thistle Tea's boosts: Bestial Wrath's immunities come with it, the
/// hunter's, on the beast it's on; they go as it goes (taken off, run out,
/// or the beast dead); one taken off isn't back until Bestial Wrath is
/// cast again.
#[tokio::test]
#[ignore = "extended"]
async fn bestial_wraths_immunities_come_and_go_with_it() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, hunter, map) = player(&realm, "P3Q12", "Wrathone", Race::Dwarf, Class::Hunter).await;
    let got = map
        .query(move |m| {
            ready(m, hunter, 60, &[]);
            let beast = dummy(m, hunter, 60, 5.0);
            let boosts = |m: &Map| {
                holders(m, beast)
                    .into_iter()
                    .filter(|(s, _)| BESTIAL_WRATH_IMMUNITIES.contains(s))
                    .collect::<Vec<_>>()
            };
            apply(m, hunter, beast, BESTIAL_WRATH);
            let on = boosts(m);
            auras::remove_spell(m, beast, BESTIAL_WRATH_IMMUNITIES[0], None);
            pass(m, 1_000);
            let one_off = boosts(m).len();
            apply(m, hunter, beast, BESTIAL_WRATH);
            let back = boosts(m).len();
            auras::remove_spell(m, beast, BESTIAL_WRATH, None);
            let removed = boosts(m).len();
            apply(m, hunter, beast, BESTIAL_WRATH);
            pass(m, 18_000);
            let ran_out = (has(m, beast, BESTIAL_WRATH), boosts(m).len());
            apply(m, hunter, beast, BESTIAL_WRATH);
            auras::clear_on_death(m, beast);
            let dead = boosts(m).len();
            (on, one_off, back, removed, ran_out, dead)
        })
        .await
        .unwrap();
    let (on, one_off, back, removed, ran_out, dead) = got;
    assert_eq!(
        on,
        BESTIAL_WRATH_IMMUNITIES.map(|s| (s, hunter)).to_vec(),
        "all four, the hunter's"
    );
    assert_eq!(one_off, 3, "one taken off stays off");
    assert_eq!(back, 4, "back with Bestial Wrath again");
    assert_eq!(removed, 0);
    assert_eq!(ran_out, (false, 0));
    assert_eq!(dead, 0);
}

const SHADOWGUARD: [(u32, u32); 6] = [
    (18137, 28377),
    (19308, 28378),
    (19309, 28379),
    (19310, 28380),
    (19311, 28381),
    (19312, 28382),
];

/// A melee blow from `actor` landed on `target`.
fn blow(actor: Guid, target: Guid, ex: u32, damage: u32) -> Event {
    Event {
        actor,
        target,
        actor_flags: procs::PROC_DONE_MELEE,
        target_flags: procs::PROC_TAKEN_MELEE,
        ex,
        damage,
        school: 1,
        ..Default::default()
    }
}

/// Thistle Tea's `ProcSpell` for Shadowguard: each rank strikes the one
/// who hit the priest with its own rank's bolt.
#[tokio::test]
#[ignore = "extended"]
async fn shadowguard_strikes_back_at_every_rank() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, priest, map) =
        player(&realm, "P3Q13", "Guardone", Race::Human, Class::Priest).await;
    let hurt = map
        .query(move |m| {
            ready(m, priest, 60, &[]);
            let mut hurt = Vec::new();
            for (rank, _) in SHADOWGUARD {
                let kobold = dummy(m, priest, 60, 3.0);
                apply(m, priest, priest, rank);
                procs::trigger(m, blow(kobold, priest, procs::EX_NORMAL_HIT, 10));
                hurt.push(health(m, kobold) < 1_000_000);
                auras::remove_spell(m, priest, rank, None);
            }
            hurt
        })
        .await
        .unwrap();
    assert_eq!(hurt, [true; 6]);
    for (_, bolt) in SHADOWGUARD {
        assert!(saw_go(&mut c, bolt).await, "{bolt}");
    }
}

const MANA_DRAIN_TRIGGER: u32 = 27522;
const MANA_DRAIN_ENERGIZE: u32 = 29471;
const MANA_DRAIN_LEECH: u32 = 27526;

/// Thistle Tea's Mana Drain: a melee or ranged blow that lands gives its
/// wearer mana and drains its target, two spells; one that misses or is
/// dodged, or a spell, neither. With charges and a cooldown, the two
/// spend one charge and share one cooldown.
#[tokio::test]
#[ignore = "extended"]
async fn mana_drain_gives_and_drains_on_a_blow_that_lands() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&MANA_DRAIN_TRIGGER) {
            s.proc_charges = 2;
        }
        d.spells.proc_events.insert(
            MANA_DRAIN_TRIGGER,
            crate::game::data::spells::ProcEvent {
                cooldown: 1_000,
                ..Default::default()
            },
        );
    });
    let (mut c, paladin, map) =
        player(&realm, "P3Q14", "Drainone", Race::Human, Class::Paladin).await;
    let got = map
        .query(move |m| {
            ready(m, paladin, 60, &[]);
            let kobold = dummy(m, paladin, 60, 3.0);
            apply(m, paladin, paladin, MANA_DRAIN_TRIGGER);
            let charges = |m: &Map| holder(m, paladin, MANA_DRAIN_TRIGGER).map(|h| h.charges);
            let mut out = Vec::new();
            for ex in [procs::EX_MISS, procs::EX_DODGE] {
                procs::trigger(m, blow(paladin, kobold, ex, 0));
                out.push(charges(m));
            }
            let mut spell = blow(paladin, kobold, procs::EX_NORMAL_HIT, 10);
            spell.actor_flags = procs::PROC_DONE_NEGATIVE_SPELL;
            procs::trigger(m, spell);
            out.push(charges(m));
            procs::trigger(m, blow(paladin, kobold, procs::EX_NORMAL_HIT, 10));
            out.push(charges(m));
            m.now += Duration::from_millis(999);
            procs::trigger(m, blow(paladin, kobold, procs::EX_NORMAL_HIT, 10));
            out.push(charges(m));
            m.now += Duration::from_millis(1);
            procs::trigger(m, blow(paladin, kobold, procs::EX_NORMAL_HIT, 10));
            out.push(charges(m));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        got,
        [Some(2), Some(2), Some(2), Some(1), Some(1), None],
        "nothing on a miss, a dodge or a spell; a charge a blow, a second apart"
    );
    assert!(saw_go(&mut c, MANA_DRAIN_ENERGIZE).await);
    assert!(saw_go(&mut c, MANA_DRAIN_LEECH).await);
}

const BLESSED_RECOVERY: [(u32, i32, u32); 3] =
    [(27811, 8, 27813), (27815, 16, 27817), (27816, 25, 27818)];

/// Thistle Tea's Blessed Recovery: a crit weapon blow taken gives each
/// rank's heal over time a third of its share of the damage a tick (8, 16
/// and 25 of 300), not a plain one, nor a spell's crit; the damage is
/// what the absorbs left; a fraction is rounded up as often as it is of
/// one; a killing blow gives none.
#[tokio::test]
#[ignore = "extended"]
async fn blessed_recovery_heals_a_share_of_a_crit_blow() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let ward = made_up(RESIST_ALL, 6, auras::AURA_SCHOOL_ABSORB, 1, 50);
        d.spells.spells.insert(RESIST_ALL, ward);
    });
    let (_c, priest, map) = player(&realm, "P3Q15", "Recoverone", Race::Human, Class::Priest).await;
    let got = map
        .query(move |m| {
            ready(m, priest, 60, &[]);
            let kobold = dummy(m, priest, 60, 3.0);
            let amount = |m: &mut Map, heal: u32| {
                let a = holder(m, priest, heal).map(|h| h.effects[0].amount);
                auras::remove_spell(m, priest, heal, None);
                a
            };
            let mut ranks = Vec::new();
            for (rank, _, heal) in BLESSED_RECOVERY {
                apply(m, priest, priest, rank);
                procs::trigger(m, blow(kobold, priest, procs::EX_CRITICAL_HIT, 300));
                ranks.push(amount(m, heal));
                auras::remove_spell(m, priest, rank, None);
            }
            let (rank, _, heal) = BLESSED_RECOVERY[2];
            apply(m, priest, priest, rank);
            procs::trigger(m, blow(kobold, priest, procs::EX_NORMAL_HIT, 300));
            let plain = amount(m, heal);
            let mut spell = blow(kobold, priest, procs::EX_CRITICAL_HIT, 300);
            spell.target_flags = procs::PROC_TAKEN_NEGATIVE_SPELL;
            spell.spell = Some(FROSTBOLT);
            procs::trigger(m, spell);
            let by_spell = amount(m, heal);
            // A third of 25% of 100: 8 or 9.
            let mut fractions = std::collections::BTreeSet::new();
            for _ in 0..64 {
                procs::trigger(m, blow(kobold, priest, procs::EX_CRITICAL_HIT, 100));
                fractions.insert(amount(m, heal));
            }
            // A real crit from the kobold of 400, fifty of it absorbed.
            if let Some(k) = m.creature_mut(kobold) {
                k.fields.set_f32(UNIT_FIELD_MINDAMAGE, 200.0);
                k.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 200.0);
            }
            apply(m, priest, priest, RESIST_ALL);
            let full = m
                .unit_fields(priest)
                .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXHEALTH));
            set_health(m, priest, full);
            crate::game::map::combat::FORCED_OUTCOME
                .with(|f| f.set(Some(crate::game::map::combat::Outcome::Crit)));
            let now = m.now;
            if let Some(cb) = m.combat_mut(kobold) {
                cb.victim = priest;
                cb.attacking = true;
                cb.next_swing = Some(now);
            }
            crate::game::map::combat::try_swing(m, kobold);
            let lost = full - health(m, priest);
            let absorbed = amount(m, heal);
            // A killing crit.
            set_health(m, priest, 1);
            if let Some(cb) = m.combat_mut(kobold) {
                cb.next_swing = Some(now);
            }
            crate::game::map::combat::try_swing(m, kobold);
            crate::game::map::combat::FORCED_OUTCOME.with(|f| f.set(None));
            let dead = (health(m, priest), amount(m, heal));
            (ranks, plain, by_spell, fractions, (lost, absorbed), dead)
        })
        .await
        .unwrap();
    let (ranks, plain, by_spell, fractions, (lost, absorbed), dead) = got;
    assert_eq!(ranks, [Some(8), Some(16), Some(25)]);
    assert_eq!((plain, by_spell), (None, None), "a crit weapon blow alone");
    assert_eq!(
        fractions.into_iter().collect::<Vec<_>>(),
        [Some(8), Some(9)],
        "8 and a third rounded either way"
    );
    assert!(lost > 0);
    let share = lost as i32 * 25;
    assert!(
        absorbed == Some(share / 300) || absorbed == Some(share / 300 + 1),
        "a third of 25% of the {lost} left: {absorbed:?}"
    );
    assert_eq!(dead, (0, None), "nothing from a killing blow");
}

const PERSISTENT_SHIELD: u32 = 26467;
const PERSISTENT_SHIELD_ABSORB: u32 = 26470;
const LESSER_HEAL: u32 = 2050;

/// Thistle Tea's Persistent Shield: a direct heal of 1,001 shields the
/// healed for 15% of it, 150; not one healed dead, nor a heal of nothing,
/// a harmful spell or a heal's tick.
#[tokio::test]
#[ignore = "extended"]
async fn persistent_shield_shields_the_directly_healed() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, priest, map) = player(&realm, "P3Q16", "Shieldone", Race::Human, Class::Priest).await;
    let (_d, friend, _) = player(&realm, "P3Q17", "Shieldtwo", Race::Human, Class::Warrior).await;
    let got = map
        .query(move |m| {
            ready(m, priest, 60, &[]);
            ready(m, friend, 60, &[]);
            apply(m, priest, priest, PERSISTENT_SHIELD);
            let shield = |m: &mut Map, flags: u32, damage: u32| {
                procs::trigger(
                    m,
                    Event {
                        actor: priest,
                        target: friend,
                        actor_flags: flags,
                        ex: procs::EX_NORMAL_HIT,
                        spell: Some(LESSER_HEAL),
                        damage,
                        school: 2,
                        ..Default::default()
                    },
                );
                let a = holder(m, friend, PERSISTENT_SHIELD_ABSORB).map(|h| h.effects[0].amount);
                auras::remove_spell(m, friend, PERSISTENT_SHIELD_ABSORB, None);
                a
            };
            let healed = shield(m, procs::PROC_DONE_POSITIVE_SPELL, 1_001);
            let nothing = shield(m, procs::PROC_DONE_POSITIVE_SPELL, 0);
            let harmful = shield(m, procs::PROC_DONE_NEGATIVE_SPELL, 1_001);
            let tick = shield(m, procs::PROC_DONE_PERIODIC, 1_001);
            crate::game::map::death::kill(m, friend);
            let dead = shield(m, procs::PROC_DONE_POSITIVE_SPELL, 1_001);
            (healed, nothing, harmful, tick, dead)
        })
        .await
        .unwrap();
    assert_eq!(got, (Some(150), None, None, None, None));
}
