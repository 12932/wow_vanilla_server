//! Thistle Tea's server message tests (Phase 3, `game/network/message`),
//! through the protocol: what wvs sends when the behaviour behind each
//! message happens, as the client receives it, byte for byte against
//! Thistle Tea's expectations and mangos-zero's layouts.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::spells::{Targets, wire_targets};
use crate::game::map::{Map, MapKey};

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

/// Wait until the map has handled everything sent before.
async fn settled(c: &mut Client) {
    c.send(CMSG_QUERY_TIME {}).await;
    c.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
}

fn packed(g: Guid) -> Vec<u8> {
    let mut b = Vec::new();
    crate::game::update::write_packed_guid(&mut b, g);
    b
}

fn spell(id: u32, effects: &[SpellEffect]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    for (i, e) in effects.iter().enumerate() {
        s.effects[i] = *e;
    }
    s
}

fn aura(aura: u32, points: i32, misc: i32, mask: u64) -> SpellEffect {
    SpellEffect {
        effect: 6,
        aura,
        base_points: points - 1,
        base_dice: 1,
        misc_value: misc,
        item_type: mask,
        target_a: 1,
        ..Default::default()
    }
}

const ENERGIZE: u32 = 900_301;
const ROOT: u32 = 900_302;
const FIRE_WARD: u32 = 900_303;
const SHARPEN: u32 = 900_304;
const FOCUS: u32 = 900_305;
const STEALTHY: u32 = 900_306;

fn data() -> GameData {
    let mut d = GameData::empty();
    for s in [
        spell(
            ENERGIZE,
            &[SpellEffect {
                effect: 30,
                base_points: 99,
                base_dice: 1,
                misc_value: 0,
                target_a: 1,
                ..Default::default()
            }],
        ),
        spell(
            ROOT,
            &[aura(crate::game::map::auras::AURA_MOD_ROOT, 0, 0, 0)],
        ),
        // Absorbs 30 fire.
        spell(
            FIRE_WARD,
            &[aura(crate::game::map::auras::AURA_SCHOOL_ABSORB, 30, 4, 0)],
        ),
        // -500 to op 10 of bit 5, flat; -100 of bit 30, percent.
        spell(
            SHARPEN,
            &[aura(
                crate::game::map::auras::AURA_ADD_FLAT_MODIFIER,
                -500,
                10,
                1 << 5,
            )],
        ),
        spell(
            FOCUS,
            &[aura(
                crate::game::map::auras::AURA_ADD_PCT_MODIFIER,
                -100,
                10,
                1 << 30,
            )],
        ),
        {
            let mut s = spell(STEALTHY, &[]);
            s.recovery_time = 10_000;
            s
        },
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// Thistle Tea's smsg_spellenergizelog_test, smsg_dispel_failed_test,
/// smsg_cooldown_event_test, smsg_environmental_damage_log_test,
/// smsg_force_move_root_test, smsg_cancel_auto_repeat_test and
/// smsg_set_spell_modifier_test: an energize's log is the target and
/// caster packed, then spell, power and amount; a failed dispel the two
/// GUIDs in full and each spell, no count; a cooldown event the spell and
/// the GUID; fire's damage its type, then what hurt, what was absorbed
/// and what resisted; a root and an unroot the packed GUID and its
/// counter (wow_messages writes the GUID whole); Auto
/// Shot's cancel nothing; a spell modifier its bit, op and signed value.
#[tokio::test]
async fn spell_logs_are_laid_out_as_1_12_has_them() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Mage).await;
    a.clear();
    // An energize, cast through the protocol.
    on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.spells.push(ENERGIZE);
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1_000);
        p.fields.set_u32(UNIT_FIELD_POWER1, 0);
    })
    .await;
    a.send(CMSG_CAST_SPELL {
        spell: ENERGIZE,
        targets: wire_targets(&Targets {
            unit: Some(g),
            ..Default::default()
        }),
    })
    .await;
    let log = a.wait_frame("SMSG_SPELLENERGIZELOG", 0x151, |_| true).await;
    let mut want = [packed(g), packed(g)].concat();
    want.extend_from_slice(&ENERGIZE.to_le_bytes());
    want.extend_from_slice(&0u32.to_le_bytes());
    want.extend_from_slice(&100u32.to_le_bytes());
    assert_eq!(log, want, "SMSG_SPELLENERGIZELOG");
    // A dispel that failed, on a creature-like GUID.
    let victim = Guid::new(0xF130_0000_0000_0001);
    on_map(&realm, move |m| {
        crate::game::map::spells::log::dispel_failed(m, g, victim, vec![123, 123, 456]);
    })
    .await;
    let failed = a.wait_frame("SMSG_DISPEL_FAILED", 0x262, |_| true).await;
    let mut want = g.guid().to_le_bytes().to_vec();
    want.extend_from_slice(&victim.guid().to_le_bytes());
    for s in [123u32, 123, 456] {
        want.extend_from_slice(&s.to_le_bytes());
    }
    assert_eq!(failed, want, "SMSG_DISPEL_FAILED");
    // A cooldown held for an event, let go.
    on_map(&realm, move |m| {
        if let Some(c) = m.caster_mut(g) {
            c.pending_cooldowns.push(STEALTHY);
        }
        crate::game::map::spells::cooldowns::release(m, g, STEALTHY, true);
    })
    .await;
    let event = a.wait_frame("SMSG_COOLDOWN_EVENT", 0x135, |_| true).await;
    let mut want = STEALTHY.to_le_bytes().to_vec();
    want.extend_from_slice(&g.guid().to_le_bytes());
    assert_eq!(event, want, "SMSG_COOLDOWN_EVENT");
    // Fire on a player with 300 fire resistance and a ward of 30: half
    // resisted, the ward's 30 absorbed, the rest hurts.
    let ward = realm.data.spells.get(FIRE_WARD).cloned().unwrap();
    on_map(&realm, move |m| {
        crate::game::map::auras::apply(m, g, g, &ward, 0, 30);
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
        p.fields.set_u32(UNIT_FIELD_HEALTH, 10_000);
        p.fields.set_u32(UNIT_FIELD_RESISTANCES + 2, 300);
        crate::game::map::environment::damage(m, g, EnvironmentalDamageType::Fire, 200);
    })
    .await;
    let fire = a
        .wait_frame("SMSG_ENVIRONMENTAL_DAMAGE_LOG", 0x1FC, |_| true)
        .await;
    let mut want = g.guid().to_le_bytes().to_vec();
    want.push(EnvironmentalDamageType::Fire.as_int());
    for v in [70u32, 30, 100] {
        want.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(fire, want, "SMSG_ENVIRONMENTAL_DAMAGE_LOG");
    // Rooted, then let go.
    let root = realm.data.spells.get(ROOT).cloned().unwrap();
    on_map(&realm, move |m| {
        crate::game::map::auras::apply(m, g, g, &root, 0, 0);
        crate::game::map::auras::remove_spell(m, g, ROOT, None);
    })
    .await;
    let rooted = a.wait_frame("SMSG_FORCE_MOVE_ROOT", 0x0E8, |_| true).await;
    assert_eq!(rooted, [packed(g), 0u32.to_le_bytes().to_vec()].concat());
    let unroot = a
        .wait_frame("SMSG_FORCE_MOVE_UNROOT", 0x0EA, |_| true)
        .await;
    assert_eq!(unroot, [packed(g), 0u32.to_le_bytes().to_vec()].concat());
    // Auto Shot cancelled.
    on_map(&realm, move |m| {
        let now = m.now;
        m.players.get_mut(&g).unwrap().combat.auto_repeat =
            Some(crate::game::map::combat::AutoRepeat {
                spell: 75,
                target: g,
                next_at: now + Duration::from_secs(60),
            });
        crate::game::map::ranged::cancel(m, g, true);
    })
    .await;
    let cancelled = a
        .wait_frame("SMSG_CANCEL_AUTO_REPEAT", 0x29C, |_| true)
        .await;
    assert!(cancelled.is_empty(), "SMSG_CANCEL_AUTO_REPEAT is empty");
    // Spell modifiers, flat and percent, signed.
    let (sharpen, focus) = (
        realm.data.spells.get(SHARPEN).cloned().unwrap(),
        realm.data.spells.get(FOCUS).cloned().unwrap(),
    );
    on_map(&realm, move |m| {
        crate::game::map::auras::apply(m, g, g, &sharpen, 0, -500);
        crate::game::map::auras::apply(m, g, g, &focus, 0, -100);
    })
    .await;
    let flat = a
        .wait_frame("SMSG_SET_FLAT_SPELL_MODIFIER", 0x266, |f| f[0] == 5)
        .await;
    assert_eq!(flat, [&[5u8, 10][..], &(-500i32).to_le_bytes()].concat());
    let pct = a
        .wait_frame("SMSG_SET_PCT_SPELL_MODIFIER", 0x267, |f| f[0] == 30)
        .await;
    assert_eq!(pct, [&[30u8, 10][..], &(-100i32).to_le_bytes()].concat());
    settled(&mut a).await;
}

const WEAR: u32 = 900_311;
const WEAR_ALL: u32 = 900_312;
const VEST: u32 = 50_200;

/// The rows of a `SMSG_SPELLLOGEXECUTE` with one effect logged: the
/// caster packed, the spell, one effect, then the effect, one target and
/// what follows.
fn execute_log(caster: Guid, spell: u32, effect: u32, rest: &[u8]) -> Vec<u8> {
    let mut v = packed(caster);
    v.extend_from_slice(&spell.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&effect.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(rest);
    v
}

/// Thistle Tea's smsg_spelllogexecute_test: an interrupt logs the victim's
/// GUID in full and the spell cut short; extra attacks the GUID and their
/// count; a durability loss the GUID, the item's entry and -1, or -1 and
/// -1 for everything worn.
#[tokio::test]
async fn spell_log_executes_are_laid_out_as_1_12_has_them() {
    let mut d = data();
    let wear = |id, slot| {
        spell(
            id,
            &[SpellEffect {
                effect: 111,
                base_points: 9,
                base_dice: 1,
                misc_value: slot,
                target_a: 1,
                ..Default::default()
            }],
        )
    };
    d.spells.spells.insert(WEAR, wear(WEAR, 4));
    d.spells.spells.insert(WEAR_ALL, wear(WEAR_ALL, -1));
    d.items.insert(
        VEST,
        crate::game::data::ItemTemplate {
            entry: VEST,
            name: "Vest".into(),
            class: 4,
            subclass: 1,
            inventory_type: 5,
            max_durability: 50,
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let victim = Guid::new(0xF130_0000_0000_0304);
    on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, g, VEST, 1, false).unwrap();
        let at = {
            let i = m.players[&g]
                .items
                .iter()
                .find(|i| i.entry == VEST)
                .unwrap();
            (i.bag, i.slot)
        };
        crate::game::map::inventory::auto_equip(m, g, at);
    })
    .await;
    a.clear();
    on_map(&realm, move |m| {
        crate::game::map::spells::log::interrupt(m, g, 2139, victim, 133);
        crate::game::map::spells::log::extra_attacks(m, g, 3391, g, 2);
        crate::game::map::spells::cast_triggered(m, g, WEAR, Some(g));
        crate::game::map::spells::cast_triggered(m, g, WEAR_ALL, Some(g));
    })
    .await;
    let mut logs = Vec::new();
    for spell in [2139u32, 3391, WEAR, WEAR_ALL] {
        logs.push(
            a.wait_frame("SMSG_SPELLLOGEXECUTE", 0x24C, move |f| {
                f.len() > 6 && f[2..].windows(4).take(5).any(|w| w == spell.to_le_bytes())
            })
            .await,
        );
    }
    let full = |g: Guid| g.guid().to_le_bytes().to_vec();
    assert_eq!(
        logs[0],
        execute_log(
            g,
            2139,
            68,
            &[full(victim), 133u32.to_le_bytes().to_vec()].concat()
        ),
        "an interrupt"
    );
    assert_eq!(
        logs[1],
        execute_log(
            g,
            3391,
            19,
            &[full(g), 2u32.to_le_bytes().to_vec()].concat()
        ),
        "extra attacks"
    );
    assert_eq!(
        logs[2],
        execute_log(
            g,
            WEAR,
            111,
            &[
                full(g),
                VEST.to_le_bytes().to_vec(),
                u32::MAX.to_le_bytes().to_vec()
            ]
            .concat()
        ),
        "a worn item's loss"
    );
    assert_eq!(
        logs[3],
        execute_log(
            g,
            WEAR_ALL,
            111,
            &[
                full(g),
                u32::MAX.to_le_bytes().to_vec(),
                u32::MAX.to_le_bytes().to_vec()
            ]
            .concat()
        ),
        "everything worn"
    );
}
