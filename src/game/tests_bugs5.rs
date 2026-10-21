//! Bugs found while mapping wvs's spells against Thistle Tea's, the fifth
//! batch: a helpful proc's damage that missed, the Warsong flag taken up by
//! the immune, the mounted and the stunned, activation-time modifiers that
//! changed no aura's ticks, a percentage of mana that made no threat, proc
//! cooldowns and charges spent on procs that came to nothing, and Nightfall
//! on a killing tick.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::battleground::{Flag, wsg};
use crate::game::map::{Map, MapHandle, auras, procs, spells};
use crate::game::tests_pvp;

const HOLY_SHIELD: u32 = 20925;
const OBSIDIAN_ARMOR: u32 = 27539;
/// Obsidian Armor's answer to fire.
const OBSIDIAN_FIRE: u32 = 27533;
const FIREBALL: u32 = 133;
const NIGHTFALL: u32 = 18094;
const SHADOW_TRANCE: u32 = 17941;
const CORRUPTION: u32 = 172;
const FIRE_NOVA_TOTEM: u32 = 1535;
const FIRE_NOVA_PULSE: u32 = 8443;
const IMPROVED_FIRE_NOVA_TOTEM: u32 = 16544;
const DRINK: u32 = 24355;
const BLOODRAGE: u32 = 29131;

/// A magic spell of the physical school, made up: Obsidian Armor has no
/// answer to it.
const PHYSICAL_BOLT: u32 = 995_001;
/// A percentage of mana with no amplitude, made up.
const SPRING: u32 = 995_002;
/// A periodic energize of mana, made up.
const WELLSPRING: u32 = 995_003;

/// A block taken from `attacker`, for `g`'s procs.
fn block(m: &mut Map, attacker: Guid, g: Guid) {
    procs::trigger(
        m,
        procs::Event {
            actor: attacker,
            target: g,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE,
            ex: procs::EX_BLOCK | procs::EX_NORMAL_HIT,
            damage: 10,
            school: 1,
            ..Default::default()
        },
    );
}

/// A harmful spell's hit from `attacker` on `g`, for their procs.
fn spell_hit(m: &mut Map, attacker: Guid, g: Guid, spell: u32) {
    let school = m
        .realm
        .data
        .spells
        .get(spell)
        .map_or(1, |s| s.school_mask());
    procs::trigger(
        m,
        procs::Event {
            actor: attacker,
            target: g,
            actor_flags: procs::PROC_DONE_NEGATIVE_SPELL,
            target_flags: procs::PROC_TAKEN_NEGATIVE_SPELL,
            ex: procs::EX_NORMAL_HIT,
            spell: Some(spell),
            damage: 10,
            school,
            ..Default::default()
        },
    );
}

/// Holy Shield's holy damage, from a helpful aura, can't miss, as Thistle
/// Tea's `ProcDamage.hit?` and mangos's `SpellHitResult` (with the aura's
/// own spell) have it: a low paladin's blocks against a creature fifty
/// levels above them strike back every time, where a spell of theirs would
/// land one time in five.
#[tokio::test]
#[ignore = "extended"]
async fn holy_shields_damage_never_misses() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, paladin, map) = player(&realm, "B5HS", "Shieldone", Race::Human, Class::Paladin).await;
    let hits = map
        .query(move |m| {
            ready(m, paladin, 10, &[]);
            let ogre = dummy(m, paladin, 60, 3.0);
            m.spells.rolls = Some(true);
            let mut hits = 0;
            for _ in 0..40 {
                // A fresh shield each time: its four charges.
                apply(m, paladin, paladin, HOLY_SHIELD);
                let before = health(m, ogre);
                block(m, ogre, paladin);
                if health(m, ogre) < before {
                    hits += 1;
                }
            }
            m.spells.rolls = None;
            hits
        })
        .await
        .unwrap();
    assert_eq!(hits, 40, "every block strikes back");
}

/// A proc that comes to nothing starts no cooldown and spends no charge, as
/// Thistle Tea's reactions and mangos's `SPELL_AURA_PROC_FAILED` have it:
/// Obsidian Armor has no answer to a physical spell, so its ten seconds
/// don't start, and the fire that follows is answered.
#[tokio::test]
#[ignore = "extended"]
async fn a_proc_that_comes_to_nothing_starts_no_cooldown() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&OBSIDIAN_ARMOR) {
            s.proc_chance = 100;
        }
        let mut bolt = made_up(PHYSICAL_BOLT, 2, 0, 0, 10);
        bolt.dmg_class = 1;
        bolt.effects[0].target_a = 6;
        d.spells.spells.insert(PHYSICAL_BOLT, bolt);
    });
    let (_c, warrior, map) = player(&realm, "B5OA", "Armorone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, warrior, 60, &[]);
            let kobold = dummy(m, warrior, 60, 5.0);
            apply(m, warrior, warrior, OBSIDIAN_ARMOR);
            let charged = |m: &mut Map| {
                if let Some(a) = auras::auras_mut(m, warrior) {
                    for h in a.holders.iter_mut().filter(|h| h.spell == OBSIDIAN_ARMOR) {
                        h.charges = 2;
                    }
                }
            };
            charged(m);
            spell_hit(m, kobold, warrior, PHYSICAL_BOLT);
            let after_physical =
                holder(m, warrior, OBSIDIAN_ARMOR).map(|h| (h.charges, h.proc_ready_at.is_some()));
            spell_hit(m, kobold, warrior, FIREBALL);
            let after_fire =
                holder(m, warrior, OBSIDIAN_ARMOR).map(|h| (h.charges, h.proc_ready_at.is_some()));
            (after_physical, after_fire, has(m, warrior, OBSIDIAN_FIRE))
        })
        .await
        .unwrap();
    let (after_physical, after_fire, answered) = out;
    assert_eq!(
        after_physical,
        Some((2, false)),
        "no answer, no cooldown and no charge"
    );
    assert!(answered, "the fire is answered");
    assert_eq!(after_fire, Some((1, true)), "and that spends them");
}

/// Nightfall gives no Shadow Trance for a Corruption tick that kills: a
/// tick's procs come after its damage (mangos's `PeriodicTick`), and the
/// class script wants a living victim (mangos's
/// `HandleOverrideClassScriptAuraProc`, Thistle Tea's `ClassScript`), so
/// nothing came of it: its charge stays.
#[tokio::test]
#[ignore = "extended"]
async fn nightfall_gives_no_shadow_trance_for_a_killing_tick() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        if let Some(s) = d.spells.spells.get_mut(&NIGHTFALL) {
            s.proc_chance = 100;
        }
    });
    let (_c, lock, map) = player(&realm, "B5NF", "Nightone", Race::Human, Class::Warlock).await;
    let out = map
        .query(move |m| {
            ready(m, lock, 60, &[CORRUPTION]);
            apply(m, lock, lock, NIGHTFALL);
            if let Some(a) = auras::auras_mut(m, lock) {
                for h in a.holders.iter_mut().filter(|h| h.spell == NIGHTFALL) {
                    h.charges = 3;
                }
            }
            let charges = |m: &Map| holder(m, lock, NIGHTFALL).map_or(0, |h| h.charges);
            // A tick that leaves its victim standing: a Shadow Trance.
            let kobold = dummy(m, lock, 60, 5.0);
            apply(m, lock, kobold, CORRUPTION);
            tick_now(m, kobold, CORRUPTION);
            let lived = (has(m, lock, SHADOW_TRANCE), charges(m));
            auras::remove_spell(m, lock, SHADOW_TRANCE, None);
            // A tick that kills.
            let doomed = dummy(m, lock, 60, 5.0);
            apply(m, lock, doomed, CORRUPTION);
            set_health(m, doomed, 1);
            tick_now(m, doomed, CORRUPTION);
            let killed = (
                health(m, doomed) == 0,
                has(m, lock, SHADOW_TRANCE),
                charges(m),
            );
            (lived, killed)
        })
        .await
        .unwrap();
    let (lived, killed) = out;
    assert_eq!(lived, (true, 2), "a tick procs Nightfall");
    assert_eq!(
        killed,
        (true, false, 2),
        "a killing tick doesn't, and spends nothing"
    );
}

/// An activation-time modifier (op 19) changes a periodic aura's interval,
/// as mangos's `Aura` takes `SPELLMOD_ACTIVATION_TIME` and Thistle Tea's
/// `Modifiers.periodic_interval` has it: Improved Fire Nova Totem's two
/// seconds off the totem's four-second pulse, its shaman's talent
/// inherited by the totem as it is put down. Without it the pulse is four
/// seconds.
#[tokio::test]
#[ignore = "extended"]
async fn an_activation_time_modifier_quickens_a_totems_pulse() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, shaman, map) = player(&realm, "B5FN", "Novaone", Race::Orc, Class::Shaman).await;
    let out = map
        .query(move |m| {
            ready(m, shaman, 60, &[]);
            let pulse = |m: &mut Map| {
                spells::cast_triggered(m, shaman, FIRE_NOVA_TOTEM, None);
                let totem = m.players[&shaman]
                    .owned
                    .totems
                    .iter()
                    .flatten()
                    .next()
                    .copied();
                totem
                    .and_then(|t| holder(m, t, FIRE_NOVA_PULSE))
                    .and_then(|h| h.effects.first().and_then(|x| x.period))
            };
            apply(m, shaman, shaman, IMPROVED_FIRE_NOVA_TOTEM);
            let talented = pulse(m);
            // The shaman's own pulse, the talent theirs.
            spells::cast_triggered(m, shaman, FIRE_NOVA_PULSE, Some(shaman));
            let own = holder(m, shaman, FIRE_NOVA_PULSE)
                .and_then(|h| h.effects.first().and_then(|x| x.period));
            auras::remove_spell(m, shaman, FIRE_NOVA_PULSE, None);
            auras::remove_spell(m, shaman, IMPROVED_FIRE_NOVA_TOTEM, None);
            let untalented = pulse(m);
            (talented, own, untalented)
        })
        .await
        .unwrap();
    let (talented, own, untalented) = out;
    assert_eq!(talented, Some(Duration::from_secs(2)), "the totem's pulse");
    assert_eq!(own, Some(Duration::from_secs(2)), "the shaman's own");
    assert_eq!(
        untalented,
        Some(Duration::from_secs(4)),
        "four seconds untalented"
    );
}

/// A percentage of mana makes threat for its caster on those fighting its
/// target, half of what it gave (mangos's `PeriodicTick` and its
/// `threatAssist`, Thistle Tea's `restore_percent_mana`): Drink's 2% a
/// second, then only what the room left gave, then none at full. One
/// without an amplitude ticks each second, as mangos's
/// `HandleAuraModTotalManaPercentRegen` and Thistle Tea's `period_ms` give
/// it.
#[tokio::test]
#[ignore = "extended"]
async fn a_percent_mana_tick_threatens_by_half_what_it_gave() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut s = made_up(SPRING, 6, auras::AURA_OBS_MOD_MANA, 0, 10);
        s.duration_index = 21;
        d.spells.spells.insert(SPRING, s);
    });
    let (mut c, mage, map) = player(&realm, "B5DR", "Drinkone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            crate::game::map::ai::attacked(m, kobold, mage);
            let threat = |m: &Map| {
                m.creature(kobold).map_or(0.0, |c| {
                    c.threat
                        .iter()
                        .find(|(g, _)| *g == mage)
                        .map_or(0.0, |(_, t)| *t)
                })
            };
            let max = |m: &Map| {
                m.unit_fields(mage)
                    .map_or(0, |f| f.get_u32(UNIT_FIELD_MAXPOWER1))
            };
            apply(m, mage, mage, DRINK);
            let mut out = Vec::new();
            for room in [None, Some(10), Some(0)] {
                let top = max(m);
                set_power(m, mage, 0, room.map_or(top / 2, |r| top - r));
                let (before, mana) = (threat(m), power(m, mage, 0));
                tick_now(m, mage, DRINK);
                out.push((power(m, mage, 0) - mana, threat(m) - before, top));
            }
            auras::remove_spell(m, mage, DRINK, None);
            apply(m, mage, mage, SPRING);
            let period = holder(m, mage, SPRING).and_then(|h| h.effects[0].period);
            (out, period)
        })
        .await
        .unwrap();
    let (ticks, period) = out;
    let (gave, threat, top) = ticks[0];
    assert_eq!(gave, top * 2 / 100, "2% of the mana");
    assert!((threat - gave as f32 / 2.0).abs() < 0.01, "{threat}");
    assert_eq!(ticks[1].0, 10, "only the room there was");
    assert!((ticks[1].1 - 5.0).abs() < 0.01, "{}", ticks[1].1);
    assert_eq!(ticks[2].0, 0, "full");
    assert!(ticks[2].1.abs() < 0.01, "no threat for nothing");
    // The tick is in the log with the whole of what it gave.
    let logged = c
        .wait_frame("the mana's tick", 0x24E, |f| {
            f.len() > 8 && f[4..8] == DRINK.to_le_bytes()
        })
        .await;
    assert_eq!(
        logged[logged.len() - 4..],
        (top * 2 / 100).to_le_bytes(),
        "the tick's amount"
    );
    assert_eq!(period, Some(Duration::from_secs(1)), "each second");
}

/// A periodic energize of rage makes threat as a heal does, half the rage
/// it gave (Bloodrage's on the warrior's foes), as Thistle Tea's
/// spell_threat_test and mangos's `threatAssist` have it; one of mana makes
/// none, as Thistle Tea has it.
#[tokio::test]
#[ignore = "extended"]
async fn an_energize_of_rage_threatens_and_one_of_mana_doesnt() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        let mut s = made_up(WELLSPRING, 6, auras::AURA_PERIODIC_ENERGIZE, 0, 50);
        s.effects[0].amplitude = 1000;
        s.duration_index = 21;
        d.spells.spells.insert(WELLSPRING, s);
    });
    let (_c, warrior, map) = player(&realm, "B5BR", "Rageone", Race::Human, Class::Warrior).await;
    let out = map
        .query(move |m| {
            ready(m, warrior, 60, &[]);
            // Out of the Battle Stance a warrior logs in in, and its threat.
            auras::remove_spell(m, warrior, crate::game::map::spells::BATTLE_STANCE, None);
            let kobold = dummy(m, warrior, 60, 5.0);
            crate::game::map::ai::attacked(m, kobold, warrior);
            let threat = |m: &Map| {
                m.creature(kobold).map_or(0.0, |c| {
                    c.threat
                        .iter()
                        .find(|(g, _)| *g == warrior)
                        .map_or(0.0, |(_, t)| *t)
                })
            };
            let mut out = Vec::new();
            for (spell, index) in [(BLOODRAGE, 1), (WELLSPRING, 0)] {
                set_power(m, warrior, index, 0);
                apply(m, warrior, warrior, spell);
                let before = threat(m);
                tick_now(m, warrior, spell);
                out.push((power(m, warrior, index), threat(m) - before));
                auras::remove_spell(m, warrior, spell, None);
            }
            out
        })
        .await
        .unwrap();
    let (rage, threat) = out[0];
    assert_eq!(rage, 10, "a rage a second");
    assert!(
        (threat - 5.0).abs() < 0.01,
        "half of it as threat: {threat}"
    );
    let (mana, threat) = out[1];
    assert_eq!(mana, 50, "the mana");
    assert!(threat.abs() < 0.01, "and no threat: {threat}");
}

const WSG: u32 = 489;
/// Made up: Divine Shield's immunity to every school, Blessing of
/// Protection's to the physical, a stealth, a mount and its steed, a stun,
/// a fear and the Spirit of Redemption.
const BUBBLE: u32 = 995_101;
const PROTECTION: u32 = 995_102;
const VEIL: u32 = 995_103;
const STEED: u32 = 995_104;
const STEED_NPC: u32 = 995_105;
const STUN: u32 = 995_106;
const FEAR: u32 = 995_107;
const SPIRIT_OF_REDEMPTION: u32 = 27827;

fn aura_spell(id: u32, effects: &[(u32, i32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Test {id}"),
        equipped_item_class: -1,
        duration_index: 21,
        ..Default::default()
    };
    for (i, (aura, misc)) in effects.iter().enumerate() {
        s.effects[i] = SpellEffect {
            effect: 6,
            aura: *aura,
            misc_value: *misc,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
    }
    s
}

/// Warsong Gulch with its flags' auras, and the auras a carrier might be
/// under.
fn gulch() -> GameData {
    let mut d = GameData::empty();
    tests_pvp::battleground(&mut d, WSG, 2, "PVPZone03", [769, 770]);
    tests_pvp::marker(&mut d, wsg::SPELL_FLAG_CARRIED[0]);
    tests_pvp::marker(&mut d, wsg::SPELL_FLAG_CARRIED[1]);
    use crate::game::map::auras::*;
    for (id, effects) in [
        (
            BUBBLE,
            &[(AURA_SCHOOL_IMMUNITY, 1), (AURA_SCHOOL_IMMUNITY, 126)][..],
        ),
        (PROTECTION, &[(AURA_SCHOOL_IMMUNITY, 1)]),
        (VEIL, &[(AURA_MOD_STEALTH, 0)]),
        (STEED, &[(AURA_MOUNTED, STEED_NPC as i32)]),
        (STUN, &[(AURA_MOD_STUN, 0)]),
        (FEAR, &[(AURA_MOD_FEAR, 0)]),
        (SPIRIT_OF_REDEMPTION, &[(AURA_DUMMY, 0)]),
    ] {
        d.spells.spells.insert(id, aura_spell(id, effects));
    }
    d.creature_templates.insert(
        STEED_NPC,
        crate::game::data::CreatureTemplate {
            entry: STEED_NPC,
            name: "Steed".into(),
            display_ids: [2404, 0, 0, 0],
            ..Default::default()
        },
    );
    d
}

/// Move `c` within the match's map, at once, to a height.
async fn put(map: &MapHandle, c: &mut Client, x: f32, y: f32, z: f32) {
    let g = c.guid;
    map.query(move |m| m.teleport(g, m.key, Vector3d { x, y, z }, 0.0))
        .await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
}

fn flags(m: &Map) -> [Option<Flag>; 2] {
    m.battleground.as_ref().map_or([None, None], |b| b.flags)
}

/// Until `f` holds on the map, or panic.
async fn until(map: &MapHandle, what: &str, f: impl Fn(&Map) -> bool + Send + Clone + 'static) {
    for _ in 0..300 {
        let f = f.clone();
        if map.query(move |m| f(m)).await.unwrap_or(false) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

fn on(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].aura != 0)
        .map(|i| (i, 0))
        .collect();
    let _ = auras::apply_spell(m, g, g, &s, &effects, auras::Context::default());
}

async fn in_the_gulch(realm: &Arc<Realm>) -> (Client, Client, MapHandle) {
    let mut a = Client::connect(realm, "B5W1");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(realm, "B5W2");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = tests_pvp::both_into(
        realm,
        &mut a,
        &mut b,
        wow_world_messages::vanilla::Map::WarsongGulch,
    )
    .await;
    (a, b, map)
}

/// The Warsong flag isn't taken up by one immune to every school (Divine
/// Shield: mangos's `CanUseBattleGroundObject`), stunned, feared or a
/// Spirit of Redemption, as Thistle Tea's `battleground_allowed?` has it
/// (one mounted is dismounted and takes it, as mangos's `GameObject::Use`:
/// question #36); one only partly immune is let take it, and a stealthed
/// one comes out of stealth to (mangos's pickup isn't a triggered cast, so
/// it ends stealth).
#[tokio::test]
async fn the_warsong_flag_wont_go_to_the_immune_the_mounted_or_the_stunned_and_ends_stealth() {
    let realm = harness::realm_with(gulch());
    let (_a, mut b, map) = in_the_gulch(&realm).await;
    let grom = b.guid;
    until(&map, "the flag up", |m| {
        matches!(flags(m)[0], Some(Flag::AtBase(_)))
    })
    .await;
    let stand = match map.query(|m| flags(m)[0]).await.flatten() {
        Some(Flag::AtBase(go)) => go,
        f => panic!("{f:?}"),
    };
    let (x, y, z, _) = wsg::FLAG_AT[0];
    put(&map, &mut b, x, y, z).await;
    let refused = map
        .query(move |m| {
            let mut taken = Vec::new();
            for spell in [BUBBLE, STUN, FEAR, SPIRIT_OF_REDEMPTION] {
                on(m, grom, spell);
                crate::game::map::world::use_object(m, grom, stand);
                taken.push((spell, flags(m)[0] != Some(Flag::AtBase(stand))));
                auras::remove_spell(m, grom, spell, None);
            }
            on(m, grom, PROTECTION);
            on(m, grom, VEIL);
            taken
        })
        .await
        .unwrap();
    for (spell, taken) in refused {
        assert!(!taken, "taken up under {spell}");
    }
    b.send(CMSG_GAMEOBJ_USE { guid: stand }).await;
    until(&map, "the flag carried", move |m| {
        flags(m)[0] == Some(Flag::Carried(grom))
    })
    .await;
    let (stealthed, protected) = map
        .query(move |m| (has(m, grom, VEIL), has(m, grom, PROTECTION)))
        .await
        .unwrap();
    assert!(!stealthed, "stealth ends as the flag is taken");
    assert!(protected, "a physical immunity stays");
}
