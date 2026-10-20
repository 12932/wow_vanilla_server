//! Round 8's auras: what an aura's spell attributes hold on its bearer.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{ATTR_EX_PREVENTS_ANIM, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, auras};

const NONE_WITHIN: Duration = Duration::from_millis(300);

/// Sap: a stun whose spell holds the body.
const SAP: u32 = 6770;
/// Dazed: a slow whose spell holds the body too.
const DAZED: u32 = 1604;
/// A stun whose spell doesn't (a Charge's).
const CHARGE_STUN: u32 = 7922;
const FEIGN_DEATH: u32 = 5384;

fn aura_spell(id: u32, aura: u32, amount: i32, attributes_ex: u32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        effects,
        duration_index: 0,
        equipped_item_class: -1,
        ..Default::default()
    };
    s.attributes[1] = attributes_ex;
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.text_emote_anims.insert(0x65, 3);
    for s in [
        aura_spell(SAP, auras::AURA_MOD_STUN, 0, ATTR_EX_PREVENTS_ANIM),
        aura_spell(DAZED, 33, -50, ATTR_EX_PREVENTS_ANIM),
        aura_spell(CHARGE_STUN, auras::AURA_MOD_STUN, 0, 0),
        aura_spell(
            FEIGN_DEATH,
            auras::AURA_FEIGN_DEATH,
            0,
            ATTR_EX_PREVENTS_ANIM,
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn players_realm(d: GameData) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, d, None)
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

fn put_on(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    auras::apply(m, g, g, &s, 0, s.effects[0].base_points + 1);
}

fn held(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .unwrap()
        .has_flag(UNIT_FIELD_FLAGS, auras::UNIT_FLAG_PREVENT_ANIM)
}

/// An emote, a text emote and a posture: what was played or heard of them.
async fn try_to_move(a: &mut Client) -> Option<&'static str> {
    a.clear();
    a.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Wave,
        emote: 0,
        target: Guid::zero(),
    })
    .await;
    a.send(CMSG_EMOTE {
        emote: Emote::OneshotWave,
    })
    .await;
    a.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Sit,
    })
    .await;
    a.gets(NONE_WITHIN, |m| match m {
        ServerOpcodeMessage::SMSG_TEXT_EMOTE(_) => Some("a text emote"),
        ServerOpcodeMessage::SMSG_EMOTE(_) => Some("an emote"),
        ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(_) => Some("a posture"),
        _ => None,
    })
    .await
}

/// A spell with `SPELL_ATTR_EX_PREVENTS_ANIM` (Sap, a stun; Dazed, a slow;
/// Feign Death) holds its bearer's body while its aura lasts, as mangos's
/// `_AddSpellAuraHolder` sets `UNIT_FLAG_PREVENT_ANIM`: an emote, a text
/// emote and a posture are refused (`HandleEmoteOpcode`,
/// `HandleTextEmoteOpcode`, `HandleStandStateChangeOpcode`). The flag goes
/// with the last such aura (`_RemoveSpellAuraHolder`), not with the first:
/// a feign that ends under a Sap leaves the body held. A stun whose spell
/// lacks the attribute holds nothing.
#[tokio::test]
async fn a_spell_that_prevents_animation_holds_emotes_and_postures_until_its_last_aura_goes() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    // A stun without the attribute: free to wave.
    let by_charge = on_map(&realm, move |m| {
        put_on(m, g, CHARGE_STUN);
        (
            auras::auras(m, g).unwrap().has_spell(CHARGE_STUN),
            held(m, g),
        )
    })
    .await;
    assert_eq!(by_charge, (true, false), "a Charge's stun holds no body");
    assert!(try_to_move(&mut a).await.is_some(), "and a wave plays");
    on_map(&realm, move |m| {
        auras::remove_spell(m, g, CHARGE_STUN, None)
    })
    .await;
    // Sapped and feigning: held.
    let sapped = on_map(&realm, move |m| {
        put_on(m, g, SAP);
        let sapped = held(m, g);
        put_on(m, g, FEIGN_DEATH);
        (sapped, held(m, g))
    })
    .await;
    assert_eq!(sapped, (true, true), "Sap and Feign Death hold the body");
    assert_eq!(try_to_move(&mut a).await, None, "nothing plays");
    // The feign ends: the Sap still holds it.
    let after_feign = on_map(&realm, move |m| {
        auras::remove_spell(m, g, FEIGN_DEATH, None);
        held(m, g)
    })
    .await;
    assert!(after_feign, "the Sap still holds it");
    assert_eq!(try_to_move(&mut a).await, None, "still nothing plays");
    // The Sap goes, and a Dazed comes: still held.
    let dazed = on_map(&realm, move |m| {
        put_on(m, g, DAZED);
        auras::remove_spell(m, g, SAP, None);
        held(m, g)
    })
    .await;
    assert!(dazed, "Dazed holds it too");
    assert_eq!(try_to_move(&mut a).await, None, "nothing plays dazed");
    // The last goes: let go.
    let free = on_map(&realm, move |m| {
        auras::remove_spell(m, g, DAZED, None);
        held(m, g)
    })
    .await;
    assert!(!free, "let go with the last");
    a.clear();
    a.send(CMSG_EMOTE {
        emote: Emote::OneshotWave,
    })
    .await;
    a.wait_for("the wave", |m| match m {
        ServerOpcodeMessage::SMSG_EMOTE(e) => (e.emote == Emote::OneshotWave).then_some(()),
        _ => None,
    })
    .await;
}

/// The class scripts' proc auras, each with the spell that sets it off and
/// the outcome its rule wants: Nightfall by Corruption's tick, Improved
/// Blizzard by Blizzard's, Improved Mend Pet by Mend Pet's, Corrupted
/// Healing by a Lesser Heal, and the druid set's Rejuvenation and Regrowth
/// gifts by theirs.
const SCRIPTED: [(u32, u32, u32); 6] = [
    (18094, 172, crate::game::map::procs::EX_NORMAL_HIT),
    (11185, 10, crate::game::map::procs::EX_NORMAL_HIT),
    (
        19572,
        136,
        crate::game::map::procs::EX_NORMAL_HIT | crate::game::map::procs::EX_PERIODIC_POSITIVE,
    ),
    (23401, 2050, crate::game::map::procs::EX_NORMAL_HIT),
    (
        28716,
        774,
        crate::game::map::procs::EX_NORMAL_HIT | crate::game::map::procs::EX_PERIODIC_POSITIVE,
    ),
    (28744, 8936, crate::game::map::procs::EX_NORMAL_HIT),
];

/// Every class script wants a living victim on the map (Thistle Tea's
/// `ClassScript.events`, mangos's `HandleOverrideClassScriptAuraProc`):
/// with its victim dead or gone, a script's proc comes to nothing, so it
/// spends no charge and starts no cooldown (mangos's
/// `SPELL_AURA_PROC_FAILED`, Thistle Tea's reactions); on a living one,
/// the same proc answers and spends both.
#[tokio::test]
#[ignore = "extended"]
async fn a_class_scripts_proc_on_a_dead_or_missing_victim_comes_to_nothing() {
    use super::tests_p3_spells_kit::{dummy, have_data, holder, player, ready, realm_plus};
    use crate::game::map::procs;
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        for (aura, _, _) in SCRIPTED {
            let pe = d.spells.proc_events.entry(aura).or_default();
            pe.chance = 100.0;
            pe.cooldown = 1000;
        }
    });
    let (_c, g, map) = player(&realm, "P8AS", "Scripter", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            let mut seen = Vec::new();
            for (aura, by, ex) in SCRIPTED {
                for victim in ["dead", "gone", "alive"] {
                    auras::remove_spell(m, g, aura, None);
                    let s = m.realm.data.spells.get(aura).unwrap().clone();
                    auras::apply(m, g, g, &s, 0, 100);
                    if let Some(a) = auras::auras_mut(m, g) {
                        for h in a.holders.iter_mut().filter(|h| h.spell == aura) {
                            h.charges = 2;
                            h.proc_ready_at = None;
                            for x in &mut h.effects {
                                x.amount = 100;
                            }
                        }
                    }
                    let other = dummy(m, g, 60, 5.0);
                    match victim {
                        "dead" => crate::game::map::combat::kill(m, g, other),
                        "gone" => drop(m.remove_creature(other)),
                        _ => {}
                    }
                    let flags = m
                        .realm
                        .data
                        .spells
                        .proc_events
                        .get(&aura)
                        .map_or(0, |p| p.flags);
                    let school = m.realm.data.spells.get(by).unwrap().school_mask();
                    procs::trigger(
                        m,
                        procs::Event {
                            actor: g,
                            target: other,
                            actor_flags: flags,
                            ex,
                            spell: Some(by),
                            damage: 10,
                            school,
                            ..Default::default()
                        },
                    );
                    let h = holder(m, g, aura).unwrap();
                    seen.push((aura, victim, h.charges, h.proc_ready_at.is_some()));
                }
            }
            seen
        })
        .await
        .unwrap();
    for (aura, victim, charges, cooling) in out {
        if victim == "alive" {
            assert_eq!(
                (charges, cooling),
                (1, true),
                "{aura} answers a living victim"
            );
        } else {
            assert_eq!(
                (charges, cooling),
                (2, false),
                "{aura} comes to nothing on a {victim} victim"
            );
        }
    }
}

const WILL_DRAIN_FESTIVE_MUG: u32 = 25864;
const FILLED_FESTIVE_MUG: u32 = 21171;
const EMPTY_FESTIVE_MUG: u32 = 21174;
/// Will Drain Festive Mug, made up with a reagent of its own.
const WILL_DRAIN_PAID: u32 = 995_864;

/// A periodic trigger's spell pays its own reagents only when the aura's
/// spell has none, as mangos's `IgnoreItemRequirements` and Thistle Tea's
/// `pays_reagents?` have it: Will Drain Festive Mug's drain takes the
/// Filled Festive Mug it empties, and without one nothing is drained. Under
/// an aura spell with a reagent of its own, the drain is free.
#[tokio::test]
#[ignore = "extended"]
async fn a_periodic_triggers_spell_pays_its_reagents_when_its_aura_spell_has_none() {
    use super::tests_p3_spells_kit::{have_data, player, ready, realm_plus, tick_now};
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut paid = d.spells.get(WILL_DRAIN_FESTIVE_MUG).unwrap().clone();
        paid.id = WILL_DRAIN_PAID;
        paid.reagents[0] = (20406, 1);
        d.spells.spells.insert(WILL_DRAIN_PAID, paid);
    });
    let (_c, g, map) = player(&realm, "P8AR", "Drinker", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[]);
            // A GM pays no reagents.
            m.players.get_mut(&g).unwrap().gm = false;
            let mugs = |m: &Map| {
                let p = &m.players[&g];
                (
                    p.carried_count_of(FILLED_FESTIVE_MUG),
                    p.carried_count_of(EMPTY_FESTIVE_MUG),
                )
            };
            let mut seen = Vec::new();
            for aura in [WILL_DRAIN_FESTIVE_MUG, WILL_DRAIN_PAID] {
                crate::game::map::inventory::add_item(m, g, FILLED_FESTIVE_MUG, 1, false)
                    .expect("room");
                let s = m.realm.data.spells.get(aura).unwrap().clone();
                auras::apply(m, g, g, &s, 0, 0);
                tick_now(m, g, aura);
                seen.push(mugs(m));
                // Drained dry, a tick more: nothing to drain.
                if aura == WILL_DRAIN_FESTIVE_MUG {
                    tick_now(m, g, aura);
                    seen.push(mugs(m));
                }
                auras::remove_spell(m, g, aura, None);
            }
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        out[0],
        (0, 1),
        "the drain takes the filled mug for an empty one"
    );
    assert_eq!(out[1], (0, 1), "with no filled mug, nothing is drained");
    assert_eq!(
        out[2],
        (1, 2),
        "under an aura spell with a reagent, the drain is free"
    );
}

/// A channel of a periodic trigger every 4 s for 10 s, a mage's; what it
/// triggers; and a one-charge modifier taking 2 s off its period.
const CHANNEL: u32 = 995_100;
const CHANNEL_TRIGGER: u32 = 995_101;
const QUICKER: u32 = 995_050;

/// A channel's trigger keeps the schedule its cast's modifiers gave it
/// after their charge is spent, as Thistle Tea's `Casting.start` and
/// mangos's `Aura` (its period taken with `SPELLMOD_ACTIVATION_TIME` as the
/// holder is made) have it: the one-charge modifier goes with the cast,
/// and the channel's trigger goes every 2 s, not 4.
#[tokio::test]
#[ignore = "extended"]
async fn a_channels_trigger_keeps_its_modified_schedule_after_the_charge_is_spent() {
    use super::tests_p3_spells_kit::{
        at, cast_fresh, has, have_data, holder, made_up, pass, player, ready, realm_plus,
    };
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut channel = made_up(CHANNEL, 6, auras::AURA_PERIODIC_TRIGGER_SPELL, 0, 0);
        channel.attributes[1] = 0x4;
        channel.family_name = 3;
        channel.family_flags = 1;
        channel.duration_index = 1;
        channel.casting_time_index = 1;
        channel.effects[0].amplitude = 4000;
        channel.effects[0].trigger_spell = CHANNEL_TRIGGER;
        d.spells.spells.insert(CHANNEL, channel);
        let mut trigger = made_up(CHANNEL_TRIGGER, 6, auras::AURA_DUMMY, 0, 1);
        trigger.duration_index = 1;
        d.spells.spells.insert(CHANNEL_TRIGGER, trigger);
        let mut quicker = made_up(QUICKER, 6, auras::AURA_ADD_FLAT_MODIFIER, 19, -2000);
        quicker.family_name = 3;
        quicker.effects[0].item_type = 1;
        quicker.duration_index = 5;
        d.spells.spells.insert(QUICKER, quicker);
    });
    let (_c, g, map) = player(&realm, "P8AC", "Channeler", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, g, 60, &[CHANNEL]);
            let s = m.realm.data.spells.get(QUICKER).unwrap().clone();
            auras::apply(m, g, g, &s, 0, -2000);
            if let Some(a) = auras::auras_mut(m, g) {
                for h in a.holders.iter_mut().filter(|h| h.spell == QUICKER) {
                    h.charges = 1;
                }
            }
            let began = cast_fresh(m, g, CHANNEL, at(g));
            let spent = !has(m, g, QUICKER);
            let period = holder(m, g, CHANNEL).and_then(|h| h.effects[0].period);
            let mut ticks = Vec::new();
            for _ in 0..4 {
                pass(m, 1000);
                ticks.push(has(m, g, CHANNEL_TRIGGER));
                auras::remove_spell(m, g, CHANNEL_TRIGGER, None);
            }
            (began, spent, period, ticks)
        })
        .await
        .unwrap();
    let (began, spent, period, ticks) = out;
    assert!(began, "the channel begins");
    assert!(spent, "the modifier's charge goes with the cast");
    assert_eq!(period, Some(Duration::from_secs(2)), "its period is 2 s");
    assert_eq!(
        ticks,
        [false, true, false, true],
        "and its trigger goes every 2 s"
    );
}

const OVERPOWER: u32 = 7384;
const BATTLE_STANCE: u32 = 2457;
const TEN_MORE: u32 = 995_200;
const TWICE_AS_MUCH: u32 = 995_201;

/// A flat physical damage done goes in the hand's fields, under its
/// percentages, and a normalized ability takes both, as mangos's
/// `CalculateMinMaxDamage` (`TOTAL_VALUE` under `TOTAL_PCT`, normalized or
/// not): with ten more damage done and twice as much, Overpower is its
/// weapon at 2.4 and its attack power, and the ten, all doubled, and its 5.
#[tokio::test]
#[ignore = "extended"]
async fn a_normalized_ability_takes_the_hands_flat_bonus_and_percentages() {
    use super::tests_p3_spells_kit::{
        apply, at, cast_fresh, dummy, have_data, health, made_up, player, ready, realm_plus,
        set_health, set_power,
    };
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut ten = made_up(TEN_MORE, 6, auras::AURA_MOD_DAMAGE_DONE, 1, 10);
        ten.duration_index = 5;
        d.spells.spells.insert(TEN_MORE, ten);
        let mut twice = made_up(TWICE_AS_MUCH, 6, 79, 1, 100);
        twice.duration_index = 5;
        d.spells.spells.insert(TWICE_AS_MUCH, twice);
    });
    let (_c, w, map) = player(&realm, "P8AN", "Overpowera", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, w, 60, &[OVERPOWER]);
            let k = dummy(m, w, 60, 2.0);
            let fields = |m: &Map| m.unit_fields(w).unwrap().get_f32(UNIT_FIELD_MINDAMAGE);
            let bare = fields(m);
            apply(m, w, w, TEN_MORE);
            apply(m, w, w, TWICE_AS_MUCH);
            let sheet = fields(m);
            let expected = {
                let p = m.players.get(&w).unwrap();
                let data = m.realm.data.clone();
                let wpn = crate::game::map::derive::gear(p, &data).main.unwrap();
                ((wpn.min + wpn.max) / 2.0 + p.derived.attack_power as f32 / 14.0 * 2.4 + 10.0)
                    * 2.0
                    + 5.0
            };
            apply(m, w, w, BATTLE_STANCE);
            crate::game::map::combat::after_outcome(
                m,
                w,
                k,
                crate::game::map::combat::Outcome::Dodge,
            );
            set_health(m, k, 100_000);
            set_power(m, w, 1, 1_000);
            cast_fresh(m, w, OVERPOWER, at(k));
            (bare, sheet, 100_000 - health(m, k), expected)
        })
        .await
        .unwrap();
    let (bare, sheet, overpower, expected) = out;
    assert_eq!(
        (sheet - bare * 2.0).round(),
        20.0,
        "the sheet's damage: the weapon's and the ten, doubled"
    );
    assert_eq!(
        overpower, expected as u32,
        "Overpower, normalized, with both"
    );
}
