//! Phase 3's second loader stage, the proc rules (Thistle Tea's
//! `world/loader/spell_proc_event_vmangos_test.exs` and the proc tests
//! beside it): what wvs reads of vmangos's `spell_proc_event`, and what
//! the rules do once an aura is on, over the real data when it is here.

use std::sync::Arc;
use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::ProcEvent;
use crate::game::harness::{self, Client};
use crate::game::map::procs::{self, *};
use crate::game::map::{Map, MapHandle, MapKey, auras, spells};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

fn realm() -> Arc<Realm> {
    harness::realm_with(harness::load_real(DB.as_path()))
}

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// A human of `class` at sixty in Elwynn, with their map.
async fn enter(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, Race::Human, class).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    let data = realm.data.clone();
    on(&map, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
    })
    .await;
    (c, g, map)
}

fn pe(d: &GameData, id: u32) -> ProcEvent {
    d.spells
        .proc_events
        .get(&id)
        .copied()
        .unwrap_or_else(|| panic!("spell_proc_event {id}"))
}

fn stacks(m: &Map, g: Guid, spell: u32) -> Option<u32> {
    auras::auras(m, g).and_then(|a| {
        a.holders
            .iter()
            .find(|h| h.spell == spell)
            .map(|h| h.stacks)
    })
}

/// `who` takes a melee blow from `from`, and a harmful spell.
fn struck(m: &mut Map, from: Guid, who: Guid, spell: Option<u32>) {
    let (actor_flags, target_flags) = match spell {
        None => (PROC_DONE_MELEE, PROC_TAKEN_MELEE | PROC_TAKEN_DAMAGE),
        Some(_) => (
            PROC_DONE_NEGATIVE_SPELL,
            PROC_TAKEN_NEGATIVE_SPELL | PROC_TAKEN_DAMAGE,
        ),
    };
    procs::trigger(
        m,
        Event {
            actor: from,
            target: who,
            actor_flags,
            target_flags,
            ex: EX_NORMAL_HIT,
            spell,
            damage: 10,
            school: if spell.is_some() { 4 } else { 1 },
            ..Default::default()
        },
    );
}

const BRITTLE_ARMOR: u32 = 24_574;
const BRITTLE_ARMOR_BONUS: u32 = 24_575;
const MERCURIAL_SHIELD: u32 = 26_463;
const MERCURIAL_SHIELD_BONUS: u32 = 26_464;
const FIREBALL: u32 = 133;

/// Thistle Tea's depleting trinkets (`spell_proc_event_vmangos_test.exs`):
/// Brittle Armor and Mercurial Shield have no `spell_proc_event` row, so
/// their DBC proc flags stand, and their dummy scripts (29284, 29286) give
/// the bonus at full stacks; a blow takes one off Brittle Armor's, a
/// harmful spell one off Mercurial Shield's.
#[tokio::test]
#[ignore = "extended"]
async fn real_depleting_trinkets_start_full_and_lose_a_stack_a_hit() {
    if !have_data() {
        return;
    }
    let d = real().unwrap();
    for id in [24_661, BRITTLE_ARMOR, MERCURIAL_SHIELD] {
        assert!(!d.spells.proc_events.contains_key(&id), "{id}");
    }
    assert_eq!(
        d.spells.get(29_284).unwrap().script_name,
        "spell_brittle_armor_dummy"
    );
    assert_eq!(
        d.spells.get(29_286).unwrap().script_name,
        "spell_mercurial_shield_dummy"
    );
    let realm = realm();
    let (_w, warrior, map) = enter(&realm, "P3OTRIN1", "Brittla", Class::Warrior).await;
    let (_m, mage, _) = enter(&realm, "P3OTRIN2", "Merca", Class::Mage).await;
    let out = on(&map, move |m| {
        spells::cast_triggered(m, warrior, BRITTLE_ARMOR, Some(warrior));
        spells::cast_triggered(m, warrior, MERCURIAL_SHIELD, Some(warrior));
        let full = (
            stacks(m, warrior, BRITTLE_ARMOR_BONUS),
            stacks(m, warrior, MERCURIAL_SHIELD_BONUS),
        );
        struck(m, mage, warrior, None);
        let blow = (
            stacks(m, warrior, BRITTLE_ARMOR_BONUS),
            stacks(m, warrior, MERCURIAL_SHIELD_BONUS),
        );
        struck(m, mage, warrior, Some(FIREBALL));
        let spell = (
            stacks(m, warrior, BRITTLE_ARMOR_BONUS),
            stacks(m, warrior, MERCURIAL_SHIELD_BONUS),
        );
        (full, blow, spell)
    })
    .await;
    assert_eq!(
        out,
        (
            (Some(10), Some(10)),
            (Some(9), Some(10)),
            (Some(9), Some(9))
        ),
        "full; a blow off the armor; a spell off the shield"
    );
}

/// Thistle Tea's `spell_proc_event_vmangos_test.exs`, the rules vmangos's
/// supported build gives: ranged equipment procs' rates a minute; the
/// healing sets' and class scripts' families, masks, periodic and outcome
/// rules; Shadowguard's shared cooldown and its strikes' lack of threat;
/// Blessed Recovery's and Deep Wounds' critical-only rule, which their
/// later ranks take from their first; Shield Block's charges spent on
/// blocks; the cast-ending trinkets, Elemental Focus and the Blue Dragon;
/// Heart of Wyrmthalak's physical blows; the sure first reflection;
/// Combustion's fire; and a 64-bit family mask kept whole.
#[test]
#[ignore = "extended"]
fn the_real_proc_rules_load_as_thistle_tea_reads_them() {
    let Some(d) = real() else {
        return;
    };
    for (id, rate) in [(23_578, 2.0), (26_480, 10.0)] {
        let p = pe(d, id);
        assert_eq!((p.ppm, p.chance), (rate, 0.0), "{id}");
    }
    // The healing sets: Holy Power from Holy Light and Flash of Light
    // only (and Holy Shock's), the priest set's from its two masks.
    let holy = pe(d, 28_789);
    assert_eq!(
        (holy.family, holy.family_mask[0], holy.flags, holy.ex),
        (10, 0xC000_6000, 0, 0)
    );
    let priest = pe(d, 28_823);
    assert_eq!(
        (
            priest.family,
            priest.family_mask[0],
            priest.flags,
            priest.ex
        ),
        (0, 0xC0, 0, 0)
    );
    // Shadowguard: one cooldown, from an absorb too, its later ranks
    // taking the first's; its strikes cause no threat.
    let shadowguard = pe(d, 18_137);
    assert_eq!((shadowguard.ex, shadowguard.cooldown), (0x403, 3_500));
    for (id, strike) in [
        (18_137, 28_377),
        (19_308, 28_378),
        (19_309, 28_379),
        (19_310, 28_380),
        (19_311, 28_381),
        (19_312, 28_382),
    ] {
        assert_eq!(d.spells.first_rank(id), 18_137, "{id}");
        assert_eq!(pe(d, id).cooldown, 3_500, "{id}");
        assert!(outcome_allowed(Some(&pe(d, id)), EX_ABSORB), "{id}");
        assert_eq!(d.spells.threat.get(&strike), Some(&(0, 0.0)), "{strike}");
    }
    // Critical only, on every rank: the rows are on the first alone.
    for (first, later) in [(27_811, [27_815, 27_816]), (12_834, [12_849, 12_867])] {
        assert_eq!(pe(d, first).ex, EX_CRITICAL_HIT);
        for id in later {
            let p = pe(d, id);
            assert_eq!(p.ex, EX_CRITICAL_HIT, "{id}");
            assert!(outcome_allowed(Some(&p), EX_CRITICAL_HIT), "{id}");
            assert!(!outcome_allowed(Some(&p), EX_NORMAL_HIT), "{id}");
        }
    }
    // Blessed Recovery's heals are flat, no bonus from healing power.
    for id in [27_813, 27_817, 27_818, 26_470] {
        assert_eq!(
            d.spells.get(id).unwrap().effects[0].bonus_coefficient,
            0.0,
            "{id}"
        );
    }
    // The class scripts' rules.
    for (id, family, mask, flags, ex, chance) in [
        (11_185, 3, 0x80, 0x50000, 0, 0.0),
        (18_094, 5, 0xA, 0x40000, 0, 0.0),
        (19_572, 9, 0x80_0000, 0x40000, 0x40000, 0.0),
        (23_401, 6, 0, 0x4000, 0, 100.0),
        (28_716, 7, 0x10, 0x48000, 0x40000, 50.0),
        (28_744, 7, 0x40, 0x44000, 0x40003, 0.0),
    ] {
        let p = pe(d, id);
        assert_eq!(
            (p.family, p.family_mask[0], p.flags, p.ex, p.chance),
            (family, mask, flags, ex, chance),
            "{id}"
        );
    }
    // Shield Block: its charges go on blocks alone.
    let block = pe(d, 2565);
    assert_eq!(block.ex, EX_BLOCK);
    assert!(outcome_allowed(Some(&block), EX_BLOCK));
    assert!(!outcome_allowed(Some(&block), EX_NORMAL_HIT));
    // At a cast's end, not a hit: the trinkets, Elemental Focus (of
    // fire, frost and nature, a shaman's harmful spells) and the Blue
    // Dragon.
    for id in [24_659, 28_200, 23_688] {
        let p = pe(d, id);
        assert_eq!(p.ex, EX_CAST_END, "{id}");
        assert!(
            outcome_allowed(Some(&p), EX_CAST_END | EX_NORMAL_HIT),
            "{id}"
        );
        assert!(!outcome_allowed(Some(&p), EX_NORMAL_HIT), "{id}");
    }
    let focus = pe(d, 16_164);
    assert_eq!(
        (focus.ex, focus.school_mask, focus.family, focus.flags),
        (EX_CAST_END, 28, 11, PROC_DONE_NEGATIVE_SPELL)
    );
    let wyrmthalak = pe(d, 27_656);
    assert_eq!((wyrmthalak.school_mask, wyrmthalak.ppm), (1, 1.0));
    assert_eq!(pe(d, 30_003).ex, EX_REFLECT);
    assert_eq!(pe(d, 11_129).school_mask, 4);
    // vmangos's 64-bit mask: Thistle Tea's two words, 96 and 128.
    let split = pe(d, 18_096);
    assert_eq!(split.family_mask[0], 96 | (128 << 32));
    assert_eq!((split.flags, split.chance), (0x50400, 0.0));
    // Its second rank has no row of its own, and takes the first's.
    assert_eq!(pe(d, 18_073).family_mask, split.family_mask);
}
