//! Phase 3's reputation, Thistle Tea's `ReputationTest` ported: a hostile
//! standing at war from the start, a standing held within its bounds, war
//! once hostile, the hidden kept hidden, spillover only up to its rank, a
//! gain's dithering and its bonus kept off losses, who may be seen, set at
//! war or inactive, and a fight's war that outlasts it once hostile.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::misc::{FactionDef, Spillover};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::harness::{self, Client};
use crate::game::map::reputation::{
    self, CMSG_SET_FACTION_ATWAR, CMSG_SET_FACTION_INACTIVE, FLAG_AT_WAR, FLAG_INACTIVE,
    FLAG_VISIBLE,
};
use crate::game::map::{Map, MapKey, auras};

const HOSTILE: u32 = 529;
const NEARLY_HOSTILE: u32 = 87;
const HIDDEN: u32 = 1;
const INVISIBLE: u32 = 2;
const PEACE: u32 = 3;
const ORDINARY: u32 = 4;
const HIGH: u32 = 5;
const AT_WAR: u32 = 6;
const SHOWN_INACTIVE: u32 = 7;
const SOURCE: u32 = 469;
const SPILLED: u32 = 72;
const UNRATED: u32 = 8;
const BONUS: u32 = 9_400_200;

/// A faction for humans, at `index`, of a base and flags.
fn faction(id: u32, index: u32, base: i32, flags: u32) -> FactionDef {
    FactionDef {
        id,
        index,
        parent: 0,
        name: format!("Faction {id}"),
        variants: [
            (1, 0, base, flags),
            (0, 0, 0, 0),
            (0, 0, 0, 0),
            (0, 0, 0, 0),
        ],
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    for f in [
        faction(HOSTILE, 13, -6_000, 0),
        faction(NEARLY_HOSTILE, 0, -2_900, 0),
        faction(HIDDEN, 1, 0, 0x04),
        faction(INVISIBLE, 2, 0, 0x08),
        faction(PEACE, 3, 0, 0x11),
        faction(ORDINARY, 4, 0, 0),
        faction(HIGH, 5, 3_100, 0),
        faction(AT_WAR, 6, 0, 0x03),
        faction(SHOWN_INACTIVE, 7, 0, 0x21),
        faction(SOURCE, 11, 0, 0),
        faction(SPILLED, 19, 3_000, 0),
        faction(UNRATED, 8, 0, 0),
    ] {
        d.misc.faction_by_index.insert(f.index, f.id);
        d.misc.factions.insert(f.id, f);
    }
    d.misc.spillovers.insert(
        SOURCE,
        vec![Spillover {
            faction: SPILLED,
            rate: 0.25,
            max_rank: crate::game::data::misc::RANK_NEUTRAL,
        }],
    );
    // No spell gains with this one.
    d.misc.reward_rates.insert(UNRATED, (1.0, 1.0, 0.0));
    d
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

async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    (realm, a, g)
}

/// A faction's total standing and flags for `g`.
async fn state(realm: &Arc<Realm>, g: Guid, faction: u32) -> (i32, u32) {
    on_map(realm, move |m| {
        let d = m.realm.data.clone();
        let p = &m.players[&g];
        let flags = p
            .row
            .state
            .reputation
            .iter()
            .find(|s| s.faction == faction)
            .map_or(0, |s| s.flags);
        (reputation::standing(p, &d, faction), flags)
    })
    .await
}

async fn modify(realm: &Arc<Realm>, g: Guid, faction: u32, delta: i32) {
    on_map(realm, move |m| {
        reputation::modify(m, g, faction, delta, true)
    })
    .await;
}

async fn handled(a: &mut Client) {
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => {
            Some(())
        }
        _ => None,
    })
    .await;
}

async fn faction_flag(a: &mut Client, opcode: u16, index: u32, on: bool) {
    let mut body = index.to_le_bytes().to_vec();
    body.push(u8::from(on));
    a.send_raw(opcode, &body).await;
    handled(a).await;
}

/// Thistle Tea's `normalize/4` and `modify/6`: a hostile base at war from
/// the start; a standing held at the most, its offset from the base; war
/// once a loss makes it hostile; hidden and forced-invisible factions not
/// shown by a change; spillover only to a faction at or under its rank.
#[tokio::test]
async fn standings_stay_in_bounds_turn_to_war_and_keep_hidden() {
    let (realm, _a, g) = setup().await;
    assert_ne!(state(&realm, g, HOSTILE).await.1 & FLAG_AT_WAR, 0);

    modify(&realm, g, HIGH, 100_000).await;
    assert_eq!(state(&realm, g, HIGH).await.0, 42_999);
    let offset = on_map(&realm, move |m| {
        m.players[&g]
            .row
            .state
            .reputation
            .iter()
            .find(|s| s.faction == HIGH)
            .unwrap()
            .standing
    })
    .await;
    assert_eq!(offset, 42_999 - 3_100);

    assert_eq!(state(&realm, g, NEARLY_HOSTILE).await.1 & FLAG_AT_WAR, 0);
    modify(&realm, g, NEARLY_HOSTILE, -200).await;
    assert_ne!(state(&realm, g, NEARLY_HOSTILE).await.1 & FLAG_AT_WAR, 0);

    for f in [HIDDEN, INVISIBLE] {
        modify(&realm, g, f, 10).await;
        assert_eq!(state(&realm, g, f).await.1 & FLAG_VISIBLE, 0, "{f}");
    }
    modify(&realm, g, ORDINARY, 10).await;
    assert_ne!(state(&realm, g, ORDINARY).await.1 & FLAG_VISIBLE, 0);

    // Friendly already: past the spillover's cap of neutral.
    modify(&realm, g, SOURCE, 100).await;
    assert_eq!(state(&realm, g, SOURCE).await.0, 100);
    assert_eq!(state(&realm, g, SPILLED).await.0, 3_000);
}

/// Thistle Tea's `calculate_gain/5`: a gain raised by a bonus aura and
/// dithered to a whole, a loss without the bonus, none at a zero rate.
#[tokio::test]
async fn a_gain_takes_its_bonus_and_a_loss_does_not() {
    let (realm, _a, g) = setup().await;
    on_map(&realm, move |m| {
        let mut s = SpellTemplate {
            id: BONUS,
            name: "Diplomacy".into(),
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect: 6,
            aura: 156,
            base_points: 9,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        };
        let _ = auras::apply_spell(m, g, g, &s, &[(0, 10)], auras::Context::default());
    })
    .await;
    on_map(&realm, move |m| {
        reputation::spell_reward(m, g, ORDINARY, 25)
    })
    .await;
    let gained = state(&realm, g, ORDINARY).await.0;
    assert!(gained == 27 || gained == 28, "27.5 dithered: {gained}");
    on_map(&realm, move |m| reputation::spell_reward(m, g, HIGH, -25)).await;
    assert_eq!(
        state(&realm, g, HIGH).await.0,
        3_100 - 25,
        "no bonus on a loss"
    );
    on_map(&realm, move |m| reputation::spell_reward(m, g, UNRATED, 25)).await;
    assert_eq!(state(&realm, g, UNRATED).await.0, 0, "a zero rate");
}

/// Thistle Tea's `set_visible/2`, `set_at_war/5` and `set_inactive/3`: an
/// ordinary faction shown, not a hidden or forced-invisible one; no war on
/// a hidden faction or one forced to peace; inactive only once shown, and
/// active again at any time.
#[tokio::test]
async fn what_a_player_may_show_set_at_war_or_put_away() {
    let (realm, mut a, g) = setup().await;
    on_map(&realm, move |m| {
        for f in [ORDINARY, HIDDEN, INVISIBLE] {
            reputation::set_visible(m, g, f);
        }
    })
    .await;
    assert_ne!(state(&realm, g, ORDINARY).await.1 & FLAG_VISIBLE, 0);
    assert_eq!(state(&realm, g, HIDDEN).await.1 & FLAG_VISIBLE, 0);
    assert_eq!(state(&realm, g, INVISIBLE).await.1 & FLAG_VISIBLE, 0);

    for (f, index) in [(HIDDEN, 1), (PEACE, 3)] {
        faction_flag(&mut a, CMSG_SET_FACTION_ATWAR, index, true).await;
        assert_eq!(state(&realm, g, f).await.1 & FLAG_AT_WAR, 0, "{f}");
    }
    faction_flag(&mut a, CMSG_SET_FACTION_ATWAR, 4, true).await;
    assert_ne!(state(&realm, g, ORDINARY).await.1 & FLAG_AT_WAR, 0);
    faction_flag(&mut a, CMSG_SET_FACTION_ATWAR, 4, false).await;
    assert_eq!(state(&realm, g, ORDINARY).await.1 & FLAG_AT_WAR, 0);

    // Not shown: not put away. Shown and put away: back again.
    faction_flag(&mut a, CMSG_SET_FACTION_INACTIVE, 5, true).await;
    assert_eq!(state(&realm, g, HIGH).await.1 & FLAG_INACTIVE, 0);
    faction_flag(&mut a, CMSG_SET_FACTION_INACTIVE, 7, false).await;
    assert_eq!(state(&realm, g, SHOWN_INACTIVE).await.1 & FLAG_INACTIVE, 0);
}

/// Thistle Tea's temporary war: a fight's war with a faction ends with
/// the fight unless the faction is hostile by then; and no fight's war
/// with a hidden faction, one forced to peace, or one at war already.
#[tokio::test]
async fn a_fights_war_ends_with_it_unless_the_faction_is_hostile() {
    let (realm, _a, g) = setup().await;
    // Through a creature of each faction.
    let marks = on_map(&realm, move |m| {
        let mut marked = Vec::new();
        for f in [ORDINARY, HIDDEN, PEACE, AT_WAR] {
            reputation::temporary_war_with(m, g, f);
            marked.push(m.players[&g].temporary_war.contains(&f));
        }
        marked
    })
    .await;
    assert_eq!(marks, vec![true, false, false, false]);
    assert_ne!(state(&realm, g, ORDINARY).await.1 & FLAG_AT_WAR, 0);
    // Hostile by the fight's end: still at war.
    modify(&realm, g, ORDINARY, -6_000).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        reputation::end_temporary_war(m.players.get_mut(&g).unwrap(), &d);
    })
    .await;
    assert_ne!(state(&realm, g, ORDINARY).await.1 & FLAG_AT_WAR, 0);
    // A fight's war with a friendly one ends with it.
    on_map(&realm, move |m| {
        reputation::temporary_war_with(m, g, SPILLED);
        let d = m.realm.data.clone();
        reputation::end_temporary_war(m.players.get_mut(&g).unwrap(), &d);
    })
    .await;
    assert_eq!(state(&realm, g, SPILLED).await.1 & FLAG_AT_WAR, 0);
}
