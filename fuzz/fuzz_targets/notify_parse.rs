#![no_main]
//! The FILE_NOTIFY_* parser (#44): any bytes, any class, any returned size.

use libfuzzer_sys::fuzz_target;
use peekbuddy_core::notify::{Class, parse};

fuzz_target!(|data: &[u8]| {
    let Some((&head, buf)) = data.split_first() else {
        return;
    };
    let class = match head % 3 {
        0 => Class::Plain,
        1 => Class::Extended,
        _ => Class::Full,
    };
    // The size the call reported need not match the buffer.
    let returned = usize::from(head >> 2) * buf.len() / 64;
    let _ = parse(buf, returned, class).count();
    let _ = parse(buf, buf.len(), class).count();
});
