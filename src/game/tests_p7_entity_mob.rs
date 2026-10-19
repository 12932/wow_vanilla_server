//! Round 7's mob tests, ported from Thistle Tea's
//! `test/game/world/entity/mob/*_test.exs` (docs/parity/matrix.csv): a
//! creature's summoned demons, their level and lifetime.

use std::time::Duration;

use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::tests_pets::{self, IMP, SUMMON_IMP, WOLF, enter, place};

/// A creature's Summon Pet makes a guardian of its own (mangos-classic's
/// `EffectSummonPet`, a `GUARDIAN_PET`), its summoner's, at its level less
/// the effect's offset, cut toward zero as mangos's float to `uint32`
/// cuts it (a level 20 summoner's -2.9 is 17, where Thistle Tea rounds
/// it to 18); an offset that leaves no level keeps the template's (where
/// Thistle Tea floors it at 1), and the spell's duration is its lifetime
/// (Thistle Tea's "controlled pets retain template names and source
/// lifetime at owner level" and "scales from owner level with signed
/// offsets and a level-one floor"; mangos's `EffectSummonGuardian`).
#[tokio::test]
async fn a_creatures_demon_is_at_its_level_less_the_offset_for_the_spells_time() {
    let mut d = tests_pets::data();
    let imp = d.creature_templates.get_mut(&IMP).unwrap();
    (imp.level_min, imp.level_max) = (5, 5);
    let base = d.spells.spells[&SUMMON_IMP].clone();
    let mut ids = Vec::new();
    for (i, offset) in [0.0f32, -2.9, -99.0].into_iter().enumerate() {
        let mut s = base.clone();
        s.id = 97_720 + i as u32;
        s.effects[0].multiple_value = offset;
        // The first lasts a minute.
        s.duration_index = if i == 0 { 4 } else { 0 };
        ids.push(s.id);
        d.spells.spells.insert(s.id, s);
    }
    let realm = crate::game::harness::realm_with(d);
    let (_w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    let summoner = place(&map, gw, WOLF, 0x70_0200, 10.0).await;
    let out = map
        .query(move |m| {
            m.creature_mut(summoner)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_LEVEL, 20);
            let now = m.now;
            let mut out = Vec::new();
            for id in ids {
                crate::game::map::spells::cast_triggered(m, summoner, id, None);
                let c = m
                    .creatures
                    .iter()
                    .map(|(_, c)| c)
                    .find(|c| c.entry == IMP && c.fields.get_u32(UNIT_CREATED_BY_SPELL) == id)
                    .unwrap();
                let s = c.scripted.as_deref().unwrap();
                out.push((
                    c.fields.get_u32(UNIT_FIELD_LEVEL),
                    s.summoner == Some(summoner) && s.guardian,
                    s.despawn_at.map(|t| t - now),
                ));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![
            (20, true, Some(Duration::from_secs(60))),
            (17, true, None),
            (5, true, None),
        ]
    );
}
