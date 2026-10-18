//! Taming the Beast: a Dwarf hunter's level-10 chain against the real data
//! when it is here. Each rod's channel, run its course, charms its beast
//! and completes the quest; the tame's Despawn Caster sends the beast
//! away; one charm at a time; the third quest's reward teaches Tame Beast,
//! and Training the Beast's teaches Beast Training, Feed Pet and Revive
//! Pet. An event quest waits on its event; a hunter learns to teach the
//! passives their pet came with.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::npc::{QUEST_SPECIAL_EXPLORATION_OR_EVENT, Quest};
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::{Map, MapHandle, MapKey, auras, pets, quests, spells};
use crate::game::tests_pets::at;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

const GRIF: u32 = 1231;
const BELIA: u32 = 10090;
const CRAG_BOAR: u32 = 1126;
const SNOW_LEOPARD: u32 = 1201;
const ICE_CLAW_BEAR: u32 = 1196;

const BOAR_QUEST: u32 = 6064;
const LEOPARD_QUEST: u32 = 6084;
const BEAR_QUEST: u32 = 6085;
const TRAINING_QUEST: u32 = 6086;

const BOAR_ROD: u32 = 15911;
const LEOPARD_ROD: u32 = 15913;
const BEAR_ROD: u32 = 15908;

const BOAR_CHANNEL: u32 = 19674;
const BOAR_TAME: u32 = 19677;
const BEAR_CHANNEL: u32 = 19548;

const TAME_BEAST: u32 = 1515;
const CALL_PET: u32 = 883;
const DISMISS_PET: u32 = 2641;
const TAMING_LESSON: u32 = 23356;
const BEAST_TRAINING: u32 = 5149;
const FEED_PET: u32 = 6991;
const REVIVE_PET: u32 = 982;
const TRAINING_LESSON: u32 = 23357;

/// A creature of `entry` at its template's level, `yards` before `g`.
fn spawn(m: &mut Map, g: Guid, entry: u32, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let t = data.creature_template(entry).cloned().expect("template");
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &t,
        t.level_min.max(1),
        at,
        o + std::f32::consts::PI,
    )
    .with_base();
    m.add_creature(c)
}

async fn until(map: &MapHandle, what: impl Fn(&Map) -> bool + Send + Clone + 'static) -> bool {
    for _ in 0..300 {
        let w = what.clone();
        if map.query(move |m| w(m)).await.unwrap_or(false) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

fn logged(m: &Map, g: Guid, quest: u32) -> bool {
    m.players[&g].journal.has(quest)
}

fn done(m: &Map, g: Guid, quest: u32) -> bool {
    let p = &m.players[&g];
    let q = m.realm.data.npc.quests.get(&quest).expect("quest");
    p.journal
        .slot_of(quest)
        .is_some_and(|s| quests::complete(p, q, &p.journal.log[s]))
}

fn rewarded(m: &Map, g: Guid, quest: u32) -> bool {
    m.players[&g].journal.rewarded.contains(&quest)
}

fn rod_at(m: &Map, g: Guid, rod: u32) -> Option<(u8, u8)> {
    let p = &m.players[&g];
    p.items
        .iter()
        .find(|i| i.entry == rod)
        .map(|i| (i.bag, i.slot))
}

fn channelling(m: &Map, g: Guid, spell: u32) -> bool {
    m.players[&g]
        .caster
        .casting
        .as_ref()
        .is_some_and(|c| c.spell == spell && c.channel_until.is_some())
}

/// The channel under way runs its course now.
fn run_out(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m
        .players
        .get_mut(&g)
        .and_then(|p| p.caster.casting.as_mut())
    {
        c.ends = now;
    }
    spells::update(m);
}

fn charmed_by(m: &Map, beast: Guid, g: Guid) -> bool {
    m.creature(beast).is_some_and(|c| {
        c.owner == Some(g)
            && c.pet
                .as_ref()
                .is_some_and(|s| s.kind == pets::Kind::Charmed)
    }) && m.players[&g].owned.charm == Some(beast)
}

async fn accept(c: &mut Client, map: &MapHandle, g: Guid, giver: Guid, quest: u32) {
    c.send(CMSG_QUESTGIVER_ACCEPT_QUEST {
        guid: giver,
        quest_id: quest,
    })
    .await;
    assert!(
        until(map, move |m| logged(m, g, quest)).await,
        "{quest} taken"
    );
}

async fn turn_in(c: &mut Client, map: &MapHandle, g: Guid, giver: Guid, quest: u32) {
    c.send(CMSG_QUESTGIVER_CHOOSE_REWARD {
        guid: giver,
        quest_id: quest,
        reward: 0,
    })
    .await;
    assert!(
        until(map, move |m| rewarded(m, g, quest)).await,
        "{quest} rewarded"
    );
}

/// Use `rod` at `beast` (the last rod's global cooldown run out) and let
/// its channel run its course.
async fn tame_with(c: &mut Client, map: &MapHandle, g: Guid, rod: u32, channel: u32, beast: Guid) {
    let (bag, slot) = map
        .query(move |m| {
            m.players.get_mut(&g).unwrap().caster.gcd_until = None;
            rod_at(m, g, rod)
        })
        .await
        .flatten()
        .expect("the quest's rod");
    c.send(CMSG_USE_ITEM {
        bag_index: bag,
        bag_slot: slot,
        spell_index: 0,
        targets: at(beast),
    })
    .await;
    assert!(
        until(map, move |m| channelling(m, g, channel)).await,
        "the rod's channel"
    );
    map.query(move |m| run_out(m, g)).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn a_dwarf_hunter_tames_with_the_rods_and_learns_tame_beast() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(crate::game::data::load::load(DB.as_path()));
    let mut c = Client::connect(&realm, "HUNTER");
    let g = c.enter("Huntra", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let grif = map
        .query(move |m| {
            let data = m.realm.data.clone();
            let p = m.players.get_mut(&g).unwrap();
            crate::game::map::stats::set_level(p, &data, 10);
            spawn(m, g, GRIF, 3.0)
        })
        .await
        .unwrap();

    // The boar: the rod's channel run out charms it and does the quest.
    accept(&mut c, &map, g, grif, BOAR_QUEST).await;
    let waiting = map
        .query(move |m| (rod_at(m, g, BOAR_ROD).is_some(), done(m, g, BOAR_QUEST)))
        .await
        .unwrap();
    assert_eq!(
        waiting,
        (true, false),
        "the rod, and the quest waits on its event"
    );
    let boar = map
        .query(move |m| spawn(m, g, CRAG_BOAR, 8.0))
        .await
        .unwrap();
    tame_with(&mut c, &map, g, BOAR_ROD, BOAR_CHANNEL, boar).await;
    c.wait_for("SMSG_QUESTUPDATE_COMPLETE", |m| match m {
        ServerOpcodeMessage::SMSG_QUESTUPDATE_COMPLETE(q) if q.quest_id == BOAR_QUEST => Some(()),
        _ => None,
    })
    .await;
    let tamed = map
        .query(move |m| (charmed_by(m, boar, g), done(m, g, BOAR_QUEST)))
        .await
        .unwrap();
    assert_eq!(tamed, (true, true), "the boar charmed, the quest done");
    turn_in(&mut c, &map, g, grif, BOAR_QUEST).await;

    // Ten minutes on, the tame's Despawn Caster: the boar goes, quietly.
    let gone = map
        .query(move |m| {
            let now = m.now;
            if let Some(a) = auras::auras_mut(m, boar) {
                for h in a.holders.iter_mut().filter(|h| h.spell == BOAR_TAME) {
                    for x in h.effects.iter_mut().filter(|x| x.next_tick.is_some()) {
                        x.next_tick = Some(now);
                    }
                }
            }
            auras::tick_due(m, boar, BOAR_TAME, g);
            (
                m.creature(boar).is_none_or(|c| !c.alive()),
                m.players[&g].owned.charm,
            )
        })
        .await
        .unwrap();
    assert_eq!(gone, (true, None), "the boar gone, not at its master");

    // The leopard, kept.
    accept(&mut c, &map, g, grif, LEOPARD_QUEST).await;
    let leopard = map
        .query(move |m| spawn(m, g, SNOW_LEOPARD, 8.0))
        .await
        .unwrap();
    tame_with(&mut c, &map, g, LEOPARD_ROD, 19687, leopard).await;
    assert!(
        until(&map, move |m| charmed_by(m, leopard, g)
            && done(m, g, LEOPARD_QUEST))
        .await,
        "the leopard charmed, the quest done"
    );
    turn_in(&mut c, &map, g, grif, LEOPARD_QUEST).await;

    // The bear, while the leopard is still charmed: one charm at a time.
    accept(&mut c, &map, g, grif, BEAR_QUEST).await;
    let bear = map
        .query(move |m| spawn(m, g, ICE_CLAW_BEAR, 8.0))
        .await
        .unwrap();
    tame_with(&mut c, &map, g, BEAR_ROD, BEAR_CHANNEL, bear).await;
    let refused = map
        .query(move |m| (charmed_by(m, bear, g), done(m, g, BEAR_QUEST)))
        .await
        .unwrap();
    assert_eq!(refused, (false, false), "a second charm is refused");

    // The leopard let go turns on its master; out of the way, the bear
    // again. Its tame's quest credit is aimed at the bear: the hunter's.
    let turned = map
        .query(move |m| {
            pets::dismiss(m, leopard);
            let turned = m.creature(leopard).is_some_and(|c| c.owner.is_none())
                && m.players[&g].owned.charm.is_none();
            m.remove_creature(leopard);
            m.remove_creature(bear);
            turned
        })
        .await
        .unwrap();
    assert!(turned, "the leopard let go");
    let bear = map
        .query(move |m| spawn(m, g, ICE_CLAW_BEAR, 8.0))
        .await
        .unwrap();
    tame_with(&mut c, &map, g, BEAR_ROD, BEAR_CHANNEL, bear).await;
    assert!(
        until(&map, move |m| charmed_by(m, bear, g)
            && done(m, g, BEAR_QUEST))
        .await,
        "the bear charmed, the quest done"
    );

    // Its reward casts Tame Beast's lesson: Tame Beast, Call Pet and
    // Dismiss Pet; the Taming Lesson it shows is not learned.
    turn_in(&mut c, &map, g, grif, BEAR_QUEST).await;
    let known = move |m: &Map, s: u32| m.players[&g].spells.contains(&s);
    let learned = map
        .query(move |m| {
            (
                known(m, TAME_BEAST) && known(m, CALL_PET) && known(m, DISMISS_PET),
                known(m, TAMING_LESSON),
            )
        })
        .await
        .unwrap();
    assert_eq!(learned, (true, false), "Tame Beast learned from the quest");

    // Training the Beast, ended at the pet trainer in Ironforge.
    accept(&mut c, &map, g, grif, TRAINING_QUEST).await;
    let belia = map.query(move |m| spawn(m, g, BELIA, 3.0)).await.unwrap();
    turn_in(&mut c, &map, g, belia, TRAINING_QUEST).await;
    let learned = map
        .query(move |m| {
            (
                known(m, BEAST_TRAINING) && known(m, FEED_PET) && known(m, REVIVE_PET),
                known(m, TRAINING_LESSON),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        learned,
        (true, false),
        "Beast Training learned from the quest"
    );
}

#[test]
fn an_event_quest_waits_on_its_event() {
    let q = Quest {
        special_flags: QUEST_SPECIAL_EXPLORATION_OR_EVENT,
        ..Default::default()
    };
    assert!(q.needs_event() && q.has_objectives());
    assert!(!Quest::default().needs_event());
    assert_eq!(pets::rods::tame_of(19548), Some(19597));
    assert_eq!(pets::rods::tame_of(1515), None);
}

#[tokio::test]
async fn a_hunter_learns_to_teach_the_passives_their_pet_came_with() {
    use crate::game::data::pets::PetAbility;
    use crate::game::tests_pets as tp;
    const THICK_HIDE: u32 = 9201;
    const TRAIN_THICK_HIDE: u32 = 9202;
    let mut d = tp::data();
    // A boar's family passive, and the hunter's spell that teaches it.
    d.pets.abilities.get_mut(&211).unwrap().push(PetAbility {
        spell: THICK_HIDE,
        skill: 211,
        acquire: 2,
        ..Default::default()
    });
    d.pets.teaching.insert(THICK_HIDE, TRAIN_THICK_HIDE);
    let none = Default::default();
    let mut hide = tp::effect(6, 5, 1);
    hide.aura = 22;
    let mut passive = tp::spell(THICK_HIDE, "Thick Hide", [hide, none, none]);
    passive.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    let mut teach = tp::effect(36, 0, 5);
    teach.trigger_spell = THICK_HIDE;
    let recipe = tp::spell(TRAIN_THICK_HIDE, "Thick Hide", [teach, none, none]);
    d.spells.spells.insert(THICK_HIDE, passive);
    d.spells.spells.insert(TRAIN_THICK_HIDE, recipe);
    let realm = harness::realm_with(d);
    let (mut h, g, map) = tp::enter(&realm, "H", "Hunter", Race::Dwarf, Class::Hunter).await;
    tp::learn(&map, g, &[tp::TAME_BEAST]);
    let boar = tp::place(&map, g, tp::BOAR, 0x70_0001, 4.0).await;
    h.sees(boar).await;
    tp::cast(&h, tp::TAME_BEAST, Some(boar)).await;
    assert!(!tp::pet_bar(&mut h).await.is_zero(), "the tamed boar");
    let known = map
        .query(move |m| {
            let pet = m.players[&g].owned.pet.unwrap();
            (
                pets::state(m, pet).is_some_and(|s| s.spells.contains(&THICK_HIDE)),
                m.players[&g].spells.contains(&TRAIN_THICK_HIDE),
            )
        })
        .await
        .unwrap();
    assert_eq!(known, (true, true), "its passive, and its master's recipe");
}
