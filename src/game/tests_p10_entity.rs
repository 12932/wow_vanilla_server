//! Round 10's entity tests (phase 2, the rows wvs did partly or
//! differently), ported from Thistle Tea's `test/game/world/entity/`
//! (docs/parity/matrix.csv): a wild summon whose loot is its creator's,
//! and where a summon stands.

use wow_world_messages::Guid;
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
