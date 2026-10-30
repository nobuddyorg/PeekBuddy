#![no_main]
//! The USN record decoder (#47): any bytes after the next-USN header.

use libfuzzer_sys::fuzz_target;
use peekbuddy_core::usn::record::records;

fuzz_target!(|data: &[u8]| {
    if let Ok(iter) = records(data) {
        for record in iter.flatten() {
            let _ = (record.name_units(), record.is_dir(), record.name_withheld());
        }
    }
});
