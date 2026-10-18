//! Round 5's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/honor_test.exs` and others
//! (docs/parity/matrix.csv): honor inspection's refusals and layout.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapKey;
use crate::game::tests_p3_combat::START;
use crate::game::tests_p3_entity::on;

const MSG_INSPECT_HONOR_STATS: u16 = 0x2D6;

/// The honor stats `a` is told of `of`, raw, if any.
async fn inspected(a: &mut Client, of: Guid) -> Option<Vec<u8>> {
    a.clear();
    a.send(MSG_INSPECT_HONOR_STATS_Client { guid: of }).await;
    tokio::time::timeout(
        Duration::from_millis(300),
        a.wait_frame("MSG_INSPECT_HONOR_STATS", MSG_INSPECT_HONOR_STATS, |_| true),
    )
    .await
    .ok()
}

/// Thistle Tea's `Honor.inspect_reply` "rejects distant, hostile, offline,
/// and other-instance targets" (with `honor_stats_are_inspected_only_near`
/// for the distant) and "dispatches inspection and encodes the build-5875
/// response": a friend's honor is told in 1.12's layout (the guid, the
/// highest rank, today's honorable and dishonorable kills as two halves,
/// yesterday's, last week's and this week's kills each padded to four
/// bytes, the lifetime kills, the contributions, last week's standing and
/// the rank bar); an enemy one could attack is told nothing (mangos's
/// `CanAttack` refusal), nor is one logged out.
#[tokio::test]
async fn honor_is_inspected_in_its_layout_from_friends_alone() {
    use crate::game::honor::{Day, Entry};
    let realm = harness::realm();
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0 + 2.0, START.1, START.2).await;
    let counter = crate::game::guid::counter(gb);
    let expected = {
        let mut h = realm.social.honor.lock();
        let day = h.day;
        let mut e = Entry {
            guid: counter,
            highest_rank: 9,
            lifetime_honorable_kills: 7,
            lifetime_dishonorable_kills: 8,
            last_week: Day {
                honorable_kills: 5,
                contribution: 10.0,
                ..Default::default()
            },
            last_standing: 12,
            ..Default::default()
        };
        e.days.insert(
            day,
            Day {
                honorable_kills: 3,
                dishonorable_kills: 2,
                ..Default::default()
            },
        );
        e.days.insert(
            day - 1,
            Day {
                honorable_kills: 4,
                contribution: 9.0,
                ..Default::default()
            },
        );
        h.entries.insert(counter, e);
        let p = h.project(counter);
        let mut want = gb.guid().to_le_bytes().to_vec();
        want.push(9);
        want.extend(3u16.to_le_bytes());
        want.extend(2u16.to_le_bytes());
        for kills in [4u32, 5, p.this_week_kills] {
            want.extend(kills.to_le_bytes());
        }
        for v in [7u32, 8, 9, 10, p.this_week_contribution, 12] {
            want.extend(v.to_le_bytes());
        }
        want.push(p.rank_bar);
        want
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(inspected(&mut a, gb).await, Some(expected), "a friend's");
    // A forsaken beside her, both flagged for a fight: one she could attack.
    let mut c = Client::connect(&realm, "C");
    let gc = c.enter("Grom", Race::Undead, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let hostile = on(&map, move |m| {
        let at = m.players[&ga].position();
        let pvp = crate::game::map::player::UNIT_FLAG_PVP;
        for g in [ga, gc] {
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_flag(UNIT_FIELD_FLAGS, pvp, true);
        }
        m.players.get_mut(&gc).map(|p| p.movement.position = at);
        crate::game::map::combat::can_attack(m, ga, gc)
    })
    .await;
    assert!(hostile, "flagged, she could attack them");
    assert_eq!(inspected(&mut a, gc).await, None, "an enemy's");
    b.send(CMSG_LOGOUT_REQUEST {}).await;
    b.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE
        )
        .then_some(())
    })
    .await;
    assert_eq!(inspected(&mut a, gb).await, None, "one logged out");
}

/// Battle and Defensive Stance as a test world has them: a shapeshift aura
/// to forms 17 and 18.
fn stances() -> crate::game::data::GameData {
    use crate::game::data::spells::{SpellEffect, SpellTemplate};
    let mut d = crate::game::data::GameData::empty();
    for (id, form) in [(crate::game::map::spells::BATTLE_STANCE, 17), (71, 18)] {
        let mut s = SpellTemplate {
            id,
            name: format!("Stance {form}"),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: crate::game::map::auras::AURA_MOD_SHAPESHIFT,
            misc_value: form,
            target_a: 1,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

/// Log `c` out and back in.
async fn relog(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(
            m,
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE
        )
        .then_some(())
    })
    .await;
    c.clear();
    let g = c.guid;
    c.login(g).await;
    crate::game::tests_p3_player2_ammo::handled(c).await;
}

/// Thistle Tea's `Spells.apply_default_auras` "starts a stanceless warrior
/// in battle stance", "preserves an existing warrior stance" and "does not
/// apply battle stance to another class": a warrior in no stance logs in
/// in Battle Stance (mangos's `_LoadAuras`); one in Defensive Stance keeps
/// it; a mage who somehow knows Battle Stance isn't put in it.
#[tokio::test]
async fn a_stanceless_warrior_logs_in_in_battle_stance() {
    use crate::game::map::spells::BATTLE_STANCE;
    let realm = harness::realm_with(stances());
    let map = realm.maps.get(&realm, MapKey::world(0));
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Mage).await;
    for g in [ga, gb] {
        on(&map, move |m| {
            m.players.get_mut(&g).unwrap().spells.push(BATTLE_STANCE)
        })
        .await;
    }
    relog(&mut a).await;
    relog(&mut b).await;
    let stance = move |m: &mut crate::game::map::Map, g: Guid| {
        let p = &m.players[&g];
        (
            p.auras.holders.iter().map(|h| h.spell).collect::<Vec<_>>(),
            crate::game::map::auras::form(m, g),
        )
    };
    let (warrior, mage) = on(&map, move |m| (stance(m, ga), stance(m, gb))).await;
    assert_eq!(warrior, (vec![BATTLE_STANCE], 17), "the warrior");
    assert_eq!(mage, (Vec::new(), 0), "the mage");
    // Defensive Stance, kept over a relog.
    on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, 71, Some(ga));
    })
    .await;
    relog(&mut a).await;
    let warrior = on(&map, move |m| stance(m, ga)).await;
    assert_eq!(warrior, (vec![71], 18), "kept");
}
