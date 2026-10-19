//! Round 7's kill rewards (Thistle Tea's `kill_credit_test.exs` and
//! `group_reward_test.exs`): a creature a player controlled is worth
//! nothing, its control let go by its death or not; a grey or no-XP kill
//! still counts for quests and standing; and a grey master's pet keeps its
//! own share of a group's kill.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::misc::{FactionDef, KillReputation, RANK_EXALTED};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::{Map, MapKey, combat};
use crate::game::tests_p3_combat::{self as p3, BRUTE, START};
use crate::game::tests_p3_entity::take_quest;
use crate::game::tests_p6_wspell::put;
use crate::game::tests_p7_combat::{BEAST_WATCHER, off};

const GUILD: u32 = 529;
const HUNT: u32 = 98_200;
/// A brute worth no XP (static flag `NO_XP`).
const WORTHLESS: u32 = 3760;

fn world() -> GameData {
    let mut d = crate::game::tests_p7_combat::data();
    let mut worthless = p3::template(WORTHLESS, 7);
    worthless.static_flags1 = 0x2;
    d.creature_templates.insert(WORTHLESS, worthless);
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
    for entry in [BRUTE, WORTHLESS] {
        d.misc.kill_reputation.insert(
            entry,
            vec![KillReputation {
                faction: GUILD,
                value: 10,
                max_rank: RANK_EXALTED,
                team: 0,
                team_award: false,
            }],
        );
    }
    d.npc.quests.insert(
        HUNT,
        crate::game::data::npc::Quest {
            id: HUNT,
            required_kills: [(BRUTE as i32, 9), (WORTHLESS as i32, 9), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    d
}

/// `g`'s experience, standing with the guild and the hunt's two kills.
fn earned(m: &Map, g: Guid) -> (u32, i32, [u32; 2]) {
    let data = m.realm.data.clone();
    let p = &m.players[&g];
    let kills = p
        .journal
        .log
        .iter()
        .find(|l| l.quest == HUNT)
        .map_or([0; 2], |l| [l.kills[0], l.kills[1]]);
    (
        p.fields.get_u32(PLAYER_XP),
        crate::game::map::reputation::standing(p, &data, GUILD),
        kills,
    )
}

/// A creature of `entry` at `level` by `g`.
fn foe(m: &mut Map, g: Guid, entry: u32, level: u8) -> Guid {
    p3::spawn(m, g, entry, level, 3.0)
}

/// Thistle Tea's `KillCredit.eligible?/1`: a creature charmed or possessed
/// by a player, killed, gives its killer no experience, standing or quest
/// credit, though its death lets the control go before the reward (it
/// gave them all before); one never controlled gives them.
#[tokio::test]
async fn a_creature_a_player_controls_is_worth_nothing_even_as_death_frees_it() {
    let (realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(world(), Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, gb, 30);
            m.players.get_mut(&gb).unwrap().row.state.rest_bonus = 0.0;
            take_quest(m, gb, HUNT);
            let start = earned(m, gb);
            let mut after = Vec::new();
            for possess in [false, true] {
                let brute = foe(m, ga, BRUTE, 30);
                crate::game::map::pets::charmed(m, ga, brute, possess);
                combat::deal_damage(m, gb, brute, 1_000_000);
                after.push((
                    m.creature(brute)
                        .is_some_and(|c| !c.alive() && c.owner.is_none()),
                    earned(m, gb),
                ));
            }
            let free = foe(m, ga, BRUTE, 30);
            combat::deal_damage(m, gb, free, 1_000_000);
            (start, after, earned(m, gb))
        })
        .await
        .unwrap();
    let (start, after, free) = r;
    assert_eq!(
        after,
        vec![(true, start); 2],
        "dead, let go, and worth nothing"
    );
    assert!(free.0 > start.0, "a free one's experience");
    assert_eq!(
        (free.1, free.2),
        (start.1 + 10, [1, 0]),
        "standing and credit"
    );
}

/// Thistle Tea's "grants quest and reputation eligibility for gray and
/// no-XP kills": a grey creature gives no experience but its quest credit
/// and a fifth of its standing; one worth no XP its credit and its whole
/// standing.
#[tokio::test]
async fn a_grey_or_worthless_kill_still_counts_for_quests_and_standing() {
    let (_realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 30);
            m.players.get_mut(&ga).unwrap().row.state.rest_bonus = 0.0;
            take_quest(m, ga, HUNT);
            let start = earned(m, ga);
            let grey = foe(m, ga, BRUTE, 20);
            crate::game::map::ai::attacked(m, grey, ga);
            combat::deal_damage(m, ga, grey, 1_000_000);
            let after_grey = earned(m, ga);
            let worthless = foe(m, ga, WORTHLESS, 30);
            crate::game::map::ai::attacked(m, worthless, ga);
            combat::deal_damage(m, ga, worthless, 1_000_000);
            (start, after_grey, earned(m, ga))
        })
        .await
        .unwrap();
    let (start, grey, worthless) = r;
    assert_eq!(
        grey,
        (start.0, start.1 + 2, [1, 0]),
        "grey: a fifth, and the credit"
    );
    assert_eq!(
        worthless,
        (start.0, start.1 + 12, [1, 1]),
        "worthless: the whole standing, and the credit"
    );
}

/// Alice and Bob, grouped, beside each other.
async fn grouped(a: &mut Client, b: &mut Client) {
    a.send(CMSG_GROUP_INVITE { name: "Bob".into() }).await;
    b.wait_for("SMSG_GROUP_INVITE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
    })
    .await;
    b.send(CMSG_GROUP_ACCEPT {}).await;
    a.wait_for("SMSG_GROUP_LIST", |m| match m {
        ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
        _ => None,
    })
    .await;
}

/// Thistle Tea's "keeps a gray owner's pet share independently of player
/// XP": a level-60 hunter for whom a level-21 kill is grey, grouped with a
/// level 20, earns nothing of it; their level-20 pet earns its master's
/// share, 58 (half of 152 x 60/80, and one); the 20 earns 20 (half of
/// 152 x 20/80, and one).
#[tokio::test]
async fn a_grey_masters_pet_keeps_its_share_of_a_group_kill() {
    let realm = crate::game::harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Dwarf, Class::Hunter).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    b.heartbeat(START.0, START.1, START.2).await;
    grouped(&mut a, &mut b).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let r = map
        .query(move |m| {
            p3::level(m, ga, 60);
            p3::level(m, gb, 20);
            for g in [ga, gb] {
                m.players.get_mut(&g).unwrap().row.state.rest_bonus = 0.0;
                put(m, g, off(0.0, 0.0, 0.0));
            }
            crate::game::map::pets::give_pet(m, ga, BEAST_WATCHER, 20).expect("a pet");
            let pet = crate::game::map::pets::pet_of(m, ga).expect("Alice's pet");
            let xp = |m: &Map| {
                (
                    m.players[&ga].fields.get_u32(PLAYER_XP),
                    m.creature(pet)
                        .unwrap()
                        .fields
                        .get_u32(UNIT_FIELD_PETEXPERIENCE),
                    m.players[&gb].fields.get_u32(PLAYER_XP),
                )
            };
            let before = xp(m);
            let brute = foe(m, ga, BRUTE, 21);
            crate::game::map::ai::attacked(m, brute, ga);
            combat::deal_damage(m, ga, brute, 1_000_000);
            (before, xp(m))
        })
        .await
        .unwrap();
    let (before, after) = r;
    assert_eq!(
        (after.0 - before.0, after.1 - before.1, after.2 - before.2),
        (0, 58, 20),
        "(the hunter, the pet, Bob)"
    );
}
