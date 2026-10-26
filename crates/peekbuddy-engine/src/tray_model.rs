//! The tray's behaviour without any Win32 (#60): state, menu, tooltip, icon
//! choice and the icon bitmaps. Everything here is pure and tested on Linux;
//! `tray_win` and `peekbuddy-win::tray` put it on screen.

/// What the tray reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Capturing normally.
    Live,
    /// All watches paused by the user.
    Paused,
    /// An unresolved gap (overflow) is recorded.
    Gap,
    /// A lost watch, a settings problem or a previous crash.
    Problem,
}

impl Health {
    const fn word(self) -> &'static str {
        match self {
            Self::Live => "Live",
            Self::Paused => "Paused",
            Self::Gap => "Gap",
            Self::Problem => "Problem",
        }
    }
}

/// One watch as the menu shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEntry {
    /// Label: the last components of the root.
    pub label: String,
    /// Paused by the user.
    pub paused: bool,
}

/// Everything the tray needs to render.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayState {
    /// Overall health.
    pub health: Health,
    /// Events per second, from writer commit notifications; 0 when idle.
    pub rate: f64,
    /// The watches, in settings order.
    pub watches: Vec<WatchEntry>,
}

impl Default for TrayState {
    fn default() -> Self {
        Self {
            health: Health::Live,
            rate: 0.0,
            watches: Vec::new(),
        }
    }
}

impl TrayState {
    /// Whether every watch is paused (and there is at least one).
    #[must_use]
    pub fn all_paused(&self) -> bool {
        self.health == Health::Paused
            || (!self.watches.is_empty() && self.watches.iter().all(|watch| watch.paused))
    }
}

/// Which icon bitmap to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconId {
    /// Capturing.
    Live,
    /// Paused.
    Paused,
    /// Needs attention.
    Problem,
}

/// The icon for `state`. A gap shows the problem icon: it needs a look.
#[must_use]
pub fn icon_id(state: &TrayState) -> IconId {
    match state.health {
        Health::Live => IconId::Live,
        Health::Paused => IconId::Paused,
        Health::Gap | Health::Problem => IconId::Problem,
    }
}

/// `NOTIFYICONDATAW.szTip` holds 128 UTF-16 units including the NUL.
pub const TIP_UNITS: usize = 128;

/// Formats an event rate: `0 ev/s`, `850 ev/s`, `1.2k ev/s`, `3.4M ev/s`.
#[must_use]
pub fn format_rate(rate: f64) -> String {
    if !rate.is_finite() || rate < 0.5 {
        return String::from("0 ev/s");
    }
    if rate < 1_000.0 {
        return format!("{rate:.0} ev/s");
    }
    if rate < 1_000_000.0 {
        return format!("{:.1}k ev/s", rate / 1_000.0);
    }
    format!("{:.1}M ev/s", rate / 1_000_000.0)
}

/// Cuts `text` to at most `max_units` UTF-16 units without splitting a
/// character, ending in an ellipsis when something was cut.
#[must_use]
pub fn truncate_units(text: &str, max_units: usize) -> String {
    if text.encode_utf16().count() <= max_units {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut units = 0;
    for ch in text.chars() {
        let width = ch.len_utf16();
        if units + width + 1 > max_units {
            break;
        }
        out.push(ch);
        units += width;
    }
    out.push('\u{2026}');
    out
}

/// The tooltip: `PeekBuddy - Live - 1.2k ev/s` while events flow, `PeekBuddy -
/// Live` when idle. Fits `szTip`.
#[must_use]
pub fn tooltip(state: &TrayState) -> String {
    let text = if state.rate >= 0.5 && state.health == Health::Live {
        format!(
            "PeekBuddy - {} - {}",
            state.health.word(),
            format_rate(state.rate)
        )
    } else {
        format!("PeekBuddy - {}", state.health.word())
    };
    truncate_units(&text, TIP_UNITS - 1)
}

/// A command the menu can issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Open or focus the viewer (default item, left click).
    OpenViewer,
    /// Pause every watch.
    PauseAll,
    /// Resume every watch.
    ResumeAll,
    /// Toggle one watch by its index in the settings.
    ToggleWatch(usize),
    /// Show the data folder in Explorer.
    OpenDataFolder,
    /// Open the viewer on the settings page.
    OpenSettings,
    /// Shut the engine down.
    Quit,
}

const ID_OPEN: u32 = 1;
const ID_PAUSE_ALL: u32 = 2;
const ID_RESUME_ALL: u32 = 3;
const ID_DATA: u32 = 4;
const ID_SETTINGS: u32 = 5;
const ID_QUIT: u32 = 6;
const ID_WATCH_BASE: u32 = 100;
const ID_WATCH_END: u32 = ID_WATCH_BASE + 100;
/// Most watches the submenu lists.
pub const MAX_MENU_WATCHES: usize = 100;

impl Command {
    /// The menu id of this command.
    #[must_use]
    pub fn id(self) -> u32 {
        match self {
            Self::OpenViewer => ID_OPEN,
            Self::PauseAll => ID_PAUSE_ALL,
            Self::ResumeAll => ID_RESUME_ALL,
            Self::OpenDataFolder => ID_DATA,
            Self::OpenSettings => ID_SETTINGS,
            Self::Quit => ID_QUIT,
            Self::ToggleWatch(index) => {
                ID_WATCH_BASE + u32::try_from(index.min(MAX_MENU_WATCHES)).unwrap_or(0)
            }
        }
    }

    /// The command behind a menu id, if it is one of ours.
    #[must_use]
    pub fn from_id(id: u32) -> Option<Self> {
        match id {
            ID_OPEN => Some(Self::OpenViewer),
            ID_PAUSE_ALL => Some(Self::PauseAll),
            ID_RESUME_ALL => Some(Self::ResumeAll),
            ID_DATA => Some(Self::OpenDataFolder),
            ID_SETTINGS => Some(Self::OpenSettings),
            ID_QUIT => Some(Self::Quit),
            other if (ID_WATCH_BASE..ID_WATCH_END).contains(&other) => {
                Some(Self::ToggleWatch((other - ID_WATCH_BASE) as usize))
            }
            _ => None,
        }
    }
}

/// One menu entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    /// A command.
    Item {
        /// Menu id; see [`Command::from_id`].
        id: u32,
        /// Label.
        label: String,
        /// The default (bold) item.
        default: bool,
        /// Shown checked.
        checked: bool,
        /// Greyed out when false.
        enabled: bool,
    },
    /// A separator line.
    Separator,
    /// A submenu.
    Submenu {
        /// Label.
        label: String,
        /// The entries.
        items: Vec<MenuItem>,
        /// Greyed out when false.
        enabled: bool,
    },
}

fn item(command: Command, label: &str) -> MenuItem {
    MenuItem::Item {
        id: command.id(),
        label: label.to_owned(),
        default: false,
        checked: false,
        enabled: true,
    }
}

/// The menu for `state`: Open PeekBuddy (default), Pause all or Resume all,
/// a submenu with one entry per watch, Open data folder, Settings, Quit.
#[must_use]
pub fn menu(state: &TrayState) -> Vec<MenuItem> {
    let mut open = item(Command::OpenViewer, "Open PeekBuddy");
    if let MenuItem::Item { default, .. } = &mut open {
        *default = true;
    }
    let pause = if state.all_paused() {
        item(Command::ResumeAll, "Resume all")
    } else {
        item(Command::PauseAll, "Pause all")
    };
    let watches: Vec<MenuItem> = state
        .watches
        .iter()
        .take(MAX_MENU_WATCHES)
        .enumerate()
        .map(|(index, watch)| MenuItem::Item {
            id: Command::ToggleWatch(index).id(),
            label: truncate_units(&watch.label, 60),
            default: false,
            checked: !watch.paused,
            enabled: true,
        })
        .collect();
    vec![
        open,
        MenuItem::Separator,
        pause,
        MenuItem::Submenu {
            label: String::from("Watches"),
            enabled: !watches.is_empty(),
            items: watches,
        },
        MenuItem::Separator,
        item(Command::OpenDataFolder, "Open data folder"),
        item(Command::OpenSettings, "Settings"),
        MenuItem::Separator,
        item(Command::Quit, "Quit"),
    ]
}

/// The effects a tray command can have. The engine implements it with real
/// ones; tests record the calls.
pub trait Actions {
    /// Open the viewer, or focus the running one.
    fn open_viewer(&mut self);
    /// Show the data folder in Explorer.
    fn open_data_folder(&mut self);
    /// Shut the engine down.
    fn quit(&mut self);
    /// The pause state of watch `index` changed.
    fn watch_paused(&mut self, index: usize, paused: bool);
    /// Every watch was paused or resumed at once (not persisted).
    fn all_paused(&mut self, paused: bool);
    /// Whether watch `index` is paused on its own switch, when the effects
    /// know (a global resume leaves such a watch paused).
    fn own_paused(&self, _index: usize) -> Option<bool> {
        None
    }
}

/// Applies `command` to `state` and runs its effect. Returns whether the
/// tray's look (icon, tooltip, menu) has to be refreshed. The real menu, a
/// tray click and a posted `WM_COMMAND` all come through here.
pub fn dispatch(state: &mut TrayState, command: Command, actions: &mut dyn Actions) -> bool {
    match command {
        Command::OpenViewer | Command::OpenSettings => {
            actions.open_viewer();
            false
        }
        Command::OpenDataFolder => {
            actions.open_data_folder();
            false
        }
        Command::Quit => {
            actions.quit();
            false
        }
        Command::PauseAll | Command::ResumeAll => {
            let paused = command == Command::PauseAll;
            actions.all_paused(paused);
            for (index, watch) in state.watches.iter_mut().enumerate() {
                watch.paused = paused || actions.own_paused(index).unwrap_or(false);
            }
            set_paused_health(state, paused);
            true
        }
        Command::ToggleWatch(index) => {
            let Some(watch) = state.watches.get_mut(index) else {
                return false;
            };
            watch.paused = !watch.paused;
            actions.watch_paused(index, watch.paused);
            let all = state.watches.iter().all(|watch| watch.paused);
            set_paused_health(state, all);
            true
        }
    }
}

/// Moves between `Live` and `Paused`; a gap or problem stays visible.
fn set_paused_health(state: &mut TrayState, all_paused: bool) {
    state.health = match (state.health, all_paused) {
        (Health::Live, true) => Health::Paused,
        (Health::Paused, false) => Health::Live,
        (other, _) => other,
    };
}

/// Taskbar theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Theme {
    /// A light taskbar: icons use dark outlines.
    Light,
    /// A dark taskbar: icons use light outlines.
    Dark,
}

/// Sizes of the icon bitmaps (all DPI scales).
pub const ICON_SIZES: [u32; 4] = [16, 20, 24, 32];

/// Renders a `size` x `size` icon as premultiplied BGRA bytes: a ring in the
/// theme's outline colour around a dot whose colour is the state.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::similar_names,
    reason = "pixel arithmetic on values between 0 and 1"
)]
pub fn render_icon(id: IconId, theme: Theme, size: u32) -> Vec<u8> {
    let (ring, dot): ([f32; 3], [f32; 3]) = (
        match theme {
            Theme::Light => [0.13, 0.13, 0.13],
            Theme::Dark => [0.95, 0.95, 0.95],
        },
        match id {
            IconId::Live => [0.18, 0.64, 0.25],
            IconId::Paused => [0.55, 0.55, 0.58],
            IconId::Problem => [0.93, 0.45, 0.10],
        },
    );
    let side = f64::from(size);
    let centre = (side - 1.0) / 2.0;
    let outer = side / 2.0 - 0.5;
    let inner = outer * 0.74;
    let pupil = outer * 0.42;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let distance =
                ((f64::from(x) - centre).powi(2) + (f64::from(y) - centre).powi(2)).sqrt();
            // Coverage with a one-pixel soft edge.
            let cover = |radius: f64| (radius + 0.5 - distance).clamp(0.0, 1.0);
            let disc = cover(outer);
            let hole = cover(inner);
            let core = cover(pupil);
            // ring where disc and not hole, dot in the middle, transparent
            // between ring and pupil would look empty: fill it with the dot
            // at lower weight so the icon reads at 16 px.
            let ring_alpha = (disc - hole).max(0.0);
            let fill_alpha = hole * 0.35 + core * 0.65;
            let sum = ring_alpha + fill_alpha;
            let alpha = sum.min(1.0);
            // Premultiplied: weights scaled so the channels never exceed alpha.
            let scale = if sum > 0.0 { (alpha / sum) as f32 } else { 0.0 };
            let channel = |ring_c: f32, dot_c: f32| {
                (ring_c * ring_alpha as f32 + dot_c * fill_alpha as f32) * scale
            };
            let to_byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels.extend_from_slice(&[
                to_byte(channel(ring[2], dot[2])),
                to_byte(channel(ring[1], dot[1])),
                to_byte(channel(ring[0], dot[0])),
                to_byte(alpha as f32),
            ]);
        }
    }
    pixels
}

/// The balloon shown once on the first run (Windows 11 parks new icons in the
/// overflow flyout).
pub const FIRST_RUN_TITLE: &str = "PeekBuddy is running";
/// Body of the first-run balloon.
pub const FIRST_RUN_TEXT: &str = "Windows may keep the icon in the overflow flyout (the ^ arrow). Drag it onto the taskbar to keep it visible.";

#[cfg(test)]
#[expect(
    clippy::indexing_slicing,
    clippy::panic,
    reason = "tests index pixel buffers and panic on a broken menu"
)]
mod tests {
    use super::{
        Command, Health, IconId, MenuItem, TIP_UNITS, Theme, TrayState, WatchEntry, format_rate,
        icon_id, menu, render_icon, tooltip, truncate_units,
    };

    fn state(health: Health, watches: &[(&str, bool)]) -> TrayState {
        TrayState {
            health,
            rate: 0.0,
            watches: watches
                .iter()
                .map(|(label, paused)| WatchEntry {
                    label: (*label).to_owned(),
                    paused: *paused,
                })
                .collect(),
        }
    }

    #[test]
    fn rates_are_short_and_human() {
        for (rate, text) in [
            (0.0, "0 ev/s"),
            (0.3, "0 ev/s"),
            (1.0, "1 ev/s"),
            (850.4, "850 ev/s"),
            (1_234.0, "1.2k ev/s"),
            (99_900.0, "99.9k ev/s"),
            (3_400_000.0, "3.4M ev/s"),
            (f64::NAN, "0 ev/s"),
            (-5.0, "0 ev/s"),
        ] {
            assert_eq!(format_rate(rate), text, "{rate}");
        }
    }

    #[test]
    fn the_tooltip_shows_the_rate_only_while_events_flow() {
        let mut live = state(Health::Live, &[]);
        assert_eq!(tooltip(&live), "PeekBuddy - Live");
        live.rate = 1_234.0;
        assert_eq!(tooltip(&live), "PeekBuddy - Live - 1.2k ev/s");
        let mut paused = state(Health::Paused, &[]);
        paused.rate = 500.0;
        assert_eq!(tooltip(&paused), "PeekBuddy - Paused");
        assert_eq!(tooltip(&state(Health::Gap, &[])), "PeekBuddy - Gap");
        assert_eq!(tooltip(&state(Health::Problem, &[])), "PeekBuddy - Problem");
    }

    #[test]
    fn truncation_fits_the_unit_budget_and_never_splits_a_character() {
        assert_eq!(truncate_units("short", 10), "short");
        let cut = truncate_units("abcdefghij", 6);
        assert_eq!(cut, "abcde\u{2026}");
        assert_eq!(cut.encode_utf16().count(), 6);
        // Four-byte characters take two UTF-16 units.
        let emoji = "\u{1F4C1}".repeat(10);
        let cut = truncate_units(&emoji, 8);
        assert!(
            cut.encode_utf16().count() <= 8,
            "{}",
            cut.encode_utf16().count()
        );
        assert!(cut.ends_with('\u{2026}'));
        assert!(cut.chars().filter(|c| *c == '\u{1F4C1}').count() >= 1);
        // Even the longest tooltip fits szTip with its NUL.
        let long = TrayState {
            health: Health::Live,
            rate: 1.0e12,
            watches: Vec::new(),
        };
        assert!(tooltip(&long).encode_utf16().count() < TIP_UNITS);
        assert_eq!(truncate_units("", 0), "");
    }

    #[test]
    fn the_icon_follows_the_health() {
        assert_eq!(icon_id(&state(Health::Live, &[])), IconId::Live);
        assert_eq!(icon_id(&state(Health::Paused, &[])), IconId::Paused);
        assert_eq!(icon_id(&state(Health::Gap, &[])), IconId::Problem);
        assert_eq!(icon_id(&state(Health::Problem, &[])), IconId::Problem);
    }

    fn labels(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|entry| match entry {
                MenuItem::Item { label, .. } => label.clone(),
                MenuItem::Separator => String::from("-"),
                MenuItem::Submenu { label, .. } => format!("{label} >"),
            })
            .collect()
    }

    #[test]
    fn the_menu_has_the_documented_entries_and_a_default_item() {
        let items = menu(&state(Health::Live, &[("Documents", false)]));
        assert_eq!(
            labels(&items),
            [
                "Open PeekBuddy",
                "-",
                "Pause all",
                "Watches >",
                "-",
                "Open data folder",
                "Settings",
                "-",
                "Quit"
            ]
        );
        assert!(matches!(&items[0], MenuItem::Item { default: true, .. }));
        let MenuItem::Submenu {
            items: watches,
            enabled,
            ..
        } = &items[3]
        else {
            panic!("no submenu");
        };
        assert!(enabled);
        assert!(
            matches!(&watches[0], MenuItem::Item { checked: true, label, .. } if label == "Documents")
        );
    }

    #[test]
    fn pause_all_turns_into_resume_all_and_the_submenu_follows_the_watches() {
        let paused = state(Health::Live, &[("A", true), ("B", true)]);
        assert_eq!(labels(&menu(&paused))[2], "Resume all");
        let mixed = state(Health::Live, &[("A", true), ("B", false)]);
        assert_eq!(labels(&menu(&mixed))[2], "Pause all");
        let none = state(Health::Live, &[]);
        let items = menu(&none);
        assert!(matches!(
            &items[3],
            MenuItem::Submenu { enabled: false, .. }
        ));
        assert_eq!(labels(&menu(&state(Health::Paused, &[])))[2], "Resume all");
    }

    fn ids(items: &[MenuItem], out: &mut Vec<u32>) {
        for entry in items {
            match entry {
                MenuItem::Item { id, .. } => out.push(*id),
                MenuItem::Submenu { items, .. } => ids(items, out),
                MenuItem::Separator => {}
            }
        }
    }

    #[test]
    fn every_menu_id_maps_back_to_its_command() {
        let commands = [
            Command::OpenViewer,
            Command::PauseAll,
            Command::ResumeAll,
            Command::OpenDataFolder,
            Command::OpenSettings,
            Command::Quit,
            Command::ToggleWatch(0),
            Command::ToggleWatch(7),
            Command::ToggleWatch(99),
        ];
        for command in commands {
            assert_eq!(Command::from_id(command.id()), Some(command));
        }
        assert_eq!(Command::from_id(0), None);
        assert_eq!(Command::from_id(5000), None);
        // Every id in a built menu is ours.
        let big = state(Health::Live, &[("w", false); 3]);
        let mut found = Vec::new();
        ids(&menu(&big), &mut found);
        assert_eq!(found.len(), 8);
        assert!(found.iter().all(|id| Command::from_id(*id).is_some()));
    }

    #[test]
    fn icons_are_square_bgra_with_a_visible_dot_in_every_state_and_size() {
        for id in [IconId::Live, IconId::Paused, IconId::Problem] {
            for theme in [Theme::Light, Theme::Dark] {
                for size in super::ICON_SIZES {
                    let pixels = render_icon(id, theme, size);
                    assert_eq!(pixels.len(), (size * size * 4) as usize);
                    let centre = ((size / 2 * size + size / 2) * 4) as usize;
                    assert!(
                        pixels[centre + 3] > 200,
                        "{id:?} {theme:?} {size}: hollow centre"
                    );
                    assert_eq!(pixels[3], 0, "the corner is transparent");
                    // Premultiplied: no channel exceeds alpha.
                    assert!(
                        pixels
                            .chunks(4)
                            .all(|px| px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3])
                    );
                }
            }
        }
        // The states differ from each other.
        assert_ne!(
            render_icon(IconId::Live, Theme::Dark, 16),
            render_icon(IconId::Problem, Theme::Dark, 16)
        );
        assert_ne!(
            render_icon(IconId::Live, Theme::Dark, 16),
            render_icon(IconId::Live, Theme::Light, 16)
        );
    }

    #[derive(Default)]
    struct Recorder {
        calls: Vec<String>,
    }

    impl super::Actions for Recorder {
        fn open_viewer(&mut self) {
            self.calls.push(String::from("viewer"));
        }
        fn open_data_folder(&mut self) {
            self.calls.push(String::from("folder"));
        }
        fn quit(&mut self) {
            self.calls.push(String::from("quit"));
        }
        fn watch_paused(&mut self, index: usize, paused: bool) {
            self.calls.push(format!("watch {index} paused={paused}"));
        }
        fn all_paused(&mut self, paused: bool) {
            self.calls.push(format!("all paused={paused}"));
        }
    }

    #[test]
    fn commands_reach_their_effects_through_one_dispatch() {
        let mut state = state(Health::Live, &[("A", false), ("B", false)]);
        let mut actions = Recorder::default();
        for (command, refresh) in [
            (Command::OpenViewer, false),
            (Command::OpenSettings, false),
            (Command::OpenDataFolder, false),
            (Command::Quit, false),
        ] {
            assert_eq!(super::dispatch(&mut state, command, &mut actions), refresh);
        }
        assert_eq!(actions.calls, ["viewer", "viewer", "folder", "quit"]);
    }

    #[test]
    fn pausing_everything_changes_the_health_and_resuming_restores_it() {
        let mut state = state(Health::Live, &[("A", false), ("B", false)]);
        let mut actions = Recorder::default();
        assert!(super::dispatch(&mut state, Command::PauseAll, &mut actions));
        assert_eq!(state.health, Health::Paused);
        assert!(state.watches.iter().all(|watch| watch.paused));
        assert_eq!(actions.calls, ["all paused=true"]);
        assert_eq!(icon_id(&state), IconId::Paused);

        actions.calls.clear();
        assert!(super::dispatch(
            &mut state,
            Command::ResumeAll,
            &mut actions
        ));
        assert_eq!(state.health, Health::Live);
        assert_eq!(actions.calls, ["all paused=false"]);
    }

    #[test]
    fn toggling_the_last_watch_pauses_the_tray_and_a_problem_stays_visible() {
        let mut state = state(Health::Live, &[("A", true), ("B", false)]);
        let mut actions = Recorder::default();
        assert!(super::dispatch(
            &mut state,
            Command::ToggleWatch(1),
            &mut actions
        ));
        assert_eq!(state.health, Health::Paused);
        assert!(super::dispatch(
            &mut state,
            Command::ToggleWatch(0),
            &mut actions
        ));
        assert_eq!(state.health, Health::Live);
        // A watch that does not exist changes nothing.
        assert!(!super::dispatch(
            &mut state,
            Command::ToggleWatch(9),
            &mut actions
        ));

        let mut problem = self::state(Health::Problem, &[("A", false)]);
        super::dispatch(&mut problem, Command::PauseAll, &mut actions);
        assert_eq!(problem.health, Health::Problem);
    }
}
