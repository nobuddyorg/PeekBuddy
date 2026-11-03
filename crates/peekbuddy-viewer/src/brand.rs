//! The family wordmark (#88): `Peek` underlined in the text colour, `Buddy`
//! in the accent colour, the two touching.
//!
//! Every Buddy app draws its name this way; the sibling sites do it with
//! `border-b-2 border-foreground` on the first part and the accent colour on
//! `Buddy`. This is the one place that knows the accent: it is for the
//! wordmark only, never for buttons, links or the selected row (a test
//! greps the sources for that).
//!
//! The wordmark is where the app draws its own name. Windows draws the title
//! bar, the tray tooltip and menu, Task Manager and the Start menu, so those
//! say plain `PeekBuddy`.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Stroke};

/// The accent on a light background (4.5:1 against the top bar).
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0xad, 0x57, 0x25);
/// The accent on a dark background: lighter, so it keeps 4.5:1 on near-black.
pub const ACCENT_DARK: Color32 = Color32::from_rgb(0xe0, 0x95, 0x4f);

/// Thickness of the rule under `Peek`, in points.
pub const UNDERLINE: f32 = 2.0;

/// The accent for a theme.
#[must_use]
pub const fn accent(dark: bool) -> Color32 {
    if dark { ACCENT_DARK } else { ACCENT_LIGHT }
}

/// The two-section layout of the name at `size` points. `foreground` is the
/// text colour of the theme.
#[must_use]
pub fn wordmark_job(size: f32, foreground: Color32, dark: bool) -> LayoutJob {
    let font = FontId::proportional(size);
    let mut job = LayoutJob::default();
    job.append(
        "Peek",
        0.0,
        TextFormat {
            font_id: font.clone(),
            color: foreground,
            underline: Stroke::new(UNDERLINE, foreground),
            ..TextFormat::default()
        },
    );
    job.append(
        "Buddy",
        0.0,
        TextFormat {
            font_id: font,
            color: accent(dark),
            ..TextFormat::default()
        },
    );
    job
}

/// What the wordmark was last built for.
#[derive(Clone)]
struct Cached {
    key: (u32, Color32, bool),
    job: Arc<LayoutJob>,
}

/// Draws the wordmark. The layout is built once per size and theme and kept
/// in the context, so a steady frame builds nothing.
pub fn wordmark(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let foreground = ui.visuals().text_color();
    let key = (size.to_bits(), foreground, dark);
    let id = egui::Id::unique("wordmark");
    let cached: Option<Cached> = ui.data(|data| data.get_temp(id));
    let job = match cached {
        Some(cached) if cached.key == key => cached.job,
        _ => {
            let job = Arc::new(wordmark_job(size, foreground, dark));
            ui.data_mut(|data| {
                data.insert_temp(
                    id,
                    Cached {
                        key,
                        job: Arc::clone(&job),
                    },
                );
            });
            job
        }
    };
    ui.add(egui::Label::new((*job).clone()).selectable(false))
}

/// The WCAG contrast ratio of two opaque colours (1 to 21).
#[must_use]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn channel(value: u8) -> f32 {
        let v = f32::from(value) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    fn luminance(c: Color32) -> f32 {
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use egui::{Color32, Stroke};

    use super::{ACCENT_DARK, ACCENT_LIGHT, UNDERLINE, accent, contrast, wordmark_job};

    #[test]
    fn peek_is_underlined_in_the_text_colour_and_buddy_has_the_accent_and_no_rule() {
        for (dark, foreground) in [(false, Color32::BLACK), (true, Color32::WHITE)] {
            let job = wordmark_job(20.0, foreground, dark);
            assert_eq!(job.text, "PeekBuddy");
            let [peek, buddy] = job.sections.as_slice() else {
                unreachable!("two sections");
            };
            assert_eq!(
                job.text.get(peek.byte_range.start.0..peek.byte_range.end.0),
                Some("Peek")
            );
            assert_eq!(peek.format.color, foreground);
            assert_eq!(peek.format.underline, Stroke::new(UNDERLINE, foreground));
            assert_eq!(
                job.text
                    .get(buddy.byte_range.start.0..buddy.byte_range.end.0),
                Some("Buddy")
            );
            assert_eq!(buddy.format.color, accent(dark));
            assert_eq!(buddy.format.underline, Stroke::NONE);
            assert_eq!(buddy.leading_space, 0.0, "the two parts touch");
        }
    }

    #[test]
    fn the_accent_pairs_are_the_family_ones() {
        assert_eq!(ACCENT_LIGHT, Color32::from_rgb(0xad, 0x57, 0x25));
        assert_eq!(ACCENT_DARK, Color32::from_rgb(0xe0, 0x95, 0x4f));
        assert_eq!(accent(false), ACCENT_LIGHT);
        assert_eq!(accent(true), ACCENT_DARK);
    }

    #[test]
    fn buddy_has_4_5_to_1_on_the_top_bar_in_both_themes() {
        let light = egui::Visuals::light().panel_fill;
        let dark = egui::Visuals::dark().panel_fill;
        assert!(
            contrast(ACCENT_LIGHT, light) >= 4.5,
            "{}",
            contrast(ACCENT_LIGHT, light)
        );
        assert!(
            contrast(ACCENT_DARK, dark) >= 4.5,
            "{}",
            contrast(ACCENT_DARK, dark)
        );
    }

    #[test]
    fn contrast_is_the_wcag_ratio() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 0.001);
    }

    /// The accent is for the wordmark only: no other module names it.
    #[test]
    fn no_other_source_file_names_the_accent() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![dir];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && path.file_name().is_some_and(|name| name != "brand.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    assert!(
                        !text.contains("ACCENT_LIGHT") && !text.contains("ACCENT_DARK"),
                        "{} uses the wordmark accent",
                        path.display()
                    );
                }
            }
        }
    }
}
