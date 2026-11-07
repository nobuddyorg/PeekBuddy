//! The argv template of a rule's command (#81).
//!
//! File names are input an attacker controls, so a placeholder is only ever a
//! whole argument: `%path%` as an argument becomes exactly one argument
//! holding the path, whatever the path contains; `x%path%` is the text
//! `x%path%`. Values stay UTF-16 (F17: a lone surrogate in a name survives),
//! and [`command_line`] quotes each argument for `CreateProcessW` so that
//! `CommandLineToArgvW` and the C runtime read back the same arguments. The
//! program is started directly, never through `cmd.exe`.

/// The placeholders, as they are written in the file.
pub const PLACEHOLDERS: [&str; 4] = ["%path%", "%kind%", "%root%", "%ts%"];

/// What the placeholders stand for.
#[derive(Debug, Clone, Copy)]
pub struct Values<'a> {
    /// The full path of the file.
    pub path: &'a [u16],
    /// The export name of the kind (`created`, `deleted`, ...).
    pub kind: &'a str,
    /// The watched root the event is in.
    pub root: &'a [u16],
    /// The event time, FILETIME ticks.
    pub ts: u64,
}

/// Whether `arg` is exactly one of the placeholders.
#[must_use]
pub fn is_placeholder(arg: &str) -> bool {
    PLACEHOLDERS.contains(&arg)
}

/// A NUL would end the command line early: it becomes U+FFFD.
fn clean(units: &[u16]) -> Vec<u16> {
    units
        .iter()
        .map(|&unit| if unit == 0 { 0xFFFD } else { unit })
        .collect()
}

/// The arguments of `argv` with the placeholders filled in, as UTF-16.
#[must_use]
pub fn expand(argv: &[String], values: &Values<'_>) -> Vec<Vec<u16>> {
    argv.iter()
        .map(|arg| match arg.as_str() {
            "%path%" => clean(values.path),
            "%kind%" => values.kind.encode_utf16().collect(),
            "%root%" => clean(values.root),
            "%ts%" => values.ts.to_string().encode_utf16().collect(),
            literal => literal.encode_utf16().filter(|&unit| unit != 0).collect(),
        })
        .collect()
}

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;

/// One argument as it must appear on a command line.
fn quote(arg: &[u16], out: &mut Vec<u16>) {
    let plain = !arg.is_empty()
        && !arg
            .iter()
            .any(|&u| matches!(u, SPACE | TAB | QUOTE | 0x0A | 0x0B));
    if plain {
        out.extend_from_slice(arg);
        return;
    }
    out.push(QUOTE);
    let mut backslashes = 0_usize;
    for &unit in arg {
        match unit {
            BACKSLASH => {
                backslashes += 1;
                out.push(unit);
            }
            QUOTE => {
                // Backslashes before a quote are doubled, then the quote
                // is escaped.
                out.extend(std::iter::repeat_n(BACKSLASH, backslashes + 1));
                out.push(QUOTE);
                backslashes = 0;
            }
            other => {
                backslashes = 0;
                out.push(other);
            }
        }
    }
    // Backslashes before the closing quote are doubled.
    out.extend(std::iter::repeat_n(BACKSLASH, backslashes));
    out.push(QUOTE);
}

/// The command line for `CreateProcessW`: the arguments, quoted and joined
/// by spaces.
#[must_use]
pub fn command_line(args: &[Vec<u16>]) -> Vec<u16> {
    let mut out = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push(SPACE);
        }
        quote(arg, &mut out);
    }
    out
}

#[cfg(test)]
mod tests;
