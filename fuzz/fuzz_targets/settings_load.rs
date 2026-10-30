#![no_main]
//! The settings file loader (#68 family): any text is a file we may find on
//! disk, and loading it never panics.

use libfuzzer_sys::fuzz_target;
use peekbuddy_core::settings::{Context, load};

fuzz_target!(|text: &str| {
    let ctx = Context {
        data_dir: String::from(r"C:\Users\x\AppData\Local\nobuddy\peekbuddy"),
        export_dir: None,
    };
    let _ = load(text, &ctx);
});
