//! Round 11j's loot, craft and weapon tests: what wvs already did, shown
//! through the protocol (Thistle Tea's gathering, item and weapon-proc
//! tests).

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::spells::Targets;
use crate::game::tests_loot::{alone, learn};

const ORE: u32 = 2770;
const MINING_PICK: u32 = 2901;
const MINING: u32 = 2575;
const VEIN: u32 = 1731;
const NONE_WITHIN: Duration = Duration::from_millis(400);

fn skill_of(m: &Map, g: Guid, skill: u32) -> u32 {
    m.players
        .get(&g)
        .and_then(|p| crate::game::map::skills::value(p, skill))
        .unwrap_or(0)
}

/// Mine `vein` until it opens (a low skill sometimes fails), each try a
/// fresh cast; whether it opened.
async fn mine(map: &crate::game::map::MapHandle, g: Guid, vein: Guid) -> bool {
    for _ in 0..60 {
        let opened = map
            .query(move |m| {
                m.now = Instant::now();
                if let Some(p) = m.players.get_mut(&g) {
                    p.caster.gcd_until = None;
                    p.caster.casting = None;
                }
                crate::game::map::spells::cast(
                    m,
                    g,
                    MINING,
                    Targets {
                        object: Some(vein),
                        ..Default::default()
                    },
                    None,
                );
                m.players.get(&g).is_some_and(|p| p.looting == Some(vein))
            })
            .await
            .unwrap_or(false);
        if opened {
            return true;
        }
    }
    false
}

/// Thistle Tea's "typed completion grants loot and one stored skill
/// point": a vein opened teaches a point; closed with its ore still in it
/// and opened again before it respawns, it teaches none (mangos's
/// `IsInSkillupList`, wvs's `loot_state.skilled`); its ore is taken once,
/// and the skill learnt is in the character's save.
#[tokio::test]
async fn a_vein_reopened_teaches_no_second_point_and_its_ore_is_taken_once() {
    let (realm, mut a, map) = alone("Alice", Class::Warrior).await;
    let g = a.guid;
    let vein = guid::game_object(VEIN, 50);
    a.sees(vein).await;
    map.query(move |m| {
        learn(m, g, MINING, 186);
        let _ = crate::game::map::inventory::add_item(m, g, MINING_PICK, 1, false);
    })
    .await;
    assert!(mine(&map, g, vein).await, "the vein opens");
    assert_eq!(map.query(move |m| skill_of(m, g, 186)).await, Some(2));
    // Closed with the ore left in it, and opened again.
    a.send(CMSG_LOOT_RELEASE { guid: vein }).await;
    a.wait_for("the window closed", |m| match m {
        ServerOpcodeMessage::SMSG_LOOT_RELEASE_RESPONSE(r) if r.guid == vein => Some(()),
        _ => None,
    })
    .await;
    assert!(mine(&map, g, vein).await, "the vein opens again");
    assert_eq!(
        map.query(move |m| skill_of(m, g, 186)).await,
        Some(2),
        "no second point from the same vein"
    );
    a.clear();
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == ORE => Some(()),
        _ => None,
    })
    .await;
    let ore = map
        .query(move |m| m.players[&g].count_of(ORE))
        .await
        .unwrap();
    assert!(ore >= 1);
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    let again = a
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == ORE => Some(()),
            _ => None,
        })
        .await;
    assert!(again.is_none(), "the ore is taken once");
    assert_eq!(
        map.query(move |m| m.players[&g].count_of(ORE)).await,
        Some(ore)
    );
    // The point is saved with the character.
    a.disconnect();
    crate::game::harness::offline(&realm, g).await;
    let saved = realm.store.load(guid::counter(g)).await.expect("saved");
    let state = &saved.state;
    assert!(
        state.skills.iter().flatten().any(|s| *s == (186, 2)),
        "{:?}",
        state.skills
    );
}

const RECIPE: u32 = 900_001;
const CRAFTED: u32 = 2001;
const BLACKSMITHING: u32 = 164;

/// Cast `spell` at once, a fresh cast (no cooldown or cast left over).
fn cast_now(m: &mut Map, g: Guid, spell: u32) {
    m.now = Instant::now();
    if let Some(p) = m.players.get_mut(&g) {
        p.caster.gcd_until = None;
        p.caster.casting = None;
    }
    crate::game::map::spells::cast(m, g, spell, Targets::default(), None);
}

/// Thistle Tea's "does not gain skill from ordinary grants, missing
/// professions, capped skills, or unique limits": an item given (not made)
/// teaches nothing; the recipe cast without its profession makes its
/// product and teaches nothing (no skill to raise); a unique product
/// already carried isn't made, and its always-orange recipe teaches
/// nothing then (the skill rises only once the product is stored,
/// mangos's `DoCreateItem` then `UpdateCraftSkill`). The trainer's cap is
/// `tests_loot::crafting_raises_the_skill_up_to_the_trainers_cap`.
#[tokio::test]
async fn a_craft_teaches_only_when_its_product_is_stored_by_one_with_the_skill() {
    let mut d = crate::game::tests_loot::data();
    if let Some(t) = d.items.get_mut(&CRAFTED) {
        t.max_count = 1;
    }
    let realm = crate::game::harness::realm_with(d);
    let mut a = crate::game::harness::Client::connect(&realm, "Alice");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    // Without the profession: made, nothing learnt.
    let (made, skill) = map
        .query(move |m| {
            learn(m, g, RECIPE, 0);
            cast_now(m, g, RECIPE);
            (
                m.players[&g].count_of(CRAFTED),
                skill_of(m, g, BLACKSMITHING),
            )
        })
        .await
        .unwrap();
    assert_eq!((made, skill), (1, 0), "made, with no skill to raise");
    // An item given teaches nothing.
    let given = map
        .query(move |m| {
            crate::game::map::skills::learn_skill(m.players.get_mut(&g).unwrap(), BLACKSMITHING, 1);
            let _ = crate::game::map::inventory::add_item(m, g, ORE, 1, false);
            skill_of(m, g, BLACKSMITHING)
        })
        .await
        .unwrap();
    assert_eq!(given, 1);
    // The unique product carried: not made, nothing learnt.
    let (made, skill) = map
        .query(move |m| {
            cast_now(m, g, RECIPE);
            (
                m.players[&g].count_of(CRAFTED),
                skill_of(m, g, BLACKSMITHING),
            )
        })
        .await
        .unwrap();
    assert_eq!((made, skill), (1, 1), "a unique product isn't made twice");
    // Its first gone, the recipe makes it and teaches (always orange).
    let (made, skill) = map
        .query(move |m| {
            let data = m.realm.data.clone();
            if let Some(p) = m.players.get_mut(&g) {
                p.items.retain(|i| i.entry != CRAFTED);
                p.refresh_inventory(&data);
            }
            cast_now(m, g, RECIPE);
            (
                m.players[&g].count_of(CRAFTED),
                skill_of(m, g, BLACKSMITHING),
            )
        })
        .await
        .unwrap();
    assert_eq!((made, skill), (1, 2));
}

// tests_p9_player_items's items and spells.
const OLD: u32 = 63_902;
const NEW: u32 = 63_903;
const REAGENT: u32 = 63_904;
const BLADE: u32 = 63_911;
const TO_NEW: u32 = 96_901;
const POISON: u32 = 43;
const WRISTS: u8 = 9;

/// Thistle Tea's "keeps source, reagents, and projections unchanged on
/// invalid equipment": worn bracers whose change is into bracers a
/// warrior can't wear are refused the change (`extra_effects::change_item`'s
/// `inventory::may_place`, mangos's `CanEquipItem` in
/// `ConvertItem`): the bracers kept as they were, worn, no new item, and
/// the stats unchanged. Its reagents are spent, unlike Thistle Tea's: they
/// go as the cast completes, before its effects (mangos's `Spell::cast`
/// takes them before `HandleEffects`, and `ConvertItem` refused gives
/// nothing back).
#[tokio::test]
async fn a_change_into_what_cant_be_worn_keeps_the_worn_item() {
    use crate::game::fields::*;
    use crate::game::store::BAG_PLAYER;
    use crate::game::tests_p9_player_items::{give_at, holds, on};
    let mut d = crate::game::tests_p9_player_items::data();
    if let Some(t) = d.items.get_mut(&NEW) {
        // Mages' alone.
        t.allowable_class = 128;
    }
    if let Some(s) = d.spells.spells.get_mut(&TO_NEW) {
        s.reagents[0] = (REAGENT, 2);
    }
    let realm = crate::game::harness::realm_with(d);
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let (old, before) = on(&map, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
        p.gm = false;
        let old = give_at(m, g, OLD, 1, (BAG_PLAYER, WRISTS));
        give_at(
            m,
            g,
            REAGENT,
            2,
            (BAG_PLAYER, crate::game::map::inventory::PACK_START),
        );
        let p = m.players.get_mut(&g).unwrap();
        p.apply_stats(&d);
        (old, p.fields.get_u32(UNIT_FIELD_STAT0 + 2))
    })
    .await;
    a.clear();
    on(&map, move |m| {
        crate::game::map::spells::cast(
            m,
            g,
            TO_NEW,
            Targets {
                unit: Some(g),
                ..Default::default()
            },
            Some(old),
        );
    })
    .await;
    a.wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(_) => Some(()),
        _ => None,
    })
    .await;
    let kept = on(&map, move |m| {
        let p = &m.players[&g];
        (
            holds(m, g, (BAG_PLAYER, WRISTS)),
            p.item_by_guid(old).is_some(),
            p.count_of(NEW),
            p.count_of(REAGENT),
            p.fields.get_u32(UNIT_FIELD_STAT0 + 2),
        )
    })
    .await;
    assert_eq!(kept, (Some((OLD, 1)), true, 0, 0, before));
}

/// Thistle Tea's "innate and enchantment procs share one hit and spend
/// one charge": a blade whose own chance on hit is sure, coated with a
/// poison of two charges, sets off both on a blow, a charge spent; both
/// again, the poison gone; then the blade's own alone.
#[tokio::test]
async fn a_blades_own_proc_and_its_poison_share_a_blow_and_a_charge() {
    use crate::game::map::enchant;
    use crate::game::tests_p3_combat::{BRUTE, equip, level, spawn};
    use crate::game::tests_p9_player_items::{blow, on, setup};
    let (_realm, _a, g, map) = setup().await;
    let seen = on(&map, move |m| {
        level(m, g, 60);
        equip(m, g, BLADE, crate::game::map::inventory::SLOT_MAIN_HAND);
        let w = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == BLADE)
            .unwrap()
            .guid;
        enchant::enchant_item(m, g, w, POISON, Some((60_000, 2)));
        let brute = spawn(m, g, BRUTE, 60, 3.0);
        let mut seen = Vec::new();
        for _ in 0..3 {
            let hit = blow(m, g, brute);
            let p = &m.players[&g];
            let i = p.item_by_guid(w).unwrap();
            let e = enchant::get(&p.items[i], enchant::TEMP_SLOT);
            seen.push((hit, e.id, e.charges));
        }
        seen
    })
    .await;
    assert_eq!(
        seen,
        [(2, POISON, 1), (2, 0, 0), (1, 0, 0)],
        "both a blow, a charge each; then the blade's own"
    );
}
