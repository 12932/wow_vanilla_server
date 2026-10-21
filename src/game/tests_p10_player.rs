//! Round 10's player: a charge's swing at what is still selected, an
//! invalid lesson (Thistle Tea's `attacking_test`, `server_movement_test`
//! and `teaching_dbc_test`).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::fields::UNIT_FIELD_TARGET;
use crate::game::harness;
use crate::game::map::combat::dist;
use crate::game::map::{Map, MapHandle, charge};
use crate::game::tests_p3_entity::{NONE_WITHIN, spell};
use crate::game::tests_parity_motion as motion;

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// The map's clock `by` on, and the charges' step at that time.
fn later(m: &mut Map, by: Duration) {
    m.now += by;
    charge::update(m);
}

const KNOCK: u32 = 97_961;

fn charge_world() -> crate::game::data::GameData {
    let mut d = motion::wolf_world();
    d.spells.spells.insert(
        KNOCK,
        spell(
            KNOCK,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_KNOCK_BACK,
                base_points: 69,
                base_dice: 1,
                misc_value: 120,
                target_a: 1,
                ..Default::default()
            }],
        ),
    );
    d.creature_spawns.get_mut(&0).unwrap()[0] =
        motion::spawn(80, motion::WOLF, motion::at(20.0, 0.0));
    d
}

/// Thistle Tea's `start_selected` and `ServerMovement.start` and `cancel`:
/// a player's charge runs along its path (part of the way at 200 ms, by
/// its target at the end) and swings on arriving at what the client still
/// has selected, though nothing had begun a fight ("arrival uses client
/// selection even when combat has cleared the attack field"). Another
/// selected on the way, none, or the player themselves: no swing ("changing
/// or clearing selection prevents a late attack"). A target dead or gone
/// by then gets none either; cut short (a teleport), the player stays
/// where the run had taken them; taken over by another's control (Thistle
/// Tea's "incoming control invalidates an old charge arrival"), it is
/// over, with no swing.
#[tokio::test]
async fn a_charge_swings_on_arriving_only_at_what_is_still_selected() {
    let (_realm, mut a, ga, map) = motion::enter(charge_world()).await;
    let g = motion::wolf();
    a.sees(g).await;
    type Change = fn(&mut Map, Guid, Guid);
    let changes: [(&str, Option<Change>); 5] = [
        ("kept", None),
        (
            "cleared",
            Some(|m, ga, _| {
                let p = m.players.get_mut(&ga).unwrap();
                p.fields.set_guid(UNIT_FIELD_TARGET, Guid::zero());
            }),
        ),
        (
            "themselves",
            Some(|m, ga, _| {
                let p = m.players.get_mut(&ga).unwrap();
                p.fields.set_guid(UNIT_FIELD_TARGET, ga);
            }),
        ),
        (
            "another",
            Some(|m, ga, _| {
                let p = m.players.get_mut(&ga).unwrap();
                p.fields
                    .set_guid(UNIT_FIELD_TARGET, Guid::new(0xF130_0000_0000_0099));
            }),
        ),
        (
            "the target gone",
            Some(|m, _, g| {
                m.remove_creature(g);
            }),
        ),
    ];
    for (what, change) in changes {
        let out = on(&map, move |m| {
            crate::game::map::combat::stop_attack(m, ga);
            let p = m.players.get_mut(&ga).unwrap();
            p.charge = None;
            p.movement.position = motion::at(0.0, 0.0);
            p.fields.set_guid(UNIT_FIELD_TARGET, g);
            m.creature_mut(g)?.position = motion::at(20.0, 0.0);
            charge::start(m, ga, g, true);
            later(m, Duration::from_millis(200));
            let partway = m.players[&ga].position();
            if let Some(change) = change {
                change(m, ga, g);
            }
            later(m, Duration::from_secs(5));
            let p = &m.players[&ga];
            Some((partway, p.position(), p.combat.attacking, p.combat.victim))
        })
        .await;
        let Some((partway, end, attacking, victim)) = out else {
            panic!("{what}: the wolf is gone too soon");
        };
        let (start, wolf) = (motion::at(0.0, 0.0), motion::at(20.0, 0.0));
        assert!(
            dist(start, partway) > 0.5 && dist(partway, wolf) > 5.0,
            "{what}: part of the way: {partway:?}"
        );
        assert!(dist(end, wolf) < 5.0, "{what}: by the wolf: {end:?}");
        if what == "kept" {
            assert!(attacking && victim == g, "{what}: it swings");
        } else {
            assert!(!attacking, "{what}: no swing");
        }
    }
    // Cut short by a teleport where it stands: it stays there.
    let (partway, end) = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.teleport = None;
        p.movement.position = motion::at(0.0, 0.0);
        p.movement.orientation = 0.0;
        let creature = crate::game::tests_p3_combat::spawn(m, ga, motion::WOLF, 1, 40.0);
        charge::start(m, ga, creature, true);
        later(m, Duration::from_millis(300));
        let partway = m.players[&ga].position();
        crate::game::map::movement::teleport_within(m, ga, partway, 0.0);
        later(m, Duration::from_secs(5));
        (partway, m.players[&ga].position())
    })
    .await;
    assert!(dist(partway, end) < 0.01, "where the run took them");
    // Taken over by another's control on the way: no swing on arriving.
    let attacking = on(&map, move |m| {
        crate::game::map::combat::stop_attack(m, ga);
        let p = m.players.get_mut(&ga).unwrap();
        p.teleport = None;
        p.movement.position = motion::at(0.0, 0.0);
        p.movement.orientation = 0.0;
        let wolf = crate::game::tests_p3_combat::spawn(m, ga, motion::WOLF, 1, 20.0);
        let p = m.players.get_mut(&ga).unwrap();
        p.fields.set_guid(UNIT_FIELD_TARGET, wolf);
        charge::start(m, ga, wolf, true);
        later(m, Duration::from_millis(200));
        let now = m.now;
        let p = m.players.get_mut(&ga).unwrap();
        p.owned.possessed_by = Some(crate::game::map::pets::possess::Possession {
            master: Guid::new(7),
            spell: 605,
            was_faction: 1,
            manual: true,
            next_think: now,
            next_cast_at: now,
            melee_fallback: false,
        });
        later(m, Duration::from_millis(50));
        let p = m.players.get_mut(&ga).unwrap();
        let over = p.charge.is_none();
        p.owned.possessed_by = None;
        later(m, Duration::from_secs(5));
        (over, m.players[&ga].combat.attacking)
    })
    .await;
    assert_eq!(attacking, (true, false), "possessed: over, no swing");
}

const BAD_LESSON: u32 = 97_960;

/// Thistle Tea's "invalid taught spells and missing reagents leave books
/// and skills untouched", as mangos's `addSpell` refuses a spell it
/// doesn't know: a lesson whose spell doesn't exist teaches nothing, and
/// says nothing.
#[tokio::test]
async fn a_lesson_of_a_spell_that_doesnt_exist_teaches_nothing() {
    let mut d = crate::game::data::GameData::empty();
    d.spells.spells.insert(
        BAD_LESSON,
        spell(
            BAD_LESSON,
            &[SpellEffect {
                effect: crate::game::map::spells::EFFECT_LEARN_SPELL,
                trigger_spell: 999_999,
                target_a: 1,
                ..Default::default()
            }],
        ),
    );
    let realm = harness::realm_with(d);
    let (mut a, g, map) =
        crate::game::tests_pets::enter(&realm, "A", "Reader", Race::Human, Class::Mage).await;
    a.clear();
    let learned = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, g, BAD_LESSON, Some(g));
        m.players[&g].spells.contains(&999_999)
    })
    .await;
    assert!(!learned, "not in the spellbook");
    let told = a
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_LEARNED_SPELL(s) if s.id == 999_999 => Some(()),
            _ => None,
        })
        .await;
    assert!(told.is_none(), "nor told");
}

/// Thistle Tea's `movement_control_test` "assigns one sequence across
/// acknowledged movement changes" and "supersedes older teleports", and
/// `travel_test`'s "rejects stale and foreign acknowledgements", as
/// mangos's session counts its orders (`GetOrderCounter`): a root and two
/// teleports take one counter each, in turn; the first teleport is stale
/// once the second is sent, its acknowledgement settling nothing, nor one
/// naming another player; the second's settles it, once.
#[tokio::test]
async fn movement_orders_share_one_counter_and_a_stale_or_foreign_ack_settles_nothing() {
    let (_realm, a, ga, map) = motion::enter(charge_world()).await;
    let counters = on(&map, move |m| {
        let p = &m.players[&ga];
        crate::game::map::movement::root_to(&p.outbound, ga, true);
        let at = p.position();
        crate::game::map::movement::teleport_within(m, ga, at, 0.0);
        let first = m.players[&ga]
            .outbound
            .pending_orders()
            .last()
            .map(|(c, _)| *c);
        crate::game::map::movement::teleport_within(m, ga, at, 1.0);
        let orders: Vec<u32> = m.players[&ga]
            .outbound
            .pending_orders()
            .iter()
            .map(|(c, _)| *c)
            .collect();
        (first.unwrap(), orders)
    })
    .await;
    let (first, orders) = counters;
    assert_eq!(orders.len(), 2, "the root and the second teleport wait");
    assert_eq!(orders[1], orders[0] + 2, "one sequence: {orders:?}");
    assert_eq!(first, orders[0] + 1, "the first teleport between them");
    let second = orders[1];
    let ack = |guid: Guid, movement_counter: u32| MSG_MOVE_TELEPORT_ACK_Client {
        guid,
        movement_counter,
        time: Default::default(),
    };
    let waiting = move |m: &Map| m.players[&ga].teleport.is_some();
    for (what, sent) in [
        ("stale", ack(ga, first)),
        ("foreign", ack(Guid::new(0x7777), second)),
    ] {
        a.send(sent).await;
        harness::an_update(&map).await;
        assert!(on(&map, move |m| waiting(m)).await, "{what}: still waiting");
    }
    a.send(ack(ga, second)).await;
    harness::an_update(&map).await;
    let settled = on(&map, move |m| {
        (
            waiting(m),
            m.players[&ga]
                .outbound
                .pending_orders()
                .iter()
                .any(|(c, _)| *c == second),
        )
    })
    .await;
    assert_eq!(settled, (false, false), "the second's settles it, once");
}

/// Thistle Tea's "death and teleport prevent late acknowledgement
/// relocation": a launch's acknowledgement that comes after its player
/// died settles the order, and moves the body nowhere (a teleport's case:
/// `tests_p7_movement_acks::a_launch_ack_settles_once_for_the_launch_sent`).
#[tokio::test]
async fn a_launch_acknowledged_after_death_moves_no_body() {
    use crate::game::conn::Order;
    let (_realm, a, ga, map) = motion::enter(charge_world()).await;
    let (counter, impulse, here) = on(&map, move |m| {
        crate::game::map::spells::cast_triggered(m, ga, KNOCK, Some(ga));
        let p = &m.players[&ga];
        let (c, impulse) = p
            .outbound
            .pending_orders()
            .into_iter()
            .find_map(|(c, o)| match o {
                Order::KnockBack { impulse, .. } => Some((c, impulse)),
                _ => None,
            })
            .expect("a launch waits");
        let here = p.position();
        crate::game::map::death::kill(m, ga);
        (c, impulse, here)
    })
    .await;
    let away = Vector3d {
        x: here.x + 2.0,
        ..here
    };
    a.send(CMSG_MOVE_KNOCK_BACK_ACK {
        guid: ga,
        counter,
        info: MovementInfo {
            flags: MovementInfo_MovementFlags::new_jumping(MovementInfo_MovementFlags_Jumping {
                cos_angle: impulse[0],
                sin_angle: impulse[1],
                xy_speed: impulse[2],
                z_speed: impulse[3],
            }),
            timestamp: 0,
            position: away,
            orientation: 0.0,
            fall_time: 0,
        },
    })
    .await;
    harness::an_update(&map).await;
    let (at, waiting) = on(&map, move |m| {
        let p = &m.players[&ga];
        (
            p.position(),
            p.outbound
                .pending_orders()
                .iter()
                .any(|(c, _)| *c == counter),
        )
    })
    .await;
    assert!(dist(at, here) < 0.01, "the body stays: {at:?}");
    assert!(!waiting, "the order settled");
}

const PARTY_BUFF: u32 = 97_962;

/// Thistle Tea's "rejects a party-only cast before spending resources when
/// its target is invalid", as mangos's `CheckCast` for a party member's
/// spell (target 35): with no target, at the caster, or at one not in the
/// party, it is refused (BAD_TARGETS), nothing spent; at a party member it
/// lands.
#[tokio::test]
async fn a_party_members_spell_is_for_a_party_member() {
    let mut d = charge_world();
    let mut buff = spell(
        PARTY_BUFF,
        &[crate::game::tests_p3_entity::aura_effect(
            crate::game::map::auras::AURA_DUMMY,
            1,
            0,
            35,
        )],
    );
    buff.mana_cost = 50;
    d.spells.spells.insert(PARTY_BUFF, buff);
    let realm = harness::realm_with(d);
    let (a, ga, map) =
        crate::game::tests_pets::enter(&realm, "A", "Alice", Race::Human, Class::Priest).await;
    let (_b, gb, _) =
        crate::game::tests_pets::enter(&realm, "B", "Bob", Race::Human, Class::Priest).await;
    let mut a = a;
    crate::game::tests_pets::learn(&map, ga, &[PARTY_BUFF]);
    let mana = move |m: &Map| {
        m.players[&ga]
            .fields
            .get_u32(crate::game::fields::UNIT_FIELD_POWER1)
    };
    for target in [None, Some(ga), Some(gb)] {
        harness::an_update(&map).await;
        let before = on(&map, move |m| mana(m)).await;
        a.clear();
        crate::game::tests_pets::cast(&a, PARTY_BUFF, target).await;
        let refused = a
            .wait_frame(
                "SMSG_CAST_RESULT",
                crate::game::map::spells::SMSG_CAST_RESULT,
                |f| u32::from_le_bytes(f[0..4].try_into().unwrap()) == PARTY_BUFF,
            )
            .await;
        assert_eq!(
            refused[4..6],
            [2, crate::game::map::spells::checks::fail::BAD_TARGETS],
            "{target:?}"
        );
        assert_eq!(on(&map, move |m| mana(m)).await, before, "nothing spent");
    }
    let (oa, ob) = (
        realm.registry.online(ga).unwrap(),
        realm.registry.online(gb).unwrap(),
    );
    assert!(crate::game::social::groups::join_directly(&realm, &oa, &ob));
    crate::game::tests_pets::cast(&a, PARTY_BUFF, Some(gb)).await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while !on(&map, move |m| {
        crate::game::map::auras::auras(m, gb).is_some_and(|x| x.has_spell(PARTY_BUFF))
    })
    .await
    {
        assert!(tokio::time::Instant::now() < deadline, "it lands on Bob");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
