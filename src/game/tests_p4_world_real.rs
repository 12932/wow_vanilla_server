//! Phase 3's fourth round against the real data, ported from Thistle
//! Tea's `test/game/world/entity/event_sink_test.exs`
//! (docs/parity/matrix.csv): spells another spell or an aura casts land
//! where the real data aims them.

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells;

const JUDGEMENT_OF_RIGHTEOUSNESS: u32 = 20187;
const JUDGEMENT_OF_COMMAND: u32 = 20467;
const SEAL_OF_COMMAND_PROC: u32 = 20424;
const HOLY_NOVA_HEAL: u32 = 23455;
const FIRE_NOVA: u32 = 8349;

/// Thistle Tea's "a released judgement trigger delivers its encoded spell
/// to the victim", "a remote command trigger stays targeted on the
/// victim", "a local command proc snapshots weapon damage", "a resolved
/// party trigger returns to the caster owner" and "a DBC area trigger
/// resolves hostile targets around its source": Judgement of Righteousness
/// and of Command, and Seal of Command's blow, cast as their seal casts
/// them, hurt the paladin's victim; Holy Nova's heal heals its wounded
/// priest; a creature's Fire Nova burns the player beside it.
#[tokio::test]
#[ignore = "extended"]
async fn the_triggered_spells_land_where_the_data_aims_them() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, paladin, map) = player(&realm, "P4R1", "Paladine", Race::Human, Class::Paladin).await;
    let out = map
        .query(move |m| {
            ready(m, paladin, 60, &[]);
            let mut hurt = Vec::new();
            for spell in [
                JUDGEMENT_OF_RIGHTEOUSNESS,
                JUDGEMENT_OF_COMMAND,
                SEAL_OF_COMMAND_PROC,
            ] {
                let victim = dummy(m, paladin, 60, 4.0);
                let before = health(m, victim);
                spells::cast_triggered(m, paladin, spell, Some(victim));
                hurt.push((spell, before > health(m, victim)));
            }
            set_health(m, paladin, 10);
            spells::cast_triggered(m, paladin, HOLY_NOVA_HEAL, Some(paladin));
            let healed = health(m, paladin) > 10;
            set_health(m, paladin, 100_000);
            let kobold = dummy(m, paladin, 60, 3.0);
            let before = health(m, paladin);
            spells::cast_triggered(m, kobold, FIRE_NOVA, Some(kobold));
            let burned = health(m, paladin) < before;
            (hurt, healed, burned)
        })
        .await
        .unwrap();
    let (hurt, healed, burned) = out;
    for (spell, hit) in hurt {
        assert!(hit, "{spell} hurts its victim");
    }
    assert!(healed, "Holy Nova's heal heals its caster");
    assert!(
        burned,
        "the creature's Fire Nova burns the player beside it"
    );
}
