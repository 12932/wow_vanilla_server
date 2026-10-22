//! `.durability off|on`, a load test's unbreakable gear, through the
//! protocol: off mends everything a GM owns, broken gear giving what it
//! gives again; nothing wears while it is off (death, a blow, a spirit
//! healer, a spell); on wears again; a player can't; and it is kept on the
//! character.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::inventory::{BANK_START, PACK_START};
use crate::game::map::{Map, combat, death, vendor};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{BRUTE, equip, setup_with, spawn};
use crate::game::tests_p3_entity::on;
use crate::game::tests_p5_player_misc::{CHEST, wear_world};

/// `SMSG_DURABILITY_DAMAGE_DEATH`'s opcode.
const SMSG_DURABILITY_DAMAGE_DEATH_OPCODE: u16 = 0x2BD;

/// Stamina's field.
const STAMINA: u16 = UNIT_FIELD_STAT0 + 2;

/// The chest of 50 with ten stamina.
fn world() -> GameData {
    let mut d = wear_world();
    if let Some(t) = d.items.get_mut(&CHEST) {
        t.stats[0] = (7, 10);
    }
    d
}

/// The durability of each of Alice's chests, worn first, then the
/// backpack's and the bank's.
fn chests(m: &Map, g: Guid) -> Vec<u32> {
    let p = &m.players[&g];
    let mut out: Vec<(u8, u32)> = p
        .items
        .iter()
        .filter(|i| i.entry == CHEST)
        .map(|i| (i.slot, i.fields.get_u32(ITEM_FIELD_DURABILITY)))
        .collect();
    out.sort_unstable();
    out.into_iter().map(|(_, d)| d).collect()
}

/// Alice in the chest, hardy.
fn chested(m: &mut Map, g: Guid) {
    equip(m, g, CHEST, 4);
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
    f.set_u32(UNIT_FIELD_HEALTH, 100_000);
}

async fn say(a: &mut Client, line: &str, reply: &str) -> String {
    a.say(line).await;
    a.system_message(reply).await
}

fn no_wear(m: &Map, g: Guid) -> bool {
    m.players[&g].row.state.no_wear
}

/// The chest worn and broken, another in the backpack and a third in the
/// bank worn down: `.durability off` mends all three, free, and the broken
/// one's stamina is had again; the client is told of the worn one's.
#[tokio::test]
async fn durability_off_mends_broken_gear_and_its_stats_come_back() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let before = on(&map, move |m| {
        chested(m, ga);
        let unbroken = m.players[&ga].fields.get_u32(STAMINA);
        for slot in [PACK_START, BANK_START] {
            crate::game::map::inventory::add_item(m, ga, CHEST, 1, false).unwrap();
            let p = m.players.get_mut(&ga).unwrap();
            let i = p.items.iter().rposition(|i| i.entry == CHEST).unwrap();
            p.items[i].slot = slot;
        }
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&ga).unwrap();
        for it in p.items.iter_mut().filter(|i| i.entry == CHEST) {
            let worn = if it.slot == 4 { 0 } else { 20 };
            it.fields.set_u32(ITEM_FIELD_DURABILITY, worn);
        }
        p.refresh_inventory(&data);
        p.apply_stats(&data);
        let money = p.money();
        (unbroken, p.fields.get_u32(STAMINA), chests(m, ga), money)
    })
    .await;
    assert_eq!(before.0, before.1 + 10, "broken, the chest's ten are gone");
    assert_eq!(before.2, vec![0, 20, 20]);
    let worn = on(&map, move |m| {
        let p = &m.players[&ga];
        p.items[p.item_at(BAG_PLAYER, 4).unwrap()].guid
    })
    .await;
    a.clear();
    let reply = say(&mut a, ".durability off", "Durability off").await;
    assert!(reply.contains("3 items mended"), "{reply}");
    let after = on(&map, move |m| {
        let p = &m.players[&ga];
        (
            p.fields.get_u32(STAMINA),
            chests(m, ga),
            p.money(),
            no_wear(m, ga),
        )
    })
    .await;
    assert_eq!(after, (before.0, vec![50, 50, 50], before.3, true));
    a.wait_for("the chest's durability, whole", |m| {
        crate::game::harness::values(m)
            .into_iter()
            .find_map(|(g, mask)| match mask {
                UpdateMask::Item(i) if g == worn => i.item_durability().filter(|d| *d == 50),
                _ => None,
            })
    })
    .await;
    let shown = say(&mut a, ".durability", "Durability is").await;
    assert!(shown.contains("off"), "{shown}");
}

/// A creature's kill wears nothing off with durability off, and the
/// client isn't told it did; on again, a tenth (5 of 50), and the death's
/// notice. Nothing is mended by turning it on.
#[tokio::test]
async fn death_wears_nothing_with_durability_off_and_a_tenth_with_it_on() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| chested(m, ga)).await;
    say(&mut a, ".durability off", "Durability off").await;
    a.clear();
    let off = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        combat::kill(m, brute, ga);
        let worn = chests(m, ga);
        // And a spirit healer's quarter.
        vendor::wear_carried(m, ga, 0.25);
        death::resurrect(m, ga, 1.0);
        (worn, chests(m, ga))
    })
    .await;
    assert_eq!(off, (vec![50], vec![50]));
    say(&mut a, ".durability on", "Durability on").await;
    let told = a.take_frames(SMSG_DURABILITY_DAMAGE_DEATH_OPCODE).len();
    assert_eq!(told, 0, "no notice of a loss that wasn't");
    let on_again = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        combat::kill(m, brute, ga);
        (chests(m, ga), no_wear(m, ga))
    })
    .await;
    assert_eq!(on_again, (vec![45], false));
    a.wait_for("SMSG_DURABILITY_DAMAGE_DEATH", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_DURABILITY_DAMAGE_DEATH).then_some(())
    })
    .await;
}

/// With durability off, a blow taken or dealt wears no point off (the roll
/// forced under the chance, on the chest's slot); a spell's wear takes
/// nothing, its mending still mends; `.debug durability` says it can't.
#[tokio::test]
async fn a_blow_with_durability_off_wears_nothing() {
    use crate::game::map::vendor::FORCED_WEAR;
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| chested(m, ga)).await;
    say(&mut a, ".durability off", "Durability off").await;
    let worn = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        FORCED_WEAR.with(|f| f.set(Some((0.1, 4))));
        let mut out = Vec::new();
        combat::deal_damage(m, brute, ga, 10);
        out.push(chests(m, ga)[0]);
        combat::deal_damage(m, ga, brute, 10);
        out.push(chests(m, ga)[0]);
        FORCED_WEAR.with(|f| f.set(None));
        out
    })
    .await;
    assert_eq!(worn, vec![50, 50], "taken, dealt");
    let refused = say(&mut a, ".debug durability 50", "doesn't wear").await;
    assert!(refused.contains(".durability on"), "{refused}");
    assert_eq!(on(&map, move |m| chests(m, ga)).await, vec![50]);
    // And on: the same blow wears its point.
    say(&mut a, ".durability on", "Durability on").await;
    let worn = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        FORCED_WEAR.with(|f| f.set(Some((0.1, 4))));
        combat::deal_damage(m, brute, ga, 10);
        FORCED_WEAR.with(|f| f.set(None));
        chests(m, ga)
    })
    .await;
    assert_eq!(worn, vec![49]);
}

/// A player isn't told of the command; a moderator is refused; neither
/// turns it off.
#[tokio::test]
async fn a_player_may_not_turn_durability_off() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| {
        m.players.get_mut(&ga).unwrap().gm_level = crate::accounts::MODERATOR;
    })
    .await;
    say(&mut a, ".durability off", "permission").await;
    on(&map, move |m| m.players.get_mut(&ga).unwrap().gm_level = 0).await;
    say(&mut a, ".durability off", "There is no such command").await;
    assert!(!on(&map, move |m| no_wear(m, ga)).await);
}

/// Off is kept on the character: after a relog, it is still off, and a
/// death wears nothing.
#[tokio::test]
async fn durability_off_survives_a_relog() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    on(&map, move |m| chested(m, ga)).await;
    say(&mut a, ".durability off", "Durability off").await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    a.login(ga).await;
    let after = on(&map, move |m| {
        let kept = no_wear(m, ga);
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        let f = &mut m.players.get_mut(&ga).unwrap().fields;
        f.set_u32(UNIT_FIELD_HEALTH, 10);
        combat::kill(m, brute, ga);
        (kept, chests(m, ga))
    })
    .await;
    assert_eq!(after, (true, vec![50]));
    let shown = say(&mut a, ".durability", "Durability is").await;
    assert!(shown.contains("off"), "{shown}");
}
