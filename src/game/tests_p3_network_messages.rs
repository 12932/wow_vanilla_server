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
    // Fire on a player with a ward of 30: the ward's 30 absorbed, the rest
    // hurts (resistance rolls by vmangos's buckets, so none here).
    let ward = realm.data.spells.get(FIRE_WARD).cloned().unwrap();
    on_map(&realm, move |m| {
        crate::game::map::auras::apply(m, g, g, &ward, 0, 30);
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 10_000);
        p.fields.set_u32(UNIT_FIELD_HEALTH, 10_000);
        crate::game::map::environment::damage(m, g, EnvironmentalDamageType::Fire, 200);
    })
    .await;
    let fire = a
        .wait_frame("SMSG_ENVIRONMENTAL_DAMAGE_LOG", 0x1FC, |_| true)
        .await;
    let mut want = g.guid().to_le_bytes().to_vec();
    want.push(EnvironmentalDamageType::Fire.as_int());
    for v in [170u32, 30, 0] {
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
    // Each with the client's next order counter (mangos's
    // `GetOrderCounter`): the unroot's past the root's.
    let rooted = a.wait_frame("SMSG_FORCE_MOVE_ROOT", 0x0E8, |_| true).await;
    let n = packed(g).len();
    assert_eq!(rooted.len(), n + 4);
    assert_eq!(&rooted[..n], &packed(g)[..]);
    let order = u32::from_le_bytes(rooted[n..].try_into().unwrap());
    let unroot = a
        .wait_frame("SMSG_FORCE_MOVE_UNROOT", 0x0EA, |_| true)
        .await;
    assert_eq!(unroot.len(), n + 4);
    assert_eq!(&unroot[..n], &packed(g)[..]);
    assert!(u32::from_le_bytes(unroot[n..].try_into().unwrap()) > order);
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

const RENEW: u32 = 900_321;
const BURST: u32 = 900_322;
const SPRING: u32 = 900_323;
const LEECH: u32 = 900_324;

fn periodic(aura: u32, points: i32, misc: i32, multiple: f32) -> SpellEffect {
    SpellEffect {
        effect: 6,
        aura,
        base_points: points - 1,
        base_dice: 1,
        misc_value: misc,
        amplitude: 1_000,
        multiple_value: multiple,
        target_a: 1,
        ..Default::default()
    }
}

/// Thistle Tea's smsg_periodicauralog_test: a tick is logged with its
/// aura's type and what it carries: a heal's amount (8); a periodic
/// energize (24) and a percentage of mana (21) their power and amount; a
/// mana leech (64) its power, what it took and the share the caster gets.
#[tokio::test]
async fn periodic_ticks_are_logged_as_1_12_has_them() {
    use crate::game::map::auras::*;
    let mut d = data();
    for (id, e) in [
        (RENEW, periodic(AURA_PERIODIC_HEAL, 25, 0, 0.0)),
        (BURST, periodic(AURA_PERIODIC_ENERGIZE, 50, 0, 0.0)),
        (SPRING, periodic(AURA_OBS_MOD_MANA, 5, 0, 0.0)),
        (LEECH, periodic(AURA_PERIODIC_MANA_LEECH, 50, 0, 0.5)),
    ] {
        d.spells.spells.insert(id, spell(id, &[e]));
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Mage).await;
    a.sees(gb).await;
    a.clear();
    let spells = realm.data.clone();
    on_map(&realm, move |m| {
        for g in [ga, gb] {
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000);
            p.fields.set_u32(UNIT_FIELD_HEALTH, 500);
            p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1_000);
            p.fields.set_u32(UNIT_FIELD_POWER1, 500);
        }
        for (id, target, amount) in [
            (RENEW, ga, 25),
            (BURST, ga, 50),
            (SPRING, ga, 5),
            (LEECH, gb, 50),
        ] {
            let s = spells.spells.get(id).cloned().unwrap();
            apply(m, ga, target, &s, 0, amount);
        }
        let now = m.now;
        for g in [ga, gb] {
            for h in m.players.get_mut(&g).unwrap().auras.holders.iter_mut() {
                for e in h.effects.iter_mut() {
                    e.next_tick = Some(now);
                }
            }
        }
        update(m);
    })
    .await;
    let log = |target: Guid, spell: u32, aura: u32, rest: &[u32]| {
        let mut v = [packed(target), packed(ga)].concat();
        for x in [spell, 1, aura].iter().chain(rest) {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    };
    // Both GUIDs pack to two bytes: the spell follows.
    let tick = |spell: u32| move |f: &[u8]| f.len() > 8 && f[4..8] == spell.to_le_bytes();
    let heal = a.wait_frame("the heal's tick", 0x24E, tick(RENEW)).await;
    assert_eq!(heal, log(ga, RENEW, AURA_PERIODIC_HEAL, &[25]));
    let energize = a
        .wait_frame("the energize's tick", 0x24E, tick(BURST))
        .await;
    assert_eq!(energize, log(ga, BURST, AURA_PERIODIC_ENERGIZE, &[0, 50]));
    let spring = a.wait_frame("the mana's tick", 0x24E, tick(SPRING)).await;
    let max = on_map(&realm, move |m| {
        m.players[&ga].fields.get_u32(UNIT_FIELD_MAXPOWER1)
    })
    .await;
    assert_eq!(
        spring,
        log(ga, SPRING, AURA_OBS_MOD_MANA, &[0, max * 5 / 100])
    );
    let leech = a.wait_frame("the leech's tick", 0x24E, tick(LEECH)).await;
    let mut want = log(gb, LEECH, AURA_PERIODIC_MANA_LEECH, &[0, 50]);
    want.extend_from_slice(&0.5f32.to_le_bytes());
    assert_eq!(leech, want);
}

const BLOOD_PRICE: u32 = 900_331;
const DEAR: u32 = 900_332;
const SWING: u32 = 900_333;
const STAB: u32 = 900_334;
const GOUGE: u32 = 900_335;
const DARKSHORE: u32 = 900_336;
const ORCS_ONLY: u32 = 900_337;
const ANVIL: u32 = 900_338;
const FEAST: u32 = 900_339;

/// The `SMSG_CAST_RESULT` for `spell` cast at `target` through the
/// protocol: its body after the spell.
async fn refused(c: &mut Client, spell: u32, target: Guid) -> Vec<u8> {
    c.send(CMSG_CAST_SPELL {
        spell,
        targets: wire_targets(&Targets {
            unit: Some(target),
            ..Default::default()
        }),
    })
    .await;
    let f = c
        .wait_frame("SMSG_CAST_RESULT", 0x130, move |f| {
            f[..4] == spell.to_le_bytes()
        })
        .await;
    f[4..].to_vec()
}

/// Thistle Tea's smsg_cast_result_test: a cost in health more than the
/// caster has is `CASTER_AURASTATE`, in mana `NO_POWER`; a melee spell
/// with its target behind is `UNIT_NOT_INFRONT`, a stab from in front
/// `NOT_BEHIND`, a gouge at a back `NOT_INFRONT`; a spell of another
/// place names the area it wants, or 0 for a rule with none; a spell
/// focus names the focus; and Cannibalize with no corpse to eat is
/// `NO_EDIBLE_CORPSES`.
#[tokio::test]
async fn cast_failures_say_why_as_1_12_has_them() {
    use crate::game::data::scripts::SpellArea;
    let mut d = data();
    let any = |id| {
        spell(
            id,
            &[SpellEffect {
                effect: 3,
                target_a: 25,
                ..Default::default()
            }],
        )
    };
    let mut blood = any(BLOOD_PRICE);
    blood.power_type = crate::game::map::spells::POWER_HEALTH;
    blood.mana_cost = 1_000_000;
    let mut dear = any(DEAR);
    dear.mana_cost = 1_000_000;
    let mut swing = any(SWING);
    swing.range_index = 2;
    let mut stab = any(STAB);
    stab.attributes[2] = crate::game::data::spells::ATTR_EX2_BEHIND_TARGET;
    stab.attributes[1] = crate::game::data::spells::ATTR_EX_FROM_BEHIND;
    let mut gouge = any(GOUGE);
    gouge.attributes[0] = 0x15_0010;
    let mut anvil = any(ANVIL);
    anvil.requires_spell_focus = 3;
    let mut feast = any(FEAST);
    feast.script_name = "spell_cannibalize".into();
    for s in [
        blood,
        dear,
        swing,
        stab,
        gouge,
        any(DARKSHORE),
        any(ORCS_ONLY),
        anvil,
        feast,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.scripts.spell_areas.push(SpellArea {
        spell: DARKSHORE,
        area: 148,
        gender: 2,
        ..Default::default()
    });
    d.scripts.spell_areas.push(SpellArea {
        spell: ORCS_ONLY,
        area: 0,
        race_mask: 0x2,
        gender: 2,
        ..Default::default()
    });
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let place = move |m: &mut Map, alice: f32, bob: f32| {
        let at = m.players[&ga].position();
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.orientation = alice;
        let p = m.players.get_mut(&gb).unwrap();
        p.movement.position = Vector3d {
            x: at.x + 3.0,
            y: at.y,
            z: at.z,
        };
        p.movement.orientation = bob;
    };
    on_map(&realm, move |m| {
        for s in [
            BLOOD_PRICE,
            DEAR,
            SWING,
            STAB,
            GOUGE,
            DARKSHORE,
            ORCS_ONLY,
            ANVIL,
            FEAST,
        ] {
            m.players.get_mut(&ga).unwrap().spells.push(s);
        }
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_FIELD_POWER1, 100);
        // Bob east of Alice; she faces west, away from him.
        place(m, std::f32::consts::PI, 0.0);
    })
    .await;
    a.clear();
    let failed = |code: u8| vec![2, code];
    assert_eq!(
        refused(&mut a, BLOOD_PRICE, ga).await,
        failed(0x12),
        "health"
    );
    assert_eq!(refused(&mut a, DEAR, ga).await, failed(0x4D), "mana");
    assert_eq!(refused(&mut a, SWING, gb).await, failed(0x7C), "behind her");
    // Face to face: a stab isn't from behind.
    on_map(&realm, move |m| place(m, 0.0, std::f32::consts::PI)).await;
    assert_eq!(
        refused(&mut a, STAB, gb).await,
        failed(0x33),
        "in front of him"
    );
    // Bob turned away: a gouge wants his face.
    on_map(&realm, move |m| place(m, 0.0, 0.0)).await;
    assert_eq!(refused(&mut a, GOUGE, gb).await, failed(0x36), "his back");
    let with = |code: u8, v: u32| [&[2u8, code][..], &v.to_le_bytes()].concat();
    assert_eq!(
        refused(&mut a, DARKSHORE, ga).await,
        with(0x5D, 148),
        "Darkshore's"
    );
    assert_eq!(
        refused(&mut a, ORCS_ONLY, ga).await,
        with(0x5D, 0),
        "no area named"
    );
    assert_eq!(refused(&mut a, ANVIL, ga).await, with(0x5E, 3), "the focus");
    assert_eq!(
        refused(&mut a, FEAST, ga).await,
        failed(0x86),
        "nothing to eat"
    );
}

const AIMED: u32 = 900_341;
const BOW: u32 = 50_210;
const ARROW: u32 = 50_211;

/// Thistle Tea's smsg_spell_cast_test: a spell of the ranged slot (Aimed
/// Shot's kind) sets the ammo flag in its start and its go, and ends each
/// with the arrow's look and inventory type, as mangos's
/// WriteAmmoToPacket writes them.
#[tokio::test]
async fn a_ranged_spell_shows_its_arrow_at_start_and_go() {
    let mut d = data();
    let mut aimed = spell(
        AIMED,
        &[SpellEffect {
            effect: 3,
            target_a: 25,
            ..Default::default()
        }],
    );
    aimed.attributes[0] = crate::game::data::spells::ATTR_RANGED;
    aimed.casting_time_index = 98;
    d.spells.cast_times.insert(98, 200);
    d.spells.spells.insert(AIMED, aimed);
    let item =
        |entry, class, subclass, inventory_type, display_id| crate::game::data::ItemTemplate {
            entry,
            name: format!("Item {entry}"),
            class,
            subclass,
            inventory_type,
            display_id,
            stackable: if class == 6 { 200 } else { 1 },
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        };
    let mut bow = item(BOW, 2, 2, 15, 8_000);
    bow.ammo_type = 2;
    bow.delay = 2_000;
    d.items.insert(BOW, bow);
    d.items.insert(ARROW, item(ARROW, 6, 2, 24, 5_996));
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        crate::game::map::inventory::add_item(m, ga, BOW, 1, false).unwrap();
        crate::game::map::inventory::add_item(m, ga, ARROW, 100, false).unwrap();
        let at = {
            let i = m.players[&ga]
                .items
                .iter()
                .find(|i| i.entry == BOW)
                .unwrap();
            (i.bag, i.slot)
        };
        crate::game::map::inventory::auto_equip(m, ga, at);
        crate::game::map::ranged::set_ammo(m, ga, ARROW);
        m.players.get_mut(&ga).unwrap().spells.push(AIMED);
    })
    .await;
    a.clear();
    a.send(CMSG_CAST_SPELL {
        spell: AIMED,
        targets: wire_targets(&Targets {
            unit: Some(gb),
            ..Default::default()
        }),
    })
    .await;
    let arrow = [5_996u32.to_le_bytes(), 24u32.to_le_bytes()].concat();
    let start = a
        .wait_frame("SMSG_SPELL_START", 0x131, |f| {
            f.windows(4).any(|w| w == AIMED.to_le_bytes())
        })
        .await;
    assert_eq!(&start[start.len() - 8..], &arrow[..], "the start's arrow");
    let go = a
        .wait_frame("SMSG_SPELL_GO", 0x132, |f| {
            f.windows(4).any(|w| w == AIMED.to_le_bytes())
        })
        .await;
    assert_eq!(&go[go.len() - 8..], &arrow[..], "the go's arrow");
    // The flags, after the item (the caster, as no item cast it) and the
    // caster, packed, and the spell: ammo's 0x20.
    let flags_at = 2 * packed(ga).len() + 4;
    assert_ne!(
        u16::from_le_bytes([start[flags_at], start[flags_at + 1]]) & 0x20,
        0
    );
}

const MIND_VISION: u32 = 900_351;
const DECEIVE: u32 = 900_352;

/// Thistle Tea's msg_channel_start_test and reputation_messages_test: a
/// channel starts with its spell and length and, cancelled, its client is
/// told 0 left; the reactions auras force are each faction and the rank
/// forced, after their count.
#[tokio::test]
async fn a_channel_and_forced_reactions_are_laid_out_as_1_12_has_them() {
    let mut d = data();
    let mut vision = spell(
        MIND_VISION,
        &[SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        }],
    );
    vision.attributes[1] = crate::game::data::spells::ATTR_EX_CHANNELED;
    vision.duration_index = 97;
    d.spells.durations.insert(
        97,
        crate::game::data::spells::SpellDuration {
            base: 8_000,
            per_level: 0,
            max: 8_000,
        },
    );
    d.spells.spells.insert(MIND_VISION, vision);
    for (id, faction, rank) in [(DECEIVE, 575, 3), (DECEIVE + 1, 529, 0)] {
        d.spells.spells.insert(
            id,
            spell(
                id,
                &[aura(
                    crate::game::map::auras::AURA_FORCE_REACTION,
                    rank,
                    faction,
                    0,
                )],
            ),
        );
    }
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Priest).await;
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().spells.push(MIND_VISION)
    })
    .await;
    a.clear();
    a.send(CMSG_CAST_SPELL {
        spell: MIND_VISION,
        targets: wire_targets(&Targets {
            unit: Some(g),
            ..Default::default()
        }),
    })
    .await;
    let start = a.wait_frame("MSG_CHANNEL_START", 0x139, |_| true).await;
    assert_eq!(
        start,
        [MIND_VISION.to_le_bytes(), 8_000u32.to_le_bytes()].concat()
    );
    a.send(CMSG_CANCEL_CHANNELLING { id: MIND_VISION }).await;
    let update = a.wait_frame("MSG_CHANNEL_UPDATE", 0x13A, |_| true).await;
    assert_eq!(update, 0u32.to_le_bytes().to_vec());
    let (first, second) = (
        realm.data.spells.get(DECEIVE).cloned().unwrap(),
        realm.data.spells.get(DECEIVE + 1).cloned().unwrap(),
    );
    on_map(&realm, move |m| {
        crate::game::map::auras::apply(m, g, g, &first, 0, 3);
        crate::game::map::auras::apply(m, g, g, &second, 0, 0);
    })
    .await;
    let reactions = a
        .wait_frame("SMSG_SET_FORCED_REACTIONS", 0x2A5, |f| f[0] == 2)
        .await;
    let mut want = 2u32.to_le_bytes().to_vec();
    for v in [529u32, 0, 575, 3] {
        want.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(reactions, want);
}

const KNOCK: u32 = 900_361;

/// Thistle Tea's knockback_test: the one knocked back is told its packed
/// GUID, a counter, the launch's cosine and sine, its speed out and its
/// speed up (negative); those who see them, at the client's word, its
/// movement and the same four.
#[tokio::test]
async fn a_knockback_is_laid_out_as_1_12_has_it() {
    let mut d = data();
    d.spells.spells.insert(
        KNOCK,
        spell(
            KNOCK,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_KNOCK_BACK,
                base_points: 69,
                base_dice: 1,
                misc_value: 120,
                target_a: 25,
                ..Default::default()
            }],
        ),
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    a.sees(gb).await;
    on_map(&realm, move |m| {
        let at = m.players[&ga].position();
        let p = m.players.get_mut(&gb).unwrap();
        p.movement.position = Vector3d {
            x: at.x + 3.0,
            y: at.y + 4.0,
            z: at.z,
        };
        crate::game::map::spells::cast_triggered(m, ga, KNOCK, Some(gb));
    })
    .await;
    let launch = b.wait_frame("SMSG_MOVE_KNOCK_BACK", 0x0EF, |_| true).await;
    // Bob's first order: the counter starts at 0.
    let mut want = packed(gb);
    want.extend_from_slice(&0u32.to_le_bytes());
    let tail: Vec<u8> = [0.6f32, 0.8, 12.0, -7.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    assert_eq!(&launch[..want.len()], &want[..]);
    let floats: Vec<f32> = launch[want.len()..]
        .chunks(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    for (got, want) in floats.iter().zip([0.6f32, 0.8, 12.0, -7.0]) {
        assert!((got - want).abs() < 1e-4, "{floats:?}");
    }
    a.clear();
    let at = on_map(&realm, move |m| m.players[&gb].position()).await;
    b.send(CMSG_MOVE_KNOCK_BACK_ACK {
        guid: gb,
        counter: 0,
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::new_jumping(MovementInfo_MovementFlags_Jumping {
                cos_angle: 0.6,
                sin_angle: 0.8,
                xy_speed: 12.0,
                z_speed: -7.0,
            }),
            timestamp: 0,
            position: at,
            orientation: 0.0,
            fall_time: 0,
        },
    })
    .await;
    let seen = a.wait_frame("MSG_MOVE_KNOCK_BACK", 0x0F1, |_| true).await;
    assert_eq!(&seen[..packed(gb).len()], &packed(gb)[..]);
    assert_eq!(
        &seen[seen.len() - 16..],
        &tail[..],
        "the jump's four after the movement"
    );
}

/// Thistle Tea's cmsg_force_move_root_ack_test: a root's acknowledgement
/// puts the player where the client says, and those who see them are
/// told with `MSG_MOVE_ROOT`, as mangos's HandleMoveRootAck tells them;
/// the same order acknowledged again is stale, and nothing; the unroot's
/// is told with `MSG_MOVE_UNROOT`.
#[tokio::test]
async fn root_acknowledgements_move_the_player_and_tell_who_sees_them() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Bob", Race::Human, Class::Warrior).await;
    b.sees(ga).await;
    let at = on_map(&realm, move |m| m.players[&ga].position()).await;
    let there = Vector3d {
        x: at.x + 1.0,
        ..at
    };
    let info = |flags| MovementInfo {
        flags,
        timestamp: 0,
        position: there,
        orientation: 0.0,
        fall_time: 0,
    };
    b.clear();
    // The server's orders, and their counters.
    let order = |root: bool| {
        on_map(&realm, move |m| {
            crate::game::map::movement::root_to(&m.players[&ga].outbound, ga, root)
        })
    };
    let counter = |m: &ServerOpcodeMessage| match m {
        ServerOpcodeMessage::SMSG_FORCE_MOVE_ROOT(r) => Some(r.counter),
        ServerOpcodeMessage::SMSG_FORCE_MOVE_UNROOT(r) => Some(r.counter),
        _ => None,
    };
    order(true).await;
    let rooted_at = a.wait_for("the root", counter).await;
    let root = || CMSG_FORCE_MOVE_ROOT_ACK {
        guid: ga,
        movement_counter: rooted_at,
        info: info(MovementInfo_MovementFlags::new_root()),
    };
    a.send(root()).await;
    let rooted = b
        .wait_for("MSG_MOVE_ROOT", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_ROOT(r) => Some(r.clone()),
            _ => None,
        })
        .await;
    assert_eq!(rooted.player, ga);
    assert_eq!(rooted.info.position, there);
    settled(&mut a).await;
    let now = on_map(&realm, move |m| m.players[&ga].position()).await;
    assert_eq!(now.x, there.x, "where the client says");
    // Again, settled already: stale.
    b.clear();
    a.send(root()).await;
    settled(&mut a).await;
    order(false).await;
    let unrooted_at = a.wait_for("the unroot", counter).await;
    a.send(CMSG_FORCE_MOVE_UNROOT_ACK {
        guid: ga,
        movement_counter: unrooted_at,
        info: info(MovementInfo_MovementFlags::empty()),
    })
    .await;
    b.wait_for("MSG_MOVE_UNROOT", |m| match m {
        ServerOpcodeMessage::MSG_MOVE_UNROOT(u) if u.player == ga => Some(()),
        _ => None,
    })
    .await;
    settled(&mut b).await;
    let roots = b
        .backlog
        .iter()
        .filter(|m| matches!(m, ServerOpcodeMessage::MSG_MOVE_ROOT(_)))
        .count();
    assert_eq!(roots, 0, "the stale root told no one");
}
