//! Round 11j's tests in the real world (the extended suite): a trainer's
//! profession and its ranks (Thistle Tea's `spells_dbc_test`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, skills};
use crate::game::tests_p3_entity::on;

const ENCHANTING: u32 = 333;
const APPRENTICE: u32 = 7411;
const JOURNEYMAN: u32 = 7412;

fn skill(m: &Map, g: Guid, s: u32) -> Option<(u32, u32)> {
    let p = &m.players[&g];
    skills::value(p, s).zip(skills::max_value(p, s))
}

/// A trainer of `learned`'s (the trainer's spell that teaches it) who
/// will deal with `g`, spawned beside them: its guid and the spell the
/// client buys.
fn trainer_of(m: &mut Map, g: Guid, learned: u32) -> Option<(Guid, u32)> {
    use crate::game::map::npc::NPC_FLAG_TRAINER;
    let data = m.realm.data.clone();
    let mut candidates: Vec<(crate::game::data::CreatureTemplate, u32)> = data
        .creature_templates
        .values()
        .filter(|t| t.npc_flags & NPC_FLAG_TRAINER != 0)
        .filter_map(|t| {
            data.npc
                .trainer_spells(t.entry, t.trainer_id)
                .into_iter()
                .find(|s| crate::game::map::spells::learns(&data, s.spell) == learned)
                .map(|s| (t.clone(), s.spell))
        })
        .collect();
    candidates.sort_by_key(|(t, _)| t.entry);
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + 2.0 * o.cos(),
        y: pos.y + 2.0 * o.sin(),
        z: pos.z,
    };
    for (t, spell) in candidates {
        let counter = m.next_creature_counter;
        m.next_creature_counter += 1;
        let c = crate::game::map::creature::Creature::new(
            crate::game::guid::unit(t.entry, counter | 0x80_0000),
            &t,
            t.level_min.max(1),
            at,
            o + std::f32::consts::PI,
        );
        let npc = m.add_creature(c.with_base());
        if crate::game::map::npc::npc_entry(m, g, npc, NPC_FLAG_TRAINER).is_some() {
            return Some((npc, spell));
        }
        m.remove_creature(npc);
    }
    None
}

async fn buy(c: &mut Client, map: &MapHandle, learned: u32) {
    let g = c.guid;
    let (npc, spell) = on(map, move |m| trainer_of(m, g, learned))
        .await
        .expect("a trainer of it");
    c.send(CMSG_TRAINER_BUY_SPELL {
        guid: npc,
        id: spell,
    })
    .await;
    c.wait_for("SMSG_TRAINER_BUY_SUCCEEDED", |m| match m {
        ServerOpcodeMessage::SMSG_TRAINER_BUY_SUCCEEDED(s) if s.id == spell => Some(()),
        ServerOpcodeMessage::SMSG_TRAINER_BUY_FAILED(f) if f.id == spell => {
            panic!("refused: {:?}", f.error)
        }
        _ => None,
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(c).await;
}

/// Thistle Tea's "teaches profession starter spells and preserves skill on
/// rank upgrades": Apprentice Enchanter bought of a trainer teaches
/// Enchanting at 1 of 75, its starting recipes (Enchant Bracer - Minor
/// Health 7418, Minor Health 7421... 7428 and Runed Copper Rod 13262 among
/// them) and not one a trainer sells (7420); Journeyman bought at 50 keeps
/// the 50 and lifts the cap to 150, and teaches the rod again to no one.
#[tokio::test]
#[ignore = "extended"]
async fn an_enchanter_trained_has_her_recipes_and_keeps_her_skill_at_the_next_rank() {
    let realm = harness::realm_with(harness::load_real(&harness::real_data("db")));
    let mut c = Client::connect(&realm, "A");
    let g = c.enter("Enchanter", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        p.set_money(1_000_000);
    })
    .await;
    buy(&mut c, &map, APPRENTICE).await;
    let known = move |m: &mut Map, s: u32| m.players[&g].spells.contains(&s);
    let (apprentice, starters, sold) = on(&map, move |m| {
        (
            skill(m, g, ENCHANTING),
            [7418, 7421, 7428, 13262].map(|s| known(m, s)),
            known(m, 7420),
        )
    })
    .await;
    assert_eq!(apprentice, Some((1, 75)), "Apprentice Enchanting");
    assert_eq!(starters, [true; 4], "its starting recipes");
    assert!(!sold, "not one its trainer sells");
    on(&map, move |m| {
        skills::set_value(m.players.get_mut(&g).unwrap(), ENCHANTING, 50);
    })
    .await;
    c.clear();
    buy(&mut c, &map, JOURNEYMAN).await;
    let journeyman = on(&map, move |m| skill(m, g, ENCHANTING)).await;
    assert_eq!(
        journeyman,
        Some((50, 150)),
        "the skill kept, the cap raised"
    );
    let rod_again = c
        .gets(std::time::Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_LEARNED_SPELL(l) if l.id == 13262 => Some(()),
            _ => None,
        })
        .await;
    assert!(rod_again.is_none(), "nothing learnt twice");
}

/// Thistle Tea's "corpse destinations summon at the body and retain
/// corpse script delivery", of the real 12699 (Summon Screecher Spirit:
/// `ALLOW_DEAD_TARGET`, a wild summon at the place its script names, 46,
/// and a dummy): cast at a dead screecher its script names, the spirit
/// (8612) comes where the body lies, and the body goes within a second
/// (`class_scripts`' dummy of 12699 on the dead).
#[tokio::test]
#[ignore = "extended"]
async fn a_screecher_spirit_rises_at_the_body_and_the_body_goes() {
    const SPIRIT_SPELL: u32 = 12699;
    const SPIRIT: u32 = 8612;
    let realm = harness::realm_with(harness::load_real(&harness::real_data("db")));
    let mut c = Client::connect(&realm, "A");
    let g = c.enter("Shaman", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let out = on(&map, move |m| {
        let data = m.realm.data.clone();
        let s = data.spells.get(SPIRIT_SPELL).cloned().expect("12699");
        assert_ne!(s.attributes[2] & 1, 0, "ALLOW_DEAD_TARGET");
        assert_eq!(
            (
                s.effects[0].effect,
                s.effects[0].target_b,
                s.effects[1].effect
            ),
            (41, 46, 3),
            "a wild summon at its script's place, then a dummy"
        );
        let entry = data
            .spells
            .script_targets
            .get(&SPIRIT_SPELL)
            .and_then(|v| v.iter().find(|(k, _)| *k == 2).map(|(_, e)| *e))
            .expect("a dead screecher named");
        let t = data
            .creature_template(entry)
            .cloned()
            .expect("the screecher");
        let p = &m.players[&g];
        let (pos, o) = (p.position(), p.movement.orientation);
        let at = Vector3d {
            x: pos.x + 4.0 * o.cos(),
            y: pos.y + 4.0 * o.sin(),
            z: pos.z,
        };
        let counter = m.next_creature_counter;
        m.next_creature_counter += 1;
        let body = m.add_creature(
            crate::game::map::creature::Creature::new(
                crate::game::guid::unit(entry, counter | 0x80_0000),
                &t,
                t.level_min.max(1),
                at,
                o,
            )
            .with_base(),
        );
        crate::game::map::combat::deal_damage(m, g, body, 1_000_000);
        let dead = m.creature(body).is_some_and(|c| !c.alive());
        let cast = crate::game::map::spells::cast_now(
            m,
            g,
            SPIRIT_SPELL,
            crate::game::map::spells::Targets {
                unit: Some(body),
                ..Default::default()
            },
        );
        let spirit = m
            .creatures
            .iter()
            .map(|(_, c)| c)
            .find(|c| c.entry == SPIRIT)
            .map(|c| c.position);
        let goes = m.creature(body).map(|c| match c.life {
            crate::game::map::creature::Life::Dead { corpse_until, .. } => {
                corpse_until <= m.now + std::time::Duration::from_secs(1)
            }
            _ => false,
        });
        (dead, cast, spirit, at, goes)
    })
    .await;
    let (dead, cast, spirit, at, goes) = out;
    assert!(dead && cast, "cast at the body");
    let spirit = spirit.expect("the spirit");
    assert!(
        (spirit.x - at.x).abs() < 1.0 && (spirit.y - at.y).abs() < 1.0,
        "{spirit:?} {at:?}"
    );
    assert_eq!(goes, Some(true), "the body goes within a second");
}
