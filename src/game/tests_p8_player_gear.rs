//! Round 8's player: gear put on and taken off.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::MapHandle;
use crate::game::map::inventory::{PACK_START, SLOT_MAIN_HAND};
use crate::game::store::BAG_PLAYER;

const TRINKET: u32 = 20_100;
/// (The initial spells' cooldowns carry 16-bit spell ids.)
const TRINKET_SPELL: u32 = 54_100;
const SWORD: u32 = 20_101;
const OTHER_SWORD: u32 = 20_102;
/// The weapon switch's global cooldown, a second and a half.
const SWITCH: u32 = 6119;
const PACK: (u8, u8) = (BAG_PLAYER, PACK_START);
const PACK2: (u8, u8) = (BAG_PLAYER, PACK_START + 1);

fn data() -> GameData {
    let mut d = GameData::empty();
    let item = |entry: u32, class: u32, subclass: u32, inventory_type: u32| ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000 + entry,
        class,
        subclass,
        inventory_type,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    };
    let mut trinket = item(TRINKET, 4, 0, 12);
    trinket.spells[0] = ItemSpell {
        id: TRINKET_SPELL,
        trigger: 0,
        cooldown: 120_000,
        ..Default::default()
    };
    d.items.insert(TRINKET, trinket);
    for e in [SWORD, OTHER_SWORD] {
        let mut sword = item(e, 2, 7, 13);
        sword.delay = 3_400;
        sword.damages[0] = (5.0, 10.0, 0);
        d.items.insert(e, sword);
    }
    let mut use_spell = SpellTemplate {
        id: TRINKET_SPELL,
        name: "Trinket".into(),
        range_index: 1,
        equipped_item_class: -1,
        ..Default::default()
    };
    use_spell.effects[0] = SpellEffect {
        effect: 3,
        target_a: 1,
        ..Default::default()
    };
    d.spells.spells.insert(TRINKET_SPELL, use_spell);
    d.spells.spells.insert(
        SWITCH,
        SpellTemplate {
            id: SWITCH,
            name: "Weapon Switch".into(),
            start_recovery_time: 1_500,
            start_recovery_category: 133,
            equipped_item_class: -1,
            ..Default::default()
        },
    );
    d
}

/// Nothing carried but what a test gives.
async fn bare(map: &MapHandle, g: Guid) {
    map.query(move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
    })
    .await
    .unwrap();
}

async fn give(map: &MapHandle, g: Guid, entry: u32, at: (u8, u8)) -> Guid {
    map.query(move |m| {
        let d = m.realm.data.clone();
        let counter = m
            .realm
            .next_item
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let item = crate::game::guid::item(counter);
        let p = m.players.get_mut(&g).unwrap();
        p.items.push(crate::game::map::player::Item::new(
            &d, item, g, entry, at.0, at.1, 1,
        ));
        p.refresh_inventory(&d);
        item
    })
    .await
    .unwrap()
}

/// The refusal `a` was sent since it last looked, by its name.
async fn refusal(a: &mut Client) -> Option<String> {
    a.send(CMSG_NAME_QUERY { guid: a.guid }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_)).then_some(())
    })
    .await;
    let r = a
        .gets(Duration::from_millis(1), |m| match m {
            ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(f) => {
                Some(format!("{:?}", f.result))
            }
            _ => None,
        })
        .await;
    a.clear();
    r.map(|r| r.split([' ', '{']).next().unwrap().to_owned())
}

/// Thistle Tea's equipment_transitions_test "commits and projects an equip
/// cooldown that also rejects actual item use" and "change sets share the
/// cooldown transition": a trinket put on starts its spell's 30 seconds
/// (`SMSG_ITEM_COOLDOWN`), a use meanwhile is refused as not ready and
/// spends nothing, and the wait is still there after a relog.
#[tokio::test]
async fn a_trinket_put_on_waits_its_thirty_seconds_through_a_use_and_a_relog() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    bare(&map, g).await;
    let trinket = give(&map, g, TRINKET, PACK).await;
    a.clear();
    a.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: PACK.0,
        source_slot: PACK.1,
    })
    .await;
    a.wait_for("SMSG_ITEM_COOLDOWN", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_COOLDOWN(c) if c.id == TRINKET_SPELL => Some(c.guid),
        _ => None,
    })
    .await;
    let worn = map
        .query(move |m| {
            m.players[&g]
                .item_at(BAG_PLAYER, 12)
                .map(|i| m.players[&g].items[i].guid)
        })
        .await
        .unwrap();
    assert_eq!(worn, Some(trinket));
    a.send(CMSG_USE_ITEM {
        bag_index: BAG_PLAYER,
        bag_slot: 12,
        spell_index: 0,
        targets: crate::game::map::spells::wire_targets(&Default::default()),
    })
    .await;
    let b = a
        .wait_frame(
            "SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |b| b[..4] == TRINKET_SPELL.to_le_bytes(),
        )
        .await;
    assert_eq!(
        (b[4], b[5]),
        (2, SpellCastResult::NotReady.as_int()),
        "refused, not ready"
    );
    let casting = map
        .query(move |m| m.players[&g].caster.casting.is_some())
        .await
        .unwrap();
    assert!(!casting);
    // Logged out and in, it waits on.
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    a.login(g).await;
    // Told in the initial spells' cooldowns, as mangos tells them.
    let told = a
        .wait_for("SMSG_INITIAL_SPELLS", |m| match m {
            ServerOpcodeMessage::SMSG_INITIAL_SPELLS(s) => s
                .cooldowns
                .iter()
                .find(|c| u32::from(c.spell_id) == TRINKET_SPELL)
                .map(|c| c.cooldown.as_millis()),
            _ => None,
        })
        .await;
    assert!(told > 25_000 && told <= 30_000, "still waiting: {told}");
    let map = realm.registry.online(g).unwrap().map();
    let left = map
        .query(move |m| {
            m.players[&g]
                .caster
                .cooldowns
                .get(&TRINKET_SPELL)
                .map(|t| t.saturating_duration_since(m.now))
        })
        .await
        .unwrap();
    assert!(
        left.is_some_and(|d| d > Duration::from_secs(25)),
        "{left:?}"
    );
}

/// Thistle Tea's equipment_transitions_test "a combat weapon swap
/// publishes the GCD and rejects a second swap": a weapon put on in a
/// fight sends the weapon switch's cooldown (6119), starts its hand's
/// swing a whole period of it away, and another weapon change is refused
/// meanwhile (`CANT_DO_RIGHT_NOW`).
#[tokio::test]
async fn a_weapon_put_on_in_a_fight_sends_the_switch_and_holds_the_next() {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    bare(&map, g).await;
    let sword = give(&map, g, SWORD, PACK).await;
    let other = give(&map, g, OTHER_SWORD, PACK2).await;
    map.query(move |m| {
        let p = m.players.get_mut(&g).unwrap();
        p.fields.set_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            true,
        );
        let d = m.realm.data.clone();
        m.players.get_mut(&g).unwrap().refresh_inventory(&d);
    })
    .await
    .unwrap();
    a.clear();
    a.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: PACK.0,
        source_slot: PACK.1,
    })
    .await;
    a.wait_for("SMSG_SPELL_COOLDOWN", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(c)
            if c.guid == g && c.cooldowns.iter().any(|x| x.id == SWITCH) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    let (worn, swing) = map
        .query(move |m| {
            let p = &m.players[&g];
            let worn = p
                .item_at(BAG_PLAYER, SLOT_MAIN_HAND)
                .map(|i| p.items[i].guid);
            let swing = m
                .combat(g)
                .and_then(|c| c.next_swing)
                .map(|t| t.saturating_duration_since(m.now));
            (worn, swing)
        })
        .await
        .unwrap();
    assert_eq!(worn, Some(sword));
    assert!(
        swing
            .is_some_and(|d| d > Duration::from_millis(3_000) && d <= Duration::from_millis(3_400)),
        "the swing a period of the sword away: {swing:?}"
    );
    a.send(CMSG_SWAP_INV_ITEM {
        source_slot: ItemSlot::try_from(PACK2.1).unwrap(),
        destination_slot: ItemSlot::try_from(SLOT_MAIN_HAND).unwrap(),
    })
    .await;
    assert_eq!(refusal(&mut a).await.as_deref(), Some("CantDoRightNow"));
    let still = map
        .query(move |m| {
            let p = &m.players[&g];
            p.item_by_guid(other)
                .map(|i| (p.items[i].bag, p.items[i].slot))
        })
        .await
        .unwrap();
    assert_eq!(still, Some(PACK2), "the other sword where it was");
}

const TALENT: u32 = 94_101;
const ARMORER: u32 = 20_104;
const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const HELM: u32 = 20_103;

/// A world with a helm of 100 points and a sword-only talent: 5% crit and
/// 3% hit with a sword (subclass 7).
fn worn_data() -> GameData {
    let mut d = data();
    d.items.insert(
        HELM,
        ItemTemplate {
            entry: HELM,
            name: "Helm".into(),
            display_id: 1,
            class: 4,
            subclass: 1,
            inventory_type: 1,
            max_durability: 100,
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    if let Some(s) = d.items.get_mut(&SWORD) {
        s.max_durability = 50;
    }
    let mut talent = SpellTemplate {
        id: TALENT,
        name: "Sword Talent".into(),
        range_index: 1,
        duration_index: 21,
        equipped_item_class: 2,
        equipped_item_subclass_mask: 1 << 7,
        ..Default::default()
    };
    for (i, (aura, amount)) in [
        (crate::game::map::auras::AURA_MOD_CRIT_PERCENT, 5),
        (crate::game::map::auras::AURA_MOD_HIT_CHANCE, 3),
    ]
    .into_iter()
    .enumerate()
    {
        talent.effects[i] = SpellEffect {
            effect: 6,
            aura,
            base_points: amount - 1,
            base_dice: 1,
            die_sides: 1,
            target_a: 1,
            ..Default::default()
        };
    }
    d.spells.spells.insert(TALENT, talent);
    // An armorer beside the start, a sword's point a copper to mend.
    d.creature_templates.insert(
        ARMORER,
        CreatureTemplate {
            entry: ARMORER,
            name: "Armorer".into(),
            display_ids: [903, 0, 0, 0],
            faction: 35,
            npc_flags: crate::game::map::vendor::NPC_FLAG_VENDOR
                | crate::game::map::vendor::NPC_FLAG_REPAIR,
            ..Default::default()
        },
    );
    d.creature_spawns.entry(0).or_default().push(CreatureSpawn {
        guid: 1,
        entries: vec![ARMORER],
        map: 0,
        x: START.0 + 2.0,
        y: START.1,
        z: START.2,
        ..Default::default()
    });
    let mut costs = vec![0; 29];
    costs[7] = 1;
    d.durability_costs.insert(0, costs);
    d
}

/// A worn item's durability.
async fn durability(map: &MapHandle, g: Guid, item: Guid) -> u32 {
    map.query(move |m| {
        let p = &m.players[&g];
        p.items[p.item_by_guid(item).unwrap()]
            .fields
            .get_u32(ITEM_FIELD_DURABILITY)
    })
    .await
    .unwrap()
}

/// Put on at `slot` what is at `at`, out of a fight.
async fn wear_at(map: &MapHandle, g: Guid, item: Guid, slot: u8) {
    map.query(move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(item).unwrap();
        p.items[i].bag = BAG_PLAYER;
        p.items[i].slot = slot;
        p.gear_dirty = true;
        let d = m.realm.data.clone();
        m.players.get_mut(&g).unwrap().refresh_inventory(&d);
    })
    .await
    .unwrap();
}

/// Thistle Tea's durability_test "weapon talents stop at breakage and
/// return with repaired equipment": a sword talent's crit goes with the
/// sword broken, and comes back mended; its hit chance stays, as mangos's
/// `_ApplyWeaponDependentAuraMods` takes only the crit (and damage) off a
/// broken weapon.
#[tokio::test]
async fn a_weapon_talents_crit_stops_while_the_weapon_is_broken() {
    let realm = harness::realm_with(worn_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    bare(&map, g).await;
    let sword = give(&map, g, SWORD, PACK).await;
    wear_at(&map, g, sword, SLOT_MAIN_HAND).await;
    map.query(move |m| crate::game::map::spells::cast_triggered(m, g, TALENT, Some(g)))
        .await
        .unwrap();
    let stats = async || {
        harness::an_update(&map).await;
        map.query(move |m| {
            let d = &m.players[&g].derived;
            (d.crit, d.hit)
        })
        .await
        .unwrap()
    };
    let (whole_crit, whole_hit) = stats().await;
    // Worn through, as a death's wear or a blow's wears it.
    map.query(move |m| crate::game::map::vendor::wear(m, g, 1.0))
        .await
        .unwrap();
    assert_eq!(durability(&map, g, sword).await, 0);
    let (broken_crit, broken_hit) = stats().await;
    assert!(
        (whole_crit - broken_crit - 5.0).abs() < 0.01,
        "the talent's 5% goes: {whole_crit} to {broken_crit}"
    );
    assert_eq!((whole_hit, broken_hit), (3.0, 3.0), "its hit stays");
    // Mended at the armorer.
    let armorer = crate::game::guid::unit(ARMORER, 1);
    a.sees(armorer).await;
    map.query(move |m| m.players.get_mut(&g).unwrap().set_money(1_000))
        .await
        .unwrap();
    a.send(CMSG_REPAIR_ITEM {
        npc: armorer,
        item: sword,
    })
    .await;
    for _ in 0..200 {
        if durability(&map, g, sword).await == 50 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(durability(&map, g, sword).await, 50, "mended");
    assert_eq!(stats().await, (whole_crit, whole_hit), "mended, back");
}

/// Thistle Tea's durability_test "one lethal transition wears equipment
/// once and resurrection permits the next penalty": a death wears the gear
/// a tenth once, however much more harm the dead take; risen, the next
/// death wears it again. The world's lava wears nothing, as mangos's
/// `EnvironmentalDamage` has it (a fall's alone does: question #1).
#[tokio::test]
async fn a_death_wears_gear_once_and_the_next_again() {
    use wow_world_messages::vanilla::EnvironmentalDamageType;
    let realm = harness::realm_with(worn_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    bare(&map, g).await;
    let helm = give(&map, g, HELM, PACK).await;
    wear_at(&map, g, helm, 0).await;
    a.clear();
    let wolf = crate::game::guid::unit(299, 1);
    map.query(move |m| {
        crate::game::map::combat::kill(m, wolf, g);
        // Dead, more harm wears nothing.
        crate::game::map::combat::kill(m, wolf, g);
        crate::game::map::environment::damage(m, g, EnvironmentalDamageType::Lava, 10_000);
    })
    .await
    .unwrap();
    a.wait_for("SMSG_DURABILITY_DAMAGE_DEATH", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_DURABILITY_DAMAGE_DEATH).then_some(())
    })
    .await;
    assert_eq!(durability(&map, g, helm).await, 90, "a tenth, once");
    a.clear();
    map.query(move |m| {
        crate::game::map::death::resurrect(m, g, 1.0);
        crate::game::map::environment::damage(m, g, EnvironmentalDamageType::Lava, 100_000);
    })
    .await
    .unwrap();
    assert_eq!(
        durability(&map, g, helm).await,
        90,
        "the lava's death, nothing"
    );
    map.query(move |m| {
        crate::game::map::death::resurrect(m, g, 1.0);
        crate::game::map::combat::kill(m, wolf, g);
    })
    .await
    .unwrap();
    a.wait_for("SMSG_DURABILITY_DAMAGE_DEATH", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_DURABILITY_DAMAGE_DEATH).then_some(())
    })
    .await;
    assert_eq!(durability(&map, g, helm).await, 80, "the next death, again");
}

/// Thistle Tea's durability_test "debug wear parses whitespace and rejects
/// invalid percentages": `.debug durability  10  ` wears a tenth; 101 and
/// words are refused, wearing nothing.
#[tokio::test]
async fn debug_durability_takes_a_percent_and_refuses_the_rest() {
    let realm = harness::realm_with(worn_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    bare(&map, g).await;
    let helm = give(&map, g, HELM, PACK).await;
    wear_at(&map, g, helm, 0).await;
    a.say(".debug durability  10  ").await;
    a.system_message("Wore gear down").await;
    assert_eq!(durability(&map, g, helm).await, 90);
    for bad in ["101", "-5", "nonsense"] {
        a.say(&format!(".debug durability {bad}")).await;
        a.system_message("Usage").await;
        assert_eq!(durability(&map, g, helm).await, 90, "{bad}: nothing worn");
    }
}

/// Thistle Tea's dev_commands_test ".debug cooldowns clears item, spell,
/// category, global, and school timers" and ".debug skill sets only known
/// skills within their cap and refreshes defense fields": the cooldowns,
/// the categories', the global ones and an interrupt's school lockout all
/// go, the client told of each spell's, an item's that wasn't learned
/// too; a skill is set only from 1 to its cap and only one known (as
/// mangos's `.setskill`), its stats following.
#[tokio::test]
async fn debug_cooldowns_and_skill_do_what_they_say_and_no_more() {
    let realm = harness::realm_with(worn_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(g).unwrap().map();
    map.query(move |m| {
        let now = m.now;
        let c = &mut m.players.get_mut(&g).unwrap().caster;
        let later = now + Duration::from_secs(60);
        c.cooldowns.insert(TRINKET_SPELL, later);
        c.category_cooldowns.insert(1, later);
        c.gcd_until = Some(later);
        c.other_gcds.push((7, later));
        c.school_lockout.push((0x7F, later));
    })
    .await
    .unwrap();
    a.clear();
    a.say(".debug cooldowns").await;
    a.system_message("Cleared 1 cooldowns").await;
    a.wait_for("SMSG_CLEAR_COOLDOWN", |m| match m {
        ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(c) if c.id == TRINKET_SPELL => Some(()),
        _ => None,
    })
    .await;
    let left = map
        .query(move |m| {
            let c = &m.players[&g].caster;
            (
                c.cooldowns.len(),
                c.category_cooldowns.len(),
                c.gcd_until.is_some(),
                c.other_gcds.len(),
                c.school_lockout.len(),
            )
        })
        .await
        .unwrap();
    assert_eq!(left, (0, 0, false, 0, 0));
    // Defense, at a level's cap of 5 a level.
    const DEFENSE: u32 = 95;
    let (max, dodge) = map
        .query(move |m| {
            let d = m.realm.data.clone();
            let p = m.players.get_mut(&g).unwrap();
            if crate::game::map::skills::value(p, DEFENSE).is_none() {
                crate::game::map::skills::learn_skill(p, DEFENSE, 1);
            }
            crate::game::map::skills::set_value(p, DEFENSE, 1);
            p.apply_stats(&d);
            let p = &m.players[&g];
            (
                crate::game::map::skills::max_value(p, DEFENSE).unwrap(),
                p.derived.dodge,
            )
        })
        .await
        .unwrap();
    a.say(&format!(".debug skill {DEFENSE} 0")).await;
    a.system_message("goes from 1 to").await;
    // Past its cap: set to the cap, and told so.
    a.say(&format!(".debug skill {DEFENSE} {}", max + 1)).await;
    a.system_message(&format!("Skill {DEFENSE} at {max}, its cap")).await;
    let capped = map
        .query(move |m| crate::game::map::skills::value(&m.players[&g], DEFENSE))
        .await
        .unwrap();
    assert_eq!(capped, Some(max));
    a.say(".debug skill 43 1").await;
    a.system_message("No skill 43").await;
    map.query(move |m| {
        crate::game::map::skills::set_value(m.players.get_mut(&g).unwrap(), DEFENSE, 1)
    })
    .await
    .unwrap();
    a.say(&format!(".debug skill {DEFENSE} {max}")).await;
    a.system_message(&format!("Skill {DEFENSE} at {max}")).await;
    let (value, higher) = map
        .query(move |m| {
            let p = &m.players[&g];
            (crate::game::map::skills::value(p, DEFENSE), p.derived.dodge)
        })
        .await
        .unwrap();
    assert_eq!(value, Some(max));
    assert!(higher >= dodge, "the dodge with it: {dodge} to {higher}");
}
