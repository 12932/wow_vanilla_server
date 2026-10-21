//! Round 10, against Thistle Tea's `pvp_test.exs`: a fight with players
//! holds the PvP flag's five minutes and the contested thirty seconds
//! where they are; out of it they run again (mangos-classic's
//! `UpdatePvPFlagTimer` and `UpdatePvPContestedFlagTimer`, frozen while
//! `inPvPCombat`).

use std::time::Duration;

use wow_world_messages::vanilla::*;

use crate::game::map::pvp;
use crate::game::tests_p3_combat::{data, setup_with};

#[tokio::test]
async fn a_fight_with_players_holds_the_flags_timers_and_out_of_it_they_run() {
    let (_realm, _a, g, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let left = |m: &crate::game::map::Map| {
                let p = &m.players[&g].pvp;
                let now = m.now;
                (
                    p.until.map(|t| t.saturating_duration_since(now).as_secs()),
                    p.contested_until
                        .map(|t| t.saturating_duration_since(now).as_secs()),
                )
            };
            // She attacks a player: contested, flagged, in a PvP fight.
            pvp::contest(m, g);
            let now = m.now;
            m.players.get_mut(&g).unwrap().combat.last_pvp = Some(now);
            pvp::update(m);
            // Four hundred seconds of it: nothing has run.
            for _ in 0..400 {
                m.now += Duration::from_secs(1);
                let now = m.now;
                m.players.get_mut(&g).unwrap().combat.last_pvp = Some(now);
                pvp::update(m);
            }
            let fighting = left(m);
            // The fight over: a second on, a second less of each.
            m.players.get_mut(&g).unwrap().combat.last_pvp = None;
            pvp::update(m);
            m.now += Duration::from_secs(1);
            pvp::update(m);
            let after = left(m);
            let flags = m.players[&g]
                .fields
                .get_u32(crate::game::fields::PLAYER_FLAGS);
            // Thirty more: the contested flag is off, the PvP one still on.
            m.now += Duration::from_secs(30);
            pvp::update(m);
            let later = left(m);
            let flags_later = m.players[&g]
                .fields
                .get_u32(crate::game::fields::PLAYER_FLAGS);
            (fighting, after, flags, later, flags_later)
        })
        .await
        .unwrap();
    let contested = pvp::PLAYER_FLAGS_CONTESTED_PVP;
    let (fighting, after, flags, later, flags_later) = r;
    assert_eq!(fighting, (Some(300), Some(30)));
    assert_eq!(after, (Some(299), Some(29)));
    assert_ne!(flags & contested, 0);
    assert_eq!(later.0, Some(269));
    assert_eq!(flags_later & contested, 0);
}
