//! Phase 5's objective carriers, Thistle Tea's `ObjectiveCarrierTest`
//! ported on a Warsong Gulch of its own: what takes a carried flag off
//! (a helpful protection from every school, a mount, death, cancelling)
//! and what doesn't (a refresh, a harmful protection, a charmed carrier's,
//! one already there); and who may take a flag up at all, stealth and
//! invisibility ending as they do.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_pvp::{battleground, both_into, marker};
use crate::game::data::spells::SpellTemplate;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::auras::{self, *};
use crate::game::map::battleground::{Flag, wsg};
use crate::game::tests_p3_aura2::{add_spells, aura_spell};

const WSG: u32 = 489;
/// A mount's creature, with a look.
const STEED: u32 = 1234;
/// Protections from every school (a helpful one, a harmful one), from the
/// physical and the rest apart; a mount; the auras that hold a carrier.
const SHIELD: u32 = 96_800;
const CURSE_SHIELD: u32 = 96_801;
const PHYSICAL_WARD: u32 = 96_802;
const MAGIC_WARD: u32 = 96_803;
pub(crate) const MOUNT: u32 = 96_804;
const HOLDS: u32 = 96_810;
const ROOT: u32 = 96_820;
const STEALTH: u32 = 96_821;
const INVISIBILITY: u32 = 96_822;
const DUMMY: u32 = 96_823;
/// The kinds that keep a flag out of reach.
const HOLDING: [u32; 5] = [
    AURA_MOD_STUN,
    AURA_MOD_CONFUSE,
    AURA_MOD_FEAR,
    AURA_FEIGN_DEATH,
    AURA_MOD_POSSESS,
];

fn gulch() -> GameData {
    let mut d = GameData::empty();
    battleground(&mut d, WSG, 2, "PVPZone03", [769, 770]);
    marker(&mut d, wsg::SPELL_FLAG_CARRIED[1]);
    // The flag's aura ends as Warsong Flag's (23333) does: on mounting,
    // changing map, losing sight and invulnerability.
    if let Some(s) = d.spells.spells.get_mut(&wsg::SPELL_FLAG_CARRIED[1]) {
        s.aura_interrupt_flags = 0x3A_0000;
    }
    d.creature_templates
        .insert(STEED, crate::game::tests_p3_combat::template(STEED, 1));
    let mut curse = aura_spell(
        CURSE_SHIELD,
        Some(10_000),
        &[(AURA_SCHOOL_IMMUNITY, 127, 0)],
    );
    curse.attributes[0] = crate::game::data::spells::ATTR_DEBUFF;
    let mut all: Vec<SpellTemplate> = vec![
        aura_spell(SHIELD, Some(10_000), &[(AURA_SCHOOL_IMMUNITY, 127, 0)]),
        curse,
        aura_spell(PHYSICAL_WARD, Some(10_000), &[(AURA_SCHOOL_IMMUNITY, 1, 0)]),
        aura_spell(MAGIC_WARD, Some(10_000), &[(AURA_SCHOOL_IMMUNITY, 126, 0)]),
        aura_spell(MOUNT, None, &[(AURA_MOUNTED, STEED as i32, 0)]),
        aura_spell(ROOT, Some(10_000), &[(AURA_MOD_ROOT, 0, 0)]),
        aura_spell(STEALTH, None, &[(AURA_MOD_STEALTH, 0, 0)]),
        aura_spell(INVISIBILITY, None, &[(AURA_MOD_INVISIBILITY, 1, 0)]),
        aura_spell(DUMMY, Some(10_000), &[(AURA_DUMMY, 0, 0)]),
    ];
    for (i, kind) in HOLDING.iter().enumerate() {
        all.push(aura_spell(HOLDS + i as u32, Some(10_000), &[(*kind, 0, 0)]));
    }
    add_spells(&mut d, all);
    d
}

/// Alice (Alliance) and Grom (Horde) in the gulch, Alice by the Horde's
/// flag at its stand.
pub(crate) async fn by_the_flag() -> (Client, Client, MapHandle) {
    let realm = harness::realm_with(gulch());
    let mut a = Client::connect(&realm, "A");
    a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Grom", Race::Orc, Class::Warrior).await;
    let map = both_into(&realm, &mut a, &mut b, Map::WarsongGulch).await;
    for _ in 0..100 {
        if map
            .query(|m| matches!(flag(m), Some(Flag::AtBase(_))))
            .await
            == Some(true)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let g = a.guid;
    let (x, y, z, _) = wsg::FLAG_AT[1];
    map.query(move |m| m.teleport(g, m.key, Vector3d { x, y, z }, 0.0))
        .await;
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    (a, b, map)
}

fn flag(m: &crate::game::map::Map) -> Option<Flag> {
    m.battleground.as_ref().and_then(|b| b.flags[1])
}

pub(crate) fn carried(m: &crate::game::map::Map, g: Guid) -> bool {
    flag(m) == Some(Flag::Carried(g))
        && auras::auras(m, g).is_some_and(|a| a.has_spell(wsg::SPELL_FLAG_CARRIED[1]))
}

/// `g` uses the Horde's flag, at its stand or on the ground: whether they
/// carry it after.
pub(crate) fn take(m: &mut crate::game::map::Map, g: Guid) -> bool {
    let go = match flag(m) {
        Some(Flag::AtBase(go) | Flag::Dropped(go)) => go,
        _ => return carried(m, g),
    };
    wsg::use_object(m, g, go);
    carried(m, g)
}

/// `id` on `target` from `caster`.
pub(crate) fn on(m: &mut crate::game::map::Map, caster: Guid, target: Guid, id: u32) {
    let s = m.realm.data.spells.get(id).cloned().expect("the spell");
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].aura != 0)
        .map(|i| (i, s.effects[i].misc_value))
        .collect();
    let _ = auras::apply_spell(m, caster, target, &s, &effects, Context::default());
}

/// Thistle Tea's carrier: a refresh of the flag's aura keeps it; a harmful
/// protection from every school, an unrelated aura on one already partly
/// protected, a charmed carrier's own protection keep it. A helpful
/// protection from every school, a mount, cancelling it and death each
/// drop it.
#[tokio::test]
async fn a_carrier_keeps_the_flag_unless_protected_mounted_or_dead() {
    let (a, b, map) = by_the_flag().await;
    let (alice, grom) = (a.guid, b.guid);
    let got = map
        .query(move |m| {
            let flag_aura = wsg::SPELL_FLAG_CARRIED[1];
            let mut kept = Vec::new();
            // Partly protected before, then carrying.
            on(m, alice, alice, PHYSICAL_WARD);
            kept.push(("taken while partly protected", take(m, alice)));
            on(m, alice, alice, DUMMY);
            kept.push(("an unrelated aura", carried(m, alice)));
            crate::game::map::spells::cast_triggered(m, alice, flag_aura, Some(alice));
            kept.push(("refreshed", carried(m, alice)));
            on(m, grom, alice, CURSE_SHIELD);
            kept.push(("a harmful protection", carried(m, alice)));
            auras::remove_spell(m, alice, CURSE_SHIELD, None);
            if let Some(f) = m.unit_fields_mut(alice) {
                f.set_guid(UNIT_FIELD_CHARMEDBY, grom);
            }
            on(m, alice, alice, SHIELD);
            kept.push(("charmed, her own protection", carried(m, alice)));
            auras::remove_spell(m, alice, SHIELD, None);
            if let Some(f) = m.unit_fields_mut(alice) {
                f.set_guid(UNIT_FIELD_CHARMEDBY, Guid::zero());
            }
            let mut dropped = Vec::new();
            for (what, id) in [("a helpful protection", SHIELD), ("a mount", MOUNT)] {
                let had = take(m, alice);
                on(m, alice, alice, id);
                wsg::update(m);
                dropped.push((
                    what,
                    had,
                    carried(m, alice),
                    matches!(flag(m), Some(Flag::Dropped(_))),
                ));
                auras::remove_spell(m, alice, id, None);
            }
            let had = take(m, alice);
            auras::remove_spell(m, alice, flag_aura, None);
            wsg::update(m);
            dropped.push((
                "cancelled",
                had,
                carried(m, alice),
                matches!(flag(m), Some(Flag::Dropped(_))),
            ));
            let had = take(m, alice);
            crate::game::map::combat::deal_damage(m, grom, alice, 10_000_000);
            wsg::update(m);
            dropped.push((
                "dead",
                had,
                carried(m, alice),
                matches!(flag(m), Some(Flag::Dropped(_))),
            ));
            (kept, dropped)
        })
        .await
        .unwrap();
    let (kept, dropped) = got;
    for (what, carried) in kept {
        assert!(carried, "{what}: still carried");
    }
    for (what, had, carried, on_the_ground) in dropped {
        assert!(had && !carried && on_the_ground, "{what}: dropped");
    }
}

/// Thistle Tea's pickup: one protected from every school (by two auras
/// together), stunned, confused, feared, feigning death, possessed or dead
/// can't take a flag up; one rooted or only partly protected can. Taking
/// it up ends stealth and invisibility. (One mounted is dismounted and
/// takes it, as mangos's `GameObject::Use`: question #36.)
#[tokio::test]
async fn only_one_free_and_not_wholly_protected_may_take_a_flag_up() {
    let (a, b, map) = by_the_flag().await;
    let (alice, grom) = (a.guid, b.guid);
    let got = map
        .query(move |m| {
            let mut refused = Vec::new();
            on(m, alice, alice, PHYSICAL_WARD);
            on(m, alice, alice, MAGIC_WARD);
            refused.push(("protected from every school", take(m, alice)));
            auras::remove_spell(m, alice, PHYSICAL_WARD, None);
            auras::remove_spell(m, alice, MAGIC_WARD, None);
            for (i, kind) in HOLDING.iter().enumerate() {
                let id = HOLDS + i as u32;
                on(m, alice, alice, id);
                refused.push((
                    if *kind == AURA_MOD_STUN {
                        "stunned"
                    } else {
                        "held"
                    },
                    take(m, alice),
                ));
                auras::remove_spell(m, alice, id, None);
            }
            let mut allowed = Vec::new();
            for id in [ROOT, PHYSICAL_WARD] {
                on(m, alice, alice, id);
                allowed.push((id, take(m, alice)));
                auras::remove_spell(m, alice, id, None);
                auras::remove_spell(m, alice, wsg::SPELL_FLAG_CARRIED[1], None);
                wsg::update(m);
            }
            on(m, alice, alice, STEALTH);
            on(m, alice, alice, INVISIBILITY);
            let hidden = auras::auras(m, alice)
                .is_some_and(|a| a.has_spell(STEALTH) && a.has_spell(INVISIBILITY));
            let took = take(m, alice);
            let seen = auras::auras(m, alice)
                .is_some_and(|a| !a.has_spell(STEALTH) && !a.has_spell(INVISIBILITY));
            auras::remove_spell(m, alice, wsg::SPELL_FLAG_CARRIED[1], None);
            wsg::update(m);
            crate::game::map::combat::deal_damage(m, grom, alice, 10_000_000);
            let dead = take(m, alice);
            (refused, allowed, (hidden, took, seen), dead)
        })
        .await
        .unwrap();
    let (refused, allowed, hidden, dead) = got;
    for (what, took) in refused {
        assert!(!took, "{what}: refused");
    }
    for (id, took) in allowed {
        assert!(took, "{id}: allowed");
    }
    assert_eq!(hidden, (true, true, true), "out of hiding as she takes it");
    assert!(!dead, "the dead can't");
}
