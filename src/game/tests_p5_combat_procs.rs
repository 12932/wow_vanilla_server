//! Phase 3's fifth round of combat tests, what a melee blow sets off
//! (Thistle Tea's `attack_feedback_test.exs`): charges, the hand a
//! weapon's talent wants, a proc's cooldown, glancing and crushing blows,
//! Flurry and extra attacks, Blade Flurry and Sweeping Strikes.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ProcEvent, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::map::procs::{self, Event};
use crate::game::map::{Map, combat};
use crate::game::tests_p3_combat::{self as base, *};

const HURT: u32 = 90_830;
const CHARGED: u32 = 90_831;
const SWORDS: u32 = 90_832;
const COOLED: u32 = 90_833;
const FLURRY: u32 = 12_966;
const BLADE_FLURRY: u32 = 13_877;
const SWEEPING_STRIKES: u32 = 12_292;
const SWEEPING_STRIKES_DAMAGE: u32 = 12_723;
const DAGGER: u32 = 63_521;

/// A proc aura of `kind`, going off on every melee blow, triggering
/// `trigger`, with `charges`.
fn proc_aura(id: u32, kind: u32, trigger: u32, charges: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Proc {id}"),
        equipped_item_class: -1,
        proc_flags: procs::PROC_DONE_MELEE,
        proc_chance: 100,
        proc_charges: charges,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        base_points: 9,
        base_dice: 1,
        target_a: 1,
        trigger_spell: trigger,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = base::data();
    let mut hurt = SpellTemplate {
        id: HURT,
        name: "Hurt".into(),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: 1,
        ..Default::default()
    };
    hurt.effects[0] = SpellEffect {
        effect: 2,
        base_points: 0,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut swords = proc_aura(SWORDS, AURA_PROC_TRIGGER_SPELL, HURT, 0);
    swords.equipped_item_class = 2;
    swords.equipped_item_subclass_mask = 1 << 7;
    for s in [
        hurt,
        proc_aura(CHARGED, AURA_PROC_TRIGGER_SPELL, HURT, 1),
        swords,
        proc_aura(COOLED, AURA_PROC_TRIGGER_SPELL, HURT, 0),
        proc_aura(FLURRY, AURA_MOD_HASTE, 0, 3),
        proc_aura(BLADE_FLURRY, AURA_MOD_HASTE, 0, 0),
        proc_aura(SWEEPING_STRIKES, AURA_DUMMY, 0, 5),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.proc_events.insert(
        COOLED,
        ProcEvent {
            cooldown: 10_000,
            ..Default::default()
        },
    );
    d.items.insert(
        DAGGER,
        crate::game::data::items::ItemTemplate {
            entry: DAGGER,
            class: 2,
            subclass: 15,
            name: "Dagger".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 1500,
            stackable: 1,
            ..Default::default()
        },
    );
    d
}

fn put_on(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    auras::apply(m, g, g, &s, 0, 10);
}

/// A melee blow of `g`'s on `victim`, of outcome `ex`.
fn blow(g: Guid, victim: Guid, ex: u32) -> Event {
    Event {
        actor: g,
        target: victim,
        actor_flags: procs::PROC_DONE_MELEE,
        target_flags: procs::PROC_TAKEN_MELEE,
        ex,
        damage: 50,
        school: 1,
        ..Default::default()
    }
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn charges(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    auras::auras(m, g)?
        .holders
        .iter()
        .find(|h| h.spell == spell)
        .map(|h| h.charges)
}

/// Thistle Tea's "DBC melee proc auras trigger their encoded spell and
/// spend charges", "sword specialization requires the weapon that actually
/// landed the hit", "VMangos proc cooldown prevents another melee trigger
/// until it expires" and "glancing and crushing attacks can proc": a proc
/// of one charge casts its spell on the victim and is gone; one held to
/// swords goes off for the sword's hand and not the dagger's; one with a
/// cooldown waits it out; a glancing or crushing blow procs as a hit.
#[tokio::test]
async fn a_melee_proc_spends_its_charge_wants_its_weapon_and_waits_its_cooldown() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            equip(m, ga, SWORD, SLOT_MAIN_HAND);
            equip(m, ga, DAGGER, SLOT_OFF_HAND);
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            let hit = blow(ga, brute, procs::EX_NORMAL_HIT);
            // One charge.
            put_on(m, ga, CHARGED);
            let before = health(m, brute);
            procs::trigger(m, hit);
            let charged = (before - health(m, brute), charges(m, ga, CHARGED));
            // Swords: the main hand's blow, then the off hand's dagger.
            put_on(m, ga, SWORDS);
            let before = health(m, brute);
            procs::trigger(m, hit);
            let main = before - health(m, brute);
            let before = health(m, brute);
            procs::trigger(
                m,
                Event {
                    offhand: true,
                    ..hit
                },
            );
            let off = before - health(m, brute);
            auras::remove_spell(m, ga, SWORDS, None);
            // Ten seconds of cooldown.
            put_on(m, ga, COOLED);
            let before = health(m, brute);
            procs::trigger(m, hit);
            procs::trigger(m, hit);
            let cooled = before - health(m, brute);
            let ready = auras::auras(m, ga)
                .and_then(|a| a.holders.iter().find(|h| h.spell == COOLED))
                .and_then(|h| h.proc_ready_at)
                .map(|t| t.duration_since(m.now).as_millis());
            // Glancing and crushing.
            let before = health(m, brute);
            for o in [combat::Outcome::Glancing, combat::Outcome::Crushing] {
                if let Some(h) = auras::auras_mut(m, ga)
                    .and_then(|a| a.holders.iter_mut().find(|h| h.spell == COOLED))
                {
                    h.proc_ready_at = None;
                }
                procs::trigger(m, blow(ga, brute, combat::outcome_ex(o)));
            }
            let glanced = before - health(m, brute);
            (charged, (main, off), (cooled, ready), glanced)
        })
        .await
        .unwrap();
    let (charged, swords, cooled, glanced) = r;
    assert_eq!(charged, (1, None), "cast, and gone with its charge");
    assert_eq!(swords, (1, 0), "the sword's hand, not the dagger's");
    assert_eq!(cooled, (1, Some(10_000)), "once in ten seconds");
    assert_eq!(glanced, 2, "a glancing and a crushing blow proc");
}

/// Thistle Tea's "extra attacks cannot proc Flurry", "blade flurry does
/// not trigger from avoided attacks", "sweeping strikes spends a charge and
/// queues VMangos secondary damage" and "sweeping strikes does not chain
/// from its triggered damage spell": Flurry's haste spends a charge on a
/// swing but none on an extra attack's (mangos's `HandleDummyAuraProc`);
/// Blade Flurry puts a landed blow's damage on another enemy close by, and
/// nothing of a dodged one; Sweeping Strikes does the same and spends a
/// charge, and its own damage sweeps nothing on and spends none.
#[tokio::test]
async fn flurry_blade_flurry_and_sweeping_strikes_go_off_as_mangos_has_them() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            let other = spawn(m, ga, BRUTE, 20, 4.0);
            let hit = blow(ga, brute, procs::EX_NORMAL_HIT);
            put_on(m, ga, FLURRY);
            procs::trigger(
                m,
                Event {
                    extra_attack: true,
                    ..hit
                },
            );
            let extra = charges(m, ga, FLURRY);
            procs::trigger(m, hit);
            let swing = charges(m, ga, FLURRY);
            auras::remove_spell(m, ga, FLURRY, None);
            put_on(m, ga, BLADE_FLURRY);
            let before = health(m, other);
            procs::trigger(m, blow(ga, brute, procs::EX_DODGE));
            let dodged = before - health(m, other);
            procs::trigger(m, hit);
            let landed = before - health(m, other);
            auras::remove_spell(m, ga, BLADE_FLURRY, None);
            put_on(m, ga, SWEEPING_STRIKES);
            let before = health(m, other);
            procs::trigger(m, hit);
            let swept = (before - health(m, other), charges(m, ga, SWEEPING_STRIKES));
            let before = health(m, other);
            procs::trigger(
                m,
                Event {
                    actor_flags: procs::PROC_DONE_MELEE_SPELL | procs::PROC_DONE_MELEE,
                    spell: Some(SWEEPING_STRIKES_DAMAGE),
                    ..hit
                },
            );
            let looped = (before - health(m, other), charges(m, ga, SWEEPING_STRIKES));
            ((extra, swing), (dodged, landed), swept, looped)
        })
        .await
        .unwrap();
    let (flurry, blade, swept, looped) = r;
    assert_eq!(flurry, (Some(3), Some(2)), "no charge for an extra attack");
    assert_eq!(blade, (0, 50), "a landed blow's fifty on the other");
    assert_eq!(swept, (50, Some(4)), "swept, a charge spent");
    assert_eq!(looped, (0, Some(4)), "its own damage sweeps nothing on");
}
