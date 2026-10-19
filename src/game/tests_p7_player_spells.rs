//! Round 7's player tests of spells, ported from Thistle Tea's
//! `test/game/world/entity/player/spellcasting_test.exs` and others
//! (docs/parity/matrix.csv): a script's cast by a player.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::{UNIT_FIELD_MAXPOWER1, UNIT_FIELD_POWER1};
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_entity::on;

/// A spell of a second and a half for ten mana, and another to be busy
/// with.
const SCRIPT_SPELL: u32 = 96_420;
const BUSY_SPELL: u32 = 96_421;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    for id in [SCRIPT_SPELL, BUSY_SPELL] {
        let mut s = crate::game::tests_p3_entity::spell(
            id,
            &[SpellEffect {
                effect: 3,
                target_a: 1,
                ..Default::default()
            }],
        );
        s.casting_time_index = 15;
        s.mana_cost = 10;
        d.spells.spells.insert(id, s);
    }
    d.spells.cast_times.insert(15, 1_500);
    d
}

/// The spell under way, and how long it has left.
fn casting(m: &Map, g: Guid) -> Option<(u32, u128)> {
    m.caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| (c.spell, c.ends.saturating_duration_since(m.now).as_millis()))
}

/// Thistle Tea's `Spellcasting.scripted_cast` "starts an unlearned script
/// spell with its real cast time", "preserves a busy cast unless the
/// script interrupts it" and "retains player death and resource
/// validation": a script has a player cast a spell they don't know, with
/// its cast time; one busy casting keeps their cast unless the script
/// interrupts it (flag 0x1); the dead, and one without the mana, cast
/// nothing and spend nothing.
#[tokio::test]
async fn a_scripts_cast_by_a_player() {
    use crate::game::map::scripts::cast;
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        assert!(!p.spells.contains(&SCRIPT_SPELL));
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1_000);
        p.fields.set_u32(UNIT_FIELD_POWER1, 1_000);
        cast(m, g, g, SCRIPT_SPELL, 0);
        let unlearned = casting(m, g);
        let known = m.players[&g].spells.contains(&SCRIPT_SPELL);
        crate::game::map::spells::interrupt(m, g);
        // Busy with another: kept, then interrupted by the script.
        crate::game::map::spells::cast(
            m,
            g,
            BUSY_SPELL,
            crate::game::map::spells::Targets::default(),
            None,
        );
        cast(m, g, g, SCRIPT_SPELL, 0);
        let kept = casting(m, g).map(|c| c.0);
        cast(m, g, g, SCRIPT_SPELL, 0x1);
        let interrupted = casting(m, g).map(|c| c.0);
        crate::game::map::spells::interrupt(m, g);
        // No mana: nothing.
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 5);
        cast(m, g, g, SCRIPT_SPELL, 0);
        let poor = (
            casting(m, g),
            m.players[&g].fields.get_u32(UNIT_FIELD_POWER1),
        );
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_POWER1, 1_000);
        crate::game::map::death::kill(m, g);
        cast(m, g, g, SCRIPT_SPELL, 0);
        let dead = casting(m, g);
        (unlearned, known, kept, interrupted, poor, dead)
    })
    .await;
    let (unlearned, known, kept, interrupted, poor, dead) = out;
    let (spell, left) = unlearned.expect("cast, unlearned");
    assert_eq!(spell, SCRIPT_SPELL);
    assert!((1_400..=1_500).contains(&left), "its cast time: {left}");
    assert!(!known, "and still not learned");
    assert_eq!(kept, Some(BUSY_SPELL), "the busy cast kept");
    assert_eq!(interrupted, Some(SCRIPT_SPELL), "interrupted by the script");
    assert_eq!(poor, (None, 5), "no mana, no cast, none spent");
    assert_eq!(dead, None, "the dead cast nothing");
}
