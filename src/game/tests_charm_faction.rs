//! A creature a priest's Mind Control holds fights for the priest's side:
//! a Horde battlemaster (immune to creatures, as vmangos has a city's
//! battlemasters) attacks a Horde guard on its bar's command and by the
//! priest's own swing, and the guard fights back; let go, it is of its own
//! side again, immune to creatures and flagged for PvP as it was. On a small world, and on the
//! real Kartra Bloodsnarl and Orgrimmar Grunt (extended).

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::combat::{UNIT_FLAG_IMMUNE_TO_NPC, can_attack};
use crate::game::map::pets::{self, ACT_COMMAND, COMMAND_ATTACK};
use crate::game::map::player::UNIT_FLAG_PVP;
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_entity::on;
use crate::game::tests_pets2::MIND_CONTROL;

/// The small world's battlemaster and guard, both of the Horde's
/// template.
const BATTLEMASTER: u32 = 97_800;
const GUARD: u32 = 97_801;
const HORDE: u32 = 2;

fn world() -> crate::game::data::GameData {
    use crate::game::tests_pets::template;
    let mut d = crate::game::tests_pets2::more();
    let mut bm = template(BATTLEMASTER, "Battlemaster", HORDE, 1);
    // Immune to creatures and flagged for PvP, as Kartra Bloodsnarl is.
    bm.unit_flags = UNIT_FLAG_IMMUNE_TO_NPC | UNIT_FLAG_PVP;
    d.creature_templates.insert(BATTLEMASTER, bm);
    d.creature_templates
        .insert(GUARD, template(GUARD, "Guard", HORDE, 1));
    d
}

/// A creature of `entry` `d` yards before `g`, hardy enough to fight on.
async fn place(map: &MapHandle, g: Guid, entry: u32, counter: u32, d: f32) -> Guid {
    let c = crate::game::tests_pets::place(map, g, entry, counter, d).await;
    on(map, move |m| hardy(m, c)).await;
    c
}

/// `g` made hardy enough to fight on.
fn hardy(m: &mut Map, g: Guid) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
        f.set_u32(UNIT_FIELD_HEALTH, 100_000);
    }
}

/// A unit's faction template, its two immunities and its PvP flag.
fn side(m: &Map, g: Guid) -> Option<(u32, u32)> {
    m.unit_fields(g).map(|f| {
        (
            f.get_u32(UNIT_FIELD_FACTIONTEMPLATE),
            f.get_u32(UNIT_FIELD_FLAGS) & pets::CHARM_FLAGS,
        )
    })
}

/// Poll the map until `check` holds, up to `within`.
async fn until(
    map: &MapHandle,
    within: Duration,
    check: impl Fn(&Map) -> bool + Send + Clone + 'static,
) -> bool {
    let end = tokio::time::Instant::now() + within;
    loop {
        let c = check.clone();
        if on(map, move |m| c(m)).await {
            return true;
        }
        if tokio::time::Instant::now() >= end {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Whether `a` fights `b`: swings at it, or holds it on its threat list.
fn fights(m: &Map, a: Guid, b: Guid) -> bool {
    m.combat(a).is_some_and(|c| c.attacking && c.victim == b)
        || m.creature(a)
            .is_some_and(|c| c.threat.iter().any(|(t, _)| *t == b))
}

fn hurt(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.get_u32(UNIT_FIELD_HEALTH) < f.get_u32(UNIT_FIELD_MAXHEALTH))
}

/// A pet action feedback, if one comes within `within`.
async fn feedback(c: &mut Client, within: Duration) -> Option<PetFeedback> {
    c.gets(within, |m| match m {
        ServerOpcodeMessage::SMSG_PET_ACTION_FEEDBACK(f) => Some(f.feedback),
        _ => None,
    })
    .await
}

/// The priest's Mind Control on `held`, through the client: its side
/// before and while held.
async fn mind_control(
    c: &mut Client,
    map: &MapHandle,
    priest: Guid,
    held: Guid,
    spell: u32,
) -> (Option<(u32, u32)>, Option<(u32, u32)>) {
    let before = on(map, move |m| side(m, held)).await;
    crate::game::tests_pets::cast(c, spell, Some(held)).await;
    assert!(
        crate::game::tests_pets2::control_update(c, held).await,
        "the priest takes hold"
    );
    // Its health taken anew as it is held, made hardy again.
    let (by_hand, during) = on(map, move |m| {
        hardy(m, held);
        (pets::possessed_by_hand(m, priest), side(m, held))
    })
    .await;
    assert_eq!(by_hand, Some(held));
    (before, during)
}

/// The bar's Attack on `target`.
async fn command_attack(c: &Client, pet: Guid, target: Guid) {
    c.send(CMSG_PET_ACTION {
        pet,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target,
    })
    .await;
}

async fn let_go(c: &mut Client, map: &MapHandle, priest: Guid, held: Guid, spell: u32) {
    on(map, move |m| {
        crate::game::map::spells::interrupt(m, priest);
        crate::game::map::auras::remove_spell(m, held, spell, None);
    })
    .await;
    assert!(
        crate::game::tests_pets2::control_update(c, priest).await,
        "the priest's own again"
    );
}

/// A creature immune to creatures, possessed, has its possessor's side
/// and immunities: it may attack its own kind and they it; the bar's
/// Attack sends it at a guard of its own faction, it strikes him and he
/// fights back. Let go, it is as it was: its own faction, immune to
/// creatures, and no enemy of the guard's.
#[tokio::test]
async fn a_possessed_battlemaster_fights_a_guard_of_its_own_side_and_is_its_own_once_let_go() {
    use crate::game::tests_pets::{enter, learn};
    let realm = crate::game::harness::realm_with(world());
    let (mut a, priest, map) = enter(&realm, "CF1", "Mindone", Race::Human, Class::Priest).await;
    learn(&map, priest, &[MIND_CONTROL]);
    let bm = place(&map, priest, BATTLEMASTER, 0x70_0100, 5.0).await;
    let guard = place(&map, priest, GUARD, 0x70_0101, 6.5).await;
    a.sees(guard).await;
    let mine = on(&map, move |m| side(m, priest)).await;
    let enemies = on(&map, move |m| {
        (can_attack(m, bm, guard), can_attack(m, guard, bm))
    })
    .await;
    assert_eq!(enemies, (false, false), "of one side");
    let (before, during) = mind_control(&mut a, &map, priest, bm, MIND_CONTROL).await;
    assert_eq!(
        before,
        Some((HORDE, UNIT_FLAG_IMMUNE_TO_NPC | UNIT_FLAG_PVP))
    );
    assert_eq!(during, mine, "the priest's side and immunities");
    let enemies = on(&map, move |m| {
        (can_attack(m, bm, guard), can_attack(m, guard, bm))
    })
    .await;
    assert_eq!(enemies, (true, true), "enemies while it is held");
    a.clear();
    command_attack(&a, bm, guard).await;
    assert_eq!(
        feedback(&mut a, Duration::from_millis(300)).await,
        None,
        "nothing refused"
    );
    assert!(
        until(&map, Duration::from_secs(10), move |m| fights(m, bm, guard)
            && hurt(m, guard))
        .await,
        "it strikes the guard"
    );
    assert!(
        until(&map, Duration::from_secs(10), move |m| fights(m, guard, bm)).await,
        "the guard fights back"
    );
    let_go(&mut a, &map, priest, bm, MIND_CONTROL).await;
    let after = on(&map, move |m| {
        (
            side(m, bm),
            can_attack(m, bm, guard),
            can_attack(m, guard, bm),
        )
    })
    .await;
    assert_eq!(after, (before, false, false), "its own side again");
}

/// The priest's own swing, possessing: the possessed unit's (mangos's
/// `HandleAttackSwingOpcode` swings the mover); let go, nothing is held.
#[tokio::test]
async fn a_possessors_swing_is_the_possessed_units() {
    use crate::game::tests_pets::{enter, learn};
    let realm = crate::game::harness::realm_with(world());
    let (mut a, priest, map) = enter(&realm, "CF2", "Mindtwo", Race::Human, Class::Priest).await;
    learn(&map, priest, &[MIND_CONTROL]);
    let bm = place(&map, priest, BATTLEMASTER, 0x70_0110, 5.0).await;
    let guard = place(&map, priest, GUARD, 0x70_0111, 6.5).await;
    a.sees(guard).await;
    mind_control(&mut a, &map, priest, bm, MIND_CONTROL).await;
    a.send(CMSG_ATTACKSWING { guid: guard }).await;
    assert!(
        until(&map, Duration::from_secs(10), move |m| fights(m, bm, guard)
            && hurt(m, guard))
        .await,
        "the battlemaster swings"
    );
    // And swings on: its swing due again, the controlled units' step
    // swings it (mangos's `PossessedAI`), as no AI of its own does.
    let again = on(&map, move |m| {
        let now = m.now;
        if let Some(c) = m.combat_mut(bm) {
            c.next_swing = Some(now);
        }
        pets::update(m);
        m.combat(bm)
            .and_then(|c| c.next_swing)
            .is_some_and(|t| t > now)
    })
    .await;
    assert!(again, "it swings again when its swing is due");
    let own = on(&map, move |m| m.combat(priest).is_some_and(|c| c.attacking)).await;
    assert!(!own, "the priest doesn't");
    let_go(&mut a, &map, priest, bm, MIND_CONTROL).await;
    let held = on(&map, move |m| pets::possessed_by_hand(m, priest)).await;
    assert_eq!(held, None, "nothing held: the swing is his own");
}

mod real {
    use super::*;
    use crate::game::tests_p3_spells_kit::{have_data, player, ready, realm, spawn};

    /// Mind Control's third rank.
    const MIND_CONTROL: u32 = 10912;
    const KARTRA_BLOODSNARL: u32 = 14942;
    const ORGRIMMAR_GRUNT: u32 = 3296;

    /// The user's report: a level 60 Alliance priest's Mind Control on
    /// Kartra Bloodsnarl (Orgrimmar's Alterac Valley battlemaster, immune
    /// to creatures), sent by her bar at an Orgrimmar Grunt: she strikes
    /// him and he fights back; let go, she is Kartra's own again.
    #[tokio::test]
    #[ignore = "extended"]
    async fn kartra_bloodsnarl_mind_controlled_fights_an_orgrimmar_grunt() {
        if !have_data() {
            return;
        }
        let realm = realm();
        let (mut a, priest, map) =
            player(&realm, "CF3", "Mindthree", Race::Human, Class::Priest).await;
        let kartra = on(&map, move |m| {
            // Out in Elwynn, with no one else within a hundred yards to
            // fight the grunt.
            let quiet = Vector3d {
                x: -9250.0,
                y: -1500.0,
                z: 60.0,
            };
            crate::game::map::movement::teleport_within(m, priest, quiet, 0.0);
            crate::game::map::movement::teleport_ack(m, priest);
            ready(m, priest, 60, &[MIND_CONTROL]);
            spawn(m, priest, KARTRA_BLOODSNARL, 61, 17.0)
        })
        .await;
        let (before, during) = mind_control(&mut a, &map, priest, kartra, MIND_CONTROL).await;
        // Beside her, wherever she came to as the priest cast (she runs at
        // him), on her far side: a possessed unit is moved by its
        // possessor's client, not sent running.
        let grunt = on(&map, move |m| spawn(m, kartra, ORGRIMMAR_GRUNT, 55, -2.0)).await;
        let mine = on(&map, move |m| side(m, priest)).await;
        assert_eq!(
            before.map(|s| s.1),
            Some(UNIT_FLAG_IMMUNE_TO_NPC | UNIT_FLAG_PVP),
            "{before:?}"
        );
        assert_eq!(during, mine, "the priest's side and immunities");
        let enemies = on(&map, move |m| {
            (can_attack(m, kartra, grunt), can_attack(m, grunt, kartra))
        })
        .await;
        assert_eq!(enemies, (true, true));
        a.clear();
        command_attack(&a, kartra, grunt).await;
        assert_eq!(feedback(&mut a, Duration::from_millis(300)).await, None);
        assert!(
            until(&map, Duration::from_secs(10), move |m| fights(
                m, kartra, grunt
            ) && hurt(m, grunt))
            .await,
            "Kartra strikes the grunt"
        );
        assert!(
            until(&map, Duration::from_secs(10), move |m| fights(
                m, grunt, kartra
            ))
            .await,
            "the grunt fights back"
        );
        let_go(&mut a, &map, priest, kartra, MIND_CONTROL).await;
        let after = on(&map, move |m| {
            (side(m, kartra), can_attack(m, kartra, grunt))
        })
        .await;
        assert_eq!(after, (before, false), "Kartra's own again");
    }
}
