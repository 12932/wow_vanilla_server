//! Phase 9's world spells, a second part (Thistle Tea's
//! `world/loader/*_dbc_test.exs` on real spells, a map and a player): a
//! set's feral speed held to the forms, Starshards' ticks logged to both
//! ends, Improved Fire Totems' one blast a Fire Nova Totem, Plagueheart's
//! threat and Ascendance's charges and stacks.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::procs::{self, Event};
use crate::game::map::spells::{self, mods, outcome};
use crate::game::map::trinkets::{ASCENDANCE, ASCENDANCE_BONUS};
use crate::game::map::{Map, auras, inventory, pets};

const WARLORDS_SANCTUARY: u32 = 398;
const FERAL_SPEED: u32 = 23_218;
const CAT_FORM: u32 = 768;
/// Three of Warlord's Sanctuary: its helmet, hauberk and boots, in their
/// slots.
const SANCTUARY: [(u32, u8); 3] = [(16_550, 0), (16_549, 4), (16_554, 7)];

/// Thistle Tea's "rechecks the real feral speed bonus on form entry and
/// exit" (`item_set_dbc_test.exs`): Warlord's Sanctuary gives 23218 at
/// three pieces, 15% more speed in cat, travel, bear and dire bear forms
/// (stances 0x95). A druid wearing three runs at 7, without it; in Cat
/// Form at 8.05, with it; out of the form at 7 again, without it.
#[tokio::test]
#[ignore = "extended"]
async fn a_sets_feral_speed_holds_only_in_a_feral_form() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let set = &d.crafts.item_sets[&WARLORDS_SANCTUARY];
    assert!(set.spells.contains(&(FERAL_SPEED, 3)), "{:?}", set.spells);
    let speed = d.spells.get(FERAL_SPEED).unwrap();
    assert_eq!(
        (
            speed.effects[0].aura,
            spells::points(speed, 0, 60),
            speed.stances
        ),
        (auras::AURA_MOD_SPEED_ALWAYS, 15, 0x95)
    );
    let (_c, g, map) = player(&realm, "P9WT1", "Sanctified", Race::Tauren, Class::Druid).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            for (entry, slot) in SANCTUARY {
                crate::game::tests_p3_combat::equip(m, g, entry, slot);
            }
            inventory::sync_gear(m, g);
            let run = |m: &Map| (m.players[&g].speeds.run, has(m, g, FERAL_SPEED));
            let worn = run(m);
            apply(m, g, g, CAT_FORM);
            let cat = run(m);
            auras::remove_spell(m, g, CAT_FORM, None);
            (worn, cat, run(m))
        })
        .await
        .unwrap();
    let (worn, cat, left) = out;
    assert_eq!(worn, (7.0, false), "out of form");
    assert!((cat.0 - 8.05).abs() < 0.001 && cat.1, "{cat:?}");
    assert_eq!(left, (7.0, false), "the form left");
}

const STARSHARDS: u32 = 19_305;

/// The damage of the first `n` periodic damage logs of Starshards from
/// `caster` that `c` is sent.
async fn starshards_logged(c: &mut Client, caster: Guid, n: usize) -> Vec<u32> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(
            c.wait_for("Starshards' log", |m| match m {
                ServerOpcodeMessage::SMSG_PERIODICAURALOG(p)
                    if p.spell == STARSHARDS && p.caster == caster =>
                {
                    p.auras.iter().find_map(|a| match a.aura_type {
                        AuraLog_AuraType::PeriodicDamage { damage1, .. } => Some(damage1),
                        _ => None,
                    })
                }
                _ => None,
            })
            .await,
        );
    }
    out
}

/// Thistle Tea's "Starshards reports each actual health change to both
/// native packet recipients" (`periodic_damage_ramps_dbc_test.exs`): each
/// of Starshards' six ticks (19305, its last rank) on a player is logged
/// (`SMSG_PERIODICAURALOG`) to that player and to its caster, each with
/// the health the tick took.
#[tokio::test]
#[ignore = "extended"]
async fn starshards_logs_each_ticks_health_lost_to_its_target_and_caster() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut tc, target, map) =
        player(&realm, "P9WT2", "Starstruck", Race::NightElf, Class::Priest).await;
    let (mut cc, caster, _) =
        player(&realm, "P9WT3", "Starcaller", Race::NightElf, Class::Priest).await;
    cc.sees_on(&map, target).await;
    let lost = map
        .query(move |m| {
            ready(m, caster, 60, &[]);
            ready(m, target, 60, &[]);
            if let Some(f) = m.unit_fields_mut(target) {
                f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
                f.set_u32(UNIT_FIELD_HEALTH, 100_000);
            }
            apply(m, caster, target, STARSHARDS);
            (0..6)
                .map(|_| {
                    let before = health(m, target);
                    tick_now(m, target, STARSHARDS);
                    before - health(m, target)
                })
                .collect::<Vec<u32>>()
        })
        .await
        .unwrap();
    assert!(lost.iter().all(|l| *l > 0), "{lost:?}");
    assert!(lost[5] > lost[0], "it ramps: {lost:?}");
    assert_eq!(
        starshards_logged(&mut tc, caster, 6).await,
        lost,
        "the target's"
    );
    assert_eq!(
        starshards_logged(&mut cc, caster, 6).await,
        lost,
        "the caster's"
    );
}

/// Each rank of Fire Nova Totem and the pulse its totem works.
const FIRE_NOVA_TOTEMS: [(u32, u32); 5] = [
    (1535, 8443),
    (8498, 8504),
    (8499, 8505),
    (11_314, 11_310),
    (11_315, 11_311),
];
/// Improved Fire Totems' ranks: a second and two off the pulse's interval
/// (op 19, Fire Nova's flag 0x20) and the totem's time (op 1, the totems'
/// flag 0x8000000).
const IMPROVED_FIRE_TOTEMS: [u32; 2] = [16_086, 16_544];

/// Thistle Tea's "both Improved Fire Totems ranks shorten each Fire Nova
/// rank to one explosion" (`periodic_modifiers_dbc_test.exs`): untalented,
/// each Fire Nova Totem pulses every 4 seconds and lasts 5; with the first
/// rank 3 and 4, with the second 2 and 3. Put down and left, each blasts
/// once (its pulse ticks once) before it goes.
#[tokio::test]
#[ignore = "extended"]
async fn improved_fire_totems_leaves_each_fire_nova_totem_one_blast() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    for (i, id) in IMPROVED_FIRE_TOTEMS.into_iter().enumerate() {
        let e = d.spells.get(id).unwrap().effects;
        let less = -1000 * (i as i32 + 1);
        assert_eq!(
            [e[0], e[1]].map(|e| (e.aura, e.misc_value, e.base_points + 1, e.item_type)),
            [
                (
                    auras::AURA_ADD_FLAT_MODIFIER,
                    mods::OP_ACTIVATION_TIME as i32,
                    less,
                    0x20
                ),
                (
                    auras::AURA_ADD_FLAT_MODIFIER,
                    mods::OP_DURATION as i32,
                    less,
                    0x800_0000
                ),
            ]
        );
    }
    let (_c, shaman, map) = player(&realm, "P9WT4", "Novatwo", Race::Orc, Class::Shaman).await;
    let out = map
        .query(move |m| {
            ready(m, shaman, 60, &[]);
            let mut out = Vec::new();
            for talent in [None, Some(16_086), Some(16_544)] {
                if let Some(t) = talent {
                    apply(m, shaman, shaman, t);
                }
                for (summon, pulse) in FIRE_NOVA_TOTEMS {
                    spells::cast_triggered(m, shaman, summon, None);
                    let totem = m.players[&shaman]
                        .owned
                        .totems
                        .iter()
                        .flatten()
                        .next()
                        .copied()
                        .expect("a totem");
                    let now = m.now;
                    let lasts = pets::state(m, totem)
                        .and_then(|s| s.expires)
                        .map(|t| t.duration_since(now).as_millis() as u64);
                    let every = holder(m, totem, pulse)
                        .and_then(|h| h.effects.first().and_then(|x| x.period))
                        .map(|p| p.as_millis() as u64);
                    // Left to its time, a tenth of a second at once.
                    let mut blasts = 0;
                    let mut gone = false;
                    for _ in 0..80 {
                        pass(m, 100);
                        if let Some(h) = holder(m, totem, pulse) {
                            blasts = h.effects[0].ticks;
                        }
                        pets::update(m);
                        if m.creature(totem).is_none() {
                            gone = true;
                            break;
                        }
                    }
                    out.push((talent, summon, every, lasts, blasts, gone));
                }
                if let Some(t) = talent {
                    auras::remove_spell(m, shaman, t, None);
                }
            }
            out
        })
        .await
        .unwrap();
    for (talent, summon, every, lasts, blasts, gone) in out {
        let (e, l) = match talent {
            None => (4000, 5000),
            Some(16_086) => (3000, 4000),
            _ => (2000, 3000),
        };
        assert_eq!(
            (every, lasts, blasts, gone),
            (Some(e), Some(l), 1, true),
            "{talent:?} {summon}"
        );
    }
}

const PLAGUEHEART: u32 = 28_746;
const SALVATION: u32 = 1038;
const SHADOW_BOLT: u32 = 686;
const CORRUPTION: u32 = 172;
/// Every spell a crit.
const SURE_CRIT: u32 = 990_960;

/// Thistle Tea's "Plagueheart reduces critical spell threat and selected
/// periodic spells separately" (`spell_threat_dbc_test.exs`): Plagueheart
/// (28746) has a quarter less threat on a crit of any magic school (aura
/// 183, 126) and a quarter less for the spells of its mask (op 2), which
/// has Corruption's flag 0x2 and not Shadow Bolt's 0x1. A Shadow Bolt's
/// threat is its damage, three quarters of it on a crit; Corruption's
/// tick three quarters, and were it a crit 0.5625; under Blessing of
/// Salvation (30% less) a tick's 0.525, and 0.7 with Plagueheart gone
/// (mangos's `CalcThreat`: `SPELLMOD_THREAT`, the critical threat, then
/// the total threat modifier).
#[tokio::test]
#[ignore = "extended"]
async fn plagueheart_cuts_a_crits_threat_and_corruptions_apart() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut s = made_up(SURE_CRIT, 6, auras::AURA_MOD_SPELL_CRIT_CHANCE, 0, 100);
        s.duration_index = 21;
        d.spells.spells.insert(SURE_CRIT, s);
    });
    let d = realm.data.clone();
    let p = d.spells.get(PLAGUEHEART).unwrap();
    assert_eq!(
        [0, 1].map(|i| (
            p.effects[i].aura,
            p.effects[i].misc_value,
            spells::points(p, i, 60)
        )),
        [
            (auras::AURA_MOD_CRITICAL_THREAT, 126, -25),
            (auras::AURA_ADD_PCT_MODIFIER, mods::OP_THREAT as i32, -25)
        ]
    );
    assert_eq!(
        p.effects[1].item_type & 0x3,
        0x2,
        "Corruption's, not the bolt's"
    );
    let (_c, g, map) = player(&realm, "P9WT5", "Plagued", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            ready(m, g, 60, &[]);
            apply(m, g, g, PLAGUEHEART);
            let threat_of = |m: &Map, foe: Guid| {
                m.creature(foe)
                    .and_then(|c| c.threat.iter().find(|(x, _)| *x == g).map(|(_, t)| *t))
                    .unwrap_or(0.0)
            };
            // A spell's threat a point of the damage it did, and the damage.
            let per_point = |m: &mut Map, spell: u32| {
                let foe = dummy(m, g, 60, 5.0);
                if spell == CORRUPTION {
                    apply(m, g, foe, CORRUPTION);
                    tick_now(m, foe, CORRUPTION);
                } else {
                    spells::cast_triggered(m, g, spell, Some(foe));
                    pass(m, 2000);
                }
                let dealt = 1_000_000 - health(m, foe);
                (threat_of(m, foe) / dealt as f32, dealt)
            };
            let bolt = per_point(m, SHADOW_BOLT);
            let tick = per_point(m, CORRUPTION);
            apply(m, g, g, SURE_CRIT);
            let crit = per_point(m, SHADOW_BOLT);
            auras::remove_spell(m, g, SURE_CRIT, None);
            // A tick never crits: Corruption's crit by the threat's own
            // reckoning, on a hundred of damage's.
            let foe = dummy(m, g, 60, 5.0);
            crate::game::map::ai::add_threat(m, foe, g, 100.0);
            let s = m.realm.data.spells.get(CORRUPTION).cloned().unwrap();
            let by = mods::for_spell(m, g, &s);
            spells::threat(m, g, foe, &s, 100.0, &by, true);
            let tick_crit = threat_of(m, foe) / 100.0;
            apply(m, g, g, SALVATION);
            let saved = per_point(m, CORRUPTION).0;
            auras::remove_spell(m, g, PLAGUEHEART, None);
            let unplagued = per_point(m, CORRUPTION).0;
            (bolt, tick, crit, tick_crit, saved, unplagued)
        })
        .await
        .unwrap();
    let (bolt, tick, crit, tick_crit, saved, unplagued) = out;
    let near = |x: f32, y: f32| (x - y).abs() < 0.001;
    assert!(bolt.1 > 0 && tick.1 > 0, "{bolt:?} {tick:?}");
    assert!(crit.1 > bolt.1, "a crit: {crit:?} {bolt:?}");
    assert!(near(bolt.0, 1.0), "{bolt:?}");
    assert!(near(crit.0, 0.75), "{crit:?}");
    assert!(near(tick.0, 0.75), "{tick:?}");
    assert!(near(tick_crit, 0.5625), "{tick_crit}");
    assert!(near(saved, 0.525), "{saved}");
    assert!(near(unplagued, 0.7), "{unplagued}");
}

const RENEW: u32 = 139;
const ARCANE_EXPLOSION: u32 = 1449;
const BLIZZARD: u32 = 10;

/// `spell`'s cast by `g` ending, as the cast tells the procs.
fn cast_end(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let (done, _) = outcome::proc_flags(&s);
    procs::trigger(
        m,
        Event {
            actor: g,
            target: g,
            actor_flags: done,
            ex: procs::EX_CAST_END,
            spell: Some(spell),
            school: s.school_mask(),
            ..Default::default()
        },
    );
}

/// Thistle Tea's "Ascendance has six scripted charges and a permanent
/// five-stack bonus" (`stacking_proc_dbc_test.exs`): Ascendance (28200)
/// has no charges in the data and lasts 20 seconds, its bonus (28204)
/// lasts and stacks to five; put on, Ascendance holds six charges (mangos's
/// `HandleAuraProcTriggerSpell`); a Renew's cast gives a stack of the
/// bonus, 40 more fire damage and 75 more healing, and spends a charge;
/// an Arcane Explosion's and a Blizzard's, area spells, do nothing.
/// (Thistle Tea's Prayer of Healing, a party's, isn't asserted: wvs's
/// `trinkets::proc` counts it, its target 20 not among the areas it
/// skips; mangos skips none.)
#[tokio::test]
#[ignore = "extended"]
async fn ascendance_holds_six_charges_and_a_renew_gives_its_bonus() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let parent = d.spells.get(ASCENDANCE).unwrap();
    let bonus = d.spells.get(ASCENDANCE_BONUS).unwrap();
    assert_eq!(
        (parent.proc_charges, d.spells.duration(parent, 60)),
        (0, Some(20_000))
    );
    assert_eq!(
        (d.spells.duration(bonus, 60), bonus.stack_amount),
        (None, 5)
    );
    let (_c, g, map) = player(&realm, "P9WT6", "Ascendant", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[RENEW]);
            let power = |m: &Map| {
                let d = &m.players[&g].derived;
                (d.spell_power[2], d.healing_power)
            };
            let count = |m: &Map| {
                (
                    holder(m, g, ASCENDANCE).map(|h| h.charges),
                    holder(m, g, ASCENDANCE_BONUS).map(|h| h.stacks),
                )
            };
            apply(m, g, g, ASCENDANCE);
            let on = count(m);
            let was = power(m);
            cast_fresh(m, g, RENEW, at(g));
            let renewed = count(m);
            let gained = (power(m).0 - was.0, power(m).1 - was.1);
            let areas = [ARCANE_EXPLOSION, BLIZZARD].map(|id| {
                cast_end(m, g, id);
                count(m)
            });
            (on, renewed, gained, areas)
        })
        .await
        .unwrap();
    let (on, renewed, gained, areas) = out;
    assert_eq!(on, (Some(6), None), "six charges");
    assert_eq!(renewed, (Some(5), Some(1)), "a Renew's stack");
    assert_eq!(gained, (40, 75), "a stack's fire damage and healing");
    assert_eq!(areas, [renewed; 2], "the area spells', nothing");
}
