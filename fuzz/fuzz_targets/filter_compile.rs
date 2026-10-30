#![no_main]
//! The pattern parser and `FilterSet::compile` (#40): never panics, bounded
//! memory for any rule text.

use libfuzzer_sys::fuzz_target;
use peekbuddy_core::filter::glob::parse;
use peekbuddy_core::filter::{FilterSet, Pattern, Rule};
use peekbuddy_core::name::CaseMode;

fuzz_target!(|text: &str| {
    let _ = parse(text, CaseMode::Insensitive);
    let _ = parse(text, CaseMode::Sensitive);
    let _ = FilterSet::compile(&[
        Rule::exclude(Pattern::Glob(text.to_owned())),
        Rule::include(Pattern::Regex(text.to_owned())),
    ]);
});
