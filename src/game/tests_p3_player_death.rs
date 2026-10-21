//! Phase 3 against Thistle Tea's death tests (`test/game/core/death`): the
//! reclaim wait's history, resurrection offers and their answers, and the
//! Soulstone's self-resurrection. Through the protocol and the map.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::corpses::{self, DECAY_MS};
use crate::game::map::death::{self, Death};
use crate::game::map::{Map, MapKey};

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

/// Wait until the map has handled what `c` sent so far.
async fn synced(c: &mut Client) {
    c.send(CMSG_PLAYED_TIME {}).await;
    c.wait_for("SMSG_PLAYED_TIME", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_PLAYED_TIME(_)).then_some(())
    })
    .await;
}

const RESURRECTION: u32 = 2006;
const SOULSTONE_AURA: u32 = 20707;
const SOULSTONE: u32 = 3026;
const GREATER_SOULSTONE: u32 = 20765;
const GREATER_SOULSTONE_RES: u32 = 20761;

fn data() -> GameData {
    let mut d = GameData::empty();
    let mut resurrection = SpellTemplate {
        id: RESURRECTION,
        name: "Resurrection".into(),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    resurrection.effects[0] = SpellEffect {
        effect: 113,
        target_a: 21,
        base_points: 69,
        base_dice: 1,
        misc_value: 135,
        ..Default::default()
    };
    d.spells.spells.insert(RESURRECTION, resurrection);
    for id in [SOULSTONE_AURA, GREATER_SOULSTONE] {
        let mut s = SpellTemplate {
            id,
            name: "Soulstone Resurrection".into(),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: crate::game::map::auras::AURA_DUMMY,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    let mut stone = SpellTemplate {
        id: SOULSTONE,
        name: "Soulstone Resurrection Effect".into(),
        equipped_item_class: -1,
        ..Default::default()
    };
    stone.effects[0] = SpellEffect {
        effect: 94,
        base_points: -401,
        base_dice: 1,
        die_sides: 1,
        misc_value: 700,
        ..Default::default()
    };
    d.spells.spells.insert(SOULSTONE, stone);
    d
}

#[tokio::test]
async fn a_death_counts_once_the_wait_is_the_live_tiers_and_a_spell_resurrection_keeps_the_history()
{
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let got = on_map(&realm, move |m| {
        let until = |m: &Map| m.players[&ga].row.state.reclaim_until;
        // Killed, and killed again while dead: one death in the history.
        death::kill(m, ga);
        let once = until(m);
        crate::game::map::combat::deal_damage(m, ga, ga, 100);
        death::kill(m, ga);
        let twice = until(m);
        (once, twice)
    })
    .await;
    let (once, twice) = got;
    assert_eq!(once, twice);
    assert!((once - corpses::unix_ms() - DECAY_MS).abs() < 5_000);
    // The wait left, as a reconnect tells it: from the release's tier,
    // and none past it.
    let left = on_map(&realm, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.row.state.reclaim_until = 600_000;
        p.row.state.released_at = 10_000;
        let p = &m.players[&ga];
        let mut out: Vec<i64> = [10_000, 35_000, 70_000, 90_000]
            .into_iter()
            .map(|now| corpses::remaining_ms(p, now))
            .collect();
        let p = m.players.get_mut(&ga).unwrap();
        p.row.state.released_at = 0;
        out.push(corpses::remaining_ms(&m.players[&ga], 90_000));
        out
    })
    .await;
    assert_eq!(left, vec![60_000, 35_000, 0, 0, 0]);
    // Released at the second tier, thirty seconds on the history has
    // fallen to the first: the live tier lets the ghost in.
    let ready = on_map(&realm, move |m| {
        let now = m.now;
        let unix = corpses::unix_ms();
        let p = m.players.get_mut(&ga).unwrap();
        p.death = Death::Ghost {
            since: now - Duration::from_secs(30),
        };
        p.row.state.released_at = unix - 30_000;
        p.row.state.reclaim_until = unix + 271_000;
        let released_tier = corpses::delay_ms(p.row.state.reclaim_until, p.row.state.released_at);
        (released_tier, corpses::ready(&m.players[&ga], now))
    })
    .await;
    assert_eq!(ready, (60_000, true));
    // A spell's resurrection ends the release, and keeps the history.
    let kept = on_map(&realm, move |m| {
        let until = m.players[&ga].row.state.reclaim_until;
        death::resurrect_with(m, ga, 50, 20);
        let p = &m.players[&ga];
        (
            p.row.state.released_at,
            p.row.state.reclaim_until == until,
            p.death,
        )
    })
    .await;
    assert_eq!(kept, (0, true, Death::Alive));
}

/// A resurrection from `caster`, on dead `target`.
async fn offered(realm: &Arc<Realm>, target: &mut Client, caster: Guid) {
    let t = target.guid;
    on_map(realm, move |m| {
        crate::game::map::spells::cast_triggered(m, caster, RESURRECTION, Some(t))
    })
    .await;
}

fn alive(m: &Map, g: Guid) -> bool {
    m.players[&g].death == Death::Alive
}

#[tokio::test]
async fn a_resurrection_is_the_first_casters_taken_from_them_once_and_a_declined_one_makes_room() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Rogue).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Priest).await;
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Carol", Race::Human, Class::Priest).await;
    b.sees(ga).await;
    c.sees(ga).await;
    on_map(&realm, move |m| {
        // Some rage to lose, and energy down, as a fight leaves them.
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_u32(UNIT_FIELD_POWER1 + 3, 10);
        death::kill(m, ga);
    })
    .await;
    offered(&realm, &mut a, gb).await;
    // Carol's, second, isn't taken in its place.
    offered(&realm, &mut a, gc).await;
    let respond = async |a: &mut Client, from: Guid, status: u8| {
        a.send(CMSG_RESURRECT_RESPONSE { guid: from, status }).await;
        synced(a).await;
    };
    respond(&mut a, gc, 1).await;
    assert!(
        !on_map(&realm, move |m| alive(m, ga)).await,
        "Carol's answered"
    );
    // Declined, it's gone: Bob's accepted next does nothing, and Carol's
    // new offer is the one.
    respond(&mut a, gb, 0).await;
    respond(&mut a, gb, 1).await;
    assert!(
        !on_map(&realm, move |m| alive(m, ga)).await,
        "a declined offer"
    );
    offered(&realm, &mut a, gc).await;
    a.clear();
    respond(&mut a, gc, 1).await;
    // Sent to Carol, she rises as she lands (mangos's
    // `DELAYED_RESURRECT_PLAYER`).
    a.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: ga,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    synced(&mut a).await;
    let risen = on_map(&realm, move |m| {
        let p = &m.players[&ga];
        (
            alive(m, ga),
            p.resurrect_offer.is_none(),
            p.fields.get_u32(UNIT_FIELD_HEALTH),
            p.fields.get_u32(UNIT_FIELD_MAXHEALTH),
            p.fields.get_u32(UNIT_FIELD_POWER1 + 3),
            p.fields.get_u32(UNIT_FIELD_MAXPOWER1 + 3),
        )
    })
    .await;
    let (up, spent, health, max_health, energy, max_energy) = risen;
    assert!(up && spent);
    assert_eq!(health, 70.min(max_health));
    assert_eq!(energy, max_energy);
    // Accepted once: again, alive, nothing.
    respond(&mut a, gc, 1).await;
    assert!(on_map(&realm, move |m| alive(m, ga)).await);
}

fn stone(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    crate::game::map::auras::apply_spell(m, g, g, &s, &[(0, 0)], Default::default()).unwrap();
}

#[tokio::test]
async fn a_soulstone_raises_its_flat_health_and_mana_clamped_and_is_forfeit_by_release_or_use() {
    let mut d = data();
    d.spells.spells.insert(GREATER_SOULSTONE_RES, {
        let mut s = d.spells.spells[&SOULSTONE].clone();
        s.id = GREATER_SOULSTONE_RES;
        s
    });
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Mage).await;
    let field = move |m: &Map, f: u16| m.players[&ga].fields.get_u32(f);
    // Each rank its own resurrection; none without a stone.
    let ranks = on_map(&realm, move |m| {
        stone(m, ga, GREATER_SOULSTONE);
        death::kill(m, ga);
        let greater = field(m, PLAYER_SELF_RES_SPELL);
        death::resurrect(m, ga, 1.0);
        death::kill(m, ga);
        let none = field(m, PLAYER_SELF_RES_SPELL);
        death::resurrect(m, ga, 1.0);
        (greater, none)
    })
    .await;
    assert_eq!(ranks, (GREATER_SOULSTONE_RES, 0));
    // Fighting, stoned, killed: up with 400 health and 700 mana, out of
    // the fight; then no stone the next time.
    on_map(&realm, move |m| {
        stone(m, ga, SOULSTONE_AURA);
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            true,
        );
        death::kill(m, ga);
    })
    .await;
    // Up by it; a second, alive, does nothing.
    a.send(CMSG_SELF_RES {}).await;
    synced(&mut a).await;
    a.send(CMSG_SELF_RES {}).await;
    synced(&mut a).await;
    let up = on_map(&realm, move |m| {
        let p = &m.players[&ga];
        (
            alive(m, ga),
            field(m, UNIT_FIELD_HEALTH).min(field(m, UNIT_FIELD_MAXHEALTH)),
            field(m, UNIT_FIELD_MAXHEALTH),
            field(m, UNIT_FIELD_POWER1),
            field(m, UNIT_FIELD_MAXPOWER1),
            field(m, PLAYER_SELF_RES_SPELL),
            p.fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            ),
        )
    })
    .await;
    let (risen, health, max_health, mana, max_mana, offer, fighting) = up;
    assert!(risen);
    assert_eq!(health, 400.min(max_health));
    assert_eq!(mana, 700.min(max_mana));
    assert_eq!((offer, fighting), (0, false));
    // A level one mage has less of either than the stone gives: clamped.
    assert!(
        max_health < 400 && max_mana < 700,
        "{max_health} {max_mana}"
    );
    // Released, the stone is forfeit: a ghost can't rise by it.
    on_map(&realm, move |m| {
        stone(m, ga, SOULSTONE_AURA);
        death::kill(m, ga);
    })
    .await;
    let held = on_map(&realm, move |m| field(m, PLAYER_SELF_RES_SPELL)).await;
    assert_eq!(held, SOULSTONE);
    a.send(CMSG_REPOP_REQUEST {}).await;
    synced(&mut a).await;
    let (offer, ghost) = on_map(&realm, move |m| {
        (
            field(m, PLAYER_SELF_RES_SPELL),
            matches!(m.players[&ga].death, Death::Ghost { .. }),
        )
    })
    .await;
    assert_eq!((offer, ghost), (0, true));
}

#[tokio::test]
async fn a_soulstone_gives_a_warrior_no_mana() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    on_map(&realm, move |m| {
        stone(m, ga, SOULSTONE_AURA);
        death::kill(m, ga);
    })
    .await;
    a.send(CMSG_SELF_RES {}).await;
    synced(&mut a).await;
    let got = on_map(&realm, move |m| {
        let f = &m.players[&ga].fields;
        (
            alive(m, ga),
            f.get_u32(UNIT_FIELD_POWER1),
            f.get_u32(UNIT_FIELD_POWER1 + 1),
        )
    })
    .await;
    assert_eq!(got, (true, 0, 0));
}
