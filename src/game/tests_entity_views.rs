//! Each viewer's own view of a unit (`map::views`): a Hunter's Mark tracks
//! its target for its hunter alone, and a creature's owner-only fields go
//! to its master, its damage and resistances to whoever's Empathy is on
//! it, as mangos-classic's `BuildValuesUpdate` and
//! `GetUpdateFieldFlagsForTarget` build each viewer's values.

use std::collections::BTreeMap;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::{Map, MapHandle, auras};
use crate::game::tests_bugs6::fields;
use crate::game::tests_pets::{BOAR, WOLF, data, effect, enter, place, spell};

const MARK: u32 = 97_300;
const LORE: u32 = 97_301;
/// Another stalking aura (Mind Vision's, beside Hunter's Mark's).
const VISION: u32 = 97_302;
const TRACK: u32 = auras::DYNFLAG_TRACK_UNIT;

fn world() -> GameData {
    let mut d = data();
    let none = SpellEffect::default();
    for (id, name, aura) in [
        (MARK, "Mark", auras::AURA_MOD_STALKED),
        (VISION, "Vision", auras::AURA_MOD_STALKED),
        (LORE, "Lore", auras::AURA_EMPATHY),
    ] {
        let mut e = effect(6, 0, 6);
        e.aura = aura;
        let mut s = spell(id, name, [e, none, none]);
        s.duration_index = 4;
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn put(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let s = m.realm.data.spells.spells[&id].clone();
    auras::apply_spell(m, caster, target, &s, &[(0, 0)], Default::default()).expect("put on");
}

/// The fields of the next values block `c` is told of `of` that `want`
/// takes.
async fn values_of(
    c: &mut Client,
    of: Guid,
    what: &str,
    want: impl Fn(&BTreeMap<u16, u32>) -> bool,
) -> BTreeMap<u16, u32> {
    c.wait_for(what, |m| {
        harness::values(m)
            .into_iter()
            .filter(|(g, _)| *g == of)
            .map(|(_, mask)| fields(&mask))
            .find(|f| want(f))
    })
    .await
}

/// The dynamic flags in the next values block `c` is told of `of` with
/// them.
async fn flags_of(c: &mut Client, of: Guid) -> u32 {
    values_of(c, of, "its dynamic flags", |f| {
        f.contains_key(&UNIT_DYNAMIC_FLAGS)
    })
    .await[&UNIT_DYNAMIC_FLAGS]
}

/// The fields of `c`'s create of `of`.
async fn create_of(c: &mut Client, of: Guid) -> BTreeMap<u16, u32> {
    c.wait_for("its create", |m| {
        let objects = match m {
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
            ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
            _ => return None,
        };
        objects.iter().find_map(|o| match &o.update_type {
            Object_UpdateType::CreateObject { guid3, mask2, .. }
            | Object_UpdateType::CreateObject2 { guid3, mask2, .. }
                if *guid3 == of =>
            {
                Some(fields(mask2))
            }
            _ => None,
        })
    })
    .await
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// A Hunter's Mark tracks its target (`UNIT_DYNFLAG_TRACK_UNIT`) for its
/// hunter alone, as mangos's `BuildValuesUpdate` takes the flag off for
/// all but the caster of a `SPELL_AURA_MOD_STALKED` on it ("hunters mark
/// effects should only be visible to owners and not all players"): the
/// one marked, a creature or a player, isn't tracked for another, nor in
/// a newcomer's create; a second stalking aura, another's, tracks it for
/// her too (its flag already up), and the first's going takes it from him
/// alone.
#[tokio::test]
async fn a_hunters_mark_tracks_its_target_for_its_hunter_alone() {
    let realm = harness::realm_with(world());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    let (mut b, gb, _) = enter(&realm, "B", "Bo", Race::Dwarf, Class::Hunter).await;
    let boar = place(&map, ga, BOAR, 0x71_0001, 6.0).await;
    a.sees(boar).await;
    b.sees(boar).await;
    a.clear();
    b.clear();
    on(&map, move |m| put(m, ga, boar, MARK)).await;
    assert_eq!(flags_of(&mut a, boar).await & TRACK, TRACK, "his mark");
    assert_eq!(flags_of(&mut b, boar).await & TRACK, 0, "not hers");
    let (mut c, gc, _) = enter(&realm, "C", "Cy", Race::Dwarf, Class::Warrior).await;
    let created = create_of(&mut c, boar).await;
    assert_eq!(
        created.get(&UNIT_DYNAMIC_FLAGS).copied().unwrap_or(0) & TRACK,
        0,
        "not in a newcomer's create"
    );
    a.clear();
    b.clear();
    on(&map, move |m| put(m, gb, boar, VISION)).await;
    assert_eq!(flags_of(&mut b, boar).await & TRACK, TRACK, "hers too");
    a.clear();
    b.clear();
    on(&map, move |m| auras::remove_spell(m, boar, MARK, Some(ga))).await;
    assert_eq!(flags_of(&mut a, boar).await & TRACK, 0, "his gone");
    assert_eq!(flags_of(&mut b, boar).await & TRACK, TRACK, "hers kept");
    // A player marked: tracked for the hunter, not for one beside him.
    a.sees(gc).await;
    a.clear();
    b.clear();
    on(&map, move |m| put(m, ga, gc, MARK)).await;
    assert_eq!(
        flags_of(&mut a, gc).await & TRACK,
        TRACK,
        "a player he marked"
    );
    assert_eq!(flags_of(&mut b, gc).await & TRACK, 0, "not for another");
}

/// A creature's damage and resistances go to whoever's Empathy (Beast
/// Lore) is on it, and to no one else (mangos's `UF_FLAG_SPECIAL_INFO`):
/// its caster is sent them as the aura lands (`HandleAuraEmpathy`), and
/// their changes after; another sees neither. A pet's owner-only fields
/// (its damage, stats, resistances) go to its master alone
/// (`UF_FLAG_OWNER_ONLY`), in his create and its changes.
#[tokio::test]
async fn damage_and_resistances_go_to_the_master_and_to_an_empathys_caster() {
    let realm = harness::realm_with(world());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    let (mut b, _gb, _) = enter(&realm, "B", "Bo", Race::Dwarf, Class::Hunter).await;
    let fire = UNIT_FIELD_RESISTANCES + 2;
    let boar = on(&map, move |m| {
        let t = m.realm.data.creature_template(BOAR).cloned().unwrap();
        let at = m.players[&ga].position();
        let mut c = Creature::new(guid::unit(BOAR, 0x71_0002), &t, 1, at, 0.0);
        c.fields.set_u32(fire, 12);
        m.add_creature(c.with_base())
    })
    .await;
    let seen = create_of(&mut b, boar).await;
    let mine = create_of(&mut a, boar).await;
    for f in [&seen, &mine] {
        assert!(f.contains_key(&UNIT_FIELD_HEALTH), "its health: {f:?}");
        assert!(
            !f.contains_key(&UNIT_FIELD_MAXDAMAGE) && !f.contains_key(&fire),
            "a stranger's create: {f:?}"
        );
    }
    a.clear();
    b.clear();
    on(&map, move |m| put(m, ga, boar, LORE)).await;
    let told = values_of(&mut a, boar, "its special information", |f| {
        f.contains_key(&fire)
    })
    .await;
    assert_eq!(told.get(&fire), Some(&12), "its resistance");
    assert!(
        told.contains_key(&UNIT_FIELD_MAXDAMAGE),
        "its damage: {told:?}"
    );
    let theirs = flags_of(&mut b, boar).await;
    assert_ne!(
        theirs & crate::game::map::pets::DYNFLAG_SPECIALINFO,
        0,
        "the flag is anyone's"
    );
    a.clear();
    b.clear();
    on(&map, move |m| {
        m.creature_mut(boar).unwrap().fields.set_u32(fire, 30);
        m.creature_mut(boar)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 5);
    })
    .await;
    let told = values_of(&mut a, boar, "its change", |f| f.contains_key(&fire)).await;
    assert_eq!(told.get(&fire), Some(&30), "the Empathy's caster");
    let theirs = values_of(&mut b, boar, "its health", |f| {
        f.contains_key(&UNIT_FIELD_HEALTH)
    })
    .await;
    assert!(!theirs.contains_key(&fire), "not another: {theirs:?}");
    // A pet: its master's create has its stats and damage; another's not.
    let pet = on(&map, move |m| {
        let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
        let at = m.players[&ga].position();
        let mut c = Creature::new(guid::unit(WOLF, 0x71_0003), &t, 1, at, 0.0);
        c.owner = Some(ga);
        c.fields.set_u32(UNIT_FIELD_STAT0, 22);
        c.fields.set_guid(UNIT_FIELD_SUMMONEDBY, ga);
        m.add_creature(c)
    })
    .await;
    let mine = create_of(&mut a, pet).await;
    let seen = create_of(&mut b, pet).await;
    assert_eq!(
        mine.get(&UNIT_FIELD_STAT0),
        Some(&22),
        "its master's: {mine:?}"
    );
    assert!(
        mine.contains_key(&UNIT_FIELD_MAXDAMAGE),
        "its damage: {mine:?}"
    );
    assert!(
        !seen.contains_key(&UNIT_FIELD_STAT0) && !seen.contains_key(&UNIT_FIELD_MAXDAMAGE),
        "another's: {seen:?}"
    );
    a.clear();
    b.clear();
    on(&map, move |m| {
        let f = &mut m.creature_mut(pet).unwrap().fields;
        f.set_u32(UNIT_FIELD_STAT0, 25);
        f.set_u32(UNIT_FIELD_HEALTH, 3);
    })
    .await;
    let told = values_of(&mut a, pet, "its stat", |f| {
        f.contains_key(&UNIT_FIELD_STAT0)
    })
    .await;
    assert_eq!(told.get(&UNIT_FIELD_STAT0), Some(&25));
    let theirs = values_of(&mut b, pet, "its health", |f| {
        f.contains_key(&UNIT_FIELD_HEALTH)
    })
    .await;
    assert!(
        !theirs.contains_key(&UNIT_FIELD_STAT0),
        "not another: {theirs:?}"
    );
}

/// The client check (`tests_client_emu`) of each viewer's own view: a
/// creature and a player marked, for the hunter and for another; Beast
/// Lore's special information sent to its caster; a pet's create for its
/// master and for a stranger, then its changes; the 1.12 client takes
/// every frame.
#[tokio::test]
#[ignore = "extended"]
async fn each_viewers_own_view_passes_the_client() {
    use crate::game::tests_client_emu as emu;
    let (check_py, wow_exe) = emu::checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let realm = harness::realm_with(world());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    let (mut b, gb, _) = enter(&realm, "B", "Bo", Race::Dwarf, Class::Hunter).await;
    let boar = place(&map, ga, BOAR, 0x71_0011, 6.0).await;
    a.sees(boar).await;
    b.sees(boar).await;
    on(&map, move |m| {
        put(m, ga, boar, MARK);
        put(m, ga, gb, MARK);
        put(m, ga, boar, LORE);
    })
    .await;
    flags_of(&mut b, boar).await;
    let pet = on(&map, move |m| {
        let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
        let at = m.players[&ga].position();
        let mut c = Creature::new(guid::unit(WOLF, 0x71_0012), &t, 1, at, 0.0);
        c.owner = Some(ga);
        c.fields.set_u32(UNIT_FIELD_STAT0, 22);
        c.fields.set_guid(UNIT_FIELD_SUMMONEDBY, ga);
        m.add_creature(c)
    })
    .await;
    a.sees(pet).await;
    b.sees(pet).await;
    on(&map, move |m| {
        m.creature_mut(pet)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_STAT0, 25);
        auras::remove_spell(m, boar, MARK, Some(ga));
        auras::remove_spell(m, gb, MARK, Some(ga));
    })
    .await;
    for _ in 0..6 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        a.captured();
        b.captured();
    }
    let (capture, counts) = emu::capture(vec![&mut a, &mut b]);
    assert!(
        counts.get(&0x0A9).copied().unwrap_or(0) + counts.get(&0x1F6).copied().unwrap_or(0) > 0,
        "the creates and values were captured"
    );
    emu::run_checker(&check_py, &wow_exe, &capture);
}

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    leader
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
}

/// A corpse's loot sparkles (`UNIT_DYNFLAG_LOOTABLE`) only for those who
/// may open it, as mangos's per-viewer `Loot::CanLoot` shows it: under
/// round robin its looter, not the rest of the group it fell to, though
/// it isn't grey to them (theirs is the kill); nor for one who comes
/// later from outside it, in their create.
#[tokio::test]
async fn a_corpses_loot_shows_only_to_those_who_may_open_it() {
    use crate::game::map::loot::{DYNFLAG_LOOTABLE, Loot};
    const TAPPED: u32 = crate::game::map::ai::DYNFLAG_TAPPED;
    let realm = harness::realm_with(world());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    let (mut c, gc, _) = enter(&realm, "C", "Cy", Race::Dwarf, Class::Hunter).await;
    group(&mut a, &mut c, "Cy").await;
    let boar = place(&map, ga, BOAR, 0x71_0021, 6.0).await;
    a.sees(boar).await;
    c.sees(boar).await;
    a.clear();
    c.clear();
    on(&map, move |m| {
        let realm = m.realm.clone();
        let b = m.creature_mut(boar).unwrap();
        crate::game::map::tap::take(&realm, b, ga);
        b.loot = Some(Loot {
            gold: 10,
            allowed: vec![ga, gc],
            method: Some(GroupLootSetting::RoundRobin),
            assigned: Some(ga),
            ..Default::default()
        });
        b.fields
            .set_flag(UNIT_DYNAMIC_FLAGS, DYNFLAG_LOOTABLE, true);
    })
    .await;
    let mine = flags_of(&mut a, boar).await;
    let theirs = flags_of(&mut c, boar).await;
    assert_eq!(
        mine & (DYNFLAG_LOOTABLE | TAPPED),
        DYNFLAG_LOOTABLE,
        "the looter's"
    );
    assert_eq!(
        theirs & (DYNFLAG_LOOTABLE | TAPPED),
        0,
        "the group's other member"
    );
    // One who comes later, outside the group, in their create.
    let created = {
        let (mut c2, _, _) = enter(&realm, "D", "Di", Race::Dwarf, Class::Hunter).await;
        create_of(&mut c2, boar).await
    };
    assert_eq!(
        created.get(&UNIT_DYNAMIC_FLAGS).copied().unwrap_or(0) & DYNFLAG_LOOTABLE,
        0,
        "nor in a stranger's create"
    );
}
