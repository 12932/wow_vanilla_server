//! Round 8's auras: what an aura's spell attributes hold on its bearer.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{ATTR_EX_PREVENTS_ANIM, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, auras};

const NONE_WITHIN: Duration = Duration::from_millis(300);

/// Sap: a stun whose spell holds the body.
const SAP: u32 = 6770;
/// Dazed: a slow whose spell holds the body too.
const DAZED: u32 = 1604;
/// A stun whose spell doesn't (a Charge's).
const CHARGE_STUN: u32 = 7922;
const FEIGN_DEATH: u32 = 5384;

fn aura_spell(id: u32, aura: u32, amount: i32, attributes_ex: u32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        effects,
        duration_index: 0,
        equipped_item_class: -1,
        ..Default::default()
    };
    s.attributes[1] = attributes_ex;
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.text_emote_anims.insert(0x65, 3);
    for s in [
        aura_spell(SAP, auras::AURA_MOD_STUN, 0, ATTR_EX_PREVENTS_ANIM),
        aura_spell(DAZED, 33, -50, ATTR_EX_PREVENTS_ANIM),
        aura_spell(CHARGE_STUN, auras::AURA_MOD_STUN, 0, 0),
        aura_spell(
            FEIGN_DEATH,
            auras::AURA_FEIGN_DEATH,
            0,
            ATTR_EX_PREVENTS_ANIM,
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn players_realm(d: GameData) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, d, None)
}

async fn on_map<T: Send + 'static>(
    realm: &Arc<Realm>,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    realm
        .maps
        .get(realm, MapKey::world(0))
        .query(f)
        .await
        .expect("the map answers")
}

fn put_on(m: &mut Map, g: Guid, spell: u32) {
    let s = m.realm.data.spells.get(spell).unwrap().clone();
    auras::apply(m, g, g, &s, 0, s.effects[0].base_points + 1);
}

fn held(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .unwrap()
        .has_flag(UNIT_FIELD_FLAGS, auras::UNIT_FLAG_PREVENT_ANIM)
}

/// An emote, a text emote and a posture: what was played or heard of them.
async fn try_to_move(a: &mut Client) -> Option<&'static str> {
    a.clear();
    a.send(CMSG_TEXT_EMOTE {
        text_emote: TextEmote::Wave,
        emote: 0,
        target: Guid::zero(),
    })
    .await;
    a.send(CMSG_EMOTE {
        emote: Emote::OneshotWave,
    })
    .await;
    a.send(CMSG_STANDSTATECHANGE {
        animation_state: UnitStandState::Sit,
    })
    .await;
    a.gets(NONE_WITHIN, |m| match m {
        ServerOpcodeMessage::SMSG_TEXT_EMOTE(_) => Some("a text emote"),
        ServerOpcodeMessage::SMSG_EMOTE(_) => Some("an emote"),
        ServerOpcodeMessage::SMSG_STANDSTATE_UPDATE(_) => Some("a posture"),
        _ => None,
    })
    .await
}

/// A spell with `SPELL_ATTR_EX_PREVENTS_ANIM` (Sap, a stun; Dazed, a slow;
/// Feign Death) holds its bearer's body while its aura lasts, as mangos's
/// `_AddSpellAuraHolder` sets `UNIT_FLAG_PREVENT_ANIM`: an emote, a text
/// emote and a posture are refused (`HandleEmoteOpcode`,
/// `HandleTextEmoteOpcode`, `HandleStandStateChangeOpcode`). The flag goes
/// with the last such aura (`_RemoveSpellAuraHolder`), not with the first:
/// a feign that ends under a Sap leaves the body held. A stun whose spell
/// lacks the attribute holds nothing.
#[tokio::test]
async fn a_spell_that_prevents_animation_holds_emotes_and_postures_until_its_last_aura_goes() {
    let realm = players_realm(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    // A stun without the attribute: free to wave.
    let by_charge = on_map(&realm, move |m| {
        put_on(m, g, CHARGE_STUN);
        (
            auras::auras(m, g).unwrap().has_spell(CHARGE_STUN),
            held(m, g),
        )
    })
    .await;
    assert_eq!(by_charge, (true, false), "a Charge's stun holds no body");
    assert!(try_to_move(&mut a).await.is_some(), "and a wave plays");
    on_map(&realm, move |m| {
        auras::remove_spell(m, g, CHARGE_STUN, None)
    })
    .await;
    // Sapped and feigning: held.
    let sapped = on_map(&realm, move |m| {
        put_on(m, g, SAP);
        let sapped = held(m, g);
        put_on(m, g, FEIGN_DEATH);
        (sapped, held(m, g))
    })
    .await;
    assert_eq!(sapped, (true, true), "Sap and Feign Death hold the body");
    assert_eq!(try_to_move(&mut a).await, None, "nothing plays");
    // The feign ends: the Sap still holds it.
    let after_feign = on_map(&realm, move |m| {
        auras::remove_spell(m, g, FEIGN_DEATH, None);
        held(m, g)
    })
    .await;
    assert!(after_feign, "the Sap still holds it");
    assert_eq!(try_to_move(&mut a).await, None, "still nothing plays");
    // The Sap goes, and a Dazed comes: still held.
    let dazed = on_map(&realm, move |m| {
        put_on(m, g, DAZED);
        auras::remove_spell(m, g, SAP, None);
        held(m, g)
    })
    .await;
    assert!(dazed, "Dazed holds it too");
    assert_eq!(try_to_move(&mut a).await, None, "nothing plays dazed");
    // The last goes: let go.
    let free = on_map(&realm, move |m| {
        auras::remove_spell(m, g, DAZED, None);
        held(m, g)
    })
    .await;
    assert!(!free, "let go with the last");
    a.clear();
    a.send(CMSG_EMOTE {
        emote: Emote::OneshotWave,
    })
    .await;
    a.wait_for("the wave", |m| match m {
        ServerOpcodeMessage::SMSG_EMOTE(e) => (e.emote == Emote::OneshotWave).then_some(()),
        _ => None,
    })
    .await;
}
