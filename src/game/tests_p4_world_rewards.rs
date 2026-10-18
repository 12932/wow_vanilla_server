//! Phase 3's fourth round of the player's kill rewards, ported from
//! Thistle Tea's `test/game/world/entity/player_group_reward_test.exs`
//! and `player_kill_reward_test.exs` (docs/parity/matrix.csv): what the
//! dead, the released and the living earn of a kill.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::misc::{FactionDef, KillReputation, RANK_EXALTED};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, take_quest};
use crate::game::tests_p4_world::on;

const GUILD: u32 = 529;
const HUNT: u32 = 98_100;

fn world() -> GameData {
    let mut d = data();
    d.misc.factions.insert(
        GUILD,
        FactionDef {
            id: GUILD,
            index: 13,
            name: "The Guild".into(),
            variants: [(0x1, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(13, GUILD);
    d.misc.kill_reputation.insert(
        BRUTE,
        vec![KillReputation {
            faction: GUILD,
            value: 10,
            max_rank: RANK_EXALTED,
            team: 0,
            team_award: false,
        }],
    );
    d.npc.quests.insert(
        HUNT,
        crate::game::data::npc::Quest {
            id: HUNT,
            required_kills: [(BRUTE as i32, 9), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    d
}

/// `g`'s experience and standing with the guild.
fn earned(m: &Map, g: Guid) -> (u32, i32) {
    let data = m.realm.data.clone();
    let p = &m.players[&g];
    (
        p.fields.get_u32(PLAYER_XP),
        crate::game::map::reputation::standing(p, &data, GUILD),
    )
}

/// A brute tapped by `g`, then slain by `g`, after `before` has had its say
/// on `g`.
fn kill(m: &mut Map, g: Guid, before: impl FnOnce(&mut Map, Guid)) -> Guid {
    let brute = spawn(m, g, BRUTE, 20, 4.0);
    crate::game::map::ai::attacked(m, brute, g);
    before(m, brute);
    crate::game::map::combat::deal_damage(m, g, brute, 1_000_000);
    brute
}

fn credit(m: &ServerOpcodeMessage) -> Option<u32> {
    match m {
        ServerOpcodeMessage::SMSG_QUESTUPDATE_ADD_KILL(k) => Some(k.kill_count),
        _ => None,
    }
}

fn experience(m: &ServerOpcodeMessage) -> Option<()> {
    matches!(m, ServerOpcodeMessage::SMSG_LOG_XPGAIN(_)).then_some(())
}

/// Thistle Tea's "awards an unreleased corpse quest credit and full
/// reputation without XP", "rechecks release at delivery and retains only
/// reputation" and "does not grant solo XP to a ghost with positive
/// health": the living earn experience, the kill's quest credit and its
/// reputation; one dead where they fell (their damage over time landing
/// the blow) the credit and the reputation, no experience; a ghost, with
/// its one point of health, the reputation alone.
#[tokio::test]
async fn the_dead_earn_a_kills_credit_and_a_ghost_its_reputation_alone() {
    let (_realm, mut a, g, map) = setup_with(world(), Class::Warrior).await;
    let start = on(&map, move |m| {
        take_quest(m, g, HUNT);
        earned(m, g)
    })
    .await;
    a.clear();
    let alive = on(&map, move |m| {
        kill(m, g, |_, _| {});
        earned(m, g)
    })
    .await;
    assert!(alive.0 > start.0, "the living earn experience");
    assert_eq!(alive.1, start.1 + 10, "and reputation");
    a.wait_for("the experience", experience).await;
    assert_eq!(a.wait_for("the credit", credit).await, 1);
    a.clear();
    let dead = on(&map, move |m| {
        kill(m, g, |m, brute| {
            crate::game::map::combat::deal_damage(m, brute, g, 1_000_000);
        });
        let p = &m.players[&g];
        (earned(m, g), p.death)
    })
    .await;
    assert!(matches!(
        dead.1,
        crate::game::map::death::Death::Dead { .. }
    ));
    assert_eq!(
        dead.0,
        (alive.0, alive.1 + 10),
        "no experience, the reputation"
    );
    assert_eq!(a.wait_for("the dead's credit", credit).await, 2);
    assert!(a.gets(NONE_WITHIN, experience).await.is_none());
    a.clear();
    let ghost = on(&map, move |m| {
        let now = m.now;
        crate::game::map::death::become_ghost(m.players.get_mut(&g).unwrap(), now);
        kill(m, g, |_, _| {});
        let p = &m.players[&g];
        (earned(m, g), p.fields.get_u32(UNIT_FIELD_HEALTH))
    })
    .await;
    assert_eq!(ghost.1, 1, "a ghost has its point of health");
    assert_eq!(ghost.0, (alive.0, alive.1 + 20), "the reputation alone");
    assert!(
        a.gets(NONE_WITHIN, |m| credit(m).or(experience(m).map(|_| 0)))
            .await
            .is_none(),
        "no credit, no experience"
    );
}

/// Thistle Tea's "fatal blows notify the active pet even when the victim
/// is gray" and "preserves capped-player rested XP and still forwards the
/// pet's group reward": a hunter at the level cap earns nothing of a
/// kill and keeps their rested pool, and of one grey to them alone; their
/// pet earns its own share of both.
#[tokio::test]
async fn a_capped_hunters_pet_earns_of_kills_its_master_cant() {
    use crate::game::map::combat::{kill_options, kill_xp_with};
    use crate::game::map::pets::{Kind, testing as hunter};
    let realm = crate::game::harness::realm_with(world());
    let mut a = crate::game::harness::Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let map = realm.registry.online(h).unwrap().map();
    let pet = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        p.row.state.rest_bonus = 1_000.0;
        let at = p.position();
        let pet = hunter::spawn(
            m,
            h,
            crate::game::tests_p3_combat::BEAST,
            Kind::Hunter,
            40,
            at,
            0.0,
        );
        m.players.get_mut(&h).unwrap().owned.pet = Some(pet);
        pet
    })
    .await;
    a.clear();
    let pet_xp = move |m: &Map| {
        m.creature(pet)
            .unwrap()
            .fields
            .get_u32(UNIT_FIELD_PETEXPERIENCE)
    };
    for level in [45, 60] {
        let (before, after, want, master) = on(&map, move |m| {
            let before = pet_xp(m);
            let brute = spawn(m, h, BRUTE, level, 4.0);
            let want = kill_xp_with(60, 40, u32::from(level), &kill_options(m, brute));
            crate::game::map::ai::attacked(m, brute, h);
            crate::game::map::combat::deal_damage(m, h, brute, 1_000_000);
            let p = &m.players[&h];
            (
                before,
                pet_xp(m),
                want,
                (p.fields.get_u32(PLAYER_XP), p.row.state.rest_bonus),
            )
        })
        .await;
        assert!(want > 0, "level {level} isn't grey to the pet");
        assert_eq!(after - before, want, "the pet's share of a level {level}");
        assert_eq!(
            master,
            (0, 1_000.0),
            "the hunter earns nothing, nor spends rest"
        );
    }
    assert!(a.gets(NONE_WITHIN, experience).await.is_none());
}

/// Thistle Tea's "marks the attacking mob faction temporarily at war": a
/// creature of a faction the player is at peace with taking them on its
/// threat list sets its faction at war with them for the fight
/// (`SMSG_SET_FACTION_ATWAR`, the faction's index and the flag).
#[tokio::test]
async fn a_creature_that_takes_one_on_is_at_war_with_them_for_the_fight() {
    let mut d = world();
    d.faction_templates
        .get_mut(&crate::game::tests_p3_combat::PACK)
        .unwrap()
        .faction = GUILD;
    let (_realm, mut a, g, map) = setup_with(d, Class::Warrior).await;
    a.clear();
    let marked = on(&map, move |m| {
        let brute = spawn(m, g, BRUTE, 20, 4.0);
        crate::game::map::ai::attacked(m, brute, g);
        m.players[&g].temporary_war.clone()
    })
    .await;
    assert_eq!(marked, vec![GUILD]);
    let war = a
        .wait_frame("SMSG_SET_FACTION_ATWAR", 0x313, |_| true)
        .await;
    let mut want = 13u32.to_le_bytes().to_vec();
    want.push(crate::game::map::reputation::FLAG_AT_WAR as u8);
    assert_eq!(war, want);
}
