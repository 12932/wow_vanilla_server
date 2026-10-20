//! Round 9, the classes on the real data: Thistle Tea's `druid_test.exs`
//! (Faerie Fire keeps stealth off), `hunter_test.exs` (Improved Aspect of
//! the Hawk, Refocus), `priest_spells_test.exs` (an aura of no charges
//! hit), `warlock_spells_test.exs` (Healthstones, Inferno, Curse of
//! Idiocy), `warrior_spells_test.exs` (a spell's threat in a stance,
//! Charge) and `self_resurrection_test.exs` (a Soulstone through Spirit of
//! Redemption).

use std::time::Duration;

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::{self as kit, pass};
use crate::game::fields::*;
use crate::game::map::auras;
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::spells::{self, class_scripts};

fn threat(
    m: &crate::game::map::Map,
    c: wow_world_messages::Guid,
    g: wow_world_messages::Guid,
) -> f32 {
    m.creature(c)
        .and_then(|c| c.threat.iter().find(|(t, _)| *t == g).map(|(_, v)| *v))
        .unwrap_or(0.0)
}

/// Thistle Tea's "its DBC dispel immunity prevents stealth auras": a
/// druid under Faerie Fire can't take Prowl, and the fire stays as it was.
#[tokio::test]
#[ignore = "extended"]
async fn faerie_fire_keeps_prowl_off() {
    const FAERIE_FIRE: u32 = 770;
    const PROWL: u32 = 5215;
    let realm = kit::realm();
    let (_c, d, map) = kit::player(&realm, "P9FF", "Druid", Race::NightElf, Class::Druid).await;
    let out = map
        .query(move |m| {
            kit::ready(m, d, 60, &[PROWL]);
            let f = kit::dummy(m, d, 60, 5.0);
            kit::apply(m, f, d, FAERIE_FIRE);
            let before = kit::holder(m, d, FAERIE_FIRE).map(|h| (h.caster, h.expires));
            kit::apply(m, d, d, PROWL);
            let after = kit::holder(m, d, FAERIE_FIRE).map(|h| (h.caster, h.expires));
            (before.is_some(), kit::has(m, d, PROWL), before == after)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (true, false, true),
        "fire on, no Prowl, the fire as it was"
    );
}

/// Thistle Tea's "keeps the aspect's base chance and applies the current
/// aura-107 rank at proc time": Aspect of the Hawk's own chance is none;
/// Improved Aspect of the Hawk learned after the aspect is up counts at the
/// proc, by its rank.
#[tokio::test]
#[ignore = "extended"]
async fn improved_aspect_of_the_hawk_counts_at_the_proc() {
    const HAWK: u32 = 13_165;
    const IMPROVED: [(u32, f32); 2] = [(19_552, 1.0), (19_556, 5.0)];
    let realm = kit::realm();
    let (_c, h, map) = kit::player(&realm, "P9AH", "Hunter", Race::Dwarf, Class::Hunter).await;
    let out = map
        .query(move |m| {
            kit::ready(m, h, 60, &[]);
            kit::apply(m, h, h, HAWK);
            let s = m.realm.data.spells.get(HAWK).cloned().unwrap();
            let mut chances = vec![crate::game::map::procs::chance(
                m,
                h,
                &s,
                Hand::Ranged,
                true,
            )];
            for (talent, _) in IMPROVED {
                auras::remove_where(m, h, |x| IMPROVED.iter().any(|(t, _)| *t == x.spell));
                kit::apply(m, h, h, talent);
                chances.push(crate::game::map::procs::chance(
                    m,
                    h,
                    &s,
                    Hand::Ranged,
                    true,
                ));
            }
            (kit::has(m, h, HAWK), chances)
        })
        .await
        .unwrap();
    assert!(out.0, "the aspect up throughout");
    assert_eq!(out.1, vec![0.0, IMPROVED[0].1, IMPROVED[1].1]);
}

/// Thistle Tea's "refocus uses hunter family masks instead of spell IDs":
/// Refocus clears Aimed Shot's cooldown, of its family's flags, and leaves
/// a trap's.
#[tokio::test]
#[ignore = "extended"]
async fn refocus_clears_the_shots_its_flags_name() {
    const REFOCUS: u32 = 24_531;
    const AIMED_SHOT: u32 = 19_434;
    const TRAP: u32 = 13_795;
    let realm = kit::realm();
    let (_c, h, map) = kit::player(&realm, "P9RF", "Hunter", Race::Dwarf, Class::Hunter).await;
    let out = map
        .query(move |m| {
            kit::ready(m, h, 60, &[AIMED_SHOT, TRAP]);
            let now = m.now;
            let c = m.caster_mut(h).unwrap();
            c.cooldowns.insert(AIMED_SHOT, now + Duration::from_secs(7));
            c.cooldowns.insert(TRAP, now + Duration::from_secs(16));
            spells::cast_triggered(m, h, REFOCUS, Some(h));
            let c = m.caster(h).unwrap();
            (
                c.cooldowns.contains_key(&AIMED_SHOT),
                c.cooldowns.contains_key(&TRAP),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (false, true),
        "Aimed Shot ready, the trap still cooling"
    );
}

/// Thistle Tea's "holders without charges are untouched by hits": an aura
/// of no charges (Inner Fire with its charges spent to none, as Thistle
/// Tea's fixture has it) is hit and stays, with none.
#[tokio::test]
#[ignore = "extended"]
async fn an_aura_of_no_charges_is_untouched_by_hits() {
    const INNER_FIRE: u32 = 588;
    let realm = kit::realm();
    let (_c, p, map) = kit::player(&realm, "P9IF", "Priest", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            kit::ready(m, p, 60, &[]);
            kit::apply(m, p, p, INNER_FIRE);
            // The data's twenty charges gone, as Thistle Tea's fixture has
            // none.
            if let Some(h) = auras::auras_mut(m, p)
                .and_then(|a| a.holders.iter_mut().find(|h| h.spell == INNER_FIRE))
            {
                h.charges = 0;
            }
            let charges = kit::holder(m, p, INNER_FIRE).map(|h| h.charges);
            let k = kit::dummy(m, p, 60, 2.0);
            for _ in 0..3 {
                crate::game::tests_p3_combat::forced_swing(m, k, p, Outcome::Hit);
                if let Some(c) = m.combat_mut(k) {
                    c.next_swing = None;
                }
            }
            let hit = kit::health(m, p) < m.unit_fields(p).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            (
                charges,
                hit,
                kit::holder(m, p, INNER_FIRE).map(|h| h.charges),
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, Some(0), "no charges");
    assert!(out.1, "hit");
    assert_eq!(out.2, Some(0), "there still, with none");
}

/// Thistle Tea's "turns the rank script effect into the matching item
/// event": Create Healthstone makes its stone, the greater one by the
/// rank of Improved Healthstone known.
#[tokio::test]
#[ignore = "extended"]
async fn a_healthstone_is_made_by_the_rank_of_improved_healthstone() {
    const CREATE: u32 = 6201;
    let realm = kit::realm();
    let (_c, w, map) = kit::player(&realm, "P9HS", "Warlock", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[]);
            spells::cast_triggered(m, w, CREATE, Some(w));
            let plain = m.players[&w].carried_count_of(5512);
            kit::ready(m, w, 60, &[18_693]);
            spells::cast_triggered(m, w, CREATE, Some(w));
            (plain, m.players[&w].carried_count_of(19_005))
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (1, 1),
        "a Minor Healthstone; with the second rank, its best"
    );
}

/// Thistle Tea's "Inferno carries only the VMangos post-summon spell
/// script": Inferno's infernal is enslaved to its warlock and rooted as
/// it lands; the same spell without its script does neither.
#[tokio::test]
#[ignore = "extended"]
async fn infernos_infernal_is_enslaved_and_rooted_by_its_script_alone() {
    const INFERNO: u32 = 1122;
    const ENSLAVED: u32 = 20_882;
    const ROOTED: u32 = 22_707;
    let realm = kit::realm();
    let (_c, w, map) = kit::player(&realm, "P9IN", "Warlock", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[]);
            let s = m.realm.data.spells.get(INFERNO).cloned().unwrap();
            let mut plain = s.clone();
            plain.script_name = String::new();
            let first = kit::spawn(m, w, 89, 60, 5.0);
            m.creature_mut(first).unwrap().owner = Some(w);
            class_scripts::after_effects(m, w, &plain);
            let unscripted = (kit::has(m, first, ENSLAVED), kit::has(m, first, ROOTED));
            class_scripts::after_effects(m, w, &s);
            let scripted = (kit::has(m, first, ENSLAVED), kit::has(m, first, ROOTED));
            (s.script_name.clone(), unscripted, scripted)
        })
        .await
        .unwrap();
    assert_eq!(out.0, "spell_warlock_inferno");
    assert_eq!(out.1, (false, false), "without its script");
    assert_eq!(out.2, (true, true), "enslaved and rooted");
}

/// Thistle Tea's "periodic stacking stops after both encoded stat losses
/// reach the VMangos cap": Curse of Idiocy pulses until its target has
/// lost ninety of intellect and of spirit to it.
#[tokio::test]
#[ignore = "extended"]
async fn curse_of_idiocy_stops_at_ninety_of_both() {
    const IDIOCY: u32 = 1010;
    let realm = kit::realm();
    let (_c, w, map) = kit::player(&realm, "P9CI", "Warlock", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[]);
            let k = kit::dummy(m, w, 60, 5.0);
            kit::apply(m, w, k, IDIOCY);
            let mut allowed = Vec::new();
            for stacks in [12, 13] {
                if let Some(h) = auras::auras_mut(m, k)
                    .and_then(|a| a.holders.iter_mut().find(|h| h.spell == IDIOCY))
                {
                    h.stacks = stacks;
                    for e in h
                        .effects
                        .iter_mut()
                        .filter(|e| e.kind == auras::AURA_MOD_STAT)
                    {
                        e.amount = -7;
                    }
                }
                let h = kit::holder(m, k, IDIOCY).unwrap();
                allowed.push(class_scripts::periodic_trigger_allowed(m, k, &h));
            }
            allowed
        })
        .await
        .unwrap();
    assert_eq!(out, vec![true, false], "84 of each lost, then 91");
}

/// Thistle Tea's "flat spell threat lands on the mob scaled by the stance
/// multiplier": Sunder Armor's 45 threat, in Defensive Stance's 1.3.
#[tokio::test]
#[ignore = "extended"]
async fn a_spells_flat_threat_takes_the_stances_multiplier() {
    const DEFENSIVE_STANCE: u32 = 71;
    const SUNDER: u32 = 7386;
    let realm = kit::realm();
    let (_c, w, map) = kit::player(&realm, "P9ST", "Warrior", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[]);
            spells::cast_triggered(m, w, DEFENSIVE_STANCE, Some(w));
            let k = kit::dummy(m, w, 60, 2.0);
            spells::cast_triggered(m, w, SUNDER, Some(k));
            (kit::has(m, k, SUNDER), threat(m, k, w))
        })
        .await
        .unwrap();
    assert!(out.0, "sundered");
    assert!((out.1 - 58.5).abs() < 0.01, "45 x 1.3: {}", out.1);
}

/// Thistle Tea's "completing a charge cast queues the charge movement
/// event": Charge cast, its warrior is on the way, to strike at the end.
#[tokio::test]
#[ignore = "extended"]
async fn a_charge_cast_sets_its_warrior_on_the_way() {
    const CHARGE: u32 = 100;
    let realm = kit::realm();
    let (_c, w, map) = kit::player(&realm, "P9CH", "Warrior", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, w, 60, &[CHARGE]);
            let k = kit::dummy(m, w, 60, 15.0);
            let cast = kit::cast_fresh(m, w, CHARGE, kit::at(k));
            (cast, crate::game::map::charge::charging(m, w))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true));
}

/// Thistle Tea's "retains protection through Spirit of Redemption's final
/// death": a priest with a Soulstone on who falls into Spirit of
/// Redemption is offered the Soulstone's rising when the spirit ends in
/// death.
#[tokio::test]
#[ignore = "extended"]
async fn a_soulstone_holds_through_spirit_of_redemption() {
    const SPIRIT_OF_REDEMPTION: u32 = 20_711;
    const SPIRIT: u32 = 27_827;
    const SOULSTONE: u32 = 20_707;
    const SOULSTONE_RES: u32 = 3026;
    let realm = kit::realm();
    let (_c, p, map) = kit::player(&realm, "P9SR", "Priest", Race::Human, Class::Priest).await;
    let out = map
        .query(move |m| {
            kit::ready(m, p, 60, &[SPIRIT_OF_REDEMPTION]);
            kit::apply(m, p, p, SOULSTONE);
            let k = kit::dummy(m, p, 60, 2.0);
            let h = kit::health(m, p);
            combat::deal_damage(m, k, p, h);
            let spirit = kit::has(m, p, SPIRIT);
            for _ in 0..16 {
                pass(m, 1_000);
            }
            let dead = m.unit_view(p).is_some_and(|u| !u.alive);
            (
                spirit,
                dead,
                m.players[&p].fields.get_u32(PLAYER_SELF_RES_SPELL),
            )
        })
        .await
        .unwrap();
    assert!(out.0, "the spirit");
    assert!(out.1, "dead at its end");
    assert_eq!(out.2, SOULSTONE_RES, "the Soulstone's rising offered");
}
