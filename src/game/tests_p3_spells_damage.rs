//! Phase 3's spell damage and healing, against the real data when it is
//! here, as Thistle Tea's tests of them have it: what a target's damage
//! taken does to a spell's hit and tick.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells::outcome::{self, Kind, Landing};
use crate::game::map::{Map, auras};

const FIRE_BLAST: u32 = 2136;
const DAMPEN_MAGIC: u32 = 604;
const CORRUPTION: u32 = 172;
const SINISTER_STRIKE: u32 = 1752;

/// `s`'s direct damage of `base` from `caster` on `target`.
fn hit_of(m: &Map, caster: Guid, target: Guid, spell: u32, base: f32) -> u32 {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    outcome::damage(m, caster, target, &s, 0, base, &Landing::default()).amount
}

/// A lasting damage taken aura of `amount` for `schools` on `g`.
fn taken(m: &mut Map, g: Guid, id: u32, schools: i32, amount: i32) {
    let mut s = made_up(id, 6, auras::AURA_MOD_DAMAGE_TAKEN, schools, amount);
    s.duration_index = 21;
    apply_template(m, g, g, &s);
}

#[tokio::test]
async fn a_spells_flat_damage_taken_goes_by_its_coefficient_and_takes_at_most_half() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, mage, map) = player(&realm, "P3D1", "Takenone", Race::Human, Class::Mage).await;
    let out = map
        .query(move |m| {
            ready(m, mage, 60, &[]);
            let kobold = dummy(m, mage, 60, 5.0);
            let data = m.realm.data.clone();
            let fire_blast = data.spells.get(FIRE_BLAST).cloned().unwrap();
            let direct = outcome::coefficient(m, &fire_blast, 0, Kind::Direct);
            let plain = hit_of(m, mage, kobold, FIRE_BLAST, 100.0);
            // Forty more fire taken: Fire Blast's share of it.
            taken(m, kobold, 990_801, 4, 40);
            let more = hit_of(m, mage, kobold, FIRE_BLAST, 100.0);
            // An ability's is whole.
            taken(m, kobold, 990_802, 1, 40);
            let strike = hit_of(m, mage, kobold, SINISTER_STRIKE, 100.0);
            auras::remove_spell(m, kobold, 990_801, None);
            auras::remove_spell(m, kobold, 990_802, None);
            // Dampen Magic's ten less, by the coefficient.
            apply(m, kobold, kobold, DAMPEN_MAGIC);
            let dampen =
                auras::auras(m, kobold).map_or(0, |a| a.sum_mask(auras::AURA_MOD_DAMAGE_TAKEN, 4));
            let dampened = hit_of(m, mage, kobold, FIRE_BLAST, 100.0);
            auras::remove_spell(m, kobold, DAMPEN_MAGIC, None);
            // A thousand less takes only half.
            taken(m, kobold, 990_803, 4, -1000);
            let halved = hit_of(m, mage, kobold, FIRE_BLAST, 60.0);
            auras::remove_spell(m, kobold, 990_803, None);
            // A tick takes its share by the tick's coefficient.
            let corruption = data.spells.get(CORRUPTION).cloned().unwrap();
            let dot = outcome::coefficient(m, &corruption, 0, Kind::Dot);
            crate::game::map::spells::cast_triggered(m, mage, CORRUPTION, Some(kobold));
            let tick = holder(m, kobold, CORRUPTION)
                .and_then(|h| h.effects.first().map(|x| x.amount))
                .unwrap_or(0);
            taken(m, kobold, 990_804, 32, 40);
            let before = health(m, kobold);
            tick_now(m, kobold, CORRUPTION);
            let ticked = before - health(m, kobold);
            (
                direct, plain, more, strike, dampen, dampened, halved, dot, tick, ticked,
            )
        })
        .await
        .unwrap();
    let (direct, plain, more, strike, dampen, dampened, halved, dot, tick, ticked) = out;
    assert_eq!(plain, 100);
    assert!(
        direct > 0.1 && direct < 0.5,
        "Fire Blast's coefficient: {direct}"
    );
    assert_eq!(
        more,
        (100.0 + 40.0 * direct) as u32,
        "forty by the coefficient"
    );
    assert_eq!(strike, 140, "an ability takes it whole");
    assert!(dampen < 0, "Dampen Magic takes some off");
    assert_eq!(
        dampened,
        (100.0 + dampen as f32 * direct) as u32,
        "by the coefficient"
    );
    assert_eq!(halved, 30, "at most half");
    assert!(tick > 0 && dot > 0.0);
    assert_eq!(
        ticked,
        (tick as f32 + 40.0 * dot) as u32,
        "a tick by its coefficient"
    );
}
