//! Round 6's combat tests of the classes' talents and racials on the real
//! world (Thistle Tea's `*_dbc_test.exs` in `test/game/core/combat`): Deep
//! Wounds, Shadowguard and Lightning Shield, the kill procs, Berserking and
//! Blood Fury, the mana drain procs and Shadowburn's shards.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{self, *};
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::map::procs::{self, Event};
use crate::game::map::{Map, MapHandle, MapKey, combat, derive};
use crate::game::tests_p3_combat::{equip, level, spawn};

const SWORD: u32 = 63_700;
const OFF_SWORD: u32 = 63_701;
/// A Kobold Vermin, for a creature of the real world.
const VERMIN: u32 = 6;

fn sword(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class: 2,
        subclass: 7,
        name: format!("Sword {entry}"),
        display_id: 1000,
        inventory_type: 13,
        delay: 2000,
        stackable: 1,
        max_durability: 100,
        ..Default::default()
    }
}

/// The real world with two test swords, and a player of `class` and
/// `race` at 60 in it.
async fn real(race: Race, class: Class) -> (Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let mut data = harness::load_real(&harness::real_data("db"));
    for t in [sword(SWORD), sword(OFF_SWORD)] {
        data.items.insert(t.entry, t);
    }
    let realm = harness::realm_with(data);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", race, class).await;
    // The Horde begin in Kalimdor.
    let horde = matches!(race, Race::Orc | Race::Undead | Race::Tauren | Race::Troll);
    let map = realm
        .maps
        .get(&realm, MapKey::world(if horde { 1 } else { 0 }));
    map.query(move |m| level(m, ga, 60)).await.unwrap();
    (realm, a, ga, map)
}

fn put_on(m: &mut Map, caster: Guid, target: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).cloned().expect("the spell");
    let points = crate::game::map::spells::points(&s, 0, 60);
    auras::apply(m, caster, target, &s, 0, points);
}

fn holder(m: &Map, g: Guid, spell: u32) -> Option<Holder> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .cloned()
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// Tick `spell`'s first effect on `target` now: the health it took.
fn tick(m: &mut Map, target: Guid, spell: u32, caster: Guid) -> u32 {
    let now = m.now;
    if let Some(h) =
        auras::auras_mut(m, target).and_then(|a| a.holders.iter_mut().find(|h| h.spell == spell))
    {
        h.effects[0].next_tick = Some(now);
    }
    let before = health(m, target);
    auras::tick_due(m, target, spell, caster);
    before.saturating_sub(health(m, target))
}

fn blow(actor: Guid, target: Guid, ex: u32) -> Event {
    Event {
        actor,
        target,
        actor_flags: procs::PROC_DONE_MELEE,
        target_flags: procs::PROC_TAKEN_MELEE,
        ex,
        damage: 100,
        school: 1,
        ..Default::default()
    }
}

const DEEP_WOUNDS: [(u32, i32); 3] = [(12_834, 6), (12_849, 12), (12_867, 18)];
const DEEP_WOUND: u32 = 12_721;

/// A warrior with a sword in each hand.
fn warrior(m: &mut Map, g: Guid) {
    m.players
        .get_mut(&g)
        .unwrap()
        .spells
        .push(derive::SPELL_DUAL_WIELD);
    equip(m, g, SWORD, SLOT_MAIN_HAND);
    equip(m, g, OFF_SWORD, SLOT_OFF_HAND);
    derive::unit(m, g);
}

/// The main hand's blow 120 in its fields, the off hand's 30 (what an
/// aura's recalculation puts back).
fn blows(m: &mut Map, g: Guid) {
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_f32(UNIT_FIELD_MINDAMAGE, 120.0);
    f.set_f32(UNIT_FIELD_MAXDAMAGE, 120.0);
    f.set_f32(UNIT_FIELD_MINOFFHANDDAMAGE, 20.0);
    f.set_f32(UNIT_FIELD_MAXOFFHANDDAMAGE, 40.0);
}

/// Thistle Tea's "critical weapon hits complete every talent rank's bleed
/// chain", "melee abilities use critical outcome eligibility" and "delayed
/// swing feedback retains the hand that dealt the critical hit": each rank
/// of Deep Wounds bleeds a crit's victim for a fifth, two or three of the
/// hand's average blow over four ticks three seconds apart in twelve
/// seconds, Heroic Strike's crit as a swing's; a hit or a miss nothing. The
/// off hand's crit bleeds by the off hand's blow (as its fields have it:
/// Thistle Tea halves it again).
#[tokio::test]
#[ignore = "extended"]
async fn deep_wounds_bleeds_each_ranks_share_of_the_crits_hand_over_four_ticks() {
    let (_realm, _a, ga, map) = real(Race::Human, Class::Warrior).await;
    let r = map
        .query(move |m| {
            warrior(m, ga);
            let mut ranks = Vec::new();
            for (talent, _) in DEEP_WOUNDS {
                put_on(m, ga, ga, talent);
                blows(m, ga);
                let victim = spawn(m, ga, VERMIN, 60, 3.0);
                procs::trigger(m, blow(ga, victim, procs::EX_NORMAL_HIT));
                procs::trigger(m, blow(ga, victim, procs::EX_MISS));
                let none = holder(m, victim, DEEP_WOUND).is_none();
                procs::trigger(m, blow(ga, victim, procs::EX_CRITICAL_HIT));
                let bleed = holder(m, victim, DEEP_WOUND);
                let shape = bleed.as_ref().map(|h| {
                    (
                        h.effects[0].amount,
                        h.duration,
                        h.effects[0].period.map(|p| p.as_millis()),
                    )
                });
                let ticks: Vec<u32> = (0..4).map(|_| tick(m, victim, DEEP_WOUND, ga)).collect();
                ranks.push((none, shape, ticks));
                auras::remove_spell(m, ga, talent, None);
            }
            // Heroic Strike's crit, and its hit.
            put_on(m, ga, ga, 12_867);
            blows(m, ga);
            let victim = spawn(m, ga, VERMIN, 60, 3.0);
            let strike = |ex| Event {
                actor_flags: procs::PROC_DONE_MELEE_SPELL,
                target_flags: procs::PROC_TAKEN_MELEE_SPELL,
                spell: Some(11_567),
                ..blow(ga, victim, ex)
            };
            procs::trigger(m, strike(procs::EX_NORMAL_HIT));
            let hit = holder(m, victim, DEEP_WOUND).is_none();
            procs::trigger(m, strike(procs::EX_CRITICAL_HIT));
            let crit = holder(m, victim, DEEP_WOUND).map(|h| h.effects[0].amount);
            // The off hand's crit.
            let other = spawn(m, ga, VERMIN, 60, 3.0);
            procs::trigger(
                m,
                Event {
                    offhand: true,
                    actor_flags: procs::PROC_DONE_MELEE | procs::PROC_DONE_OFFHAND,
                    ..blow(ga, other, procs::EX_CRITICAL_HIT)
                },
            );
            let off = holder(m, other, DEEP_WOUND).map(|h| h.effects[0].amount);
            (ranks, (hit, crit), off)
        })
        .await
        .unwrap();
    let (ranks, strike, off) = r;
    for ((_, amount), (none, shape, ticks)) in DEEP_WOUNDS.iter().zip(&ranks) {
        assert!(none, "a hit or a miss bleeds nothing");
        assert_eq!(
            *shape,
            Some((*amount, Some(12_000), Some(3_000))),
            "{ranks:?}"
        );
        assert_eq!(*ticks, vec![*amount as u32; 4], "four ticks");
    }
    assert_eq!(
        strike,
        (true, Some(18)),
        "Heroic Strike's crit, not its hit"
    );
    assert_eq!(off, Some(4), "30 x 0.6 over four");
}

/// Thistle Tea's "critical spells require a currently usable melee
/// weapon", "refresh replaces the old bleed and death clears its remaining
/// ticks" and "bleed immunity rejects the proc's aura": a spell's crit
/// bleeds the victim only with a whole melee weapon in hand (mangos's
/// `IsTriggeredAtSpellProcEvent` for the talent's weapon); another crit
/// puts the bleed on again, one of it; the victim's death takes it; one
/// immune to bleeding takes none.
#[tokio::test]
#[ignore = "extended"]
async fn deep_wounds_wants_a_whole_weapon_starts_over_and_spares_the_immune() {
    let (_realm, _a, ga, map) = real(Race::Human, Class::Warrior).await;
    let r = map
        .query(move |m| {
            warrior(m, ga);
            put_on(m, ga, ga, 12_867);
            let fireball = |victim| Event {
                actor_flags: procs::PROC_DONE_NEGATIVE_SPELL,
                target_flags: procs::PROC_TAKEN_NEGATIVE_SPELL,
                spell: Some(133),
                school: 4,
                ..blow(ga, victim, procs::EX_CRITICAL_HIT)
            };
            let armed = spawn(m, ga, VERMIN, 60, 3.0);
            procs::trigger(m, fireball(armed));
            let armed = holder(m, armed, DEEP_WOUND).is_some();
            // Broken, then gone.
            let p = m.players.get_mut(&ga).unwrap();
            let i = p.items.iter().position(|i| i.entry == SWORD).unwrap();
            p.items[i].fields.set_u32(ITEM_FIELD_MAXDURABILITY, 100);
            p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, 0);
            let broken = spawn(m, ga, VERMIN, 60, 3.0);
            procs::trigger(m, fireball(broken));
            let broken = holder(m, broken, DEEP_WOUND).is_some();
            m.players.get_mut(&ga).unwrap().items.remove(i);
            let bare = spawn(m, ga, VERMIN, 60, 3.0);
            procs::trigger(m, fireball(bare));
            let bare = holder(m, bare, DEEP_WOUND).is_some();
            // Again, and the victim dies.
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            blows(m, ga);
            let victim = spawn(m, ga, VERMIN, 60, 3.0);
            procs::trigger(m, blow(ga, victim, procs::EX_CRITICAL_HIT));
            tick(m, victim, DEEP_WOUND, ga);
            procs::trigger(m, blow(ga, victim, procs::EX_CRITICAL_HIT));
            let again: Vec<(i32, u32, Option<u32>)> = auras::auras(m, victim)
                .map(|a| {
                    a.holders
                        .iter()
                        .filter(|h| h.spell == DEEP_WOUND)
                        .map(|h| (h.effects[0].amount, h.stacks, h.duration))
                        .collect()
                })
                .unwrap_or_default();
            combat::kill(m, ga, victim);
            let after_death = holder(m, victim, DEEP_WOUND).is_some();
            // Immune to bleeding (mechanic 15).
            let immune = spawn(m, ga, VERMIN, 60, 3.0);
            crate::game::tests_p3_combat::aura_of(m, immune, 91_500, AURA_MECHANIC_IMMUNITY, 15, 0);
            procs::trigger(m, blow(ga, immune, procs::EX_CRITICAL_HIT));
            let spared = holder(m, immune, DEEP_WOUND).is_none();
            (armed, broken, bare, again, after_death, spared)
        })
        .await
        .unwrap();
    let (armed, broken, bare, again, after_death, spared) = r;
    assert!(armed, "a spell's crit with a sword");
    assert!(!broken && !bare, "not with it broken, nor none");
    assert_eq!(
        again,
        vec![(18, 1, Some(12_000))],
        "one bleed, started over"
    );
    assert!(!after_death, "gone with the victim");
    assert!(spared, "immune to bleeding");
}

const SHADOWGUARD: [(u32, u32, u32); 6] = [
    (18_137, 28_377, 20),
    (19_308, 28_378, 35),
    (19_309, 28_379, 51),
    (19_310, 28_380, 70),
    (19_311, 28_381, 90),
    (19_312, 28_382, 116),
];
const LIGHTNING_SHIELD: [(u32, u32); 7] = [
    (324, 13),
    (325, 29),
    (905, 51),
    (945, 80),
    (8134, 114),
    (10_431, 154),
    (10_432, 198),
];

fn charges(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    holder(m, g, spell).map(|h| h.charges)
}

/// Thistle Tea's "all ranks damage the attacker through triggered delivery
/// without damage threat", "attackers share the cooldown and the third
/// charge removes the aura" and "expiry and death remove the retaliatory
/// aura": each rank of Shadowguard strikes back at the one who struck the
/// priest for its own bolt, a charge spent and three and a half seconds
/// before the next, whoever strikes; the top rank's bolt gives no threat
/// (`spell_threat`); the third charge takes the aura. It lasts ten
/// minutes; dead, the priest has none, and a blow on them sets nothing off.
#[tokio::test]
#[ignore = "extended"]
async fn shadowguard_strikes_back_by_its_rank_three_times_three_and_a_half_seconds_apart() {
    let (_realm, _a, gp, map) = real(Race::Human, Class::Priest).await;
    let r = map
        .query(move |m| {
            let mut ranks = Vec::new();
            for (rank, _, _) in SHADOWGUARD {
                let kobold = spawn(m, gp, VERMIN, 60, 3.0);
                put_on(m, gp, gp, rank);
                let before = health(m, kobold);
                procs::trigger(m, blow(kobold, gp, procs::EX_NORMAL_HIT));
                let ready = holder(m, gp, rank)
                    .and_then(|h| h.proc_ready_at)
                    .map(|t| t.duration_since(m.now).as_millis());
                ranks.push((before - health(m, kobold), charges(m, gp, rank), ready));
                auras::remove_spell(m, gp, rank, None);
            }
            // The top rank: no threat; two strikers share the cooldown;
            // the third charge.
            let (a, b) = (spawn(m, gp, VERMIN, 60, 3.0), spawn(m, gp, VERMIN, 60, 4.0));
            put_on(m, gp, gp, 19_312);
            let lasts = holder(m, gp, 19_312).and_then(|h| h.duration);
            procs::trigger(m, blow(a, gp, procs::EX_NORMAL_HIT));
            let threat = m
                .creature(a)
                .and_then(|c| c.threat.iter().find(|t| t.0 == gp).map(|t| t.1));
            let a_before = health(m, b);
            procs::trigger(m, blow(b, gp, procs::EX_NORMAL_HIT));
            let cooling = (a_before - health(m, b), charges(m, gp, 19_312));
            let rearm = |m: &mut Map| {
                if let Some(h) = auras::auras_mut(m, gp)
                    .and_then(|x| x.holders.iter_mut().find(|h| h.spell == 19_312))
                {
                    h.proc_ready_at = None;
                }
            };
            rearm(m);
            procs::trigger(m, blow(b, gp, procs::EX_NORMAL_HIT));
            let second = charges(m, gp, 19_312);
            rearm(m);
            procs::trigger(m, blow(a, gp, procs::EX_NORMAL_HIT));
            let third = charges(m, gp, 19_312);
            // Dead.
            put_on(m, gp, gp, 19_312);
            combat::kill(m, a, gp);
            let dead = charges(m, gp, 19_312);
            let before = health(m, b);
            procs::trigger(m, blow(b, gp, procs::EX_NORMAL_HIT));
            (
                ranks,
                lasts,
                threat,
                cooling,
                (second, third),
                (dead, before - health(m, b)),
            )
        })
        .await
        .unwrap();
    let (ranks, lasts, threat, cooling, spent, dead) = r;
    for ((_, _, amount), got) in SHADOWGUARD.iter().zip(&ranks) {
        assert_eq!(*got, (*amount, Some(2), Some(3_500)), "{ranks:?}");
    }
    assert_eq!(lasts, Some(600_000), "ten minutes");
    assert!(threat.is_none_or(|t| t == 0.0), "{threat:?}");
    assert_eq!(cooling, (0, Some(2)), "the other striker waits it out");
    assert_eq!(spent, (Some(1), None), "the third charge takes it");
    assert_eq!(dead, (None, 0), "the dead have none");
}

/// Thistle Tea's Lightning Shield "all ranks resolve actual damage without
/// counter-proccing Shadowguard" and "cooldown, charge exhaustion, expiry
/// and death retain the shared lifecycle", Shadowguard's "Lightning Shield
/// retaliation cannot counter-proc Shadowguard" and "a spell absorbed by
/// Power Word Shield still consumes a Shadowguard charge": each rank of
/// Lightning Shield strikes back for its own amount, three charges three
/// and a half seconds apart, ten minutes; a striker's Shadowguard doesn't
/// answer it (a proc's damage procs only what allows it), but answers an
/// Earth Shock; a Shadow Bolt wholly absorbed by Power Word: Shield still
/// spends a charge of Shadowguard and strikes back.
#[tokio::test]
#[ignore = "extended"]
async fn lightning_shield_strikes_back_by_its_rank_and_no_proc_answers_it() {
    let (realm, _a, gs, map) = real(Race::Orc, Class::Shaman).await;
    let r = map
        .query(move |m| {
            let kobold = spawn(m, gs, VERMIN, 60, 3.0);
            put_on(m, kobold, kobold, 19_312);
            let mut ranks = Vec::new();
            for (rank, _) in LIGHTNING_SHIELD {
                put_on(m, gs, gs, rank);
                let before = health(m, kobold);
                procs::trigger(m, blow(kobold, gs, procs::EX_NORMAL_HIT));
                let h = holder(m, gs, rank);
                ranks.push((
                    before - health(m, kobold),
                    h.as_ref().map(|h| h.charges),
                    h.as_ref()
                        .and_then(|h| h.proc_ready_at)
                        .map(|t| t.duration_since(m.now).as_millis()),
                    h.and_then(|h| h.duration),
                ));
                auras::remove_spell(m, gs, rank, None);
            }
            // The kobold's Shadowguard answered none of it; an Earth Shock
            // from the shaman it answers.
            let untouched = charges(m, kobold, 19_312);
            let before = health(m, gs);
            crate::game::map::spells::cast_triggered(m, gs, 8042, Some(kobold));
            let answered = (charges(m, kobold, 19_312), before > health(m, gs));
            (ranks, untouched, answered)
        })
        .await
        .unwrap();
    let (ranks, untouched, answered) = r;
    for ((_, amount), got) in LIGHTNING_SHIELD.iter().zip(&ranks) {
        assert_eq!(
            *got,
            (*amount, Some(2), Some(3_500), Some(600_000)),
            "{ranks:?}"
        );
    }
    assert_eq!(untouched, Some(3), "no answer to a proc's damage");
    assert_eq!(answered, (Some(2), true), "an Earth Shock answered");
    // Power Word: Shield takes a whole Shadow Bolt; Shadowguard answers.
    let mut c = Client::connect(&realm, "B");
    let gp = c.enter("Bob", Race::Troll, Class::Priest).await;
    let r = map
        .query(move |m| {
            level(m, gp, 60);
            let kobold = spawn(m, gp, VERMIN, 60, 3.0);
            put_on(m, gp, gp, 19_312);
            put_on(m, gp, gp, 17);
            let before = (health(m, gp), health(m, kobold));
            crate::game::map::spells::cast_triggered(m, kobold, 686, Some(gp));
            (
                before.0 - health(m, gp),
                charges(m, gp, 19_312),
                before.1 > health(m, kobold),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (0, Some(2), true),
        "absorbed whole, a charge spent, struck back"
    );
}
