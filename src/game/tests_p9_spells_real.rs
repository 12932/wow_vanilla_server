//! Round 9, against Thistle Tea's spell tests on the real data: the feral
//! passives held by the druid's forms and Enrage ended with Bear Form
//! (`passive_spells_dbc_test.exs`), Pyroclasm for a creature that never
//! learned the fire (`pyroclasm_dbc_test.exs`), and a disenchant and a
//! coating that cost nothing when cancelled (`casting_test.exs`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::{self as kit, has, holder};
use crate::game::map::auras::{self, AURA_MOD_CRIT_PERCENT};
use crate::game::map::enchant::{self, TEMP_SLOT};
use crate::game::map::procs::{self, EX_NORMAL_HIT, Event};
use crate::game::map::{Map, spells};

/// The feral talents: Sharpened Claws, Feral Instinct, Blood Frenzy,
/// Primal Fury and Feline Grace.
const SHARPENED_CLAWS: u32 = 16_944;
const FERAL: [u32; 5] = [SHARPENED_CLAWS, 16_951, 16_954, 16_961, 20_719];
const CAT_FORM: u32 = 768;
const BEAR_FORM: u32 = 5487;
const ENRAGE: u32 = 5229;
const BATTLE_STANCE: u32 = 2457;
const BERSERKER_STANCE: u32 = 2458;
const BERSERKER_RAGE: u32 = 18_499;
const FORM_BATTLE: u8 = 17;

/// Which of the feral talents `g` holds.
fn feral_held(m: &Map, g: Guid) -> Vec<u32> {
    FERAL.into_iter().filter(|s| has(m, g, *s)).collect()
}

/// `g`'s critical strike chance from its auras.
fn crit_percent(m: &Map, g: Guid) -> i32 {
    auras::auras(m, g).map_or(0, |a| {
        a.effects()
            .filter(|(_, x)| x.kind == AURA_MOD_CRIT_PERCENT)
            .map(|(_, x)| x.amount)
            .sum()
    })
}

/// A druid's feral talents hold in the forms their data names, none out of
/// form: in Cat Form Sharpened Claws (its 6% crit), Feral Instinct, Blood
/// Frenzy and Feline Grace, not Primal Fury; in Bear Form Sharpened Claws,
/// Feral Instinct and Primal Fury, not the cat's two. Bear Form cancelled
/// takes all five, and the Enrage cast in it; restoring the passives then
/// adds nothing. Berserker Rage names a stance but stays when it is left: a
/// warrior's goes on from Berserker Stance into Battle Stance (Thistle
/// Tea's "feral critical strike, stealth, proc and falling passives follow
/// their DBC forms" and "Enrage ends with Bear Form while
/// stance-independent warrior buffs survive"; mangos-classic's
/// `Aura::HandleShapeshiftBoosts`).
#[tokio::test]
#[ignore = "extended"]
async fn feral_passives_follow_their_forms_and_enrage_ends_with_bear() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_d, dr, map) = kit::player(&realm, "P9RA", "Feral", Race::Tauren, Class::Druid).await;
    let druid = map
        .query(move |m| {
            let mut learn = FERAL.to_vec();
            learn.push(ENRAGE);
            kit::ready(m, dr, 60, &learn);
            let out_of_form = feral_held(m, dr);
            spells::cast_triggered(m, dr, CAT_FORM, None);
            let cat = feral_held(m, dr);
            let claws = holder(m, dr, SHARPENED_CLAWS).and_then(|h| {
                h.effects
                    .iter()
                    .find(|x| x.kind == AURA_MOD_CRIT_PERCENT)
                    .map(|x| x.amount)
            });
            let crit = crit_percent(m, dr);
            spells::cast_triggered(m, dr, BEAR_FORM, None);
            let bear = feral_held(m, dr);
            let cat_left = has(m, dr, CAT_FORM);
            spells::cast_triggered(m, dr, ENRAGE, None);
            let enraged = has(m, dr, ENRAGE);
            auras::cancel(m, dr, BEAR_FORM);
            let normal = (
                feral_held(m, dr),
                has(m, dr, ENRAGE),
                has(m, dr, BEAR_FORM),
                auras::form(m, dr),
            );
            spells::restore_form_passives(m, dr);
            let restored = feral_held(m, dr);
            let rage_ends =
                spells::ends_with_form(m.realm.data.spells.get(BERSERKER_RAGE).unwrap());
            (
                out_of_form,
                cat,
                claws,
                crit,
                bear,
                cat_left,
                enraged,
                normal,
                restored,
                rage_ends,
            )
        })
        .await
        .unwrap();
    let (out_of_form, cat, claws, crit, bear, cat_left, enraged, normal, restored, rage_ends) =
        druid;
    assert_eq!(out_of_form, Vec::<u32>::new(), "out of form");
    assert_eq!(cat, vec![16_944, 16_951, 16_954, 20_719], "in Cat Form");
    assert_eq!((claws, crit), (Some(6), 6), "Sharpened Claws' crit");
    assert_eq!(bear, vec![16_944, 16_951, 16_961], "in Bear Form");
    assert!(!cat_left, "Bear Form takes Cat Form's place");
    assert!(enraged, "Enrage in Bear Form");
    assert_eq!(
        normal,
        (Vec::new(), false, false, 0),
        "Bear Form cancelled: its passives and Enrage gone"
    );
    assert_eq!(restored, Vec::<u32>::new(), "restoring adds nothing");
    assert!(!rage_ends, "Berserker Rage doesn't end with its stance");
    let (_w, wr, wmap) = kit::player(&realm, "P9RB", "Stanced", Race::Human, Class::Warrior).await;
    let warrior = wmap
        .query(move |m| {
            kit::ready(
                m,
                wr,
                60,
                &[BATTLE_STANCE, BERSERKER_STANCE, BERSERKER_RAGE],
            );
            spells::cast_triggered(m, wr, BERSERKER_STANCE, None);
            spells::cast_triggered(m, wr, BERSERKER_RAGE, None);
            let raging = has(m, wr, BERSERKER_RAGE);
            spells::cast_triggered(m, wr, BATTLE_STANCE, None);
            (raging, auras::form(m, wr), has(m, wr, BERSERKER_RAGE))
        })
        .await
        .unwrap();
    assert_eq!(
        warrior,
        (true, FORM_BATTLE, true),
        "Berserker Rage kept into Battle Stance"
    );
}

const PYROCLASM: u32 = 18_073;
const PYROCLASM_STUN: u32 = 18_093;
const HELLFIRE_EFFECT: u32 = 5857;

/// A kobold carrying Pyroclasm, with no Hellfire of its own, stuns the
/// player its Hellfire Effect hits for 100, by the talent's chance a tick:
/// the stun is the kobold's, and the talent stays on it. wvs looks the fire
/// up in the data, not in a spellbook (Thistle Tea's "creature owners
/// retain unlearned damage spells in proc feedback"; mangos-classic's
/// `Unit::HandleProcTriggerSpellAuraProc`).
#[tokio::test]
#[ignore = "extended"]
async fn pyroclasm_stuns_for_a_creature_that_never_learned_the_fire() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_a, w, map) = kit::player(&realm, "P9RC", "Stunned", Race::Human, Class::Warrior).await;
    let (tries, stunner, mob, kept) = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, w, 60, &[]);
            let mob = kit::dummy(m, w, 60, 5.0);
            kit::apply(m, mob, mob, PYROCLASM);
            // A 26% chance spread over Hellfire's fifteen ticks: a stun in
            // a few thousand hits is all but certain.
            let tries = (1..=3_000u32).find(|_| {
                auras::remove_spell(m, w, PYROCLASM_STUN, None);
                if let Some(c) = m.caster_mut(w) {
                    c.diminishing.clear();
                }
                procs::trigger(
                    m,
                    Event {
                        actor: mob,
                        target: w,
                        actor_flags: procs::PROC_DONE_NEGATIVE_SPELL,
                        target_flags: procs::PROC_TAKEN_NEGATIVE_SPELL | procs::PROC_TAKEN_DAMAGE,
                        ex: EX_NORMAL_HIT,
                        spell: Some(HELLFIRE_EFFECT),
                        damage: 100,
                        school: 4,
                        ..Default::default()
                    },
                );
                has(m, w, PYROCLASM_STUN)
            });
            let stunner = holder(m, w, PYROCLASM_STUN).map(|h| h.caster);
            (tries, stunner, mob, has(m, mob, PYROCLASM))
        })
        .await
        .unwrap();
    assert!(tries.is_some(), "no stun in 3,000 hits");
    assert_eq!(stunner, Some(mob), "the kobold's stun");
    assert!(kept, "the talent stays on the kobold");
}

const DISENCHANT: u32 = 13_262;
const ENCHANTING: u32 = 333;
/// A green dagger, its disenchant always giving something.
const JEWELED_DAGGER: u32 = 1917;
const WORN_DAGGER: u32 = 2092;
const INSTANT_POISON: u32 = 8679;
const INSTANT_POISON_VIAL: u32 = 6947;
const INSTANT_POISON_ENCHANT: u32 = 323;
const MAIN_HAND: u8 = 15;
const SMSG_LOOT_RESPONSE: u16 = 0x160;

fn on_item(item: Guid) -> spells::Targets {
    spells::Targets {
        item: Some(item),
        ..Default::default()
    }
}

fn enchanting(m: &Map, g: Guid) -> u32 {
    crate::game::map::skills::value(&m.players[&g], ENCHANTING).unwrap_or(0)
}

/// A packet the map answers at once, after whatever it sent before.
async fn fence(c: &mut crate::game::harness::Client) {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    c.send(MSG_LOOKING_FOR_GROUP_Client {}).await;
    c.wait_for("MSG_LOOKING_FOR_GROUP", |m| {
        matches!(m, ServerOpcodeMessage::MSG_LOOKING_FOR_GROUP(_)).then_some(())
    })
    .await;
}

/// A disenchant cancelled a second into its three takes nothing and gives
/// nothing, even once its time is up; cast to the end, it takes the dagger
/// and shows its loot once, the skill rising once. A coating of Instant
/// Poison cancelled spends no vial and puts nothing on the weapon; cast to
/// the end, it coats the weapon's temporary slot with its charges and
/// spends one vial (Thistle Tea's "queues disenchant exactly once after
/// completion and never after cancellation" and "coating costs travel
/// with the completed enchant and interruption consumes nothing";
/// mangos-classic's `Spell::cancel`, and `Spell::TakeCastItem` and
/// `Spell::TakeReagents` at the cast's launch).
#[tokio::test]
#[ignore = "extended"]
async fn an_interrupted_disenchant_or_coating_costs_nothing() {
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (mut c, g, map) = kit::player(&realm, "P9RD", "Coater", Race::Human, Class::Rogue).await;
    let disenchant = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, g, 60, &[DISENCHANT]);
            crate::game::map::skills::learn_skill(m.players.get_mut(&g).unwrap(), ENCHANTING, 1);
            crate::game::map::inventory::add_item(m, g, JEWELED_DAGGER, 1, false).unwrap();
            let item = m.players[&g]
                .items
                .iter()
                .find(|i| i.entry == JEWELED_DAGGER)
                .unwrap()
                .guid;
            let state = |m: &Map| {
                let p = &m.players[&g];
                (
                    p.count_of(JEWELED_DAGGER),
                    m.loot_state.item_loot.contains_key(&item),
                    p.looting == Some(item),
                    enchanting(m, g),
                )
            };
            let began = kit::cast_fresh(m, g, DISENCHANT, on_item(item));
            let casting = kit::casting(m, g);
            kit::pass(m, 1_000);
            spells::cancel(m, g);
            let stopped = kit::casting(m, g);
            kit::pass(m, 3_000);
            let cancelled = state(m);
            let again = kit::cast_fresh(m, g, DISENCHANT, on_item(item));
            kit::pass(m, 3_000);
            let done = state(m);
            kit::pass(m, 3_000);
            let later = (state(m), m.loot_state.item_loot.len());
            (began, casting, stopped, cancelled, again, done, later)
        })
        .await
        .unwrap();
    let (began, casting, stopped, cancelled, again, done, later) = disenchant;
    assert_eq!((began, casting), (true, Some(DISENCHANT)), "the cast begun");
    assert_eq!(stopped, None, "cancelled");
    assert_eq!(
        cancelled,
        (1, false, false, 1),
        "cancelled: the dagger kept, no loot, no rise"
    );
    assert!(again, "cast again");
    assert_eq!(
        done,
        (0, true, true, 2),
        "done: the dagger gone, its loot shown, one rise"
    );
    assert_eq!(later, (done, 1), "once");
    fence(&mut c).await;
    let windows = c
        .captured()
        .iter()
        .filter(|f| u16::from_le_bytes([f[2], f[3]]) == SMSG_LOOT_RESPONSE)
        .count();
    assert_eq!(windows, 1, "one loot window");

    let coating = map
        .query(move |m| {
            if let Some(p) = m.players.get_mut(&g) {
                p.looting = None;
            }
            crate::game::tests_p3_combat::equip(m, g, WORN_DAGGER, MAIN_HAND);
            crate::game::map::inventory::add_item(m, g, INSTANT_POISON_VIAL, 2, false).unwrap();
            let p = &m.players[&g];
            let weapon = p
                .items
                .iter()
                .find(|i| i.bag == crate::game::store::BAG_PLAYER && i.slot == MAIN_HAND)
                .unwrap()
                .guid;
            let vial = p
                .items
                .iter()
                .find(|i| i.entry == INSTANT_POISON_VIAL)
                .unwrap()
                .guid;
            let state = |m: &Map| {
                let p = &m.players[&g];
                let coat = p
                    .item_by_guid(weapon)
                    .map(|i| enchant::get(&p.items[i], TEMP_SLOT));
                (
                    p.count_of(INSTANT_POISON_VIAL),
                    coat.map_or(0, |e| e.id),
                    coat.map_or(0, |e| e.charges),
                )
            };
            kit::fresh(m, g);
            let began = spells::cast(m, g, INSTANT_POISON, on_item(weapon), Some(vial));
            let casting = kit::casting(m, g);
            kit::pass(m, 1_000);
            spells::cancel(m, g);
            let stopped = kit::casting(m, g);
            kit::pass(m, 3_000);
            let cancelled = state(m);
            kit::fresh(m, g);
            let again = spells::cast(m, g, INSTANT_POISON, on_item(weapon), Some(vial));
            kit::pass(m, 3_000);
            let done = state(m);
            (began, casting, stopped, cancelled, again, done)
        })
        .await
        .unwrap();
    let (began, casting, stopped, cancelled, again, done) = coating;
    assert_eq!(
        (began, casting),
        (true, Some(INSTANT_POISON)),
        "the coat begun"
    );
    assert_eq!(stopped, None, "cancelled");
    assert_eq!(cancelled, (2, 0, 0), "cancelled: both vials, no coat");
    assert!(again, "coated again");
    let charges = realm
        .data
        .crafts
        .enchant_charges
        .get(&INSTANT_POISON)
        .copied()
        .unwrap_or(0);
    assert_eq!(
        done,
        (1, INSTANT_POISON_ENCHANT, charges),
        "done: one vial spent, the coat on"
    );
}
