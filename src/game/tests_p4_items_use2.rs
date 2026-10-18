//! Phase 4's items in use on the real data, Thistle Tea's
//! `HealingPowerDbcTest` and `ReactiveArmorDbcTest` ported: the healing
//! sets' procs from their own direct heals and the healed one's class
//! buff; Obsidian Armor's wards by school and Adaptive Warding's
//! resistances under Mage Armor; what each does once its gear or Mage
//! Armor is gone, or its bearer dead. The rest is in `tests_p4_items_use`.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::map::procs::{self, Event};
use crate::game::map::{Map, auras, death, derive, inventory};
use crate::game::tests_p3_spells_kit::{
    apply, have_data, holder, made_up, pass, player, ready, realm_plus,
};
use crate::game::tests_p4_items_gear::wear;

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

const HOLY_POWER: u32 = 28789;
const TOTEMIC_POWER: u32 = 28823;
const OBSIDIAN_ARMOR: u32 = 27539;
const ADAPTIVE_WARDING: u32 = 28764;
const ABSORBS: [u32; 6] = [27533, 27534, 27535, 27536, 27538, 27540];
/// Adaptive Warding's resistances: fire, nature, frost, shadow, arcane.
const RESISTANCES: [u32; 5] = [28765, 28768, 28766, 28769, 28770];
const MAGE_ARMOR: [u32; 3] = [6117, 22782, 22783];
const PLATE: u32 = 750;
/// Redemption Armor, The Earthshatterer and Frostfire Regalia, six and
/// four of them, by slot; the Thick Obsidian Breastplate.
const REDEMPTION: [(u32, u8); 6] = [
    (22428, 0),
    (22429, 2),
    (22425, 4),
    (22431, 5),
    (22427, 6),
    (22430, 7),
];
const EARTHSHATTER: [(u32, u8); 6] = [
    (22466, 0),
    (22467, 2),
    (22464, 4),
    (22470, 5),
    (22465, 6),
    (22468, 7),
];
const FROSTFIRE: [(u32, u8); 4] = [(22498, 0), (22499, 2), (22496, 4), (22497, 6)];
const OBSIDIAN_BREASTPLATE: u32 = 22196;
/// Bolts of each school but the physical, made up: holy, fire, nature,
/// frost, shadow, arcane.
const BOLTS: [u32; 6] = [995_201, 995_202, 995_203, 995_204, 995_205, 995_206];

/// What the realm's data said before the procs were made sure.
#[derive(Debug, Default)]
struct Read {
    chances: Vec<(u32, u32, u32, u32, u32)>,
    sets: Vec<(u32, Vec<(u32, u32)>)>,
    absorbs: Vec<(u32, Option<u32>, u32, i32, i32, i32)>,
    resistances: Vec<(u32, Option<u32>, u32, i32, i32)>,
}

/// The real data with the sets' and armors' procs made sure, and the
/// bolts; what was read of them first.
fn realm() -> (Arc<Realm>, Read) {
    let mut read = Read::default();
    let realm = realm_plus(|d| {
        for id in [HOLY_POWER, TOTEMIC_POWER, OBSIDIAN_ARMOR, ADAPTIVE_WARDING] {
            let s = d.spells.spells.get_mut(&id).unwrap();
            read.chances.push((
                id,
                s.proc_chance,
                s.proc_charges,
                s.proc_flags,
                s.effects[0].aura,
            ));
            s.proc_chance = 100;
        }
        for set in [526, 527, 528] {
            read.sets
                .push((set, d.crafts.item_sets.get(&set).unwrap().spells.clone()));
        }
        for id in ABSORBS {
            let s = d.spells.get(id).unwrap();
            let e = &s.effects[0];
            read.absorbs.push((
                id,
                d.spells.duration(s, 60),
                e.aura,
                e.base_points,
                e.die_sides,
                e.misc_value,
            ));
        }
        for id in RESISTANCES {
            let s = d.spells.get(id).unwrap();
            let e = &s.effects[0];
            read.resistances.push((
                id,
                d.spells.duration(s, 60),
                e.aura,
                e.base_points,
                e.die_sides,
            ));
        }
        for (school, id) in BOLTS.iter().enumerate() {
            let mut bolt = made_up(*id, 2, 0, 0, 20);
            bolt.school = school as u32 + 1;
            bolt.dmg_class = 1;
            bolt.effects[0].target_a = 6;
            d.spells.spells.insert(*id, bolt);
        }
    });
    (realm, read)
}

/// Nothing on `g`, then `pieces` worn, their auras synced.
fn dress(m: &mut Map, g: Guid, pieces: &[(u32, u8)]) {
    strip(m, g);
    for (entry, slot) in pieces {
        wear(m, g, *entry, *slot);
    }
}

fn strip(m: &mut Map, g: Guid) {
    let d = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().items.clear();
    m.players.get_mut(&g).unwrap().refresh_inventory(&d);
    inventory::sync_gear(m, g);
    derive::unit(m, g);
}

fn gear(m: &Map, g: Guid, spell: u32) -> bool {
    m.players[&g]
        .auras
        .holders
        .iter()
        .any(|h| h.equipped && h.spell == spell)
}

/// `healer`'s heal `spell` on `target` landing as `ex`, from a
/// periodic heal if `periodic`, for their procs.
fn heal(m: &mut Map, healer: Guid, target: Guid, spell: u32, ex: u32, periodic: bool) {
    let (actor_flags, target_flags) = if periodic {
        (procs::PROC_DONE_PERIODIC, procs::PROC_TAKEN_PERIODIC)
    } else {
        (
            procs::PROC_DONE_POSITIVE_SPELL,
            procs::PROC_TAKEN_POSITIVE_SPELL,
        )
    };
    procs::trigger(
        m,
        Event {
            actor: healer,
            target,
            actor_flags,
            target_flags,
            ex,
            spell: Some(spell),
            damage: 100,
            school: 2,
            ..Default::default()
        },
    );
}

/// A bolt of `school` (2 fire to 6 arcane) from `attacker` on `g`, for
/// their procs.
fn bolt(m: &mut Map, attacker: Guid, g: Guid, school: usize) {
    procs::trigger(
        m,
        Event {
            actor: attacker,
            target: g,
            actor_flags: procs::PROC_DONE_NEGATIVE_SPELL,
            target_flags: procs::PROC_TAKEN_NEGATIVE_SPELL,
            ex: procs::EX_NORMAL_HIT,
            spell: Some(BOLTS[school - 1]),
            damage: 20,
            school: 1 << school,
            ..Default::default()
        },
    );
}

/// Which of `spells` are on `g`.
fn on(m: &Map, g: Guid, spells: &[u32]) -> Vec<u32> {
    spells.iter().copied().filter(|s| has(m, g, *s)).collect()
}

fn take_off(m: &mut Map, g: Guid, spells: &[u32]) {
    for s in spells {
        auras::remove_spell(m, g, *s, None);
    }
}

/// Holy Power's and Totemic Power's buffs, by the class of the healed:
/// a warrior's armor, a mana user's mana, a hunter's attack power, a
/// caster's spell damage.
const BUFFS: [(u32, [(u8, u32); 4]); 2] = [
    (HOLY_POWER, [(1, 28790), (2, 28795), (3, 28791), (8, 28793)]),
    (
        TOTEMIC_POWER,
        [(1, 28827), (2, 28824), (3, 28826), (8, 28825)],
    ),
];

fn all_buffs() -> Vec<u32> {
    BUFFS
        .iter()
        .flat_map(|(_, b)| b.iter().map(|(_, s)| *s))
        .collect()
}

/// What a class's buff gives: armor, the aura's mana a five seconds,
/// attack power, fire damage.
fn bonus(m: &Map, g: Guid, class: u8) -> i64 {
    let f = &m.players[&g].fields;
    match class {
        1 => i64::from(f.get_u32(UNIT_FIELD_RESISTANCES)),
        2 => m.players[&g]
            .auras
            .holders
            .iter()
            .flat_map(|h| h.effects.iter())
            .filter(|x| x.kind == 85)
            .map(|x| i64::from(x.amount))
            .sum(),
        3 => {
            i64::from(f.get_u32(UNIT_FIELD_ATTACK_POWER))
                + i64::from(f.get_u32(UNIT_FIELD_ATTACK_POWER_MODS) as i32)
        }
        _ => i64::from(f.get_u32(PLAYER_FIELD_MOD_DAMAGE_DONE_POS + 2)),
    }
}

/// Thistle Tea's "six-piece bonuses retain their ten-percent
/// direct-heal rules": Redemption Armor's six give Holy Power, the
/// Earthshatterer's Totemic Power, each a dummy aura proccing one heal
/// in ten, without charges; from the set's own direct heals, normal or
/// critical (Holy Light and Flash of Light; Healing Wave and Lesser
/// Healing Wave), not a periodic heal nor a cast's end, nor the other
/// set's heals, Purge, Chain Heal, Holy Shock or Renew.
#[tokio::test]
#[ignore = "extended"]
async fn the_healing_sets_proc_from_their_own_direct_heals() {
    if !have_data() {
        return;
    }
    let (realm, read) = realm();
    for id in [HOLY_POWER, TOTEMIC_POWER] {
        let row = read.chances.iter().find(|r| r.0 == id).unwrap();
        assert_eq!(
            (row.1, row.2, row.3, row.4),
            (10, 0, procs::PROC_DONE_POSITIVE_SPELL, auras::AURA_DUMMY),
            "{id}"
        );
    }
    for (set, spell) in [(528, HOLY_POWER), (527, TOTEMIC_POWER)] {
        let spells = &read.sets.iter().find(|s| s.0 == set).unwrap().1;
        assert!(spells.contains(&(spell, 6)), "{set}: {spells:?}");
    }
    let (_a, healer, map) = player(&realm, "U4H1", "Healone", Race::Human, Class::Paladin).await;
    let (_b, w, _) = player(&realm, "U4H2", "Healtwo", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, healer, 60, &[PLATE]);
            ready(m, w, 60, &[]);
            let mut out = Vec::new();
            for (pieces, allowed, rejected) in [
                (
                    &REDEMPTION,
                    [635, 19750],
                    vec![331, 8004, 8013, 20473, 25914, 139],
                ),
                (&EARTHSHATTER, [331, 8004], vec![635, 19750, 1064]),
            ] {
                dress(m, healer, pieces);
                for id in allowed {
                    for (ex, periodic, what) in [
                        (procs::EX_NORMAL_HIT, false, "normal"),
                        (procs::EX_CRITICAL_HIT, false, "critical"),
                        (procs::EX_PERIODIC_POSITIVE, true, "periodic"),
                        (procs::EX_CAST_END, false, "cast end"),
                    ] {
                        heal(m, healer, w, id, ex, periodic);
                        out.push((id, what, !on(m, w, &all_buffs()).is_empty()));
                        take_off(m, w, &all_buffs());
                    }
                }
                for id in rejected {
                    heal(m, healer, w, id, procs::EX_NORMAL_HIT, false);
                    out.push((id, "other", !on(m, w, &all_buffs()).is_empty()));
                    take_off(m, w, &all_buffs());
                }
            }
            out
        })
        .await
        .unwrap();
    for (id, what, buffed) in out {
        let expected = matches!(what, "normal" | "critical");
        assert_eq!(buffed, expected, "{id} {what}");
    }
}

/// Thistle Tea's "both sets deliver the class buff through resolved
/// healing feedback": for a warrior armor, a paladin mana, a hunter
/// attack power and a mage spell damage, from its healer, eight
/// seconds of it; again, still one, its bonus once; run out or its
/// bearer dead, gone, and the bonus with it.
#[tokio::test]
#[ignore = "extended"]
async fn the_healing_sets_give_the_healed_their_classs_buff() {
    if !have_data() {
        return;
    }
    let (realm, _) = realm();
    let (_a, healer, map) = player(&realm, "U4C0", "Givesone", Race::Human, Class::Paladin).await;
    let mut clients = Vec::new();
    let mut healed = Vec::new();
    for (i, (race, class)) in [
        (Race::Human, Class::Warrior),
        (Race::Human, Class::Paladin),
        (Race::Dwarf, Class::Hunter),
        (Race::Human, Class::Mage),
    ]
    .into_iter()
    .enumerate()
    {
        let (c, g, _) = player(
            &realm,
            &format!("U4C{}", i + 1),
            &format!("Takes{}", ["one", "two", "three", "four"][i]),
            race,
            class,
        )
        .await;
        clients.push(c);
        healed.push((class.as_int(), g));
    }
    let out = map
        .query(move |m| {
            ready(m, healer, 60, &[PLATE]);
            let mut out = Vec::new();
            for (pieces, (set_spell, buffs), heal_id) in [
                (&REDEMPTION, BUFFS[0], 19750),
                (&EARTHSHATTER, BUFFS[1], 331),
            ] {
                dress(m, healer, pieces);
                assert!(gear(m, healer, set_spell), "{set_spell} worn");
                for (class, g) in healed.iter().copied() {
                    ready(m, g, 60, &[]);
                    derive::unit(m, g);
                    let buff = buffs.iter().find(|b| b.0 == class).unwrap().1;
                    let before = bonus(m, g, class);
                    heal(m, healer, g, heal_id, procs::EX_NORMAL_HIT, false);
                    derive::unit(m, g);
                    let h = holder(m, g, buff);
                    let given = bonus(m, g, class) - before;
                    // Again: one buff, its bonus once.
                    heal(m, healer, g, heal_id, procs::EX_NORMAL_HIT, false);
                    derive::unit(m, g);
                    let count = m.players[&g]
                        .auras
                        .holders
                        .iter()
                        .filter(|h| h.spell == buff)
                        .count();
                    let again = bonus(m, g, class) - before;
                    // Run out.
                    pass(m, 8_100);
                    derive::unit(m, g);
                    let expired = !has(m, g, buff) && bonus(m, g, class) == before;
                    // Again, and its bearer dead.
                    heal(m, healer, g, heal_id, procs::EX_NORMAL_HIT, false);
                    let back = has(m, g, buff);
                    death::kill(m, g);
                    let dead = !has(m, g, buff);
                    death::resurrect(m, g, 1.0);
                    out.push((
                        set_spell,
                        class,
                        h.map(|h| (h.caster == healer, h.item.is_none(), h.duration)),
                        given,
                        count,
                        again,
                        expired,
                        back,
                        dead,
                    ));
                }
            }
            out
        })
        .await
        .unwrap();
    for (set_spell, class, h, given, count, again, expired, back, dead) in out {
        let what = format!("{set_spell} on class {class}");
        assert_eq!(h, Some((true, true, Some(8_000))), "{what}");
        let expected = match class {
            1 => 700,
            2 => 28,
            3 => 140,
            _ => 80,
        };
        assert_eq!((given, again), (expected, expected), "{what}");
        assert_eq!(
            (count, expired, back, dead),
            (1, true, true, true),
            "{what}"
        );
    }
}

/// Thistle Tea's "set removal stops new procs without removing an
/// already delivered buff" and "a recipient dying after feedback
/// prevents delivery": the set taken off, Holy Power goes and gives no
/// more, and the buff given stays; worn again, it is back. A dead
/// recipient gets nothing.
#[tokio::test]
#[ignore = "extended"]
async fn a_healing_set_taken_off_gives_no_more_and_the_dead_get_nothing() {
    if !have_data() {
        return;
    }
    let (realm, _) = realm();
    let (_a, healer, map) = player(&realm, "U4R1", "Setone", Race::Human, Class::Paladin).await;
    let (_b, mage, _) = player(&realm, "U4R2", "Settwo", Race::Human, Class::Mage).await;
    let (_c, other, _) = player(&realm, "U4R3", "Setthree", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, healer, 60, &[PLATE]);
            dress(m, healer, &REDEMPTION);
            heal(m, healer, mage, 19750, procs::EX_NORMAL_HIT, false);
            let given = has(m, mage, 28793);
            strip(m, healer);
            let gone = !gear(m, healer, HOLY_POWER);
            heal(m, healer, other, 19750, procs::EX_NORMAL_HIT, false);
            let no_more = !has(m, other, 28793);
            let kept = has(m, mage, 28793);
            dress(m, healer, &REDEMPTION);
            let back = gear(m, healer, HOLY_POWER);
            death::kill(m, other);
            heal(m, healer, other, 19750, procs::EX_NORMAL_HIT, false);
            let dead = !has(m, other, 28793);
            (given, gone, no_more, kept, back, dead)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true, true, true, true));
}

/// Thistle Tea's "real equipment procs retain their chances and
/// school-specific effects": Obsidian Armor procs three hits in ten,
/// Adaptive Warding (the Frostfire Regalia's four) two, each a dummy
/// aura on spells taken; the obsidian wards absorb 300 to 500 of their
/// school for six seconds, the warding's resistances are 35 for thirty.
#[tokio::test]
#[ignore = "extended"]
async fn the_reactive_armors_procs_and_answers_are_the_datas() {
    if !have_data() {
        return;
    }
    let (_realm, read) = realm();
    for (id, chance) in [(OBSIDIAN_ARMOR, 30), (ADAPTIVE_WARDING, 20)] {
        let row = read.chances.iter().find(|r| r.0 == id).unwrap();
        assert_eq!(
            (row.1, row.3, row.4),
            (chance, procs::PROC_TAKEN_NEGATIVE_SPELL, auras::AURA_DUMMY),
            "{id}"
        );
    }
    let frostfire = &read.sets.iter().find(|s| s.0 == 526).unwrap().1;
    assert!(frostfire.contains(&(ADAPTIVE_WARDING, 4)), "{frostfire:?}");
    for (id, duration, aura, base, die, _) in &read.absorbs {
        assert_eq!(
            (*duration, *aura, *base, *die),
            (Some(6_000), auras::AURA_SCHOOL_ABSORB, 299, 201),
            "{id}"
        );
    }
    for (id, duration, aura, base, die) in &read.resistances {
        assert_eq!(
            (*duration, *aura, *base, *die),
            (Some(30_000), auras::AURA_MOD_RESISTANCE, 34, 1),
            "{id}"
        );
    }
}

/// The absorb left of `g`'s ward `spell`.
fn ward(m: &Map, g: Guid, spell: u32) -> Option<i32> {
    holder(m, g, spell).map(|h| {
        h.effects
            .iter()
            .filter(|x| x.kind == auras::AURA_SCHOOL_ABSORB)
            .map(|x| x.left)
            .sum()
    })
}

/// Thistle Tea's "Obsidian Armor rolls its shield after damage and
/// absorbs only its school": a fire spell taken, the breastplate wards
/// against fire (300 to 500 of it); frost goes through it; fire is
/// soaked, partly then wholly, and the ward used up goes. Unused, it
/// runs out in six seconds, and the armor answers again ten seconds
/// after it last did. Thistle Tea's "removing equipment prevents new
/// procs but lets an existing ward expire": the breastplate taken off, its
/// ward stays its six seconds, and there is no ward more.
#[tokio::test]
#[ignore = "extended"]
async fn obsidian_armor_wards_against_the_school_that_hit() {
    if !have_data() {
        return;
    }
    let (realm, _) = realm();
    let (_a, w, map) = player(&realm, "U4O1", "Wardone", Race::Human, Class::Warrior).await;
    let (_b, foe, _) = player(&realm, "U4O2", "Wardtwo", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[PLATE]);
            dress(m, w, &[(OBSIDIAN_BREASTPLATE, 4)]);
            let worn = gear(m, w, OBSIDIAN_ARMOR);
            bolt(m, foe, w, 2);
            let wards = on(m, w, &ABSORBS);
            let capacity = ward(m, w, 27533).unwrap_or(0);
            let frost = auras::absorb(m, w, 50, 1 << 4);
            let after_frost = ward(m, w, 27533);
            let partly = auras::absorb(m, w, 125, 1 << 2);
            let after_partly = ward(m, w, 27533);
            let wholly = auras::absorb(m, w, capacity as u32, 1 << 2);
            let used_up = ward(m, w, 27533).is_none();
            // Within its ten seconds, no answer; a ward unused runs out.
            pass(m, 1_000);
            bolt(m, foe, w, 2);
            let within = on(m, w, &ABSORBS);
            pass(m, 9_100);
            bolt(m, foe, w, 2);
            let again = on(m, w, &ABSORBS);
            // The breastplate taken off: the ward stays its time, and
            // there is no answer more.
            strip(m, w);
            let kept = (!gear(m, w, OBSIDIAN_ARMOR), on(m, w, &ABSORBS));
            pass(m, 6_100);
            let expired = on(m, w, &ABSORBS);
            pass(m, 5_000);
            bolt(m, foe, w, 2);
            let ran_out = (kept, expired, on(m, w, &ABSORBS));
            (
                worn,
                wards,
                capacity,
                frost,
                after_frost,
                partly,
                after_partly,
                wholly,
                used_up,
                within,
                again,
                ran_out,
            )
        })
        .await
        .unwrap();
    let (
        worn,
        wards,
        capacity,
        frost,
        after_frost,
        partly,
        after_partly,
        wholly,
        used_up,
        within,
        again,
        ran_out,
    ) = out;
    assert!(worn);
    assert_eq!(wards, vec![27533], "fire's ward");
    assert!((300..=500).contains(&capacity), "{capacity}");
    assert_eq!(frost, (50, 0), "frost goes through");
    assert_eq!(after_frost, Some(capacity));
    assert_eq!(partly, (0, 125));
    assert_eq!(after_partly, Some(capacity - 125));
    let rest = (capacity - 125) as u32;
    assert_eq!(wholly, (capacity as u32 - rest, rest));
    assert!(used_up, "used up, gone");
    assert!(within.is_empty(), "its ten seconds");
    assert_eq!(again, vec![27533], "after them");
    let (kept, expired, unanswered) = ran_out;
    assert_eq!(kept, (true, vec![27533]), "taken off, the ward stays");
    assert!(expired.is_empty(), "six seconds");
    assert!(unanswered.is_empty(), "no answer unworn");
}

/// Fire, nature, frost, shadow and arcane, by school index.
const SCHOOLS: [usize; 5] = [2, 3, 4, 5, 6];

/// Thistle Tea's "every Mage Armor rank enables the correct resistance
/// and expiry restores the baseline", "removing equipment prevents new
/// procs but lets an existing ward expire" and "cancelling Mage Armor
/// stops further warding without erasing the active resistance": under
/// any rank of Mage Armor, Adaptive Warding answers each school with 35
/// of its resistance, thirty seconds; without it, nothing. The regalia
/// taken off, or Mage Armor cancelled, there is no answer more, and the
/// resistance given stays its time; its bearer dead, it goes.
#[tokio::test]
#[ignore = "extended"]
async fn adaptive_warding_answers_each_school_under_mage_armor() {
    if !have_data() {
        return;
    }
    let (realm, _) = realm();
    let (_a, mage, map) = player(&realm, "U4A1", "Adaptone", Race::Human, Class::Mage).await;
    let (_b, foe, _) = player(&realm, "U4A2", "Adapttwo", Race::Human, Class::Mage).await;
    let resistance = move |m: &Map, school: usize| {
        m.players[&mage]
            .fields
            .get_u32(UNIT_FIELD_RESISTANCES + school as u16)
    };
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            dress(m, mage, &FROSTFIRE);
            let worn = gear(m, mage, ADAPTIVE_WARDING);
            // Without Mage Armor, no answer.
            bolt(m, foe, mage, 2);
            let bare = on(m, mage, &RESISTANCES);
            let mut ranks = Vec::new();
            for armor in MAGE_ARMOR {
                apply(m, mage, mage, armor);
                for (i, school) in SCHOOLS.into_iter().enumerate() {
                    derive::unit(m, mage);
                    let before = resistance(m, school);
                    bolt(m, foe, mage, school);
                    derive::unit(m, mage);
                    let answered = on(m, mage, &RESISTANCES);
                    let up = resistance(m, school) - before;
                    pass(m, 30_100);
                    derive::unit(m, mage);
                    let back = resistance(m, school) == before;
                    ranks.push((armor, answered == vec![RESISTANCES[i]], up, back));
                }
                take_off(m, mage, &[armor]);
            }
            // Taken off: the given stays, there is no answer more, and
            // death takes it.
            apply(m, mage, mage, 22783);
            bolt(m, foe, mage, 2);
            strip(m, mage);
            let unworn = !gear(m, mage, ADAPTIVE_WARDING);
            let stays = on(m, mage, &RESISTANCES);
            pass(m, 1_000);
            bolt(m, foe, mage, 3);
            let unanswered = on(m, mage, &RESISTANCES);
            death::kill(m, mage);
            let dead = on(m, mage, &RESISTANCES);
            death::resurrect(m, mage, 1.0);
            // Mage Armor cancelled: the same.
            dress(m, mage, &FROSTFIRE);
            derive::unit(m, mage);
            // Without Mage Armor's own resistance.
            let base = resistance(m, 2);
            apply(m, mage, mage, 22783);
            bolt(m, foe, mage, 2);
            auras::cancel(m, mage, 22783);
            derive::unit(m, mage);
            let cancelled = (!has(m, mage, 22783), resistance(m, 2) - base);
            bolt(m, foe, mage, 3);
            let after_cancel = on(m, mage, &RESISTANCES);
            pass(m, 30_100);
            derive::unit(m, mage);
            let expired = resistance(m, 2) == base;
            (
                worn,
                bare,
                ranks,
                unworn,
                stays,
                unanswered,
                dead,
                cancelled,
                after_cancel,
                expired,
            )
        })
        .await
        .unwrap();
    let (worn, bare, ranks, unworn, stays, unanswered, dead, cancelled, after_cancel, expired) =
        out;
    assert!(worn);
    assert!(bare.is_empty(), "no answer without Mage Armor");
    for (armor, answered, up, back) in ranks {
        assert_eq!((answered, up, back), (true, 35, true), "{armor}");
    }
    assert!(unworn);
    assert_eq!(stays, vec![28765], "the given stays");
    assert_eq!(unanswered, vec![28765], "no answer more");
    assert!(dead.is_empty(), "gone with death");
    assert_eq!(cancelled, (true, 35));
    assert_eq!(after_cancel, vec![28765], "no answer once cancelled");
    assert!(expired, "its thirty seconds");
}
