use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use peekbuddy_core::settings::{Settings, Theme, to_toml};

use super::{About, SettingsAction, SettingsEdit, SettingsScreen, megabytes};

type State = (SettingsScreen, Vec<SettingsAction>);

fn about() -> About {
    About {
        version: String::from("1.2.3"),
        git_sha: String::from("abc1234"),
        target: String::from("x64"),
        adapter: None,
    }
}

fn harness(settings: &Settings) -> Harness<'static, State> {
    let mut screen = SettingsScreen::new(
        String::from(r"C:\Users\me\AppData\Local\nobuddy\PeekBuddy"),
        about(),
    );
    screen.load(settings);
    Harness::builder()
        .with_size(egui::vec2(700.0, 1400.0))
        .build_ui_state(
            |ui, (screen, taken): &mut State| {
                taken.extend(screen.show(ui));
            },
            (screen, Vec::new()),
        )
}

fn focus_and_type(harness: &mut Harness<'static, State>, label: &str, text: &str) {
    harness.get_by_label(label).focus();
    harness.run();
    harness.get_by_label(label).type_text(text);
    harness.run();
}

#[test]
fn the_form_shows_what_is_saved_and_saving_without_a_change_sends_the_same_values() {
    let mut settings = Settings::default();
    settings.store.max_age_days = Some(30);
    settings.store.max_total_mb = Some(2048);
    settings.view.coalesce_ms = 200;
    settings.general.theme = Theme::Dark;
    let mut harness = harness(&settings);
    harness.run();
    assert_eq!(
        harness
            .get_by_label("Keep at most (days)")
            .value()
            .as_deref(),
        Some("30")
    );
    assert_eq!(
        harness
            .get_by_label("Keep at most (rows)")
            .value()
            .as_deref(),
        Some("")
    );
    harness.get_by_label("Save").click();
    harness.run();
    let [SettingsAction::Apply(edit)] = harness.state().1.as_slice() else {
        unreachable!("one apply is sent, got {:?}", harness.state().1);
    };
    let mut back = settings.clone();
    edit.apply_to(&mut back);
    assert_eq!(back, settings, "an untouched form changes nothing");
}

#[test]
fn every_setting_round_trips_into_the_file() {
    let edit = SettingsEdit {
        autostart: true,
        theme: Theme::Light,
        coalesce_ms: 300,
        max_age_days: Some(90),
        max_total_mb: None,
        max_rows: Some(5_000_000),
        presets: vec![String::from("build-dirs"), String::from("temp-lock-swap")],
    };
    let mut settings = Settings::default();
    edit.apply_to(&mut settings);
    let text = to_toml(&settings).unwrap();
    insta::assert_snapshot!(text);
    // The engine's loader reads the same values back.
    let loaded =
        peekbuddy_core::settings::load(&text, &peekbuddy_core::settings::Context::default());
    let peekbuddy_core::settings::Loaded::Ok { settings: back, .. } = loaded else {
        unreachable!("the written file loads");
    };
    assert!(back.general.autostart);
    assert_eq!(back.general.theme, Theme::Light);
    assert_eq!(back.view.coalesce_ms, 300);
    assert_eq!(back.store.max_age_days, Some(90));
    assert_eq!(back.store.max_total_mb, None);
    assert_eq!(back.store.max_rows, Some(5_000_000));
    assert_eq!(back.filters.presets, edit.presets);
}

#[test]
fn typed_values_are_sent_and_empty_caps_mean_no_cap() {
    let mut harness = harness(&Settings::default());
    harness.run();
    focus_and_type(&mut harness, "Keep at most (days)", "14");
    focus_and_type(&mut harness, "Keep at most (MB)", "500");
    harness.get_by_label("Save").click();
    harness.run();
    let [SettingsAction::Apply(edit)] = harness.state().1.as_slice() else {
        unreachable!("one apply is sent");
    };
    assert_eq!(edit.max_age_days, Some(14));
    assert_eq!(edit.max_total_mb, Some(500));
    assert_eq!(edit.max_rows, None);
    assert!(!edit.autostart, "autostart stays off unless it is ticked");
}

#[test]
fn text_that_is_not_a_number_is_refused_next_to_its_field_and_nothing_is_sent() {
    let mut harness = harness(&Settings::default());
    harness.run();
    focus_and_type(&mut harness, "Keep at most (days)", "soon");
    harness.get_by_label("Save").click();
    harness.run();
    harness.get_by_label("enter a whole number");
    harness.get_by_label("Nothing was changed.");
    assert_eq!(harness.state().1, []);
}

#[test]
fn the_engines_refusal_is_shown_next_to_the_field_and_the_form_stays_as_typed() {
    let mut harness = harness(&Settings::default());
    harness.run();
    focus_and_type(
        &mut harness,
        "Group rapid events of one file within (ms)",
        "0",
    );
    harness.state_mut().0.on_result(
        false,
        "9:1: view.coalesce_ms: the coalescing window must be 50 to 500 ms\n12:1: store.max_age_days: retention must be at least 1 day",
        None,
    );
    harness.run();
    harness.get_by_label_contains("view.coalesce_ms: the coalescing window must be 50 to 500 ms");
    harness.get_by_label("Not saved.");
    harness.get_by_label("Nothing was changed.");
    assert_eq!(
        harness
            .get_by_label("Group rapid events of one file within (ms)")
            .value()
            .as_deref(),
        Some("1500")
    );
}

#[test]
fn a_saved_answer_reloads_the_form_from_the_saved_settings() {
    let mut harness = harness(&Settings::default());
    harness.run();
    let mut saved = Settings::default();
    saved.store.max_rows = Some(10);
    harness.state_mut().0.on_result(true, "saved", Some(&saved));
    harness.run();
    harness.get_by_label("Saved.");
    assert_eq!(
        harness
            .get_by_label("Keep at most (rows)")
            .value()
            .as_deref(),
        Some("10")
    );
}

#[test]
fn presets_are_labelled_as_hidden_in_view_and_recorded() {
    let mut harness = harness(&Settings::default());
    harness.run();
    harness.get_by_label_contains("Git internals - hidden in view (recorded)");
    harness.get_by_label_contains("Cloud placeholder churn - hidden in view (recorded)");
    harness
        .get_by_label_contains("Git internals - hidden in view (recorded)")
        .click();
    harness.get_by_label("Save").click();
    harness.run();
    let [SettingsAction::Apply(edit)] = harness.state().1.as_slice() else {
        unreachable!("one apply is sent");
    };
    assert_eq!(edit.presets, ["git-internals"]);
}

#[test]
fn the_screen_says_where_the_data_is_what_it_uses_and_what_the_build_is() {
    let mut harness = harness(&Settings::default());
    harness.state_mut().0.set_usage(1_234_567_890);
    harness.run();
    harness.get_by_label_contains("The log uses 1234.5 MB on disk now.");
    harness.get_by_label(r"C:\Users\me\AppData\Local\nobuddy\PeekBuddy");
    harness.get_by_label_contains("Commit abc1234 - x64 - renderer not started yet");
    harness.get_by_label("1.2.3");
    harness
        .state_mut()
        .0
        .set_adapter(String::from("WARP / dx12"));
    harness.run();
    harness.get_by_label_contains("Commit abc1234 - x64 - WARP / dx12");
    harness.get_by_label_contains("MIT licence");
    harness.get_by_label_contains("THIRD_PARTY_LICENSES.html");
    harness.get_by_label_contains("never leaves your disk");
    harness.get_by_label("Open folder").click();
    harness
        .get_by_label("Show the welcome screens again")
        .click();
    harness.run();
    assert_eq!(
        harness.state().1,
        [
            SettingsAction::OpenDataFolder,
            SettingsAction::ShowOnboarding
        ]
    );
}

#[test]
fn diagnostics_are_previewed_before_anything_can_be_copied() {
    let mut harness = harness(&Settings::default());
    harness.run();
    harness.get_by_label("Copy diagnostics...").click();
    harness.run();
    assert_eq!(harness.state().1, [SettingsAction::ShowDiagnostics]);
    harness.get_by_label("Collecting diagnostics...");
    harness
        .state_mut()
        .0
        .set_diagnostics(Some(String::from("watch C:\\secret\\folder")));
    harness.run();
    harness.get_by_label("Diagnostics");
    harness.get_by_label_contains("contains the folders you watch");
    harness.get_by_label_contains("C:\\secret\\folder");
    harness.get_by_label("Copy to clipboard");
    harness.get_by_label("Close").click();
    harness.run();
    assert!(harness.query_by_label("Copy to clipboard").is_none());
    harness.get_by_label("Copy diagnostics...");
}

#[test]
fn megabytes_are_decimal_with_one_digit() {
    assert_eq!(megabytes(0), "0.0 MB");
    assert_eq!(megabytes(99_999), "0.0 MB");
    assert_eq!(megabytes(1_500_000), "1.5 MB");
    assert_eq!(megabytes(2_048_000_000), "2048.0 MB");
}

#[test]
fn every_control_of_the_settings_screen_and_the_diagnostics_preview_has_a_name() {
    let mut harness = harness(&Settings::default());
    harness.run();
    crate::a11y::assert_every_control_is_named(&harness);
    harness
        .state_mut()
        .0
        .set_diagnostics(Some(String::from("text")));
    harness.run();
    crate::a11y::assert_every_control_is_named(&harness);
}
