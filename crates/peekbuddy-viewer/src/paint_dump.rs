//! Layout snapshots of the screens (#71): text, not pixels.
//!
//! An image snapshot needs a GPU renderer (`egui_kittest` with its `wgpu`
//! feature). The hosted runners draw on WARP, and the anti-aliasing and the
//! glyph rasterisation of a software adapter change with the driver build, so
//! a committed PNG would fail after a runner image update without any change
//! in the viewer. The layout does not depend on a renderer: what egui lays
//! out (the AccessKit tree with the rectangle of every control and label) and
//! what it paints (the text, the filled rectangles and every other shape, with
//! their colours) is plain data and the same on every host. [`dump`] writes it
//! as text, and `insta` compares it with the committed file. A moved control,
//! a clipped label, a changed colour in one theme or a layout that breaks at
//! 1.5 pixels per point shows up as a diff of lines; what the pixels look like
//! is a review of the owner's own screen.
//!
//! Positions are in points, rounded to a tenth. egui snaps its layout to the
//! pixel grid of the frame, so 1.0 and 1.5 pixels per point differ.

use std::fmt::Write as _;

use egui::{Color32, Rect, Shape};
use egui_kittest::Harness;
use egui_kittest::kittest::NodeT as _;

/// The ways a screen is looked at: the theme and the pixels per point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Look {
    pub(crate) theme: egui::Theme,
    pub(crate) ppp: f32,
}

impl Look {
    /// Light and dark, at 1.0 and 1.5 pixels per point.
    pub(crate) const ALL: [Self; 4] = [
        Self::new(egui::Theme::Light, 1.0),
        Self::new(egui::Theme::Light, 1.5),
        Self::new(egui::Theme::Dark, 1.0),
        Self::new(egui::Theme::Dark, 1.5),
    ];

    pub(crate) const fn new(theme: egui::Theme, ppp: f32) -> Self {
        Self { theme, ppp }
    }

    /// The name of the snapshot file part, like `dark_1.5ppp`.
    pub(crate) fn tag(self) -> String {
        let theme = match self.theme {
            egui::Theme::Light => "light",
            egui::Theme::Dark => "dark",
        };
        format!("{theme}_{:.1}ppp", self.ppp)
    }
}

/// `#rrggbb`, with the alpha appended when the colour is not opaque.
fn hex(color: Color32) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// A coordinate rounded to a tenth. Adding zero turns `-0.0` into `0.0`.
fn tenth(value: f32) -> f32 {
    (value * 10.0).round() / 10.0 + 0.0
}

fn rect_text(rect: Rect) -> String {
    format!(
        "{:.1},{:.1} {:.1}x{:.1}",
        tenth(rect.min.x),
        tenth(rect.min.y),
        tenth(rect.width()),
        tenth(rect.height())
    )
}

/// `text` cut to `max` characters, with the cut shown.
fn short(text: &str, max: usize) -> String {
    let one_line = text.replace('\n', "\\n");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let head: String = one_line.chars().take(max).collect();
        format!("{head}...")
    }
}

fn paint(shape: &Shape, region: Rect, out: &mut String) {
    match shape {
        Shape::Vec(shapes) => {
            for inner in shapes {
                paint(inner, region, out);
            }
        }
        Shape::Noop => {}
        Shape::Text(text) => {
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            if !region.intersects(bounds) {
                return;
            }
            // One colour per section of the text (the wordmark has two).
            let mut colors: Vec<String> = Vec::new();
            for section in &text.galley.job.sections {
                let color = text.override_text_color.unwrap_or(
                    if section.format.color == Color32::PLACEHOLDER {
                        text.fallback_color
                    } else {
                        section.format.color
                    },
                );
                let color = hex(color);
                if colors.last() != Some(&color) {
                    colors.push(color);
                }
            }
            let _ = writeln!(
                out,
                "text {} {} {:?}",
                rect_text(bounds),
                colors.join("/"),
                short(text.galley.text(), 60)
            );
        }
        Shape::Rect(rect) => {
            if !region.intersects(rect.rect) {
                return;
            }
            let _ = write!(out, "rect {}", rect_text(rect.rect));
            if rect.fill != Color32::TRANSPARENT {
                let _ = write!(out, " fill {}", hex(rect.fill));
            }
            if !rect.stroke.is_empty() {
                let _ = write!(
                    out,
                    " stroke {} {:.1}",
                    hex(rect.stroke.color),
                    tenth(rect.stroke.width)
                );
            }
            out.push('\n');
        }
        other => {
            let bounds = other.visual_bounding_rect();
            if region.intersects(bounds) {
                let name = format!("{other:?}");
                let kind = name.split(['(', ' ', '{']).next().unwrap_or("shape");
                let _ = writeln!(out, "{} {}", kind.to_lowercase(), rect_text(bounds));
            }
        }
    }
}

/// The screen of `harness` as text: the accessible tree and the paint list of
/// everything inside `region`, under a header with `scene` and `look`.
pub(crate) fn dump<S>(harness: &Harness<'_, S>, scene: &str, look: Look, region: Rect) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "scene: {scene}");
    let _ = writeln!(out, "look: {}", look.tag());
    let _ = writeln!(out, "region: {}", rect_text(region));
    let _ = writeln!(out, "\n[accessible tree]");
    for node in harness.root().children_recursive() {
        let access = node.accesskit_node();
        let label = access.label().unwrap_or_default();
        if access.bounding_box().is_none() {
            continue;
        }
        let rect = node.rect();
        if label.is_empty() || !region.intersects(rect) {
            continue;
        }
        let _ = writeln!(
            out,
            "{:?} {:?} {}",
            access.role(),
            short(&label, 60),
            rect_text(rect)
        );
    }
    let _ = writeln!(out, "\n[painted, back to front]");
    for clipped in &harness.output().shapes {
        paint(&clipped.shape, region, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use egui::{Rect, pos2};
    use egui_kittest::Harness;

    use super::{Look, dump, hex, short, tenth};

    #[test]
    fn numbers_and_colours_are_written_the_same_everywhere() {
        assert_eq!(tenth(-0.04), 0.0);
        assert_eq!(format!("{:.1}", tenth(-0.04)), "0.0");
        assert_eq!(tenth(12.34), 12.3);
        assert_eq!(hex(egui::Color32::from_rgb(1, 2, 255)), "#0102ff");
        let see_through = hex(egui::Color32::from_rgba_unmultiplied(10, 20, 30, 128));
        assert!(see_through.len() == 9 && see_through.ends_with("80"));
        assert_eq!(short("abc", 3), "abc");
        assert_eq!(short("abcd", 3), "abc...");
        assert_eq!(short("a\nb", 9), "a\\nb");
    }

    #[test]
    fn the_dump_lists_what_is_inside_the_region_and_nothing_else() {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(300.0, 200.0))
            .build_ui(|ui| {
                ui.label("inside");
                ui.add_space(400.0);
                ui.label("far below");
            });
        harness.run();
        let look = Look::ALL[0];
        let region = Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 100.0));
        let text = dump(&harness, "test", look, region);
        assert!(text.starts_with("scene: test\nlook: light_1.0ppp\n"));
        assert!(text.contains("\"inside\""), "{text}");
        assert!(!text.contains("far below"), "{text}");
        assert!(text.contains("[painted, back to front]"));
    }

    #[test]
    fn a_look_names_its_theme_and_scale() {
        let tags: Vec<String> = Look::ALL.iter().map(|look| look.tag()).collect();
        assert_eq!(
            tags,
            ["light_1.0ppp", "light_1.5ppp", "dark_1.0ppp", "dark_1.5ppp"]
        );
    }
}
