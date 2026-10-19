//! Phase 6's loader round, the spells (Thistle Tea's `world/loader/*_dbc_test.exs`
//! of spells): what wvs reads of the real spells, and what they do.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::fields::*;
use crate::game::map::combat::{self, Outcome};
use crate::game::map::spells::{self, fail};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::forced_swing;

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

fn channel(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_CHANNEL_SPELL))
}

/// `g` casting `spell`, its cast time to go, as though begun.
fn begun(m: &mut Map, g: Guid, spell: u32) {
    let now = m.now;
    if let Some(c) = m.caster_mut(g) {
        c.casting = Some(spells::Casting {
            spell,
            targets: at(g),
            ends: now + Duration::from_millis(1500),
            item: None,
            pushbacks: 0,
            channel_until: None,
            duration: 1500,
        });
    }
}

const BLACKFATHOM_CHANNELING: u32 = 8734;
const DESTROY_SPEAR: u32 = 16_557;

/// Thistle Tea's `CombatState.enter` (`combat_entry_dbc_test.exs`) and
/// mangos-classic's `SetInCombatState`: Blackfathom Channeling (a duration
/// of -1) runs on past a minute, until its creature's fight starts (it is
/// not for one); a fight stops a mount's (458) and Resurrection's (2006)
/// cast, not Fireball's, Conjure Food's, Summon Imp's or the
/// Hearthstone's; Destroy Spear's channel stops at the first blow, which
/// is what starts the fight (mangos's channel that damage breaks, any
/// unit's).
#[tokio::test]
#[ignore = "extended"]
async fn real_channels_and_casts_stop_or_go_on_as_a_fight_starts() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P6LFIGHT", "Fighta", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let acolyte = dummy(m, g, 30, 5.0);
        spells::cast_triggered(m, acolyte, BLACKFATHOM_CHANNELING, Some(acolyte));
        let started = (casting(m, acolyte), channel(m, acolyte));
        pass(m, 61_000);
        let later = (casting(m, acolyte), channel(m, acolyte));
        combat::enter(m, acolyte, false);
        let fought = (casting(m, acolyte), channel(m, acolyte));
        let guard = dummy(m, g, 30, 3.0);
        spells::cast_triggered(m, guard, DESTROY_SPEAR, Some(guard));
        let spear = casting(m, guard);
        forced_swing(m, g, guard, Outcome::Hit);
        let struck = (casting(m, guard), channel(m, guard));
        let mut stopped = Vec::new();
        for id in [458, 2006, 133, 587, 688, 8690] {
            combat::leave_combat(m, g);
            begun(m, g, id);
            combat::enter(m, g, false);
            stopped.push((id, casting(m, g).is_none()));
            fresh(m, g);
        }
        (started, later, fought, spear, struck, stopped)
    })
    .await;
    let (started, later, fought, spear, struck, stopped) = out;
    let on = Some(BLACKFATHOM_CHANNELING);
    assert_eq!(started, (on, BLACKFATHOM_CHANNELING));
    assert_eq!(later, (on, BLACKFATHOM_CHANNELING), "a minute on");
    assert_eq!(fought, (None, 0), "the fight ends it");
    assert_eq!(spear, Some(DESTROY_SPEAR));
    assert_eq!(struck, (None, 0), "a blow ends Destroy Spear");
    assert_eq!(
        stopped,
        vec![
            (458, true),
            (2006, true),
            (133, false),
            (587, false),
            (688, false),
            (8690, false)
        ]
    );
}

const HAMMER_OF_JUSTICE: u32 = 853;
const KIDNEY_SHOT: u32 = 408;
const FEAR: u32 = 5782;
const SILENCE: u32 = 15_487;
const DIVINE_SHIELD: u32 = 642;
const DIVINE_SHIELD_2: u32 = 1020;
const BLESSING_OF_PROTECTION: u32 = 1022;
const ICE_BLOCK: u32 = 11_958;
const BLINK: u32 = 1953;
const WILL_OF_THE_FORSAKEN: u32 = 7744;
const FIREBALL: u32 = 133;

/// What stops `g` casting `spell` at itself under `controls`: the cast
/// check's code.
fn under(m: &mut Map, g: Guid, from: Guid, controls: &[u32], spell: u32) -> Option<u8> {
    let data = m.realm.data.clone();
    for c in [HAMMER_OF_JUSTICE, KIDNEY_SHOT, FEAR, SILENCE] {
        crate::game::map::auras::remove_spell(m, g, c, None);
    }
    for c in controls {
        apply(m, from, g, *c);
    }
    let s = data.spells.get(spell).unwrap().clone();
    let mods = spells::mods::for_spell(m, g, &s);
    fresh(m, g);
    spells::checks::check(m, g, &s, &at(g), None, false, &mods)
}

/// Thistle Tea's `CasterState` (`immunity_escape_dbc_test.exs`) and
/// mangos-classic's `CheckCasterAuras`: a spell whose immunities purge
/// (`SPELL_ATTR_EX_IMMUNITY_PURGES_EFFECT`) is cast under what they cancel,
/// and only that. Divine Shield (both ranks) and Ice Block (every school)
/// under a holy stun, a fear and a silence at once; Blessing of Protection
/// (physical) under Kidney Shot, not Hammer of Justice (holy) nor with a
/// fear; Will of the Forsaken under a fear; Blink (stun and root) under a
/// stun, not a silence, nor a stun and a silence (mangos's silence check
/// comes after the stun's; Thistle Tea lets it through). Fireball, under
/// any of them, isn't.
#[tokio::test]
#[ignore = "extended"]
async fn real_escapes_break_only_what_their_immunities_cancel() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P6LESCAPE", "Escapa", Race::Human, Class::Paladin).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let foe = dummy(m, g, 60, 5.0);
        let all = [HAMMER_OF_JUSTICE, FEAR, SILENCE];
        vec![
            under(m, g, foe, &all, DIVINE_SHIELD),
            under(m, g, foe, &all, DIVINE_SHIELD_2),
            under(m, g, foe, &all, ICE_BLOCK),
            under(m, g, foe, &[KIDNEY_SHOT], BLESSING_OF_PROTECTION),
            under(m, g, foe, &[KIDNEY_SHOT, FEAR], BLESSING_OF_PROTECTION),
            under(m, g, foe, &[HAMMER_OF_JUSTICE], BLESSING_OF_PROTECTION),
            under(m, g, foe, &[FEAR], WILL_OF_THE_FORSAKEN),
            under(m, g, foe, &[HAMMER_OF_JUSTICE], BLINK),
            under(m, g, foe, &[SILENCE], BLINK),
            under(m, g, foe, &[HAMMER_OF_JUSTICE, SILENCE], BLINK),
            under(m, g, foe, &[HAMMER_OF_JUSTICE], FIREBALL),
            under(m, g, foe, &[FEAR], FIREBALL),
            under(m, g, foe, &[SILENCE], FIREBALL),
        ]
    })
    .await;
    let control =
        |c: Option<u8>| c.filter(|c| [fail::STUNNED, fail::FLEEING, fail::SILENCED].contains(c));
    let out: Vec<Option<u8>> = out.into_iter().map(control).collect();
    assert_eq!(
        out,
        vec![
            None,
            None,
            None,
            None,
            Some(fail::FLEEING),
            Some(fail::STUNNED),
            None,
            None,
            Some(fail::SILENCED),
            Some(fail::SILENCED),
            Some(fail::STUNNED),
            Some(fail::FLEEING),
            Some(fail::SILENCED),
        ]
    );
}
