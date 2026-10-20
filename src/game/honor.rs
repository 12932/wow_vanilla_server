//! Honor, realm-wide: each character's kills and contribution by day, its
//! rank points, and the weekly ranking that turns a week's contribution
//! into rank points by standing, as vanilla 1.12 (and Thistle Tea's
//! `Core.Honor`) ranks it. Kept in the store's records, one per character.

use std::collections::BTreeMap;

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

pub const KIND: &str = "honor";

/// The weekday the week turns over on: 1 Monday to 7 Sunday; Thistle
/// Tea's Tuesday.
pub const DEFAULT_RESET_WEEKDAY: u8 = 2;

/// What a day earned.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Day {
    pub honorable_kills: u32,
    pub dishonorable_kills: u32,
    pub contribution: f32,
    /// Kills of each victim today, for diminishing returns.
    #[serde(default)]
    pub victims: BTreeMap<u64, u32>,
}

/// One character's honor.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub guid: u32,
    /// 0 Alliance, 1 Horde.
    pub team: u8,
    pub level: u8,
    pub days: BTreeMap<i64, Day>,
    pub rank_points: f32,
    pub highest_rank: u8,
    pub lifetime_honorable_kills: u32,
    pub lifetime_dishonorable_kills: u32,
    pub last_week: Day,
    /// Standing in last week's ranking: 1 the best.
    pub last_standing: u32,
    pub last_settled_week: Option<i64>,
}

/// What an award is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwardKind {
    Honorable,
    Dishonorable,
    /// Battleground objectives and wins, quests.
    Bonus,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Award {
    pub kind: AwardKind,
    pub points: f32,
    /// The victim, for an honorable kill: its key for diminishing returns.
    pub victim: Option<u64>,
    /// The victim's rank, for `SMSG_PVP_CREDIT`.
    pub victim_rank: u32,
}

/// The fields a character's honor shows on its client.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Projection {
    pub rank: u8,
    pub highest_rank: u8,
    pub rank_bar: u8,
    pub today_honorable: u32,
    pub today_dishonorable: u32,
    pub yesterday_kills: u32,
    pub yesterday_contribution: u32,
    pub this_week_kills: u32,
    pub this_week_contribution: u32,
    pub last_week_kills: u32,
    pub last_week_contribution: u32,
    pub last_week_standing: u32,
    pub lifetime_honorable: u32,
    pub lifetime_dishonorable: u32,
}

/// The realm's honor.
#[derive(Debug, Default)]
pub struct Ledger {
    pub entries: AHashMap<u32, Entry>,
    /// Days since 1970.
    pub day: i64,
    pub week_start: i64,
    pub weekday: u8,
    /// Entries changed since they were last kept.
    pub dirty: ahash::AHashSet<u32>,
    /// A week settled by an award's [`Ledger::on`], for the sweep to tell.
    pub week_turned: bool,
    /// Kills a week needs to be ranked (`[world] min_honor_kills`);
    /// [`MINIMUM_KILLS`] if unset.
    pub minimum_kills: Option<u32>,
}

/// Today, in days since 1970.
pub fn today() -> i64 {
    chrono::Utc::now().timestamp().div_euclid(86_400)
}

/// The first day of the week `day` is in, the week turning on `weekday`.
pub fn week_start(day: i64, weekday: u8) -> i64 {
    // 1970-01-01, day 0, was a Thursday (4).
    day - (day + 4 - i64::from(weekday.clamp(1, 7))).rem_euclid(7)
}

impl Ledger {
    pub fn new(day: i64, weekday: u8) -> Self {
        Self {
            day,
            week_start: week_start(day, weekday),
            weekday: weekday.clamp(1, 7),
            ..Default::default()
        }
    }

    /// The week turns on `weekday` from now on.
    pub fn set_weekday(&mut self, weekday: u8) {
        let weekday = weekday.clamp(1, 7);
        if weekday != self.weekday {
            self.weekday = weekday;
            self.week_start = week_start(self.day, weekday);
        }
    }

    /// The ledger the store kept, brought up to today.
    pub fn load(records: &[(String, u64, String)], day: i64, weekday: u8) -> Self {
        let mut l = Self::new(day, weekday);
        let mut oldest: Option<i64> = None;
        for (_, _, data) in records.iter().filter(|r| r.0 == KIND) {
            if let Ok(e) = serde_json::from_str::<Entry>(data) {
                if let Some(w) = e.last_settled_week {
                    oldest = Some(oldest.map_or(w, |o: i64| o.min(w)));
                }
                l.entries.insert(e.guid, e);
            }
        }
        // Settle the weeks that passed while the realm was down, from the
        // week after the last one settled.
        if let Some(w) = oldest {
            let first = w + 7;
            if first < l.week_start {
                l.week_start = first;
                l.day = first;
                l.advance(day);
            }
        }
        l
    }

    /// A character honor knows of, with its team and level now.
    pub fn register(&mut self, guid: u32, team: u8, level: u8) {
        let e = self.entries.entry(guid).or_insert_with(|| Entry {
            guid,
            ..Default::default()
        });
        if e.team != team || e.level != level {
            e.team = team;
            e.level = level;
            self.dirty.insert(guid);
        }
    }

    /// The calendar moves to `day`: each week that ends is ranked.
    /// Returns whether a week was settled.
    pub fn advance(&mut self, day: i64) -> bool {
        let mut settled = false;
        while day >= self.week_start + 7 {
            self.settle_week(self.week_start + 6, false);
            self.week_start += 7;
            settled = true;
        }
        if day > self.day {
            self.day = day;
        }
        settled
    }

    /// The ledger as of `day`, for an award: the calendar moved on first,
    /// so the first award after midnight goes on the new day, not the one
    /// the sweep last saw (Thistle Tea's "settles before the first award
    /// after midnight"). A week this settles is told at the next sweep.
    pub fn on(&mut self, day: i64) -> &mut Self {
        if self.advance(day) {
            self.week_turned = true;
        }
        self
    }

    /// Rank the week so far, now, rather than at its end (a GM's
    /// `.honor week`); what's counted won't count again.
    pub fn settle_now(&mut self) {
        self.settle_week(self.day, true);
        let day = self.day;
        for e in self.entries.values_mut() {
            e.days.retain(|d, _| *d > day);
        }
    }

    /// Rank the week from `week_start` to `last` for everyone.
    fn settle_week(&mut self, last: i64, force: bool) {
        let start = self.week_start;
        let candidates: Vec<Candidate> = self
            .entries
            .values()
            .map(|e| {
                let t = totals(e, start, last);
                Candidate {
                    guid: e.guid,
                    team: e.team,
                    level: e.level,
                    honorable_kills: t.honorable_kills,
                    contribution: t.contribution,
                    rank_points: e.rank_points,
                }
            })
            .collect();
        let standings = weekly_with(&candidates, self.minimum_kills.unwrap_or(MINIMUM_KILLS));
        for (guid, s) in standings {
            let Some(e) = self.entries.get_mut(&guid) else {
                continue;
            };
            if !force && e.last_settled_week.is_some_and(|w| start <= w) {
                continue;
            }
            let mut week = totals(e, start, last);
            week.victims.clear();
            e.last_week = week;
            e.last_standing = s.position;
            e.last_settled_week = Some(start);
            e.rank_points = s.rank_points;
            e.days.retain(|d, _| *d >= start + 6);
            e.highest_rank = e.highest_rank.max(rank_number(e.rank_points));
            self.dirty.insert(guid);
        }
    }

    /// Record an award for `guid` today.
    pub fn award(&mut self, guid: u32, a: Award) {
        let day = self.day;
        let Some(e) = self.entries.get_mut(&guid) else {
            return;
        };
        if a.points <= 0.0 && a.kind != AwardKind::Dishonorable {
            return;
        }
        let d = e.days.entry(day).or_default();
        match a.kind {
            AwardKind::Honorable => {
                d.honorable_kills += 1;
                d.contribution += a.points;
                if let Some(v) = a.victim {
                    *d.victims.entry(v).or_default() += 1;
                }
                e.lifetime_honorable_kills += 1;
            }
            AwardKind::Dishonorable => {
                d.dishonorable_kills += 1;
                e.lifetime_dishonorable_kills += 1;
                e.rank_points = (e.rank_points - a.points).max(0.0);
            }
            AwardKind::Bonus => d.contribution += a.points,
        }
        e.highest_rank = e.highest_rank.max(rank_number(e.rank_points));
        self.dirty.insert(guid);
    }

    /// Today's kills of `victim` by `guid`.
    pub fn victim_kills(&self, guid: u32, victim: u64) -> u32 {
        self.entries
            .get(&guid)
            .and_then(|e| e.days.get(&self.day))
            .and_then(|d| d.victims.get(&victim).copied())
            .unwrap_or(0)
    }

    /// `killer` (with `share` of the credit) killed `victim`: the award,
    /// if it's an honorable kill.
    pub fn player_kill(&mut self, killer: u32, victim: u32, share: f32) -> Option<Award> {
        let (k, v) = (self.entries.get(&killer)?, self.entries.get(&victim)?);
        if k.team == v.team || share <= 0.0 {
            return None;
        }
        let key = u64::from(victim);
        let count = self.victim_kills(killer, key);
        let rank = visual_rank(v.rank_points);
        let points = (kill_points(k.level, v.level, rank, count) * share).trunc();
        if points <= 0.0 {
            return None;
        }
        let award = Award {
            kind: AwardKind::Honorable,
            points,
            victim: Some(key),
            victim_rank: u32::from(rank_number(v.rank_points).max(5)),
        };
        self.award(killer, award);
        Some(award)
    }

    pub fn rank(&self, guid: u32) -> u8 {
        self.entries
            .get(&guid)
            .map_or(0, |e| rank_number(e.rank_points))
    }

    /// What `guid`'s fields show.
    pub fn project(&self, guid: u32) -> Projection {
        let Some(e) = self.entries.get(&guid) else {
            return Projection::default();
        };
        let today = e.days.get(&self.day).cloned().unwrap_or_default();
        let yesterday = e.days.get(&(self.day - 1)).cloned().unwrap_or_default();
        let week = totals(e, self.week_start, self.day);
        Projection {
            rank: rank_number(e.rank_points),
            highest_rank: e.highest_rank,
            rank_bar: rank_progress(e.rank_points),
            today_honorable: today.honorable_kills,
            today_dishonorable: today.dishonorable_kills,
            yesterday_kills: yesterday.honorable_kills,
            yesterday_contribution: yesterday.contribution as u32,
            this_week_kills: week.honorable_kills,
            this_week_contribution: week.contribution as u32,
            last_week_kills: e.last_week.honorable_kills,
            last_week_contribution: e.last_week.contribution as u32,
            last_week_standing: e.last_standing,
            lifetime_honorable: e.lifetime_honorable_kills,
            lifetime_dishonorable: e.lifetime_dishonorable_kills,
        }
    }

    /// The changed entries, to keep.
    pub fn take_dirty(&mut self) -> Vec<Entry> {
        let dirty: Vec<u32> = self.dirty.drain().collect();
        dirty
            .into_iter()
            .filter_map(|g| self.entries.get(&g).cloned())
            .collect()
    }

    /// For a GM: set rank points (capped by level).
    pub fn set_rank_points(&mut self, guid: u32, points: f32) {
        if let Some(e) = self.entries.get_mut(&guid) {
            e.rank_points = points.clamp(0.0, rank_maximum(e.level));
            e.highest_rank = e.highest_rank.max(rank_number(e.rank_points));
            self.dirty.insert(guid);
        }
    }
}

/// An entry's totals over `first..=last`.
fn totals(e: &Entry, first: i64, last: i64) -> Day {
    e.days
        .range(first..=last)
        .fold(Day::default(), |mut t, (_, d)| {
            t.honorable_kills += d.honorable_kills;
            t.dishonorable_kills += d.dishonorable_kills;
            t.contribution += d.contribution;
            t
        })
}

/// The levels below which a kill counts for nothing, and the zero
/// difference, as for XP.
fn zero_difference(level: u8) -> f32 {
    match level {
        0..=7 => 5.0,
        8..=9 => 6.0,
        10..=11 => 7.0,
        12..=15 => 8.0,
        16..=19 => 9.0,
        20..=29 => 11.0,
        30..=39 => 12.0,
        40..=44 => 13.0,
        45..=49 => 14.0,
        50..=54 => 15.0,
        55..=59 => 16.0,
        _ => 17.0,
    }
}

fn level_factor(killer: u8, victim: u8) -> f32 {
    if victim >= killer {
        return 1.0 + f32::from((victim - killer).min(4)) * 0.05;
    }
    if u32::from(victim) <= crate::game::map::combat::gray_level(u32::from(killer)) {
        return 0.0;
    }
    let d = zero_difference(killer);
    (d + f32::from(victim) - f32::from(killer)) / d
}

fn level_coefficient(level: u8) -> f32 {
    match level {
        60.. => 1.0,
        50..=59 => 0.9545,
        40..=49 => 0.5707,
        30..=39 => 0.3434,
        20..=29 => 0.2070,
        _ => 0.1212,
    }
}

/// An honorable kill's contribution: by the killer's level, the victim's
/// level and rank, less a tenth for each kill of the victim today.
pub fn kill_points(killer_level: u8, victim_level: u8, victim_rank: u8, previous: u32) -> f32 {
    if killer_level == 0 || victim_level == 0 || victim_rank > 14 || previous > 9 {
        return 0.0;
    }
    level_coefficient(killer_level)
        * (1.0 - previous as f32 / 10.0)
        * 188.3
        * (0.05331 * f32::from(victim_rank)).exp()
        * level_factor(killer_level, victim_level)
}

/// A dishonorable kill's cost in rank points.
pub fn dishonorable_points(level: u8) -> f32 {
    let l = f32::from(level);
    match level {
        0..=29 => 10.0,
        30..=35 => 10.0 + 1.5 * (l - 29.0),
        36..=41 => 19.0 + 2.0 * (l - 35.0),
        42..=50 => 31.0 + 3.2 * (l - 41.0),
        _ => (60.0 + 4.0 * (l - 50.0)).min(100.0),
    }
}

/// The rank rank points give: 0 none, 5 to 18 (Private to Grand Marshal is
/// 5 to 18 less 4).
pub fn rank_number(points: f32) -> u8 {
    if points <= 0.0 {
        0
    } else if points < 2000.0 {
        5
    } else {
        ((points as i64 / 5000) + 6).min(18) as u8
    }
}

/// The rank a client shows, 0 to 14.
pub fn visual_rank(points: f32) -> u8 {
    rank_number(points).saturating_sub(4)
}

fn rank_bounds(rank: u8) -> (f32, f32) {
    match rank {
        0 | 5 => (0.0, 2000.0),
        6 => (2000.0, 5000.0),
        r => (f32::from(r - 6) * 5000.0, f32::from(r - 5) * 5000.0),
    }
}

/// How far into its rank, 0 to 255.
pub fn rank_progress(points: f32) -> u8 {
    let (lo, hi) = rank_bounds(rank_number(points));
    let p = points.clamp(lo, hi);
    ((p - lo) / (hi - lo) * 255.0) as u8
}

/// Last week's rank points decayed and this week's earning added.
pub fn decay(points: f32, earning: f32) -> f32 {
    let mut delta = earning - (points * 0.2 + 0.5).floor();
    if delta < 0.0 {
        delta = (delta / 2.0).max(-2500.0);
    }
    (points + delta).max(0.0)
}

/// The most rank points a level may hold.
pub fn rank_maximum(level: u8) -> f32 {
    let l = f32::from(level);
    match level {
        0..=29 => 6500.0,
        30..=35 => 7150.0 + 975.0 * (l - 30.0),
        36..=39 => 12025.0 + 1300.0 * (l - 35.0),
        40..=43 => 17225.0 + 1625.0 * (l - 39.0),
        44..=52 => 23725.0 + 2275.0 * (l - 43.0),
        53..=60 => 44200.0 + 2600.0 * (l - 52.0),
        _ => 65000.0,
    }
}

const BRACKETS: [f32; 14] = [
    1.0, 0.845, 0.697, 0.566, 0.436, 0.327, 0.228, 0.159, 0.100, 0.060, 0.035, 0.020, 0.008, 0.003,
];
const EARNINGS: [f32; 15] = [
    0.0, 400.0, 1000.0, 2000.0, 3000.0, 4000.0, 5000.0, 6000.0, 7000.0, 8000.0, 9000.0, 10000.0,
    11000.0, 12000.0, 13000.0,
];
/// Kills a week needs to be ranked, by default (`[world]
/// min_honor_kills`).
pub const MINIMUM_KILLS: u32 = 15;

/// One character in a week's ranking.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub guid: u32,
    pub team: u8,
    pub level: u8,
    pub honorable_kills: u32,
    pub contribution: f32,
    pub rank_points: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Standing {
    pub position: u32,
    pub earning: f32,
    pub rank_points: f32,
}

/// The week's ranking: each side ranked by contribution, with the rank
/// points its standing earns; those who didn't take part decay.
pub fn weekly(candidates: &[Candidate]) -> Vec<(u32, Standing)> {
    weekly_with(candidates, MINIMUM_KILLS)
}

/// [`weekly`], with `minimum_kills` a week to be ranked.
pub fn weekly_with(candidates: &[Candidate], minimum_kills: u32) -> Vec<(u32, Standing)> {
    let mut out = Vec::new();
    for team in [0u8, 1] {
        let mut ranked: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| {
                c.team == team && c.honorable_kills >= minimum_kills && c.contribution > 0.0
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.contribution
                .total_cmp(&a.contribution)
                .then(a.guid.cmp(&b.guid))
        });
        let scores: Vec<f32> = ranked.iter().map(|c| c.contribution).collect();
        let brackets: Vec<usize> = BRACKETS
            .iter()
            .map(|b| (b * scores.len() as f32).round() as usize)
            .collect();
        let breakpoints = breakpoints(&scores, &brackets);
        for (i, c) in ranked.iter().enumerate() {
            let earning = earning(c.contribution, &brackets, &breakpoints);
            let points = decay(c.rank_points, earning).min(rank_maximum(c.level));
            out.push((
                c.guid,
                Standing {
                    position: i as u32 + 1,
                    earning,
                    rank_points: points,
                },
            ));
        }
    }
    for c in candidates {
        if !out.iter().any(|(g, _)| *g == c.guid) {
            out.push((
                c.guid,
                Standing {
                    position: 0,
                    earning: 0.0,
                    rank_points: decay(c.rank_points, 0.0),
                },
            ));
        }
    }
    out
}

fn cutoff_honor(scores: &[f32], cutoff: usize) -> f32 {
    if cutoff == 0 {
        return 0.0;
    }
    match scores.get(cutoff - 1).copied().unwrap_or(0.0) {
        h if h > 0.0 => h + scores.get(cutoff).copied().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn breakpoints(scores: &[f32], brackets: &[usize]) -> Vec<f32> {
    let top = scores.first().copied().unwrap_or(0.0);
    let mut out = vec![0.0];
    let (mut previous, mut reached_top) = (0.0f32, false);
    for b in brackets.iter().take(14).skip(1) {
        let honor = cutoff_honor(scores, *b);
        if honor > 0.0 {
            out.push(honor / 2.0);
            previous = honor / 2.0;
        } else if !reached_top {
            out.push(if previous > 0.0 { top } else { 0.0 });
            previous = 0.0;
            reached_top = true;
        } else {
            out.push(0.0);
            previous = 0.0;
        }
    }
    out.push(if reached_top { 0.0 } else { top });
    out
}

fn earning(contribution: f32, brackets: &[usize], breakpoints: &[f32]) -> f32 {
    let index = (0..14)
        .find(|i| brackets[*i] == 0 || breakpoints[*i] > contribution)
        .unwrap_or(14);
    let lower = breakpoints[index.saturating_sub(1)];
    let upper = breakpoints[index];
    let max = EARNINGS[index];
    if upper > contribution && contribution >= lower {
        let min = EARNINGS[index.saturating_sub(1)];
        min + (max - min) * (contribution - lower) / (upper - lower)
    } else {
        max
    }
}

/// The group credit rate for `count` members sharing a kill.
pub fn group_rate(count: usize) -> f32 {
    match count {
        0..=2 => 1.0,
        3 => 1.166,
        4 => 1.3,
        5 => 1.4,
        n => (1.0 - n as f32 * 0.05).max(0.01),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_points_follow_the_vanilla_formula() {
        // Two level 60s, the victim unranked: 188.3.
        assert!((kill_points(60, 60, 0, 0) - 188.3).abs() < 0.01);
        // The second kill of the same victim today: a tenth less.
        assert!((kill_points(60, 60, 0, 1) - 169.47).abs() < 0.01);
        assert_eq!(kill_points(60, 60, 0, 10), 0.0);
        // A gray victim is worth nothing.
        assert_eq!(kill_points(60, 40, 0, 0), 0.0);
        assert!((dishonorable_points(60) - 100.0).abs() < 0.01);
    }

    #[test]
    fn ranks_and_their_bars() {
        assert_eq!(rank_number(0.0), 0);
        assert_eq!(rank_number(1.0), 5);
        assert_eq!(rank_number(2000.0), 6);
        assert_eq!(rank_number(5000.0), 7);
        assert_eq!(rank_number(65000.0), 18);
        assert_eq!(visual_rank(5000.0), 3);
        assert_eq!(rank_progress(1000.0), 127);
        // 20% decay, halved: 10000 with nothing earned loses 1000.
        assert!((decay(10000.0, 0.0) - 9000.0).abs() < 0.01);
        assert!((decay(0.0, 400.0) - 399.5).abs() < 1.0);
    }

    #[test]
    fn the_week_turns_on_its_weekday() {
        // 2026-10-14 is a Wednesday.
        let wed = chrono::NaiveDate::from_ymd_opt(2026, 10, 14)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
            .div_euclid(86_400);
        assert_eq!(week_start(wed, 3), wed);
        assert_eq!(week_start(wed + 6, 3), wed);
        assert_eq!(week_start(wed + 7, 3), wed + 7);
        assert_eq!(week_start(wed, 2), wed - 1);
    }
}
