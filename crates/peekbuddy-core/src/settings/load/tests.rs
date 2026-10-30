use proptest::prelude::*;

use super::super::migrate::Step;
use super::super::{
    COLUMNS, CURRENT_VERSION, Filters, Settings, Store, Theme, View, Watch, default_toml, to_toml,
};
use super::{Context, Loaded, Problem, load, load_with};

fn ctx() -> Context {
    Context {
        data_dir: String::from(r"C:\Users\Me\AppData\Local\nobuddy\PeekBuddy"),
        export_dir: Some(String::from(r"C:\Users\Me\Exports")),
    }
}

fn problems(text: &str) -> Vec<Problem> {
    match load(text, &ctx()) {
        Loaded::Invalid(problems) => problems,
        other => panic!("expected a problem, got {other:?}"),
    }
}

fn messages(text: &str) -> String {
    problems(text)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_default_file_loads_to_the_default() {
    let Loaded::Ok {
        settings,
        migrated_from,
    } = load(&default_toml(), &ctx())
    else {
        panic!("default did not load");
    };
    // A new file also writes the retention caps of a fresh install.
    let mut expected = Settings::default();
    expected.store.max_age_days = Some(crate::settings::DEFAULT_MAX_AGE_DAYS);
    expected.store.max_total_mb = Some(crate::settings::DEFAULT_MAX_TOTAL_MB);
    assert_eq!(*settings, expected);
    assert_eq!(migrated_from, None);
    assert!(settings.general.first_run);
    assert_eq!(settings.watches, []);
}

#[test]
fn the_v1_fixture_loads_and_normalises_its_paths() {
    let text = include_str!("../../../tests/fixtures/settings/v1.toml");
    let loaded = load(text, &ctx());
    let Loaded::Ok {
        settings,
        migrated_from,
    } = loaded
    else {
        panic!("fixture did not load: {loaded:#?}");
    };
    assert_eq!(migrated_from, None);
    assert_eq!(settings.version, CURRENT_VERSION);
    assert_eq!(settings.general.theme, Theme::Dark);
    assert_eq!(settings.store.max_age_days, Some(30));
    assert_eq!(settings.view.coalesce_ms, 200);
    assert_eq!(settings.watches.len(), 1);
    assert_eq!(settings.watches[0].path, r"\\?\C:\Users\Someone\Documents");
}

#[test]
fn invalid_inputs_get_positioned_helpful_messages() {
    let cases: &[(&str, &str)] = &[
        ("syntax", "version = 1\n[general\n"),
        ("no version", "[general]\nautostart = true\n"),
        ("unknown key", "version = 1\n[general]\nautostrat = true\n"),
        (
            "unknown section",
            "version = 1\n[genral]\ntheme = \"dark\"\n",
        ),
        (
            "wrong type",
            "version = 1\n[general]\nautostart = \"yes\"\n",
        ),
        ("bad theme", "version = 1\n[general]\ntheme = \"blue\"\n"),
        (
            "bad glob",
            "version = 1\n[filters]\nexclude_globs = [\"[abc\"]\n",
        ),
        (
            "bad regex",
            "version = 1\n[filters]\nexclude_regex = [\"(unclosed\"]\n",
        ),
        (
            "unknown preset",
            "version = 1\n[filters]\npresets = [\"temp-lock-swop\"]\n",
        ),
        ("coalesce range", "version = 1\n[view]\ncoalesce_ms = 5\n"),
        (
            "unknown column",
            "version = 1\n[view]\ncolumns = [\"time\", \"pth\"]\n",
        ),
        (
            "relative path",
            "version = 1\n[[watch]]\nid = \"a\"\npath = 'Documents\\Work'\n",
        ),
        (
            "data dir as root",
            "version = 1\n[[watch]]\nid = \"a\"\npath = 'C:\\Users\\Me\\AppData\\Local\\nobuddy\\PeekBuddy\\segments'\n",
        ),
        (
            "export dir as root",
            "version = 1\n[[watch]]\nid = \"a\"\npath = 'C:\\Users\\Me\\Exports'\n",
        ),
        (
            "unc buffer",
            "version = 1\n[[watch]]\nid = \"a\"\npath = '\\\\server\\share'\nbuffer_kib = 128\n",
        ),
        (
            "duplicate id and root",
            "version = 1\n[[watch]]\nid = \"a\"\npath = 'C:\\x'\n[[watch]]\nid = \"a\"\npath = 'c:\\X\\'\n",
        ),
        (
            "bad id",
            "version = 1\n[[watch]]\nid = \"My Docs\"\npath = 'C:\\x'\n",
        ),
    ];
    let mut report = String::new();
    for (name, text) in cases {
        report.push_str(&format!("# {name}\n{}\n\n", messages(text)));
    }
    insta::assert_snapshot!(report);
}

#[test]
fn a_newer_file_loads_leniently_and_says_so() {
    let text = "version = 7\n[general]\ntheme = \"dark\"\nfuture_key = 1\n[hologram]\nx = 1\n";
    match load(text, &ctx()) {
        Loaded::TooNew { version, settings } => {
            assert_eq!(version, 7);
            assert_eq!(settings.general.theme, Theme::Dark);
        }
        other => panic!("{other:?}"),
    }
    // Even a newer file that cannot be understood is not an error.
    assert!(matches!(
        load("version = 9\n[general]\ntheme = 12\n", &ctx()),
        Loaded::TooNew { version: 9, .. }
    ));
}

fn rename_theme(mut table: toml::Table) -> Result<toml::Table, super::super::MigrationError> {
    if let Some(toml::Value::Table(general)) = table.get_mut("general")
        && let Some(value) = general.remove("appearance")
    {
        general.insert(String::from("theme"), value);
    }
    Ok(table)
}

#[test]
fn an_older_file_is_migrated_with_the_steps_of_the_build() {
    // The harness for the day version 2 exists: a version 1 file migrates
    // with the build's steps, then validation compares with the build's own
    // version, which is still 1 here, so it reports exactly that.
    let steps: &[Step] = &[rename_theme];
    let text = "version = 1\n[general]\nappearance = \"light\"\n";
    match load_with(text, &ctx(), 2, steps) {
        Loaded::Invalid(problems) => {
            assert!(
                problems.iter().any(|p| p.key_path == "version"),
                "{problems:?}"
            );
            assert!(
                !problems.iter().any(|p| p.message.contains("unknown key")),
                "the step did not run: {problems:?}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_committed_version_fixture_migrates_to_current_and_validates() {
    // One entry per committed version; add `v2` next to `v1` with version 2.
    let fixtures = [include_str!("../../../tests/fixtures/settings/v1.toml")];
    for (index, text) in fixtures.iter().enumerate() {
        match load(text, &ctx()) {
            Loaded::Ok { settings, .. } => {
                assert_eq!(settings.version, CURRENT_VERSION, "v{}", index + 1);
            }
            other => panic!("v{}: {other:?}", index + 1),
        }
    }
}

#[test]
fn problems_are_listed_together() {
    let text = "version = 1\n[view]\ncoalesce_ms = 5\ncolumns = [\"nope\"]\n[filters]\nexclude_globs = [\"[x\"]\n";
    assert!(problems(text).len() >= 3);
}

fn arb_settings() -> impl Strategy<Value = Settings> {
    let presets = [
        "browser-caches",
        "build-dirs",
        "git-internals",
        "temp-lock-swap",
    ];
    let watch = (
        any::<bool>(),
        any::<bool>(),
        proptest::sample::subsequence(presets.to_vec(), 0..=3),
        proptest::option::of(4u32..=64),
        any::<bool>(),
        proptest::collection::vec("[a-z]{1,6}", 0..3),
    );
    (
        (any::<bool>(), any::<bool>(), 0..3u8),
        (
            proptest::option::of(1u32..=36_500),
            proptest::option::of(1u64..=1_000_000),
            proptest::option::of(1u64..=1_000_000_000),
        ),
        (
            50u32..=500,
            proptest::sample::subsequence(COLUMNS.to_vec(), 1..=COLUMNS.len()),
        ),
        (
            proptest::collection::vec("[a-z]{1,6}", 0..3),
            proptest::sample::subsequence(presets.to_vec(), 0..=4),
        ),
        proptest::collection::vec(watch, 0..4),
    )
        .prop_map(|(general, store, view, filters, watches)| {
            let mut settings = Settings::default();
            settings.general.autostart = general.0;
            settings.general.first_run = general.1;
            settings.general.theme =
                [Theme::System, Theme::Light, Theme::Dark][usize::from(general.2)];
            settings.store = Store {
                max_age_days: store.0,
                max_total_mb: store.1,
                max_rows: store.2,
            };
            settings.view = View {
                coalesce_ms: view.0,
                columns: view.1.into_iter().map(String::from).collect(),
            };
            settings.filters = Filters {
                exclude_globs: filters.0.iter().map(|g| format!("*.{g}")).collect(),
                exclude_regex: vec![],
                presets: filters.1.into_iter().map(String::from).collect(),
            };
            settings.watches = watches
                .into_iter()
                .enumerate()
                .map(|(i, w)| Watch {
                    id: format!("w{i}"),
                    path: format!(r"\\?\C:\watched\p{i}"),
                    recursive: w.0,
                    enabled: w.1,
                    presets: w.2.into_iter().map(String::from).collect(),
                    buffer_kib: w.3,
                    follow_link_target: w.4,
                    exclude_globs: w.5.iter().map(|g| format!("*.{g}")).collect(),
                })
                .collect();
            settings
        })
}

proptest! {
    #[test]
    fn serialising_then_loading_gives_the_same_settings(settings in arb_settings()) {
        let text = to_toml(&settings).unwrap();
        match load(&text, &ctx()) {
            Loaded::Ok { settings: loaded, .. } => prop_assert_eq!(*loaded, settings),
            other => prop_assert!(false, "{:?}\n{}", other, text),
        }
    }

    #[test]
    fn arbitrary_text_never_panics(text in "\\PC{0,400}") {
        let _ = load(&text, &ctx());
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        let _ = load(&String::from_utf8_lossy(&bytes), &ctx());
    }
}
