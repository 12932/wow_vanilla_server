//! Round 10's tests of what each viewer is sent of a unit, ported from
//! Thistle Tea's `test/game/network/empathy_update_test.exs`: an Empathy's
//! (Beast Lore's) caster is sent a creature's damage, both hands', and its
//! resistances, in its values and its create, and none of its other
//! owner-only fields; a bystander, what anyone sees, its dynamic flags as
//! they are.

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
use crate::game::map::pets::DYNFLAG_SPECIALINFO;
use crate::game::map::{Map, auras, visibility};
use crate::game::tests_bugs6::fields;
use crate::game::tests_pets::{BOAR, data, effect, enter, spell};

const LORE: u32 = 97_311;
/// A dynamic flag no viewer sees otherwise (`UNIT_DYNFLAG_ROOTED`).
const ROOTED: u32 = 0x08;

fn world() -> GameData {
    let mut d = data();
    let mut e = effect(6, 0, 6);
    e.aura = auras::AURA_EMPATHY;
    let mut s = spell(
        LORE,
        "Lore",
        [e, SpellEffect::default(), SpellEffect::default()],
    );
    s.duration_index = 4;
    d.spells.spells.insert(s.id, s);
    d
}

/// The fields of the next values block of `of` that `c` is told, with
/// `field` in it.
async fn values_with(c: &mut Client, of: Guid, field: u16) -> BTreeMap<u16, u32> {
    c.wait_for("its values", |m| {
        harness::values(m)
            .into_iter()
            .filter(|(g, _)| *g == of)
            .map(|(_, mask)| fields(&mask))
            .find(|f| f.contains_key(&field))
    })
    .await
}

/// The fields of `c`'s next create of `of`.
async fn create_of(c: &mut Client, of: Guid) -> BTreeMap<u16, u32> {
    c.wait_for("its create", |m| {
        let objects = match m {
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(u) => &u.objects,
            ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(u) => &u.objects,
            _ => return None,
        };
        objects.iter().find_map(|o| match &o.update_type {
            Object_UpdateType::CreateObject2 { guid3, mask2, .. } if *guid3 == of => {
                Some(fields(mask2))
            }
            _ => None,
        })
    })
    .await
}

/// The damage fields, both hands'.
const DAMAGE: [u16; 4] = [
    UNIT_FIELD_MINDAMAGE,
    UNIT_FIELD_MAXDAMAGE,
    UNIT_FIELD_MINOFFHANDDAMAGE,
    UNIT_FIELD_MAXOFFHANDDAMAGE,
];
/// Owner-only fields that aren't an Empathy's: a stat, base mana and
/// health, attack power.
const PRIVATE: [u16; 4] = [
    UNIT_FIELD_STAT0,
    UNIT_FIELD_BASE_MANA,
    UNIT_FIELD_BASE_HEALTH,
    UNIT_FIELD_ATTACK_POWER,
];
/// Its resistances, armor first: one of them none.
const RESISTANCES: [u32; 7] = [300, 0, 10, 20, 30, 40, 50];

/// Whether `f` has `c`'s damage and resistances as they are (a resistance
/// of none needs no word: the client has it so), and none of `PRIVATE`.
fn special(f: &BTreeMap<u16, u32>, c: &[u32]) -> Result<(), String> {
    let wanted = DAMAGE
        .iter()
        .copied()
        .chain((0..7).map(|i| UNIT_FIELD_RESISTANCES + i));
    for (field, value) in wanted.zip(c.iter().copied()) {
        if f.get(&field).copied().unwrap_or(0) != value {
            return Err(format!("{field:#x}: {:?}, not {value}", f.get(&field)));
        }
    }
    match PRIVATE.iter().find(|p| f.contains_key(p)) {
        Some(p) => Err(format!("{p:#x} sent")),
        None => Ok(()),
    }
}

/// Thistle Tea's `to_packet/2` "caster receives damage and every resistance
/// but no unrelated private fields" and "bystanders receive public fields
/// and preserved loot flags", as mangos-classic's
/// `GetUpdateFieldFlagsForTarget` has them (`UF_FLAG_SPECIAL_INFO` for the
/// one `CanSeeSpecialInfoOf`): Beast Lore's caster is sent the creature's
/// damage, both hands', and its resistances as the aura lands, in their
/// changes after and in a create when it comes back into sight, and none
/// of its stats, base mana, attack power or power costs; a bystander is
/// sent its health and its dynamic flags as they are, Beast Lore's flag
/// with them (mangos sets it on the unit: `ApplyModUInt32Value`, where
/// Thistle Tea takes it off for a bystander), and none of those fields.
#[tokio::test]
async fn an_empathys_caster_alone_is_sent_damage_and_resistances_in_values_and_creates() {
    let realm = harness::realm_with(world());
    let (mut a, ga, map) = enter(&realm, "A", "Ari", Race::Dwarf, Class::Hunter).await;
    let (mut b, gb, _) = enter(&realm, "B", "Bo", Race::Dwarf, Class::Hunter).await;
    let boar = map
        .query(move |m| {
            let t = m.realm.data.creature_template(BOAR).cloned().unwrap();
            let at = m.players[&ga].position();
            let mut c = Creature::new(guid::unit(BOAR, 0x71_0101), &t, 1, at, 0.0);
            let f = &mut c.fields;
            for (field, v) in DAMAGE.iter().zip([12.5, 25.0, 3.0, 6.0]) {
                f.set_f32(*field, v);
            }
            for (i, r) in RESISTANCES.iter().enumerate() {
                f.set_u32(UNIT_FIELD_RESISTANCES + i as u16, *r);
            }
            for p in PRIVATE {
                f.set_u32(p, 20);
            }
            f.set_u32(UNIT_DYNAMIC_FLAGS, ROOTED);
            m.add_creature(c.with_base())
        })
        .await
        .unwrap();
    create_of(&mut a, boar).await;
    create_of(&mut b, boar).await;
    a.clear();
    b.clear();
    // What it holds: its damage as words, its resistances, its other
    // owner-only fields set.
    let held = move |m: &Map| {
        let f = &m.creature(boar).unwrap().fields;
        let mut words: Vec<u32> = DAMAGE.iter().map(|d| f.get_u32(*d)).collect();
        words.extend((0..7).map(|i| f.get_u32(UNIT_FIELD_RESISTANCES + i)));
        let private: Vec<u32> = PRIVATE.iter().map(|p| f.get_u32(*p)).collect();
        (words, private)
    };
    let (words, private) = map
        .query(move |m| {
            let s = m.realm.data.spells.spells[&LORE].clone();
            auras::apply_spell(m, ga, boar, &s, &[(0, 0)], Default::default()).expect("put on");
            held(m)
        })
        .await
        .unwrap();
    assert!(
        private.iter().all(|v| *v != 0),
        "its other owner-only fields are set: {private:?}"
    );
    let told = values_with(&mut a, boar, UNIT_FIELD_MINOFFHANDDAMAGE).await;
    special(&told, &words).unwrap_or_else(|e| panic!("the caster's values: {e}: {told:?}"));
    let seen = values_with(&mut b, boar, UNIT_DYNAMIC_FLAGS).await;
    assert_eq!(
        seen[&UNIT_DYNAMIC_FLAGS],
        ROOTED | DYNFLAG_SPECIALINFO,
        "a bystander's flags as they are"
    );
    let hidden = |f: &BTreeMap<u16, u32>| {
        DAMAGE
            .iter()
            .chain(PRIVATE.iter())
            .chain([UNIT_FIELD_RESISTANCES, UNIT_FIELD_RESISTANCES + 6].iter())
            .all(|x| !f.contains_key(x))
    };
    assert!(hidden(&seen), "a bystander's values: {seen:?}");
    // A change: its health for both, its armor for the caster alone.
    a.clear();
    b.clear();
    let words = map
        .query(move |m| {
            let f = &mut m.creature_mut(boar).unwrap().fields;
            f.set_u32(UNIT_FIELD_RESISTANCES, 350);
            f.set_u32(UNIT_FIELD_HEALTH, 5);
            held(m).0
        })
        .await
        .unwrap();
    let told = values_with(&mut a, boar, UNIT_FIELD_RESISTANCES).await;
    assert_eq!(
        told[&UNIT_FIELD_RESISTANCES], words[4],
        "the caster's armor"
    );
    assert!(PRIVATE.iter().all(|p| !told.contains_key(p)), "{told:?}");
    let seen = values_with(&mut b, boar, UNIT_FIELD_HEALTH).await;
    assert_eq!(seen[&UNIT_FIELD_HEALTH], 5);
    assert!(hidden(&seen), "a bystander's change: {seen:?}");
    // Out of sight and back: each one's create as they see it.
    a.clear();
    b.clear();
    map.call(move |m| {
        for o in [ga, gb] {
            visibility::part(m, o, boar);
            visibility::introduce(m, o, boar);
        }
    });
    let mine = create_of(&mut a, boar).await;
    special(&mine, &words).unwrap_or_else(|e| panic!("the caster's create: {e}: {mine:?}"));
    assert_eq!(mine[&UNIT_FIELD_HEALTH], 5);
    let theirs = create_of(&mut b, boar).await;
    assert!(hidden(&theirs), "a bystander's create: {theirs:?}");
    assert_eq!(theirs[&UNIT_FIELD_HEALTH], 5);
    assert_eq!(theirs[&UNIT_DYNAMIC_FLAGS], ROOTED | DYNFLAG_SPECIALINFO);
}
