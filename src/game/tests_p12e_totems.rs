//! Round 12e: effect 92, `ENCHANT_HELD_ITEM`, as mangos-classic's
//! `Spell::EffectEnchantHeldItem`: the weapon totems' enchantment on the
//! main hand of each of the party in reach, for ten seconds when the spell
//! names no time, never over another temporary one; Improved Weapon
//! Totems' percent on Windfury Totem's attack power and on Flametongue
//! Totem's strike, which goes by its weapon's speed with no spell power.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_combat::equip;
use super::tests_p3_spells_kit::{self as kit, cast_fresh, health};
use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{
    ATTR_EX2_CANT_CRIT, ATTR_EX3_ALWAYS_HIT, Enchantment, SpellEffect,
};
use crate::game::harness::Client;
use crate::game::map::Map;
use crate::game::map::auras::{AURA_DUMMY, AURA_MOD_ATTACK_POWER};
use crate::game::map::enchant::{self, TEMP_SLOT};
use crate::game::map::procs::{self, EX_NORMAL_HIT, Event};
use crate::game::map::spells::Targets;

const SWORD: u32 = 991_200;
/// The totems' effects (Windfury's icon, Flametongue's), their
/// enchantments, the attack Windfury's gives and Flametongue's proc; a
/// poison's enchantment, temporary too.
const WINDFURY_HELD: u32 = 997_601;
const FLAMETONGUE_HELD: u32 = 997_602;
const WINDFURY_ENCHANT: u32 = 9_601;
const FLAMETONGUE_ENCHANT: u32 = 9_602;
const POISON_ENCHANT: u32 = 9_603;
const WINDFURY_ATTACK: u32 = 997_603;
const FLAMETONGUE_PROC: u32 = 997_604;
/// The real ids the code names: Improved Weapon Totems' first rank, and
/// the totem's strike.
const IMPROVED_WEAPON_TOTEMS: u32 = 29192;
const FLAMETONGUE_STRIKE: u32 = 16368;
const R30: u32 = 990_601;

fn totems(d: &mut GameData) {
    d.spells.radii.insert(R30, 30.0);
    d.items.insert(
        SWORD,
        ItemTemplate {
            entry: SWORD,
            class: 2,
            subclass: 7,
            name: "Sword".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 2000,
            stackable: 1,
            ..Default::default()
        },
    );
    let held = |id: u32, icon: u32, enchant: u32| {
        let mut s = bare(id, 0);
        s.family_name = 11;
        s.family_flags = 0x0400_0000;
        s.spell_icon = icon;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: 92,
                base_points: -1,
                base_dice: 1,
                misc_value: enchant as i32,
                target_a: 20,
                radius_index: R30,
                ..Default::default()
            },
        );
        s
    };
    let mut attack = bare(WINDFURY_ATTACK, 0);
    attack.family_name = 11;
    attack.spell_icon = 1397;
    attack.duration_index = LASTS_5S;
    effect(&mut attack, 0, aura(AURA_MOD_ATTACK_POWER, 0, 100, 1));
    let mut proc = bare(FLAMETONGUE_PROC, 2);
    proc.family_name = 11;
    proc.family_flags = 0x4_0000_0000;
    effect(
        &mut proc,
        0,
        SpellEffect {
            effect: 3,
            base_points: 999,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut strike = bare(FLAMETONGUE_STRIKE, 2);
    strike.attributes[2] |= ATTR_EX2_CANT_CRIT;
    strike.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    effect(&mut strike, 0, damage(1));
    let mut talent = bare(IMPROVED_WEAPON_TOTEMS, 0);
    talent.duration_index = LASTS_LONG;
    effect(&mut talent, 0, aura(AURA_DUMMY, 0, 15, 1));
    effect(&mut talent, 1, aura(AURA_DUMMY, 0, 8, 1));
    insert(
        d,
        [
            held(WINDFURY_HELD, 1397, WINDFURY_ENCHANT),
            held(FLAMETONGUE_HELD, 72, FLAMETONGUE_ENCHANT),
            attack,
            proc,
            strike,
            talent,
        ],
    );
    for (id, spell) in [
        (WINDFURY_ENCHANT, WINDFURY_ATTACK),
        (FLAMETONGUE_ENCHANT, FLAMETONGUE_PROC),
        (POISON_ENCHANT, 0),
    ] {
        d.spells.enchantments.insert(
            id,
            Enchantment {
                id,
                effects: [(1, 100, 0, spell), (0, 0, 0, 0), (0, 0, 0, 0)],
                name: format!("Enchantment {id}"),
            },
        );
    }
}

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    member
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) if !l.members.is_empty() => Some(()),
            _ => None,
        })
        .await;
}

/// `g`'s main hand's temporary enchantment.
fn held(m: &Map, g: Guid) -> enchant::Enchant {
    let p = &m.players[&g];
    let i = p
        .item_at(crate::game::store::BAG_PLAYER, 15)
        .expect("a main hand");
    enchant::get(&p.items[i], TEMP_SLOT)
}

/// One blow of `g`'s that lands on `target`.
fn blow(m: &mut Map, g: Guid, target: Guid) {
    procs::trigger(
        m,
        Event {
            actor: g,
            target,
            actor_flags: procs::PROC_DONE_MELEE,
            target_flags: procs::PROC_TAKEN_MELEE,
            ex: EX_NORMAL_HIT,
            school: 1,
            damage: 10,
            ..Default::default()
        },
    );
}

/// A shaman's Windfury Totem's effect: the warrior in their party gets
/// its enchantment on his sword for ten seconds (the spell names no time
/// and no points), and is told its time; the shaman too; a stranger
/// beside them doesn't. A poison on the stranger's sword stays: the
/// totem's doesn't go over another. With Improved Weapon Totems, the
/// warrior's Windfury attack gives 115 attack power, not 100. Before,
/// effect 92 did nothing at all.
#[tokio::test]
async fn a_weapon_totems_enchantment_is_on_the_partys_main_hands() {
    let realm = fast_realm(totems);
    let (mut a, s, map) = human(&realm, "P12ET", "Totemic", Class::Paladin).await;
    let (mut b, w, _) = human(&realm, "P12EU", "Wielding", Class::Warrior).await;
    let (_c, x, _) = human(&realm, "P12EV", "Stranger", Class::Warrior).await;
    group(&mut a, &mut b, "Wielding").await;
    map.query(move |m| {
        for g in [s, w, x] {
            kit::ready(m, g, 60, &[]);
            equip(m, g, SWORD, 15);
        }
        let i = m.players[&x]
            .item_at(crate::game::store::BAG_PLAYER, 15)
            .unwrap();
        let sword = m.players[&x].items[i].guid;
        enchant::enchant_item(m, x, sword, POISON_ENCHANT, Some((60_000, 0)));
    })
    .await
    .unwrap();
    b.clear();
    let (mine, his, theirs) = map
        .query(move |m| {
            cast_fresh(m, s, WINDFURY_HELD, Targets::default());
            (held(m, s), held(m, w), held(m, x))
        })
        .await
        .unwrap();
    let ten = enchant::Enchant {
        id: WINDFURY_ENCHANT,
        duration_ms: 10_000,
        charges: 0,
    };
    assert_eq!(his, ten, "the warrior's sword has it, for ten seconds");
    assert_eq!(mine, ten, "the shaman's too");
    assert_eq!(theirs.id, POISON_ENCHANT, "the stranger's poison stays");
    let told = b
        .wait_for("SMSG_ITEM_ENCHANT_TIME_UPDATE", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_ENCHANT_TIME_UPDATE(u) => Some((u.slot, u.duration)),
            _ => None,
        })
        .await;
    assert_eq!(told, (TEMP_SLOT as u32, 10), "and he is told its time");
    let (plain, improved) = map
        .query(move |m| {
            let kobold = kit::dummy(m, w, 60, 2.0);
            blow(m, w, kobold);
            let plain = kit::holder(m, w, WINDFURY_ATTACK).map(|h| h.effects[0].amount);
            crate::game::map::auras::remove_spell(m, w, WINDFURY_ATTACK, None);
            kit::apply(m, s, s, IMPROVED_WEAPON_TOTEMS);
            cast_fresh(m, s, WINDFURY_HELD, Targets::default());
            blow(m, w, kobold);
            let improved = kit::holder(m, w, WINDFURY_ATTACK).map(|h| h.effects[0].amount);
            (plain, improved)
        })
        .await
        .unwrap();
    assert_eq!(plain, Some(100), "its attack, as it is");
    assert_eq!(improved, Some(115), "and with the talent, 15% more");
}

/// Flametongue Totem's enchantment's proc strikes for its points a
/// hundredth a second of the sword's two seconds (20), and Improved Weapon
/// Totems' second effect's 8% more (21), the wielder's spell power not
/// counted. Before, the totem's enchantment never landed.
#[tokio::test]
async fn flametongue_totems_strike_goes_by_the_weapon_and_the_talent() {
    let realm = fast_realm(totems);
    let (_a, s, map) = human(&realm, "P12EF", "Flamer", Class::Paladin).await;
    let (plain, improved) = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, s, 60, &[]);
            equip(m, s, SWORD, 15);
            let kobold = kit::dummy(m, s, 60, 2.0);
            cast_fresh(m, s, FLAMETONGUE_HELD, Targets::default());
            let before = health(m, kobold);
            blow(m, s, kobold);
            let plain = before - health(m, kobold);
            kit::apply(m, s, s, IMPROVED_WEAPON_TOTEMS);
            cast_fresh(m, s, FLAMETONGUE_HELD, Targets::default());
            let before = health(m, kobold);
            blow(m, s, kobold);
            (plain, before - health(m, kobold))
        })
        .await
        .unwrap();
    assert_eq!(plain, 20, "the weapon's speed's share of its points");
    assert_eq!(improved, 21, "and 8% more with the talent");
}

/// On the real data: an orc shaman's Windfury Totem (8512) in reach of
/// the warrior in his party puts Windfury Totem's first rank (1783) on
/// the warrior's sword as its passive (8515) ticks.
#[tokio::test]
#[ignore = "extended"]
async fn a_real_windfury_totem_enchants_the_partys_weapons() {
    let realm = kit::realm();
    let (mut a, s, map) =
        kit::player(&realm, "P12EW", "Windcaller", Race::Orc, Class::Shaman).await;
    let (mut b, w, _) = kit::player(&realm, "P12EX", "Windborne", Race::Orc, Class::Warrior).await;
    group(&mut a, &mut b, "Windborne").await;
    let got = map
        .query(move |m| {
            kit::ready(m, s, 60, &[8512]);
            kit::ready(m, w, 60, &[]);
            crate::game::map::inventory::add_item(m, s, 5178, 1, false).expect("an air totem");
            equip(m, w, 25, 15);
            let cast = cast_fresh(m, s, 8512, Targets::default());
            let totem = m
                .creatures
                .iter()
                .map(|(_, c)| c)
                .find(|c| c.owner == Some(s))
                .map(|c| c.guid);
            for _ in 0..12 {
                kit::pass(m, 1_000);
            }
            (cast, totem, held(m, w).id)
        })
        .await
        .unwrap();
    assert!(got.0, "the totem is cast");
    assert!(got.1.is_some(), "and stands");
    assert_eq!(got.2, 1783, "its enchantment is on the warrior's sword");
}
