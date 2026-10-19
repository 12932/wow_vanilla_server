//! Round 6, against Thistle Tea's `bt/combat_test.exs` and
//! `bt/melee_readiness_test.exs` (docs/parity/matrix.csv): a creature's
//! first swing, the swing errors a player is told, a victim gone from
//! sight, each hand's own weapon skill, a queued strike's lifecycle, extra
//! attacks and a chain on the next swing, and a pet turning to swing.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{ATTR_ON_NEXT_SWING, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{self, AURA_MOD_SKILL};
use crate::game::map::combat::{self, Hand, Outcome};
use crate::game::map::{Map, ai, derive};
use crate::game::tests_p3_combat::{self as base, BRUTE, SWORD, aura_of, equip, setup_with, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, on};

const DAGGER: u32 = 63_101;
/// A next-swing strike that wants a weapon and 15 rage; one that cleaves
/// a second enemy.
const HEAVY: u32 = 95_301;
const CLEAVE: u32 = 95_302;

fn world() -> GameData {
    let mut d = base::data();
    d.items.insert(
        DAGGER,
        ItemTemplate {
            entry: DAGGER,
            class: 2,
            subclass: 15,
            name: "Dagger".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 1500,
            stackable: 1,
            ..Default::default()
        },
    );
    let strike = |id: u32, chain: u32| {
        let mut s = SpellTemplate {
            id,
            name: format!("Strike {id}"),
            range_index: 1,
            equipped_item_class: 2,
            dmg_class: 2,
            power_type: 1,
            mana_cost: 150,
            ..Default::default()
        };
        s.attributes[0] = ATTR_ON_NEXT_SWING;
        s.effects[0] = SpellEffect {
            effect: 2,
            base_points: 9,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            chain_target: chain,
            ..Default::default()
        };
        s.dmg_multiplier = [1.0; 3];
        s
    };
    d.spells.spells.insert(HEAVY, strike(HEAVY, 0));
    d.spells.spells.insert(CLEAVE, strike(CLEAVE, 2));
    d
}

/// `g`'s white swings for fifty, as the field holds it until its stats
/// are worked out again.
fn white(m: &mut Map, g: Guid) {
    let f = m.unit_fields_mut(g).unwrap();
    f.set_f32(UNIT_FIELD_MINDAMAGE, 50.0);
    f.set_f32(UNIT_FIELD_MAXDAMAGE, 50.0);
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH)
}

/// One creature turn, 50 ms on.
fn turn(m: &mut Map) {
    m.now += Duration::from_millis(50);
    ai::update_creatures(m);
}

/// How many of the messages `pick` takes have come, in what has.
async fn count(c: &mut Client, pick: fn(&ServerOpcodeMessage) -> bool) -> usize {
    let mut n = 0;
    while c
        .gets(NONE_WITHIN, |m| pick(m).then_some(()))
        .await
        .is_some()
    {
        n += 1;
    }
    n
}

fn not_in_range(m: &ServerOpcodeMessage) -> bool {
    matches!(m, ServerOpcodeMessage::SMSG_ATTACKSWING_NOTINRANGE)
}

fn bad_facing(m: &ServerOpcodeMessage) -> bool {
    matches!(m, ServerOpcodeMessage::SMSG_ATTACKSWING_BADFACING)
}

/// Thistle Tea's "swings immediately on fresh aggro when already in reach"
/// and "retries shortly instead of arming a full swing timer when out of
/// reach on fresh aggro": a creature set on a player in reach swings at
/// its first turn; one out of reach (rooted, so it can't close) arms no
/// swing timer, and swings at its first turn once the player comes into
/// reach (mangos's `UpdateMeleeAttackingState`, whose swing errors leave
/// the hands ready to try again).
#[tokio::test]
async fn a_creature_swings_at_its_first_turn_in_reach_and_out_of_reach_keeps_its_swing_ready() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = on(&map, move |m| {
        let swung = |m: &Map, g: Guid| {
            let now = m.now;
            m.creature(g)
                .unwrap()
                .combat
                .next_swing
                .is_some_and(|t| t > now)
        };
        // Enough of herself to take their blows.
        base::level(m, ga, 60);
        let near = spawn(m, ga, BRUTE, 20, 1.0);
        ai::attacked(m, near, ga);
        let ready = m.creature(near).unwrap().combat.next_swing == Some(m.now);
        turn(m);
        let at_once = swung(m, near);
        combat::kill(m, ga, near);
        // Rooted, twenty yards off.
        let far = spawn(m, ga, BRUTE, 20, 20.0);
        aura_of(m, far, 96_500, auras::AURA_MOD_ROOT, 0, 1);
        ai::attacked(m, far, ga);
        let mut armed = false;
        for _ in 0..10 {
            turn(m);
            armed |= swung(m, far);
        }
        let at = m.creature(far).unwrap().position;
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.position = Vector3d {
            x: at.x - 1.0,
            ..at
        };
        turn(m);
        (ready && at_once, armed, swung(m, far))
    })
    .await;
    assert!(r.0, "in reach: a swing at its first turn");
    assert!(!r.1, "out of reach: no swing, none armed");
    assert!(r.2, "come into reach: a swing at its next turn");
}

/// Thistle Tea's "notifies a player once per continuous out-of-range swing
/// error", "restarting an attack permits fresh error feedback" and "swing
/// errors go only to the explicit owner context": out of range, a player
/// is told once however long it lasts (it was each second); a different
/// error is told, and the first again after it; stopping and starting the
/// attack tells it afresh; another player near hears none of it (mangos's
/// `LastSwingErrorMsg`).
#[tokio::test]
async fn a_swing_error_is_told_once_while_it_lasts_again_when_it_changes_and_to_the_attacker_alone()
{
    let realm = crate::game::harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let start = base::START;
    a.heartbeat(start.0, start.1, start.2).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let brute = on(&map, move |m| {
        let p = m.players.get_mut(&gb).unwrap();
        p.movement.position = Vector3d {
            x: start.0 + 2.0,
            y: start.1 + 2.0,
            z: start.2,
        };
        let brute = spawn(m, ga, BRUTE, 20, 20.0);
        combat::start_attack(m, ga, brute);
        // Ten seconds of updates, out of range.
        for _ in 0..10 {
            m.now += Duration::from_millis(1_000);
            combat::try_swing(m, ga);
        }
        brute
    })
    .await;
    assert_eq!(count(&mut a, not_in_range).await, 1, "told once");
    on(&map, move |m| {
        // In reach, turned away: another error; then out of range again.
        let at = m.creature(brute).unwrap().position;
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.position = Vector3d {
            x: at.x - 1.0,
            ..at
        };
        p.movement.orientation = std::f32::consts::PI;
        for _ in 0..3 {
            m.now += Duration::from_millis(1_000);
            combat::try_swing(m, ga);
        }
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.position.x -= 20.0;
        m.now += Duration::from_millis(1_000);
        combat::try_swing(m, ga);
        // Stopped and started again: told afresh.
        combat::stop_attack(m, ga);
        combat::start_attack(m, ga, brute);
    })
    .await;
    assert_eq!(count(&mut a, bad_facing).await, 1, "facing away: told once");
    assert_eq!(
        count(&mut a, not_in_range).await,
        2,
        "out of range again, and again after a restart"
    );
    assert_eq!(count(&mut b, not_in_range).await, 0, "Bob hears none");
    assert_eq!(count(&mut b, bad_facing).await, 0);
}

/// Thistle Tea's "retries a ready off hand without changing a future
/// main-hand deadline": an off hand ready, out of range, is told so and
/// swings once in reach, the main hand's later deadline untouched; the
/// error is gone with the swing.
#[tokio::test]
async fn a_ready_off_hand_tries_again_and_leaves_a_later_main_hand_be() {
    let (_realm, mut a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = on(&map, move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .spells
            .push(derive::SPELL_DUAL_WIELD);
        equip(m, ga, SWORD, 15);
        equip(m, ga, DAGGER, 16);
        let brute = spawn(m, ga, BRUTE, 20, 20.0);
        let now = m.now;
        let main = now + Duration::from_secs(5);
        {
            let c = m.combat_mut(ga).unwrap();
            c.victim = brute;
            c.attacking = true;
            c.next_swing = Some(main);
            c.next_offhand = Some(now);
        }
        combat::try_swing(m, ga);
        let waiting = m.combat(ga).unwrap().next_offhand == Some(now);
        let at = m.creature(brute).unwrap().position;
        m.players.get_mut(&ga).unwrap().movement.position = Vector3d {
            x: at.x - 1.0,
            ..at
        };
        m.now += Duration::from_millis(100);
        let before = health(m, brute);
        combat::FORCED_OUTCOME.with(|f| f.set(Some(Outcome::Hit)));
        combat::try_swing(m, ga);
        combat::FORCED_OUTCOME.with(|f| f.set(None));
        let c = m.combat(ga).unwrap();
        (
            waiting,
            health(m, brute) < before,
            c.next_swing == Some(main),
            c.last_swing_error,
        )
    })
    .await;
    assert!(r.0, "the off hand waits ready");
    assert!(r.1, "and swings in reach");
    assert!(r.2, "the main hand's deadline untouched");
    assert_eq!(r.3, None, "the error gone");
    assert_eq!(count(&mut a, not_in_range).await, 1);
}

/// Thistle Tea's "dead attackers, ghosts, dead victims, and concealed
/// victims do not swing" and "retained melee targets must still be
/// detectable": a player's victim gone invisible, or into a stealth they
/// don't see through, isn't swung at, and the attack stops; one they
/// detect, or marked, is (the dead are `tests_p3_combat5`'s).
#[tokio::test]
async fn a_player_swings_at_no_victim_gone_from_their_sight() {
    use crate::game::map::auras::{
        AURA_MOD_INVISIBILITY, AURA_MOD_STALKED, AURA_MOD_STEALTH, AURA_MOD_STEALTH_DETECT,
    };
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = on(&map, move |m| {
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        let swings = |m: &mut Map| {
            let before = health(m, brute);
            base::forced_swing(m, ga, brute, Outcome::Hit);
            (health(m, brute) < before, m.combat(ga).unwrap().attacking)
        };
        aura_of(m, brute, 96_510, AURA_MOD_INVISIBILITY, 0, 100);
        let invisible = swings(m);
        auras::remove_spell(m, brute, 96_510, None);
        aura_of(m, brute, 96_511, AURA_MOD_STEALTH, 0, 1_000);
        let stealthed = swings(m);
        aura_of(m, ga, 96_512, AURA_MOD_STEALTH_DETECT, 0, 1_000);
        let detected = swings(m);
        auras::remove_spell(m, ga, 96_512, None);
        let mut mark = SpellTemplate {
            id: 96_513,
            name: "Mark".into(),
            equipped_item_class: -1,
            ..Default::default()
        };
        mark.effects[0] = SpellEffect {
            effect: 6,
            aura: AURA_MOD_STALKED,
            target_a: 6,
            ..Default::default()
        };
        let _ = auras::apply_spell(m, ga, brute, &mark, &[(0, 1)], Default::default());
        let marked = swings(m);
        (invisible, stealthed, detected, marked)
    })
    .await;
    assert_eq!(
        r.0,
        (false, false),
        "invisible: no swing, the attack stopped"
    );
    assert_eq!(r.1, (false, false), "stealthed: none");
    assert_eq!(r.2, (true, true), "detected: swung at");
    assert_eq!(r.3, (true, true), "marked: swung at");
}

/// Thistle Tea's "white swings and abilities share bonuses and leave
/// progression to the defender" and "replaces a disarmed queued weapon
/// ability with an unarmed swing": each hand's white swing rolls with its
/// own weapon's skill, a skill bonus counting for the hand it is for (the
/// off hand rolled the main hand's: mangos's `GetWeaponSkillValue` takes
/// the hand's); disarmed, the main hand is unarmed and the off hand keeps
/// its weapon, and a queued strike that wants a weapon gives way to a
/// white swing.
#[tokio::test]
async fn each_hand_rolls_its_own_weapons_skill_and_disarmed_a_queued_strike_gives_way() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = on(&map, move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .spells
            .push(derive::SPELL_DUAL_WIELD);
        equip(m, ga, SWORD, 15);
        equip(m, ga, DAGGER, 16);
        // Five more in swords.
        aura_of(m, ga, 96_520, AURA_MOD_SKILL, 43, 5);
        let brute = spawn(m, ga, BRUTE, 20, 1.0);
        let skill = |m: &Map, hand| combat::roll_for(m, ga, brute, hand).unwrap().attacker_skill;
        let armed = (skill(m, Hand::Main), skill(m, Hand::Off));
        aura_of(m, ga, 96_521, auras::AURA_MOD_DISARM, 0, 1);
        let disarmed = (skill(m, Hand::Main), skill(m, Hand::Off));
        // A queued strike that wants the weapon: a white swing instead.
        white(m, ga);
        m.unit_fields_mut(ga)
            .unwrap()
            .set_u32(UNIT_FIELD_POWER1 + 1, 1_000);
        m.players.get_mut(&ga).unwrap().spells.push(HEAVY);
        combat::queue_next_swing(m, ga, HEAVY);
        let before = health(m, brute);
        base::forced_swing(m, ga, brute, Outcome::Hit);
        (
            armed,
            disarmed,
            m.combat(ga).unwrap().queued,
            before - health(m, brute),
            m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1 + 1),
        )
    })
    .await;
    let ((main, off), (bare, kept), queued, dealt, rage) = r;
    assert_eq!(main, off + 5, "the sword's bonus is the main hand's alone");
    assert_eq!(kept, off, "disarmed, the off hand keeps its dagger");
    assert_ne!(bare, main, "disarmed, the main hand is unarmed");
    assert_eq!(queued, None, "the strike gone");
    assert_eq!(dealt, 50, "a white swing");
    assert_eq!(rage, 1_000, "nothing paid");
}

/// Thistle Tea's "offhand accuracy follows the live queued attack
/// lifecycle": while a strike waits for the main hand, the off hand's
/// white swings lose dual wielding's miss; once it goes, or is turned
/// away for want of rage (a white swing in its place), they have it again.
#[tokio::test]
async fn the_off_hand_misses_less_only_while_a_strike_waits() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = on(&map, move |m| {
        m.players
            .get_mut(&ga)
            .unwrap()
            .spells
            .extend([derive::SPELL_DUAL_WIELD, HEAVY]);
        equip(m, ga, SWORD, 15);
        equip(m, ga, DAGGER, 16);
        let brute = spawn(m, ga, BRUTE, 20, 1.0);
        let penalty = |m: &Map| {
            combat::roll_for(m, ga, brute, Hand::Off)
                .unwrap()
                .dual_wield
        };
        let plain = penalty(m);
        combat::queue_next_swing(m, ga, HEAVY);
        let waiting = penalty(m);
        // Too little rage: turned away, a white swing for it.
        white(m, ga);
        m.unit_fields_mut(ga)
            .unwrap()
            .set_u32(UNIT_FIELD_POWER1 + 1, 149);
        let before = health(m, brute);
        base::forced_swing(m, ga, brute, Outcome::Hit);
        let white = before - health(m, brute) == 50;
        let after = (m.combat(ga).unwrap().queued, penalty(m));
        // With rage, it goes, and the penalty is back.
        m.unit_fields_mut(ga)
            .unwrap()
            .set_u32(UNIT_FIELD_POWER1 + 1, 1_000);
        combat::queue_next_swing(m, ga, HEAVY);
        base::forced_swing(m, ga, brute, Outcome::Hit);
        let paid = m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1 + 1);
        (
            plain,
            waiting,
            white,
            after,
            paid,
            m.combat(ga).unwrap().queued,
            penalty(m),
        )
    })
    .await;
    assert!(r.0, "two weapons: the penalty");
    assert!(!r.1, "a strike waiting: none");
    assert!(r.2, "turned away: a white swing");
    assert_eq!(r.3, (None, true), "and the penalty back");
    assert_eq!(
        (r.4, r.5, r.6),
        (850, None, true),
        "spent: the penalty back"
    );
}

/// Thistle Tea's "swings and extra attacks use the melee victim after
/// selection changes", "player batches wait for facing and can execute
/// while fear is suppressed" and "resolves queued chain weapon attacks
/// against nearby enemies": a player's swings and the attacks owed go at
/// the one attacked, whoever is selected; owed ones wait while the player
/// faces away, and go under a fear that can't make them flee; a strike on
/// the next swing that chains (Cleave) takes a second enemy beside the
/// first.
#[tokio::test]
async fn swings_go_at_the_victim_owed_ones_wait_to_face_it_and_a_cleave_takes_a_second() {
    use crate::game::map::auras::{AURA_MOD_FEAR, AURA_PREVENTS_FLEEING};
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let r = on(&map, move |m| {
        // Far above them: her blows and her Cleave aren't turned aside.
        base::level(m, ga, 60);
        let brute = spawn(m, ga, BRUTE, 20, 1.0);
        let other = spawn(m, ga, BRUTE, 20, 2.5);
        let (home, facing) = {
            let p = &m.players[&ga];
            (p.position(), p.movement.orientation)
        };
        combat::start_attack(m, ga, brute);
        m.unit_fields_mut(ga)
            .unwrap()
            .set_guid(UNIT_FIELD_TARGET, other);
        let (b0, o0) = (health(m, brute), health(m, other));
        base::forced_swing(m, ga, brute, Outcome::Hit);
        m.combat_mut(ga).unwrap().extra_attacks = 2;
        m.combat_mut(ga).unwrap().next_swing = Some(m.now + Duration::from_secs(60));
        let b1 = health(m, brute);
        combat::FORCED_OUTCOME.with(|f| f.set(Some(Outcome::Hit)));
        combat::try_swing(m, ga);
        let owed = (b0 > b1, b1 > health(m, brute), o0 == health(m, other));
        // Owed again, turned away: they wait; under a fear that can't
        // make one flee, they go.
        m.combat_mut(ga).unwrap().extra_attacks = 2;
        m.players.get_mut(&ga).unwrap().movement.orientation += std::f32::consts::PI;
        let b2 = health(m, brute);
        combat::try_swing(m, ga);
        let waited = (m.combat(ga).unwrap().extra_attacks, health(m, brute) == b2);
        m.players.get_mut(&ga).unwrap().movement.orientation -= std::f32::consts::PI;
        aura_of(m, ga, 96_530, AURA_MOD_FEAR, 0, 1);
        aura_of(m, ga, 96_531, AURA_PREVENTS_FLEEING, 0, 1);
        combat::start_attack(m, ga, brute);
        let b3 = health(m, brute);
        m.combat_mut(ga).unwrap().extra_attacks = 2;
        combat::try_swing(m, ga);
        let feared = (m.combat(ga).unwrap().extra_attacks, health(m, brute) < b3);
        auras::remove_spell(m, ga, 96_530, None);
        auras::remove_spell(m, ga, 96_531, None);
        combat::FORCED_OUTCOME.with(|f| f.set(None));
        // Where the fear may have moved her from.
        let p = m.players.get_mut(&ga).unwrap();
        p.movement.position = home;
        p.movement.orientation = facing;
        // Cleave, queued: the brute and the other beside it.
        m.unit_fields_mut(ga)
            .unwrap()
            .set_u32(UNIT_FIELD_POWER1 + 1, 1_000);
        m.players.get_mut(&ga).unwrap().spells.push(CLEAVE);
        equip(m, ga, SWORD, 15);
        combat::queue_next_swing(m, ga, CLEAVE);
        let (b4, o4) = (health(m, brute), health(m, other));
        base::forced_swing(m, ga, brute, Outcome::Hit);
        let cleaved = (health(m, brute) < b4, health(m, other) < o4);
        (owed, waited, feared, cleaved)
    })
    .await;
    assert_eq!(
        r.0,
        (true, true, true),
        "the swing and the owed at the brute"
    );
    assert_eq!(r.1, (2, true), "turned away: the owed wait");
    assert_eq!(r.2.0, 0, "under a fear that can't make one flee: they go");
    assert!(r.2.1);
    assert_eq!(r.3, (true, true), "Cleave takes both");
}

/// Thistle Tea's "stationary creatures and pets turn to a melee victim
/// before swinging": a pet set on a wolf behind it turns to it with a
/// facing move its master sees, as a creature does
/// (`tests_parity_combat::a_fighting_creature_turns_to_face_its_victim`);
/// it turned without a word.
#[tokio::test]
async fn a_pet_turns_to_its_victim_with_a_facing_move() {
    let (_realm, mut h, gh, map, pet, wolf) = crate::game::tests_pets::tamed().await;
    h.sees(pet).await;
    h.sees(wolf).await;
    on(&map, move |m| {
        let at = m.creature(pet).unwrap().position;
        let c = m.creature_mut(pet).unwrap();
        c.orientation = 0.0;
        let w = m.creature_mut(wolf).unwrap();
        w.position = Vector3d {
            x: at.x - 1.5,
            ..at
        };
        w.fields.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
        w.fields.set_u32(UNIT_FIELD_HEALTH, 100_000);
        crate::game::map::pets::attack_target(m, pet, wolf);
        let _ = gh;
    })
    .await;
    let wolf_bytes = wolf.guid().to_le_bytes();
    h.wait_frame("the pet's facing move", ai::SMSG_MONSTER_MOVE, |b| {
        b.windows(9).any(|w| w[0] == 3 && w[1..] == wolf_bytes)
    })
    .await;
}
