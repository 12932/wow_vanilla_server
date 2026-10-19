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
use crate::game::tests_p3_combat::START;
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

/// A place's aura (in Elwynn or Westfall, wanting another aura first),
/// that other aura, and one a quest taken brings until it is done.
const PLACE: u32 = 96_430;
const NEEDED: u32 = 96_431;
const QUESTED: u32 = 96_432;
/// Elwynn's own, wanting nothing.
const HOME: u32 = 96_433;
const QUEST: u32 = 64_500;
const GIVER: u32 = 64_501;

fn areas_world() -> GameData {
    use crate::game::data::scripts::SpellArea;
    let mut d = crate::game::tests_p3_combat::data();
    for id in [PLACE, NEEDED, QUESTED, HOME] {
        let s = crate::game::tests_p3_entity::spell(
            id,
            &[SpellEffect {
                effect: 6,
                aura: crate::game::map::auras::AURA_DUMMY,
                target_a: 1,
                ..Default::default()
            }],
        );
        d.spells.spells.insert(id, s);
    }
    for area in [12, 40] {
        d.scripts.spell_areas.push(SpellArea {
            spell: PLACE,
            area,
            aura_spell: NEEDED as i32,
            gender: 2,
            autocast: true,
            ..Default::default()
        });
    }
    d.scripts.spell_areas.push(SpellArea {
        spell: HOME,
        area: 12,
        gender: 2,
        autocast: true,
        ..Default::default()
    });
    d.scripts.spell_areas.push(SpellArea {
        spell: QUESTED,
        quest_start: QUEST,
        quest_start_active: true,
        quest_end: QUEST,
        gender: 2,
        autocast: true,
        ..Default::default()
    });
    d.npc.quests.insert(
        QUEST,
        crate::game::data::npc::Quest {
            id: QUEST,
            title: "Marked".into(),
            ..Default::default()
        },
    );
    let mut giver = crate::game::tests_p3_combat::template(GIVER, 7);
    giver.faction = crate::game::tests_p3_combat::HUMANS;
    giver.npc_flags = crate::game::map::npc::NPC_FLAG_QUESTGIVER;
    d.creature_templates.insert(GIVER, giver);
    d.npc.creature_starts.insert(GIVER, vec![QUEST]);
    d
}

/// Thistle Tea's `SpellAreas.reconcile` "applies once, preserves
/// alternatives, removes on exit and restores on reentry", "removes
/// dependent auras when their prerequisite disappears", "reevaluates
/// death, resurrection" and "quest acceptance grants an aura": a place's
/// aura comes in its zone with the aura it needs, stays in its other
/// zone, goes outside both and comes back on returning; goes the moment
/// the aura it needs goes, and comes back with it (mangos's aura handler
/// judges the rules that ask for it); goes at death and is back at
/// resurrection (mangos's `ResurrectPlayer` updates the zone "to reapply
/// auras"); and a quest's comes as it is accepted and goes as it is
/// abandoned.
#[tokio::test]
async fn a_places_aura_follows_the_place_its_prerequisite_life_and_quest() {
    let realm = harness::realm_with(areas_world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = on(&map, move |m| {
        use crate::game::map::spell_areas::reconcile;
        let has = |m: &Map, s| m.players[&g].auras.has_spell(s);
        let mut seen = Vec::new();
        let at = |m: &mut Map, zone: u32| {
            let p = m.players.get_mut(&g).unwrap();
            p.zone = zone;
            p.area = zone;
            reconcile(m, g);
        };
        at(m, 12);
        seen.push(("Elwynn, without", has(m, PLACE)));
        let needed = m.realm.data.spells.get(NEEDED).unwrap().clone();
        crate::game::map::auras::apply_spell(m, g, g, &needed, &[(0, 0)], Default::default())
            .unwrap();
        seen.push(("with it", has(m, PLACE)));
        at(m, 40);
        seen.push(("Westfall", has(m, PLACE)));
        at(m, 1);
        seen.push(("Dun Morogh", has(m, PLACE)));
        at(m, 12);
        seen.push(("Elwynn again", has(m, PLACE)));
        crate::game::map::auras::remove_spell(m, g, NEEDED, None);
        seen.push(("its need gone", has(m, PLACE)));
        crate::game::map::auras::apply_spell(m, g, g, &needed, &[(0, 0)], Default::default())
            .unwrap();
        seen.push(("its need back", has(m, PLACE)));
        seen.push(("Elwynn's own", has(m, HOME)));
        crate::game::map::death::kill(m, g);
        seen.push(("dead", has(m, HOME)));
        crate::game::map::death::resurrect(m, g, 1.0);
        seen.push(("risen", has(m, HOME)));
        let giver = crate::game::tests_p3_combat::spawn(m, g, GIVER, 20, 2.0);
        seen.push(("no quest", has(m, QUESTED)));
        crate::game::map::quests::accept(m, g, giver, QUEST);
        seen.push(("quest taken", has(m, QUESTED)));
        let slot = m.players[&g].journal.slot_of(QUEST).unwrap() as u8;
        crate::game::map::quests::abandon(m, g, slot);
        seen.push(("quest abandoned", has(m, QUESTED)));
        seen
    })
    .await;
    assert_eq!(
        out,
        [
            ("Elwynn, without", false),
            ("with it", true),
            ("Westfall", true),
            ("Dun Morogh", false),
            ("Elwynn again", true),
            ("its need gone", false),
            ("its need back", true),
            ("Elwynn's own", true),
            ("dead", false),
            ("risen", true),
            ("no quest", false),
            ("quest taken", true),
            ("quest abandoned", false),
        ]
    );
}
