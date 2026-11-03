# Brand: the wordmark

Every Buddy app writes its name the same way: the first part in the text
colour with a 2 px rule under it, and `Buddy` in the accent colour without a
rule. For this app that is <u>Peek</u>**Buddy**, with `Buddy` in the colour
below. The sibling sites do it with `border-b-2 border-foreground` on the first
part; the viewer builds the same two-section `LayoutJob`.

## Spec

| Part | Colour | Rule |
| --- | --- | --- |
| `Peek` | the text colour of the theme | 2 px, text colour |
| `Buddy` | accent: light `#ad5725`, dark `#e0954f` | none |

- The two parts touch; there is no space.
- The accent is for the wordmark only: never for buttons, links or the selected
  row (the site keeps the same rule, wordmark and chrome need different
  contrast). A test fails if another source file names the accent constants.
- `Buddy` keeps 4.5:1 against the top bar in both themes (computed in a test:
  light 5.0:1 or more, dark 7.0:1 or more). The dark shade is lighter so it
  reads on near-black.
- Code: `peekbuddy_viewer::brand::wordmark(ui, size)`. The layout is built once
  per size and theme and kept in the egui context, so a steady frame builds
  nothing.

## Where it appears

The viewer's top bar draws the wordmark. The welcome screen and the About view
(#70) will too.

## Where it cannot

Windows draws the window title bar, the tray tooltip and the tray menu, Task
Manager, the Start menu and the winget listing; they have no colour or
underline and say plain `PeekBuddy`.

## Font

egui's default font has no semibold weight. The wordmark uses the default
font at a larger size. The family display font (Archivo, SIL OFL) is not
embedded: a subset of the glyphs of `PeekBuddy` was not measured, and the
binary budget is already over its target with the renderer (#252).

## Not done yet

The logo (`peek.svg`, the folder with eyes, from nobuddy.org) and the exe and
tray icons generated from it need that file, which lives in another
repository; they are a follow-up. Until then the exe has no icon of its own and
the viewer window uses the default.
