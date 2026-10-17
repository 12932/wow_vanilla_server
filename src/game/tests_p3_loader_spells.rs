//! Phase 3's loader tests for spells (Thistle Tea's `world/loader`
//! `spell_vmangos_test.exs` and its `*_dbc_test.exs`): what wvs reads of
//! the real spells, over the real data when it is here, and the rules it
//! works out of them (exclusive kinds, forms, reflection, facing).

use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::map::auras::{self, Exclusive, *};
use crate::game::map::spells::{self, checks, outcome};
use crate::game::map::{objects, pets};

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

fn spell(d: &GameData, id: u32) -> &SpellTemplate {
    d.spells.get(id).unwrap_or_else(|| panic!("spell {id}"))
}

/// Whether `s` has an aura effect of `aura` with `misc`.
fn aura_misc(s: &SpellTemplate, aura: u32, misc: i32) -> bool {
    s.effects
        .iter()
        .any(|e| matches!(e.effect, 6 | 27 | 35) && e.aura == aura && e.misc_value == misc)
}

fn effect_of(s: &SpellTemplate, effect: u32) -> Option<&sd::SpellEffect> {
    s.effects.iter().find(|e| e.effect == effect)
}

fn exclusive(d: &GameData, id: u32) -> Option<Exclusive> {
    auras::exclusive(spell(d, id), d.spells.elixirs.get(&id).copied())
}

/// Thistle Tea's `spell_vmangos_test.exs`, its loader's facts that wvs
/// reads too: the trap spells, wild objects, Detect Magic, reflection and
/// invisibility, vmangos's effect fixes and per-level points, the stances
/// and what is locked to them, spell threat, Sunder's stacks, the health
/// costs, crit auras and Shatter's scripts, the rogue's and the paladin's,
/// warlock's and hunter's spells, a forced reaction, and trainers' own
/// greetings.
#[test]
fn the_real_spells_load_as_thistle_tea_reads_them() {
    let Some(d) = real() else {
        return;
    };
    // Detect Traps sees traps (detection kind 3); Disarm Trap opens their
    // locks (type 4), in stealth too.
    assert!(aura_misc(
        spell(d, 2836),
        AURA_MOD_INVISIBILITY_DETECTION,
        3
    ));
    let disarm = spell(d, 1842);
    assert_eq!(
        effect_of(disarm, spells::EFFECT_OPEN_LOCK).map(|e| e.misc_value),
        Some(4)
    );
    assert!(disarm.attr(1, sd::ATTR_EX_ALLOW_WHILE_STEALTHED));
    // Wild objects: a quest's egg for two minutes, a trap for one, and a
    // chest of two.
    for (id, entry, ms) in [(13_563, 161_513, 120_000), (25_648, 180_647, 60_000)] {
        let s = spell(d, id);
        assert_eq!(
            effect_of(s, objects::EFFECT_SUMMON_OBJECT_WILD).map(|e| e.misc_value),
            Some(entry),
            "{id}"
        );
        assert_eq!(d.spells.duration(s, 60), Some(ms), "{id}");
    }
    assert_eq!(
        spell(d, 11_756)
            .effects
            .iter()
            .filter(|e| e.effect == objects::EFFECT_SUMMON_OBJECT_WILD)
            .count(),
        2
    );
    // Detect Magic shows an enemy's auras for two minutes, a magic effect.
    let detect = spell(d, 2855);
    assert_eq!(d.spells.duration(detect, 60), Some(120_000));
    assert_eq!(detect.dispel, 1);
    assert_eq!(
        (detect.effects[0].aura, detect.effects[0].target_a),
        (AURA_AURAS_VISIBLE, 6)
    );
    // Sheen of Zanza reflects some, and its first reflection is sure and
    // spent; Magic Reflection reflects.
    let sheen = spell(d, 24_417);
    assert!(
        sheen
            .effects
            .iter()
            .any(|e| e.aura == AURA_REFLECT_SPELLS && e.base_points == 2)
    );
    assert_eq!(
        effect_of(sheen, spells::EFFECT_TRIGGER_SPELL).map(|e| e.trigger_spell),
        Some(30_003)
    );
    let first = spell(d, 30_003);
    assert_eq!(first.proc_charges, 1);
    assert!(
        first
            .effects
            .iter()
            .any(|e| e.aura == AURA_REFLECT_SPELLS && e.base_points == 99)
    );
    assert!(spell(d, 20_223).has_aura(AURA_REFLECT_SPELLS));
    // What may be reflected: Fireball and Frostbolt; not Sinister Strike, a
    // blow, nor Frost Nova, whose attribute says no.
    assert!(outcome::reflectable(spell(d, 133)));
    assert!(outcome::reflectable(spell(d, 116)));
    assert!(!outcome::reflectable(spell(d, 1752)));
    let nova = spell(d, 122);
    assert!(nova.attr(1, sd::ATTR_EX_NO_REFLECTION));
    assert!(!outcome::reflectable(nova));
    // A potion's invisibility, and every warlock's rank that sees it.
    assert!(aura_misc(spell(d, 11_392), AURA_MOD_INVISIBILITY, 0));
    for id in [132, 2970, 11_743] {
        assert!(
            aura_misc(spell(d, id), AURA_MOD_INVISIBILITY_DETECTION, 0),
            "{id}"
        );
    }
    // vmangos's spell_effect_mod: Bloodthirst triggers its heal buff on
    // the caster, Holy Light's first rank heals.
    let bloodthirst = spell(d, 23_881).effects[1];
    assert_eq!(
        (
            bloodthirst.effect,
            bloodthirst.trigger_spell,
            bloodthirst.target_a
        ),
        (spells::EFFECT_TRIGGER_SPELL, 23_885, 1)
    );
    assert!(spell(d, 635).has_effect(spells::EFFECT_HEAL));
    // A spell's level, and the points it gains a level past it.
    let fireball = spell(d, 133);
    assert_eq!(fireball.spell_level, 1);
    assert!(
        fireball
            .effects
            .iter()
            .any(|e| e.real_points_per_level > 0.0)
    );
    let mut fixed = fireball.clone();
    fixed.effects[0].die_sides = 0;
    fixed.effects[0].dice_per_level = 0.0;
    assert!(spells::points(&fixed, 0, fireball.max_level) > spells::points(&fixed, 0, 1));
    // The stances: each a form, its passives (threat and damage) brought by
    // the form; abilities locked to one.
    for (id, form) in [(2457, 17), (71, 18), (2458, 19)] {
        let s = spell(d, id);
        assert_eq!(s.stances, 0, "{id}");
        assert!(aura_misc(s, AURA_MOD_SHAPESHIFT, form), "{id}");
        assert_eq!(exclusive(d, id), Some(Exclusive::Shapeshift));
    }
    assert!(spell(d, 21_156).has_aura(AURA_MOD_THREAT));
    let defensive = spell(d, 7376);
    assert!(defensive.has_aura(AURA_MOD_THREAT));
    assert!(defensive.has_aura(AURA_MOD_DAMAGE_PERCENT_TAKEN));
    let overpower = spell(d, 7384);
    assert_eq!(overpower.stances, 0x1_0000);
    assert_eq!(checks::form_error(overpower, 17), None);
    assert!(checks::form_error(overpower, 19).is_some());
    assert!(checks::form_error(overpower, 0).is_some());
    let whirlwind = spell(d, 1680);
    assert_eq!(whirlwind.stances, 0x4_0000);
    assert_eq!(checks::form_error(whirlwind, 19), None);
    assert!(checks::form_error(whirlwind, 17).is_some());
    // spell_threat: Heroic Strike's flat threat, Revenge's multiplier, and
    // none for a spell without a row.
    assert_eq!(d.spells.threat.get(&78), Some(&(20, 1.0)));
    assert_eq!(d.spells.threat.get(&6572), Some(&(63, 2.25)));
    assert_eq!(d.spells.threat.get(&133), None);
    // Sunder Armor stacks to five.
    assert_eq!(spell(d, 7386).stack_amount, 5);
    // Bloodrage costs a fifth of base health (power -2) for its rage.
    let bloodrage = spell(d, 2687);
    assert_eq!(
        (bloodrage.power_type as i32, bloodrage.mana_cost_percentage),
        (-2, 20)
    );
    assert!(bloodrage.has_effect(spells::EFFECT_ENERGIZE));
    // Spell crit, of all schools and of one; Shatter's class scripts.
    assert!(spell(d, 2916).has_aura(AURA_MOD_SPELL_CRIT_CHANCE));
    assert!(spell(d, 5835).has_aura(AURA_MOD_SPELL_CRIT_CHANCE_SCHOOL));
    let shatter: Vec<i32> = [11_170, 12_982, 12_983, 12_984, 12_985]
        .iter()
        .map(|id| {
            spell(d, *id)
                .effects
                .iter()
                .find(|e| e.aura == AURA_OVERRIDE_CLASS_SCRIPTS)
                .map_or(0, |e| e.misc_value)
        })
        .collect();
    assert_eq!(shatter, vec![849, 910, 911, 912, 913]);
    assert!(spell(d, 5857).attr(2, sd::ATTR_EX2_CANT_CRIT));
    // The rogue's: Feint's threat, Cold Blood's crit modifier, Vanish's
    // stealth, its freeing and its threat wipe; Backstab and Ambush from
    // behind; Stealth and Cold Blood's cooldowns from their auras' ends.
    assert!(spell(d, 1966).has_effect(spells::EFFECT_THREAT));
    assert!(
        spell(d, 14_177)
            .effects
            .iter()
            .any(|e| e.aura == AURA_ADD_FLAT_MODIFIER
                && e.misc_value == 7
                && e.item_type == 0x20206)
    );
    let vanish = spell(d, 1856);
    let triggers: Vec<u32> = vanish
        .effects
        .iter()
        .filter(|e| e.effect == spells::EFFECT_TRIGGER_SPELL)
        .map(|e| e.trigger_spell)
        .collect();
    assert_eq!(triggers, vec![11_327, 18_461]);
    assert!(vanish.has_effect(spells::EFFECT_SANCTUARY));
    assert!(spell(d, 53).from_behind());
    assert!(spell(d, 8676).from_behind());
    assert!(!spell(d, 1776).from_behind());
    for id in [1784, 14_177] {
        assert!(spell(d, id).attr(0, 0x0200_0000), "{id} waits on its aura");
    }
    // Only an ability blocked whole says so: Shield Bash, not a judgement
    // or a seal.
    assert!(spell(d, 72).attr(3, 0x8));
    assert!(!spell(d, 20_467).attr(3, 0x8));
    assert!(!spell(d, 20_424).attr(3, 0x8));
    // The paladin's: a seal one at a time, with its judgement's spell; a
    // blessing one a paladin; an aura an area's; Exorcism, Holy Wrath and
    // Turn Undead for undead and demons; Lay on Hands heals to full.
    assert_eq!(exclusive(d, 20_287), Some(Exclusive::PaladinSeal));
    let seal = spell(d, 20_287).effects[2];
    assert_eq!((seal.aura, seal.base_points), (AURA_DUMMY, 20_279));
    assert!(!spell(d, 20_271).from_behind());
    assert_eq!(exclusive(d, 19_740), Some(Exclusive::PaladinBlessing));
    assert_eq!(exclusive(d, 465), Some(Exclusive::PaladinAura));
    assert!(spell(d, 465).has_effect(spells::EFFECT_APPLY_AREA_AURA_PARTY));
    assert_eq!(spell(d, 879).target_creature_type, 36);
    assert_eq!(spell(d, 2812).target_creature_type, 36);
    assert_eq!(spell(d, 2878).target_creature_type, 32);
    assert!(spell(d, 633).has_effect(spells::EFFECT_HEAL_MAX_HEALTH));
    assert!(spell(d, 639).has_effect(spells::EFFECT_HEAL));
    // Flash of Light heals through its script, as 19993.
    assert!(spell(d, 19_750).has_effect(spells::EFFECT_SCRIPT_EFFECT));
    assert!(spell(d, 19_993).has_effect(spells::EFFECT_HEAL));
    // The warlock's: Drain Life leeches health, Drain Mana mana, the pet's
    // drain is the pet's; Health Funnel's costs; the Inferno's charm, its
    // blast about the caster and its root; the family flags its scripts
    // go by; one curse a warlock; Soul Link on the pet and its split.
    assert!(spell(d, 6789).has_effect(spells::EFFECT_HEALTH_LEECH));
    assert!(spell(d, 5138).has_aura(AURA_PERIODIC_MANA_LEECH));
    let pet_drain = effect_of(spell(d, 18_220), spells::EFFECT_POWER_DRAIN).unwrap();
    assert_eq!(pet_drain.target_a, 5);
    let funnel = spell(d, 11_693);
    assert_eq!(
        (
            funnel.power_type as i32,
            funnel.mana_cost,
            funnel.mana_per_second
        ),
        (-2, 45, 33)
    );
    assert!(spell(d, 20_882).has_aura(AURA_MOD_CHARM));
    let landing = effect_of(spell(d, 22_703), spells::EFFECT_SCHOOL_DAMAGE).unwrap();
    assert_eq!((landing.target_a, landing.target_b), (22, 15));
    assert!(spell(d, 22_707).has_aura(AURA_MOD_ROOT));
    for (id, flag) in [
        (1454, 0x0004_0000u64),
        (11_689, 0x0004_0000),
        (17_962, 0x200),
        (348, 0x4),
        (980, 0x400),
    ] {
        let s = spell(d, id);
        assert!(s.family_name == 5 && s.family_flags & flag != 0, "{id}");
    }
    for id in [702, 980, 1490, 18_223] {
        assert_eq!(exclusive(d, id), Some(Exclusive::WarlockCurse), "{id}");
    }
    assert!(spell(d, 1714).has_aura(AURA_MOD_CASTING_SPEED));
    assert!(
        spell(d, 19_028)
            .effects
            .iter()
            .any(|e| e.effect == spells::EFFECT_DUMMY && e.target_a == 5)
    );
    let soul_link = spell(d, 25_228);
    assert!(
        soul_link
            .effects
            .iter()
            .any(|e| e.effect == pets::EFFECT_APPLY_AREA_AURA_PET
                && e.aura == AURA_SPLIT_DAMAGE_PCT
                && e.base_points == 29)
    );
    // The hunter's, shaman's and druid's: a trap, a totem in its slot, a
    // quiver's haste, a form, the aspects and the tracking, a shield.
    assert_eq!(
        effect_of(spell(d, 1499), objects::EFFECT_SUMMON_OBJECT_SLOT1).map(|e| e.misc_value),
        Some(2561)
    );
    let searing = spell(d, 3599);
    assert!(
        searing
            .effects
            .iter()
            .any(|e| e.effect == pets::EFFECT_SUMMON_TOTEM_SLOT1 && e.misc_value == 2523)
    );
    assert!(spell(d, 29_414).has_aura(AURA_MOD_RANGED_AMMO_HASTE));
    assert_eq!(exclusive(d, 768), Some(Exclusive::Shapeshift));
    assert_eq!(exclusive(d, 13_163), Some(Exclusive::HunterAspect));
    assert_eq!(exclusive(d, 13_161), Some(Exclusive::HunterAspect));
    assert_eq!(exclusive(d, 1494), Some(Exclusive::Tracking));
    for (id, kind) in [(2383, 2), (2580, 3), (2481, 6)] {
        assert_eq!(exclusive(d, id), Some(Exclusive::Tracking), "{id}");
        assert!(aura_misc(spell(d, id), AURA_TRACK_RESOURCES, kind), "{id}");
    }
    assert_eq!(exclusive(d, 324), Some(Exclusive::ShamanShield));
    // A forced reaction: the faction and the rank it is seen at.
    let reaction = spell(d, 6405)
        .effects
        .iter()
        .find(|e| e.aura == AURA_FORCE_REACTION)
        .copied()
        .unwrap();
    assert_eq!((reaction.misc_value, reaction.base_points), (575, 3));
    // Heals' coefficients: Flash Heal's, Renew's, Holy Light's; Flash of
    // Light's ranks have none of their own, their triggered heal (19993,
    // as wvs's script casts) its 0.429.
    for (id, c) in [
        (10_917, 0.429),
        (10_929, 0.2),
        (25_292, 0.714),
        (19_993, 0.429),
    ] {
        assert_eq!(spell(d, id).effects[0].bonus_coefficient, c, "{id}");
    }
    for id in [19_750, 19_939, 19_940, 19_941, 19_942, 19_943] {
        assert_eq!(spell(d, id).effects[0].bonus_coefficient, 0.0, "{id}");
    }
    // Trainers greet with their own text where vmangos has one.
    let greeting = |e: u32| d.npc.trainer_greetings.get(&e).map(String::as_str);
    assert_eq!(
        greeting(198),
        Some("Hello, mage!  Ready for some training?")
    );
    assert!(greeting(223).is_some_and(|g| g.contains("beast hides into armor")));
    assert_eq!(
        greeting(913),
        Some("Hello, warrior!  Ready for some training?")
    );
    assert_eq!(greeting(2870), None);
}

/// The first effect of `s` with aura `aura`, its points at the spell's own
/// level (Thistle Tea's `Effect.roll(effect, 0)`), and its `misc`.
fn aura_points(s: &SpellTemplate, aura: u32) -> Option<(usize, i32, i32)> {
    (0..3)
        .find(|i| s.effects[*i].aura == aura)
        .map(|i| (i, spells::points(s, i, 0), s.effects[i].misc_value))
}

/// Thistle Tea's loader facts for aura amounts (`spell_attack_power`,
/// `spell_percent_stat`, `spell_percent_damage`, `spell_power_burn` and
/// `spell_mana_shield`'s `*_dbc_test.exs`): signed attack power and its shares; stat shares and
/// their stat (-1 all), and resurrection sickness through death where a
/// world buff isn't; damage a share of health, and its cadence; mana
/// burns, and every Mana Shield's two mana a point.
#[test]
fn the_real_aura_amounts_load_as_thistle_tea_reads_them() {
    let Some(d) = real() else {
        return;
    };
    for (id, aura, amount) in [
        (1160, AURA_MOD_ATTACK_POWER, -35),
        (11_556, AURA_MOD_ATTACK_POWER, -140),
        (11_717, AURA_MOD_ATTACK_POWER, 90),
        (25_289, AURA_MOD_ATTACK_POWER, 232),
        (9199, AURA_MOD_ATTACK_POWER_PCT, 50),
    ] {
        assert_eq!(
            aura_points(spell(d, id), aura).map(|p| p.1),
            Some(amount),
            "{id}"
        );
    }
    assert!(spell(d, 15_007).attr(3, sd::ATTR_EX3_DEATH_PERSISTENT));
    assert!(!spell(d, 23_735).attr(3, sd::ATTR_EX3_DEATH_PERSISTENT));
    for (id, amount, stat) in [
        (2146, -75, -1),
        (11_630, -50, -1),
        (11_631, -25, -1),
        (15_007, -75, -1),
        (16_734, 2, 3),
        (16_735, 2, 3),
        (16_736, 2, 3),
        (16_737, 2, 3),
        (16_738, 2, 3),
        (23_735, 10, 0),
        (23_736, 10, 1),
        (23_737, 10, 2),
        (23_738, 10, 4),
        (23_766, 10, 3),
        (24_705, 25, 2),
        (26_035, 10, -1),
        (26_393, 10, -1),
        (26_462, 10, -1),
    ] {
        let p = aura_points(spell(d, id), AURA_MOD_PERCENT_STAT).unwrap();
        assert_eq!((p.1, p.2), (amount, stat), "{id}");
    }
    for (id, index, percent, ms) in [
        (23_072, 1, 20, 4000),
        (23_127, 0, 5, 3000),
        (23_449, 0, 10, 2000),
        (24_910, 0, 20, 2000),
        (25_838, 1, 90, 20_000),
    ] {
        let s = spell(d, id);
        let p = aura_points(s, AURA_PERIODIC_DAMAGE_PERCENT).unwrap();
        assert_eq!(
            (p.0, p.1, s.effects[p.0].amplitude),
            (index, percent, ms),
            "{id}"
        );
    }
    for (id, points, ms, multiple) in [
        (19_659, 399, 3000, 1.0),
        (23_153, 49, 1000, 0.0),
        (24_619, 499, 2000, 1.0),
    ] {
        let s = spell(d, id);
        let e = s
            .effects
            .iter()
            .find(|e| e.aura == AURA_POWER_BURN_MANA)
            .unwrap();
        assert_eq!(
            (e.base_points, e.amplitude, e.multiple_value, e.misc_value),
            (points, ms, multiple, 0),
            "{id}"
        );
    }
    // Blood Siphon and Blood Funnel are vmangos's spell_effect_mod's: a
    // periodic heal and damage, no funnel.
    for id in [24_322, 24_617] {
        let s = spell(d, id);
        assert!(!s.has_aura(AURA_PERIODIC_HEALTH_FUNNEL), "{id}");
        assert!(
            s.has_aura(AURA_PERIODIC_HEAL) && s.has_aura(AURA_PERIODIC_DAMAGE),
            "{id}"
        );
    }
    for id in [1463, 8494, 8495, 10_191, 10_192, 10_193] {
        let s = spell(d, id);
        let shields: Vec<(i32, f32)> = s
            .effects
            .iter()
            .filter(|e| e.effect != 0)
            .map(|e| (e.misc_value, e.multiple_value))
            .collect();
        assert!(s.has_aura(AURA_MANA_SHIELD), "{id}");
        assert_eq!(shields, vec![(1, 2.0)], "{id}");
    }
}

/// Thistle Tea's exclusive kinds on the real spells
/// (`slow_exclusivity_dbc_test.exs`, `consumable_stacking_dbc_test.exs`,
/// `spell_elixir_test.exs`): class snares one at a time and a melee speed
/// penalty its own, but a daze or a spell with other auras none; foods,
/// drinks, both, and Well Fed; flasks by their `spell_elixir` mask, and no
/// rule for an ordinary elixir's 0.
#[test]
fn the_real_exclusive_kinds_are_thistle_teas() {
    let Some(d) = real() else {
        return;
    };
    for id in [116, 120, 1715, 2974, 3409, 3600, 8056, 11_201] {
        assert_eq!(exclusive(d, id), Some(Exclusive::Snare), "{id}");
    }
    // vmangos's spell_mod gives Concussive Shot and Piercing Howl the daze
    // attribute: as a daze, no snare's.
    for id in [5116, 12_323] {
        assert!(spell(d, id).attributes[1] & 0x4_0000 != 0, "{id}");
        assert_eq!(exclusive(d, id), None, "{id}");
    }
    assert_eq!(exclusive(d, 6343), Some(Exclusive::NegativeHaste));
    for id in [1604, 1098, 22_812] {
        assert_eq!(exclusive(d, id), None, "{id}");
    }
    for (id, kind) in [
        (5004, Exclusive::Food),
        (434, Exclusive::Food),
        (430, Exclusive::Drink),
        (1137, Exclusive::Drink),
        (24_707, Exclusive::FoodAndDrink),
        (25_990, Exclusive::FoodAndDrink),
        (19_705, Exclusive::WellFed),
        (19_711, Exclusive::WellFed),
        (24_799, Exclusive::WellFed),
        (18_125, Exclusive::WellFed),
    ] {
        assert_eq!(exclusive(d, id), Some(kind), "{id}");
    }
    // vmangos's spell_mod lets Blessed Sunfruit Juice and Alterac Spring
    // Water stack (no family, so not the Well Fed rule's).
    for id in [18_141, 23_697] {
        assert_eq!(
            (spell(d, id).family_name, exclusive(d, id)),
            (0, None),
            "{id}"
        );
    }
    assert_eq!(exclusive(d, 18_192), None);
    for id in [17_624, 17_626, 17_627, 17_628, 17_629] {
        assert_eq!(d.spells.elixirs.get(&id), Some(&3), "{id}");
        assert_eq!(exclusive(d, id), Some(Exclusive::Flask), "{id}");
    }
    for id in [2374, 11_328, 17_538] {
        assert_eq!(d.spells.elixirs.get(&id).copied().unwrap_or(0), 0, "{id}");
        assert_eq!(auras::exclusive(spell(d, id), Some(0)), None, "{id}");
    }
}
