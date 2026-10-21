//! Round 12e: which buffs stand together, as mangos-classic's
//! `SpellStacker::IsStackableAuraEffect` has them: two paladins'
//! different auras, each on the whole party; Mark of the Wild and Gift of
//! the Wild, one spell of two ranks, never both.

use std::time::{Duration, Instant};

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::harness::{self, Client};
use crate::game::map::auras::{AURA_DAMAGE_SHIELD, AURA_MOD_RESISTANCE};
use crate::game::map::spells::Targets;

const DEVOTION: u32 = 997_701;
const RETRIBUTION: u32 = 997_702;
const R30: u32 = 990_701;
/// Mark of the Wild and Gift of the Wild: a druid's, of the same family
/// flags, +285 armor each; a weaker mark of those flags, +100.
const MARK: u32 = 997_711;
const GIFT: u32 = 997_712;
const WEAK_MARK: u32 = 997_713;

fn wild(d: &mut GameData) {
    d.spells.radii.insert(R30, 30.0);
    let buff = |id: u32, target: u32, amount: i32| {
        let mut s = bare(id, 3);
        s.family_name = 7;
        s.family_flags = 0x4_0000;
        s.duration_index = LASTS_LONG;
        s.range_index = 1;
        effect(
            &mut s,
            0,
            SpellEffect {
                radius_index: R30,
                ..super::tests_p3_cast_kit::aura(AURA_MOD_RESISTANCE, 1, amount, target)
            },
        );
        s
    };
    insert(
        d,
        [
            buff(MARK, 21, 285),
            buff(GIFT, 37, 285),
            buff(WEAK_MARK, 21, 100),
        ],
    );
}

fn auras_of_paladins(d: &mut GameData) {
    d.spells.radii.insert(R30, 30.0);
    let party_aura = |id: u32, flags: u64, aura: u32, misc: i32| {
        let mut s = bare(id, 1);
        s.family_name = 10;
        s.family_flags = flags;
        s.duration_index = LASTS_LONG;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 35,
                radius_index: R30,
                ..super::tests_p3_cast_kit::aura(aura, misc, 100, 1)
            },
        );
        s
    };
    insert(
        d,
        [
            party_aura(DEVOTION, 0x40, AURA_MOD_RESISTANCE, 1),
            party_aura(RETRIBUTION, 0x8, AURA_DAMAGE_SHIELD, 2),
        ],
    );
}

pub(super) async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    member
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
            _ => None,
        })
        .await;
}

/// Two paladins in a party, one with Devotion Aura and one with
/// Retribution Aura: each of them has both, and keeps both over the
/// auras' refreshes; one paladin's switch to the other aura takes his
/// first off. Before, each kept only one paladin's aura, the other's
/// taking its place at every refresh.
#[tokio::test]
async fn two_paladins_auras_stand_together() {
    let realm = fast_realm(auras_of_paladins);
    let (mut a, p, map) = human(&realm, "P12EP", "Devout", Class::Paladin).await;
    let (mut b, q, _) = human(&realm, "P12EQ", "Vengeful", Class::Paladin).await;
    group(&mut a, &mut b, "Vengeful").await;
    map.query(move |m| {
        kit::ready(m, p, 60, &[DEVOTION, RETRIBUTION]);
        kit::ready(m, q, 60, &[DEVOTION, RETRIBUTION]);
        cast_fresh(m, p, DEVOTION, Targets::default());
        cast_fresh(m, q, RETRIBUTION, Targets::default());
    })
    .await
    .unwrap();
    let both = move |m: &mut crate::game::map::Map| {
        [p, q].map(|g| kit::has(m, g, DEVOTION) && kit::has(m, g, RETRIBUTION))
    };
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let now = map.query(both).await.unwrap();
        if now == [true, true] {
            break;
        }
        assert!(
            Instant::now() < until,
            "both auras on both paladins: {now:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Over the refreshes that follow (one a second), both stay.
    let start = map.query(|m| m.stats.updates).await.unwrap();
    loop {
        assert_eq!(map.query(both).await.unwrap(), [true, true], "both stay");
        if map.query(|m| m.stats.updates).await.unwrap() > start + 30 {
            break;
        }
        harness::an_update(&map).await;
    }
    // The Devout paladin switches to Retribution: his Devotion goes from
    // both of them, and each keeps a Retribution (his own the other's
    // doesn't take off him, as mangos's `AreaAura::Update` leaves another
    // caster's of the same spell be).
    map.query(move |m| {
        cast_fresh(m, p, RETRIBUTION, Targets::default());
    })
    .await
    .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let now = map
            .query(move |m| {
                [p, q].map(|g| !kit::has(m, g, DEVOTION) && kit::has(m, g, RETRIBUTION))
            })
            .await
            .unwrap();
        if now == [true, true] {
            break;
        }
        assert!(Instant::now() < until, "Devotion gone: {now:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A druid's Mark of the Wild on a warrior, then the Gift of the Wild on
/// his party: the Gift takes the Mark's place (one spell, as mangos's
/// "multirank" of two helpful spells of one family's flags), so his armor
/// counts 285 once; the Mark again takes the Gift's. A weaker mark of the
/// same flags is refused while the Mark is on, and the Mark stays. Before,
/// the Mark and the Gift both stayed, and the armor counted twice.
#[tokio::test]
async fn mark_and_gift_of_the_wild_dont_stand_together() {
    let realm = fast_realm(wild);
    let (mut a, d, map) = human(&realm, "P12EM", "Wildling", Class::Warrior).await;
    let (mut b, w, _) = human(&realm, "P12EN", "Marked", Class::Warrior).await;
    group(&mut a, &mut b, "Marked").await;
    let out = map
        .query(move |m| {
            kit::ready(m, d, 60, &[MARK, GIFT, WEAK_MARK]);
            let armor = |m: &crate::game::map::Map| {
                m.unit_fields(w)
                    .map(|f| f.get_u32(crate::game::fields::UNIT_FIELD_RESISTANCES))
                    .unwrap_or(0)
            };
            let bare_armor = armor(m);
            cast_fresh(m, d, MARK, at(w));
            let marked = (kit::has(m, w, MARK), armor(m));
            cast_fresh(m, d, GIFT, at(w));
            let gifted = (kit::has(m, w, MARK), kit::has(m, w, GIFT), armor(m));
            cast_fresh(m, d, MARK, at(w));
            let again = (kit::has(m, w, MARK), kit::has(m, w, GIFT));
            kit::fresh(m, d);
            let s = m.realm.data.spells.get(WEAK_MARK).cloned().unwrap();
            let mods = crate::game::map::spells::mods::for_spell(m, d, &s);
            let weak =
                crate::game::map::spells::checks::check(m, d, &s, &at(w), None, false, &mods);
            (
                bare_armor,
                marked,
                gifted,
                again,
                weak,
                kit::has(m, w, MARK),
            )
        })
        .await
        .unwrap();
    let (bare_armor, marked, gifted, again, weak, kept) = out;
    assert_eq!(marked, (true, bare_armor + 285), "the Mark: 285 armor");
    assert_eq!(
        gifted,
        (false, true, bare_armor + 285),
        "the Gift in the Mark's place, its armor counted once"
    );
    assert_eq!(again, (true, false), "and the Mark again in the Gift's");
    assert_eq!(
        weak,
        Some(crate::game::map::spells::fail::AURA_BOUNCED),
        "a weaker mark is refused"
    );
    assert!(kept, "and the Mark stays");
}

/// Inspiration (a priest's) and Ancestral Fortitude (a shaman's): +25%
/// armor each, both magic.
const INSPIRATION: u32 = 997_721;
const FORTITUDE: u32 = 997_722;

fn armor_buffs(d: &mut GameData) {
    let buff = |id: u32, family: u32| {
        let mut s = bare(id, 0);
        s.family_name = family;
        s.dispel = 1;
        s.duration_index = LASTS_LONG;
        effect(
            &mut s,
            0,
            super::tests_p3_cast_kit::aura(
                crate::game::map::auras::AURA_MOD_RESISTANCE_PCT,
                1,
                25,
                1,
            ),
        );
        s
    };
    insert(d, [buff(INSPIRATION, 6), buff(FORTITUDE, 11)]);
}

/// Inspiration on a warrior, then Ancestral Fortitude: the second takes
/// the first's place, as mangos's stacker has two helpful armor
/// percentages of one dispel type. Before, both stayed and multiplied.
#[tokio::test]
async fn inspiration_and_ancestral_fortitude_dont_stand_together() {
    let realm = fast_realm(armor_buffs);
    let (_c, w, map) = human(&realm, "P12EO", "Tanking", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::apply(m, w, w, INSPIRATION);
            kit::apply(m, w, w, FORTITUDE);
            (kit::has(m, w, INSPIRATION), kit::has(m, w, FORTITUDE))
        })
        .await
        .unwrap();
    assert_eq!(out, (false, true), "the second in the first's place");
}
