//! A melee blow of every kind, through the 1.12.1 client's own parser
//! (`tools/client_check/attacker_state.py`).
//!
//! `SMSG_ATTACKERSTATEUPDATE`'s hit info is a set of flags (a main-hand miss
//! is 0x12, a crushing blow 0x8002), as wow_messages reads it. The client's
//! parser branches on the debug flag (0x1) alone, so these are read alike;
//! this checks that each blow wvs sends is read exactly, and that a blow's
//! damage lines stay within the five the client holds.

use std::time::Duration;

use wow_world_messages::Guid;

use crate::game::data::*;
use crate::game::harness;
use crate::game::map::MapKey;
use crate::game::map::combat::{self, *};
use crate::game::tests_client_emu::{START, capture, checker, enter, run_checker, spawn, template};

const BRUTE: u32 = 306;

fn brute() -> GameData {
    let mut d = GameData::empty();
    let mut t = template(BRUTE, "Brute", 14);
    t.level_min = 63;
    t.level_max = 63;
    d.creature_templates.insert(BRUTE, t);
    d.creature_spawns
        .insert(0, vec![spawn(1, BRUTE, (START.0 + 1.0, START.1, START.2))]);
    d
}

const OUTCOMES: [Outcome; 8] = [
    Outcome::Miss,
    Outcome::Dodge,
    Outcome::Parry,
    Outcome::Glancing,
    Outcome::Block,
    Outcome::Crit,
    Outcome::Crushing,
    Outcome::Hit,
];

/// Every swing `attacker` makes at `victim`, once with each outcome.
fn swings(m: &mut crate::game::map::Map, attacker: Guid, victim: Guid) {
    combat::start_attack(m, attacker, victim);
    for o in OUTCOMES {
        combat::FORCED_OUTCOME.with(|f| f.set(Some(o)));
        if let Some(c) = m.combat_mut(attacker) {
            c.next_swing = None;
        }
        combat::try_swing(m, attacker);
    }
    combat::FORCED_OUTCOME.with(|f| f.set(None));
    combat::stop_attack(m, attacker);
}

#[tokio::test]
#[ignore = "extended"]
async fn a_melee_blow_of_every_kind_passes_the_client() {
    let (check_py, wow_exe) = checker()
        .expect("the client-emulation check needs tools/client_check/WoW.exe and `uv`: see docs/TESTING.md");
    let realm = harness::realm_with(brute());
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (mut a, ga) = enter(&realm, "A", "Alice").await;
    let brute = map
        .query(|m| m.creatures.iter().map(|(_, c)| c.guid).next().unwrap())
        .await
        .unwrap();
    a.sees(brute).await;
    map.query(move |m| {
        m.players.get_mut(&ga).unwrap().misc.god = true;
        // The brute's blows on her (the 0x12 and 0x8002 of the Eranikus
        // fight among them), and hers on it.
        swings(m, brute, ga);
        swings(m, ga, brute);
        // What the swings above can't make here: an off hand's, a blow
        // resisted in part (an elemental's) and one soaked by a ward.
        let out = m.players[&ga].outbound.clone();
        for (hit_info, damage, state, blocked, absorbed, resisted, school) in [
            (
                HITINFO_NORMALSWING2 | HITINFO_LEFTSWING,
                12,
                VICTIMSTATE_NORMAL,
                0,
                0,
                0,
                0,
            ),
            (
                HITINFO_NORMALSWING2 | HITINFO_LEFTSWING | HITINFO_MISS,
                0,
                VICTIMSTATE_UNAFFECTED,
                0,
                0,
                0,
                0,
            ),
            (
                HITINFO_NORMALSWING2 | HITINFO_RESIST,
                30,
                VICTIMSTATE_NORMAL,
                0,
                0,
                10,
                2,
            ),
            (
                HITINFO_NORMALSWING2 | HITINFO_ABSORB,
                0,
                VICTIMSTATE_NORMAL,
                0,
                40,
                0,
                0,
            ),
            (
                HITINFO_NORMALSWING2 | HITINFO_CRUSHING | HITINFO_ABSORB | HITINFO_RESIST,
                5,
                VICTIMSTATE_NORMAL,
                0,
                20,
                5,
                4,
            ),
            (HITINFO_NORMALSWING2, 3, VICTIMSTATE_BLOCKS, 9, 0, 0, 0),
            (HITINFO_NORMALSWING2, 0, VICTIMSTATE_IS_IMMUNE, 0, 0, 0, 0),
        ] {
            out.send_frame(combat::attacker_state_full(
                brute, ga, hit_info, damage, state, blocked, absorbed, resisted, school,
            ));
        }
    })
    .await
    .unwrap();
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        a.captured();
    }
    let (capture, counts) = capture(vec![&mut a]);
    let blows = counts
        .get(&combat::SMSG_ATTACKERSTATEUPDATE)
        .copied()
        .unwrap_or(0);
    assert!(
        blows >= 15,
        "the blows were captured, each kind once at least: {blows}"
    );
    run_checker(&check_py, &wow_exe, &capture);
}
