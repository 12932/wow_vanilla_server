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

/// A chest piece of fifty durability.
const CHEST: u32 = 64_600;

fn wear_world() -> crate::game::data::GameData {
    let mut d = crate::game::tests_p3_combat::data();
    d.items.insert(
        CHEST,
        crate::game::data::ItemTemplate {
            entry: CHEST,
            class: 4,
            subclass: 1,
            name: "Chest".into(),
            display_id: 1000,
            inventory_type: 5,
            max_durability: 50,
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    d
}

/// Thistle Tea's `Durability` "nonlethal combat rolls independently for
/// attacker and victim" and "absorbed and nonlethal environmental damage
/// do not wear gear": a blow between two players wears a point off each
/// one's rolled slot when the roll comes under half a percent; a
/// creature's blow wears only its victim; a roll over it, a blow of
/// nothing, and one's own harm (the world's) wear nothing.
#[tokio::test]
async fn a_blow_wears_each_player_by_their_own_roll() {
    use crate::game::map::combat::deal_damage;
    use crate::game::map::vendor::FORCED_WEAR;
    use crate::game::store::BAG_PLAYER;
    let (realm, _a, ga, map) =
        crate::game::tests_p3_combat::setup_with(wear_world(), Class::Warrior).await;
    let (_b, gb) = crate::game::tests_p3_entity::bob(&realm, ga).await;
    let worn = on(&map, move |m| {
        for g in [ga, gb] {
            crate::game::tests_p3_combat::equip(m, g, CHEST, 4);
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
            m.players
                .get_mut(&g)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, 100_000);
        }
        let brute = crate::game::tests_p3_combat::spawn(
            m,
            ga,
            crate::game::tests_p3_combat::BRUTE,
            20,
            30.0,
        );
        let durability = |m: &crate::game::map::Map, g: Guid| {
            let p = &m.players[&g];
            let i = p.item_at(BAG_PLAYER, 4).unwrap();
            p.items[i].fields.get_u32(ITEM_FIELD_DURABILITY)
        };
        let mut out = Vec::new();
        for (roll, attacker, victim, damage) in [
            (0.1, ga, gb, 10),
            (0.1, brute, gb, 10),
            (0.5, ga, gb, 10),
            (0.1, ga, gb, 0),
            (0.1, gb, gb, 10),
        ] {
            FORCED_WEAR.with(|f| f.set(Some((roll, 4))));
            deal_damage(m, attacker, victim, damage);
            out.push((durability(m, ga), durability(m, gb)));
        }
        FORCED_WEAR.with(|f| f.set(None));
        out
    })
    .await;
    assert_eq!(
        worn,
        [(49, 49), (49, 48), (49, 48), (49, 48), (49, 48)],
        "both; the victim alone; over the roll; nothing dealt; one's own harm"
    );
}

/// Two swords, each with a chance-on-hit spell of a hundred percent: the
/// main hand's marks its victim; the off hand's marks it otherwise, and
/// its second gives extra attacks.
const MAIN_SWORD: u32 = 64_601;
const OFF_SWORD: u32 = 64_602;
const MAIN_MARK: u32 = 95_610;
const OFF_MARK: u32 = 95_611;
const EXTRA_MARK: u32 = 95_612;

fn proc_world() -> crate::game::data::GameData {
    use crate::game::data::items::ItemSpell;
    use crate::game::tests_p3_entity::{aura_effect, spell};
    let mut d = crate::game::tests_p3_combat::data();
    d.spells.durations.insert(
        9,
        crate::game::data::spells::SpellDuration {
            base: 30_000,
            per_level: 0,
            max: 30_000,
        },
    );
    for id in [MAIN_MARK, OFF_MARK, EXTRA_MARK] {
        // A harmful mark on the enemy (a snare of nothing).
        let mut s = spell(
            id,
            &[aura_effect(
                crate::game::map::auras::AURA_MOD_DECREASE_SPEED,
                -1,
                0,
                6,
            )],
        );
        s.duration_index = 9;
        s.proc_chance = 100;
        if id == EXTRA_MARK {
            s.effects[1] = crate::game::data::spells::SpellEffect {
                effect: 19,
                base_points: 0,
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            };
        }
        d.spells.spells.insert(id, s);
    }
    let sword = |entry: u32, procs: &[u32]| {
        let mut t = d.items[&crate::game::tests_p3_combat::SWORD].clone();
        t.entry = entry;
        for (i, id) in procs.iter().enumerate() {
            t.spells[i] = ItemSpell {
                id: *id,
                trigger: 2,
                ..Default::default()
            };
        }
        t
    };
    let main = sword(MAIN_SWORD, &[MAIN_MARK]);
    let off = sword(OFF_SWORD, &[OFF_MARK, EXTRA_MARK]);
    d.items.insert(MAIN_SWORD, main);
    d.items.insert(OFF_SWORD, off);
    d
}

/// Thistle Tea's `WeaponProcs` "offhand hits use only the current offhand
/// weapon" and "extra attack recursion stops before spending enchantment
/// charges": a main-hand hit procs the main hand's spell alone, an
/// off-hand hit the off hand's; and an extra attack's hit doesn't proc the
/// spell that gives extra attacks.
#[tokio::test]
async fn each_hand_procs_its_own_weapon_and_an_extra_attack_no_more() {
    use crate::game::map::procs::{EX_NORMAL_HIT, Event, PROC_DONE_MELEE, PROC_DONE_OFFHAND};
    let (_realm, _a, ga, map) =
        crate::game::tests_p3_combat::setup_with(proc_world(), Class::Warrior).await;
    let marks = on(&map, move |m| {
        crate::game::tests_p3_combat::equip(m, ga, MAIN_SWORD, 15);
        crate::game::tests_p3_combat::equip(m, ga, OFF_SWORD, 16);
        let mut out = Vec::new();
        for (offhand, extra_attack) in [(false, false), (true, false), (true, true)] {
            let brute = crate::game::tests_p3_combat::spawn(
                m,
                ga,
                crate::game::tests_p3_combat::BRUTE,
                20,
                3.0,
            );
            let e = Event {
                actor: ga,
                target: brute,
                actor_flags: if offhand {
                    PROC_DONE_OFFHAND
                } else {
                    PROC_DONE_MELEE
                },
                ex: EX_NORMAL_HIT,
                damage: 10,
                school: 1,
                offhand,
                extra_attack,
                ..Default::default()
            };
            crate::game::map::enchant::weapon_procs(m, &e);
            let has = |spell| {
                crate::game::map::auras::auras(m, brute).is_some_and(|a| a.has_spell(spell))
            };
            out.push((has(MAIN_MARK), has(OFF_MARK), has(EXTRA_MARK)));
            m.remove_creature(brute);
        }
        out
    })
    .await;
    assert_eq!(
        marks,
        [
            (true, false, false),
            (false, true, true),
            (false, true, false)
        ],
        "the main hand's; the off hand's; the off hand's but the extra attacks'"
    );
}
