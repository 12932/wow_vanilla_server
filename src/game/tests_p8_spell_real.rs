//! Round 8, on the real data: Thistle Tea's `trap_proc_dbc_test.exs` (what
//! Entrapment's ranks proc on) and `transporter_dbc_test.exs` (the
//! transporters' mishaps).

use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::{self as kit, pass};
use crate::game::fields::*;
use crate::game::map::auras::*;

/// Entrapment's ranks and their chances; the traps' effects (Immolation,
/// Explosive and Frost Trap's) and Freezing Trap's.
const ENTRAPMENT: [(u32, u32); 5] = [
    (19_184, 5),
    (19_387, 10),
    (19_388, 15),
    (19_389, 20),
    (19_390, 25),
];
const TRAPS: [u32; 3] = [13_797, 13_812, 13_810];
const FREEZING_TRAP: u32 = 3_355;

/// Each of Entrapment's ranks has its own chance and procs on the fire and
/// frost traps' effects alone, its first rank's rule carried to the others
/// (Thistle Tea's "all ranks retain their real chances and eligible trap
/// families"; mangos-classic's `FillHigherRanks`).
#[tokio::test]
#[ignore = "extended"]
async fn every_rank_of_entrapment_procs_on_the_traps_by_its_chance() {
    let realm = kit::realm();
    let data = &realm.data.spells;
    for (rank, chance) in ENTRAPMENT {
        let talent = data.get(rank).expect("the rank");
        assert_eq!(talent.proc_chance, chance, "{rank}'s chance");
        let pe = data.proc_events.get(&rank);
        assert!(pe.is_some(), "{rank}'s rule");
        for trap in TRAPS {
            assert!(
                crate::game::map::procs::spell_allowed(pe, data.get(trap), 0),
                "{rank} on {trap}"
            );
        }
        assert!(
            !crate::game::map::procs::spell_allowed(pe, data.get(FREEZING_TRAP), 0),
            "{rank} on Freezing Trap"
        );
    }
}

/// The transporters' mishaps: a malfunction makes its traveller a
/// ten-second stunned look of its creature's and gives the look back; the
/// Everlook fire burns a tenth of the traveller's health every two seconds
/// till they are dead and no more; an Evil Twin lasts two hours and goes
/// as any aura does (Thistle Tea's "malfunction transforms and stuns for
/// ten seconds, then restores appearance", "Everlook fire damages the
/// traveler every two seconds and stops on death" and "Evil Twin lasts two
/// hours and can be removed through ordinary aura expiry").
#[tokio::test]
#[ignore = "extended"]
async fn the_transporters_mishaps_do_what_they_say() {
    const MALFUNCTION: u32 = 23_444;
    const FIRE: u32 = 23_449;
    const EVIL_TWIN: u32 = 23_445;
    let realm = kit::realm();
    let (_c, a, map) = kit::player(&realm, "P8TR1", "Traveller", Race::Gnome, Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let native = m.unit_fields(a).unwrap().get_u32(UNIT_FIELD_DISPLAYID);
            let malfunctioned = m
                .realm
                .data
                .creature_template(14_681)
                .map(|t| t.display_ids[0]);
            kit::apply(m, a, a, MALFUNCTION);
            let look = m.unit_fields(a).unwrap().get_u32(UNIT_FIELD_DISPLAYID);
            let stunned = auras(m, a).is_some_and(|x| x.has(AURA_MOD_STUN));
            pass(m, 9_900);
            let still = kit::has(m, a, MALFUNCTION);
            pass(m, 200);
            let after = (
                kit::has(m, a, MALFUNCTION),
                auras(m, a).is_some_and(|x| x.has(AURA_MOD_STUN)),
                m.unit_fields(a).unwrap().get_u32(UNIT_FIELD_DISPLAYID),
            );
            let malfunction = (Some(look) == malfunctioned, stunned, still, after, native);
            // The fire.
            let max = m.unit_fields(a).unwrap().get_u32(UNIT_FIELD_MAXHEALTH);
            kit::set_health(m, a, max);
            kit::apply(m, a, a, FIRE);
            let mut burns = Vec::new();
            for _ in 0..2 {
                pass(m, 2_000);
                burns.push(max - kit::health(m, a));
            }
            kit::set_health(m, a, max / 20);
            pass(m, 2_000);
            let dead = !m.unit_view(a).is_some_and(|u| u.alive);
            let burning_dead = kit::has(m, a, FIRE);
            // The twin.
            crate::game::map::death::resurrect_with(m, a, max, 0);
            kit::apply(m, a, a, EVIL_TWIN);
            let lasts = kit::holder(m, a, EVIL_TWIN)
                .and_then(|h| h.expires)
                .map(|t| t.saturating_duration_since(m.now).as_secs());
            pass(m, 7_200_100);
            (
                malfunction,
                max,
                burns,
                dead,
                burning_dead,
                lasts,
                kit::has(m, a, EVIL_TWIN),
            )
        })
        .await
        .unwrap();
    let ((looked, stunned, still, after, native), max, burns, dead, burning_dead, lasts, twin) =
        out;
    assert!(looked && stunned && still, "the malfunction");
    assert_eq!(after, (false, false, native), "after ten seconds");
    assert_eq!(
        burns,
        vec![max / 10, 2 * (max / 10)],
        "a tenth each two seconds"
    );
    assert!(dead && !burning_dead, "the fire ends at death");
    assert_eq!(lasts, Some(7_200), "two hours");
    assert!(!twin, "the twin gone");
}
