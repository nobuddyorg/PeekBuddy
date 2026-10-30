#![no_main]
//! The export readers (#56): CSV and JSON Lines from arbitrary text.

use libfuzzer_sys::fuzz_target;
use peekbuddy_core::export::read::{ReadOptions, read_csv, read_jsonl};

fuzz_target!(|text: &str| {
    let _ = read_csv(text, ReadOptions::default());
    let _ = read_csv(text, ReadOptions { excel_safe: true });
    let _ = read_jsonl(text);
});
