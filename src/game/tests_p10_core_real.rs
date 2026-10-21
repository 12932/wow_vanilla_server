//! Round 10, the core on the real data: Thistle Tea's
//! `db/mangos/player_crit_per_agility_test.exs` (vmangos's agility per
//! crit percent, by class and level).

/// A class's agility per crit percent at a level is vmangos's table's, and
/// past its last level the last one's.
#[test]
#[ignore = "extended"]
fn melee_crit_per_agility_is_vmangos_s_table_s() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let rate = |class, level| d.misc.crit_per_agility(class, level).unwrap();
    assert!((rate(1, 30) - 10.395).abs() < 0.0001, "{}", rate(1, 30));
    assert!((rate(8, 1) - 11.1111).abs() < 0.0001, "{}", rate(8, 1));
    assert_eq!(rate(4, 70), rate(4, 60));
}
