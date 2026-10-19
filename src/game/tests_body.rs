//! The body: eating and drinking seat the player, as mangos-classic's
//! `_AddSpellAuraHolder` sits a unit an aura that standing ends goes on,
//! and standing up, moving, jumping or turning ends the meal.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::{ClientOpcodeMessage as C, ServerOpcodeMessage as S};
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, auras};

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
