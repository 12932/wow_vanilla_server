//! What spans maps: chat channels, groups, and the friend and ignore lists
//! of those online. Each sits behind its own short lock, never held across
//! an `.await`; players elsewhere are reached through the registry.

pub mod auction;
pub mod channels;
pub mod friends;
pub mod groups;
pub mod guilds;
pub mod mail;

use std::sync::Arc;

use parking_lot::Mutex;
use wow_world_messages::Guid;

use crate::game::Realm;
use crate::game::registry::Online;

#[derive(Default)]
pub struct Social {
    /// A lock a channel, behind the directory's (`channels::Channels`).
    pub channels: channels::Channels,
    pub groups: Mutex<groups::Groups>,
    pub lists: Mutex<friends::Lists>,
    pub guilds: Mutex<guilds::Guilds>,
    pub mail: Mutex<mail::Mails>,
    pub auctions: Mutex<auction::Auctions>,
    pub instances: Mutex<crate::game::instances::Instances>,
    pub battlegrounds: Mutex<crate::game::battlegrounds::Battlegrounds>,
    /// The seasonal events on.
    pub events: Mutex<ahash::AHashSet<u32>>,
    pub weather: Mutex<ahash::AHashMap<u32, crate::game::weather::Weather>>,
    pub tickets: Mutex<crate::game::map::tickets::Tickets>,
    pub meeting_stones: Mutex<crate::game::map::meeting_stones::MeetingStones>,
    pub honor: Mutex<crate::game::honor::Ledger>,
    /// Server-wide world states events publish: state → value.
    pub world_states: Mutex<ahash::AHashMap<u32, u32>>,
    /// Events the world starts and stops, not the calendar.
    pub driven_events: Mutex<ahash::AHashMap<u32, bool>>,
    /// Events a GM started or stopped: on or off, until when.
    pub gm_events: Mutex<ahash::AHashMap<u32, (bool, i64)>>,
    pub invasion: Mutex<crate::game::invasion::State>,
    pub elemental: Mutex<crate::game::elemental::State>,
    pub omen: Mutex<crate::game::omen::Watch>,
    /// Where Jaina Proudmoore is: at home in Theramore, or in a city.
    pub jaina: Mutex<crate::game::champions::State>,
    /// Where Rexxar is: at home in Desolace, or in a city.
    pub rexxar: Mutex<crate::game::champions::State>,
    /// Gurubashi Arena's champions.
    pub arena: Mutex<crate::game::map::arena::History>,
    /// What the continents' world PvP holds, for the dungeons that share
    /// its buffs.
    pub outdoor: Mutex<crate::game::map::outdoor_pvp::Shared>,
    /// Panics a test arms in the sweeps and the transports' clock.
    #[cfg(test)]
    pub faults: crate::game::supervisor::Faults,
}

impl Social {
    /// The services, with what the store kept of them.
    pub fn load(records: &[(String, u64, String)]) -> Self {
        Self {
            guilds: Mutex::new(guilds::Guilds::load(records)),
            mail: Mutex::new(mail::Mails::load(records)),
            auctions: Mutex::new(auction::Auctions::load(records)),
            instances: Mutex::new(crate::game::instances::Instances::load(records)),
            tickets: Mutex::new(crate::game::map::tickets::Tickets::load(records)),
            arena: Mutex::new(crate::game::map::arena::History::load(records)),
            honor: Mutex::new(crate::game::honor::Ledger::load(
                records,
                crate::game::honor::today(),
                crate::game::honor::DEFAULT_RESET_WEEKDAY,
            )),
            ..Default::default()
        }
    }
}

/// A message for whoever is online as `guid`.
pub fn send_to<M: wow_world_messages::vanilla::ServerMessage>(realm: &Realm, guid: Guid, msg: &M) {
    if let Some(o) = realm.registry.online(guid) {
        o.outbound.send(msg);
    }
}

/// A frame for whoever is online as `guid`.
pub fn send_frame(realm: &Realm, guid: Guid, frame: &Arc<[u8]>) {
    if let Some(o) = realm.registry.online(guid) {
        o.outbound.send_frame(frame.clone());
    }
}

/// The recipients [`send_frames`] looks up under one registry read: a
/// login waits at most this many sends for the lock.
const FAN_OUT_CHUNK: usize = 256;

/// A frame for each of `guids` online: a channel's, a guild's or a group's
/// line, sent with no service's lock held, the registry read once a
/// [`FAN_OUT_CHUNK`].
pub fn send_frames(realm: &Realm, guids: &[Guid], frame: &Arc<[u8]>) {
    for chunk in guids.chunks(FAN_OUT_CHUNK) {
        realm.registry.each_online(chunk, |o| {
            o.outbound.send_frame(frame.clone());
        });
    }
}

/// `guids` less those ignoring `speaker`: the lists asked once, not once a
/// recipient, and let go before the fan-out.
pub fn not_ignoring(realm: &Realm, speaker: Guid, guids: &mut Vec<Guid>) {
    let ignorers = realm.social.lists.lock().ignorers(speaker);
    if !ignorers.is_empty() {
        guids.retain(|g| !ignorers.contains(g));
    }
}

/// Whether `a` and `b` may talk: the same team, or either a GM.
pub fn same_side(a: &Online, b: &Online) -> bool {
    a.gm || b.gm
        || crate::game::registry::Team::of_race(a.race)
            == crate::game::registry::Team::of_race(b.race)
}

/// How often mail and auctions are swept for what has come due.
pub const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The sweeps, while the realm lasts.
pub fn start_timers(realm: &Arc<Realm>) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let weak = Arc::downgrade(realm);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        let mut watch = Watch::default();
        loop {
            tick.tick().await;
            let Some(realm) = weak.upgrade() else {
                return;
            };
            sweep_watched(&realm, &mut watch, std::time::Instant::now());
        }
    });
}

/// A sweep: its name for the log, and what it does.
type Sweep = (&'static str, fn(&Realm));

/// Everything time drives outside the maps.
const SWEEPS: [Sweep; 10] = [
    ("the auction sweep", auction::sweep),
    ("the mail sweep", mail::sweep),
    ("the instance sweep", crate::game::instances::sweep),
    ("the battleground sweep", crate::game::battlegrounds::sweep),
    ("the event sweep", crate::game::events::sweep),
    ("the invasion sweep", crate::game::invasion::sweep),
    (
        "the elemental invasion sweep",
        crate::game::elemental::sweep,
    ),
    ("the weather sweep", crate::game::weather::sweep),
    (
        "the meeting stone sweep",
        crate::game::map::meeting_stones::sweep,
    ),
    ("the pvp sweep", crate::game::map::pvp::sweep),
];

/// Everything time drives outside the maps.
pub fn sweep(realm: &Realm) {
    for (_, f) in SWEEPS {
        f(realm);
    }
}

/// Each sweep's watch.
#[derive(Debug, Default)]
pub struct Watch([crate::game::supervisor::Backoff; SWEEPS.len()]);

/// [`sweep`], each part under its own watch: one that panics rests (twice
/// as long each time it panics again) while the rest go on. Before, a
/// panic ended the task, and with it mail, auctions, instances' resets and
/// the rest, silently.
pub fn sweep_watched(realm: &Realm, watch: &mut Watch, now: std::time::Instant) {
    for ((what, f), b) in SWEEPS.iter().zip(&mut watch.0) {
        b.run(what, now, || {
            #[cfg(test)]
            realm.social.faults.check(what);
            f(realm)
        });
    }
}
