//! Phase 9's world spells, a third part (Thistle Tea's
//! `world/loader/*_dbc_test.exs` on real spells, a map and a player): the
//! engineers' trinkets' summons.

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::spells;
use crate::game::map::{Map, inventory};

/// The engineers' trinkets: each one's item, its spell, the summon it
/// triggers and the creature that comes.
const TRINKETS: [(u32, u32, u32, u32); 4] = [
    (16_022, 23_074, 19_804, 12_473),
    (10_576, 23_075, 12_749, 8615),
    (4396, 23_076, 4073, 2678),
    (10_725, 23_133, 13_166, 8836),
];

/// Thistle Tea's "engineering trinket wrappers retain their item through
/// triggered delivery" (`spell_guardian_dbc_test.exs`): each engineer's
/// trinket's spell (Arcanite, Mithril Mechanical and Mechanical
/// Dragonling, Gnomish Battle Chicken) used from its item triggers its
/// summon with the item, which its go names (mangos's
/// `CastSpell(m_caster, .., m_CastItem)`), and its creature comes (the
/// chicken a guardian too, by vmangos's `spell_effect_mod`, where Thistle
/// Tea's test takes the client's totem); cast with no item, it summons
/// nothing.
#[tokio::test]
#[ignore = "extended"]
async fn an_engineers_trinkets_summon_keeps_its_item() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (mut c, g, map) = player(&realm, "P9WS7", "Tinkerer", Race::Gnome, Class::Warrior).await;
    for (item, wrapper, summon, entry) in TRINKETS {
        let out = map
            .query(move |m| {
                ready(m, g, 60, &[]);
                let summons = |m: &Map| {
                    m.creatures
                        .iter()
                        .filter(|(_, c)| c.entry == entry && c.owner == Some(g))
                        .count()
                };
                spells::cast_triggered(m, g, wrapper, Some(g));
                let without = summons(m);
                inventory::add_item(m, g, item, 1, false).expect("room");
                let p = &m.players[&g];
                let guid = p.items.iter().rfind(|i| i.entry == item).unwrap().guid;
                fresh(m, g);
                spells::cast(m, g, wrapper, at(g), Some(guid));
                (guid, without, summons(m))
            })
            .await
            .unwrap();
        let (guid, without, with) = out;
        assert_eq!(
            (without, with),
            (0, 1),
            "{wrapper}: none without its item, one with"
        );
        let named = c
            .gets(std::time::Duration::from_millis(500), |msg| match msg {
                ServerOpcodeMessage::SMSG_SPELL_GO(go) if go.spell == summon => Some(go.cast_item),
                _ => None,
            })
            .await;
        assert_eq!(named, Some(guid), "{summon}'s go names its item");
    }
}
