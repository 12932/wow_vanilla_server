//! Round 5's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/{corpses,self_resurrection,summoning,travel,movement_control}_test.exs`
//! (docs/parity/matrix.csv): what a stale or misplaced request to release,
//! reclaim, rise or travel leaves alone.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::Graveyard;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::death::{self, Death};
use crate::game::map::{Map, MapKey, misc};
use crate::game::tests_p3_combat::{HELPER, START, setup_with, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};
use crate::game::tests_p3_player2_ammo::handled;

/// Whether `c` is sent a teleport within [`NONE_WITHIN`].
async fn teleported(c: &mut Client) -> Option<MovementInfo> {
    c.gets(NONE_WITHIN, |m| match m {
        ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info),
        _ => None,
    })
    .await
}

/// Release the dead `g` and take up the teleport, returning where it went.
async fn release(c: &mut Client, g: Guid) -> MovementInfo {
    c.send(CMSG_REPOP_REQUEST {}).await;
    let info = c
        .wait_for("the teleport to the graveyard", |m| match m {
            ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(t) => Some(t.info),
            _ => None,
        })
        .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: c.teleport_order(),
        time: Default::default(),
    })
    .await;
    handled(c).await;
    info
}

fn death_of(m: &Map, g: Guid) -> Death {
    m.players[&g].death
}

/// A graveyard of either side on map 0, 30 yards east of the start, facing
/// north-west.
const FACING: f32 = 2.5;

fn graveyard_world() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.graveyards.push(Graveyard {
        id: 1,
        map: 0,
        x: START.0 + 30.0,
        y: START.1,
        z: START.2,
        orientation: FACING,
        zone: 0,
        faction: 0,
    });
    d
}

/// Thistle Tea's `Corpses` "ignores a stale rescue request after
/// resurrection", "rejects a distant, missing, or different-copy corpse"
/// and "requires a released ghost and a ready session", and
/// `MovementControl`'s "turns the spirit to the graveyard's facing": a
/// release asked for by the living does nothing; the released spirit
/// stands at the graveyard facing as it does; a corpse is reclaimed only by
/// its released ghost, within reach of it, and only while it is there.
#[tokio::test]
async fn release_and_reclaim_refuse_the_living_the_unreleased_the_far_and_the_corpseless() {
    let (_realm, mut a, ga, map) = setup_with(graveyard_world(), Class::Warrior).await;
    a.send(CMSG_REPOP_REQUEST {}).await;
    handled(&mut a).await;
    assert!(teleported(&mut a).await.is_none(), "the living stay");
    let (death, corpse) = on(&map, move |m| {
        (death_of(m, ga), m.players[&ga].row.state.corpse.is_some())
    })
    .await;
    assert_eq!((death, corpse), (Death::Alive, false));
    // Dead, not released: the reclaim does nothing.
    let corpse = crate::game::guid::corpse(crate::game::guid::counter(ga));
    on(&map, move |m| death::kill(m, ga)).await;
    a.send(CMSG_RECLAIM_CORPSE { guid: corpse }).await;
    handled(&mut a).await;
    assert!(matches!(
        on(&map, move |m| death_of(m, ga)).await,
        Death::Dead { .. }
    ));
    // Released: at the graveyard, as it faces.
    let at = release(&mut a, ga).await;
    assert!((at.position.x - (START.0 + 30.0)).abs() < 0.01, "{at:?}");
    assert!((at.orientation - FACING).abs() < 0.001, "{at:?}");
    // The wait over, but thirty yards off: the reclaim radius is 39, from
    // the graveyard's 30, so place the ghost 40 off.
    let reclaim = async |a: &mut Client| {
        a.send(CMSG_RECLAIM_CORPSE { guid: corpse }).await;
        handled(a).await;
        on(&map, move |m| death_of(m, ga)).await
    };
    let place = move |m: &mut Map, dx: f32| {
        let since = m.now - Duration::from_secs(60);
        let p = m.players.get_mut(&ga).unwrap();
        p.death = Death::Ghost { since };
        p.movement.position = Vector3d {
            x: START.0 + dx,
            y: START.1,
            z: START.2,
        };
    };
    on(&map, move |m| place(m, 40.0)).await;
    assert!(ghost(reclaim(&mut a).await), "too far");
    // Near, but the corpse gone: nothing to rise in.
    let kept = on(&map, move |m| {
        place(m, 2.0);
        m.players.get_mut(&ga).unwrap().row.state.corpse.take()
    })
    .await;
    assert!(ghost(reclaim(&mut a).await), "no corpse");
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().row.state.corpse = kept
    })
    .await;
    assert_eq!(reclaim(&mut a).await, Death::Alive, "near its corpse");
    // Risen, a second release asks nothing.
    a.clear();
    a.send(CMSG_REPOP_REQUEST {}).await;
    handled(&mut a).await;
    assert!(teleported(&mut a).await.is_none(), "a stale release");
    assert_eq!(on(&map, move |m| death_of(m, ga)).await, Death::Alive);
}

/// Reincarnation, as `tests_parity_misc` has it: the spell, its passive, the
/// Ankh.
const REINCARNATION: u32 = 21_169;
const REINCARNATION_PASSIVE: u32 = 20_608;
const ANKH: u32 = 17_030;

fn reincarnation_world() -> crate::game::data::GameData {
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    let mut d = crate::game::tests_p3_combat::data();
    let mut s = SpellTemplate {
        id: REINCARNATION,
        name: "Reincarnation".into(),
        category: 1161,
        category_recovery_time: 3_600_000,
        equipped_item_class: -1,
        ..Default::default()
    };
    s.reagents[0] = (ANKH, 1);
    s.effects[0] = SpellEffect {
        effect: 94,
        base_points: 19,
        base_dice: 1,
        ..Default::default()
    };
    d.spells.spells.insert(REINCARNATION, s);
    let mut passive = SpellTemplate {
        id: REINCARNATION_PASSIVE,
        name: "Reincarnation (passive)".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    passive.attributes[0] = 0x40;
    passive.effects[0] = SpellEffect {
        effect: 6,
        aura: 4,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(REINCARNATION_PASSIVE, passive);
    d.items.insert(
        ANKH,
        crate::game::data::ItemTemplate {
            entry: ANKH,
            name: "Ankh".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    d
}

/// Thistle Tea's `SelfResurrection` "offers learned Reincarnation only with
/// a carried Ankh", "rejects Reincarnation during cooldown" and "ignores
/// requests before login or while alive": a shaman who dies without an
/// Ankh is offered nothing; with one, Reincarnation, unless its category is
/// cooling down, and the offer returns the moment it is over. A living
/// player's `CMSG_SELF_RES` does nothing, even with the field set.
#[tokio::test]
async fn reincarnation_is_offered_only_with_an_ankh_and_off_its_cooldown() {
    let realm = crate::game::harness::realm_with(reincarnation_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Thrall", Race::Orc, Class::Shaman).await;
    let key = realm.registry.online(ga).expect("online").map().key;
    let map = realm.maps.get(&realm, key);
    let offered = move |m: &mut Map| {
        death::kill(m, ga);
        let s = m.players[&ga].fields.get_u32(PLAYER_SELF_RES_SPELL);
        death::resurrect(m, ga, 1.0);
        s
    };
    let none = on(&map, move |m| {
        crate::game::map::spells::learn(m, ga, REINCARNATION_PASSIVE);
        offered(m)
    })
    .await;
    assert_eq!(none, 0, "no Ankh");
    let with = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, ANKH, 1, false).unwrap();
        offered(m)
    })
    .await;
    assert_eq!(with, REINCARNATION, "an Ankh carried");
    let (cooling, over) = on(&map, move |m| {
        let now = m.now;
        let c = &mut m.players.get_mut(&ga).unwrap().caster;
        c.category_cooldowns
            .insert(1161, now + Duration::from_millis(1));
        let cooling = offered(m);
        let c = &mut m.players.get_mut(&ga).unwrap().caster;
        c.category_cooldowns.insert(1161, now);
        (cooling, offered(m))
    })
    .await;
    assert_eq!(
        (cooling, over),
        (0, REINCARNATION),
        "its category's cooldown"
    );
    // Alive, with the field set: nothing taken, nothing started.
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(PLAYER_SELF_RES_SPELL, REINCARNATION);
        p.caster.category_cooldowns.clear();
    })
    .await;
    a.clear();
    a.send(CMSG_SELF_RES {}).await;
    handled(&mut a).await;
    let (ankhs, cooling) = on(&map, move |m| {
        let p = &m.players[&ga];
        (p.count_of(ANKH), p.caster.category_cooldowns.len())
    })
    .await;
    assert_eq!((ankhs, cooling), (1, 0), "the living don't rise again");
}

/// Thistle Tea's `Summoning` "rejects expired, mismatched, or combat
/// summons": a summons is taken up only from the one who offered it,
/// before it runs out, out of combat; a refused one is used up.
#[tokio::test]
async fn a_summons_is_taken_only_from_its_summoner_in_time_and_out_of_combat() {
    let (_realm, mut a, ga, map) =
        setup_with(crate::game::tests_p3_combat::data(), Class::Warrior).await;
    let summoner = crate::game::guid::player(4242);
    let offer = move |m: &mut Map, for_ms: u64| {
        let until = m.now + Duration::from_millis(for_ms);
        m.players.get_mut(&ga).unwrap().misc.summon = Some(misc::Summon {
            by: summoner,
            to: MapKey::world(0),
            at: Vector3d {
                x: START.0 + 50.0,
                y: START.1,
                z: START.2,
            },
            orientation: 0.0,
            until,
        });
    };
    let take = async |a: &mut Client, by: Guid| {
        a.clear();
        a.send(CMSG_SUMMON_RESPONSE { summoner: by }).await;
        handled(a).await;
        teleported(a).await.is_some()
    };
    on(&map, move |m| offer(m, 60_000)).await;
    assert!(!take(&mut a, ga).await, "another's summons");
    assert!(
        on(&map, move |m| m.players[&ga].misc.summon.is_none()).await,
        "a refused summons is spent"
    );
    on(&map, move |m| offer(m, 0)).await;
    assert!(!take(&mut a, summoner).await, "run out");
    let brute = on(&map, move |m| {
        offer(m, 60_000);
        // One on her side, so that the fight is only the flag.
        let c = spawn(m, ga, HELPER, 20, 3.0);
        crate::game::map::combat::engage(m, ga, c);
        c
    })
    .await;
    assert!(!take(&mut a, summoner).await, "in combat");
    on(&map, move |m| {
        crate::game::map::combat::leave_combat(m, ga);
        m.remove_creature(brute);
        offer(m, 60_000);
    })
    .await;
    assert!(take(&mut a, summoner).await, "its summoner's, in time");
}

/// Thistle Tea's `Travel` "ignores unsolicited and replayed worldport
/// acknowledgements": a `MSG_MOVE_WORLDPORT_ACK` the server didn't ask
/// for sends the client nothing: no new world, no create of itself.
#[tokio::test]
async fn an_unasked_worldport_acknowledgement_is_ignored() {
    let (_realm, mut a, ga, map) =
        setup_with(crate::game::tests_p3_combat::data(), Class::Warrior).await;
    a.sees(ga).await;
    a.clear();
    a.send(ClientOpcodeMessage::MSG_MOVE_WORLDPORT_ACK).await;
    handled(&mut a).await;
    assert!(!a.sees_within(ga, NONE_WITHIN).await, "no create of itself");
    let awaiting = on(&map, move |m| m.players[&ga].awaiting_worldport).await;
    assert!(!awaiting);
}

use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;

fn ghost(d: Death) -> bool {
    matches!(d, Death::Ghost { .. })
}

/// Thistle Tea's `SelfResurrection` "prioritizes a captured Soulstone
/// without spending an Ankh" and "does not offer an unknown or malformed
/// resurrection spell": a shaman with a Soulstone on them as they die is
/// offered the stone's resurrection, not Reincarnation; and a
/// Reincarnation with no self-resurrection in it is offered not at all.
#[tokio::test]
async fn a_soulstone_comes_before_reincarnation_and_a_malformed_one_is_not_offered() {
    const SOULSTONE: u32 = 20_707;
    const STONE_RES: u32 = 3026;
    for malformed in [false, true] {
        let mut d = reincarnation_world();
        if malformed {
            d.spells.spells.get_mut(&REINCARNATION).unwrap().effects[0].effect = 3;
        }
        let realm = crate::game::harness::realm_with(d);
        let mut a = Client::connect(&realm, "A");
        let ga = a.enter("Thrall", Race::Orc, Class::Shaman).await;
        let key = realm.registry.online(ga).expect("online").map().key;
        let map = realm.maps.get(&realm, key);
        let offered = on(&map, move |m| {
            crate::game::map::spells::learn(m, ga, REINCARNATION_PASSIVE);
            crate::game::map::inventory::add_item(m, ga, ANKH, 1, false).unwrap();
            let bare = {
                death::kill(m, ga);
                let s = m.players[&ga].fields.get_u32(PLAYER_SELF_RES_SPELL);
                death::resurrect(m, ga, 1.0);
                s
            };
            crate::game::tests_p3_combat::aura_of(m, ga, SOULSTONE, 4, 0, 1);
            death::kill(m, ga);
            let stoned = m.players[&ga].fields.get_u32(PLAYER_SELF_RES_SPELL);
            (bare, stoned, m.players[&ga].count_of(ANKH))
        })
        .await;
        if malformed {
            assert_eq!(offered.0, 0, "a Reincarnation that raises no one");
        } else {
            assert_eq!(
                offered,
                (REINCARNATION, STONE_RES, 1),
                "the stone first, the Ankh kept"
            );
        }
    }
}
