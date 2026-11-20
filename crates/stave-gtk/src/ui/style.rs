//! The window's look: libadwaita's, with the design's own sizes and
//! colours for the title bar's switcher, pill and build button, the parts
//! tree, the inspector's cards and the status bar. Colours are
//! libadwaita's named colours, so they follow the light, dark and
//! high-contrast styles the moment the style manager changes; where the
//! design's own translucent tints would stay faint in high contrast, the
//! `HIGH_CONTRAST` sheet makes them solid.

use gtk::gdk;

/// Sizes and shapes, the same in light and dark. Text sizes are in `rem`,
/// a share of the interface font (11 pt, about 14.7 px, by default), so
/// they grow with the desktop's Large Text and text-scaling settings as
/// libadwaita's own do; a size in `px` would stay put. Code is in the
/// monospace font the user chose (libadwaita's `--monospace-font-family`),
/// and words inside the editor in the document font.
const SHARED: &str = r#"
.stave-header { min-height: 50px; }
.stave-header > windowhandle > box { padding: 0 8px; }
.stave-header button.stave-icon-button { min-width: 34px; min-height: 34px; padding: 0; border-radius: 8px; }
.stave-header button.stave-sidebar-toggle { min-width: 36px; min-height: 36px; }
.stave-title { padding: 0 6px; }
.stave-title .title { font-weight: 700; font-size: 0.955rem; }
.stave-title .subtitle { font-size: 0.818rem; }

.stave-switcher { padding: 3px; border-radius: 10px; font-size: 0.886rem; font-weight: 600; }
.stave-switcher > toggle { min-height: 32px; padding: 0 16px; border-radius: 8px; }
.stave-switcher > toggle box { border-spacing: 6px; }
.stave-switcher.compact > toggle { padding: 0 10px; }
.stave-switcher > separator { background: none; min-width: 0; margin: 0; }

menubutton.stave-scene > button { min-height: 34px; padding: 0 8px 0 10px; border-radius: 8px; font-size: 0.886rem; font-weight: 600; }
menubutton.stave-scene > button > box { border-spacing: 6px; }

button.stave-pill { min-height: 34px; padding: 0 12px; border-radius: 17px; font-size: 0.886rem; font-weight: 600; }
button.stave-pill box { border-spacing: 6px; }
button.stave-pill.open { outline: 2px solid currentColor; outline-offset: -2px; }

button.stave-build { min-width: 36px; min-height: 34px; padding: 0; border-radius: 8px; }

.stave-sidebar { padding: 12px 10px; }
.stave-tree { background: none; font-size: 0.886rem; }
.stave-tree > row { padding: 0; border-radius: 6px; min-height: 0; margin: 0 0 1px 0; }
.stave-tree .stave-entry { padding: 5px 8px 5px 20px; border-spacing: 8px; }
.stave-tree .stave-entry.nested { padding-left: 34px; }
.stave-tree .stave-entry.nested2 { padding-left: 48px; }
.stave-tree .stave-assembly { padding: 5px 8px; border-spacing: 8px; font-weight: 600; }
.stave-tree .stave-assembly.nested { padding-left: 22px; }
.stave-tree .stave-detail { font-size: 0.818rem; }
.stave-tree .stave-assembly .stave-detail { font-weight: 500; }
.stave-tree .stave-heading { padding: 10px 8px 6px 8px; font-size: 0.818rem; font-weight: 700; letter-spacing: 0.02em; }
.stave-tree .stave-heading.first { padding-top: 6px; }
.stave-tree .stave-library-entry { padding: 4px 8px 5px 12px; border-spacing: 1px; }
.stave-tree .stave-sidebar-note { padding: 4px 8px 0 8px; font-size: 0.886rem; }

.stave-inspector { padding: 18px 16px; }
.stave-inspector-panel { min-width: 0; }
.stave-eyebrow { font-size: 0.818rem; font-weight: 600; }
.stave-inspector-title { font-size: 1.364rem; font-weight: 700; }
.stave-description { font-size: 0.886rem; }
.stave-card { border-radius: 12px; font-size: 0.886rem; }
.stave-card > .stave-card-row { padding: 11px 14px; border-spacing: 12px; }
.stave-card > button.stave-card-row { padding: 10px 14px; border-radius: 0; min-height: 0; font-weight: normal; }
.stave-card > :first-child { border-top-left-radius: 12px; border-top-right-radius: 12px; }
.stave-card > :last-child { border-bottom-left-radius: 12px; border-bottom-right-radius: 12px; border-bottom: none; }
.stave-card .stave-value { font-weight: 600; }
/* Upper case drawn, not written: a screen reader reads the words. */
.stave-caps { text-transform: uppercase; }
.stave-section { font-size: 0.818rem; font-weight: 700; letter-spacing: 0.02em; }
.stave-link { font-size: 0.818rem; font-weight: 600; padding: 0 2px; min-height: 0; }
.stave-code { padding: 12px 14px; border-radius: 12px; font-family: var(--monospace-font-family, monospace); font-size: 0.818rem; }
.stave-footnote { font-size: 0.818rem; }


.stave-floating { border-radius: 12px; padding: 8px 14px; font-size: 0.886rem; border-spacing: 12px; }
.stave-placeholder { font-size: 0.886rem; }

.stave-crumbs { min-height: 30px; padding: 0 16px; font-size: 0.818rem; }
.stave-crumb.current { font-weight: 600; }
textview.stave-editor { font-family: var(--monospace-font-family, monospace); font-size: 0.886rem; }
.stave-editor-footer { min-height: 28px; padding: 0 16px; font-size: 0.818rem; }
.stave-lens { font-family: var(--document-font-family, sans-serif); font-size: 0.784rem; border-spacing: 2px; }
button.stave-lens-link { min-height: 0; min-width: 0; padding: 0 2px; font-weight: normal; border-radius: 4px; }
.stave-peek { border-radius: 10px; margin-top: 2px; }
.stave-peek-head { padding: 7px 12px; font-family: var(--document-font-family, sans-serif); font-size: 0.818rem; border-radius: 10px 10px 0 0; }
.stave-peek-path { font-weight: 700; }
.stave-peek-body { padding: 6px 0; }
.stave-peek-line { min-height: 21px; }
.stave-peek-number { padding-right: 12px; font-family: var(--monospace-font-family, monospace); font-size: 0.852rem; }
.stave-peek-code { font-family: var(--monospace-font-family, monospace); font-size: 0.852rem; }
.stave-hover { border-radius: 10px; padding: 10px 12px; font-family: var(--document-font-family, sans-serif); font-size: 0.886rem; }
.stave-hover-kind { font-size: 0.75rem; font-weight: 700; letter-spacing: 0.04em; }
.stave-hover-signature { font-family: var(--monospace-font-family, monospace); font-size: 0.852rem; font-weight: 600; }
.stave-hover-where, .stave-hover-help { font-size: 0.818rem; }
.stave-uses-head { padding: 4px 8px 4px 16px; font-size: 0.886rem; }
.stave-uses-list { background: none; }
.stave-uses-list > row { padding: 0; }
.stave-use { padding: 6px 16px; font-size: 0.852rem; }
.stave-use-place { font-weight: 600; }
.stave-use-line { font-family: var(--monospace-font-family, monospace); font-size: 0.818rem; }
.stave-chip { border-radius: 16px; padding: 6px 12px 6px 8px; font-size: 0.818rem; font-weight: 600; }
.stave-stage.page { padding: 16px; border-spacing: 14px; }
.stave-empty-title { font-size: 1.364rem; font-weight: 700; }
.stave-empty-description { font-size: 0.955rem; }
.stave-stage.page .stave-figure { border-radius: 14px; }
.stave-toolbar { padding: 4px; border-radius: 12px; border-spacing: 4px; }
.stave-tool-slider { padding: 4px 10px 2px 2px; }
.stave-tool-end { font-size: 0.75rem; }
.stave-camera { padding: 4px; border-radius: 10px; }
.stave-camera button { min-width: 32px; min-height: 32px; padding: 0; border-radius: 7px; }
.stave-camera.joined { border-bottom-left-radius: 0; }
.stave-camera-separator { min-height: 1px; margin: 2px 4px; }
.stave-pull-popout { padding: 4px 6px 4px 4px; border-radius: 10px 0 0 10px; border-spacing: 4px; }
.stave-moves { padding: 4px 6px 4px 10px; border-radius: 12px; }
.stave-moves .stave-dock-label { min-height: 30px; padding: 0 2px 0 0; }
.stave-contact-key { padding: 6px 12px; border-radius: 14px; font-size: 0.818rem; font-weight: 600; }
.stave-plan-switch { padding: 3px; border-radius: 10px; }
.stave-plan-switch button { min-width: 34px; min-height: 30px; padding: 0; border-radius: 7px; }
.stave-focus-card { padding: 12px 16px; border-radius: 14px; font-size: 0.886rem; }
.stave-focus-card.stave-problem-card { padding: 14px 18px 12px 18px; }
button.stave-play { min-width: 40px; min-height: 40px; padding: 0; }
.stave-card-dim { font-size: 0.818rem; }
.stave-card-value { font-family: var(--monospace-font-family, monospace); font-size: 0.886rem; font-weight: 500; }
.stave-card-blocked { font-size: 0.818rem; font-weight: 600; }
.stave-card-name { font-size: 0.886rem; font-weight: 600; }
.stave-focus-caption { font-size: 0.886rem; }
.stave-focus-caption .stave-card-dim { font-size: 0.886rem; }
.stave-pictures button.stave-picture { padding: 10px; border-radius: 12px; font-weight: normal; }
.stave-picture-caption { font-size: 0.818rem; }

button.stave-close { min-width: 28px; min-height: 28px; padding: 0; }
.stave-problem-assembly { padding: 5px 8px; font-weight: 600; }
.stave-problem-assembly.depth1 { padding-left: 22px; }
.stave-problem-assembly.depth2 { padding-left: 36px; }
.stave-problem-assembly .stave-detail { font-size: 0.818rem; font-weight: 500; }
.stave-problem-part { padding: 5px 8px 5px 20px; }
.stave-problem-part.depth1 { padding-left: 30px; }
.stave-problem-part.depth2 { padding-left: 44px; }
.stave-problem-part .stave-detail { font-size: 0.818rem; }
.stave-problem-kind { font-weight: 600; }
.stave-problem-places { font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; }
.stave-eyebrow.stave-eyebrow-bold, .stave-eyebrow.stave-problem-eyebrow { font-weight: 700; letter-spacing: 0.02em; }
.stave-problem-title { font-size: 1.295rem; }
.stave-code.stave-diagnostic { padding: 14px; font-size: 0.784rem; }
.stave-problem-row { padding: 4px 0; }
.stave-problem-heading { font-weight: 600; }
.stave-problem-code { font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; font-weight: 700; }
.stave-problem-place { font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; padding: 0 2px; min-height: 20px; }
.stave-code.stave-problem-snippet { padding: 6px 10px; border-radius: 8px; font-size: 0.784rem; }
.stave-problem-note, .stave-problem-help { font-size: 0.852rem; }
.stave-problems-stale { padding: 8px 12px 2px 12px; }
.stave-number { min-width: 22px; min-height: 22px; border-radius: 11px; font-size: 0.818rem; font-weight: 700; }
.stave-card > .stave-card-row.stave-toggle-row { padding: 10px 14px; }
.stave-card > .stave-card-row.compact { padding: 8px 14px; }
.stave-card > .stave-card-row.compact .stave-number { min-width: 20px; min-height: 20px; border-radius: 10px; font-size: 0.75rem; }
expander.stave-more title { font-size: 0.886rem; font-weight: 600; }
expander.stave-more title > arrow { min-width: 12px; min-height: 12px; }

.stave-panel { border-radius: 14px; }
.stave-model-head { padding: 8px 10px 0 14px; }
.stave-model-note { padding: 6px 14px 12px 14px; font-size: 0.818rem; }
.stave-tree .stave-joint { padding: 4px 8px 4px 34px; border-spacing: 8px; font-size: 0.852rem; }
.stave-joint-word { font-size: 0.682rem; font-weight: 700; padding: 1px 4px; border-radius: 3px; }
.stave-mark { min-width: 8px; min-height: 8px; border-radius: 4px; }
.stave-mark.warning { border-radius: 2px; }
.stave-card-panel { border-radius: 16px; }
.stave-card-panel .stave-inspector { padding: 14px 16px; }
.stave-dock { padding: 4px 8px; border-spacing: 6px; font-size: 0.886rem; }
.stave-dock button.stave-tool { min-height: 34px; padding: 0 12px; border-radius: 9px; font-weight: 600; }
.stave-dock .stave-dock-separator { min-width: 1px; margin: 6px 2px; }
.stave-dock .stave-dock-label { font-size: 0.818rem; padding: 0 4px; }
.stave-dock button.stave-move { min-height: 30px; padding: 0 10px; border-radius: 15px; font-size: 0.818rem; font-weight: 600; }
.stave-code-tabs { padding: 4px 6px; border-spacing: 2px; }
button.stave-code-tab { min-height: 28px; padding: 0 10px; border-radius: 7px; font-size: 0.852rem; font-weight: 500; }
button.stave-code-tab.active { font-weight: 700; }
.stave-code-tab-name { font-family: var(--monospace-font-family, monospace); }
.stave-panel-list row { padding: 6px 10px; }
.stave-panel-empty { padding: 16px; }
.stave-status-text { font-size: 0.818rem; padding: 0 8px; }
.stave-model .stave-tree { background: transparent; }
.stave-editor-head { min-height: 44px; padding: 0 8px 0 16px; }
.stave-editor-title { font-family: var(--monospace-font-family, monospace); font-size: 0.886rem; font-weight: 600; }
button.stave-problem-step { min-width: 28px; min-height: 28px; padding: 0; border-radius: 14px; }
.stave-pin { min-width: 24px; min-height: 24px; padding: 0; border-radius: 12px 12px 12px 2px; font-size: 0.818rem; font-weight: 700; }
.stave-pin.warning { border-radius: 4px; }
.stave-pin-label { padding: 3px 8px; border-radius: 8px; font-size: 0.818rem; font-weight: 600; }
.stave-edge-pin { min-height: 32px; padding: 0 12px 0 10px; border-radius: 16px; font-size: 0.818rem; font-weight: 600; }
.stave-edge-pin box { border-spacing: 6px; }
.stave-finder { border-radius: 16px; }
.stave-finder entry { min-height: 52px; font-size: 1.159rem; border-radius: 16px 16px 0 0; }
.stave-finder-filters { padding: 10px 16px 6px 16px; border-spacing: 6px; font-size: 0.818rem; font-weight: 600; }
.stave-finder-filters togglebutton { min-height: 28px; padding: 0 10px; border-radius: 14px; }
.stave-finder-list { background: none; padding: 4px 8px 8px 8px; }
.stave-finder-list > row { border-radius: 9px; min-height: 36px; padding: 0 10px; }
.stave-finder-heading { padding: 10px 8px 4px 8px; font-size: 0.784rem; font-weight: 700; letter-spacing: 0.03em; }
.stave-finder-hit { border-spacing: 10px; font-size: 0.92rem; }
.stave-finder-hit .stave-detail { font-size: 0.818rem; }
.stave-finder-line { font-family: var(--monospace-font-family, monospace); font-size: 0.852rem; }
.stave-finder-foot { padding: 10px 16px; font-size: 0.818rem; border-spacing: 16px; border-radius: 0 0 16px 16px; }
/* The 3D view shows when it has the keyboard (its arrow keys turn it). */
staveviewport:focus-visible { outline: 2px solid alpha(@accent_color, 0.8); outline-offset: -2px; }
"#;

/// The colours, from libadwaita's named colours, so the light and dark
/// styles (Main.dc.html, MainDark.dc.html) both come from this one sheet
/// and follow the style manager the moment it changes. Only the code
/// blocks are dark in both styles, as the design has it (darker still,
/// with an edge, on the dark style's dark panels).
const COLOURS: &str = r#"
.stave-header { background: @headerbar_bg_color; color: @headerbar_fg_color; box-shadow: inset 0 -1px @headerbar_shade_color; }
/* Solid whether or not the window has focus: libadwaita fades an
   unfocused window's header bar, which let the desktop show through. */
headerbar.stave-header:backdrop { background-color: @headerbar_bg_color; transition: none; }
headerbar.stave-header:backdrop > windowhandle { filter: none; }
window.stave-window { background-color: @window_bg_color; }
/* The panels' headers (TabStrip): a slim strip of flush tabs, as in an
   IDE; the shown tab carries an accent line, the others are dimmer. */
stavetabstrip { min-height: 28px; background: @headerbar_bg_color; color: @headerbar_fg_color; box-shadow: inset 0 -1px @headerbar_shade_color; }
stavetabstrip .stave-tab { border-radius: 0; color: alpha(currentColor, 0.65); }
stavetabstrip .stave-tab:hover { background: alpha(currentColor, 0.05); color: currentColor; }
stavetabstrip .stave-tab.shown { color: currentColor; background: alpha(currentColor, 0.06); box-shadow: inset 0 -2px @accent_bg_color; }
stavetabstrip button.stave-tab-show { min-height: 28px; padding: 0 4px 0 10px; border-radius: 0; background: transparent; color: inherit; font-size: 0.818rem; font-weight: 600; }
stavetabstrip button.stave-tab-show image { -gtk-icon-size: 14px; }
stavetabstrip .stave-tab-fixed button.stave-tab-show { padding: 0 10px; }
stavetabstrip button.stave-tab-close { min-height: 18px; min-width: 18px; margin: 0 6px 0 0; padding: 0; border-radius: 4px; opacity: 0; }
stavetabstrip .stave-tab.shown button.stave-tab-close, stavetabstrip .stave-tab:hover button.stave-tab-close { opacity: 0.7; }
stavetabstrip button.stave-tab-close:hover { opacity: 1; background: alpha(currentColor, 0.1); }
stavetabstrip button.stave-tab-close image { -gtk-icon-size: 12px; }
stavetabstrip .stave-tab-end button { min-height: 24px; min-width: 24px; padding: 0; margin: 2px 4px; border-radius: 5px; }
.stave-header button.stave-icon-button:disabled { color: alpha(currentColor, 0.45); }
.stave-header button.stave-sidebar-toggle:checked { background: alpha(currentColor, 0.1); }
.stave-title .subtitle, .dim { color: alpha(currentColor, 0.7); }
menubutton.stave-scene > button { background: alpha(currentColor, 0.08); }
menubutton.stave-scene > button:hover { background: alpha(currentColor, 0.12); }
.stave-scene-float menubutton.stave-scene > button { background: transparent; }
.stave-scene-float menubutton.stave-scene > button:hover { background: alpha(currentColor, 0.07); }
.stave-problems-chip.clean button.stave-pill { background: alpha(@success_color, 0.15); color: @success_color; }
.stave-problems-chip.warnings button.stave-pill { background: alpha(@warning_color, 0.18); color: @warning_color; }
.stave-problems-chip.errors button.stave-pill { background: alpha(@error_color, 0.14); color: @error_color; }
.stave-problems-chip.unknown button.stave-pill { background: alpha(currentColor, 0.08); color: alpha(currentColor, 0.7); }
button.stave-build { background: @accent_bg_color; color: @accent_fg_color; }
button.stave-build:hover { background: mix(@accent_bg_color, @accent_fg_color, 0.1); }
button.stave-build:disabled { background: alpha(currentColor, 0.08); color: alpha(@headerbar_fg_color, 0.45); }
button.stave-build.building:disabled { background: alpha(@accent_bg_color, 0.45); color: @accent_fg_color; }

.stave-sidebar { background: @sidebar_bg_color; color: @sidebar_fg_color; box-shadow: inset -1px 0 @sidebar_shade_color; }
.stave-tree > row:hover { background: alpha(currentColor, 0.06); }
.stave-tree > row:selected { background: @accent_bg_color; color: @accent_fg_color; }
.stave-tree > row:selected .stave-detail { color: alpha(@accent_fg_color, 0.9); }
.stave-tree .stave-detail, .stave-tree .stave-heading { color: alpha(currentColor, 0.7); }

.stave-inspector-panel { background: @window_bg_color; color: @window_fg_color; box-shadow: inset 1px 0 @headerbar_shade_color; }
.stave-eyebrow, .stave-description, .stave-section, .stave-footnote, .stave-key { color: alpha(currentColor, 0.7); }
.stave-card { background: @card_bg_color; color: @card_fg_color; box-shadow: 0 0 0 1px @card_shade_color; }
.stave-card > .stave-card-row { border-bottom: 1px solid alpha(currentColor, 0.08); }
.stave-card > button.stave-card-row { background: transparent; color: inherit; }
.stave-card > button.stave-card-row:hover { background: alpha(currentColor, 0.05); }
.stave-link { color: @accent_color; }
/* Every code block (the "Defined in the text" snippet, a problem's
   `stave check` text, a library's import line) is one style: dark in
   both styles, as the design has it. On the dark style's panels
   (#222226, cards about #38383b) #1e1e24 would all but vanish, so there
   it sits a clear step darker with a thin light edge (the window has
   `stave-dark` in the dark style). */
.stave-code { background: #1e1e24; color: #e7e5ea; }
.stave-dark .stave-code { background: #121216; box-shadow: inset 0 0 0 1px alpha(@window_fg_color, 0.16); }

.stave-floating { background: alpha(@view_bg_color, 0.94); color: @view_fg_color; box-shadow: 0 2px 10px alpha(black, 0.15); }

.stave-editor-column { background: @view_bg_color; color: @view_fg_color; box-shadow: inset -1px 0 mix(@view_bg_color, @view_fg_color, 0.15); }
.stave-crumbs { color: alpha(currentColor, 0.7); box-shadow: inset 0 -1px alpha(currentColor, 0.07); }
.stave-crumb.current { color: @view_fg_color; }
.stave-editor-footer { color: alpha(currentColor, 0.7); box-shadow: inset 0 1px alpha(currentColor, 0.07); }
.stave-lens { color: alpha(currentColor, 0.7); }
button.stave-lens-link { color: @accent_color; background: none; }
button.stave-lens-link:hover { background: alpha(@accent_color, 0.1); }
.stave-peek { background: mix(@view_bg_color, @accent_bg_color, 0.02); box-shadow: 0 0 0 1px alpha(@accent_bg_color, 0.35), 0 4px 14px alpha(@accent_bg_color, 0.12); }
.stave-peek-head { background: alpha(@accent_bg_color, 0.1); box-shadow: inset 0 -1px alpha(@accent_bg_color, 0.18); }
.stave-peek-path { color: @accent_color; }
.stave-peek-what, .stave-peek-number { color: alpha(currentColor, 0.6); }
.stave-peek-line.current { background: alpha(@accent_bg_color, 0.06); }
.stave-peek-line.current .stave-peek-number { color: @accent_color; }
.stave-hover { background: @popover_bg_color; color: @popover_fg_color; box-shadow: 0 0 0 1px alpha(black, 0.1), 0 4px 16px alpha(black, 0.18); }
.stave-hover-kind, .stave-hover-where, .stave-hover-help { color: alpha(currentColor, 0.7); }
.stave-hover-kind.error { color: @error_color; }
.stave-hover-kind.warning { color: @warning_color; }
.stave-uses { background: @view_bg_color; box-shadow: inset 0 1px @headerbar_shade_color; }
.stave-uses-list > row:hover { background: alpha(currentColor, 0.05); }
.stave-use-place { color: @accent_color; }
.stave-use-line { color: alpha(currentColor, 0.6); }
.stave-chip { background: alpha(@view_bg_color, 0.92); color: @accent_color; box-shadow: 0 1px 6px alpha(black, 0.1); }
/* The stage is the 3D view's own ground colour (render::Backdrop draws
   #e8e5df light, #242428 dark), so the page round the view matches the
   picture; it is the one place a fixed colour must stay. What sits on it
   is in the window's text colour, which libadwaita darkens or lightens
   for high contrast. */
.stave-stage { background: #e8e5df; }
.stave-stage.dark { background: #242428; }
.stave-empty { color: @window_fg_color; }
.stave-empty-icon, .stave-empty-description { opacity: 0.75; }
.stave-toolbar, .stave-plan-switch { background: alpha(@view_bg_color, 0.92); color: @view_fg_color; box-shadow: 0 1px 6px alpha(black, 0.1); }
.stave-camera { background: alpha(@view_bg_color, 0.88); color: @view_fg_color; }
.stave-camera button:hover { background: alpha(currentColor, 0.07); }
.stave-camera button:checked { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-camera button.stave-tool-play { color: @accent_color; }
.stave-camera-separator { background: alpha(currentColor, 0.15); }
.stave-moves { background: alpha(@view_bg_color, 0.92); color: @view_fg_color; box-shadow: 0 1px 6px alpha(black, 0.1); }
.stave-toolbar button.stave-tool, .stave-plan-switch button { background: transparent; color: inherit; }
.stave-toolbar button.stave-tool:hover, .stave-plan-switch button:hover { background: alpha(currentColor, 0.07); }
.stave-toolbar button.stave-tool:checked, .stave-plan-switch button:checked { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-slider { color: @accent_bg_color; }
.stave-tool-end, .stave-card-dim { color: alpha(currentColor, 0.7); }
.stave-contact-key { background: alpha(@view_bg_color, 0.9); color: @error_color; }
.stave-focus-card { background: alpha(@view_bg_color, 0.95); color: @view_fg_color; box-shadow: 0 4px 16px alpha(black, 0.12); }
button.stave-play { background: @accent_bg_color; color: @accent_fg_color; }
button.stave-play:hover { background: mix(@accent_bg_color, @accent_fg_color, 0.1); }
.stave-card-value.blocked, .stave-card-blocked { color: @error_color; }
.stave-focus-caption { color: @window_fg_color; }
.stave-pictures button.stave-picture { background: alpha(@view_bg_color, 0.45); color: @view_fg_color; }
.stave-pictures button.stave-picture:hover { background: alpha(@view_bg_color, 0.7); }
.stave-pictures button.stave-picture.current { background: @view_bg_color; box-shadow: 0 0 0 2px @accent_bg_color; }

.stave-problem-icon.error { color: @error_color; }
.stave-problem-icon.warning { color: @warning_color; }
.stave-problem-icon.note { color: alpha(currentColor, 0.7); }
.stave-problem-code.error { color: @error_color; }
.stave-problem-code.warning { color: @warning_color; }
.stave-problem-note { color: alpha(currentColor, 0.75); }
.stave-problem-help { color: @success_color; }
.stave-panel-list, .stave-problems-stale { color: @window_fg_color; }
.stave-panel-list > row:selected { background: alpha(@accent_bg_color, 0.14); color: @window_fg_color; }
.stave-problem-places { color: @accent_color; }
.stave-eyebrow.stave-problem-eyebrow.error { color: @error_color; }
.stave-eyebrow.stave-problem-eyebrow.warning { color: @warning_color; }
.stave-number { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-card .stave-value.good { color: @success_color; }
.stave-card .stave-value.bad { color: @error_color; }

.stave-panel, .stave-edge-pin, .stave-pin-label { background: alpha(@view_bg_color, 0.95); color: @view_fg_color; box-shadow: 0 2px 12px alpha(black, 0.12); }
.stave-card-panel { background: @view_bg_color; color: @view_fg_color; box-shadow: 0 0 0 1px alpha(black, 0.08), 0 10px 32px alpha(black, 0.18); }
.stave-joint-word { background: alpha(currentColor, 0.1); }
.stave-mark.error { background: @error_color; }
.stave-mark.warning { background: @warning_color; }
.stave-mark.involved { box-shadow: inset 0 0 0 1.5px @error_color; }
.stave-dock button.stave-tool { background: transparent; color: inherit; }
.stave-dock button.stave-tool:hover { background: alpha(currentColor, 0.07); }
.stave-dock button.stave-tool:checked { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-dock .stave-dock-separator { background: alpha(currentColor, 0.15); }
.stave-dock .stave-dock-label { color: alpha(currentColor, 0.7); }
.stave-dock button.stave-move { background: alpha(currentColor, 0.06); color: inherit; }
.stave-dock button.stave-move.error { box-shadow: inset 0 0 0 1px alpha(@error_color, 0.6); }
.stave-dock button.stave-move.warning { box-shadow: inset 0 0 0 1px alpha(@warning_color, 0.7); }
.stave-dock button.stave-move.current { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-code-head { background: @headerbar_bg_color; box-shadow: inset 0 -1px @headerbar_shade_color; }
button.stave-code-tab { background: transparent; color: alpha(currentColor, 0.8); }
button.stave-code-tab:hover { background: alpha(currentColor, 0.07); }
button.stave-code-tab.active { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-editor-column { background: @view_bg_color; color: @view_fg_color; }
/* The Details panel's scrolled window takes the inspector's own colour, so
   the panel reads as one surface: the inspector is only as tall as what
   it shows, and the scrolled window fills the rest of the panel. */
.stave-details { background: @window_bg_color; color: @window_fg_color; }
.stave-editor-head { background: @headerbar_bg_color; border-bottom: 1px solid alpha(currentColor, 0.1); }
.stave-pin { background: @error_bg_color; color: @error_fg_color; box-shadow: 0 0 0 4px alpha(@error_bg_color, 0.22); }
.stave-pin.warning { background: @warning_bg_color; color: @warning_fg_color; box-shadow: 0 0 0 4px alpha(@warning_bg_color, 0.25); }
.stave-pin-label { color: @error_color; }
.stave-pin-label.warning { color: @warning_fg_color; }
.stave-edge-pin { color: @view_fg_color; }
.stave-finder { background: @view_bg_color; color: @view_fg_color; box-shadow: 0 0 0 1px alpha(black, 0.1), 0 20px 50px alpha(black, 0.3); }
.stave-finder entry { background: transparent; box-shadow: inset 0 -1px alpha(currentColor, 0.1); }
.stave-finder-filters togglebutton { background: alpha(currentColor, 0.07); color: inherit; }
.stave-finder-filters togglebutton:checked { background: @view_fg_color; color: @view_bg_color; }
.stave-finder-list > row:selected { background: @accent_bg_color; color: @accent_fg_color; }
.stave-finder-list > row:selected .stave-detail { color: inherit; opacity: 0.9; }
.stave-finder-heading { color: alpha(currentColor, 0.7); }
.stave-finder-heading.problems { color: @error_color; }
.stave-finder-foot { background: alpha(currentColor, 0.04); color: alpha(currentColor, 0.7); }
.stave-scrim { background: alpha(black, 0.28); }
"#;

/// The snippet's colours: keyword, type or call, number, string, comment.
pub struct CodeColours {
    /// `assembly`.
    pub keyword: &'static str,
    /// `FramePanelDoor`, `overlay(`.
    pub name: &'static str,
    /// `18mm`.
    pub number: &'static str,
    /// Strings.
    pub text: &'static str,
    /// Comments, and a diagnostic's arrows and gutter.
    pub comment: &'static str,
    /// An error's `error[code]` and carets.
    pub error: &'static str,
    /// A warning's `warning[code]`.
    pub warning: &'static str,
}

/// The snippet's colours on its dark background (#1e1e24 light, #121216
/// dark): each at least 4.5:1 against both (tested below).
pub const CODE: CodeColours = CodeColours {
    keyword: "#f0a3c4",
    name: "#8fd0ff",
    number: "#ffcf80",
    text: "#8ff0a4",
    comment: "#8b8a93",
    error: "#ff7b85",
    warning: "#ffcf80",
};

/// The code blocks' backgrounds as `COLOURS` writes them: light style,
/// dark style.
/// Gives a button drawn as a link (the `stave-link` and `stave-lens-link`
/// classes) the hand cursor, so it reads as clickable; inside the text
/// view it would otherwise keep the text cursor.
pub fn as_link(button: gtk::Button) -> gtk::Button {
    use gtk::prelude::WidgetExt;
    button.set_cursor_from_name(Some("pointer"));
    button
}

/// The window shell (compact H2): the 34 px header with its menu bar,
/// 26 px tab strips with the open files beside the Model, the status bar
/// with Problems and Output folded into it, and the tool groups' drop
/// outlines. Loaded after `SHARED` and `COLOURS`, so these win.
const SHELL: &str = r#"
/* The Files panel: 24 px rows, like the Parts panel's. */
.stave-panel-list.stave-files-list > row { min-height: 24px; padding: 2px 10px; margin: 0 4px; font-size: 0.852rem; }
.stave-panel-list.stave-files-list > row image { -gtk-icon-size: 14px; }
headerbar.stave-header { min-height: 34px; }
.stave-header > windowhandle > box { padding: 0 4px; min-height: 34px; }
.stave-header > windowhandle > box > box { border-spacing: 4px; }
.stave-header windowcontrols button { min-width: 22px; min-height: 22px; padding: 2px; margin: 0; }
.stave-header button.stave-icon-button { min-width: 26px; min-height: 26px; border-radius: 6px; }
.stave-header popovermenubar.stave-menu-bar { min-height: 26px; }
.stave-menu-bar, .stave-menu-bar label { font-size: 0.852rem; }
.stave-header popovermenubar.stave-menu-bar > item { min-height: 24px; padding: 0 8px; border-radius: 6px; }
.stave-header popovermenubar.stave-menu-bar > item:hover, .stave-header popovermenubar.stave-menu-bar > item:selected { background: alpha(currentColor, 0.08); }
.stave-header .stave-switcher { padding: 2px; border-radius: 8px; font-size: 0.818rem; }
.stave-header .stave-switcher > toggle { min-height: 22px; padding: 0 12px; border-radius: 6px; }
.stave-header .stave-switcher > toggle box { border-spacing: 5px; }
.stave-header .stave-switcher.compact > toggle { padding: 0 8px; }
.stave-header button.stave-pill { min-height: 24px; padding: 0 10px; border-radius: 12px; font-size: 0.818rem; }
.stave-header button.stave-problem-step { min-width: 22px; min-height: 22px; }
.stave-header button.stave-build { min-width: 30px; min-height: 26px; border-radius: 6px; }

stavetabstrip { min-height: 26px; }
stavetabstrip button.stave-tab-show { min-height: 26px; font-size: 0.818rem; }
stavetabstrip .stave-tab-end button { min-height: 20px; min-width: 20px; margin: 3px 4px; }
stavetabstrip button.stave-file-tab { padding-right: 10px; }
.stave-file-tab-name { font-family: var(--monospace-font-family, monospace); font-weight: 500; font-size: 0.784rem; }
.stave-mark.unsaved { min-width: 7px; min-height: 7px; border-radius: 4px; background: @warning_bg_color; }

panelstatusbar { min-height: 24px; font-size: 0.818rem; }
panelstatusbar button, panelstatusbar menubutton > button { min-height: 22px; padding: 0 8px; border-radius: 0; }
button.stave-status-tab { min-height: 22px; padding: 0 10px; border-radius: 0; font-size: 0.818rem; font-weight: 600; color: alpha(currentColor, 0.75); }
button.stave-status-tab:hover { color: currentColor; background: alpha(currentColor, 0.06); }
button.stave-status-tab.shown { color: currentColor; background: alpha(currentColor, 0.08); box-shadow: inset 0 2px @accent_bg_color; }
.stave-status-count { min-width: 16px; padding: 0 5px; border-radius: 8px; font-size: 0.75rem; background: alpha(@success_color, 0.18); color: @success_color; }
.stave-status-count.some { background: alpha(@warning_color, 0.2); color: @warning_color; }
.stave-status-text { font-size: 0.784rem; color: alpha(currentColor, 0.7); }
textview.stave-output { font-size: 0.818rem; background: transparent; }

/* A panel's empty state ("No problems", "No files", "No file open"): every
   one is a compact status page, and libadwaita's compact page (a 96 px
   icon, 24 px margins and gaps, about 230 px tall) is taller than the
   bottom dock's Problems panel (about 190 px at 1440x900), so its
   description was cut off at the panel's edge. A 48 px icon and 12 px
   margins and gaps make it about 130 px, with room for a second line of
   description and Large Text. In a still shorter frame the page drops
   its icon and shows a real scrollbar (empty_state.rs), and a shadow
   marks the edge where the words go on (GTK draws a scrolled window's
   undershoot only on a side with more to scroll to). */
statuspage.compact > scrolledwindow > viewport > box { margin: 12px; border-spacing: 12px; }
statuspage.compact > scrolledwindow > viewport > box > clamp > box { border-spacing: 12px; }
statuspage.compact > scrolledwindow > viewport > box > clamp > box > .icon { -gtk-icon-size: 48px; }
statuspage.compact > scrolledwindow > viewport > box > clamp > box > box { border-spacing: 6px; }
statuspage.compact > scrolledwindow > viewport > box > clamp > box > box > .title { font-size: 1.18rem; }
statuspage.compact > scrolledwindow > undershoot.top { box-shadow: inset 0 1px color-mix(in srgb, var(--shade-color) 75%, transparent); background: linear-gradient(to bottom, color-mix(in srgb, var(--shade-color) 75%, transparent), transparent 8px); }
statuspage.compact > scrolledwindow > undershoot.bottom { box-shadow: inset 0 -1px color-mix(in srgb, var(--shade-color) 75%, transparent); background: linear-gradient(to top, color-mix(in srgb, var(--shade-color) 75%, transparent), transparent 8px); }

.stave-dock-targets { color: @accent_color; }
.stave-dragging { opacity: 0.85; box-shadow: 0 6px 18px alpha(black, 0.35); }
.stave-camera.flat-row, .stave-camera.flat-row.joined { border-radius: 10px; }
"#;

/// High contrast (libadwaita's high-contrast style, or the desktop's
/// "high contrast" setting): the design's dimmed words take the full text
/// colour, its translucent floating cards turn solid with an edge, and the
/// faint lines between rows and round code blocks darken. A provider of
/// its own, added after the other sheets while the style manager says
/// high contrast and taken away when it stops.
const HIGH_CONTRAST: &str = r#"
.stave-title .subtitle, .dim,
.stave-tree .stave-detail, .stave-tree .stave-heading,
.stave-eyebrow, .stave-description, .stave-section, .stave-footnote, .stave-key,
.stave-crumbs, .stave-editor-footer, .stave-lens,
.stave-peek-what, .stave-peek-number,
.stave-hover-kind, .stave-hover-where, .stave-hover-help,
.stave-use-line, .stave-tool-end, .stave-card-dim,
.stave-problem-icon.note, .stave-problem-note,
.stave-dock .stave-dock-label, button.stave-code-tab,
.stave-finder-heading, .stave-finder-foot,
button.stave-status-tab, .stave-status-text, stavetabstrip .stave-tab,
.stave-problems-chip.unknown button.stave-pill { color: inherit; }
.stave-empty-icon, .stave-empty-description { opacity: 1; }
.stave-floating, .stave-chip, .stave-toolbar, .stave-plan-switch, .stave-camera,
.stave-moves, .stave-focus-card, .stave-panel, .stave-edge-pin, .stave-pin-label,
.stave-contact-key, .stave-pictures button.stave-picture {
  background: @view_bg_color; box-shadow: 0 0 0 1px @view_fg_color;
}
.stave-card > .stave-card-row { border-bottom-color: alpha(currentColor, 0.5); }
.stave-crumbs, .stave-editor-footer { box-shadow: none; }
.stave-code, .stave-dark .stave-code { box-shadow: inset 0 0 0 1px @window_fg_color; }
.stave-panel-list > row:selected { background: @accent_bg_color; color: @accent_fg_color; }
.stave-dock button.stave-move { box-shadow: inset 0 0 0 1px alpha(currentColor, 0.6); }
"#;

#[cfg(test)]
const CODE_BACKGROUNDS: [&str; 2] = ["#1e1e24", "#121216"];

/// Shows a problem mark on a dot: red for an error, amber for a warning, a
/// red ring for a part a problem only involves; hidden for none.
pub fn show_mark(dot: &gtk::Box, mark: Option<stave_app::problems::Mark>) {
    use gtk::prelude::*;
    use stave_app::problems::Mark;
    for class in ["error", "warning", "involved"] {
        dot.remove_css_class(class);
    }
    let class = match mark {
        Some(Mark::Error) => "error",
        Some(Mark::Warning) => "warning",
        Some(Mark::Involved) => "involved",
        None => {
            dot.set_visible(false);
            return;
        }
    };
    dot.add_css_class(class);
    dot.set_visible(true);
}

/// A dot for a problem mark (hidden for none).
pub fn mark_dot(mark: Option<stave_app::problems::Mark>) -> gtk::Box {
    let dot = gtk::Box::builder()
        .css_classes(["stave-mark"])
        .valign(gtk::Align::Center)
        .build();
    show_mark(&dot, mark);
    dot
}

/// Adds the high-contrast sheet to `display` while `on`, takes it away
/// otherwise.
fn show_high_contrast(display: &gdk::Display, provider: &gtk::CssProvider, on: bool) {
    // Removing first means adding twice never stacks two copies.
    gtk::style_context_remove_provider_for_display(display, provider);
    if on {
        gtk::style_context_add_provider_for_display(
            display,
            provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Installs the window's style for `display`. Colours are libadwaita's
/// named colours (the 3D view's stage and the always-dark code blocks
/// aside), so nothing needs swapping when the style turns light, dark or
/// high contrast.
pub fn install(display: &gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!("{SHARED}{COLOURS}{SHELL}"));
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let high_contrast = gtk::CssProvider::new();
    high_contrast.load_from_string(HIGH_CONTRAST);
    let manager = adw::StyleManager::for_display(display);
    show_high_contrast(display, &high_contrast, manager.is_high_contrast());
    let shown = display.clone();
    manager.connect_high_contrast_notify(move |manager| {
        show_high_contrast(&shown, &high_contrast, manager.is_high_contrast());
    });
    // A selectable label (a problem's `stave check` text, focus mode's
    // insert line) would otherwise select all its text whenever it takes
    // the keyboard focus, so it looks highlighted for no reason. It can
    // still be selected with the mouse to copy.
    gtk::Settings::for_display(display).set_gtk_label_select_on_focus(false);
}

#[cfg(test)]
mod tests {
    use super::{CODE, CODE_BACKGROUNDS, COLOURS, HIGH_CONTRAST, SHARED, SHELL};

    /// The relative luminance of `#rrggbb` (WCAG 2).
    fn luminance(hex: &str) -> f64 {
        let channel = |range: std::ops::Range<usize>| {
            let value = f64::from(u8::from_str_radix(&hex[range], 16).unwrap()) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1..3) + 0.7152 * channel(3..5) + 0.0722 * channel(5..7)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn code_colours_read_on_both_code_backgrounds() {
        for background in CODE_BACKGROUNDS {
            assert!(COLOURS.contains(&format!("background: {background}")));
            for colour in [
                "#e7e5ea",
                CODE.keyword,
                CODE.name,
                CODE.number,
                CODE.text,
                CODE.comment,
                CODE.error,
                CODE.warning,
            ] {
                let ratio = contrast(colour, background);
                assert!(ratio >= 4.5, "{colour} on {background}: {ratio:.2}:1");
            }
        }
    }

    /// The header puts its tone class (`errors`, `warnings`...) on the
    /// chip round the pill, so the pill's colours must be keyed on the
    /// chip: keyed on the pill itself they never apply and an error shows
    /// grey.
    #[test]
    fn the_problems_pill_takes_its_colour_from_the_chip() {
        for tone in ["clean", "warnings", "errors", "unknown"] {
            assert!(
                COLOURS.contains(&format!(".stave-problems-chip.{tone} button.stave-pill {{")),
                "{tone}"
            );
            assert!(
                !COLOURS.contains(&format!("button.stave-pill.{tone}")),
                "{tone}"
            );
        }
    }

    /// The `#rrggbb` colours written in `sheet`.
    fn hex_colours(sheet: &str) -> Vec<&str> {
        sheet
            .match_indices('#')
            .filter_map(|(at, _)| sheet.get(at..at + 7))
            .filter(|colour| colour[1..].chars().all(|c| c.is_ascii_hexdigit()))
            .collect()
    }

    /// Every colour follows libadwaita's style (light, dark, high
    /// contrast) except the 3D view's stage, which must match the picture
    /// the renderer draws, and the code blocks, dark in every style with
    /// colours tested above.
    #[test]
    fn colours_are_named_except_the_stage_and_the_code_blocks() {
        let fixed = ["#e8e5df", "#242428", "#1e1e24", "#121216", "#e7e5ea"];
        let css = |sheet: &'static str| {
            // Comments may name colours.
            let mut rules = String::new();
            for (index, part) in sheet.split("/*").enumerate() {
                rules.push_str(if index == 0 {
                    part
                } else {
                    part.split_once("*/").map_or("", |(_, rest)| rest)
                });
            }
            rules
        };
        for sheet in [SHARED, COLOURS, SHELL, HIGH_CONTRAST] {
            let rules = css(sheet);
            for colour in hex_colours(&rules) {
                assert!(fixed.contains(&colour), "{colour} is not a named colour");
            }
        }
    }

    /// Every sheet parses in the GTK this runs on: a rule GTK can't read
    /// is dropped without a word at run time.
    #[test]
    fn the_sheets_parse() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            for sheet in [
                format!("{SHARED}{COLOURS}{SHELL}"),
                HIGH_CONTRAST.to_owned(),
            ] {
                let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                let provider = gtk::CssProvider::new();
                let seen = errors.clone();
                provider.connect_parsing_error(move |_, section, error| {
                    seen.borrow_mut().push(format!(
                        "line {}: {error}",
                        section.start_location().lines() + 1
                    ));
                });
                provider.load_from_string(&sheet);
                assert_eq!(*errors.borrow(), Vec::<String>::new());
            }
        });
    }

    /// In high contrast a dimmed detail takes the full text colour.
    #[test]
    fn high_contrast_undims_the_details() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            use gtk::prelude::*;
            let Some(display) = gtk::gdk::Display::default() else {
                return;
            };
            let detail = gtk::Label::builder()
                .label("18 mm")
                .css_classes(["stave-detail"])
                .build();
            let tree = gtk::Box::builder().css_classes(["stave-tree"]).build();
            tree.append(&detail);
            let window = gtk::Window::builder().child(&tree).build();
            let sheets = gtk::CssProvider::new();
            sheets.load_from_string(&format!("{SHARED}{COLOURS}{SHELL}"));
            gtk::style_context_add_provider_for_display(
                &display,
                &sheets,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            let high_contrast = gtk::CssProvider::new();
            high_contrast.load_from_string(HIGH_CONTRAST);
            window.present();
            let alpha = |on: bool| {
                super::show_high_contrast(&display, &high_contrast, on);
                // The style is worked out again on the next frame.
                let context = gtk::glib::MainContext::default();
                for _ in 0..50 {
                    context.iteration(false);
                }
                detail.color().alpha()
            };
            let normal = alpha(false);
            let high = alpha(true);
            super::show_high_contrast(&display, &high_contrast, false);
            gtk::style_context_remove_provider_for_display(&display, &sheets);
            window.destroy();
            assert!(normal < high, "{normal} then {high}");
        });
    }

    /// Text sizes follow the interface font (and so Large Text) and the
    /// fonts are the user's own: no size in pixels, no font named.
    #[test]
    fn text_sizes_are_relative_and_fonts_unnamed() {
        for sheet in [SHARED, COLOURS, SHELL, HIGH_CONTRAST] {
            for rule in sheet.split(';') {
                let rule = rule.trim();
                if let Some(size) = rule
                    .rsplit("font-size:")
                    .next()
                    .filter(|_| rule.contains("font-size:"))
                {
                    assert!(!size.trim().ends_with("px"), "{rule}");
                }
                assert!(!rule.contains("font-family: \""), "{rule}");
            }
        }
    }

    /// A label sized by the sheet grows when Large Text raises the
    /// display's resolution, as libadwaita's own text does.
    #[test]
    fn text_grows_with_large_text() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            use gtk::prelude::*;
            let Some(display) = gtk::gdk::Display::default() else {
                return;
            };
            let settings = gtk::Settings::for_display(&display);
            let dpi = settings.gtk_xft_dpi();
            let provider = gtk::CssProvider::new();
            provider.load_from_string(SHARED);
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            let size = |scaled: i32| {
                settings.set_gtk_xft_dpi(scaled);
                let label = gtk::Label::builder()
                    .label("Shelf")
                    .css_classes(["stave-status-text"])
                    .build();
                let window = gtk::Window::builder().child(&label).build();
                let size = label
                    .pango_context()
                    .font_description()
                    .map_or(0, |font| font.size());
                window.destroy();
                size
            };
            let normal = size(96 * 1024);
            let large = size(120 * 1024);
            settings.set_gtk_xft_dpi(dpi);
            gtk::style_context_remove_provider_for_display(&display, &provider);
            assert!(large > normal, "{normal} then {large}");
        });
    }

    /// The Problems panel's empty state fits the room the bottom dock
    /// gives it at the default 1440x900 window (about 194 px under its
    /// tab strip), whole: icon, title and description, at the panel's
    /// width there and in a 760 px window (where the description wraps),
    /// with normal and Large Text. libadwaita's own compact page needs
    /// about 230 px and lost its description at the panel's edge.
    #[test]
    fn the_empty_problems_panel_fits_the_bottom_dock() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            use gtk::prelude::*;
            let Some(display) = gtk::gdk::Display::default() else {
                return;
            };
            const ROOM: i32 = 190;
            let settings = gtk::Settings::for_display(&display);
            let (dpi, font) = (settings.gtk_xft_dpi(), settings.gtk_font_name());
            settings.set_gtk_font_name(Some("Adwaita Sans 11"));
            let sheets = gtk::CssProvider::new();
            sheets.load_from_string(&format!("{SHARED}{COLOURS}{SHELL}"));
            gtk::style_context_add_provider_for_display(
                &display,
                &sheets,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            let page = adw::StatusPage::builder()
                .icon_name("object-select-symbolic")
                .title("No problems")
                .description(
                    "Every part fits, every joint closes and everything that moves moves clear.",
                )
                .css_classes(["compact"])
                .build();
            let window = gtk::Window::builder().child(&page).build();
            window.present();
            let mut heights = Vec::new();
            for scaled in [96 * 1024, 120 * 1024] {
                settings.set_gtk_xft_dpi(scaled);
                crate::ui::gtk_test::settle();
                // The panel's width in a 1440 and a 760 px window.
                for width in [1166, 540] {
                    let (_, natural, _, _) = page.measure(gtk::Orientation::Vertical, width);
                    heights.push((scaled / 1024, width, natural));
                }
            }
            settings.set_gtk_xft_dpi(dpi);
            settings.set_gtk_font_name(font.as_deref());
            window.destroy();
            gtk::style_context_remove_provider_for_display(&display, &sheets);
            assert!(
                heights.iter().all(|&(_, _, natural)| natural <= ROOM),
                "(dpi, width, height): {heights:?}"
            );
        });
    }
}
