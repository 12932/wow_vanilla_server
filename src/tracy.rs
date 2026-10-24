//! Tracy, the frame profiler (docs/PROFILING.md): a frame per map update,
//! a zone per update step, packet and store transaction, plots once a
//! second, and the log's warnings and errors.
//!
//! With the `tracy` feature (on by default) and `WOW_TRACY=1`, the client
//! starts at boot and listens on 127.0.0.1:8086 for the profiler; it
//! records only while a profiler is connected (Tracy's on-demand mode).
//! Without `WOW_TRACY=1` nothing starts, and every helper here is one
//! relaxed load of a flag, false. Without the feature they are nothing.
//!
//! The helpers:
//! - [`zone!`](crate::zone) for a zone named by a literal, and
//!   [`Zone::named`] for one named at run time (an update step, an opcode);
//! - [`FrameName`] and [`Frame`], a frame per map update;
//! - [`Plot`], a value over time;
//! - [`messages`], the log's warnings and errors as Tracy messages.

// Without the feature, what would go to Tracy goes nowhere.
#![cfg_attr(
    not(feature = "tracy"),
    allow(dead_code, unused_variables, clippy::needless_return)
)]

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether zones, frames and plots are being made: Tracy started (or, in
/// a test, the recorder on).
static ON: AtomicBool = AtomicBool::new(false);

/// Whether Tracy is on. A relaxed load of a flag: checked before anything
/// else is done.
#[inline(always)]
pub fn on() -> bool {
    cfg!(any(feature = "tracy", test)) && ON.load(Ordering::Relaxed)
}

/// Start Tracy if `WOW_TRACY=1`. True if it started.
pub fn start_from_env() -> bool {
    if !matches!(std::env::var("WOW_TRACY").as_deref(), Ok("1")) {
        return false;
    }
    #[cfg(feature = "tracy")]
    {
        let _ = tracy_client::Client::start();
        ON.store(true, Ordering::Relaxed);
        true
    }
    #[cfg(not(feature = "tracy"))]
    {
        eprintln!("WOW_TRACY=1, but this build has no Tracy: build it with `--features tracy`");
        false
    }
}

/// Whether Tracy itself is running (not just the tests' recorder).
#[inline]
fn tracy_running() -> bool {
    #[cfg(feature = "tracy")]
    return tracy_client::Client::is_running();
    #[cfg(not(feature = "tracy"))]
    return false;
}

/// Leak a string with a NUL after it, once for each distinct string: Tracy
/// keeps the pointers of names (and tells frames and plots apart by them).
fn intern(s: &str) -> &'static str {
    use parking_lot::Mutex;
    static NAMES: Mutex<Option<ahash::AHashMap<String, &'static str>>> = Mutex::new(None);
    let mut names = NAMES.lock();
    let names = names.get_or_insert_with(Default::default);
    if let Some(n) = names.get(s) {
        return n;
    }
    let mut owned = String::with_capacity(s.len() + 1);
    owned.push_str(s);
    owned.push('\0');
    let leaked: &'static str = Box::leak(owned.into_boxed_str());
    names.insert(s.to_owned(), leaked);
    leaked
}

/// Where a zone is: its name, function, file and line, NUL-terminated.
pub struct Location {
    /// The name, NUL-terminated (for the tests' recorder and the world
    /// benchmark's probe).
    #[cfg_attr(not(test), allow(dead_code))]
    name: &'static str,
    #[cfg(feature = "tracy")]
    data: tracy_client::sys::___tracy_source_location_data,
}

// SAFETY: the pointers are to `'static` strings, never written.
unsafe impl Sync for Location {}
unsafe impl Send for Location {}

impl Location {
    /// `name`, `function` and `file` end in a NUL (the `zone!` macro adds
    /// them).
    pub const fn new(
        name: &'static str,
        function: &'static str,
        file: &'static str,
        line: u32,
    ) -> Self {
        #[cfg(not(feature = "tracy"))]
        let _ = (function, file, line);
        Self {
            name,
            #[cfg(feature = "tracy")]
            data: tracy_client::sys::___tracy_source_location_data {
                name: name.as_ptr().cast(),
                function: function.as_ptr().cast(),
                file: file.as_ptr().cast(),
                line,
                color: 0,
            },
        }
    }

    /// The location of a zone named at run time, made once for each name
    /// (and kept for good: Tracy keeps its pointer).
    fn interned(
        name: &'static str,
        function: &'static str,
        at: &'static std::panic::Location<'static>,
    ) -> &'static Location {
        use parking_lot::Mutex;
        static LOCATIONS: Mutex<
            Option<ahash::AHashMap<(&'static str, &'static str), &'static Location>>,
        > = Mutex::new(None);
        let mut locs = LOCATIONS.lock();
        let locs = locs.get_or_insert_with(Default::default);
        if let Some(l) = locs.get(&(function, name)) {
            return l;
        }
        let loc: &'static Location = Box::leak(Box::new(Location::new(
            intern(name),
            intern(function),
            intern(at.file()),
            at.line(),
        )));
        locs.insert((function, name), loc);
        loc
    }
}

/// A zone: from its making to its drop. Not to be held over an `.await`
/// (Tracy wants a zone ended on the thread it began on), which `!Send`
/// makes sure of.
#[must_use]
pub struct Zone {
    #[cfg(feature = "tracy")]
    ctx: Option<tracy_client::sys::___tracy_c_zone_context>,
    /// For the world benchmark's probe, while it records: the zone's name
    /// and start (`map::bench_probe::zone_done`).
    #[cfg(all(feature = "world-bench", unix))]
    probe: Option<(&'static str, std::time::Instant)>,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Zone {
    const OFF: Zone = Zone {
        #[cfg(feature = "tracy")]
        ctx: None,
        #[cfg(all(feature = "world-bench", unix))]
        probe: None,
        _not_send: std::marker::PhantomData,
    };

    /// A zone at `loc` (use [`zone!`](crate::zone)).
    #[inline(always)]
    pub fn begin(loc: &'static Location) -> Zone {
        if !on() {
            return Zone::probed(loc.name);
        }
        Zone::begin_on(loc)
    }

    /// Off, but timed for the world benchmark's probe while it records.
    #[inline(always)]
    fn probed(name: &'static str) -> Zone {
        #[cfg(all(feature = "world-bench", unix))]
        if crate::game::map::bench_probe::enabled() {
            return Zone {
                probe: Some((name, std::time::Instant::now())),
                ..Zone::OFF
            };
        }
        let _ = name;
        Zone::OFF
    }

    #[inline(never)]
    fn begin_on(loc: &'static Location) -> Zone {
        #[cfg(test)]
        record::push(record::Event::Zone(loc.name.trim_end_matches('\0')));
        #[cfg(feature = "tracy")]
        if tracy_running() {
            // SAFETY: Tracy is running, and `loc` lives for good.
            let ctx = unsafe { tracy_client::sys::___tracy_emit_zone_begin(&loc.data, 1) };
            return Zone {
                ctx: Some(ctx),
                ..Zone::OFF
            };
        }
        let _ = loc;
        Zone::OFF
    }

    /// A zone named at run time: `name` (an update step, an opcode) under
    /// `function` (what kind of zone it is), at the caller's line.
    #[inline(always)]
    #[track_caller]
    pub fn named(function: &'static str, name: &'static str) -> Zone {
        if !on() {
            let _ = function;
            return Zone::probed(name);
        }
        Zone::begin_on(Location::interned(
            name,
            function,
            std::panic::Location::caller(),
        ))
    }

    /// A number shown with the zone (a count of what it did).
    #[inline]
    pub fn value(&self, v: u64) {
        #[cfg(feature = "tracy")]
        if let Some(ctx) = self.ctx {
            // SAFETY: a zone begun and not yet ended.
            unsafe { tracy_client::sys::___tracy_emit_zone_value(ctx, v) };
        }
        let _ = v;
    }

    /// Text shown with the zone.
    #[inline]
    pub fn text(&self, text: &str) {
        #[cfg(feature = "tracy")]
        if let Some(ctx) = self.ctx {
            // SAFETY: a zone begun and not yet ended; Tracy copies the text.
            unsafe {
                tracy_client::sys::___tracy_emit_zone_text(ctx, text.as_ptr().cast(), text.len())
            };
        }
        let _ = text;
    }
}

impl Drop for Zone {
    #[inline]
    fn drop(&mut self) {
        #[cfg(all(feature = "world-bench", unix))]
        if let Some((name, at)) = self.probe {
            crate::game::map::bench_probe::zone_done(name, at.elapsed());
        }
        #[cfg(feature = "tracy")]
        if let Some(ctx) = self.ctx {
            // SAFETY: a zone begun, ended once, on its thread.
            unsafe { tracy_client::sys::___tracy_emit_zone_end(ctx) };
        }
    }
}

/// A zone named by a literal, from here to the end of the scope:
/// `let _z = zone!("visibility");`. Bind it (`_z`, not `_`), or it ends at
/// once.
#[macro_export]
macro_rules! zone {
    ($name:literal) => {{
        static LOC: $crate::tracy::Location = $crate::tracy::Location::new(
            concat!($name, "\0"),
            concat!(module_path!(), "\0"),
            concat!(file!(), "\0"),
            line!(),
        );
        $crate::tracy::Zone::begin(&LOC)
    }};
}

/// The name of a set of frames: one per map, its updates.
#[derive(Clone, Copy, Debug)]
pub struct FrameName(&'static str);

impl FrameName {
    pub fn new(name: &str) -> Self {
        Self(intern(name))
    }
}

/// A frame (a map's update), from its making to its drop: Tracy's
/// discontinuous frames, so a frame's length is the update's time, not
/// the time between updates.
#[must_use]
pub struct Frame {
    name: Option<&'static str>,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Frame {
    #[inline]
    pub fn begin(name: Option<FrameName>) -> Frame {
        let name = name.filter(|_| on()).map(|n| n.0);
        if let Some(n) = name {
            #[cfg(test)]
            record::push(record::Event::Frame(n.trim_end_matches('\0')));
            #[cfg(feature = "tracy")]
            if tracy_running() {
                // SAFETY: Tracy is running; the name is NUL-terminated and
                // kept for good.
                unsafe { tracy_client::sys::___tracy_emit_frame_mark_start(n.as_ptr().cast()) };
            }
        }
        Frame {
            name,
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for Frame {
    #[inline]
    fn drop(&mut self) {
        #[cfg(feature = "tracy")]
        if let Some(n) = self.name
            && tracy_running()
        {
            // SAFETY: as in `begin`.
            unsafe { tracy_client::sys::___tracy_emit_frame_mark_end(n.as_ptr().cast()) };
        }
    }
}

/// How a plot's values are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Number,
    /// Bytes, shown as KB, MB.
    Bytes,
}

/// A value plotted over time.
#[derive(Clone, Copy, Debug)]
pub struct Plot {
    name: &'static str,
    format: Format,
}

impl Plot {
    pub fn new(name: &str, format: Format) -> Self {
        Self {
            name: intern(name),
            format,
        }
    }

    /// The plot's value now.
    #[inline]
    pub fn set(&self, v: f64) {
        if !on() {
            return;
        }
        #[cfg(test)]
        record::push(record::Event::Plot(self.name.trim_end_matches('\0'), v));
        #[cfg(feature = "tracy")]
        if tracy_running() {
            let name = self.name.as_ptr().cast();
            let format = match self.format {
                Format::Number => tracy_client::sys::TracyPlotFormatEnum_TracyPlotFormatNumber,
                Format::Bytes => tracy_client::sys::TracyPlotFormatEnum_TracyPlotFormatMemory,
            } as i32;
            // SAFETY: Tracy is running; the name is NUL-terminated and kept
            // for good. The config goes with each value (a second apart),
            // as one sent before the profiler connected is lost.
            unsafe {
                tracy_client::sys::___tracy_emit_plot_config(name, format, 1, 1, 0);
                tracy_client::sys::___tracy_emit_plot(name, v);
            }
        }
    }
}

/// The log's warnings and errors, as Tracy messages (yellow and red).
pub fn messages<S: tracing::Subscriber>() -> Option<impl tracing_subscriber::Layer<S>> {
    tracy_running().then_some(Messages)
}

struct Messages;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Messages {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let level = *event.metadata().level();
        if level > tracing::Level::WARN {
            return;
        }
        let mut text = String::new();
        event.record(&mut Text(&mut text));
        let color: u32 = if level == tracing::Level::ERROR {
            0xFF4040
        } else {
            0xFFC040
        };
        #[cfg(feature = "tracy")]
        if let Some(c) = tracy_client::Client::running() {
            c.color_message(&text, color, 0);
        }
        let _ = color;
    }
}

/// An event's message, then its other fields as `name=value`.
struct Text<'a>(&'a mut String);

impl tracing::field::Visit for Text<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write as _;
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, "{}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            if !self.0.is_empty() {
                self.0.push(' ');
            }
            self.0.push_str(value);
        } else {
            self.record_debug(field, &value);
        }
    }
}

/// What the helpers emitted, for the tests: the recorder turns them on
/// (without Tracy), and records every zone, frame and plot, from every
/// thread, in order. Each test is a process of its own under nextest.
#[cfg(test)]
pub mod record {
    use parking_lot::Mutex;

    #[derive(Clone, Debug, PartialEq)]
    pub enum Event {
        Zone(&'static str),
        Frame(&'static str),
        Plot(&'static str, f64),
    }

    static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());

    pub(super) fn push(e: Event) {
        let mut events = EVENTS.lock();
        // A bound, for a test that records a long run.
        if events.len() < 1 << 20 {
            events.push(e);
        }
    }

    /// Turn the helpers on, recording.
    pub fn start() {
        super::ON.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// What has been recorded, taken.
    pub fn take() -> Vec<Event> {
        std::mem::take(&mut EVENTS.lock())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    use wow_world_messages::vanilla::*;

    use super::record::{self, Event};
    use crate::game::harness::{self, Client};

    fn zones(events: &[Event]) -> Vec<&'static str> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Zone(z) => Some(*z),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn helpers_off_emit_nothing_and_cost_a_flag() {
        assert!(!super::on());
        let n = 1_000_000u32;
        let started = Instant::now();
        for _ in 0..n {
            let z = crate::zone!("off");
            z.value(1);
            let _n = super::Zone::named("update step", "off too");
        }
        let each = started.elapsed() / n;
        assert!(record::take().is_empty());
        // A load and a branch; even unoptimized, far below a microsecond.
        assert!(each < Duration::from_micros(1), "{each:?} a zone");
    }

    #[tokio::test]
    async fn a_map_update_is_a_frame_of_step_zones_a_packet_a_zone_and_the_plots_go_on() {
        record::start();
        let realm = harness::realm();
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        a.heartbeat(-8949.0, -132.0, 83.5).await;
        a.say("hello").await;
        a.wait_for("the say", |m| match m {
            ServerOpcodeMessage::SMSG_MESSAGECHAT(c) if c.message == "hello" => Some(()),
            _ => None,
        })
        .await;
        // Some updates after it.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let events = record::take();
        let zones = zones(&events);
        for z in [
            "populate",
            "arrival",
            "CMSG_MESSAGECHAT",
            // A movement packet by its opcode, not wow_messages's
            // `MSG_MOVE_HEARTBEAT_Client`.
            "MSG_MOVE_HEARTBEAT",
            "update",
            "players",
            "gear",
            "creatures",
            "creatures' turns",
            "visibility",
            "values",
            "values: players",
            "values: creatures",
            "regeneration",
            "despawn timers",
            "units with auras",
            "batches",
        ] {
            assert!(zones.contains(&z), "no zone {z:?} in {zones:?}");
        }
        // Each update a frame of its map's, its steps within it.
        let frames: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                Event::Frame(f) => Some(*f),
                _ => None,
            })
            .collect();
        assert!(frames.len() >= 3, "{frames:?}");
        let map = frames[0];
        let first = events.iter().position(|e| *e == Event::Frame(map)).unwrap();
        assert_eq!(events[first + 1], Event::Zone("update"));
        assert_eq!(events[first + 2], Event::Zone("players"));
        // The map's plots, with Alice in it.
        let players = format!("{map}: players");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Plot(p, v) if *p == players && *v == 1.0)),
            "no {players:?} of 1 in {events:?}"
        );
        for p in ["creatures awake", "mailbox", "outbound queued"] {
            let name = format!("{map}: {p}");
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, Event::Plot(n, _) if *n == name)),
                "no plot {name:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_save_is_a_store_transaction_zone() {
        record::start();
        let realm = harness::realm();
        let mut a = Client::connect(&realm, "A");
        a.enter("Alice", Race::Human, Class::Warrior).await;
        a.send(CMSG_LOGOUT_REQUEST {}).await;
        a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
        })
        .await;
        realm.store.flush().await;
        let events = record::take();
        assert!(zones(&events).contains(&"store transaction"), "{events:?}");
        assert_eq!(realm.store.queued(), 0);
    }
}
