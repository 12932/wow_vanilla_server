//! Round 8's world tests, ported from Thistle Tea's
//! `test/game/world/system/` (docs/parity/matrix.csv): creature groups a
//! script breaks up.

use wow_world_messages::Guid;

use crate::game::data::GameData;
use crate::game::data::scripts::{GROUP_AGGRO_TOGETHER, GROUP_FORMATION, GroupMember, ScriptStep};
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::creature_groups::{self, members_of};
use crate::game::map::scripts;
use crate::game::tests_p3_entity::step;
use crate::game::tests_p4_items_quests::{START, player, realm_of, spawn, template, world};

const LEADER: u32 = 997_850;
const FIRST: u32 = 997_851;
const SECOND: u32 = 997_852;

/// A database group: its leader, and two members in formation behind it
/// that fight together.
fn group_world() -> GameData {
    let mut d = world();
    for (i, entry) in [LEADER, FIRST, SECOND].into_iter().enumerate() {
        d.creature_templates
            .insert(entry, template(entry, &format!("Walker {i}")));
        spawn(
            &mut d,
            0,
            entry,
            (START.0 + 10.0 + 3.0 * i as f32, START.1, START.2),
        );
    }
    let member = |guid, distance| GroupMember {
        guid,
        distance,
        angle: std::f32::consts::PI,
        flags: GROUP_FORMATION | GROUP_AGGRO_TOGETHER,
    };
    d.scripts.groups.insert(
        LEADER,
        vec![member(LEADER, 0.0), member(FIRST, 3.0), member(SECOND, 6.0)],
    );
    for g in [LEADER, FIRST, SECOND] {
        d.scripts.group_of.insert(g, LEADER);
    }
    d
}

fn creature(entry: u32) -> Guid {
    guid::unit(entry, entry)
}

/// Where `g` stands in its group: its members here, whether it keeps a
/// place in a formation, and its place there.
fn standing(m: &Map, g: Guid) -> (Vec<Guid>, bool, bool) {
    (
        members_of(m, g),
        creature_groups::following(m, g),
        creature_groups::formation_home(m, g).is_some(),
    )
}

/// Thistle Tea's `CreatureGroups.leave/4` "removal survives reload and
/// invalidates queued commands" and "the original leader disbands every
/// member", and `formation/3` "leaving and disbanding remove formation
/// snapshots": a member's script leaving its database group (79) takes it
/// out, its place in the formation gone, the rest of the group still
/// together; dead and back, it is still out; a script's join (78) puts it
/// by its leader again. The leader's own leaving disbands the group: no
/// member is of it, nor keeps a place behind it, and the one it took back
/// is let go too (vmangos's `SCRIPT_COMMAND_LEAVE_CREATURE_GROUP` by
/// `DisbandGroup`). Justin's walk in Stormwind disbands his so.
#[tokio::test]
async fn a_member_leaves_its_group_and_its_leader_leaving_disbands_it() {
    let realm = realm_of(group_world());
    let (mut a, ga, map) = player(&realm, "Alice").await;
    let (leader, first, second) = (creature(LEADER), creature(FIRST), creature(SECOND));
    a.sees(second).await;
    let out = map
        .query(move |m| {
            let together = standing(m, first);
            let leave = step(79, [0; 4]);
            assert!(scripts::execute(m, &leave, first, first));
            let left = (standing(m, first), standing(m, second));
            crate::game::map::combat::kill(m, ga, first);
            crate::game::map::ai::respawn_now(m, first);
            let back = (
                m.creature(first).is_some_and(|c| c.alive()),
                standing(m, first),
            );
            let join = ScriptStep {
                x: 3.0,
                ..step(78, [GROUP_FORMATION, 0, 0, 0])
            };
            assert!(scripts::execute(m, &join, first, leader));
            let rejoined = members_of(m, first);
            assert!(scripts::execute(m, &leave, leader, leader));
            let disbanded = (
                standing(m, leader).0,
                standing(m, first),
                standing(m, second),
                creature_groups::is_leader(m, leader),
            );
            (together, left, back, rejoined, disbanded)
        })
        .await
        .unwrap();
    let (together, left, back, rejoined, disbanded) = out;
    assert_eq!(
        together,
        (vec![leader, first, second], true, true),
        "in its group and formation"
    );
    assert_eq!(
        left,
        ((vec![], false, false), (vec![leader, second], true, true)),
        "out, the rest together"
    );
    assert_eq!(back, (true, (vec![], false, false)), "back and still out");
    assert_eq!(rejoined, vec![leader, first], "joined again by a script");
    assert_eq!(
        disbanded,
        (
            vec![],
            (vec![], false, false),
            (vec![], false, false),
            false
        ),
        "the leader's leaving disbands it"
    );
}
