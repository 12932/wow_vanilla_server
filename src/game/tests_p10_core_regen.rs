//! Round 10, against Thistle Tea's `power/regen_test.exs`: a creature a
//! player controls (a pet, a charmed creature) heals from its spirit as a
//! player of its class, four seconds' worth every four seconds, out of a
//! fight; polymorphed, a tenth, in a fight or out; a guardian a twentieth.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::auras;
use crate::game::map::creature::Creature;
use crate::game::map::pets::{self, Kind, PetState};
use crate::game::tests_p3_combat::*;

const POLYMORPH: u32 = 98_201;

/// A warrior-class creature of `kind`, `g`'s, of `spirit` and at `health`
/// of `max`.
fn controlled(m: &mut Map, g: Guid, kind: Kind, spirit: u32, health: u32, max: u32) -> Guid {
    let t = template(BRUTE, 7);
    let pos = m.players[&g].position();
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(guid::unit(BRUTE, counter | 0x80_0000), &t, 60, pos, 0.0);
    c.fields.set_u32(UNIT_FIELD_STAT0 + 4, spirit);
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, max);
    c.fields.set_u32(UNIT_FIELD_HEALTH, health);
    c.owner = Some(g);
    c.pet = Some(Box::new(PetState::new(kind, g, m.now)));
    let c = c.with_base();
    m.add_creature(c)
}

fn health(m: &Map, c: Guid) -> u32 {
    m.unit_fields(c).unwrap().get_u32(UNIT_FIELD_HEALTH)
}

/// Its health after a regeneration `secs` from now.
fn after(m: &mut Map, c: Guid, secs: u64) -> u32 {
    let at = m.now + Duration::from_secs(secs);
    pets::ai::regen(m, c, at);
    health(m, c)
}

fn in_combat(m: &mut Map, c: Guid) {
    let f = m.unit_fields_mut(c).unwrap();
    let flags = f.get_u32(UNIT_FIELD_FLAGS);
    f.set_u32(
        UNIT_FIELD_FLAGS,
        flags | crate::game::map::player::UNIT_FLAG_IN_COMBAT,
    );
}

#[tokio::test]
async fn a_controlled_creature_heals_from_its_spirit_and_a_tenth_polymorphed() {
    let mut d = data();
    // A mage's Polymorph: a confusion of the mage's family that prevents
    // by silence.
    let mut poly = SpellTemplate {
        id: POLYMORPH,
        name: "Polymorph".into(),
        equipped_item_class: -1,
        family_name: 3,
        prevention_type: 1,
        ..Default::default()
    };
    poly.effects[0] = SpellEffect {
        effect: 6,
        aura: auras::AURA_MOD_CONFUSE,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(POLYMORPH, poly);
    let (_realm, _a, g, map) = setup_with(d, Class::Warrior).await;
    let r = map
        .query(move |m| {
            let mut out = Vec::new();
            // A demon and a charmed creature of 100 spirit: a warrior's
            // 103 a tick, four of them; not before its four seconds.
            for kind in [Kind::Demon, Kind::Charmed, Kind::Hunter] {
                let c = controlled(m, g, kind, 100, 3_000, 31_440);
                let early = after(m, c, 2);
                out.push((early, after(m, c, 4)));
            }
            // Of 10 spirit: nothing.
            let dull = controlled(m, g, Kind::Demon, 10, 3_000, 31_440);
            out.push((after(m, dull, 4), 0));
            // In a fight: nothing; polymorphed in one, a tenth of 305.
            let sheep = controlled(m, g, Kind::Demon, 100, 50, 305);
            in_combat(m, sheep);
            let fighting = after(m, sheep, 4);
            let s = m.realm.data.spells.get(POLYMORPH).unwrap().clone();
            auras::apply_spell(m, g, sheep, &s, &[(0, 0)], Default::default()).unwrap();
            out.push((fighting, after(m, sheep, 8)));
            // A guardian, a twentieth.
            let guardian = controlled(m, g, Kind::Guardian, 100, 1_000, 10_000);
            out.push((after(m, guardian, 4), 0));
            out
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        vec![
            (3_000, 3_412),
            (3_000, 3_412),
            (3_000, 3_412),
            (3_000, 0),
            (50, 80),
            (1_500, 0),
        ]
    );
}
