//! The setup's face: questions (dialoguer), progress bars and spinners
//! (indicatif), Ctrl-C, and logs kept clear of the bars.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use dialoguer::console::Term;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use rustigator_mapbuild::progress::{Progress, Stage};

use super::maps::MapInfo;
use super::plan::{Todo, Watch};

/// The bars being drawn, for the logs to go past.
static BARS: Mutex<Option<MultiProgress>> = Mutex::new(None);

fn bars() -> Option<MultiProgress> {
    BARS.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// A line for the player, above the bars if they are drawn.
pub fn say(line: impl AsRef<str>) {
    match bars() {
        Some(m) if !m.is_hidden() => {
            let _ = m.println(line);
        }
        _ => eprintln!("{}", line.as_ref()),
    }
}

/// The logs' writer (stdout), which clears the bars for each line and
/// draws them again after.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogWriter;

pub struct LogLine(Vec<u8>);

impl Write for LogLine {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for LogLine {
    fn drop(&mut self) {
        let write = || {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(&self.0);
            let _ = out.flush();
        };
        match bars() {
            Some(m) => m.suspend(write),
            None => write(),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogWriter {
    type Writer = LogLine;

    fn make_writer(&'a self) -> LogLine {
        LogLine(Vec::new())
    }
}

/// Ctrl-C while the setup runs: the first press stops the search, or the
/// build after the tile in hand; a second ends the process. Watched until
/// dropped, on a thread of its own.
pub struct CtrlC {
    pub stop: Arc<AtomicBool>,
    end: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl CtrlC {
    pub fn watch() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (end, ended) = tokio::sync::oneshot::channel::<()>();
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("setup-ctrl-c".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                rt.block_on(async move {
                    tokio::pin!(ended);
                    loop {
                        tokio::select! {
                            _ = &mut ended => return,
                            r = tokio::signal::ctrl_c() => {
                                if r.is_err() {
                                    return;
                                }
                                if flag.swap(true, Ordering::SeqCst) {
                                    let _ = Term::stderr().show_cursor();
                                    eprintln!();
                                    std::process::exit(130);
                                }
                                say("Stopping (Ctrl-C again to quit at once).");
                            }
                        }
                    }
                });
            })
            .ok();
        Self {
            stop,
            end: Some(end),
            thread,
        }
    }

    pub fn pressed(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Forget a press that has been acted on.
    pub fn reset(&self) {
        self.stop.store(false, Ordering::SeqCst);
    }
}

impl Drop for CtrlC {
    fn drop(&mut self) {
        if let Some(end) = self.end.take() {
            let _ = end.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// `d` as a person reads it: "4m 10s", "52s", "1h 03m".
pub fn short(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, s % 3600 / 60),
    }
}

fn style(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template)
        .expect("a progress template")
        .progress_chars("#-")
        .with_key("eta", |s: &ProgressState, w: &mut dyn std::fmt::Write| {
            let _ = w.write_str(&short(s.eta()));
        })
        .with_key(
            "elapsed",
            |s: &ProgressState, w: &mut dyn std::fmt::Write| {
                let _ = w.write_str(&short(s.elapsed()));
            },
        )
}

const OVERALL: &str = "{prefix:<9} [{bar:24}] {percent:>3}%   {elapsed} so far, about {eta} left";
const TILES: &str = "{prefix:<9} [{bar:24}] {msg} {human_pos} / {human_len} tiles   ETA {eta}";
const MODELS: &str = "{prefix:<9} [{bar:24}] {human_pos} / {human_len} models";

/// A spinner with a message, for the search and the checks.
pub fn spinner(message: impl Into<std::borrow::Cow<'static, str>>) -> ProgressBar {
    let s = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
    s.set_style(ProgressStyle::with_template("{spinner} {wide_msg}").expect("a spinner template"));
    s.set_message(message);
    s.enable_steady_tick(Duration::from_millis(100));
    s
}

fn key(stage: Stage<'_>) -> String {
    match stage {
        Stage::Models => "models".to_owned(),
        Stage::Navmesh(m) => format!("navmesh {m}"),
        Stage::Terrain(m) => format!("terrain {m}"),
    }
}

fn title(name: &str) -> &str {
    MapInfo::by_name(name).map_or(name, |m| m.title)
}

#[derive(Default)]
struct State {
    terrain: Option<ProgressBar>,
    /// The terrain's maps not done yet.
    terrain_left: HashSet<String>,
    models: Option<ProgressBar>,
    navmesh: HashMap<String, ProgressBar>,
    /// Steps done and the total of each stage begun.
    steps: HashMap<String, (u64, u64)>,
    done: HashSet<String>,
}

/// The build's bars: the whole of it, the terrain, the models, and the map
/// being built. Each map, once built, leaves a line above them.
pub struct Bars {
    multi: MultiProgress,
    overall: ProgressBar,
    /// Each stage's share of the overall bar.
    weights: HashMap<String, u64>,
    state: Mutex<State>,
    stop: Arc<AtomicBool>,
}

impl Bars {
    pub fn new(todo: &Todo, stop: Arc<AtomicBool>) -> Self {
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
        let stages = todo.stages();
        let weights: HashMap<String, u64> = stages.iter().map(|&(s, w)| (key(s), w)).collect();
        let overall = multi.add(
            ProgressBar::new(stages.iter().map(|s| s.1).sum())
                .with_style(style(OVERALL))
                .with_prefix("Overall"),
        );
        let mut state = State::default();
        if !todo.terrain.is_empty() {
            let tiles = todo.terrain.iter().map(|m| m.terrain_tiles as u64).sum();
            state.terrain = Some(
                multi.add(
                    ProgressBar::new(tiles)
                        .with_style(style(TILES))
                        .with_prefix("Terrain"),
                ),
            );
            state.terrain_left = todo
                .terrain
                .iter()
                .map(|m| key(Stage::Terrain(m.name)))
                .collect();
        }
        if todo.models {
            state.models = Some(
                multi.add(
                    ProgressBar::new(0)
                        .with_style(style(MODELS))
                        .with_prefix("Models"),
                ),
            );
        }
        *BARS.lock().unwrap_or_else(PoisonError::into_inner) = Some(multi.clone());
        Self {
            multi,
            overall,
            weights,
            state: Mutex::new(state),
            stop,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update_overall(&self, state: &State) {
        let done: f64 = self
            .weights
            .iter()
            .map(|(k, &w)| {
                let part = if state.done.contains(k) {
                    1.0
                } else {
                    match state.steps.get(k) {
                        Some(&(n, total)) if total > 0 => (n as f64 / total as f64).min(1.0),
                        _ => 0.0,
                    }
                };
                w as f64 * part
            })
            .sum();
        self.overall.set_position(done as u64);
    }

    fn line(&self, line: String) {
        if self.multi.is_hidden() {
            eprintln!("{line}");
        } else {
            let _ = self.multi.println(line);
        }
    }
}

impl Progress for Bars {
    fn started(&self, stage: Stage<'_>, total: u64) {
        let mut state = self.state();
        match stage {
            Stage::Terrain(name) => {
                if let Some(bar) = &state.terrain {
                    if !state.steps.keys().any(|k| k.starts_with("terrain ")) {
                        bar.reset_elapsed();
                        bar.reset_eta();
                    }
                    // The estimate's count for this map, made the real one.
                    let expected = MapInfo::by_name(name).map_or(0, |m| m.terrain_tiles as u64);
                    let len = bar.length().unwrap_or(0);
                    bar.set_length(len.saturating_sub(expected) + total);
                    bar.set_message(title(name).to_owned());
                }
            }
            Stage::Models => {
                if let Some(bar) = &state.models {
                    bar.set_length(total);
                    bar.reset_elapsed();
                    bar.reset_eta();
                }
            }
            Stage::Navmesh(name) => {
                let bar = self.multi.add(
                    ProgressBar::new(total)
                        .with_style(style(TILES))
                        .with_prefix("Navmesh")
                        .with_message(title(name).to_owned()),
                );
                state.navmesh.insert(key(stage), bar);
            }
        }
        state.steps.insert(key(stage), (0, total));
        self.update_overall(&state);
    }

    fn advanced(&self, stage: Stage<'_>, n: u64) {
        let mut state = self.state();
        let k = key(stage);
        if let Some(s) = state.steps.get_mut(&k) {
            s.0 += n;
        }
        let bar = match stage {
            Stage::Terrain(_) => state.terrain.as_ref(),
            Stage::Models => state.models.as_ref(),
            Stage::Navmesh(_) => state.navmesh.get(&k),
        };
        if let Some(bar) = bar {
            bar.inc(n);
        }
        self.update_overall(&state);
    }

    fn finished(&self, stage: Stage<'_>) {
        self.done(stage);
    }

    fn cancelled(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

impl Watch for Bars {
    fn done(&self, stage: Stage<'_>) {
        let mut state = self.state();
        let k = key(stage);
        if !state.done.insert(k.clone()) {
            return;
        }
        let steps = state.steps.get(&k).map_or(0, |s| s.1);
        match stage {
            Stage::Terrain(_) => {
                state.terrain_left.remove(&k);
                if state.terrain_left.is_empty()
                    && let Some(bar) = state.terrain.take()
                {
                    bar.finish_and_clear();
                    self.multi.remove(&bar);
                    self.line(format!(
                        "{:<9} {} tiles in {}",
                        "Terrain",
                        bar.position(),
                        short(bar.elapsed())
                    ));
                }
            }
            Stage::Models => {
                if let Some(bar) = state.models.take() {
                    bar.finish_and_clear();
                    self.multi.remove(&bar);
                    self.line(format!(
                        "{:<9} {} models in {}",
                        "Models",
                        bar.position(),
                        short(bar.elapsed())
                    ));
                }
            }
            Stage::Navmesh(name) => {
                let elapsed = state.navmesh.remove(&k).map(|bar| {
                    bar.finish_and_clear();
                    self.multi.remove(&bar);
                    bar.elapsed()
                });
                self.line(format!(
                    "{:<9} {}: {steps} tiles in {}",
                    "Navmesh",
                    title(name),
                    short(elapsed.unwrap_or_default())
                ));
            }
        }
        self.update_overall(&state);
    }
}

impl Drop for Bars {
    fn drop(&mut self) {
        let state = self.state();
        for bar in state
            .terrain
            .iter()
            .chain(&state.models)
            .chain(state.navmesh.values())
            .chain([&self.overall])
        {
            bar.finish_and_clear();
        }
        drop(state);
        let _ = self.multi.clear();
        *BARS.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The player's answer, or `None` if they backed out (Esc, Ctrl-C) or
/// there is no terminal to ask on.
pub fn confirm(prompt: &str, default: bool) -> Option<bool> {
    let answer = dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(default)
        .interact_opt();
    restore(answer)
}

pub fn select(prompt: &str, items: &[String], default: usize) -> Option<usize> {
    let answer = dialoguer::Select::new()
        .with_prompt(prompt)
        .items(items)
        .default(default)
        .interact_opt();
    restore(answer)
}

pub fn pick(prompt: &str, items: &[String], checked: &[bool]) -> Option<Vec<usize>> {
    let answer = dialoguer::MultiSelect::new()
        .with_prompt(prompt)
        .items(items)
        .defaults(checked)
        .max_length(16)
        .interact_opt();
    restore(answer)
}

/// The cursor back, which a prompt cut short leaves hidden.
fn restore<T>(answer: dialoguer::Result<Option<T>>) -> Option<T> {
    match answer {
        Ok(a) => a,
        Err(_) => {
            let _ = Term::stderr().show_cursor();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_short() {
        assert_eq!(short(Duration::from_secs(52)), "52s");
        assert_eq!(short(Duration::from_secs(250)), "4m 10s");
        assert_eq!(short(Duration::from_secs(3780)), "1h 03m");
    }

    #[test]
    fn the_bars_follow_a_build_to_the_end_without_a_terminal() {
        use super::super::builder::Fake;
        use super::super::discover::tests::Temp;
        use super::super::maps::Selection;
        use super::super::plan::{Dirs, build};
        let t = Temp::new("bars");
        let dirs = Dirs {
            nav: t.0.join("nav"),
            terrain: t.0.join("terrain"),
        };
        let fake = Fake::default();
        let todo = Todo::of(
            &Selection::parse("Shadowfang,PVPZone02").unwrap(),
            &dirs,
            &fake,
        );
        let bars = Bars::new(&todo, Arc::new(AtomicBool::new(false)));
        build(
            std::path::Path::new("client"),
            &dirs,
            &todo,
            1,
            &fake,
            &bars,
        )
        .unwrap();
        assert_eq!(bars.overall.position(), bars.overall.length().unwrap());
        assert_eq!(bars.state().done.len(), 4);
        assert!(bars.state().terrain.is_none() && bars.state().navmesh.is_empty());
    }
}
