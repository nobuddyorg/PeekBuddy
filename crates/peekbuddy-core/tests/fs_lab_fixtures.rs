//! The fs-lab fixtures (#36, report 0006): raw RDCW and USN records captured
//! on hosted Windows runners, stored as JSON lines with names as u16 arrays.
//! The tests pin the signatures the classifier (#49) and the name handling
//! (#39) rely on; they run on Linux without Win32.

use serde_json::Value;

const FIXTURES: &[(&str, &str)] = &[
    ("L1-c-x64", include_str!("fixtures/fs-lab/L1-c-x64.jsonl")),
    ("L1-ntfs-x64", include_str!("fixtures/fs-lab/L1-ntfs-x64.jsonl")),
    ("L2-ntfs-x64", include_str!("fixtures/fs-lab/L2-ntfs-x64.jsonl")),
    ("L3-c-x64", include_str!("fixtures/fs-lab/L3-c-x64.jsonl")),
    ("L3-ntfs-arm", include_str!("fixtures/fs-lab/L3-ntfs-arm.jsonl")),
    ("L3-ntfs-x64", include_str!("fixtures/fs-lab/L3-ntfs-x64.jsonl")),
    ("N1-ntfs-x64", include_str!("fixtures/fs-lab/N1-ntfs-x64.jsonl")),
    ("N2-c-x64", include_str!("fixtures/fs-lab/N2-c-x64.jsonl")),
    ("N2-ntfs-arm", include_str!("fixtures/fs-lab/N2-ntfs-arm.jsonl")),
    ("N2-ntfs-x64", include_str!("fixtures/fs-lab/N2-ntfs-x64.jsonl")),
    ("N3-ntfs-x64", include_str!("fixtures/fs-lab/N3-ntfs-x64.jsonl")),
    ("N4-c-x64", include_str!("fixtures/fs-lab/N4-c-x64.jsonl")),
    ("N4-ntfs-arm", include_str!("fixtures/fs-lab/N4-ntfs-arm.jsonl")),
    ("N4-ntfs-x64", include_str!("fixtures/fs-lab/N4-ntfs-x64.jsonl")),
    ("S1-c-x64", include_str!("fixtures/fs-lab/S1-c-x64.jsonl")),
    ("S1-ntfs-x64", include_str!("fixtures/fs-lab/S1-ntfs-x64.jsonl")),
];

const USN_RENAME_OLD: u64 = 0x1000;
const USN_RENAME_NEW: u64 = 0x2000;
const USN_STREAM_CHANGE: u64 = 0x20_0000;
const USN_HARD_LINK_CHANGE: u64 = 0x1_0000;
const USN_REPARSE_POINT_CHANGE: u64 = 0x10_0000;

struct Fixture {
    header: Value,
    rows: Vec<Value>,
}

fn load(id: &str) -> Fixture {
    let text = FIXTURES.iter().find(|(name, _)| *name == id).map(|(_, t)| *t).expect("known fixture");
    let mut lines = text.lines().map(|l| serde_json::from_str::<Value>(l).expect("a JSON line"));
    let header = lines.next().expect("a header");
    Fixture { header, rows: lines.collect() }
}

fn units(row: &Value) -> Vec<u16> {
    row.get("name_u16")
        .and_then(Value::as_array)
        .expect("name_u16")
        .iter()
        .map(|u| u16::try_from(u.as_u64().expect("a number")).expect("a UTF-16 unit"))
        .collect()
}

fn rdcw_name(row: &Value) -> String {
    String::from_utf16_lossy(&units(row))
}

fn str_of<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn num(row: &Value, key: &str) -> u64 {
    row.get(key).and_then(Value::as_u64).unwrap_or_default()
}

/// Rows between the operation `label` and the next operation.
fn after<'a>(f: &'a Fixture, label: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    let mut inside = false;
    for row in &f.rows {
        if str_of(row, "src") == "op" {
            inside = str_of(row, "label") == label;
            continue;
        }
        if inside {
            out.push(row);
        }
    }
    out
}

fn actions(rows: &[&Value]) -> Vec<String> {
    rows.iter().filter(|r| str_of(r, "src") == "rdcw").map(|r| str_of(r, "action").to_string()).collect()
}

#[test]
fn every_fixture_is_well_formed() {
    for (id, _) in FIXTURES {
        let f = load(id);
        assert_eq!(str_of(&f.header, "src"), "header", "{id}");
        assert!(num(&f.header, "os_build") >= 26_100, "{id}");
        assert_eq!(str_of(&f.header, "rdcw_class"), "Full", "{id}");
        let (first, last) = (f.rows.first().expect("rows"), f.rows.last().expect("rows"));
        assert_eq!(str_of(first, "label"), "scenario start", "{id}");
        assert!(f.rows.iter().any(|r| str_of(r, "src") == "op" && str_of(r, "label") == "scenario end"), "{id}");
        let _ = last;
        for row in f.rows.iter().filter(|r| str_of(r, "src") == "rdcw") {
            assert!(!units(row).is_empty(), "{id}: empty name");
            assert!(row.get("file_id").is_some() && row.get("name_flags").is_some(), "{id}: Full class fields");
        }
    }
}

#[test]
fn a_case_only_rename_is_removed_then_old_then_new() {
    for id in ["N2-ntfs-x64", "N2-ntfs-arm", "N2-c-x64"] {
        let f = load(id);
        let rows = after(&f, "case-only rename a.txt -> A.txt");
        assert_eq!(actions(&rows), ["Removed", "RenamedOldName", "RenamedNewName"], "{id}");
        let rdcw: Vec<String> = rows.iter().filter(|r| str_of(r, "src") == "rdcw").map(|r| rdcw_name(r)).collect();
        assert_eq!(rdcw, ["a.txt", "a.txt", "A.txt"], "{id}");
        // Only the rename records carry name flags 0x80; the file id is the same throughout.
        let ids: Vec<u64> = rows.iter().filter(|r| str_of(r, "src") == "rdcw").map(|r| num(r, "file_id")).collect();
        assert!(ids.windows(2).all(|w| w[0] == w[1]), "{id}");
    }
}

#[test]
fn usn_pairs_a_rename_by_frn_old_then_new() {
    let f = load("N2-c-x64");
    let usn: Vec<&Value> = f.rows.iter().filter(|r| str_of(r, "src") == "usn").collect();
    let old = usn.iter().position(|r| num(r, "reason") & USN_RENAME_OLD != 0).expect("a RENAME_OLD_NAME record");
    let new = usn.iter().position(|r| num(r, "reason") & USN_RENAME_NEW != 0).expect("a RENAME_NEW_NAME record");
    assert!(old < new);
    assert_eq!(str_of(usn.get(old).expect("old"), "frn"), str_of(usn.get(new).expect("new"), "frn"));
    assert_eq!(str_of(usn.get(old).expect("old"), "name"), "a.txt");
    assert_eq!(str_of(usn.get(new).expect("new"), "name"), "A.txt");
}

#[test]
fn the_short_name_is_reported_with_flag_two_and_renames_with_0x80() {
    let f = load("N1-ntfs-x64");
    let write = after(&f, "write through the short name");
    assert!(write.iter().all(|r| rdcw_name(r) == "ALONGF~1.TXT" && num(r, "name_flags") == 2));
    let rename = after(&f, "rename through the short name");
    assert_eq!(actions(&rename), ["RenamedOldName", "RenamedNewName"]);
    assert!(rename.iter().all(|r| num(r, "name_flags") == 0x80));
    // The first create is reported under the long name with the NTFS flag.
    let create = after(&f, "create long name");
    assert!(create.iter().all(|r| num(r, "name_flags") == 1));
}

#[test]
fn a_write_through_a_hard_link_reports_the_path_used_while_usn_reports_the_first_name() {
    let f = load("L1-ntfs-x64");
    let rows = after(&f, "write through the link");
    assert!(rows.iter().all(|r| rdcw_name(r) == "link.txt"));
    let linked = after(&f, "create hard link");
    assert_eq!(actions(&linked), ["Added"]);
    assert_eq!(num(linked.first().expect("one record"), "name_flags"), 0x80);

    let u = load("L1-c-x64");
    let usn_write: Vec<&Value> = after(&u, "write through the link").into_iter().filter(|r| str_of(r, "src") == "usn").collect();
    assert!(!usn_write.is_empty() && usn_write.iter().all(|r| str_of(r, "name") == "orig.txt"));
    let usn_link = after(&u, "create hard link");
    let usn_link: Vec<&&Value> = usn_link.iter().filter(|r| str_of(r, "src") == "usn").collect();
    assert!(usn_link.iter().any(|r| num(r, "reason") & USN_HARD_LINK_CHANGE != 0 && str_of(r, "name") == "link.txt"));
}

#[test]
fn reparse_points_are_first_reported_as_plain_added_then_modified_with_the_tag() {
    for (id, label, tag) in [
        ("L2-ntfs-x64", "create junction", 0xA000_0003_u64),
        ("L3-ntfs-x64", "create file symlink", 0xA000_000C),
        ("L3-ntfs-arm", "create file symlink", 0xA000_000C),
    ] {
        let f = load(id);
        let rows = after(&f, label);
        assert_eq!(actions(&rows), ["Added", "Modified"], "{id}");
        assert_eq!(num(rows.first().expect("added"), "reparse_tag"), 0, "{id}");
        assert_eq!(num(rows.get(1).expect("modified"), "reparse_tag"), tag, "{id}");
    }
    let f = load("L3-c-x64");
    let rows = after(&f, "create file symlink");
    assert!(rows.iter().any(|r| str_of(r, "src") == "usn" && num(r, "reason") & USN_REPARSE_POINT_CHANGE != 0));
}

#[test]
fn a_stream_write_is_a_modified_record_for_the_file_without_the_stream_name() {
    let f = load("S1-ntfs-x64");
    let rows = after(&f, "create an alternate data stream");
    assert_eq!(actions(&rows), ["Modified"]);
    assert_eq!(rdcw_name(rows.first().expect("one record")), "file.txt");
    assert_eq!(num(rows.first().expect("one record"), "file_size"), 4);
    // Deleting the stream produces no RDCW record at all.
    assert!(after(&f, "delete the stream (file stays)").is_empty());

    let u = load("S1-c-x64");
    let stream_usn = u.rows.iter().filter(|r| str_of(r, "src") == "usn" && num(r, "reason") & USN_STREAM_CHANGE != 0).count();
    assert!(stream_usn >= 3, "STREAM_CHANGE records for create, append and delete");
}

#[test]
fn lone_surrogates_survive_in_rdcw_names() {
    for id in ["N4-ntfs-x64", "N4-ntfs-arm", "N4-c-x64"] {
        let f = load(id);
        let all: Vec<u16> = f.rows.iter().filter(|r| str_of(r, "src") == "rdcw").flat_map(units).collect();
        assert!(all.contains(&0xD800), "{id}: lone high surrogate");
        assert!(all.contains(&0xDC00), "{id}: lone low surrogate");
        for row in f.rows.iter().filter(|r| str_of(r, "src") == "rdcw" && str_of(r, "action") == "Added") {
            let u = units(row);
            if u.contains(&0xD800) || u.contains(&0xDC00) {
                // Not valid UTF-16: a lossy conversion would change the name.
                assert!(String::from_utf16(&u).is_err(), "{id}");
            }
        }
    }
}

#[test]
fn per_directory_case_sensitivity_gives_two_names_differing_by_case() {
    let f = load("N3-ntfs-x64");
    let names: Vec<String> = f.rows.iter().filter(|r| str_of(r, "src") == "rdcw" && str_of(r, "action") == "Added").map(rdcw_name).collect();
    assert!(names.contains(&"cs\\file.txt".to_string()) && names.contains(&"cs\\FILE.txt".to_string()), "{names:?}");
}

#[test]
fn arm64_and_x64_agree_on_the_rdcw_action_sequence() {
    for scenario in ["N2", "N4", "L3"] {
        let seq = |arch: &str| -> Vec<(String, String)> {
            load(&format!("{scenario}-ntfs-{arch}"))
                .rows
                .iter()
                .filter(|r| str_of(r, "src") == "rdcw")
                .map(|r| (str_of(r, "action").to_string(), rdcw_name(r)))
                .collect()
        };
        assert_eq!(seq("x64"), seq("arm"), "{scenario}");
    }
}
