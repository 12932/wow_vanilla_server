//! Thistle Tea's pet message tests (Phase 3, `game/network/message`),
//! through the protocol with the pets' own world (`tests_pets::data`): a
//! pet's bar and spells, its autocast set by either message, commands, a
//! cancelled aura, the info asked again; a hunter's pet renamed badly,
//! untrained, and run off.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellTemplate;
use crate::game::harness;
use crate::game::map::pets::{self, ACT_COMMAND, ACT_ENABLED, ACT_PASSIVE, COMMAND_ATTACK};
use crate::game::tests_pets::{
    FIREBOLT, SUMMON_IMP, WOLF, cast, data, effect, enter, learn, pet_bar, place, spell, tamed,
};

/// A passive spell an imp may know.
const IMP_PASSIVE: u32 = 900_501;
/// A helpful aura on the imp.
const WARD: u32 = 900_502;

fn unpack(b: u32) -> (u32, u32) {
    (b & 0x00FF_FFFF, b >> 24)
}

fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// `SMSG_PET_SPELLS`' known spells: (id, kind), after the 56 bytes of the
/// GUID, duration, react, command, two bytes and the ten buttons.
fn known(b: &[u8]) -> Vec<(u32, u32)> {
    let n = usize::from(b[56]);
    (0..n).map(|i| unpack(word(b, 57 + i * 4))).collect()
}

/// Thistle Tea's pet_messages_test: an imp's bar and its known spells, a
/// passive one listed after the bar's four with the passive kind; its
/// autocast turned off by `CMSG_PET_SPELL_AUTOCAST` (not for a spell it
/// doesn't know) and on by `CMSG_PET_SET_ACTION`'s 0xC1 button; the bar
/// sent again when asked; an attack without a target refused with
/// `NOTHING_TO_ATTACK`; an attack stopped; a helpful aura cancelled.
#[tokio::test]
async fn an_imps_bar_autocast_commands_and_auras_go_as_1_12_has_them() {
    let mut d = data();
    let mut passive: SpellTemplate = spell(
        IMP_PASSIVE,
        "Imp Skin",
        [effect(6, 0, 1), effect(0, 0, 0), effect(0, 0, 0)],
    );
    passive.attributes[0] = crate::game::data::spells::ATTR_PASSIVE;
    d.spells.spells.insert(IMP_PASSIVE, passive);
    let mut ward = spell(
        WARD,
        "Ward",
        [effect(6, 5, 1), effect(0, 0, 0), effect(0, 0, 0)],
    );
    ward.effects[0].aura = crate::game::map::auras::AURA_MOD_STAT;
    d.spells.spells.insert(WARD, ward);
    let realm = harness::realm_with(d);
    let (mut w, gw, map) = enter(&realm, "W", "Warlock", Race::Human, Class::Warlock).await;
    learn(&map, gw, &[SUMMON_IMP]);
    let wolf = place(&map, gw, WOLF, 0x70_0003, 18.0).await;
    w.sees(wolf).await;
    cast(&w, SUMMON_IMP, None).await;
    let imp = pet_bar(&mut w).await;
    assert!(!imp.is_zero());
    let bar = w.wait_frame("SMSG_PET_SPELLS", 0x179, |_| true).await;
    assert_eq!(bar.len(), 62, "one spell, no cooldowns");
    assert_eq!(u64::from_le_bytes(bar[..8].try_into().unwrap()), imp.guid());
    assert_eq!(unpack(word(&bar, 16 + 3 * 4)), (FIREBOLT, ACT_ENABLED));
    assert_eq!(
        known(&bar),
        vec![(FIREBOLT, ACT_ENABLED)],
        "Firebolt, cast of its own accord"
    );
    // A passive learned: known, after the bar's spells, never on the bar.
    map.query(move |m| pets::learn(m, imp, IMP_PASSIVE)).await;
    let bar = w.wait_frame("SMSG_PET_SPELLS", 0x179, |_| true).await;
    assert_eq!(
        known(&bar),
        vec![(FIREBOLT, ACT_ENABLED), (IMP_PASSIVE, ACT_PASSIVE)]
    );
    assert!((0..10).all(|i| unpack(word(&bar, 16 + i * 4)).0 != IMP_PASSIVE));
    // Autocast off, a spell it doesn't know ignored, then on by its button.
    let autocast =
        move |m: &mut crate::game::map::Map| pets::state(m, imp).unwrap().autocast.clone();
    w.send(CMSG_PET_SPELL_AUTOCAST {
        guid: imp,
        id: FIREBOLT,
        autocast_enabled: false,
    })
    .await;
    w.send(CMSG_PET_SPELL_AUTOCAST {
        guid: imp,
        id: 999,
        autocast_enabled: true,
    })
    .await;
    w.send(CMSG_REQUEST_PET_INFO {}).await;
    let bar = w
        .wait_frame("SMSG_PET_SPELLS asked again", 0x179, |_| true)
        .await;
    assert_eq!(map.query(autocast).await.unwrap(), Vec::<u32>::new());
    assert_eq!(
        known(&bar)[0],
        (FIREBOLT, pets::ACT_DISABLED),
        "no longer of its own accord"
    );
    w.send(CMSG_PET_SET_ACTION {
        guid: imp,
        position1: 3,
        data1: pets::button(FIREBOLT, ACT_ENABLED),
        extra: None,
    })
    .await;
    w.send(CMSG_REQUEST_PET_INFO {}).await;
    let bar = w
        .wait_frame("SMSG_PET_SPELLS after the button", 0x179, |_| true)
        .await;
    assert_eq!(map.query(autocast).await.unwrap(), vec![FIREBOLT]);
    assert_eq!(unpack(word(&bar, 16 + 3 * 4)), (FIREBOLT, ACT_ENABLED));
    // Attack with no target: nothing to attack.
    w.send(CMSG_PET_ACTION {
        pet: imp,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: Guid::zero(),
    })
    .await;
    let feedback = w
        .wait_frame("SMSG_PET_ACTION_FEEDBACK", 0x2C6, |_| true)
        .await;
    assert_eq!(feedback, vec![2], "NOTHING_TO_ATTACK");
    // At the wolf, then stopped.
    w.send(CMSG_PET_ACTION {
        pet: imp,
        data: pets::button(u32::from(COMMAND_ATTACK), ACT_COMMAND),
        target: wolf,
    })
    .await;
    let mut attacking = false;
    for _ in 0..200 {
        attacking = map
            .query(move |m| m.creature(imp).is_some_and(|c| c.combat.attacking))
            .await
            .unwrap();
        if attacking {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(attacking, "the imp attacks");
    w.send(CMSG_PET_STOP_ATTACK { pet: imp }).await;
    w.send(CMSG_QUERY_TIME {}).await;
    w.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
    let stopped = map
        .query(move |m| m.creature(imp).is_some_and(|c| !c.combat.attacking))
        .await
        .unwrap();
    assert!(stopped, "stopped");
    // A helpful aura on the imp, cancelled by its master.
    let ward = realm.data.spells.get(WARD).cloned().unwrap();
    let on = map
        .query(move |m| {
            crate::game::map::auras::apply(m, gw, imp, &ward, 0, 5);
            crate::game::map::auras::auras(m, imp).is_some_and(|a| a.has_spell(WARD))
        })
        .await
        .unwrap();
    assert!(on, "the ward on the imp");
    w.send(CMSG_PET_CANCEL_AURA {
        guid: imp,
        id: WARD,
    })
    .await;
    w.send(CMSG_QUERY_TIME {}).await;
    w.wait_for("SMSG_QUERY_TIME_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_QUERY_TIME_RESPONSE(_)).then_some(())
    })
    .await;
    let warded = map
        .query(move |m| crate::game::map::auras::auras(m, imp).is_some_and(|a| a.has_spell(WARD)))
        .await
        .unwrap();
    assert!(!warded, "the ward cancelled");
}

/// Thistle Tea's pet_messages_test and pet_untraining_test: a hunter's
/// pet named badly is refused with an empty `SMSG_PET_NAME_INVALID`; its
/// untraining's price is quoted with its GUID; and a bond broken sends it
/// off with an empty `SMSG_PET_BROKEN`.
#[tokio::test]
async fn a_hunters_pet_is_named_untrained_and_runs_off_as_1_12_has_it() {
    let (_realm, mut h, gh, map, pet, _wolf) = tamed().await;
    h.send(CMSG_PET_RENAME {
        pet,
        name: "x1".into(),
    })
    .await;
    let invalid = h.wait_frame("SMSG_PET_NAME_INVALID", 0x178, |_| true).await;
    assert!(invalid.is_empty());
    map.query(move |m| pets::untrain_offer(m, gh)).await;
    let quote = h
        .wait_frame("SMSG_PET_UNLEARN_CONFIRM", 0x2F1, |_| true)
        .await;
    assert_eq!(&quote[..8], &pet.guid().to_le_bytes());
    assert_eq!(quote.len(), 12, "the GUID and the price");
    map.query(move |m| pets::change_loyalty(m, pet, -1_000_000))
        .await;
    let broken = h.wait_frame("SMSG_PET_BROKEN", 0x2AF, |_| true).await;
    assert!(broken.is_empty());
    let gone = map.query(move |m| m.creature(pet).is_none()).await.unwrap();
    assert!(gone, "run off");
}
