//! Phase 5's procs that ask for gear, Thistle Tea's `ProcEquipmentTest`
//! ported: a warrior with a sword, a dagger in the off hand and a bow, and
//! auras that proc only with a weapon or shield of their kinds in the hand
//! that did it.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{self as sd, ProcEvent, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::{self, *};
use crate::game::map::procs::{self, *};
use crate::game::tests_p3_aura2::{add_spells, aura_spell, put};
use crate::game::tests_p3_combat::{BRUTE, equip, setup_with, spawn};

const SWORD: u32 = 96_950;
const DAGGER: u32 = 96_951;
const BOW: u32 = 96_952;
const SHIELD: u32 = 96_953;
/// Procs of a sword, a bow, no kind, a dagger, any weapon, a shield, a
/// buckler only, a sword's that asks for nothing, and one of no gear.
const SWORD_PROC: u32 = 96_960;
const BOW_PROC: u32 = 96_961;
const NO_KIND_PROC: u32 = 96_962;
const DAGGER_PROC: u32 = 96_963;
const ANY_PROC: u32 = 96_964;
const SHIELD_PROC: u32 = 96_965;
const BUCKLER_PROC: u32 = 96_966;
const EXEMPT_PROC: u32 = 96_967;
const FREE_PROC: u32 = 96_968;
const CAST_END_PROC: u32 = 96_969;
const MARK: u32 = 96_970;
/// A disarm; a shot, an auto-repeat shot, an off-hand blow that repeats,
/// a main-hand blow, a frost bolt.
const DISARM: u32 = 96_971;
const SHOT: u32 = 96_972;
const AUTO_SHOT: u32 = 96_973;
const OFF_STRIKE: u32 = 96_974;
const MAIN_STRIKE: u32 = 96_975;
const BOLT: u32 = 96_976;

fn data() -> GameData {
    let mut d = crate::game::tests_p3_aura2::data();
    for (entry, class, subclass, inventory_type) in [
        (SWORD, 2, 7, 13),
        (DAGGER, 2, 15, 13),
        (BOW, 2, 2, 15),
        (SHIELD, 4, 6, 14),
    ] {
        d.items.insert(
            entry,
            ItemTemplate {
                entry,
                class,
                subclass,
                name: format!("Gear {entry}"),
                display_id: 1000,
                inventory_type,
                delay: 2000,
                max_durability: 50,
                stackable: 1,
                ..Default::default()
            },
        );
    }
    let proc = |id: u32, class: i32, mask: i32| {
        let mut s = crate::game::tests_p3_aura2::proc_spell(id, 0x3_0156, MARK);
        s.equipped_item_class = class;
        s.equipped_item_subclass_mask = mask;
        s.proc_charges = 2;
        s
    };
    let mut exempt = proc(EXEMPT_PROC, 2, 0x80);
    exempt.attributes[3] = sd::ATTR_EX3_NO_PROC_EQUIP_REQUIREMENT;
    let spell = |id: u32, dmg_class: u32| {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            dmg_class,
            equipped_item_class: -1,
            range_index: 1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: crate::game::map::spells::EFFECT_SCHOOL_DAMAGE,
            base_points: 9,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        };
        s
    };
    let mut auto = spell(AUTO_SHOT, 0);
    auto.attributes[0] = sd::ATTR_RANGED;
    auto.attributes[2] = sd::ATTR_EX2_AUTO_REPEAT;
    let mut off = spell(OFF_STRIKE, 2);
    off.attributes[0] = sd::ATTR_RANGED;
    off.attributes[2] = sd::ATTR_EX2_AUTO_REPEAT;
    off.attributes[3] = sd::ATTR_EX3_REQ_OFFHAND;
    add_spells(
        &mut d,
        [
            proc(SWORD_PROC, 2, 0x80),
            proc(BOW_PROC, 2, 0x4),
            proc(NO_KIND_PROC, 2, 0),
            proc(DAGGER_PROC, 2, 0x8000),
            proc(ANY_PROC, 2, 0xF_FFFF),
            proc(SHIELD_PROC, 4, 64),
            proc(BUCKLER_PROC, 4, 32),
            exempt,
            proc(FREE_PROC, -1, 0),
            proc(CAST_END_PROC, 2, 0x80),
            aura_spell(MARK, Some(10_000), &[(AURA_DUMMY, 0, 0)]),
            aura_spell(DISARM, Some(10_000), &[(AURA_MOD_DISARM, 0, 0)]),
            spell(SHOT, 3),
            auto,
            off,
            spell(MAIN_STRIKE, 2),
            spell(BOLT, 1),
        ],
    );
    for id in [SWORD_PROC, BOW_PROC, DAGGER_PROC, ANY_PROC, SHIELD_PROC] {
        d.spells.proc_events.insert(
            id,
            ProcEvent {
                cooldown: 5_000,
                ..Default::default()
            },
        );
    }
    d.spells.proc_events.insert(
        CAST_END_PROC,
        ProcEvent {
            ex: EX_CAST_END,
            ..Default::default()
        },
    );
    d
}

/// What `g` does to `other`: a blow (of the off hand), or a spell, of `ex`.
fn done(g: Guid, other: Guid, flags: u32, spell: Option<u32>, offhand: bool, ex: u32) -> Event {
    Event {
        actor: g,
        target: other,
        actor_flags: flags,
        ex,
        spell,
        damage: 30,
        school: 1,
        offhand,
        ranged: flags & (PROC_DONE_RANGED | PROC_DONE_RANGED_SPELL) != 0,
        ..Default::default()
    }
}

fn melee(g: Guid, other: Guid) -> Event {
    done(g, other, PROC_DONE_MELEE, None, false, EX_NORMAL_HIT)
}

/// Whether `proc` on `g` goes off for `e`: its mark, taken off again, and
/// the proc put back whole.
fn procs_for(m: &mut Map, g: Guid, proc: u32, e: Event) -> bool {
    auras::remove_spell(m, g, proc, None);
    let _ = put(m, g, g, proc, &[0]);
    procs::trigger(m, e);
    let marked = auras::auras(m, g).is_some_and(|a| a.has_spell(MARK));
    auras::remove_spell(m, g, MARK, None);
    auras::remove_spell(m, g, proc, None);
    marked
}

fn broken(m: &mut Map, g: Guid, slot: u8, broken: bool) {
    if let Some(p) = m.players.get_mut(&g)
        && let Some(i) = p
            .items
            .iter_mut()
            .find(|i| i.bag == crate::game::store::BAG_PLAYER && i.slot == slot)
    {
        i.fields.set_u32(ITEM_FIELD_MAXDURABILITY, 50);
        i.fields
            .set_u32(ITEM_FIELD_DURABILITY, if broken { 0 } else { 50 });
    }
}

fn unequip(m: &mut Map, g: Guid, slot: u8) {
    if let Some(p) = m.players.get_mut(&g) {
        p.items
            .retain(|i| !(i.bag == crate::game::store::BAG_PLAYER && i.slot == slot));
    }
}

/// Thistle Tea's hands: a sword's proc goes off on the main hand's blow,
/// not the off hand's dagger, nor a shot; a bow's on a shot and an
/// auto-repeat one; one of no kind never. An off-hand blow goes by the off
/// hand whatever else it is; a main-hand blow by the main. An empty or
/// broken hand procs nothing; disarmed, the main hand doesn't, the others
/// do; in a feral form no hand does. A shield's proc wants a whole shield
/// of its kind. A spell that asks for nothing and a creature's aura need no
/// gear; a harmful spell's proc wants the weapon too, as a cast's end's and
/// a kill's do, while what is done to one asks for none.
#[tokio::test]
async fn a_gear_proc_wants_the_hands_whole_weapon_of_its_kind() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let got = map
        .query(move |m| {
            equip(m, ga, SWORD, 15);
            equip(m, ga, DAGGER, 16);
            equip(m, ga, BOW, 17);
            let brute = spawn(m, ga, BRUTE, 1, 3.0);
            let shot = done(
                ga,
                brute,
                PROC_DONE_RANGED_SPELL,
                Some(SHOT),
                false,
                EX_NORMAL_HIT,
            );
            let auto = done(
                ga,
                brute,
                PROC_DONE_RANGED,
                Some(AUTO_SHOT),
                false,
                EX_NORMAL_HIT,
            );
            let off_strike = done(
                ga,
                brute,
                PROC_DONE_MELEE_SPELL,
                Some(OFF_STRIKE),
                false,
                EX_NORMAL_HIT,
            );
            let main_strike = done(
                ga,
                brute,
                PROC_DONE_MELEE_SPELL,
                Some(MAIN_STRIKE),
                false,
                EX_NORMAL_HIT,
            );
            let off_swing = done(ga, brute, PROC_DONE_MELEE, None, true, EX_NORMAL_HIT);
            let hands = vec![
                (
                    "sword, main",
                    procs_for(m, ga, SWORD_PROC, melee(ga, brute)),
                    true,
                ),
                ("sword, off", procs_for(m, ga, SWORD_PROC, off_swing), false),
                ("sword, shot", procs_for(m, ga, SWORD_PROC, shot), false),
                ("bow, shot", procs_for(m, ga, BOW_PROC, shot), true),
                ("bow, auto shot", procs_for(m, ga, BOW_PROC, auto), true),
                (
                    "no kind",
                    procs_for(m, ga, NO_KIND_PROC, melee(ga, brute)),
                    false,
                ),
                (
                    "sword, off strike",
                    procs_for(m, ga, SWORD_PROC, off_strike),
                    false,
                ),
                (
                    "dagger, off strike",
                    procs_for(m, ga, DAGGER_PROC, off_strike),
                    true,
                ),
                (
                    "dagger, main strike",
                    procs_for(m, ga, DAGGER_PROC, main_strike),
                    false,
                ),
                (
                    "dagger, off swing",
                    procs_for(m, ga, DAGGER_PROC, off_swing),
                    true,
                ),
            ];
            // Broken, and empty.
            let mut gone = Vec::new();
            for (slot, e) in [(15, melee(ga, brute)), (16, off_swing), (17, shot)] {
                broken(m, ga, slot, true);
                gone.push(procs_for(m, ga, ANY_PROC, e));
                broken(m, ga, slot, false);
            }
            // Disarmed; feral.
            let _ = put(m, ga, ga, DISARM, &[0]);
            let disarmed = (
                procs_for(m, ga, ANY_PROC, melee(ga, brute)),
                procs_for(m, ga, ANY_PROC, off_swing),
                procs_for(m, ga, ANY_PROC, shot),
            );
            auras::remove_spell(m, ga, DISARM, None);
            let mut feral = Vec::new();
            for form in [FORM_CAT, FORM_BEAR, FORM_DIREBEAR] {
                if let Some(f) = m.unit_fields_mut(ga) {
                    f.set_byte(UNIT_FIELD_BYTES_1, 2, form);
                }
                for e in [melee(ga, brute), off_swing, shot] {
                    feral.push(procs_for(m, ga, ANY_PROC, e));
                }
            }
            if let Some(f) = m.unit_fields_mut(ga) {
                f.set_byte(UNIT_FIELD_BYTES_1, 2, 0);
            }
            // A shield.
            let mut shield = vec![procs_for(m, ga, SHIELD_PROC, melee(ga, brute))];
            equip(m, ga, SHIELD, 16);
            shield.push(procs_for(m, ga, SHIELD_PROC, melee(ga, brute)));
            shield.push(procs_for(m, ga, BUCKLER_PROC, melee(ga, brute)));
            broken(m, ga, 16, true);
            shield.push(procs_for(m, ga, SHIELD_PROC, melee(ga, brute)));
            // Nothing asked: exempt, no gear, a creature's.
            unequip(m, ga, 15);
            let empty = procs_for(m, ga, SWORD_PROC, melee(ga, brute));
            let exempt = (
                procs_for(m, ga, EXEMPT_PROC, melee(ga, brute)),
                procs_for(m, ga, FREE_PROC, melee(ga, brute)),
                procs_for(m, brute, SWORD_PROC, melee(brute, ga)),
            );
            // A harmful spell's, a cast's end's and a kill's want the weapon;
            // one taken doesn't.
            let bolt = done(
                ga,
                brute,
                PROC_DONE_NEGATIVE_SPELL,
                Some(BOLT),
                false,
                EX_CRITICAL_HIT,
            );
            let cast_end = done(
                ga,
                brute,
                PROC_DONE_NEGATIVE_SPELL,
                Some(BOLT),
                false,
                EX_CAST_END,
            );
            let kill = done(ga, brute, PROC_KILL, None, false, EX_NORMAL_HIT);
            let taken = Event {
                actor: brute,
                target: ga,
                target_flags: PROC_TAKEN_NEGATIVE_SPELL,
                ex: EX_NORMAL_HIT,
                spell: Some(BOLT),
                damage: 30,
                school: 1,
                ..Default::default()
            };
            let unarmed = (
                procs_for(m, ga, SWORD_PROC, bolt),
                procs_for(m, ga, CAST_END_PROC, cast_end),
                procs_for(m, ga, SWORD_PROC, kill),
                procs_for(m, ga, SWORD_PROC, taken),
            );
            // Armed: the bolt's goes off, a charge spent and its cooldown on.
            equip(m, ga, SWORD, 15);
            let _ = put(m, ga, ga, SWORD_PROC, &[0]);
            procs::trigger(m, bolt);
            let armed = auras::auras(m, ga).and_then(|a| {
                a.holders.iter().find(|h| h.spell == SWORD_PROC).map(|h| {
                    (
                        h.charges,
                        h.proc_ready_at.map(|t| t.saturating_duration_since(m.now)),
                    )
                })
            });
            let marked = auras::auras(m, ga).is_some_and(|a| a.has_spell(MARK));
            (
                hands,
                gone,
                disarmed,
                feral,
                shield,
                empty,
                exempt,
                unarmed,
                (marked, armed),
            )
        })
        .await
        .unwrap();
    let (hands, gone, disarmed, feral, shield, empty, exempt, unarmed, armed) = got;
    for (what, got, want) in hands {
        assert_eq!(got, want, "{what}");
    }
    assert_eq!(gone, vec![false; 3], "broken hands");
    assert_eq!(disarmed, (false, true, true), "disarmed: the main hand");
    assert_eq!(feral, vec![false; 9], "feral: no hand");
    assert_eq!(
        shield,
        vec![false, true, false, false],
        "a whole shield of its kind"
    );
    assert!(!empty, "an empty hand");
    assert_eq!(exempt, (true, true, true));
    assert_eq!(unarmed, (false, false, false, true));
    assert_eq!(
        armed,
        (true, Some((1, Some(std::time::Duration::from_secs(5)))))
    );
}
