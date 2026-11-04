#![no_main]
//! The viewer's filter text (#67): any text a user can paste into the filter
//! bar parses or is refused with a position, and what parses prints back to
//! text that parses to the same filter.

use libfuzzer_sys::fuzz_target;
use peekbuddy_viewer::filterspec::FilterSpec;

fuzz_target!(|text: &str| {
    if let Ok(spec) = FilterSpec::parse(text) {
        assert_eq!(FilterSpec::parse(&spec.to_text()), Ok(spec));
    }
});
