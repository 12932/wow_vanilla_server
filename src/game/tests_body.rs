//! The body: eating and drinking seat the player, as mangos-classic's
//! `_AddSpellAuraHolder` sits a unit an aura that standing ends goes on,
//! and standing up, moving, jumping or turning ends the meal; and a
//! corpse its player rises from leaves bones where it lay, for
//! `[players] bones_hours`, as mangos's `ConvertCorpseForPlayer` makes
//! them.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage as C, ServerOpcodeMessage as S};
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::death::{self, Death};
use crate::game::map::{Map, MapHandle, auras, bones, grid};

/// Food (Tough Jerky's), a buff food (Spiced Wolf Meat's) and its Well
/// Fed, as the real rows have them: the meals' aura interrupt flags
/// 0x40080 (standing, under water) and attributes 0x18000100.
const FOOD: u32 = 433;
const BUFF_FOOD: u32 = 5004;
const WELL_FED: u32 = 19705;
const JERKY: u32 = 117;
const WOLF_MEAT: u32 = 2680;

fn meal(id: u32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: 84,
        base_points: 16,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: "Food".into(),
        attributes: [0x1800_0100, 0, 0, 0, 0],
        interrupt_flags: 1,
        aura_interrupt_flags: 0x4_0080,
        equipped_item_class: -1,
        spell_icon: 358,
        effects,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.spells.spells.insert(FOOD, meal(FOOD));
    let mut buff = meal(BUFF_FOOD);
    buff.effects[1] = SpellEffect {
        effect: 6,
        aura: auras::AURA_PERIODIC_TRIGGER_SPELL,
        amplitude: 10_000,
        trigger_spell: WELL_FED,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(BUFF_FOOD, buff);
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: 29,
        base_points: 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(
        WELL_FED,
        SpellTemplate {
            id: WELL_FED,
            name: "Well Fed".into(),
            attributes: [0x0800_0100, 0, 0, 0, 0],
            equipped_item_class: -1,
            spell_icon: 59,
            effects,
            ..Default::default()
        },
    );
    for (entry, spell) in [(JERKY, FOOD), (WOLF_MEAT, BUFF_FOOD)] {
        let mut item = crate::game::data::ItemTemplate {
            entry,
            name: "Meal".into(),
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        };
        item.spells[0] = ItemSpell {
            id: spell,
            trigger: 0,
            charges: -1,
            cooldown: -1,
            ..Default::default()
        };
        d.items.insert(entry, item);
    }
    d
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

async fn enter(realm: &Arc<Realm>, account: &str) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c
        .enter(
            &format!("Body{}", account.to_lowercase()),
            Race::Human,
            Class::Warrior,
        )
        .await;
    let map = realm.registry.online(g).unwrap().map();
    (c, g, map)
}

/// Wait until the map has handled everything sent before.
async fn settled(c: &mut Client) {
    c.send(CMSG_QUERY_TIME {}).await;
    c.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, S::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
}

async fn posture(c: &mut Client, state: UnitStandState) {
    c.wait_for("SMSG_STANDSTATE_UPDATE", |m| match m {
        S::SMSG_STANDSTATE_UPDATE(u) if u.state == state => Some(()),
        _ => None,
    })
    .await;
}

/// Eat one of `item` from the bags, over the protocol.
async fn eat(c: &mut Client, map: &MapHandle, item: u32) {
    let g = c.guid;
    let (bag, slot) = on(map, move |m| {
        if m.players[&g].count_of(item) == 0 {
            crate::game::map::inventory::add_item(m, g, item, 5, false).unwrap();
        }
        let i = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == item)
            .unwrap();
        (i.bag, i.slot)
    })
    .await;
    c.send(CMSG_USE_ITEM {
        bag_index: bag,
        bag_slot: slot,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty(),
        },
    })
    .await;
}

/// `g`'s posture, and whether it has each of `spells`.
async fn state(map: &MapHandle, g: Guid, spells: &'static [u32]) -> (u8, Vec<bool>) {
    on(map, move |m| {
        let p = &m.players[&g];
        let has = spells.iter().map(|s| p.auras.has_spell(*s)).collect();
        (p.stand_state(), has)
    })
    .await
}

fn moved(flags: MovementInfo_MovementFlags, at: Vector3d, orientation: f32) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: 0,
        position: at,
        orientation,
        fall_time: 0.0,
    }
}

/// Right-clicking food sits the eater down, their client told; standing
/// up, moving, jumping and turning on the spot each end the meal and
/// stand them, as mangos's movement stands a sitter (`SetStandState`
/// ending what standing ends).
#[tokio::test]
async fn food_sits_the_eater_and_standing_moving_jumping_or_turning_ends_it() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = enter(&realm, "A").await;
    let at = on(&map, move |m| m.players[&g].position()).await;
    let ends: [(&str, Vec<C>); 4] = [
        (
            "standing",
            vec![
                CMSG_STANDSTATECHANGE {
                    animation_state: UnitStandState::Stand,
                }
                .into(),
            ],
        ),
        (
            "moving",
            vec![
                MSG_MOVE_START_FORWARD_Client {
                    info: moved(
                        MovementInfo_MovementFlags::new_forward(),
                        Vector3d {
                            x: at.x + 0.5,
                            ..at
                        },
                        0.0,
                    ),
                }
                .into(),
                MSG_MOVE_STOP_Client {
                    info: moved(MovementInfo_MovementFlags::empty(), at, 0.0),
                }
                .into(),
            ],
        ),
        (
            "jumping",
            vec![
                MSG_MOVE_JUMP_Client {
                    info: moved(
                        MovementInfo_MovementFlags::empty().set_jumping(Default::default()),
                        at,
                        0.0,
                    ),
                }
                .into(),
                MSG_MOVE_FALL_LAND_Client {
                    info: moved(MovementInfo_MovementFlags::empty(), at, 0.0),
                }
                .into(),
            ],
        ),
        (
            "turning",
            vec![
                MSG_MOVE_START_TURN_LEFT_Client {
                    info: moved(MovementInfo_MovementFlags::new_turn_left(), at, 0.0),
                }
                .into(),
                MSG_MOVE_STOP_TURN_Client {
                    info: moved(MovementInfo_MovementFlags::empty(), at, 0.5),
                }
                .into(),
            ],
        ),
    ];
    for (how, packets) in ends {
        a.clear();
        eat(&mut a, &map, JERKY).await;
        posture(&mut a, UnitStandState::Sit).await;
        assert_eq!(
            state(&map, g, &[FOOD]).await,
            (1, vec![true]),
            "seated and eating, before {how}"
        );
        for p in packets {
            a.send(p).await;
        }
        posture(&mut a, UnitStandState::Stand).await;
        settled(&mut a).await;
        assert_eq!(
            state(&map, g, &[FOOD]).await,
            (0, vec![false]),
            "{how} ends the meal"
        );
    }
}

/// A buff food's Well Fed, which comes while the player eats (its
/// periodic trigger), neither stands them nor goes when they stand: only
/// the meal does.
#[tokio::test]
async fn a_buff_foods_well_fed_outlasts_the_meal_standing_ends() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = enter(&realm, "A").await;
    eat(&mut a, &map, WOLF_MEAT).await;
    posture(&mut a, UnitStandState::Sit).await;
    let fed = on(&map, move |m| {
        m.now += Duration::from_millis(10_001);
        auras::update(m);
        let p = &m.players[&g];
        (
            p.stand_state(),
            p.auras.has_spell(BUFF_FOOD),
            p.auras.has_spell(WELL_FED),
        )
    })
    .await;
    assert_eq!(
        fed,
        (1, true, true),
        "still seated and eating, and well fed"
    );
    a.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Stand,
    })
    .await;
    settled(&mut a).await;
    assert_eq!(
        state(&map, g, &[BUFF_FOOD, WELL_FED]).await,
        (0, vec![false, true]),
        "the meal ends, Well Fed stays"
    );
    // Walking off doesn't end it either.
    let at = on(&map, move |m| m.players[&g].position()).await;
    a.send(MSG_MOVE_START_FORWARD_Client {
        info: moved(
            MovementInfo_MovementFlags::new_forward(),
            Vector3d {
                x: at.x + 0.5,
                ..at
            },
            0.0,
        ),
    })
    .await;
    settled(&mut a).await;
    assert_eq!(state(&map, g, &[WELL_FED]).await.1, vec![true]);
}

/// Move player `g` `dx` yards east of where it is, as the map sees it.
fn put(m: &mut Map, g: Guid, dx: f32) {
    let Some(p) = m.players.get_mut(&g) else {
        return;
    };
    p.movement.position.x += dx;
    p.move_check = None;
    let at = p.movement.position;
    let (old, new) = (p.cell, grid::cell_of(at.x, at.y));
    p.cell = new;
    m.grid.relocate(g, old, new);
    m.visibility_dirty.insert(g);
}

/// Kill `c`'s player and release its spirit, where it stands (the test
/// world has no graveyard), acknowledging the teleport.
async fn die_and_release(c: &mut Client, map: &MapHandle) {
    let g = c.guid;
    on(map, move |m| {
        m.players
            .get_mut(&g)
            .unwrap()
            .fields
            .set_u32(UNIT_FIELD_HEALTH, 0);
        death::kill(m, g);
    })
    .await;
    c.send(CMSG_REPOP_REQUEST {}).await;
    c.wait_for("the teleport to the graveyard", |m| {
        matches!(m, S::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: g,
        movement_counter: 0,
        time: Default::default(),
    })
    .await;
}

/// `owner`'s bones on the map: their GUID, place, flags, owner and worn
/// gear, once there are some.
async fn bones_of(map: &MapHandle, owner: Guid) -> (Guid, Vector3d, u32, Guid, u32) {
    for _ in 0..400 {
        let found = on(map, move |m| {
            m.corpses
                .values()
                .find(|c| c.owner == owner && c.is_bones())
                .map(|c| {
                    let worn = (0..19u16)
                        .map(|i| c.fields.get_u32(CORPSE_FIELD_ITEM + i))
                        .fold(0, |a, b| a | b);
                    (
                        c.guid,
                        c.position,
                        c.fields.get_u32(CORPSE_FIELD_FLAGS),
                        c.fields.get_guid(CORPSE_FIELD_OWNER),
                        worn,
                    )
                })
        })
        .await;
        if let Some(b) = found {
            return b;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("no bones of {owner}");
}

/// A ghost who reclaims their corpse leaves bones where it lay: another
/// player is told to forget the corpse and of the bones, a corpse of the
/// bones kind (`CORPSE_FLAG_UNK2 | CORPSE_FLAG_BONES`, the owner kept, no
/// gear), and so is the risen; they lie `bones_hours` (18), then go.
#[tokio::test]
async fn a_reclaimed_corpse_is_bones_everyone_sees_until_their_hours_are_up() {
    let realm = harness::realm();
    let (mut a, ga, map) = enter(&realm, "A").await;
    let (mut b, _, _) = enter(&realm, "B").await;
    b.sees(ga).await;
    die_and_release(&mut a, &map).await;
    let corpse = guid::corpse(guid::counter(ga));
    b.sees(corpse).await;
    let lay = on(&map, move |m| m.corpses[&corpse].position).await;
    // The wait to reclaim over.
    on(&map, move |m| {
        let since = m.now - Duration::from_secs(60);
        m.players.get_mut(&ga).unwrap().death = Death::Ghost { since };
    })
    .await;
    a.send(CMSG_RECLAIM_CORPSE { guid: corpse }).await;
    b.forgets(corpse).await;
    let (bones, at, flags, owner, worn) = bones_of(&map, ga).await;
    assert_ne!(bones, corpse, "bones are a new object");
    assert_eq!((at.x, at.y, at.z), (lay.x, lay.y, lay.z), "where it lay");
    assert_eq!((flags, owner, worn), (0x05, ga, 0));
    b.sees(bones).await;
    a.sees(bones).await;
    assert!(on(&map, move |m| m.players[&ga].death == Death::Alive).await);
    // 18 hours from the reclaim they lie, to the moment; then they go.
    let (hours, lying) = on(&map, move |m| {
        let until = m.bones.iter().find(|(_, g)| *g == bones).unwrap().0;
        let hours = until.saturating_duration_since(m.now);
        m.now = until - Duration::from_millis(1);
        bones::update(m);
        let before = m.corpses.contains_key(&bones);
        m.now = until;
        bones::update(m);
        (hours, (before, m.corpses.contains_key(&bones)))
    })
    .await;
    let lie = Duration::from_secs(18 * 3600);
    assert!(
        hours <= lie && hours > lie - Duration::from_secs(30),
        "{hours:?}"
    );
    assert_eq!(lying, (true, false));
    b.forgets(bones).await;
    a.forgets(bones).await;
}

/// Raised where the ghost stands, far from the corpse, the bones are
/// where the corpse lay; with `bones_hours` 0 there are none.
#[tokio::test]
async fn a_raised_ghosts_bones_lie_at_the_corpse_and_none_at_zero_hours() {
    for hours in [18, 0] {
        let mut cfg = harness::test_config();
        cfg.players.bones_hours = hours;
        let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
        let realm = Realm::new(cfg, store, startup, GameData::empty(), None);
        let (mut a, ga, map) = enter(&realm, "A").await;
        let (mut b, _, _) = enter(&realm, "B").await;
        die_and_release(&mut a, &map).await;
        let corpse = guid::corpse(guid::counter(ga));
        b.sees(corpse).await;
        let lay = on(&map, move |m| m.corpses[&corpse].position).await;
        let risen = on(&map, move |m| {
            put(m, ga, 20.0);
            death::resurrect_with(m, ga, 50, 0)
        })
        .await;
        assert!(risen);
        b.forgets(corpse).await;
        if hours == 0 {
            settled(&mut b).await;
            let left = on(&map, move |m| m.corpses.len()).await;
            assert_eq!(left, 0, "no bones at 0 hours");
            continue;
        }
        let (bones, at, ..) = bones_of(&map, ga).await;
        assert_eq!((at.x, at.y), (lay.x, lay.y), "at the corpse, not the ghost");
        b.sees(bones).await;
    }
}
