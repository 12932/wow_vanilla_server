//! Phase 3, round 5, against Thistle Tea's `paladin_spells_test.exs`, on
//! the real data: the judgements' procs and releases, the seals' procs on
//! blows that land and on whom, Holy Shock, Divine Intervention and
//! Blessing of Light.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::{
    self as kit, at, fresh, has, health, power, set_health, set_power,
};
use crate::game::data::spells::ATTR_EX2_CANT_CRIT;
use crate::game::fields::*;
use crate::game::map::procs::{self, EX_MISS, EX_NORMAL_HIT, Event};
use crate::game::map::{Map, spells};

const JUDGEMENT_OF_WISDOM: u32 = 20354;
const SEAL_OF_RIGHTEOUSNESS: u32 = 21084;
const SEAL_OF_LIGHT: u32 = 20165;
const SEAL_OF_LIGHT_HEAL: u32 = 20167;
const JUDGEMENT: u32 = 20271;
const JUDGEMENT_OF_LIGHT: u32 = 20185;
const HOLY_SHOCK: u32 = 20473;
const DIVINE_INTERVENTION: u32 = 19752;
const BLESSING_OF_LIGHT: u32 = 19977;
const HOLY_LIGHT: u32 = 639;
const RETRIBUTION_AURA: u32 = 7294;

fn realm() -> std::sync::Arc<crate::game::Realm> {
    kit::realm_plus(|d| {
        for id in [
            HOLY_LIGHT,
            19968,
            HOLY_SHOCK,
            25914,
            25912,
            SEAL_OF_LIGHT_HEAL,
        ] {
            if let Some(s) = d.spells.spells.get_mut(&id) {
                s.attributes[2] |= ATTR_EX2_CANT_CRIT;
                for e in s.effects.iter_mut() {
                    e.die_sides = e.die_sides.min(1);
                }
            }
        }
    })
}

/// A blow from `from` at `to`, landed or missed.
fn blow(m: &mut Map, from: Guid, to: Guid, ex: u32) {
    procs::trigger(
        m,
        Event {
            actor: from,
            target: to,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE | procs::PROC_TAKEN_DAMAGE,
            ex,
            damage: if ex == EX_MISS { 0 } else { 10 },
            school: 1,
            ..Default::default()
        },
    );
}

fn hardy(m: &mut Map, g: Guid) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
        f.set_u32(UNIT_FIELD_HEALTH, 100);
    }
}

/// Judgement of Wisdom on a kobold gives mana to whoever strikes it, by
/// its rank's energize (its half chance over forty blows); the Judgement
/// released from Seal of Righteousness hurts the kobold, from Seal of
/// Light puts Judgement of Light on it; the seal goes (Thistle Tea's
/// Judgement procs and Judgement validation tests).
#[tokio::test]
#[ignore = "extended"]
async fn judgements_give_to_the_striker_and_released_seals_land_on_the_victim() {
    if !kit::have_data() {
        return;
    }
    let realm = realm();
    let (_a, p, map) = kit::player(&realm, "P5PA", "Judger", Race::Human, Class::Paladin).await;
    let (_b, mage, _) = kit::player(&realm, "P5PB", "Striker", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, p, 60, &[JUDGEMENT, SEAL_OF_RIGHTEOUSNESS, SEAL_OF_LIGHT]);
            kit::ready(m, mage, 60, &[]);
            let kobold = kit::dummy(m, p, 60, 3.0);
            kit::apply(m, p, kobold, JUDGEMENT_OF_WISDOM);
            set_power(m, mage, 0, 0);
            for _ in 0..40 {
                blow(m, mage, kobold, EX_NORMAL_HIT);
            }
            let wisdom = power(m, mage, 0);
            // Released seals.
            let mut released = Vec::new();
            for seal in [SEAL_OF_RIGHTEOUSNESS, SEAL_OF_LIGHT] {
                let target = kit::dummy(m, p, 60, 3.0);
                fresh(m, p);
                spells::cast_triggered(m, p, seal, Some(p));
                let sealed = has(m, p, seal);
                fresh(m, p);
                let went = spells::cast(m, p, JUDGEMENT, at(target), None);
                released.push((
                    sealed,
                    went,
                    has(m, p, seal),
                    1_000_000 - health(m, target),
                    has(m, target, JUDGEMENT_OF_LIGHT),
                ));
            }
            (wisdom, released)
        })
        .await
        .unwrap();
    let (wisdom, released) = out;
    assert!(wisdom > 0, "the striker's mana");
    let (sealed, went, kept, hurt, light) = released[0];
    assert!(
        sealed && went && !kept,
        "Righteousness judged: {:?}",
        released[0]
    );
    assert!(hurt > 0 && !light, "its damage");
    let (sealed, went, kept, hurt, light) = released[1];
    assert!(sealed && went && !kept, "Light judged: {:?}", released[1]);
    assert!(light && hurt == 0, "its judgement on the kobold");
}

/// A seal procs on a blow that lands, not one avoided; Seal of Light
/// heals its paladin for the blows they strike, not for those they take
/// (Thistle Tea's `trigger_seal/2` tests). Retribution Aura strikes back
/// (its aura-derived effects).
#[tokio::test]
#[ignore = "extended"]
async fn seals_proc_on_blows_that_land_and_seal_of_light_heals_its_paladin() {
    if !kit::have_data() {
        return;
    }
    let realm = realm();
    let (_a, p, map) = kit::player(&realm, "P5PC", "Sealer", Race::Human, Class::Paladin).await;
    let out = map
        .query(move |m| {
            kit::ready(m, p, 60, &[]);
            let kobold = kit::dummy(m, p, 60, 3.0);
            hardy(m, p);
            spells::cast_triggered(m, p, SEAL_OF_LIGHT, Some(p));
            let mut missed = 0;
            for _ in 0..20 {
                blow(m, p, kobold, EX_MISS);
                missed += health(m, p) - 100;
                set_health(m, p, 100);
            }
            let mut taken = 0;
            for _ in 0..20 {
                blow(m, kobold, p, EX_NORMAL_HIT);
                taken += health(m, p).saturating_sub(100);
                set_health(m, p, 100);
            }
            let mut struck = 0;
            for _ in 0..60 {
                blow(m, p, kobold, EX_NORMAL_HIT);
                struck += health(m, p) - 100;
                set_health(m, p, 100);
            }
            let on_kobold = has(m, kobold, SEAL_OF_LIGHT_HEAL);
            // Retribution Aura strikes back at a blow, and stays.
            spells::cast_triggered(m, p, RETRIBUTION_AURA, Some(p));
            let before = health(m, kobold);
            procs::damage_shields(m, kobold, p);
            let retribution = (before - health(m, kobold), has(m, p, RETRIBUTION_AURA));
            (missed, taken, struck, on_kobold, retribution)
        })
        .await
        .unwrap();
    let (missed, taken, struck, on_kobold, retribution) = out;
    assert!(retribution.0 > 0 && retribution.1, "{retribution:?}");
    assert_eq!(missed, 0, "nothing from a miss");
    assert_eq!(taken, 0, "nothing from a blow taken");
    assert!(struck > 0, "the paladin healed for blows struck");
    assert!(!on_kobold);
}

/// Holy Shock heals a friend and hurts a foe; Divine Intervention kills its
/// paladin alone; Blessing of Light adds to a Holy Light on its wearer
/// what `blessing_of_light_bonus` says, by Holy Light's coefficient
/// (Thistle Tea's scripted spell effects).
#[tokio::test]
#[ignore = "extended"]
async fn holy_shock_divine_intervention_and_blessing_of_light() {
    if !kit::have_data() {
        return;
    }
    let realm = realm();
    let (_a, p, map) = kit::player(&realm, "P5PD", "Shocker", Race::Human, Class::Paladin).await;
    let (_b, b, _) = kit::player(&realm, "P5PE", "Friend", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, p, 60, &[HOLY_SHOCK, HOLY_LIGHT]);
            kit::ready(m, b, 60, &[]);
            let kobold = kit::dummy(m, p, 60, 3.0);
            hardy(m, b);
            spells::cast_triggered(m, p, HOLY_SHOCK, Some(b));
            let healed = health(m, b) > 100;
            spells::cast_triggered(m, p, HOLY_SHOCK, Some(kobold));
            let hurt = health(m, kobold) < 1_000_000;
            // Holy Light, then with the blessing.
            let light = |m: &mut Map| {
                set_health(m, b, 100);
                fresh(m, p);
                spells::cast_triggered(m, p, HOLY_LIGHT, Some(b));
                health(m, b) - 100
            };
            let t = m.realm.data.spells.get(HOLY_LIGHT).unwrap().clone();
            eprintln!(
                "DEBUG 639 {:?} fam {} flags {:x} script {:?}",
                t.effects[0], t.family_name, t.family_flags, t.script_name
            );
            let plain = light(m);
            spells::cast_triggered(m, p, BLESSING_OF_LIGHT, Some(b));
            let bonus = spells::scripts::blessing_of_light_bonus(m, b, true);
            let blessed = light(m);
            // Divine Intervention at the friend.
            hardy(m, b);
            spells::cast_triggered(m, p, DIVINE_INTERVENTION, Some(b));
            let alive = (
                m.unit_view(p).is_some_and(|u| u.alive),
                m.unit_view(b).is_some_and(|u| u.alive),
            );
            (healed, hurt, plain, bonus, blessed, alive)
        })
        .await
        .unwrap();
    let (healed, hurt, plain, bonus, blessed, alive) = out;
    assert!(healed, "Holy Shock heals a friend");
    assert!(hurt, "and hurts a foe");
    assert!(plain > 0 && bonus > 0, "{plain} {bonus}");
    assert_eq!(blessed as i32, plain as i32 + bonus, "the blessing's bonus");
    assert_eq!(alive, (false, true), "the paladin alone dies");
}
