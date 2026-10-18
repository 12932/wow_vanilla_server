//! Round 5's pet targeting tests, ported from Thistle Tea's
//! `test/game/core/ai/bt/pet/{target_selection,targeting}_test.exs`
//! (docs/parity/matrix.csv): whom a hunter's pet takes on of its own
//! accord, and the crowd control it leaves be. The pet is the boar of
//! `tests_pets::tamed`.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::map::pets::{self, ACT_COMMAND, COMMAND_ATTACK, COMMAND_STAY, REACT_AGGRESSIVE};
use crate::game::map::{Map, ai, combat};
use crate::game::tests_p5_ai_pets::tamed_with;
use crate::game::tests_pets::{self, WOLF, place};

/// A stun that damage breaks: Sap's kind.
const SAP: u32 = 9970;

fn data() -> GameData {
    let mut d = tests_pets::data();
    let mut e = tests_pets::effect(6, 0, 6);
    e.aura = crate::game::map::auras::AURA_MOD_STUN;
    let none = SpellEffect::default();
    let mut s = tests_pets::spell(SAP, "Sap", [e, none, none]);
    s.duration_index = 4;
    s.aura_interrupt_flags = crate::game::map::auras::INTERRUPT_DAMAGE;
    d.spells.spells.insert(SAP, s);
    d
}

/// `gs` hard to kill: the pet's blows don't end the test's fights.
fn sturdy(m: &mut Map, gs: &[Guid]) {
    for g in gs {
        if let Some(c) = m.creature_mut(*g) {
            c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
            c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            c.base.max_health = 1_000_000;
        }
    }
}

/// `a` fights `b`.
fn attacks(m: &mut Map, a: Guid, b: Guid) {
    combat::start_attack(m, a, b);
    combat::engage(m, a, b);
}

fn victim(m: &Map, g: Guid) -> Option<Guid> {
    m.combat(g).filter(|c| c.attacking).map(|c| c.victim)
}

/// The pet's updates, a second apart.
fn think(m: &mut Map, times: u32) {
    for _ in 0..times {
        m.now += Duration::from_secs(1);
        pets::update(m);
    }
}

/// `by` saps `who`.
fn sap(m: &mut Map, by: Guid, who: Guid) {
    let t = m.realm.data.spells.get(SAP).cloned().unwrap();
    crate::game::map::auras::apply_spell(m, by, who, &t, &[(0, 0)], Default::default()).unwrap();
}

/// The master's Attack, at `target`.
fn command_attack(m: &mut Map, owner: Guid, pet: Guid, target: Guid) {
    pets::handle(
        m,
        owner,
        ClientOpcodeMessage::CMSG_PET_ACTION(CMSG_PET_ACTION {
            pet,
            data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
            target,
        }),
    );
}

/// What a pet takes on of its own accord: not its master's selection
/// while nobody fights, but one that fights it, its master in a fight or
/// not; and of those, not one under crowd control damage would break,
/// whoever is nearer. Crowd control put on its victim after its master's
/// Attack has it let that victim be, and with every other foe held it
/// takes nothing up; an Attack at a foe already held breaks it (Thistle
/// Tea's `TargetSelection.next`, `Targeting.automatic_allowed?`;
/// mangos's `PetAI::UpdateAI` and `HasAuraPetShouldAvoidBreaking`).
#[tokio::test]
async fn a_pet_takes_on_who_fights_it_and_leaves_crowd_control_be_unless_sent_at_it() {
    let (_realm, _h, gh, map, pet, far) = tamed_with(data()).await;
    let a = place(&map, gh, WOLF, 0x70_0010, 3.0).await;
    let b = place(&map, gh, WOLF, 0x70_0011, 6.0).await;
    let c = place(&map, gh, WOLF, 0x70_0012, 9.0).await;
    let picks = map
        .query(move |m| {
            for g in [gh, pet, far] {
                combat::leave_combat(m, g);
            }
            sturdy(m, &[a, b, c]);
            let mut picks = Vec::new();
            // Selected, not fought.
            if let Some(p) = m.players.get_mut(&gh) {
                p.fields.set_guid(UNIT_FIELD_TARGET, a);
            }
            think(m, 1);
            picks.push(victim(m, pet));
            // It fights the pet; the master isn't fighting.
            attacks(m, b, pet);
            think(m, 1);
            picks.push(victim(m, pet));
            ai::die(m, b, gh);
            // The nearer is sapped.
            attacks(m, a, pet);
            attacks(m, c, pet);
            sap(m, gh, a);
            think(m, 1);
            picks.push(victim(m, pet));
            // Sent at the farther, which is sapped then: let be, and the
            // other held too, nothing.
            command_attack(m, gh, pet, c);
            think(m, 1);
            picks.push(victim(m, pet));
            sap(m, gh, c);
            think(m, 1);
            picks.push(victim(m, pet));
            // Sent at one already held: it breaks it.
            command_attack(m, gh, pet, a);
            think(m, 1);
            picks.push(victim(m, pet));
            picks
        })
        .await
        .unwrap();
    assert_eq!(
        picks,
        [None, Some(b), Some(c), Some(c), None, Some(a)],
        "the selection, its attacker, past the sapped, the command, sapped after it, an Attack at the sapped"
    );
}

/// Told to stay, an aggressive pet takes up a foe within its reach but
/// none farther, and doesn't leave its place for one that strikes it from
/// beyond it (Thistle Tea's `Targeting.automatic_allowed?` and
/// `retaliation?`).
#[tokio::test]
async fn a_staying_pet_takes_on_only_what_is_in_its_reach() {
    let (_realm, _h, gh, map, pet, far) = tamed_with(data()).await;
    let picks = map
        .query(move |m| {
            for g in [gh, pet, far] {
                combat::leave_combat(m, g);
            }
            sturdy(m, &[far]);
            let s = pets::state_mut(m, pet).unwrap();
            s.react = REACT_AGGRESSIVE;
            s.command = COMMAND_STAY;
            let at = m.creature(pet).unwrap().position;
            s_at(m, pet, at);
            // A hostile wolf ten yards off.
            let wolf_at = Vector3d { x: at.x + 10.0, ..at };
            crate::game::map::scripts::place(m, far, wolf_at, 0.0);
            think(m, 1);
            let distant = victim(m, pet);
            attacks(m, far, pet);
            think(m, 3);
            let held = m.creature(pet).unwrap().position;
            let struck_from_afar = combat::dist(held, at) < 0.5
                && !matches!(
                    m.creature(pet).unwrap().motion,
                    crate::game::map::creature::Motion::Moving(_)
                );
            let near = Vector3d { x: at.x + 1.0, ..at };
            crate::game::map::scripts::place(m, far, near, 0.0);
            think(m, 1);
            (distant, struck_from_afar, victim(m, pet))
        })
        .await
        .unwrap();
    assert_eq!(picks, (None, true, Some(far)), "none afar, held where it stays when struck from afar, the near");
}

fn s_at(m: &mut Map, pet: Guid, at: Vector3d) {
    if let Some(s) = pets::state_mut(m, pet) {
        s.stay_at = Some(at);
    }
}

/// A passive pet takes nothing up, even struck; a defensive one keeps the
/// living victim it has when another strikes it, and turns on the other
/// once it is dead. An aggressive guardian, unlike a pet, breaks crowd
/// control (Thistle Tea's `Targeting.automatic_allowed?`, `retaliation?`
/// "keeps a living victim and permits replacement after its death", and
/// "aggressive guardians retain their uncontrolled attack behavior";
/// mangos's `PetAI::AttackedBy`).
#[tokio::test]
async fn a_passive_pet_fights_nothing_and_a_pet_keeps_its_living_victim() {
    let (_realm, _h, gh, map, pet, far) = tamed_with(data()).await;
    let a = place(&map, gh, WOLF, 0x70_0010, 3.0).await;
    let b = place(&map, gh, WOLF, 0x70_0011, 5.0).await;
    let c = place(&map, gh, WOLF, 0x70_0012, 7.0).await;
    let picks = map
        .query(move |m| {
            for g in [gh, pet, far] {
                combat::leave_combat(m, g);
            }
            sturdy(m, &[a, b, c]);
            let mut picks = Vec::new();
            pets::state_mut(m, pet).unwrap().react = pets::REACT_PASSIVE;
            attacks(m, a, pet);
            think(m, 2);
            picks.push(victim(m, pet));
            pets::state_mut(m, pet).unwrap().react = pets::REACT_DEFENSIVE;
            think(m, 1);
            picks.push(victim(m, pet));
            attacks(m, b, pet);
            think(m, 2);
            picks.push(victim(m, pet));
            ai::die(m, a, gh);
            think(m, 1);
            picks.push(victim(m, pet));
            // A guardian, aggressive, beside a sapped wolf fighting its
            // master.
            ai::die(m, b, gh);
            let at = m.players[&gh].position();
            let guardian = crate::game::map::pets::testing::spawn(
                m,
                gh,
                WOLF,
                pets::Kind::Guardian,
                1,
                at,
                0.0,
            );
            pets::state_mut(m, guardian).unwrap().react = REACT_AGGRESSIVE;
            attacks(m, c, gh);
            sap(m, gh, c);
            think(m, 1);
            picks.push(victim(m, guardian));
            picks
        })
        .await
        .unwrap();
    assert_eq!(
        picks,
        [None, Some(a), Some(a), Some(b), Some(c)],
        "passive; defensive, its attacker; kept; the other; the guardian breaks the sap"
    );
}

// A pet's own buffs: Dash (a speed whose cooldown outlasts it), Prowl
// (not in a fight, and ending an attack), Mend (a heal over time), a buff
// that stacks, and one over its party.
const DASH: u32 = 9980;
const PROWL: u32 = 9981;
const MEND: u32 = 9982;
const STACKING: u32 = 9983;
const AREA: u32 = 9984;

fn autocast_data() -> GameData {
    let mut d = data();
    let none = SpellEffect::default();
    let aura = |kind: u32, points: i32| {
        let mut e = tests_pets::effect(6, points, 1);
        e.aura = kind;
        e
    };
    let mut add = |id: u32, name: &str, e: SpellEffect, f: &dyn Fn(&mut crate::game::data::spells::SpellTemplate)| {
        let mut s = tests_pets::spell(id, name, [e, none, none]);
        s.duration_index = 4;
        f(&mut s);
        d.spells.spells.insert(id, s);
    };
    use crate::game::map::auras::{AURA_MOD_INCREASE_SPEED, AURA_PERIODIC_HEAL};
    add(DASH, "Dash", aura(AURA_MOD_INCREASE_SPEED, 40), &|s| {
        s.duration_index = 3;
        s.recovery_time = 30_000;
    });
    add(PROWL, "Prowl", aura(4, 0), &|s| {
        s.attributes[0] |= 0x0010_0000 | 0x1000_0000;
    });
    add(MEND, "Mend", {
        let mut e = aura(AURA_PERIODIC_HEAL, 5);
        e.amplitude = 3000;
        e
    }, &|_| {});
    add(STACKING, "Stacking", aura(29, 1), &|s| s.stack_amount = 3);
    let mut area = tests_pets::effect(35, 1, 1);
    area.aura = 29;
    area.radius_index = 9;
    add(AREA, "Area", area, &|_| {});
    d
}

/// The pet's buffs on itself, with their stacks, by spell.
fn buffs(m: &Map, pet: Guid) -> Vec<(u32, u32)> {
    let mut b: Vec<(u32, u32)> = crate::game::map::auras::auras(m, pet)
        .map(|a| {
            a.holders
                .iter()
                .filter(|h| [DASH, PROWL, MEND, STACKING, AREA].contains(&h.spell))
                .map(|h| (h.spell, h.stacks.max(1)))
                .collect()
        })
        .unwrap_or_default();
    b.sort_unstable();
    b
}

/// What a pet's autocast puts on itself: idle, a buff for a fight waits
/// (Dash, whose cooldown outlasts it), Prowl doesn't, and a heal waits
/// for a wound; in a fight Prowl, which would end its attack, waits, and
/// Dash goes only while its victim is out of reach. A buff that stacks is
/// cast again over itself; one over the party isn't (Thistle Tea's
/// `Autocast.allowed?`).
#[tokio::test]
async fn a_pets_autocast_buffs_wait_for_their_time() {
    let (_realm, _h, gh, map, pet, far) = tamed_with(autocast_data()).await;
    let steps = map
        .query(move |m| {
            for g in [gh, pet, far] {
                combat::leave_combat(m, g);
            }
            sturdy(m, &[far]);
            let mut steps = Vec::new();
            let with = |m: &mut Map, autocast: &[u32]| {
                if let Some(c) = m.caster_mut(pet) {
                    c.cooldowns.clear();
                    c.gcd_until = None;
                }
                pets::state_mut(m, pet).unwrap().autocast = autocast.to_vec();
                think(m, 1);
                buffs(m, pet)
            };
            steps.push(with(m, &[DASH, MEND, PROWL]));
            crate::game::map::auras::remove_spell(m, pet, PROWL, None);
            let max = m.creature(pet).unwrap().fields.get_u32(UNIT_FIELD_MAXHEALTH);
            m.creature_mut(pet)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, max / 2);
            steps.push(with(m, &[DASH, MEND]));
            crate::game::map::auras::remove_spell(m, pet, MEND, None);
            // A victim twenty yards off.
            let at = m.creature(pet).unwrap().position;
            let off = Vector3d { x: at.x + 20.0, ..at };
            crate::game::map::scripts::place(m, far, off, 0.0);
            pets::attack_target(m, pet, far);
            m.note_fighting(pet);
            steps.push(with(m, &[PROWL, DASH]));
            crate::game::map::auras::remove_spell(m, pet, DASH, None);
            combat::stop_attack(m, pet);
            combat::leave_combat(m, pet);
            combat::leave_combat(m, far);
            steps.push(with(m, &[STACKING]));
            steps.push(with(m, &[STACKING]));
            steps.push(with(m, &[AREA]));
            steps.push(with(m, &[AREA]));
            steps
        })
        .await
        .unwrap();
    assert_eq!(steps[0], [(PROWL, 1)], "idle: Prowl, no Dash, no heal at full health");
    assert_eq!(steps[1], [(MEND, 1)], "a wound: the heal");
    assert_eq!(steps[2], [(DASH, 1)], "fighting from afar: Dash, not Prowl");
    assert_eq!(steps[3], [(STACKING, 1)]);
    assert_eq!(steps[4], [(STACKING, 2)], "stacked");
    assert_eq!(steps[5], [(STACKING, 2), (AREA, 1)]);
    assert_eq!(steps[6], [(STACKING, 2), (AREA, 1)], "not again");
}
