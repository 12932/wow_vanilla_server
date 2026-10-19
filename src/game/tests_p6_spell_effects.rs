//! Phase 3, round 6, against Thistle Tea's `mechanical_healing_test.exs`
//! and `spell_teaching_test.exs`: repairing a mechanical friend, and
//! spells that teach spells and skill steps.

use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, health, set_health};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::{CreatureTemplate, FactionTemplate, GameData};
use crate::game::fields::*;
use crate::game::map::auras::AURA_MOD_HEALING_PCT;
use crate::game::map::spells::{self, checks, fail, mods::Mods};
use crate::game::map::{Map, skills};

/// A mechanical friend, a beast friend, and a mechanical foe.
const GOLEM: u32 = 9_001;
const PET: u32 = 9_002;
const FOE: u32 = 9_003;
/// Repair of 700 at a friendly mechanical unit; a halving of healing taken.
const REPAIR: u32 = 990_930;
const HALVED: u32 = 990_931;

fn creature(entry: u32, creature_type: u32, faction: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Made-up {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 60,
        level_max: 60,
        faction,
        creature_type,
        ..Default::default()
    }
}

fn faction(id: u32, faction: u32, ours: u32, friends: u32, enemies: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction,
        ours,
        friend_mask: friends,
        enemy_mask: enemies,
        ..Default::default()
    }
}

fn repairs(d: &mut GameData) {
    // A human's template, Stormwind's, a monster's.
    d.faction_templates.insert(1, faction(1, 1, 0x3, 0x2, 0x8));
    d.faction_templates
        .insert(11, faction(11, 72, 0x2, 0x2, 0xC));
    d.faction_templates.insert(14, faction(14, 14, 0x8, 0, 0x7));
    for t in [
        creature(GOLEM, 9, 11),
        creature(PET, 1, 11),
        creature(FOE, 9, 14),
    ] {
        d.creature_templates.insert(t.entry, t);
    }
    let mut repair = bare(REPAIR, 0);
    repair.target_creature_type = 1 << 8;
    effect(
        &mut repair,
        0,
        SpellEffect {
            effect: spells::EFFECT_HEAL_MECHANICAL,
            base_points: 699,
            base_dice: 1,
            target_a: 21,
            ..Default::default()
        },
    );
    let mut halved = bare(HALVED, 0);
    halved.duration_index = LASTS_LONG;
    effect(&mut halved, 0, aura(AURA_MOD_HEALING_PCT, 0, -50, 1));
    let spells: Vec<SpellTemplate> = vec![repair, halved];
    insert(d, spells);
}

fn why(
    m: &mut Map,
    caster: wow_world_messages::Guid,
    target: wow_world_messages::Guid,
) -> Option<u8> {
    kit::fresh(m, caster);
    let s = m.realm.data.spells.get(REPAIR).cloned().unwrap();
    checks::check(m, caster, &s, &at(target), None, false, &Mods::default())
}

/// A repair mends a friendly mechanical unit by its amount, no more than
/// its health's maximum, and less for a cut to the healing it takes; it
/// is cast only at a living, friendly, mechanical unit (Thistle Tea's
/// mechanical healing; mangos's `EffectHealMechanical`, a heal of a
/// `TargetCreatureType` of mechanicals).
#[tokio::test]
async fn a_repair_mends_a_living_friendly_machine() {
    let realm = fast_realm(repairs);
    let (_c, a, map) = human(&realm, "P6EA", "Tinker", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[REPAIR]);
            let golem = kit::spawn(m, a, GOLEM, 60, 5.0);
            let beast = kit::spawn(m, a, PET, 60, 5.0);
            let foe = kit::spawn(m, a, FOE, 60, 5.0);
            let mut mended = Vec::new();
            for (from, halved) in [(100, false), (999_900, false), (100, true)] {
                set_health(m, golem, from);
                if halved {
                    kit::apply(m, golem, golem, HALVED);
                }
                let w = why(m, a, golem);
                assert!(cast_fresh(m, a, REPAIR, at(golem)), "{w:?}");
                mended.push(health(m, golem) - from);
            }
            let checks = (why(m, a, golem), why(m, a, beast), why(m, a, foe));
            set_health(m, golem, 0);
            let later = m.now + std::time::Duration::from_secs(600);
            if let Some(c) = m.creature_mut(golem) {
                c.life = crate::game::map::creature::Life::Dead {
                    corpse_until: later,
                    respawn_at: later,
                };
            }
            (mended, checks, why(m, a, golem))
        })
        .await
        .unwrap();
    let (mended, checks, dead) = out;
    assert_eq!(mended, [700, 100, 350], "its amount, capped, halved");
    assert_eq!(
        checks,
        (None, Some(fail::BAD_TARGETS), Some(fail::TARGET_ENEMY)),
        "a machine, a beast, a foe"
    );
    assert_eq!(dead, Some(fail::TARGETS_DEAD));
}

/// Two spells to learn and a cooking step: the spells for teaching.
const TAUGHT_A: u32 = 990_932;
const TAUGHT_B: u32 = 990_933;
const TEACH: u32 = 990_934;
const TEACH_AT: u32 = 990_935;
const COOKING: u32 = 185;
const BAD_STEP: u32 = 990_936;

fn teaching(d: &mut GameData) {
    let learn = |spell: u32, target: u32| SpellEffect {
        effect: spells::EFFECT_LEARN_SPELL,
        trigger_spell: spell,
        target_a: target,
        ..Default::default()
    };
    let step = |points: i32, target: u32| SpellEffect {
        effect: spells::EFFECT_SKILL_STEP,
        misc_value: COOKING as i32,
        base_points: points - 1,
        base_dice: 1,
        target_a: target,
        ..Default::default()
    };
    let teach = |id: u32, target: u32| {
        let mut s = bare(id, 0);
        effect(&mut s, 0, learn(TAUGHT_A, target));
        effect(&mut s, 1, learn(TAUGHT_B, target));
        effect(&mut s, 2, step(3, target));
        s
    };
    let mut bad = bare(BAD_STEP, 0);
    effect(&mut bad, 0, step(-2, 1));
    let spells: Vec<SpellTemplate> = vec![
        bare(TAUGHT_A, 0),
        bare(TAUGHT_B, 0),
        teach(TEACH, 1),
        teach(TEACH_AT, 25),
        bad,
    ];
    insert(d, spells);
}

/// A spell of two spells to learn and a skill step teaches both and raises
/// the skill's step and cap, keeping its value and slot; an NPC's teaching
/// cast reaches its player and teaches a creature nothing; a negative step
/// teaches nothing (Thistle Tea's `SpellTeaching`; mangos's
/// `EffectLearnSpell` and `EffectLearnSkill`).
#[tokio::test]
async fn a_teaching_spell_teaches_its_spells_and_steps_its_skill() {
    let realm = fast_realm(teaching);
    let (_c, a, map) = human(&realm, "P6EB", "Pupil", Class::Warrior).await;
    let (_d, b, _) = human(&realm, "P6EC", "Other", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[TEACH]);
            let p = m.players.get_mut(&a).unwrap();
            skills::learn_skill(p, COOKING, 4);
            let slot = (0..128u16)
                .find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == COOKING);
            let base = PLAYER_SKILL_INFO_1_1 + slot.unwrap() * 3;
            p.fields.set_u32(base + 1, 125 | (300 << 16));
            cast_fresh(m, a, TEACH, Default::default());
            let p = &m.players[&a];
            let known = (p.spells.contains(&TAUGHT_A), p.spells.contains(&TAUGHT_B));
            let skill = (
                p.fields.get_u32(base) >> 16,
                p.fields.get_u32(base + 1) & 0xFFFF,
                p.fields.get_u32(base + 1) >> 16,
            );
            // A trainer's cast at another player, and at a creature.
            let trainer = kit::dummy(m, b, 60, 3.0);
            spells::cast_triggered(m, trainer, TEACH_AT, Some(b));
            let other = &m.players[&b];
            let taught = (
                other.spells.contains(&TAUGHT_A),
                skills::max_value(other, COOKING),
            );
            let kobold = kit::dummy(m, b, 60, 4.0);
            spells::cast_triggered(m, trainer, TEACH_AT, Some(kobold));
            spells::cast_triggered(m, b, BAD_STEP, None);
            let after_bad = skills::max_value(&m.players[&b], COOKING);
            (
                known,
                skill,
                slot == (0..128u16).find(|i| {
                    m.players[&a].fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == COOKING
                }),
                taught,
                after_bad,
            )
        })
        .await
        .unwrap();
    let (known, skill, same_slot, taught, after_bad) = out;
    assert_eq!(known, (true, true), "both spells");
    assert_eq!(skill, (3, 125, 225), "step 3, its value kept, cap 225");
    assert!(same_slot);
    assert_eq!(taught, (true, Some(225)), "the trainer's pupil");
    assert_eq!(after_bad, Some(225), "a negative step teaches nothing");
}

/// A bandage channelled 8 s at 250 a second, of the bandage mechanic, and
/// the lockout it leaves: a minute's immunity to that mechanic.
const BANDAGE: u32 = 990_937;
const LASTS_8S: u32 = 902;
const LASTS_60S: u32 = 903;

fn bandages(d: &mut GameData) {
    use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellDuration};
    use crate::game::map::auras::{AURA_MECHANIC_IMMUNITY, AURA_PERIODIC_HEAL, INTERRUPT_DAMAGE};
    for (i, ms) in [(LASTS_8S, 8_000), (LASTS_60S, 60_000)] {
        d.spells.durations.insert(
            i,
            SpellDuration {
                base: ms,
                per_level: 0,
                max: ms,
            },
        );
    }
    let mut bandage = bare(BANDAGE, 0);
    bandage.script_name = "spell_first_aid".into();
    bandage.mechanic = 16;
    bandage.attributes[1] |= ATTR_EX_CHANNELED;
    bandage.duration_index = LASTS_8S;
    bandage.aura_interrupt_flags = INTERRUPT_DAMAGE;
    effect(
        &mut bandage,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_HEAL, 0, 250, 21)
        },
    );
    let mut lockout = bare(spells::class_scripts::RECENTLY_BANDAGED, 0);
    lockout.duration_index = LASTS_60S;
    effect(&mut lockout, 0, aura(AURA_MECHANIC_IMMUNITY, 16, 0, 21));
    insert(d, [bandage, lockout]);
}

/// A bandage at a friend heals them each second of its channel, the
/// lockout it leaves on them no bar to it, and the lockout stays when the
/// bandage is done; damage ends the healing, not the lockout. While it
/// lasts the friend can be bandaged by no one, themselves too
/// (TARGET_AURASTATE), until it runs out; the bandager's own lockout is no
/// bar to bandaging another (Thistle Tea's `FirstAid`; mangos's
/// `spell_first_aid`, Recently Bandaged a mechanic immunity).
#[tokio::test]
async fn a_bandage_heals_through_its_lockout_which_bars_the_next() {
    use crate::game::map::spells::class_scripts::RECENTLY_BANDAGED;
    let realm = fast_realm(bandages);
    let (_c, a, map) = human(&realm, "P6ED", "Medic", Class::Warrior).await;
    let (_d, b, _) = human(&realm, "P6EE", "Patient", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[BANDAGE]);
            kit::ready(m, b, 60, &[BANDAGE]);
            let near = m.players[&a].position();
            m.players.get_mut(&b).unwrap().movement.position = near;
            for g in [a, b] {
                if let Some(f) = m.unit_fields_mut(g) {
                    f.set_u32(UNIT_FIELD_MAXHEALTH, 3_000);
                }
                set_health(m, g, 100);
            }
            let s = m.realm.data.spells.get(BANDAGE).cloned().unwrap();
            kit::fresh(m, a);
            let w = checks::check(m, a, &s, &at(b), None, false, &Mods::default());
            assert!(cast_fresh(m, a, BANDAGE, at(b)), "{w:?}");
            assert!(kit::has(m, b, BANDAGE), "{:?}", kit::casting(m, a));
            let locked = kit::has(m, b, RECENTLY_BANDAGED);
            let mut healed = Vec::new();
            for _ in 0..8 {
                kit::pass(m, 1_000);
                healed.push(health(m, b));
            }
            let after = (kit::has(m, b, BANDAGE), kit::has(m, b, RECENTLY_BANDAGED));
            let barred = {
                let s = m.realm.data.spells.get(BANDAGE).cloned().unwrap();
                let by = |m: &mut Map, g| {
                    kit::fresh(m, g);
                    checks::check(m, g, &s, &at(b), None, false, &Mods::default())
                };
                (by(m, a), by(m, b))
            };
            // The lockout's minute out; bandaged again, then struck.
            crate::game::map::auras::remove_spell(m, b, RECENTLY_BANDAGED, None);
            set_health(m, b, 100);
            cast_fresh(m, a, BANDAGE, at(b));
            crate::game::map::combat::deal_damage(m, a, b, 1);
            kit::pass(m, 1_000);
            let struck = (
                health(m, b),
                kit::has(m, b, BANDAGE),
                kit::has(m, b, RECENTLY_BANDAGED),
            );
            // The medic locked out, the patient not.
            crate::game::map::auras::remove_spell(m, b, RECENTLY_BANDAGED, None);
            kit::apply(m, a, a, RECENTLY_BANDAGED);
            kit::fresh(m, a);
            let s = m.realm.data.spells.get(BANDAGE).cloned().unwrap();
            let other = checks::check(m, a, &s, &at(b), None, false, &Mods::default());
            (locked, healed, after, barred, struck, other)
        })
        .await
        .unwrap();
    let (locked, healed, after, barred, struck, other) = out;
    assert!(locked, "the lockout at once");
    assert_eq!(
        healed,
        [350, 600, 850, 1_100, 1_350, 1_600, 1_850, 2_100],
        "healing through it"
    );
    assert_eq!(after, (false, true), "the bandage done, the lockout stays");
    let aurastate = Some(fail::TARGET_AURASTATE);
    assert_eq!(barred, (aurastate, aurastate), "by another, by themselves");
    assert_eq!(struck, (99, false, true), "damage ends the healing");
    assert_eq!(other, None, "the medic's lockout bars no one else");
}

/// A gun and a vest of 100 durability; a mend of the ranged slot by
/// 50,000; a kill and a wear of the chest by 5%; a wear of other slots.
const GUN: u32 = 9_101;
const VEST: u32 = 9_102;
const MEND_RANGED: u32 = 990_938;
const KILL_AND_WEAR: u32 = 990_939;
const WEAR_SLOT: u32 = 990_940;

fn durability(d: &mut GameData) {
    use crate::game::data::items::ItemTemplate;
    for (entry, class, inventory_type) in [(GUN, 2, 26), (VEST, 4, 5)] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class,
                inventory_type,
                name: format!("Item {entry}"),
                display_id: 1000 + entry,
                stackable: 1,
                allowable_class: -1,
                allowable_race: -1,
                max_durability: 100,
                ..Default::default()
            },
        );
    }
    let wear = |effect: u32, points: i32, slot: i32| SpellEffect {
        effect,
        base_points: points - 1,
        base_dice: 1,
        misc_value: slot,
        target_a: 25,
        ..Default::default()
    };
    let mut mend = bare(MEND_RANGED, 0);
    mend.attributes[2] |= crate::game::data::spells::ATTR_EX2_ALLOW_DEAD_TARGET;
    effect(
        &mut mend,
        0,
        wear(spells::EFFECT_DURABILITY_DAMAGE, -50_000, 17),
    );
    let mut kill = bare(KILL_AND_WEAR, 0);
    effect(
        &mut kill,
        0,
        SpellEffect {
            effect: 1,
            target_a: 25,
            ..Default::default()
        },
    );
    effect(
        &mut kill,
        1,
        wear(spells::EFFECT_DURABILITY_DAMAGE_PCT, 5, 4),
    );
    insert(d, [mend, kill]);
    d.spells.spells.insert(WEAR_SLOT, {
        let mut s = bare(WEAR_SLOT, 0);
        effect(&mut s, 0, wear(spells::EFFECT_DURABILITY_DAMAGE, 5, 15));
        s
    });
}

/// Durability effects: a kill and a wear of the chest in one spell both
/// land, the wear on the one just killed; a mend by negative points
/// repairs a dead player's ranged weapon; a wear at a creature does
/// nothing, nor one of a slot that isn't gear (Thistle Tea's durability
/// effects; mangos's `EffectDurabilityDamage`, which skips creatures and
/// slots past the bags).
#[tokio::test]
async fn durability_effects_reach_the_dead_and_only_a_players_gear() {
    use crate::game::map::player::Item;
    use crate::game::store::BAG_PLAYER;
    let realm = fast_realm(durability);
    let (_c, a, map) = human(&realm, "P6EF", "Worn", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let d = m.realm.data.clone();
            let mut worn = Vec::new();
            for (entry, slot) in [(GUN, 17u8), (VEST, 4)] {
                let counter = m
                    .realm
                    .next_item
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let g = crate::game::guid::item(counter);
                let p = m.players.get_mut(&a).unwrap();
                p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
                p.items
                    .push(Item::new(&d, g, a, entry, BAG_PLAYER, slot, 1));
                worn.push(slot);
            }
            let durability = |m: &Map, slot: u8| {
                m.players[&a]
                    .items
                    .iter()
                    .find(|i| i.bag == BAG_PLAYER && i.slot == slot)
                    .map(|i| i.fields.get_u32(ITEM_FIELD_DURABILITY))
            };
            let kobold = kit::dummy(m, a, 60, 5.0);
            if let Some(i) = m
                .players
                .get_mut(&a)
                .unwrap()
                .items
                .iter_mut()
                .find(|i| i.slot == 17)
            {
                i.fields.set_u32(ITEM_FIELD_DURABILITY, 10);
            }
            spells::cast_triggered(m, kobold, KILL_AND_WEAR, Some(a));
            let killed = (m.unit_view(a).is_some_and(|u| u.alive), durability(m, 4));
            spells::cast_triggered(m, kobold, MEND_RANGED, Some(a));
            let mended = durability(m, 17);
            spells::cast_triggered(m, kobold, WEAR_SLOT, Some(kobold));
            spells::cast_triggered(m, kobold, WEAR_SLOT, Some(a));
            (killed, mended, durability(m, 4), durability(m, 17))
        })
        .await
        .unwrap();
    let (killed, mended, vest, gun) = out;
    assert_eq!(
        killed,
        (false, Some(85)),
        "killed (a death's 10%), and worn by 5% more"
    );
    assert_eq!(mended, Some(100), "a dead player's gun mended");
    assert_eq!(
        (vest, gun),
        (Some(85), Some(100)),
        "slot 15 is empty: nothing"
    );
}

/// A spell whose effect triggers a bolt of 10; an instant fire blast of
/// 5; 70 more fire damage; a quest's completion by spell.
const TRIGGERING: u32 = 990_941;
const TRIGGERED_BOLT: u32 = 990_942;
const FIRE_BLAST: u32 = 990_943;
const FIRE_POWER: u32 = 990_944;
const COMPLETES: u32 = 990_945;
const QUEST: u32 = 2_561;

fn triggers(d: &mut GameData) {
    use crate::game::map::auras::AURA_MOD_DAMAGE_DONE;
    let mut triggering = bare(TRIGGERING, 0);
    effect(
        &mut triggering,
        0,
        SpellEffect {
            effect: spells::EFFECT_TRIGGER_SPELL,
            trigger_spell: TRIGGERED_BOLT,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut bolt = bare(TRIGGERED_BOLT, 2);
    effect(&mut bolt, 0, damage(10));
    let mut blast = bare(FIRE_BLAST, 2);
    blast.spell_level = 20;
    effect(&mut blast, 0, damage(5));
    let mut power = bare(FIRE_POWER, 0);
    power.duration_index = LASTS_LONG;
    effect(&mut power, 0, aura(AURA_MOD_DAMAGE_DONE, 4, 70, 1));
    let mut completes = bare(COMPLETES, 0);
    effect(
        &mut completes,
        0,
        SpellEffect {
            effect: spells::EFFECT_QUEST_COMPLETE,
            misc_value: QUEST as i32,
            target_a: 1,
            ..Default::default()
        },
    );
    insert(d, [triggering, bolt, blast, power, completes]);
}

/// A trigger effect casts its spell at the unit it lands on; an instant's
/// spell damage bonus is by the least coefficient, a cast of a second and
/// a half's, 70 giving 30; a quest-completing effect credits the player
/// it lands on (Thistle Tea's trigger, coefficient and quest-complete
/// effects; mangos's `EffectTriggerSpell`, `SpellBonusWithCoeffs` and
/// `EffectQuestComplete`).
#[tokio::test]
async fn a_trigger_casts_an_instant_takes_the_least_coefficient_a_quest_completes() {
    let realm = fast_realm(triggers);
    let (_c, a, map) = human(&realm, "P6EG", "Triggered", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[TRIGGERING, FIRE_BLAST, COMPLETES]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            cast_fresh(m, a, TRIGGERING, at(kobold));
            let triggered = 1_000_000 - health(m, kobold);
            kit::apply(m, a, a, FIRE_POWER);
            set_health(m, kobold, 1_000_000);
            cast_fresh(m, a, FIRE_BLAST, at(kobold));
            let blast = 1_000_000 - health(m, kobold);
            m.players
                .get_mut(&a)
                .unwrap()
                .journal
                .add(crate::game::map::quests::Logged {
                    quest: QUEST,
                    ..Default::default()
                });
            cast_fresh(m, a, COMPLETES, Default::default());
            let p = &m.players[&a];
            let credited = p.journal.slot_of(QUEST).map(|i| p.journal.log[i].explored);
            (triggered, blast, credited)
        })
        .await
        .unwrap();
    assert_eq!(out, (10, 35, Some(true)));
}
