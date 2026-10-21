//! Round 10's entity tests (phase 2, the rows wvs did partly or
//! differently), ported from Thistle Tea's `test/game/world/entity/`
//! (docs/parity/matrix.csv): a wild summon whose loot is its creator's,
//! and where a summon stands.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::Vector3d;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::{Map, pets};
use crate::game::tests_pets::{data, effect, enter, learn, spell, template};

/// An engineer's dummy, made up: its loot its creator's, no experience
/// (vmangos's static flags `CREATOR_LOOT` and `NO_XP`).
const DUMMY: u32 = 990_211;
/// A plain wild summon.
const PLAIN: u32 = 990_212;
const SUMMON_DUMMY: u32 = 990_213;
const SUMMON_PLAIN: u32 = 990_214;

fn wild_world() -> GameData {
    let mut d = data();
    let none = SpellEffect::default();
    for (id, entry, flags) in [
        (SUMMON_DUMMY, DUMMY, pets::CREATOR_LOOT | 0x2),
        (SUMMON_PLAIN, PLAIN, 0),
    ] {
        let mut t = template(entry, "Wild Summon", 14, 1);
        t.static_flags1 = flags;
        d.creature_templates.insert(entry, t);
        let mut e = effect(pets::EFFECT_SUMMON_WILD, 0, 1);
        e.misc_value = entry as i32;
        d.spells
            .spells
            .insert(id, spell(id, "Summon", [e, none, none]));
    }
    d
}

fn summoned(m: &Map, entry: u32) -> Option<Guid> {
    m.creatures
        .iter()
        .find(|(_, c)| c.entry == entry && c.alive())
        .map(|(_, c)| c.guid)
}

/// A player's wild summon whose loot is its creator's (an engineer's
/// Target Dummy, vmangos's `CREATOR_LOOT`) is made by them and tapped for
/// them as it comes, so a stranger's blow takes nothing; one of no
/// experience gives its creator none; a plain one is no one's until
/// struck, and gives its killer experience (Thistle Tea's "creator loot
/// claims salvage and honors no experience"; mangos's `AIM_Initialize`,
/// `SetLootRecipient(GetCreator())`). wvs tapped none for its creator.
#[tokio::test]
async fn a_creators_wild_summon_is_tapped_for_them_and_gives_no_experience() {
    let realm = crate::game::harness::realm_with(wild_world());
    let (_a, me, map) = enter(&realm, "A", "Engineer", Race::Human, Class::Warrior).await;
    let (_b, stranger, _) = enter(&realm, "B", "Stranger", Race::Human, Class::Warrior).await;
    learn(&map, me, &[SUMMON_DUMMY, SUMMON_PLAIN]);
    let out = map
        .query(move |m| {
            crate::game::map::spells::cast(m, me, SUMMON_DUMMY, Default::default(), None);
            crate::game::map::spells::cast(m, me, SUMMON_PLAIN, Default::default(), None);
            let dummy = summoned(m, DUMMY)?;
            let plain = summoned(m, PLAIN)?;
            let made = |g: Guid| {
                let c = m.creature(g).unwrap();
                (
                    c.fields.get_guid(UNIT_FIELD_CREATEDBY),
                    c.fields.get_guid(UNIT_FIELD_SUMMONEDBY),
                    c.tapped_by,
                )
            };
            let (dummy_made, plain_made) = (made(dummy), made(plain));
            // A stranger's blow takes neither the dummy's tap nor its kill.
            crate::game::map::combat::deal_damage(m, stranger, dummy, 1);
            let still = m.creature(dummy).unwrap().tapped_by;
            let xp = |m: &Map| m.players[&me].fields.get_u32(PLAYER_XP);
            let before = xp(m);
            crate::game::map::combat::deal_damage(m, me, dummy, 100_000);
            let from_dummy = xp(m) - before;
            crate::game::map::combat::deal_damage(m, me, plain, 100_000);
            let from_plain = xp(m) - before - from_dummy;
            Some((dummy_made, plain_made, still, from_dummy, from_plain))
        })
        .await
        .unwrap()
        .expect("both summoned");
    let (dummy_made, plain_made, still, from_dummy, from_plain) = out;
    assert_eq!(
        dummy_made,
        (me, Guid::zero(), Some(me)),
        "made by its caster, tapped for them"
    );
    assert_eq!(
        plain_made,
        (Guid::zero(), Guid::zero(), None),
        "no one's until struck"
    );
    assert_eq!(still, Some(me), "a stranger's blow takes no tap");
    assert_eq!(from_dummy, 0, "no experience from a no-experience summon");
    assert!(from_plain > 0, "a plain one gives its killer experience");
}

/// The entry a script turns a creature into.
const CHANGED: u32 = 990_217;

/// Script command 27 (`update_entry`) makes a creature another entry, the
/// same unit: its watcher is told the new entry, looks, level and npc
/// flags in its values, under the same GUID, and the new entry's query
/// answers its name (Thistle Tea's "publishes current identity and client
/// fields while retaining the process"). The level is the new entry's,
/// rolled in its range whatever the old one was (mangos's `SelectLevel`
/// at `UpdateEntry`): wvs kept an old level that fell in the new range.
#[tokio::test]
async fn an_update_entry_tells_its_watcher_the_new_entry_and_level() {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    let mut d = data();
    let mut changed = template(CHANGED, "Changed", 14, 1);
    changed.display_ids = [102, 0, 0, 0];
    changed.level_min = 15;
    changed.level_max = 15;
    changed.npc_flags = 3;
    d.creature_templates.insert(CHANGED, changed);
    // One of 10 to 20, at 12: the old level in the new range.
    let mut ranged = template(CHANGED + 1, "Ranged", 14, 1);
    ranged.level_min = 10;
    ranged.level_max = 20;
    d.creature_templates.insert(CHANGED + 1, ranged);
    let realm = crate::game::harness::realm_with(d);
    let (mut a, me, map) = enter(&realm, "A", "Watcher", Race::Human, Class::Warrior).await;
    let wolf =
        crate::game::tests_pets::place(&map, me, crate::game::tests_pets::WOLF, 0x70_0002, 5.0)
            .await;
    a.sees(wolf).await;
    map.query(move |m| {
        let step = crate::game::data::scripts::ScriptStep {
            command: 27,
            datalong: [CHANGED, 0, 0, 0],
            ..Default::default()
        };
        crate::game::map::scripts::execute(m, &step, wolf, wolf);
    })
    .await
    .unwrap();
    let told = a
        .wait_for("the new entry's values", |m| {
            crate::game::harness::values(m)
                .into_iter()
                .find(|(g, _)| *g == wolf)
                .map(|(_, mask)| crate::game::tests_bugs6::fields(&mask))
                .filter(|f| f.get(&OBJECT_FIELD_ENTRY) == Some(&CHANGED))
        })
        .await;
    assert_eq!(told.get(&UNIT_FIELD_DISPLAYID), Some(&102), "its looks");
    assert_eq!(told.get(&UNIT_FIELD_LEVEL), Some(&15), "its level");
    assert_eq!(told.get(&UNIT_NPC_FLAGS), Some(&3), "its npc flags");
    a.send(CMSG_CREATURE_QUERY {
        creature: CHANGED,
        guid: wolf,
    })
    .await;
    let name = a
        .wait_for("its query's answer", |m| match m {
            ServerOpcodeMessage::SMSG_CREATURE_QUERY_RESPONSE(r) if r.creature_entry == CHANGED => {
                Some(r.found.clone())
            }
            _ => None,
        })
        .await
        .expect("it is known")
        .name1;
    assert_eq!(name, "Changed");
    // From level 12 to an entry of 10 to 20: rolled anew, not kept.
    let levels = map
        .query(move |m| {
            let mut seen = std::collections::BTreeSet::new();
            for i in 0..40 {
                let c = m.creature_mut(wolf).unwrap();
                c.fields.set_u32(UNIT_FIELD_LEVEL, 12);
                let to = if i % 2 == 0 { CHANGED + 1 } else { CHANGED };
                crate::game::map::scripts::update_entry(m, wolf, to);
                if to == CHANGED + 1 {
                    seen.insert(m.creature(wolf).unwrap().fields.get_u32(UNIT_FIELD_LEVEL));
                }
            }
            seen
        })
        .await
        .unwrap();
    assert!(
        levels.len() > 1 && levels.iter().all(|l| (10..=20).contains(l)),
        "rolled in its range: {levels:?}"
    );
}

const AHEAD: u32 = 990_215;
const AT_PLACE: u32 = 990_216;

/// A summon put at a spot about its summoner stands short of what hides
/// the spot from them: a wild summon five yards ahead, a totem two yards
/// out and a pet at its master's side, each behind a wall a yard off,
/// come this side of it; one a spell puts at a place it names stands
/// there, wall or no (Thistle Tea's "clips implicit minion positions at
/// terrain while retaining explicit destinations"; mangos's
/// `GetClosePoint`, a spot in sight). wvs put each at its spot.
#[tokio::test]
async fn a_summons_spot_is_short_of_what_hides_it_and_a_named_place_is_kept() {
    let mut d = wild_world();
    d.spells.radii.insert(13, 5.0);
    let none = SpellEffect::default();
    let mut ahead = effect(pets::EFFECT_SUMMON_WILD, 0, 0);
    ahead.misc_value = PLAIN as i32;
    ahead.radius_index = 13;
    d.spells
        .spells
        .insert(AHEAD, spell(AHEAD, "Ahead", [ahead, none, none]));
    let mut placed = effect(pets::EFFECT_SUMMON_WILD, 0, 0);
    placed.misc_value = DUMMY as i32;
    d.spells
        .spells
        .insert(AT_PLACE, spell(AT_PLACE, "At Place", [placed, none, none]));
    let realm = crate::game::harness::realm_with(d);
    let (_w, me, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(
        &map,
        me,
        &[
            AHEAD,
            AT_PLACE,
            crate::game::tests_pets::SUMMON_IMP,
            crate::game::tests_pets::STRENGTH_OF_EARTH_TOTEM,
        ],
    );
    let out = map
        .query(move |m| {
            let p = m.players[&me].position();
            let o = m.players[&me].movement.orientation;
            // Along and across the warlock's facing.
            let at = |along: f32, across: f32| Vector3d {
                x: p.x + along * o.cos() - across * o.sin(),
                y: p.y + along * o.sin() + across * o.cos(),
                z: p.z,
            };
            let ahead_of = |q: Vector3d| (q.x - p.x) * o.cos() + (q.y - p.y) * o.sin();
            // A wall a yard ahead and one a yard behind, ten yards wide.
            m.spells.walls.push((at(1.0, -5.0), at(1.0, 5.0)));
            m.spells.walls.push((at(-1.0, -5.0), at(-1.0, 5.0)));
            for s in [
                AHEAD,
                crate::game::tests_pets::SUMMON_IMP,
                crate::game::tests_pets::STRENGTH_OF_EARTH_TOTEM,
            ] {
                crate::game::map::spells::cast(m, me, s, Default::default(), None);
            }
            // The place the cast names, eight yards ahead, beyond the wall.
            let beyond = crate::game::map::spells::Targets {
                dest: Some(at(8.0, 0.0)),
                ..Default::default()
            };
            crate::game::map::spells::cast(m, me, AT_PLACE, beyond, None);
            let pos = |g: Option<Guid>| g.and_then(|g| m.position_of(g)).map(ahead_of);
            let owned = &m.players[&me].owned;
            Some((
                pos(summoned(m, PLAIN)).expect("ahead"),
                pos(summoned(m, DUMMY)).expect("at the place"),
                pos(owned.totems.iter().flatten().next().copied()).expect("the totem"),
                pos(owned.pet).expect("the imp"),
            ))
        })
        .await
        .unwrap()
        .expect("each summoned");
    let (wild, named, totem, pet) = out;
    assert!(
        (0.0..1.0).contains(&wild),
        "the wild summon this side of the wall ahead: {wild}"
    );
    assert!((named - 8.0).abs() < 0.01, "the named place kept: {named}");
    assert!(
        (0.0..1.0).contains(&totem),
        "the totem this side of the wall: {totem}"
    );
    assert!(
        (-1.0..=0.0).contains(&pet),
        "the pet this side of the wall behind: {pet}"
    );
}
