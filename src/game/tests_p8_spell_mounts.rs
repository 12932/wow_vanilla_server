//! Round 8, against Thistle Tea's `mount_test.exs`: what of Discombobulate
//! takes a rider off their mount (vmangos's script of the Discombobulator
//! Ray, Thistle Tea's `MountSync.dismount_on_transform`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh};
use crate::game::data::GameData;
use crate::game::data::spells::SpellTemplate;
use crate::game::map::Map;
use crate::game::map::auras::*;

/// A mount of the kobold's look; the Discombobulator Ray's transform and
/// slow; another transform of the same; a buff; an immunity to
/// transforms.
const MOUNT: u32 = 998_501;
const DISCOMBOBULATE: u32 = 4_060;
const COSTUME: u32 = 998_502;
const BUFF: u32 = 998_503;
const NO_TRANSFORMS: u32 = 998_504;

fn data(d: &mut GameData) {
    let mut mount = bare(MOUNT, 0);
    mount.duration_index = LASTS_LONG;
    effect(&mut mount, 0, aura(AURA_MOUNTED, KOBOLD as i32, 0, 1));
    effect(
        &mut mount,
        1,
        aura(AURA_MOD_INCREASE_MOUNTED_SPEED, 0, 100, 1),
    );
    let mut ray = bare(DISCOMBOBULATE, 0);
    ray.duration_index = LASTS_5S;
    effect(&mut ray, 0, aura(AURA_TRANSFORM, KOBOLD as i32, 0, 6));
    effect(&mut ray, 1, aura(AURA_MOD_DECREASE_SPEED, 0, -20, 6));
    let mut costume = ray.clone();
    costume.id = COSTUME;
    let mut buff = bare(BUFF, 0);
    buff.duration_index = LASTS_LONG;
    effect(&mut buff, 0, aura(AURA_DUMMY, 0, 0, 1));
    let mut immune: SpellTemplate = bare(NO_TRANSFORMS, 0);
    immune.duration_index = LASTS_LONG;
    effect(
        &mut immune,
        0,
        aura(AURA_STATE_IMMUNITY, AURA_TRANSFORM as i32, 0, 1),
    );
    insert(d, [mount, ray, costume, buff, immune]);
}

/// The Black Qiraji Battle Tank's summons; a dungeon; the Temple of
/// Ahn'Qiraj.
const BLACK_QIRAJI: u32 = 26_656;
const DUNGEON: u32 = 33;
const TEMPLE: u32 = 531;

fn places(d: &mut GameData) {
    data(d);
    for (id, kind) in [(DUNGEON, 1), (TEMPLE, 2)] {
        d.maps.insert(
            id,
            crate::game::data::MapInfo {
                id,
                internal_name: format!("Map{id}"),
                name: format!("Map {id}"),
                instance_type: kind,
                player_limit: 0,
            },
        );
    }
    let mut tank = bare(BLACK_QIRAJI, 0);
    tank.attributes[0] |= crate::game::data::spells::ATTR_ALLOW_WHILE_MOUNTED;
    insert(d, [tank]);
}

/// The Black Qiraji Battle Tank goes where a mount goes, and in a dungeon
/// only in its Temple, where another mount doesn't; triggered, anywhere;
/// not swimming; and neither it nor a mount on a ship's deck not known to
/// be open to the sky (Thistle Tea's "Black Qiraji is allowed in its
/// temple and ordinary mount maps" and, of "transports distinguish exposed
/// decks and the Black Qiraji restriction", the closed deck and the tank).
#[tokio::test]
async fn the_qiraji_tank_goes_where_mounts_go_and_in_its_temple() {
    use crate::game::map::mounts::{NO_MOUNTS_ALLOWED, ONLY_ABOVEWATER, check};
    let realm = fast_realm(places);
    let (_c, a, map) = human(&realm, "P8MT4", "Tanker", Class::Warrior).await;
    let out = map
        .query(move |m| {
            let s = |m: &Map, id: u32| m.realm.data.spells.get(id).unwrap().clone();
            let (tank, mount) = (s(m, BLACK_QIRAJI), s(m, MOUNT));
            let world = (check(m, a, &tank, false), check(m, a, &mount, false));
            let home = m.key;
            m.key.map = DUNGEON;
            let dungeon = (
                check(m, a, &tank, false),
                check(m, a, &mount, false),
                check(m, a, &tank, true),
            );
            m.key.map = TEMPLE;
            let temple = (check(m, a, &tank, false), check(m, a, &mount, false));
            m.key = home;
            let flags = m.players[&a].movement.flags;
            m.players.get_mut(&a).unwrap().movement.flags =
                MovementInfo_MovementFlags::empty().set_swimming(Default::default());
            let swimming = check(m, a, &tank, false);
            m.players.get_mut(&a).unwrap().movement.flags = MovementInfo_MovementFlags::empty()
                .set_on_transport(MovementInfo_MovementFlags_OnTransport {
                    transport: TransportInfo {
                        guid: Guid::new(0x1FC0_0000_0000_0001),
                        position: Vector3d {
                            x: 1.0,
                            y: 2.0,
                            z: 3.0,
                        },
                        orientation: 0.0,
                    },
                });
            let deck = (check(m, a, &tank, false), check(m, a, &mount, false));
            m.players.get_mut(&a).unwrap().movement.flags = flags;
            (world, dungeon, temple, swimming, deck)
        })
        .await
        .unwrap();
    let no = Some(NO_MOUNTS_ALLOWED);
    assert_eq!(out.0, (None, None), "the world");
    assert_eq!(out.1, (no, no, None), "a dungeon");
    assert_eq!(out.2, (None, no), "the Temple");
    assert_eq!(out.3, Some(ONLY_ABOVEWATER), "swimming");
    assert_eq!(out.4, (no, no), "a closed deck");
}

fn riding(m: &Map, g: Guid) -> bool {
    kit::has(m, g, MOUNT)
}

/// Discombobulate's transform takes its target off their mount as it
/// lands, again as it lands again, and not when the mount comes after it;
/// (a cast of a mount is refused to the transformed, as mangos-classic's
/// `SPELL_FAILED_NOT_SHAPESHIFT`);
/// a buff or another transform of the same look leaves the mount (Thistle
/// Tea's "only a new Discombobulate transformation interrupts a mount").
#[tokio::test]
async fn only_discombobulate_landing_takes_a_rider_off() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8MT1", "Rider", Class::Warrior).await;
    let (_c2, b, _) = human(&realm, "P8MT2", "Gnome", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MOUNT, BUFF]);
            kit::ready(m, b, 60, &[DISCOMBOBULATE, COSTUME]);
            kit::apply(m, b, a, DISCOMBOBULATE);
            // Mounted as it holds (a script's mount: a cast of one is
            // refused to the transformed).
            kit::apply(m, a, a, MOUNT);
            let after = riding(m, a);
            kit::apply(m, a, a, BUFF);
            let buffed = riding(m, a);
            kit::apply(m, b, a, DISCOMBOBULATE);
            let again = riding(m, a);
            remove_where(m, a, |h| h.spell == DISCOMBOBULATE);
            cast_fresh(m, a, MOUNT, Default::default());
            kit::apply(m, b, a, COSTUME);
            (after, buffed, again, riding(m, a))
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, false, true));
}

/// One immune to transforms keeps their mount and their look, and takes
/// Discombobulate's slow (Thistle Tea's "transform immunity retains the
/// mount while the Discombobulate slow lands").
#[tokio::test]
async fn one_immune_to_transforms_keeps_their_mount_and_is_slowed() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P8MT3", "Steady", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[MOUNT]);
            kit::apply(m, a, a, NO_TRANSFORMS);
            cast_fresh(m, a, MOUNT, Default::default());
            let kobold = kit::dummy(m, a, 60, 5.0);
            // The kobold's ray, at the rider.
            assert!(cast_fresh(m, kobold, DISCOMBOBULATE, at(a)), "cast");
            let transformed = auras(m, a).is_some_and(|x| x.has(AURA_TRANSFORM));
            let slowed = auras(m, a).is_some_and(|x| x.has(AURA_MOD_DECREASE_SPEED));
            (riding(m, a), transformed, slowed)
        })
        .await
        .unwrap();
    assert_eq!(out, (true, false, true));
}
