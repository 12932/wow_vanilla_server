//! Round 12f: a pet keeps its auras while it is away and has them back
//! when it comes again, less the time it was away, as mangos-classic's
//! `Pet::_SaveAuras` and `_LoadAuras` keep them (`pet_aura`): over its
//! master's mount, a logout and a change of map.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{SpellDuration, SpellEffect};
use crate::game::fields::*;
use crate::game::harness;
use crate::game::map::{Map, MapHandle, MapKey, auras, spells};
use crate::game::tests_pets::{SUMMON_IMP, cast, enter, learn, pet_bar, spell};

/// A buff for the imp: ten minutes of more stamina, cast at a unit.
const FORTITUDE: u32 = 995_300;
const TEN_MINUTES: u32 = 995;
/// One the imp casts on itself, and a stealth, which isn't kept.
const OWN_BUFF: u32 = 995_301;
const PROWL: u32 = 995_302;

fn data() -> GameData {
    let mut d = crate::game::tests_pets::data();
    d.spells.durations.insert(
        TEN_MINUTES,
        SpellDuration {
            base: 600_000,
            ..Default::default()
        },
    );
    let none = SpellEffect::default();
    let buff = |id: u32, name: &str, aura: u32, target_a: u32| {
        let mut e = crate::game::tests_pets::effect(6, 9, target_a);
        e.aura = aura;
        e.misc_value = 2;
        let mut s = spell(id, name, [e, none, none]);
        s.duration_index = TEN_MINUTES;
        s
    };
    for s in [
        buff(FORTITUDE, "Fortitude", auras::AURA_MOD_STAT, 6),
        buff(OWN_BUFF, "Own buff", auras::AURA_MOD_STAT, 1),
        buff(PROWL, "Prowl", auras::AURA_MOD_STEALTH, 1),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// What is left of each buff on `pet`: Fortitude's from the warlock, the
/// imp's own and its stealth.
fn left(m: &Map, pet: Guid, gw: Guid) -> [Option<Duration>; 3] {
    let now = m.now;
    let of = |spell: u32, by: Guid| {
        auras::auras(m, pet)
            .and_then(|a| {
                a.holders
                    .iter()
                    .find(|h| h.spell == spell && h.caster == by)
                    .cloned()
            })
            .and_then(|h| h.expires)
            .map(|t| t.saturating_duration_since(now))
    };
    [of(FORTITUDE, gw), of(OWN_BUFF, pet), of(PROWL, pet)]
}

/// The warlock's imp out and buffed: by the warlock, by itself, and
/// stealthed.
async fn buffed_imp(
    name: &str,
) -> (
    std::sync::Arc<crate::game::Realm>,
    harness::Client,
    Guid,
    MapHandle,
    Guid,
) {
    let realm = harness::realm_with(data());
    let (mut w, gw, map) = enter(&realm, name, "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    assert!(!imp.is_zero());
    let got = map
        .query(move |m| {
            spells::cast_triggered(m, gw, FORTITUDE, Some(imp));
            spells::cast_triggered(m, imp, OWN_BUFF, Some(imp));
            spells::cast_triggered(m, imp, PROWL, Some(imp));
            left(m, imp, gw)
        })
        .await
        .unwrap();
    assert!(got.iter().all(Option::is_some), "buffed: {got:?}");
    (realm, w, gw, map, imp)
}

/// Whether what came back has the buffs it had, but stealth, with no more
/// time than they had.
fn kept(got: [Option<Duration>; 3]) {
    let ten = Duration::from_secs(600);
    let [fortitude, own, prowl] = got;
    assert!(
        fortitude.is_some_and(|l| l <= ten && l > ten - Duration::from_secs(60)),
        "the warlock's buff kept: {fortitude:?}"
    );
    assert!(
        own.is_some_and(|l| l <= ten && l > ten - Duration::from_secs(60)),
        "its own buff kept, as its own: {own:?}"
    );
    assert_eq!(prowl, None, "its stealth not kept");
}

/// Over its master's mount: the imp goes for a while and comes back with
/// its buffs.
#[tokio::test]
async fn a_pet_keeps_its_buffs_over_its_masters_mount() {
    let (_realm, mut w, gw, map, imp) = buffed_imp("P12FPMNT").await;
    w.clear();
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&gw) {
            p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 14_376);
        }
    });
    assert!(pet_bar(&mut w).await.is_zero(), "gone while mounted");
    let gone = map.query(move |m| m.creature(imp).is_none()).await.unwrap();
    assert!(gone);
    map.call(move |m| {
        if let Some(p) = m.players.get_mut(&gw) {
            p.fields.set_u32(UNIT_FIELD_MOUNTDISPLAYID, 0);
        }
    });
    let back = pet_bar(&mut w).await;
    assert!(!back.is_zero(), "back once down");
    kept(map.query(move |m| left(m, back, gw)).await.unwrap());
}

/// Over a logout: the imp comes back at login with its buffs.
#[tokio::test]
async fn a_pet_keeps_its_buffs_over_a_logout() {
    let (_realm, mut w, gw, map, _imp) = buffed_imp("P12FPLOG").await;
    w.send(CMSG_LOGOUT_REQUEST {}).await;
    w.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    w.clear();
    w.login(gw).await;
    let back = pet_bar(&mut w).await;
    assert!(!back.is_zero());
    kept(map.query(move |m| left(m, back, gw)).await.unwrap());
}

/// Over a change of map: the imp comes along to Kalimdor with its buffs.
#[tokio::test]
async fn a_pet_keeps_its_buffs_over_a_change_of_map() {
    let (realm, mut w, gw, map, _imp) = buffed_imp("P12FPMAP").await;
    w.clear();
    map.call(move |m| {
        m.teleport(
            gw,
            MapKey::world(1),
            Vector3d {
                x: -618.5,
                y: -4251.7,
                z: 38.7,
            },
            0.0,
        )
    });
    w.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    w.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    let back = w
        .wait_for("the imp's bar", |m| match m {
            ServerOpcodeMessage::SMSG_PET_SPELLS(s) if !s.pet.is_zero() => Some(s.pet),
            _ => None,
        })
        .await;
    let kalimdor = realm.registry.online(gw).unwrap().map();
    assert_eq!(kalimdor.key, MapKey::world(1));
    kept(kalimdor.query(move |m| left(m, back, gw)).await.unwrap());
}
