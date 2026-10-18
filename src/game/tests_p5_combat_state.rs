//! Phase 3's fifth round of combat tests, entering combat (Thistle Tea's
//! `combat_state_test.exs`): a cast not for a fight stops as one begins,
//! one for a fight goes on, and a fight going on already stops nothing.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{ATTR_NOT_IN_COMBAT, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, combat};
use crate::game::tests_p3_combat::{self as base, *};

const PEACEFUL: u32 = 90_900;
const ORDINARY: u32 = 90_901;

fn ten_seconds(id: u32, peaceful: bool) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        casting_time_index: 91,
        ..Default::default()
    };
    if peaceful {
        s.attributes[0] |= ATTR_NOT_IN_COMBAT;
    }
    s.effects[0] = SpellEffect {
        effect: 3,
        target_a: 1,
        ..Default::default()
    };
    s
}

fn data() -> GameData {
    let mut d = base::data();
    d.spells.cast_times.insert(91, 10_000);
    for s in [ten_seconds(PEACEFUL, true), ten_seconds(ORDINARY, false)] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn begin(m: &mut Map, g: Guid, spell: u32) {
    m.players.get_mut(&g).unwrap().caster.gcd_until = None;
    spells::cast(
        m,
        g,
        spell,
        Targets {
            unit: Some(g),
            ..Default::default()
        },
        None,
    );
}

fn casting(m: &Map, g: Guid) -> Option<u32> {
    m.caster(g).and_then(|c| c.casting.as_ref()).map(|c| c.spell)
}

/// Thistle Tea's "interrupts delayed peaceful casts once", "preserves
/// combat spells", "combat refresh ... do not run entry interruption" and
/// "player contact ... share interruption", as mangos's `SetInCombatState`
/// has it: a ten-second cast not for a fight stops as a blow puts its
/// caster in one, and so does a channel not for one; one for a fight goes
/// on; and in a fight already, another blow stops nothing.
#[tokio::test]
async fn a_cast_not_for_a_fight_stops_as_one_begins() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .extend([PEACEFUL, ORDINARY]);
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            begin(m, ga, PEACEFUL);
            let began = casting(m, ga);
            combat::engage(m, brute, ga);
            let struck = casting(m, ga);
            combat::leave_combat(m, ga);
            begin(m, ga, ORDINARY);
            combat::engage(m, brute, ga);
            let ordinary = casting(m, ga);
            // In the fight already: a peaceful cast put there by hand
            // isn't stopped by the next blow.
            spells::interrupt(m, ga);
            let p = m.players.get_mut(&ga).unwrap();
            p.caster.casting = None;
            p.fields.set_flag(
                crate::game::fields::UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                false,
            );
            begin(m, ga, PEACEFUL);
            in_combat(m, ga);
            combat::engage(m, brute, ga);
            let refreshed = casting(m, ga);
            // Channelled, out of the fight: it stops too.
            combat::leave_combat(m, ga);
            let now = m.now;
            if let Some(c) = m.players.get_mut(&ga).unwrap().caster.casting.as_mut() {
                c.duration = 0;
                c.channel_until = Some(now + std::time::Duration::from_secs(10));
            }
            combat::engage(m, brute, ga);
            let channel = casting(m, ga);
            (began, struck, ordinary, refreshed, channel)
        })
        .await
        .unwrap();
    assert_eq!(r.4, None, "a peaceful channel stops");
    assert_eq!(r.0, Some(PEACEFUL), "begun");
    assert_eq!(r.1, None, "stopped by the fight");
    assert_eq!(r.2, Some(ORDINARY), "a fight's goes on");
    assert_eq!(r.3, Some(PEACEFUL), "a fight going on stops nothing");
}

/// Mark `g` in combat, as a fight going on has them.
fn in_combat(m: &mut Map, g: Guid) {
    m.players.get_mut(&g).unwrap().fields.set_flag(
        crate::game::fields::UNIT_FIELD_FLAGS,
        crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        true,
    );
}
