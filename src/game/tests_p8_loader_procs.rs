//! Phase 8's loader round, the procs (Thistle Tea's `world/loader/proc_*_dbc_test.exs`,
//! `cast_proc_dbc_test.exs`, `class_script_dbc_test.exs`): what the real
//! proc auras go off on, with vmangos's `spell_proc_event` over them, and
//! the one-cast modifiers.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, ProcEvent};
use crate::game::map::procs::{self, *};
use crate::game::map::spells::{self, mods, outcome};
use crate::game::map::{Map, MapHandle, auras};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

/// `spell` (or a swing) between `actor` and `target`, with `ex`, as the
/// spell's own cast or hit tells the procs of both.
fn event(m: &mut Map, actor: Guid, target: Guid, spell: Option<u32>, ex: u32) {
    let (actor_flags, target_flags, school) = match spell {
        Some(id) => {
            let s = m.realm.data.spells.get(id).cloned().unwrap();
            let (done, taken) = outcome::proc_flags(&s);
            (done, taken, s.school_mask())
        }
        None => (PROC_DONE_MELEE, PROC_TAKEN_MELEE | PROC_TAKEN_DAMAGE, 1),
    };
    procs::trigger(
        m,
        Event {
            actor,
            target,
            actor_flags,
            target_flags: if ex & EX_CAST_END != 0 {
                0
            } else {
                target_flags
            },
            ex,
            spell,
            damage: 10,
            school,
            ..Default::default()
        },
    );
}

/// Thistle Tea's proc damage (`proc_damage_dbc_test.exs`): Flameblade,
/// the fire and frost weapon and armor procs, Holy Shield and Blessing of
/// Sanctuary deal their damage by a proc (`PROC_TRIGGER_DAMAGE`), not as
/// a damage shield. Holy Shield's and Sanctuary's later ranks have their
/// first rank's block-only rule (`spell_proc_event`'s procEx 0x40): a
/// blow that lands does nothing, a blocked one strikes back, without a
/// shield on (their spells need none). Flameblade (7808) burns what its
/// bearer strikes, not what strikes it.
#[tokio::test]
#[ignore = "extended"]
async fn proc_damage_strikes_on_its_rule_and_stays_no_shield() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    for id in [7808, 8788, 9782, 9784, 16_624, 20_911, 20_925, 20_928] {
        let s = d.spells.get(id).unwrap();
        let kinds: Vec<u32> = s.effects.iter().map(|e| e.aura).collect();
        assert!(kinds.contains(&auras::AURA_PROC_TRIGGER_DAMAGE), "{id}");
        assert!(!kinds.contains(&auras::AURA_DAMAGE_SHIELD), "{id}");
    }
    let ranks = [20_925, 20_927, 20_928, 20_911, 20_912, 20_913, 20_914];
    for id in ranks {
        assert_eq!(
            d.spells.proc_events.get(&id).map(|p| p.ex),
            Some(EX_BLOCK),
            "{id}"
        );
    }
    let (_c, g, map) = player(&realm, "P8LBLOCK", "Blocker", Race::Human, Class::Paladin).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let mut seen = Vec::new();
        for id in ranks {
            let foe = dummy(m, g, 60, 2.0);
            apply(m, g, g, id);
            event(m, foe, g, None, EX_NORMAL_HIT);
            let landed = health(m, foe);
            event(m, foe, g, None, EX_BLOCK);
            seen.push((id, landed == 1_000_000, health(m, foe) < 1_000_000));
            auras::remove_spell(m, g, id, None);
        }
        let foe = dummy(m, g, 60, 2.0);
        apply(m, g, g, 7808);
        event(m, foe, g, None, EX_NORMAL_HIT);
        let struck = health(m, foe);
        event(m, g, foe, None, EX_NORMAL_HIT);
        (seen, struck == 1_000_000, health(m, foe) < 1_000_000)
    })
    .await;
    let (seen, spared, burned) = out;
    for (id, landed, blocked) in seen {
        assert!(landed, "{id}: a blow that lands, nothing");
        assert!(blocked, "{id}: a blocked one, damage");
    }
    assert!(spared, "Flameblade: no burn for what strikes it");
    assert!(burned, "a burn for what it strikes");
}

/// Thistle Tea's "an explicit higher-rank proc rule takes precedence"
/// (vmangos's `LoadSpellProcEvents`): a later rank with a rule of its own
/// keeps it when the ranks take their first's; one without takes the
/// first's.
#[test]
#[ignore = "extended"]
fn a_later_ranks_own_proc_rule_stands_over_its_firsts() {
    if !have_data() {
        return;
    }
    let mut d = crate::game::harness::load_real(&crate::game::harness::real_data("db"));
    let s = &mut d.spells;
    let own = ProcEvent {
        ex: EX_BLOCK,
        ppm: 2.5,
        ..Default::default()
    };
    s.proc_events.insert(20_928, own);
    s.proc_events.remove(&20_927);
    s.inherit_proc_events();
    assert_eq!(s.proc_events[&20_928].ppm, 2.5, "its own");
    assert_eq!(s.proc_events[&20_927].ex, s.proc_events[&20_925].ex);
    assert_eq!(s.proc_events[&20_927].ppm, 0.0, "its first's");
}

/// Thistle Tea's equipment rule (`proc_equipment_dbc_test.exs`): the
/// weapon talents (Sword Specialization's and Mace Specialization's
/// ranks, 13964) want a weapon to proc; Shield Block, Redoubt and Holy
/// Shield are a shield's and proc without one
/// (`SPELL_ATTR_EX3_NO_PROC_EQUIP_REQUIREMENT`), as the Holy Shield test
/// above has it.
#[test]
#[ignore = "extended"]
fn weapon_talents_want_their_weapon_and_shield_spells_dont() {
    let Some(d) = real() else {
        return;
    };
    for id in [12_281, 12_284, 12_834, 12_867, 13_964] {
        let s = d.spells.get(id).unwrap();
        assert_eq!(s.equipped_item_class, 2, "{id}");
        assert!(!s.attr(3, sd::ATTR_EX3_NO_PROC_EQUIP_REQUIREMENT), "{id}");
    }
    for id in [2565, 20_127, 20_928] {
        let s = d.spells.get(id).unwrap();
        assert_eq!(s.equipped_item_class, 4, "{id}");
        assert!(s.attr(3, sd::ATTR_EX3_NO_PROC_EQUIP_REQUIREMENT), "{id}");
    }
}

const ELEMENTAL_FOCUS: u32 = 16_164;
const CLEARCASTING: u32 = 16_246;
const BLUE_DRAGON: u32 = 23_688;
const BLUE_DRAGON_AURA: u32 = 23_684;

/// Whether `id`, cast by `g` at `at`, set `aura` going: on its cast's end,
/// and on a hit alone.
fn on_cast_end(m: &mut Map, g: Guid, at: Guid, id: u32, aura: u32) -> (bool, bool) {
    auras::remove_spell(m, g, aura, None);
    event(m, g, at, Some(id), EX_CAST_END | EX_NORMAL_HIT);
    let ended = has(m, g, aura);
    auras::remove_spell(m, g, aura, None);
    event(m, g, at, Some(id), EX_NORMAL_HIT);
    let hit = has(m, g, aura);
    auras::remove_spell(m, g, aura, None);
    (ended, hit)
}

/// Thistle Tea's cast-end procs (`cast_proc_dbc_test.exs`), sure to go off:
/// Elemental Focus (10%) gives Clearcasting at the end of a harmful
/// shaman spell of fire, nature or frost (Lightning Bolt, Flame Shock,
/// Frost Shock), not on its hit, nor for a Frostbolt or a Healing Wave.
/// The Blue Dragon (2%) comes of any spell's end, helpful or harmful
/// (Lightning Bolt, Healing Wave, and Arcane Intellect, a spell, not an
/// ability, as mangos's `PrepareMasksForProcSystem` has it), not of
/// Hunter's Mark or Detect Magic, which suppress their caster's procs.
/// A Frost Shock cast for real gives Clearcasting.
#[tokio::test]
#[ignore = "extended"]
async fn elemental_focus_and_the_blue_dragon_come_of_a_casts_end() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        for id in [ELEMENTAL_FOCUS, BLUE_DRAGON] {
            if let Some(s) = d.spells.spells.get_mut(&id) {
                s.proc_chance = 100;
            }
        }
    });
    let d = realm.data.clone();
    let focus = d.spells.proc_events[&ELEMENTAL_FOCUS];
    assert_eq!(
        (focus.school_mask, focus.family, focus.flags, focus.ex),
        (28, 11, PROC_DONE_NEGATIVE_SPELL, EX_CAST_END)
    );
    assert_eq!(d.spells.proc_events[&BLUE_DRAGON].ex, EX_CAST_END);
    for id in [1130, 2855] {
        assert!(
            d.spells
                .get(id)
                .unwrap()
                .attr(3, sd::ATTR_EX3_SUPPRESS_CASTER_PROCS),
            "{id}"
        );
    }
    let (_c, g, map) = player(&realm, "P8LFOCUS", "Focuser", Race::Orc, Class::Shaman).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let foe = dummy(m, g, 60, 10.0);
        apply(m, g, g, ELEMENTAL_FOCUS);
        let focus: Vec<(u32, (bool, bool))> = [403, 8050, 8056, 116, 331]
            .into_iter()
            .map(|id| {
                let at = if id == 331 { g } else { foe };
                (id, on_cast_end(m, g, at, id, CLEARCASTING))
            })
            .collect();
        let cast = cast_fresh(m, g, 8056, at(foe));
        let real = (cast, has(m, g, CLEARCASTING));
        auras::remove_spell(m, g, CLEARCASTING, None);
        auras::remove_spell(m, g, ELEMENTAL_FOCUS, None);
        apply(m, g, g, BLUE_DRAGON);
        let dragon: Vec<(u32, (bool, bool))> = [403, 331, 1459, 1130, 2855]
            .into_iter()
            .map(|id| {
                let at = if matches!(id, 331 | 1459) { g } else { foe };
                (id, on_cast_end(m, g, at, id, BLUE_DRAGON_AURA))
            })
            .collect();
        (focus, real, dragon)
    })
    .await;
    let (focus, real, dragon) = out;
    assert_eq!(
        focus,
        vec![
            (403, (true, false)),
            (8050, (true, false)),
            (8056, (true, false)),
            (116, (false, false)),
            (331, (false, false)),
        ]
    );
    assert_eq!(real, (true, true), "a real Frost Shock");
    assert_eq!(
        dragon,
        vec![
            (403, (true, false)),
            (331, (true, false)),
            (1459, (true, false)),
            (1130, (false, false)),
            (2855, (false, false)),
        ]
    );
}

const NIGHTFALL: [u32; 2] = [18_094, 18_095];
const SHADOW_TRANCE: u32 = 17_941;
const SHADOW_BOLT: u32 = 686;
const CORRUPTION: u32 = 172;
const DRAIN_LIFE: u32 = 689;

/// `g`'s cast time of `id` now, with its modifiers.
fn cast_ms(m: &Map, g: Guid, id: u32) -> u32 {
    let s = m.realm.data.spells.get(id).cloned().unwrap();
    let mods = mods::for_spell(m, g, &s);
    spells::cast_time(m, g, &s, &mods)
}

/// Thistle Tea's Nightfall (`class_script_dbc_test.exs`), sure to go off:
/// both ranks (2% and 4%) give Shadow Trance on a Corruption tick or a
/// Drain Life tick, not on a Shadow Bolt. Shadow Trance makes the next
/// Shadow Bolt instant, once: it has one charge (mangos's
/// `HandleAddModifier`, its data has none), and the bolt spends it.
#[tokio::test]
#[ignore = "extended"]
async fn nightfall_gives_one_instant_shadow_bolt_from_a_drains_or_corruptions_tick() {
    if !have_data() {
        return;
    }
    let realm = realm_plus(|d| {
        for id in NIGHTFALL {
            if let Some(s) = d.spells.spells.get_mut(&id) {
                s.proc_chance = 100;
            }
        }
    });
    let d = realm.data.clone();
    assert_eq!(d.spells.get(SHADOW_TRANCE).unwrap().proc_charges, 0);
    let chances: Vec<u32> = NIGHTFALL
        .iter()
        .map(|id| real().map_or(0, |r| r.spells.get(*id).unwrap().proc_chance))
        .collect();
    assert_eq!(chances, vec![2, 4]);
    let (_c, g, map) = player(
        &realm,
        "P8LNIGHT",
        "Nightfaller",
        Race::Human,
        Class::Warlock,
    )
    .await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[SHADOW_BOLT]);
        let mut procs = Vec::new();
        for talent in NIGHTFALL {
            apply(m, g, g, talent);
            for id in [CORRUPTION, DRAIN_LIFE] {
                auras::remove_spell(m, g, SHADOW_TRANCE, None);
                let foe = dummy(m, g, 60, 10.0);
                apply(m, g, foe, id);
                tick_now(m, foe, id);
                procs.push((talent, id, has(m, g, SHADOW_TRANCE)));
            }
            auras::remove_spell(m, g, SHADOW_TRANCE, None);
            let foe = dummy(m, g, 60, 10.0);
            event(m, g, foe, Some(SHADOW_BOLT), EX_NORMAL_HIT);
            procs.push((talent, SHADOW_BOLT, has(m, g, SHADOW_TRANCE)));
            auras::remove_spell(m, g, talent, None);
        }
        let plain = cast_ms(m, g, SHADOW_BOLT);
        apply(m, g, g, SHADOW_TRANCE);
        let charges = holder(m, g, SHADOW_TRANCE).map(|h| h.charges);
        let tranced = cast_ms(m, g, SHADOW_BOLT);
        let foe = dummy(m, g, 60, 10.0);
        let cast = cast_fresh(m, g, SHADOW_BOLT, at(foe));
        let instant = casting(m, g).is_none();
        (
            procs,
            plain,
            charges,
            tranced,
            cast && instant,
            has(m, g, SHADOW_TRANCE),
            cast_ms(m, g, SHADOW_BOLT),
        )
    })
    .await;
    let (procs, plain, charges, tranced, instant, kept, after) = out;
    assert_eq!(
        procs,
        vec![
            (18_094, CORRUPTION, true),
            (18_094, DRAIN_LIFE, true),
            (18_094, SHADOW_BOLT, false),
            (18_095, CORRUPTION, true),
            (18_095, DRAIN_LIFE, true),
            (18_095, SHADOW_BOLT, false),
        ]
    );
    assert!(plain > 0);
    assert_eq!(charges, Some(1), "one charge");
    assert_eq!(tranced, 0, "an instant bolt");
    assert!(instant, "cast at once");
    assert!(!kept, "and the trance spent");
    assert_eq!(after, plain, "the next one as it was");
}

const NETHERWIND_FOCUS: u32 = 22_008;

/// Thistle Tea's "Netherwind Focus has one cast charge despite its zero DBC
/// proc charges": the next mage spell it names (Fireball) is instant and
/// spends it; Fire Blast, which it doesn't name, leaves it be.
#[tokio::test]
#[ignore = "extended"]
async fn netherwind_focus_makes_one_spell_instant() {
    if !have_data() {
        return;
    }
    let realm = realm();
    assert_eq!(
        realm
            .data
            .spells
            .get(NETHERWIND_FOCUS)
            .unwrap()
            .proc_charges,
        0
    );
    let (_c, g, map) = player(&realm, "P8LWIND", "Windfocus", Race::Human, Class::Mage).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[133, 2136]);
        let foe = dummy(m, g, 60, 10.0);
        let plain = cast_ms(m, g, 133);
        apply(m, g, g, NETHERWIND_FOCUS);
        let charges = holder(m, g, NETHERWIND_FOCUS).map(|h| h.charges);
        let focused = cast_ms(m, g, 133);
        cast_fresh(m, g, 2136, at(foe));
        let after_blast = has(m, g, NETHERWIND_FOCUS);
        let cast = cast_fresh(m, g, 133, at(foe));
        let instant = casting(m, g).is_none();
        (
            plain,
            charges,
            focused,
            after_blast,
            cast && instant,
            has(m, g, NETHERWIND_FOCUS),
        )
    })
    .await;
    assert!(out.0 > 0);
    assert_eq!(out.1, Some(1), "one charge");
    assert_eq!(out.2, 0, "an instant Fireball");
    assert!(out.3, "Fire Blast leaves it");
    assert!(out.4, "the Fireball cast at once");
    assert!(!out.5, "and spends it");
}
