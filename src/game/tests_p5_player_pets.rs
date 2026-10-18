//! Round 5's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/{login,mini_pet_owner}_test.exs`
//! (docs/parity/matrix.csv): which saved pet comes back at login, and a
//! companion's look, its slot apart from the pet's, and its going.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::{CreatureTemplate, GameData};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::death::{self, Death};
use crate::game::map::pets::testing as hunter;
use crate::game::map::pets::{Kind, dismiss};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p3_player2_ammo::handled;

const WOLF: u32 = 963_101;
/// Two companions of level one, one of them a quest giver; and the spells
/// that call each.
const CRITTER: u32 = 990_101;
const OTHER_CRITTER: u32 = 990_102;
const CALL_CRITTER: u32 = 990_500;
const CALL_OTHER: u32 = 990_501;

fn data() -> GameData {
    let mut d = GameData::empty();
    let beast = |entry: u32, name: &str, level: u8| CreatureTemplate {
        entry,
        name: name.into(),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: level,
        level_max: level,
        faction: 14,
        unit_class: 1,
        creature_type: 1,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        health_max: 500,
        ..Default::default()
    };
    d.creature_templates.insert(
        WOLF,
        CreatureTemplate {
            family: 1,
            ..beast(WOLF, "Wolf", 20)
        },
    );
    d.creature_templates
        .insert(CRITTER, beast(CRITTER, "Critter", 1));
    d.creature_templates.insert(
        OTHER_CRITTER,
        CreatureTemplate {
            npc_flags: 2,
            ..beast(OTHER_CRITTER, "Other Critter", 1)
        },
    );
    d.pets.levels.insert(
        (1, 20),
        crate::game::data::pets::PetLevel {
            health: 600,
            mana: 0,
            armor: 400,
            damage: (0.0, 0.0),
            stats: [40, 30, 40, 20, 20],
        },
    );
    for (id, entry) in [(CALL_CRITTER, CRITTER), (CALL_OTHER, OTHER_CRITTER)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Call {entry}"),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: crate::game::map::pets::EFFECT_SUMMON_CRITTER,
            misc_value: entry as i32,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

/// The map `g` is on now.
fn map_of(realm: &Arc<Realm>, g: Guid) -> MapHandle {
    let key = realm.registry.online(g).expect("online").map().key;
    realm.maps.get(realm, key)
}

/// A level 20 dwarf hunter with her wolf out.
async fn hunter_and_wolf() -> (Arc<Realm>, Client, Guid, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let pet = on(&map_of(&realm, h), move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        let at = p.position();
        let pet = hunter::spawn(m, h, WOLF, Kind::Hunter, 20, at, 0.0);
        m.players.get_mut(&h).unwrap().owned.pet = Some(pet);
        crate::game::map::pets::sync_record(m, h);
        pet
    })
    .await;
    (realm, a, h, pet)
}

/// Log `c` out and back in, and whether a pet of theirs is out then.
async fn relogged_with_pet(realm: &Arc<Realm>, c: &mut Client) -> bool {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    let g = c.guid;
    c.login(g).await;
    handled(c).await;
    on(&map_of(realm, g), move |m| {
        m.players[&g]
            .owned
            .pet
            .is_some_and(|pet| m.creature(pet).is_some())
    })
    .await
}

/// Thistle Tea's `Login.restore_companion` "keeps an explicitly dismissed
/// hunter pet absent across automatic restoration", "does not resummon a
/// pet that died before logout", "does not resummon the saved pet while
/// the character is dead" and "... while the character is a ghost": a
/// hunter's pet out at logout is out again at login; one she dismissed,
/// or that died, isn't; nor is it while she is dead or a ghost.
#[tokio::test]
async fn a_saved_pet_returns_at_login_unless_dismissed_dead_or_its_hunter_a_ghost() {
    let (realm, mut a, h, _) = hunter_and_wolf().await;
    assert!(relogged_with_pet(&realm, &mut a).await, "out at logout");
    on(&map_of(&realm, h), move |m| {
        let pet = m.players[&h].owned.pet.unwrap();
        dismiss(m, pet);
    })
    .await;
    assert!(!relogged_with_pet(&realm, &mut a).await, "dismissed");
    // Called back, then killed.
    on(&map_of(&realm, h), move |m| {
        let r = m.players[&h].row.state.pets.current.clone().unwrap();
        m.players.get_mut(&h).unwrap().row.state.pets.active = true;
        let pet = hunter::from_record(m, h, &r).unwrap();
        hunter::die(m, pet);
    })
    .await;
    assert!(
        !relogged_with_pet(&realm, &mut a).await,
        "died before logout"
    );
    // Released, a ghost: not while she is one. (Dead and not released,
    // she would rise at login, as mangos has it, and her pet with her.)
    on(&map_of(&realm, h), move |m| {
        let p = m.players.get_mut(&h).unwrap();
        let pets = &mut p.row.state.pets;
        pets.current.as_mut().unwrap().dead = false;
        pets.active = true;
        death::kill(m, h);
    })
    .await;
    a.send(CMSG_REPOP_REQUEST {}).await;
    a.wait_for("the teleport to where the ghost goes", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: h,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
    assert!(
        !relogged_with_pet(&realm, &mut a).await,
        "its hunter a ghost"
    );
    let ghost = on(&map_of(&realm, h), move |m| m.players[&h].death).await;
    assert!(matches!(ghost, Death::Ghost { .. }), "{ghost:?}");
    // Risen, the pet is back.
    on(&map_of(&realm, h), move |m| {
        death::resurrect(m, h, 1.0);
    })
    .await;
    assert!(relogged_with_pet(&realm, &mut a).await, "risen, it is back");
}

/// Thistle Tea's "does not resummon the saved pet while the character is
/// dead" can't arise at a 1.12 login: a player who logged out dead and
/// unreleased rises at login (mangos's `ResurrectPlayer(0.5f)` in
/// `LoadFromDB`), and so does their pet.
#[tokio::test]
async fn one_who_logged_out_dead_and_unreleased_rises_at_login_with_the_pet() {
    let (realm, mut a, h, _) = hunter_and_wolf().await;
    on(&map_of(&realm, h), move |m| death::kill(m, h)).await;
    assert!(relogged_with_pet(&realm, &mut a).await);
    let death = on(&map_of(&realm, h), move |m| m.players[&h].death).await;
    assert_eq!(death, Death::Alive);
}

/// Cast `spell` on oneself and wait for it to be handled.
async fn call(c: &mut Client, spell: u32) {
    c.send(CMSG_CAST_SPELL {
        spell,
        targets: crate::game::map::spells::wire_targets(&Default::default()),
    })
    .await;
    handled(c).await;
}

/// The companion of `h` out now, if any, and its entry.
fn critter(m: &Map, h: Guid) -> Option<(Guid, u32)> {
    let c = m.players.get(&h)?.owned.critter?;
    m.creature(c).map(|x| (c, x.entry))
}

/// Thistle Tea's `MiniPetOwner` "toggles and replaces critters without
/// changing the combat slot": a companion is of its template's level and
/// npc flags, not its owner's; immune to players and creatures (0x300);
/// made by its spell, summoned by its owner; and passive. Another
/// companion replaces it, and calling the same one again sends it away.
/// The hunter's wolf, in her pet slot, stays out throughout.
#[tokio::test]
async fn a_companion_is_its_templates_immune_and_apart_from_the_pet() {
    let (realm, mut a, h, wolf) = hunter_and_wolf().await;
    let map = map_of(&realm, h);
    on(&map, move |m| {
        m.players
            .get_mut(&h)
            .unwrap()
            .spells
            .extend([CALL_CRITTER, CALL_OTHER]);
    })
    .await;
    call(&mut a, CALL_CRITTER).await;
    let first = on(&map, move |m| critter(m, h)).await;
    let (first, entry) = first.expect("a companion out");
    assert_eq!(entry, CRITTER);
    a.sees(first).await;
    call(&mut a, CALL_OTHER).await;
    let (second, look) = on(&map, move |m| {
        let (g, entry) = critter(m, h).expect("the other out");
        let c = m.creature(g).unwrap();
        let f = &c.fields;
        (
            (g, entry),
            (
                f.get_u32(UNIT_FIELD_LEVEL),
                f.get_u32(UNIT_NPC_FLAGS),
                f.get_u32(UNIT_FIELD_FLAGS) & 0x300,
                f.get_u32(UNIT_CREATED_BY_SPELL),
                f.get_guid(UNIT_FIELD_SUMMONEDBY),
                c.pet.as_deref().map(|s| s.react),
                m.creature(first).is_none(),
                m.players[&h].owned.pet,
            ),
        )
    })
    .await;
    assert_eq!(second.1, OTHER_CRITTER);
    assert_eq!(
        look,
        (
            1,
            2,
            0x300,
            CALL_OTHER,
            h,
            Some(crate::game::map::pets::REACT_PASSIVE),
            true,
            Some(wolf)
        ),
        "level, npc flags, immunity, spell, owner, stance, the first gone, the pet slot"
    );
    a.forgets(first).await;
    // The other's spell again: it goes, and nothing replaces it.
    call(&mut a, CALL_OTHER).await;
    let (none, wolf_out) = on(&map, move |m| {
        (
            critter(m, h).is_none() && m.creature(second.0).is_none(),
            m.players[&h].owned.pet == Some(wolf) && m.creature(wolf).is_some(),
        )
    })
    .await;
    assert!(none, "toggled away");
    assert!(wolf_out, "the pet slot untouched");
}

/// Thistle Tea's `MiniPetOwner` "death and world transitions tear down
/// critters" and "missing owner presence tears down a live follower": a
/// companion goes when its owner dies, and when they leave the map
/// (logging out among the ways).
#[tokio::test]
async fn a_companion_goes_with_its_owners_death_and_departure() {
    let (realm, mut a, h, _) = hunter_and_wolf().await;
    let map = map_of(&realm, h);
    on(&map, move |m| {
        m.players.get_mut(&h).unwrap().spells.push(CALL_CRITTER);
    })
    .await;
    call(&mut a, CALL_CRITTER).await;
    let (c, _) = on(&map, move |m| critter(m, h)).await.expect("out");
    a.sees(c).await;
    on(&map, move |m| death::kill(m, h)).await;
    a.forgets(c).await;
    let gone = on(&map, move |m| m.creature(c).is_none()).await;
    assert!(gone, "its owner died");
    on(&map, move |m| death::resurrect(m, h, 1.0)).await;
    call(&mut a, CALL_CRITTER).await;
    let (c, _) = on(&map, move |m| critter(m, h)).await.expect("out again");
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    let gone = on(&map, move |m| m.creature(c).is_none()).await;
    assert!(gone, "its owner left");
}
