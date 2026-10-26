//! The tray on screen (#60): connects the pure model to the Win32 icon.

use std::path::PathBuf;

use peekbuddy_win::tray::{Entry, Icon, Menu, Tray, taskbar_uses_light_theme};
use peekbuddy_win::window::{WindowEvent, WindowHandle};

use crate::tray_model::{
    Actions, Command, FIRST_RUN_TEXT, FIRST_RUN_TITLE, ICON_SIZES, IconId, MenuItem, Theme,
    TrayState, dispatch, icon_id, menu, render_icon, tooltip,
};

/// Real effects of the tray commands.
struct RealActions {
    window: WindowHandle,
    exe: PathBuf,
    data_root: PathBuf,
}

/// Explorer does not take `\\?\` paths.
fn plain(path: &std::path::Path) -> PathBuf {
    let text = path.to_string_lossy();
    PathBuf::from(text.strip_prefix(r"\\?\UNC\").map_or_else(
        || text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned(),
        |rest| format!(r"\\{rest}"),
    ))
}

impl Actions for RealActions {
    fn open_viewer(&mut self) {
        // The viewer's own mutex keeps a second one from opening; focusing
        // the running one goes through the pipe (#61).
        let _ = std::process::Command::new(&self.exe)
            .arg("--viewer")
            .spawn();
    }

    fn open_data_folder(&mut self) {
        let _ = std::process::Command::new("explorer.exe")
            .arg(plain(&self.data_root))
            .spawn();
    }

    fn quit(&mut self) {
        let _ = self.window.post_close();
    }

    fn watch_paused(&mut self, _index: usize, _paused: bool) {
        // Applying a pause to the capture layer and persisting `enabled` is
        // #63; the tray shows the state it was asked for.
    }
}

fn to_entry(item: &MenuItem) -> Entry {
    match item {
        MenuItem::Item {
            id,
            label,
            default,
            checked,
            enabled,
        } => Entry::Item {
            id: *id,
            label: label.clone(),
            default: *default,
            checked: *checked,
            enabled: *enabled,
        },
        MenuItem::Separator => Entry::Separator,
        MenuItem::Submenu {
            label,
            items,
            enabled,
        } => Entry::Submenu {
            label: label.clone(),
            items: items.iter().map(to_entry).collect(),
            enabled: *enabled,
        },
    }
}

/// Where the shutdown path finds the controller.
pub type SharedTray = std::rc::Rc<std::cell::RefCell<Option<TrayController>>>;

/// The tray icon of the engine.
pub struct TrayController {
    tray: Tray,
    actions: RealActions,
    /// What the tray shows.
    pub state: TrayState,
    theme: Theme,
    shown: Option<(IconId, Theme)>,
    icon: Option<Icon>,
    present: bool,
    /// How often the icon was added again after the taskbar was recreated.
    pub re_adds: u32,
}

impl TrayController {
    /// Creates the controller for the engine window.
    #[must_use]
    pub fn new(window: WindowHandle, exe: PathBuf, data_root: PathBuf, state: TrayState) -> Self {
        Self {
            tray: Tray::new(window),
            actions: RealActions {
                window,
                exe,
                data_root,
            },
            state,
            theme: current_theme(),
            shown: None,
            icon: None,
            present: false,
            re_adds: 0,
        }
    }

    fn icon_for(&mut self) -> std::io::Result<()> {
        let wanted = (icon_id(&self.state), self.theme);
        if self.shown != Some(wanted) || self.icon.is_none() {
            // The 16 px frame is the system small icon at 100 % scaling; the
            // shell scales the larger ones in for high DPI.
            let size = ICON_SIZES.get(2).copied().unwrap_or(24);
            let icon = Icon::from_bgra(size, &render_icon(wanted.0, wanted.1, size))?;
            self.icon = Some(icon);
            self.shown = Some(wanted);
        }
        Ok(())
    }

    /// Adds the icon. A failure is not fatal (no explorer yet at logon, a
    /// taskbar-less session): the engine keeps capturing and `TaskbarCreated`
    /// retries. Returns whether the icon is shown.
    pub fn show(&mut self) -> bool {
        self.present = self.try_add().is_ok();
        self.present
    }

    fn try_add(&mut self) -> std::io::Result<()> {
        self.icon_for()?;
        match &self.icon {
            Some(icon) => self.tray.add(icon, &tooltip(&self.state)),
            None => Err(std::io::Error::other("no icon")),
        }
    }

    /// Shows the one-time hint about Windows 11's overflow flyout.
    pub fn first_run_balloon(&self) {
        if self.present {
            let _ = self.tray.balloon(FIRST_RUN_TITLE, FIRST_RUN_TEXT);
        }
    }

    /// Removes the icon (the last shutdown step but one).
    pub fn remove(&mut self) {
        self.tray.remove();
        self.present = false;
    }

    /// Whether the icon is currently shown.
    #[must_use]
    pub const fn is_present(&self) -> bool {
        self.present
    }

    fn refresh(&mut self) {
        if !self.present {
            return;
        }
        if self.icon_for().is_ok()
            && let Some(icon) = &self.icon
        {
            let _ = self.tray.update(icon, &tooltip(&self.state));
        }
    }

    fn run(&mut self, command: Command) {
        if dispatch(&mut self.state, command, &mut self.actions) {
            self.refresh();
        }
    }

    fn show_menu(&mut self, window: WindowHandle) {
        let entries: Vec<Entry> = menu(&self.state).iter().map(to_entry).collect();
        let Ok(popup) = Menu::build(&entries) else {
            return;
        };
        if let Some(command) = popup
            .track(window, Some(Command::OpenViewer.id()))
            .and_then(Command::from_id)
        {
            self.run(command);
        }
    }

    /// Handles a window event that concerns the tray. Returns whether it did.
    pub fn handle(&mut self, event: WindowEvent, window: WindowHandle) -> bool {
        match event {
            WindowEvent::TraySelect => self.run(Command::OpenViewer),
            WindowEvent::TrayMenu => self.show_menu(window),
            WindowEvent::Command(id) => {
                if let Some(command) = Command::from_id(id) {
                    self.run(command);
                }
            }
            WindowEvent::TaskbarCreated => {
                self.present = self.try_add().is_ok();
                if self.present {
                    self.re_adds += 1;
                }
            }
            WindowEvent::ThemeChanged => {
                self.theme = current_theme();
                self.refresh();
            }
            _ => return false,
        }
        true
    }
}

fn current_theme() -> Theme {
    if taskbar_uses_light_theme() {
        Theme::Light
    } else {
        Theme::Dark
    }
}

/// The last two path components of a watch root, for the menu.
#[must_use]
pub fn label_for(path: &str) -> String {
    let bare = path
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| path.strip_prefix(r"\\?\"))
        .unwrap_or(path);
    let parts: Vec<&str> = bare.split('\\').filter(|part| !part.is_empty()).collect();
    let tail = parts.len().saturating_sub(2);
    parts
        .get(tail..)
        .map_or_else(|| path.to_owned(), |parts| parts.join("\\"))
}

#[cfg(test)]
mod tests {
    use super::{label_for, plain};
    use std::path::Path;

    #[test]
    fn labels_are_the_last_two_components() {
        assert_eq!(label_for(r"\\?\C:\Users\Me\Documents"), r"Me\Documents");
        assert_eq!(label_for(r"\\?\C:\"), r"C:");
        assert_eq!(label_for(r"\\?\UNC\server\share"), r"server\share");
    }

    #[test]
    fn explorer_gets_plain_paths() {
        assert_eq!(
            plain(Path::new(r"\\?\C:\Users\Me")),
            Path::new(r"C:\Users\Me")
        );
        assert_eq!(
            plain(Path::new(r"\\?\UNC\srv\share\x")),
            Path::new(r"\\srv\share\x")
        );
        assert_eq!(plain(Path::new(r"C:\plain")), Path::new(r"C:\plain"));
    }
}
