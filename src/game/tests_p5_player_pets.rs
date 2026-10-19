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
        movement_counter: a.teleport_order(),
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

/// A stable master, of no side.
const STABLE_MASTER: u32 = 963_102;

fn stable_world() -> GameData {
    let mut d = data();
    d.creature_templates.insert(
        STABLE_MASTER,
        CreatureTemplate {
            entry: STABLE_MASTER,
            name: "Stable Master".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 30,
            level_max: 30,
            faction: 35,
            unit_class: 1,
            creature_type: 7,
            health_multiplier: 1.0,
            damage_multiplier: 1.0,
            base_attack_time: 2000,
            health_max: 500,
            npc_flags: crate::game::map::pets::NPC_FLAG_STABLEMASTER,
            ..Default::default()
        },
    );
    d
}

/// What `CMSG_BUY_STABLE_SLOT` of `npc` comes to.
async fn buy_stall(c: &mut Client, npc: Guid) -> StableResult {
    c.clear();
    c.send(CMSG_BUY_STABLE_SLOT { npc }).await;
    crate::game::tests_pets::stable_result(c).await
}

/// Thistle Tea's `PetStable` "requires a ready living hunter and a nearby
/// live stable master" and "revalidates distance and world on every
/// request": a stall is bought only by a living hunter from a living stable
/// master within reach; any other is told the stable's error (mangos's
/// `CheckStableMaster`, `GetNPCIfCanInteractWith`'s hunters alone).
#[tokio::test]
async fn a_stall_is_bought_by_a_living_hunter_from_a_master_in_reach() {
    use crate::game::map::creature::Life;
    let realm = harness::realm_with(stable_world());
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let map = map_of(&realm, h);
    let master = on(&map, move |m| {
        m.players.get_mut(&h).unwrap().set_money(100_000);
        crate::game::tests_p3_combat::spawn(m, h, STABLE_MASTER, 30, 2.0)
    })
    .await;
    a.sees(master).await;
    let set = move |m: &mut Map, f: &dyn Fn(&mut crate::game::map::creature::Creature)| {
        f(m.creature_mut(master).unwrap());
    };
    let later = std::time::Duration::from_secs(60);
    on(&map, move |m| {
        let at = m.now + later;
        set(m, &|c| {
            c.life = Life::Dead {
                corpse_until: at,
                respawn_at: at,
            }
        })
    })
    .await;
    assert_eq!(
        buy_stall(&mut a, master).await,
        StableResult::ErrStable,
        "dead"
    );
    on(&map, move |m| {
        set(m, &|c| {
            c.life = Life::Alive;
            c.fields.set_u32(UNIT_NPC_FLAGS, 0);
        })
    })
    .await;
    assert_eq!(
        buy_stall(&mut a, master).await,
        StableResult::ErrStable,
        "unflagged"
    );
    on(&map, move |m| {
        let at = m.players[&h].position();
        set(m, &|c| {
            c.fields.set_u32(
                UNIT_NPC_FLAGS,
                crate::game::map::pets::NPC_FLAG_STABLEMASTER,
            );
            c.position = Vector3d {
                x: at.x + 8.0,
                ..at
            };
        })
    })
    .await;
    assert_eq!(
        buy_stall(&mut a, master).await,
        StableResult::ErrStable,
        "out of reach"
    );
    assert_eq!(
        buy_stall(&mut a, h).await,
        StableResult::ErrStable,
        "herself"
    );
    on(&map, move |m| {
        let at = m.players[&h].position();
        set(m, &|c| {
            c.position = Vector3d {
                x: at.x + 2.0,
                ..at
            }
        });
        death::kill(m, h);
    })
    .await;
    assert_eq!(
        buy_stall(&mut a, master).await,
        StableResult::ErrStable,
        "a dead hunter"
    );
    on(&map, move |m| death::resurrect(m, h, 1.0)).await;
    assert_eq!(
        buy_stall(&mut a, master).await,
        StableResult::SuccessBuySlot,
        "alive, in reach"
    );
    // A warlock beside it.
    let mut w = Client::connect(&realm, "W");
    let gw = w.enter("Warlock", Race::Human, Class::Warlock).await;
    let wmap = map_of(&realm, gw);
    let master = on(&wmap, move |m| {
        m.players.get_mut(&gw).unwrap().set_money(100_000);
        crate::game::tests_p3_combat::spawn(m, gw, STABLE_MASTER, 30, 2.0)
    })
    .await;
    w.sees(master).await;
    assert_eq!(
        buy_stall(&mut w, master).await,
        StableResult::ErrStable,
        "not a hunter"
    );
}

/// Thistle Tea's `Pets.abandon` "removes only the current bond, process,
/// metadata, and client controls": abandoning another's guid does nothing;
/// abandoning her pet removes it, its record and the bar, and leaves the
/// stable as it was.
#[tokio::test]
async fn an_abandoned_pet_goes_with_its_bar_and_record_not_the_stable() {
    let (realm, mut a, h, wolf) = hunter_and_wolf().await;
    let map = map_of(&realm, h);
    let stabled = on(&map, move |m| {
        let p = m.players.get_mut(&h).unwrap();
        let mut r = p.row.state.pets.current.clone().unwrap();
        r.number += 100;
        r.name = "Stabled".into();
        p.row.state.pets.stable_slots = 1;
        p.row.state.pets.stable.push((1, r));
        p.row.state.pets.stable.clone()
    })
    .await;
    a.send(CMSG_PET_ABANDON {
        pet: crate::game::guid::player(4242),
    })
    .await;
    handled(&mut a).await;
    let still = on(&map, move |m| m.creature(wolf).is_some()).await;
    assert!(still, "another's guid");
    a.clear();
    a.send(CMSG_PET_ABANDON { pet: wolf }).await;
    assert_eq!(
        crate::game::tests_pets::pet_bar(&mut a).await,
        Guid::zero(),
        "the bar gone"
    );
    let after = on(&map, move |m| {
        let p = &m.players[&h];
        (
            m.creature(wolf).is_none(),
            p.owned.pet,
            p.row.state.pets.current.is_none(),
            p.row.state.pets.active,
            p.row.state.pets.stable.clone(),
        )
    })
    .await;
    assert_eq!(after, (true, None, true, false, stabled));
}

/// A pet trainer whose menu offers to untrain a pet.
const PET_TRAINER: u32 = 963_103;
const PET_TRAINER_MENU: u32 = 963_103;
/// Two abilities the wolf was trained in, and its family's own.
const TRAINED: [u32; 2] = [95_600, 95_601];
const FAMILY: u32 = 95_602;

fn untraining_world() -> GameData {
    use crate::game::data::npc::{GossipMenu, GossipOption};
    let mut d = data();
    d.creature_templates.insert(
        PET_TRAINER,
        CreatureTemplate {
            entry: PET_TRAINER,
            name: "Pet Trainer".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 30,
            level_max: 30,
            faction: 35,
            unit_class: 1,
            creature_type: 7,
            health_multiplier: 1.0,
            damage_multiplier: 1.0,
            base_attack_time: 2000,
            health_max: 500,
            gossip_menu_id: PET_TRAINER_MENU,
            npc_flags: crate::game::map::npc::NPC_FLAG_GOSSIP
                | crate::game::map::npc::NPC_FLAG_TRAINER,
            trainer_type: 3,
            ..Default::default()
        },
    );
    d.npc.gossip_menus.insert(
        PET_TRAINER_MENU,
        GossipMenu {
            texts: vec![(100, 0)],
            options: vec![GossipOption {
                id: 0,
                kind: 17,
                text: "I wish to untrain my pet.".into(),
                ..Default::default()
            }],
        },
    );
    d
}

/// The offer `c` is told on choosing to untrain at `npc`: whether the
/// window closed, and the pet and price.
async fn offered(c: &mut Client, npc: Guid) -> (bool, Option<(Guid, u32)>) {
    c.clear();
    c.send(CMSG_GOSSIP_HELLO { guid: npc }).await;
    c.send(CMSG_GOSSIP_SELECT_OPTION {
        guid: npc,
        gossip_list_id: 0,
        unknown: None,
    })
    .await;
    handled(c).await;
    let closed = c
        .gets(std::time::Duration::from_millis(1), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GOSSIP_COMPLETE).then_some(())
        })
        .await
        .is_some();
    let offer = c
        .gets(std::time::Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_PET_UNLEARN_CONFIRM(o) => Some((o.pet, o.talent_reset_cost)),
            _ => None,
        })
        .await;
    (closed, offer)
}

/// Thistle Tea's `PetUntraining` "quotes the current pet without charging
/// or clearing abilities", "requires an eligible owner and a nearby live
/// pet trainer", "charges once, projects the empty bar, and retains reset
/// history on suspension" and "leaves abilities and payment intact when
/// funds are insufficient": the trainer's option closes the window and
/// quotes the pet's price, charging nothing; out of reach, or dead, it
/// quotes nothing. Short of the price, the untraining is refused and the
/// pet keeps its abilities; paid, they go but its family's, its training
/// points come back, and the next price is higher.
#[tokio::test]
async fn a_pet_is_untrained_for_its_price_at_a_trainer_in_reach() {
    use crate::game::map::creature::Life;
    let realm = harness::realm_with(untraining_world());
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let map = map_of(&realm, h);
    let (wolf, trainer) = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        p.set_money(10_000);
        let at = p.position();
        let wolf = hunter::spawn(m, h, WOLF, Kind::Hunter, 20, at, 0.0);
        m.players.get_mut(&h).unwrap().owned.pet = Some(wolf);
        let s = m.creature_mut(wolf).unwrap().pet.as_deref_mut().unwrap();
        s.family_spells = vec![FAMILY];
        s.spells = vec![FAMILY, TRAINED[0], TRAINED[1]];
        s.training_points = 0;
        let trainer = crate::game::tests_p3_combat::spawn(m, h, PET_TRAINER, 30, 2.0);
        let f = &mut m.creature_mut(trainer).unwrap().fields;
        f.set_u32(
            UNIT_NPC_FLAGS,
            crate::game::map::npc::NPC_FLAG_GOSSIP | crate::game::map::npc::NPC_FLAG_TRAINER,
        );
        (wolf, trainer)
    })
    .await;
    a.sees(trainer).await;
    let spells = move |m: &mut Map| {
        let c = m.creature(wolf).unwrap();
        let s = c.pet.as_deref().unwrap();
        (s.spells.clone(), m.players[&h].money())
    };
    let (closed, offer) = offered(&mut a, trainer).await;
    assert!(closed, "the window closes");
    let (pet, price) = offer.expect("a price");
    assert_eq!(pet, wolf);
    assert_eq!(
        on(&map, spells).await,
        (vec![FAMILY, TRAINED[0], TRAINED[1]], 10_000),
        "nothing charged, nothing forgotten"
    );
    let at = |yards: f32| {
        move |m: &mut Map| {
            let at = m.players[&h].position();
            m.creature_mut(trainer).unwrap().position = Vector3d {
                x: at.x + yards,
                ..at
            };
        }
    };
    on(&map, at(10.0)).await;
    assert_eq!(offered(&mut a, trainer).await.1, None, "out of reach");
    on(&map, at(2.0)).await;
    on(&map, move |m| {
        let later = m.now + std::time::Duration::from_secs(60);
        m.creature_mut(trainer).unwrap().life = Life::Dead {
            corpse_until: later,
            respawn_at: later,
        };
    })
    .await;
    assert_eq!(offered(&mut a, trainer).await.1, None, "dead");
    // Short of the price.
    on(&map, move |m| {
        m.creature_mut(trainer).unwrap().life = Life::Alive;
        m.players.get_mut(&h).unwrap().set_money(price - 1);
    })
    .await;
    a.clear();
    a.send(CMSG_PET_UNLEARN { pet: wolf }).await;
    let refused = a
        .wait_for("SMSG_BUY_FAILED", |m| match m {
            ServerOpcodeMessage::SMSG_BUY_FAILED(f) => Some(f.result),
            _ => None,
        })
        .await;
    assert_eq!(refused, BuyResult::NotEnoughtMoney);
    let (kept, _) = on(&map, spells).await;
    assert_eq!(kept, vec![FAMILY, TRAINED[0], TRAINED[1]], "kept");
    // Paid: another's pet first, then hers.
    on(&map, move |m| {
        m.players.get_mut(&h).unwrap().set_money(10_000)
    })
    .await;
    a.send(CMSG_PET_UNLEARN {
        pet: crate::game::guid::player(4242),
    })
    .await;
    handled(&mut a).await;
    assert_eq!(on(&map, spells).await.1, 10_000, "not her pet");
    a.send(CMSG_PET_UNLEARN { pet: wolf }).await;
    handled(&mut a).await;
    let (left, money) = on(&map, spells).await;
    assert_eq!(
        (left, money),
        (vec![FAMILY], 10_000 - price),
        "untrained, once paid"
    );
    // Taught again, it has something to forget.
    on(&map, move |m| {
        let s = m.creature_mut(wolf).unwrap().pet.as_deref_mut().unwrap();
        s.spells.push(TRAINED[0]);
    })
    .await;
    let (_, again) = offered(&mut a, trainer).await;
    assert!(again.expect("a price").1 > price, "dearer the next time");
}
