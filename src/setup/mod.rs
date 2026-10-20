//! The data setup (docs/DATA_SETUP_PLAN.md): asks where the player's WoW
//! 1.12.1 client is and builds the navmesh (`data/nav`) and the terrain
//! (`data/terrain`) from it, with a question or two and progress bars.
//! `wvs-setup` runs it; the server offers it at start-up, on a terminal,
//! when either is missing.

pub mod builder;
pub mod discover;
pub mod maps;
mod pe;
pub mod plan;
mod platform;
pub mod ui;

use std::io::IsTerminal;
use std::path::PathBuf;
#[cfg(test)]
use std::time::{Duration, Instant};

pub use builder::{Builder, Fake, Rustigator};
pub use maps::Selection;
pub use plan::Dirs;

use discover::Client;
use maps::{MAPS, about};
use plan::{Origin, Stop, Todo};
use ui::say;

/// What the player asked for on the command line.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The client: its folder, its `WoW.exe` or its `Data`; else `WOW_DIR`;
    /// else asked.
    pub client: Option<PathBuf>,
    /// What to build; else asked, or everything.
    pub maps: Option<Selection>,
    /// Ask nothing: the client given, and everything (or `maps`).
    pub yes: bool,
    /// Build afresh what another client built, unasked (setting it aside).
    pub fresh: bool,
    pub threads: Option<u32>,
}

/// How the setup ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// What was asked is built (or was already).
    Built,
    /// Ctrl-C: what was finished is kept.
    Stopped,
    /// The player said no.
    Declined,
}

/// Whether someone is at a terminal to ask.
pub fn terminal() -> bool {
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
}

/// Find the client, choose what to build, check, and build it into `dirs`.
/// Asks unless `opts.yes`. An error says what to do.
pub fn run(opts: &Options, dirs: &Dirs, builder: &dyn Builder) -> Result<Outcome, String> {
    let ctrl_c = ui::CtrlC::watch();
    let ask = !opts.yes;
    let Some(client) = choose_client(opts, ask, dirs)? else {
        return Ok(Outcome::Declined);
    };
    let Some(marks) = settle(opts, ask, dirs, &client)? else {
        return Ok(Outcome::Declined);
    };
    let Some(selection) = choose_maps(opts, ask, dirs, builder) else {
        return Ok(Outcome::Declined);
    };
    let todo = Todo::of(&selection, dirs, builder);
    if todo.is_empty() {
        say(format!(
            "Nothing left to build: {} and {} have it all.",
            dirs.nav.display(),
            dirs.terrain.display()
        ));
        return Ok(Outcome::Built);
    }
    let threads = opts
        .threads
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get() as u32));

    let checking = ui::spinner("Checking the client's files...");
    let mut names: Vec<&str> = todo.navmesh.iter().map(|m| m.name).collect();
    for m in &todo.terrain {
        if !names.contains(&m.name) {
            names.push(m.name);
        }
    }
    let checked = builder.preflight(&client.data, &names, threads);
    checking.finish_and_clear();
    if ctrl_c.pressed() {
        return Ok(Outcome::Stopped);
    }
    let checked = checked.map_err(|e| {
        format!(
            "{} doesn't read: {e}. If it names an archive, that one may be damaged: copy the \
             client's Data again from the original, or repair it with its launcher",
            client.data.display()
        )
    })?;
    if !checked.problems.is_empty() {
        let shown: Vec<String> = checked
            .problems
            .iter()
            .take(10)
            .map(|p| format!("  {p}"))
            .collect();
        return Err(format!(
            "the client at {} is missing files the build needs:\n{}{}\nCopy its Data again from \
             the original, or repair it with its launcher",
            client.root.display(),
            shown.join("\n"),
            match checked.problems.len() {
                n if n > 10 => format!("\n  and {} more", n - 10),
                _ => String::new(),
            }
        ));
    }
    plan::check_space(dirs, &todo, platform::free_space)?;
    for dir in &marks {
        let writes = (*dir == dirs.nav && (todo.models || !todo.navmesh.is_empty()))
            || (*dir == dirs.terrain && !todo.terrain.is_empty());
        if writes {
            plan::mark(dir, &client)
                .map_err(|e| format!("can't write in {}: {e}", dir.display()))?;
        }
    }

    let bars = ui::Bars::new(&todo, ctrl_c.stop.clone());
    let result = plan::build(&client.data, dirs, &todo, threads, builder, &bars);
    drop(bars);
    match result {
        Ok(built) => {
            let mut wrote = Vec::new();
            if todo.models || !todo.navmesh.is_empty() {
                wrote.push(format!(
                    "{} ({})",
                    dirs.nav.display(),
                    maps::size(plan::dir_size(&dirs.nav))
                ));
            }
            if !todo.terrain.is_empty() {
                wrote.push(format!(
                    "{} ({})",
                    dirs.terrain.display(),
                    maps::size(plan::dir_size(&dirs.terrain))
                ));
            }
            say(format!(
                "Done: {} in {}.",
                wrote.join(", "),
                ui::short(built.elapsed)
            ));
            Ok(Outcome::Built)
        }
        Err(Stop::Cancelled) => {
            say("Stopped. What was finished is kept: run the setup again to carry on.");
            Ok(Outcome::Stopped)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// What to do with output another client built (or something the setup
/// didn't): build it afresh, setting the old aside, or keep it and build
/// what it's missing. The directories to note this client in; `None` to
/// stop.
fn settle(
    opts: &Options,
    ask: bool,
    dirs: &Dirs,
    client: &Client,
) -> Result<Option<Vec<PathBuf>>, String> {
    let mut marks = Vec::new();
    for (dir, what) in [(&dirs.nav, "navmesh"), (&dirs.terrain, "terrain")] {
        let (said, fresh_first) = match plan::origin(dir, client) {
            Origin::Empty => {
                marks.push(dir.clone());
                continue;
            }
            Origin::Same => continue,
            Origin::Other(other) => (
                format!("{} was built from another client: {other}.", dir.display()),
                true,
            ),
            Origin::Unknown => (
                format!(
                    "{} holds a {what} the setup didn't build, from a client it can't tell.",
                    dir.display()
                ),
                false,
            ),
        };
        let fresh = if opts.fresh {
            true
        } else if ask {
            say(&said);
            let items = [
                format!(
                    "Build it afresh from this client (keeping the old one as {}.old)",
                    dir.display()
                ),
                "Keep it, and build only what it's missing".to_owned(),
                "Stop".to_owned(),
            ];
            match ui::select("What now?", &items, if fresh_first { 0 } else { 1 }) {
                Some(0) => true,
                Some(1) => false,
                _ => return Ok(None),
            }
        } else if fresh_first {
            return Err(format!(
                "{said} Give --fresh to build it afresh from this client (the old one is kept \
                 beside it)"
            ));
        } else {
            say(format!(
                "{said} Building only what it's missing (--fresh builds it afresh)."
            ));
            false
        };
        if fresh {
            let to = plan::set_aside(dir)
                .map_err(|e| format!("can't move {} aside: {e}", dir.display()))?;
            say(format!(
                "The old one is now {}: delete it when you no longer need it.",
                to.display()
            ));
            marks.push(dir.clone());
        }
    }
    Ok(Some(marks))
}

/// The client to build from: the one given, or the one the player names
/// when asked (again, with what was wrong, until it is one or they stop).
fn choose_client(opts: &Options, ask: bool, dirs: &Dirs) -> Result<Option<Client>, String> {
    let given = opts.client.clone().or_else(|| {
        std::env::var_os("WOW_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
    });
    if let Some(path) = given {
        let client = discover::given(&discover::clean(&path.to_string_lossy()))?;
        found(&client);
        return Ok(Some(client));
    }
    if !ask {
        return Err(
            "say where the WoW 1.12.1 client is: --client <path> (its folder, its WoW.exe or its \
             Data folder), or set WOW_DIR to it"
                .to_owned(),
        );
    }
    say("The navmesh and terrain are made from your WoW 1.12.1 client.");
    let last = plan::last_client(dirs).map(|p| p.display().to_string());
    loop {
        let Some(answer) = ui::input(
            "Where is it? Its folder, WoW.exe or Data folder (paste or drag it here)",
            last.as_deref(),
        ) else {
            return Ok(None);
        };
        if answer.trim().is_empty() {
            say("Nothing given: stopping. Run the setup again when you have the client.");
            return Ok(None);
        }
        match discover::given(&discover::clean(&answer)) {
            Ok(client) => {
                found(&client);
                return Ok(Some(client));
            }
            Err(e) => say(format!(
                "{e}. Try again (or press Enter on an empty line to stop)."
            )),
        }
    }
}

/// Which client the setup builds from, and what to know about it.
fn found(client: &Client) {
    say(format!("Using {}.", client.describe()));
    if let Some(note) = client.note() {
        say(format!("Note: {note}."));
    }
}

/// What to build: as given, as chosen, or everything.
fn choose_maps(opts: &Options, ask: bool, dirs: &Dirs, builder: &dyn Builder) -> Option<Selection> {
    if let Some(s) = &opts.maps {
        return Some(s.clone());
    }
    if !ask {
        return Some(Selection::everything());
    }
    let left = |s: &Selection| {
        let todo = Todo::of(s, dirs, builder);
        let (nav, terrain) = todo.bytes();
        if todo.is_empty() {
            "built already".to_owned()
        } else {
            about(nav + terrain)
        }
    };
    let everything = Selection::everything();
    let east = Selection::eastern_kingdoms();
    let items = [
        format!(
            "Everything: {} maps, {}",
            everything.navmeshes().count(),
            left(&everything)
        ),
        format!(
            "Eastern Kingdoms only (Gurubashi Arena, Stormwind, the Undercity): {}",
            left(&east)
        ),
        "Pick maps...".to_owned(),
    ];
    match ui::select("Build what?", &items, 0)? {
        0 => Some(everything),
        1 => Some(east),
        _ => {
            let items: Vec<String> = MAPS
                .iter()
                .map(|m| {
                    let only = if m.has_navmesh() {
                        ""
                    } else {
                        ", terrain only"
                    };
                    format!(
                        "{} ({}{only}): {}",
                        m.title,
                        m.name,
                        left(&Selection(vec![m]))
                    )
                })
                .collect();
            let checked: Vec<bool> = MAPS.iter().map(|m| m.id == 0).collect();
            let picked = ui::pick("Which maps? (space picks, enter builds)", &items, &checked)?;
            (!picked.is_empty()).then(|| Selection(picked.into_iter().map(|i| &MAPS[i]).collect()))
        }
    }
}

/// What of the world's data is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    pub nav: bool,
    pub terrain: bool,
}

impl Missing {
    pub fn of(dirs: &Dirs, builder: &dyn Builder) -> Self {
        Self {
            nav: !plan::has_navmesh(&dirs.nav, builder),
            terrain: !plan::has_terrain(&dirs.terrain),
        }
    }
}

/// What the server does at start-up about missing data.
#[derive(Debug, PartialEq, Eq)]
pub enum Startup {
    /// Nothing is missing.
    Ready,
    /// No one to ask: log this, and start.
    Hint(String),
    /// Say what is missing, ask, and run the setup on a yes.
    Ask { missing: String, question: String },
}

pub fn startup(missing: Missing, dirs: &Dirs, terminal: bool) -> Startup {
    let what = match (missing.nav, missing.terrain) {
        (false, false) => return Startup::Ready,
        (true, false) => format!(
            "No navmesh in {}: creatures will walk straight lines.",
            dirs.nav.display()
        ),
        (false, true) => format!(
            "No terrain in {}: liquids outside buildings have no kind (no deep-sea fatigue, no \
             lava).",
            dirs.terrain.display()
        ),
        (true, true) => format!(
            "No navmesh or terrain in {} and {}: creatures will walk straight lines, and liquids \
             outside buildings have no kind.",
            dirs.nav.display(),
            dirs.terrain.display()
        ),
    };
    let it = if missing.nav && missing.terrain {
        "them"
    } else {
        "it"
    };
    if terminal {
        Startup::Ask {
            missing: what,
            question: format!("Build {it} now from your WoW 1.12.1 client?"),
        }
    } else {
        Startup::Hint(format!(
            "{what} Build {it} from your WoW 1.12.1 client with `cargo run --release --bin \
             wvs-setup` (or `wvs-setup`)."
        ))
    }
}

/// How long the start's question waits for an answer before the server
/// starts as it is: a server started again by a script, in tmux or screen,
/// has no one to answer it.
pub const STARTUP_QUESTION_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The note that the start's question was declined (or not answered), so
/// it isn't asked again: beside the navmesh's directory (`data/`).
pub fn declined_note(dirs: &Dirs) -> PathBuf {
    dirs.nav.with_file_name("setup-declined.txt")
}

/// At the server's start: if the navmesh or the terrain is missing, offer
/// the setup when someone is at a terminal, else log how to make them.
/// Whether to start the server.
pub fn at_startup(world: &mut crate::config::WorldConfig) -> bool {
    at_startup_with(world, terminal(), &Rustigator, &mut |question| {
        ui::confirm_within(question, true, STARTUP_QUESTION_WAIT)
    })
}

/// [`at_startup`], asking with `ask`. A "no", or no answer within
/// [`STARTUP_QUESTION_WAIT`], is remembered ([`declined_note`]): the start
/// asks no more, and logs how to build what is missing instead.
pub fn at_startup_with(
    world: &mut crate::config::WorldConfig,
    terminal: bool,
    builder: &dyn Builder,
    ask: &mut dyn FnMut(&str) -> ui::Answer,
) -> bool {
    let dirs = Dirs::of(world);
    let note = declined_note(&dirs);
    let question = match startup(
        Missing::of(&dirs, builder),
        &dirs,
        terminal && !note.exists(),
    ) {
        Startup::Ready => return true,
        Startup::Hint(hint) => {
            tracing::warn!("{hint}");
            return true;
        }
        Startup::Ask { missing, question } => {
            say(missing);
            question
        }
    };
    let answer = ask(&question);
    if answer != ui::Answer::Yes {
        let why = if answer == ui::Answer::Unanswered {
            "No answer: starting"
        } else {
            "Starting"
        };
        say(format!(
            "{why} as it is: `cargo run --release --bin wvs-setup` builds what is missing later. \
             Not asked again while {} is there.",
            note.display()
        ));
        if let Some(dir) = note.parent().filter(|d| !d.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(
            &note,
            "The server's start doesn't offer to build the navmesh and the terrain while this file \
             is here: it was declined, or not answered. Build them with `cargo run --release --bin \
             wvs-setup` (or `wvs-setup`), or delete this file to be asked again.\n",
        )
        .inspect_err(|e| tracing::debug!("{}: {e}", note.display()));
        return true;
    }
    let start = match run(&Options::default(), &dirs, builder) {
        Ok(Outcome::Declined) => true,
        Ok(_) => ui::confirm("Start the server now?", true) == Some(true),
        Err(e) => {
            say(format!("The setup didn't finish: {e}."));
            ui::confirm("Start the server without it?", true) == Some(true)
        }
    };
    world.default_dirs();
    start
}

#[cfg(test)]
mod tests {
    use super::discover::tests::{Temp, fake_client};
    use super::*;

    fn dirs(t: &Temp) -> Dirs {
        Dirs {
            nav: t.0.join("nav"),
            terrain: t.0.join("terrain"),
        }
    }

    #[test]
    fn start_up_asks_only_at_a_terminal() {
        let t = Temp::new("startup");
        let d = dirs(&t);
        let nothing = Missing::of(&d, &Fake::default());
        assert_eq!(
            nothing,
            Missing {
                nav: true,
                terrain: true
            }
        );
        let Startup::Hint(hint) = startup(nothing, &d, false) else {
            panic!("asked without a terminal");
        };
        assert!(
            hint.contains("creatures will walk straight lines"),
            "{hint}"
        );
        assert!(hint.contains("wvs-setup"), "{hint}");
        let Startup::Ask { missing, question } = startup(
            Missing {
                nav: true,
                terrain: false,
            },
            &d,
            true,
        ) else {
            panic!("not asked at a terminal");
        };
        assert!(missing.starts_with("No navmesh in "), "{missing}");
        assert!(missing.ends_with("creatures will walk straight lines."));
        assert_eq!(question, "Build it now from your WoW 1.12.1 client?");
        assert_eq!(
            startup(
                Missing {
                    nav: false,
                    terrain: false
                },
                &d,
                true
            ),
            Startup::Ready
        );
    }

    #[test]
    fn start_up_without_a_terminal_starts_without_asking_or_building() {
        let t = Temp::new("startup-quiet");
        let mut world = crate::config::WorldConfig {
            nav_dir: Some(t.0.join("nav")),
            terrain_dir: Some(t.0.join("terrain")),
            ..Default::default()
        };
        let started = Instant::now();
        assert!(at_startup_with(
            &mut world,
            false,
            &Fake::default(),
            &mut |q| panic!("asked {q}")
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!t.0.join("nav").exists() && !t.0.join("terrain").exists());
    }

    /// Review 5 #20: at a terminal with no one there, the start waited for
    /// an answer for good. It starts as it is once the question goes
    /// unanswered, and a "no" or no answer isn't asked again.
    #[test]
    fn start_up_at_a_terminal_starts_unanswered_and_asks_once() {
        let t = Temp::new("startup-unanswered");
        let mut world = crate::config::WorldConfig {
            nav_dir: Some(t.0.join("nav")),
            terrain_dir: Some(t.0.join("terrain")),
            ..Default::default()
        };
        let mut asked = 0;
        assert!(at_startup_with(
            &mut world,
            true,
            &Fake::default(),
            &mut |_| {
                asked += 1;
                ui::Answer::Unanswered
            }
        ));
        assert_eq!(asked, 1);
        assert!(t.0.join("setup-declined.txt").exists());
        assert!(!t.0.join("nav").exists(), "nothing built");
        assert!(at_startup_with(
            &mut world,
            true,
            &Fake::default(),
            &mut |q| panic!("asked {q} again")
        ));
        // The question's wait ends, and a line typed in time is the answer.
        let started = Instant::now();
        let none = ui::line_within(
            || {
                std::thread::sleep(Duration::from_secs(5));
                Some("y".into())
            },
            Duration::from_millis(50),
        );
        assert_eq!(none, None);
        assert!(started.elapsed() < Duration::from_secs(2));
        let typed = ui::line_within(|| Some("n\n".into()), Duration::from_secs(5));
        assert_eq!(typed, Some(Some("n\n".into())));
    }

    #[test]
    fn a_scripted_run_builds_what_it_was_given() {
        let t = Temp::new("scripted");
        fake_client(&t.0.join("WoW"), &discover::ARCHIVES, None);
        let d = dirs(&t);
        let opts = Options {
            client: Some(t.0.join("WoW")),
            maps: Some(Selection::parse("Shadowfang,DeadminesInstance").unwrap()),
            yes: true,
            threads: Some(2),
            ..Default::default()
        };
        assert_eq!(run(&opts, &d, &Fake::default()), Ok(Outcome::Built));
        let fake = Fake::default();
        assert!(fake.navmesh_current(&d.nav, "Shadowfang"));
        assert!(fake.navmesh_current(&d.nav, "DeadminesInstance"));
        assert!(fake.models_current(&d.nav));
        assert!(plan::terrain_current(
            &d.terrain,
            MapInfo::by_name("Shadowfang").unwrap()
        ));
        // Again: built already.
        assert_eq!(run(&opts, &d, &Fake::default()), Ok(Outcome::Built));
        // A client with problems, or none at all, says what to do.
        let broken = Fake {
            problems: vec!["World\\Maps\\Azeroth\\Azeroth.wdt is missing".into()],
            ..Default::default()
        };
        let all = Options {
            maps: Some(Selection::everything()),
            ..opts.clone()
        };
        let err = run(&all, &d, &broken).unwrap_err();
        assert!(
            err.contains("Azeroth.wdt is missing") && err.contains("Copy its Data again"),
            "{err}"
        );
        let nowhere = Options {
            client: Some(t.0.join("nowhere")),
            ..opts.clone()
        };
        assert!(
            run(&nowhere, &d, &Fake::default())
                .unwrap_err()
                .contains("nothing at")
        );
        // Asking nothing, it must be told where the client is.
        let untold = Options {
            client: None,
            ..opts.clone()
        };
        if std::env::var_os("WOW_DIR").is_none() {
            assert!(
                run(&untold, &d, &Fake::default())
                    .unwrap_err()
                    .contains("--client")
            );
        }
        let tbc = t.0.join("TBC");
        fake_client(&tbc, &["common.MPQ", "expansion.MPQ"], None);
        let err = run(
            &Options {
                client: Some(tbc),
                ..opts
            },
            &d,
            &Fake::default(),
        )
        .unwrap_err();
        assert!(err.contains("The Burning Crusade"), "{err}");
    }

    #[test]
    fn what_another_client_built_is_built_afresh_only_when_asked() {
        let t = Temp::new("other-client");
        fake_client(&t.0.join("WoW"), &discover::ARCHIVES, None);
        fake_client(
            &t.0.join("Other"),
            &[&discover::ARCHIVES[..], &["patch-2.MPQ"]].concat(),
            None,
        );
        let d = dirs(&t);
        let opts = Options {
            client: Some(t.0.join("WoW")),
            maps: Some(Selection::parse("Shadowfang").unwrap()),
            yes: true,
            ..Default::default()
        };
        assert_eq!(run(&opts, &d, &Fake::default()), Ok(Outcome::Built));
        let other = Options {
            client: Some(t.0.join("Other")),
            ..opts.clone()
        };
        let err = run(&other, &d, &Fake::default()).unwrap_err();
        assert!(
            err.contains("was built from another client") && err.contains("--fresh"),
            "{err}"
        );
        let fresh = Options {
            fresh: true,
            ..other
        };
        assert_eq!(run(&fresh, &d, &Fake::default()), Ok(Outcome::Built));
        assert!(t.0.join("nav.old/Shadowfang.map").exists());
        assert!(t.0.join("terrain.old").is_dir());
        assert!(Fake::default().navmesh_current(&d.nav, "Shadowfang"));
        // A bake from elsewhere is kept, and only what it lacks is built.
        let t = Temp::new("unknown-bake");
        fake_client(&t.0.join("WoW"), &discover::ARCHIVES, None);
        let d = dirs(&t);
        std::fs::create_dir_all(&d.nav).unwrap();
        std::fs::write(d.nav.join("Shadowfang.map"), b"from elsewhere").unwrap();
        let opts = Options {
            client: Some(t.0.join("WoW")),
            ..opts
        };
        assert_eq!(run(&opts, &d, &Fake::default()), Ok(Outcome::Built));
        assert_eq!(
            std::fs::read(d.nav.join("Shadowfang.map")).unwrap(),
            b"from elsewhere"
        );
        assert!(!d.nav.join("setup.json").exists() && d.terrain.join("setup.json").exists());
    }

    use super::maps::MapInfo;
}
