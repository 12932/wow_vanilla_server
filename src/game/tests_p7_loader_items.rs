//! Phase 7's loader round, the items, quests and objects (Thistle Tea's
//! `world/loader/` tests of item templates, loot, properties, sets, vendors
//! and the like): what wvs reads of the real world.

use crate::game::data::GameData;
use crate::game::data::npc::QUEST_SPECIAL_REPEATABLE;

fn real() -> Option<&'static GameData> {
    crate::game::data::load::real_data()
}

/// Thistle Tea's Alterac Valley quests (`alterac_cavalry_vmangos_test.exs`,
/// `alterac_ground_vmangos_test.exs`), on the real quests: the stable
/// returns (7001, 7027) hand over their tamer's item, repeat, and are done
/// by an event; the hide turn-ins (7002, 7026) take one enemy hide; the
/// supply quests (5892, 6985, 5893, 6982) repeat and take ten of their
/// scraps; the automatic orders (6846, 6901) need no accepting, take one
/// order each, and the orders are one to a player, for Alterac Valley's
/// area alone, and last 15 minutes.
#[test]
#[ignore = "extended"]
fn real_alterac_quests_take_what_they_should() {
    let Some(d) = real() else {
        return;
    };
    let quest = |id: u32| d.npc.quests.get(&id).unwrap_or_else(|| panic!("{id}"));
    for (tame, item, hide, hide_item) in
        [(7001, 17_626, 7002, 17_642), (7027, 17_689, 7026, 17_643)]
    {
        let q = quest(tame);
        assert_eq!(q.source_item, item, "{tame}");
        assert_ne!(q.special_flags & QUEST_SPECIAL_REPEATABLE, 0, "{tame}");
        assert!(q.needs_event(), "{tame}");
        assert_eq!(quest(hide).required_items[0], (hide_item, 1), "{hide}");
    }
    for (id, item) in [
        (5892, 17_522),
        (6985, 17_522),
        (5893, 17_542),
        (6982, 17_542),
    ] {
        let q = quest(id);
        assert_eq!(q.required_items[0], (item, 10), "{id}");
        assert_ne!(q.special_flags & QUEST_SPECIAL_REPEATABLE, 0, "{id}");
        assert!(!q.auto_complete, "{id}");
    }
    for (id, item) in [(6846, 17_353), (6901, 17_442)] {
        let q = quest(id);
        assert_eq!((q.method, q.auto_complete), (0, true), "{id}");
        assert_eq!(q.required_items[0], (item, 1), "{id}");
        let t = d.item(item).unwrap();
        assert_eq!((t.max_count, t.area_bound, t.duration), (1, 2597, 900));
    }
}

/// Thistle Tea's item templates (`item_duration_vmangos_test.exs`,
/// `weapon_procs_vmangos_test.exs`, `mana_drain_vmangos_test.exs`,
/// `reactive_armor_vmangos_test.exs`): an ordinary lifetime (10439, five
/// minutes), a real-time one (19807, four hours, its flag 0x10000) and a
/// conjured trinket's (22736, five minutes, conjured, a two-hander) as
/// the seed has them. Weapons' own chance-on-hit spells (647, 1982,
/// 19019) at their speeds, 19019 with its 8 procs a minute (25.3% at 1.9
/// s, `enchant::item_spell_chance`), 13937's spell a use, no proc; Black
/// Grasp equips its mana proc (27522), whose spells (27526, 29471) take
/// no spell power; the Obsidian breastplate equips Obsidian Armor (27539),
/// whose procs rest ten seconds (`spell_proc_event.Cooldown`).
#[test]
#[ignore = "extended"]
fn real_item_lifetimes_and_spells_load_as_the_seed_has_them() {
    let Some(d) = real() else {
        return;
    };
    let item = |id: u32| d.item(id).unwrap_or_else(|| panic!("{id}"));
    assert_eq!((item(10_439).duration, item(10_439).flags), (300, 0));
    assert_eq!(
        (item(19_807).duration, item(19_807).flags),
        (14_400, 0x1_0000)
    );
    let t = item(22_736);
    assert_eq!((t.duration, t.flags, t.inventory_type), (300, 229_378, 21));
    assert_ne!(t.flags & 0x2, 0, "conjured");
    let procs = |id: u32| -> Vec<(u32, f32)> {
        item(id)
            .spells
            .iter()
            .filter(|s| s.id != 0 && s.trigger == 2)
            .map(|s| (s.id, s.ppm_rate))
            .collect()
    };
    for (entry, delay, spell, ppm) in [
        (647, 2600, 17_152, 0.0),
        (1982, 2800, 18_211, 0.0),
        (19_019, 1900, 21_992, 8.0),
    ] {
        assert_eq!(item(entry).delay, delay, "{entry}");
        assert_eq!(procs(entry), vec![(spell, ppm)], "{entry}");
    }
    assert!(procs(13_937).is_empty());
    let chance = crate::game::map::enchant::item_spell_chance(8.0, 0, 1900);
    assert!((chance - 25.33).abs() < 0.01, "{chance}");

    let grasp = item(22_194);
    assert_eq!(grasp.inventory_type, 10);
    assert_eq!((grasp.spells[2].id, grasp.spells[2].trigger), (27_522, 1));
    for spell in [27_526, 29_471] {
        let s = d.spells.get(spell).unwrap();
        assert_eq!(s.effects[0].bonus_coefficient, 0.0, "{spell}");
    }
    let plate = item(22_196);
    assert_eq!((plate.spells[0].id, plate.spells[0].trigger), (27_539, 1));
    assert_eq!(d.spells.proc_events[&27_539].cooldown, 10_000);
}

/// Thistle Tea's loot and properties (`item_loot_vmangos_test.exs`,
/// `item_property_dbc_test.exs`, `item_property_vmangos_test.exs`): the
/// Heavy Junkbox (16885) has its loot, every item of it a template, and
/// 150 to 600 copper; twenty openings give items. "of the Bear" (1182) is
/// stamina and strength (72, 69), "of Stamina" (19) stamina alone, and
/// there is no property 0; Skullcrusher Mace (1608) rolls from pool 5269,
/// of more than ten choices, 184 at 0.1001%, every chance a percent.
#[test]
#[ignore = "extended"]
fn real_lockbox_loot_and_random_properties_load() {
    use rand::SeedableRng;
    let Some(d) = real() else {
        return;
    };
    let table = &d.loot.item[&16_885];
    assert!(!table.is_empty());
    for e in table.iter().filter(|e| e.min > 0) {
        assert!(d.item(e.item).is_some(), "{}", e.item);
    }
    let junkbox = d.item(16_885).unwrap();
    assert_eq!((junkbox.min_money_loot, junkbox.max_money_loot), (150, 600));
    let mut rng = rand::rngs::StdRng::seed_from_u64(3);
    let mut opened = 0;
    for _ in 0..20 {
        let mut out = Vec::new();
        crate::game::map::loot::roll(d, table, &[], &mut rng, &mut out, 0);
        opened += usize::from(!out.is_empty());
    }
    assert!(opened > 0);

    let p = &d.crafts;
    assert_eq!(p.random_property_names[&1182], "of the Bear");
    assert_eq!(p.random_properties[&1182][..3], [72, 69, 0]);
    assert_eq!(p.random_property_names[&19], "of Stamina");
    assert_eq!(p.random_properties[&19][..3], [72, 0, 0]);
    assert!(!p.random_properties.contains_key(&0));
    assert_eq!(d.item(1608).unwrap().random_property, 5269);
    let pool = &p.random_pools[&5269];
    assert!(pool.len() > 10);
    assert!(
        pool.iter()
            .any(|(id, c)| *id == 184 && (c - 0.1001).abs() < 1e-6)
    );
    assert!(
        pool.iter()
            .all(|(id, c)| *id > 0 && *c > 0.0 && *c <= 100.0)
    );
    assert!(!p.random_pools.contains_key(&0));
}

/// Thistle Tea's sets, targets and locks (`item_set_dbc_test.exs`,
/// `item_target_vmangos_test.exs`, `lock_test.exs`): The Gladiator (1)
/// gives its four bonuses at two to five pieces; Bloodvine Garb (421) its
/// one at three, for tailors of 300; no set 0. The real
/// `item_required_target` rows: 9328 on a living 7977, 8149 on a dead
/// 7318, 15908 on a living 1196, 9618 on any of four dead, 1599 on
/// nothing, 37 in all. Lock 36 asks for its key (3467) first, then its
/// lock type 1 at 100, its empty slots dropped.
#[test]
#[ignore = "extended"]
fn real_sets_item_targets_and_locks_keep_their_order() {
    let Some(d) = real() else {
        return;
    };
    let set = |id: u32| d.crafts.item_sets.get(&id);
    let mut gladiator = set(1).unwrap().spells.clone();
    gladiator.sort_by_key(|(_, n)| *n);
    assert_eq!(gladiator, vec![(9761, 2), (7514, 3), (9140, 4), (7597, 5)]);
    let garb = set(421).unwrap();
    assert_eq!(
        (
            garb.required_skill,
            garb.required_skill_rank,
            garb.spells.clone()
        ),
        (197, 300, vec![(18_382, 3)])
    );
    assert!(set(0).is_none());

    let targets = |item: u32| {
        d.crafts
            .item_targets
            .get(&item)
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(targets(9328), vec![(7977, true)]);
    assert_eq!(targets(8149), vec![(7318, false)]);
    assert_eq!(targets(15_908), vec![(1196, true)]);
    let mut stones = targets(9618);
    stones.sort_unstable();
    assert_eq!(
        stones,
        vec![(2927, false), (2928, false), (2929, false), (7808, false)]
    );
    assert!(targets(1599).is_empty());
    assert_eq!(
        d.crafts.item_targets.values().map(Vec::len).sum::<usize>(),
        37
    );

    let lock = &d.crafts.locks[&36];
    assert_eq!((lock[0].kind, lock[0].index), (1, 3467), "the key first");
    assert_eq!((lock[1].kind, lock[1].index, lock[1].skill), (2, 1, 100));
    assert!(lock.iter().all(|k| k.kind != 0));
    assert!(!d.crafts.locks.contains_key(&999_999));
}

/// Thistle Tea's vendors and meeting stones (`vendor_vmangos_test.exs`,
/// `meeting_stone_vmangos_test.exs`): Innkeeper Allison's (1464) list is
/// her own and her template's (100, its Love is in the Air goods), each
/// item once; the Deadmines stone (178834) is a meeting stone for levels
/// 17 to 26 at area 1581, and the innkeepers' option (gossip script 2001)
/// queues for the same area.
#[test]
#[ignore = "extended"]
fn real_vendor_lists_and_meeting_stones_combine() {
    let Some(d) = real() else {
        return;
    };
    let items: Vec<u32> = d.vendor_items(1464).iter().map(|v| v.item).collect();
    assert!(items.contains(&117));
    for i in [21_815, 21_829, 21_833] {
        assert!(items.contains(&i), "{i}");
    }
    let mut unique = items.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), items.len());

    let stone = &d.gameobject_templates[&178_834];
    assert_eq!((stone.kind, &stone.data[..3]), (23, &[17, 26, 1581][..]));
    let m = d.misc.meeting_stones[&178_834];
    assert_eq!((m.min_level, m.max_level, m.area), (17, 26, 1581));
    let steps = &d.scripts.gossip[&2001];
    assert_eq!(steps.len(), 1);
    assert_eq!((steps[0].command, steps[0].datalong[0]), (36, 1581));
}
