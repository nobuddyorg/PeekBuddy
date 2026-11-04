//! Light and dark (#65).
//!
//! The viewer follows the system theme (`ThemePreference::System`) unless
//! `settings.toml` says otherwise (`[general] theme`, edited in #70). egui
//! resolves "system" itself and switches live when Windows does; the title
//! bar follows through the app module.

use peekbuddy_core::settings::Theme;

/// The egui preference for the theme in the settings.
#[must_use]
pub const fn preference(theme: Theme) -> egui::ThemePreference {
    match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    }
}

#[cfg(test)]
mod tests {
    use peekbuddy_core::settings::Theme;

    use super::preference;

    #[test]
    fn each_setting_maps_to_its_own_preference() {
        assert_eq!(preference(Theme::System), egui::ThemePreference::System);
        assert_eq!(preference(Theme::Light), egui::ThemePreference::Light);
        assert_eq!(preference(Theme::Dark), egui::ThemePreference::Dark);
    }
}
