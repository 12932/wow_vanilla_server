//! Phase 7's loader round, the quests (Thistle Tea's `world/loader/` tests
//! of quests' rewards, sources and flags): what wvs reads of the real
//! quests, and what it does with them.

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::map::{Map, MapHandle};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

const ARCANE_CLOAKING: u32 = 28_006;
const NAXXRAMAS_FLAG: u32 = 9378;
const ENTRY_CONDITION: u32 = 9124;

/// Thistle Tea's attunement (`quest_rewards_vmangos_test.exs`,
/// `quest_rewards_dbc_test.exs`) and mangos's `AreaExploredOrEventHappens`:
/// The Dread Citadel (9123) casts Arcane Cloaking on its rewarded (no
/// spell shown, the cast its own); Arcane Cloaking puts on the Entry Flag
/// Trigger (29296), which pulses the Entry Flag Effect (29294), the quest
/// credit of 9378. 9378 is a flag quest (`QUEST_FLAGS_AUTO_REWARDED`), at
/// 60, done by an event: never in the log, it is rewarded the moment its
/// credit comes, and with it the Naxxramas entrance's condition (9124) is
/// met; one of 59 isn't rewarded.
#[tokio::test]
#[ignore = "extended"]
async fn arcane_cloaking_attunes_to_naxxramas() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let dread = &d.npc.quests[&9123];
    assert_eq!(
        (dread.reward_spell, dread.reward_spell_cast),
        (0, ARCANE_CLOAKING)
    );
    let flag = &d.npc.quests[&NAXXRAMAS_FLAG];
    assert_ne!(
        flag.flags & crate::game::map::quests::QUEST_FLAGS_AUTO_REWARDED,
        0
    );
    assert_eq!(flag.min_level, 60);
    assert!(flag.needs_event());
    let trigger = &d.spells.get(29_296).unwrap().effects[0];
    assert_eq!((trigger.aura, trigger.trigger_spell), (23, 29_294));
    let credit = &d.spells.get(29_294).unwrap().effects[0];
    assert_eq!(
        (credit.effect, credit.misc_value),
        (16, NAXXRAMAS_FLAG as i32)
    );

    let (_c, g, map) = player(&realm, "P7QNAXX", "Attuna", Race::Human, Class::Mage).await;
    let attuned = |level: u8| {
        on(&map, move |m| {
            ready(m, g, level, &[]);
            m.players
                .get_mut(&g)
                .unwrap()
                .journal
                .rewarded
                .remove(&NAXXRAMAS_FLAG);
            crate::game::map::spells::cast_triggered(m, g, ARCANE_CLOAKING, Some(g));
            let cloaked = has(m, g, 29_296);
            pass(m, 2100);
            let p = &m.players[&g];
            (
                cloaked,
                p.journal.rewarded.contains(&NAXXRAMAS_FLAG),
                p.journal.has(NAXXRAMAS_FLAG),
                crate::game::map::conditions::met(m, ENTRY_CONDITION, None, Some(g)),
            )
        })
    };
    assert_eq!(attuned(59).await, (true, false, false, false), "too low");
    assert_eq!(attuned(60).await, (true, true, false, true), "attuned");
}

/// Thistle Tea's Moonglade (`quest_rewards_vmangos_test.exs`) and
/// mangos's `HandleQuestgiverAcceptQuestOpcode`: a quest's `SrcSpell` is
/// cast on the one who accepts it. A night elf druid taking Moonglade
/// (5921) from Dendrite Starblaze (4217) learns Teleport: Moonglade
/// (19027, teaching 18960).
#[tokio::test]
#[ignore = "extended"]
async fn accepting_moonglade_teaches_its_teleport() {
    if !have_data() {
        return;
    }
    let realm = realm();
    assert_eq!(realm.data.npc.quests[&5921].source_spell, 19_027);
    let (_c, g, map) = player(&realm, "P7QMOON", "Mooner", Race::NightElf, Class::Druid).await;
    let out = on(&map, move |m| {
        ready(m, g, 10, &[]);
        let giver = spawn(m, g, 4217, 30, 2.0);
        let before = m.players[&g].spells.contains(&18_960);
        crate::game::map::quests::accept(m, g, giver, 5921);
        let p = &m.players[&g];
        (before, p.journal.has(5921), p.spells.contains(&18_960))
    })
    .await;
    assert_eq!(out, (false, true, true));
}
