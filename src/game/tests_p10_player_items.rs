//! Round 10's player and items: a weapon's procs from a usable weapon, a
//! bobber gone with its line, a disenchant's leftovers stored (Thistle
//! Tea's `weapon_procs_test`, `fishing_maps_test` and `disenchant_test`).

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::fields::UNIT_FIELD_BYTES_1;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::loot::{self, Loot, LootKind, LootSlot};
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapKey, auras, death, enchant, inventory, procs, spells};
use crate::game::tests_loot::{FISHING, LOOT_ITEM, POISON, equip, learn};
use crate::game::tests_p3_entity::{aura_effect, on, spell};

const DISARM: u32 = 97_980;
const DUST: u32 = 63_980;
const SHARD: u32 = 63_981;
const SCROLL: u32 = 63_982;
const WEAK: u32 = 97_981;
const STRONG: u32 = 97_982;

fn world() -> GameData {
    let mut d = crate::game::tests_loot::data();
    d.spells.spells.insert(
        DISARM,
        spell(DISARM, &[aura_effect(auras::AURA_MOD_DISARM, 0, 0, 1)]),
    );
    for entry in [DUST, SHARD] {
        d.items.insert(
            entry,
            crate::game::data::ItemTemplate {
                entry,
                name: format!("Item {entry}"),
                stackable: 20,
                allowable_class: -1,
                allowable_race: -1,
                ..Default::default()
            },
        );
    }
    // A scroll's buff at a friend, and its stronger rank.
    for (id, rank) in [(WEAK, 1), (STRONG, 4)] {
        let mut effect = aura_effect(auras::AURA_MOD_STAT, rank * 2, 0, 21);
        effect.base_dice = 1;
        d.spells.spells.insert(id, spell(id, &[effect]));
        d.spells.ranks.insert(
            id,
            crate::game::data::spells::SpellRank {
                prev: if id == STRONG { WEAK } else { 0 },
                first: WEAK,
                rank: rank as u32,
                req: 0,
            },
        );
    }
    let mut use_spells = [crate::game::data::items::ItemSpell::default(); 5];
    use_spells[0] = crate::game::data::items::ItemSpell {
        id: WEAK,
        charges: -1,
        ..Default::default()
    };
    d.items.insert(
        SCROLL,
        crate::game::data::ItemTemplate {
            entry: SCROLL,
            name: "Scroll".into(),
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            spells: use_spells,
            ..Default::default()
        },
    );
    d
}

async fn alone(class: Class) -> (Client, Guid, crate::game::map::MapHandle) {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, class).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (a, g, map)
}

/// Thistle Tea's "equipment usability and live combatants are checked
/// when feedback arrives", as mangos's `CastItemCombatSpell` takes a
/// usable weapon (`GetWeaponForAttack`, `CanUseEquippedWeapon`): a
/// poisoned blade's hit procs nothing and spends no charge from a disarmed
/// main hand, in cat form, or from a dead wielder; from one who may use it,
/// it does.
#[tokio::test]
async fn a_weapons_procs_wait_for_a_living_wielder_who_may_use_it() {
    let (_a, g, map) = alone(Class::Rogue).await;
    let kobold = guid::unit(3001, 21);
    let seen = on(&map, move |m| {
        let blade = equip(m, g, LOOT_ITEM, 15);
        learn(m, g, POISON, 0);
        spells::cast(
            m,
            g,
            POISON,
            Targets {
                item: Some(blade),
                ..Default::default()
            },
            None,
        );
        let hit = procs::Event {
            actor: g,
            target: kobold,
            actor_flags: procs::PROC_DONE_MELEE,
            ex: procs::EX_NORMAL_HIT,
            ..Default::default()
        };
        let charges = |m: &Map| {
            let p = &m.players[&g];
            let i = p.items.iter().find(|i| i.guid == blade).unwrap();
            enchant::get(i, enchant::TEMP_SLOT).charges
        };
        let mut seen = vec![charges(m)];
        // Disarmed.
        spells::cast_triggered(m, g, DISARM, Some(g));
        procs::trigger(m, hit);
        seen.push(charges(m));
        auras::remove_spell(m, g, DISARM, None);
        // In cat form.
        let set_form = |m: &mut Map, form: u8| {
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_byte(UNIT_FIELD_BYTES_1, 2, form);
        };
        set_form(m, auras::FORM_CAT);
        procs::trigger(m, hit);
        seen.push(charges(m));
        set_form(m, 0);
        // Dead.
        death::kill(m, g);
        procs::trigger(m, hit);
        seen.push(charges(m));
        death::resurrect(m, g, 1.0);
        // Able.
        procs::trigger(m, hit);
        seen.push(charges(m));
        seen
    })
    .await;
    assert_eq!(seen, vec![2, 2, 2, 2, 1]);
}

/// Thistle Tea's "resolves launch requirements before attaching one
/// interruptible bobber", as mangos's channel cancel removes the spell's
/// objects (`RemoveGameObject`): a fishing line cut short (the channel
/// interrupted) takes its bobber with it, and its client forgets it.
#[tokio::test]
async fn a_fishing_line_cut_short_takes_its_bobber() {
    let (mut a, g, map) = alone(Class::Warrior).await;
    let bobber = on(&map, move |m| {
        learn(m, g, FISHING, 356);
        spells::cast(m, g, FISHING, Targets::default(), None);
        m.loot_state.bobbers.first().map(|b| b.guid)
    })
    .await
    .expect("a bobber in the water");
    a.sees(bobber).await;
    let gone = on(&map, move |m| {
        spells::interrupt(m, g);
        (
            m.loot_state.bobbers.is_empty(),
            m.gameobject(bobber).is_none(),
        )
    })
    .await;
    assert_eq!(gone, (true, true));
    a.forgets(bobber).await;
}

/// Thistle Tea's "automatically stores remaining materials and never
/// grants them again", as mangos's `Loot::Release` stores what's left of
/// a disenchant (`AutoStore`): the shard taken, the window released, the
/// dust left goes into the bags, once.
#[tokio::test]
async fn a_disenchants_leftovers_go_into_the_bags_at_release() {
    let (a, g, map) = alone(Class::Warrior).await;
    let item = guid::item(990_001);
    on(&map, move |m| {
        let slot = |entry, count| LootSlot {
            item: entry,
            count,
            ..Default::default()
        };
        loot::show_item_loot(
            m,
            g,
            item,
            Loot {
                items: vec![slot(SHARD, 1), slot(DUST, 3)],
                kind: LootKind::Disenchant,
                owner: Some(g),
                ..Default::default()
            },
        );
        loot::take_item(m, g, 0);
    })
    .await;
    a.send(CMSG_LOOT_RELEASE { guid: item }).await;
    harness::an_update(&map).await;
    a.send(CMSG_LOOT_RELEASE { guid: item }).await;
    harness::an_update(&map).await;
    let had = on(&map, move |m| {
        let p = &m.players[&g];
        (
            p.count_of(SHARD),
            p.count_of(DUST),
            m.loot_state.item_loot.contains_key(&item),
        )
    })
    .await;
    assert_eq!(had, (1, 3, false), "the dust stored, once");
}

/// Thistle Tea's "another recipient's current aura prevents scroll
/// consumption", as mangos's `CheckCast` bounces a lower rank of a chain
/// on its target: a scroll read at a friend under its stronger rank is
/// refused (AURA_BOUNCED) before it costs, the scroll kept.
#[tokio::test]
async fn a_scroll_at_a_friend_under_its_stronger_rank_is_refused() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    a.sees(gb).await;
    on(&map, move |m| {
        inventory::add_item(m, ga, SCROLL, 3, false).unwrap();
        spells::cast_triggered(m, gb, STRONG, Some(gb));
    })
    .await;
    let at = on(&map, move |m| {
        let p = &m.players[&ga];
        let i = p.items.iter().find(|i| i.entry == SCROLL).unwrap();
        (i.bag, i.slot)
    })
    .await;
    a.clear();
    a.send(CMSG_USE_ITEM {
        bag_index: at.0,
        bag_slot: at.1,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty()
                .set_unit(SpellCastTargets_SpellCastTargetFlags_Unit::Unit { unit_target: gb }),
        },
    })
    .await;
    let refused = a
        .wait_frame("SMSG_CAST_RESULT", spells::SMSG_CAST_RESULT, |f| {
            u32::from_le_bytes(f[0..4].try_into().unwrap()) == WEAK
        })
        .await;
    assert_eq!(refused[4..6], [2, spells::checks::fail::AURA_BOUNCED]);
    let after = on(&map, move |m| {
        (
            m.players[&ga].count_of(SCROLL),
            auras::auras(m, gb).is_some_and(|a| a.has_spell(WEAK)),
        )
    })
    .await;
    assert_eq!(after, (3, false), "kept, and nothing on the friend");
}
