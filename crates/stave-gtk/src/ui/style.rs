//! The window's look: libadwaita's, with the design's own sizes and
//! colours for the title bar's switcher, pill and build button, the parts
//! tree, the inspector's cards and the status bar. Colours come in a light
//! and a dark set, swapped when the style changes.

use gtk::gdk;

/// Sizes and shapes, the same in light and dark.
const SHARED: &str = r#"
.stave-header { min-height: 50px; }
.stave-header > windowhandle > box { padding: 0 8px; }
.stave-header button.stave-icon-button { min-width: 34px; min-height: 34px; padding: 0; border-radius: 8px; }
.stave-header button.stave-sidebar-toggle { min-width: 36px; min-height: 36px; }
.stave-title { padding: 0 6px; }
.stave-title .title { font-weight: 700; font-size: 14px; }
.stave-title .subtitle { font-size: 12px; }

.stave-switcher { padding: 3px; border-radius: 10px; font-size: 13px; font-weight: 600; }
.stave-switcher > toggle { min-height: 32px; padding: 0 16px; border-radius: 8px; }
.stave-switcher > toggle box { border-spacing: 6px; }
.stave-switcher.compact > toggle { padding: 0 10px; }
.stave-switcher > separator { background: none; min-width: 0; margin: 0; }

menubutton.stave-scene > button { min-height: 34px; padding: 0 8px 0 10px; border-radius: 8px; font-size: 13px; font-weight: 600; }
menubutton.stave-scene > button > box { border-spacing: 6px; }

button.stave-pill { min-height: 34px; padding: 0 12px; border-radius: 17px; font-size: 13px; font-weight: 600; }
button.stave-pill box { border-spacing: 6px; }
button.stave-pill.open { outline: 2px solid currentColor; outline-offset: -2px; }

button.stave-build { min-width: 36px; min-height: 34px; padding: 0; border-radius: 8px; }

.stave-sidebar { padding: 12px 10px; }
.stave-tree { background: none; font-size: 13px; }
.stave-tree > row { padding: 0; border-radius: 6px; min-height: 0; margin: 0 0 1px 0; }
.stave-tree .stave-entry { padding: 5px 8px 5px 20px; border-spacing: 8px; }
.stave-tree .stave-entry.nested { padding-left: 34px; }
.stave-tree .stave-entry.nested2 { padding-left: 48px; }
.stave-tree .stave-assembly { padding: 5px 8px; border-spacing: 8px; font-weight: 600; }
.stave-tree .stave-assembly.nested { padding-left: 22px; }
.stave-tree .stave-detail { font-size: 12px; }
.stave-tree .stave-assembly .stave-detail { font-weight: 500; }
.stave-tree .stave-heading { padding: 10px 8px 6px 8px; font-size: 12px; font-weight: 700; letter-spacing: 0.02em; }
.stave-tree .stave-heading.first { padding-top: 6px; }
.stave-tree .stave-library-entry { padding: 4px 8px 5px 12px; border-spacing: 1px; }
.stave-tree .stave-sidebar-note { padding: 4px 8px 0 8px; font-size: 13px; }

.stave-inspector { padding: 18px 16px; }
.stave-inspector-panel { min-width: 0; }
.stave-eyebrow { font-size: 12px; font-weight: 600; }
.stave-inspector-title { font-size: 20px; font-weight: 700; }
.stave-description { font-size: 13px; }
.stave-card { border-radius: 12px; font-size: 13px; }
.stave-card > .stave-card-row { padding: 11px 14px; border-spacing: 12px; }
.stave-card > button.stave-card-row { padding: 10px 14px; border-radius: 0; min-height: 0; font-weight: normal; }
.stave-card > :first-child { border-top-left-radius: 12px; border-top-right-radius: 12px; }
.stave-card > :last-child { border-bottom-left-radius: 12px; border-bottom-right-radius: 12px; border-bottom: none; }
.stave-card .stave-value { font-weight: 600; }
.stave-section { font-size: 12px; font-weight: 700; letter-spacing: 0.02em; }
.stave-link { font-size: 12px; font-weight: 600; padding: 0 2px; min-height: 0; }
.stave-code { padding: 12px 14px; border-radius: 12px; font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 12px; }
.stave-footnote { font-size: 12px; }


.stave-floating { border-radius: 12px; padding: 8px 14px; font-size: 13px; border-spacing: 12px; }
.stave-placeholder { font-size: 13px; }

.stave-crumbs { min-height: 30px; padding: 0 16px; font-size: 12px; }
.stave-crumb.current { font-weight: 600; }
textview.stave-editor { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 13px; }
.stave-editor-footer { min-height: 28px; padding: 0 16px; font-size: 12px; }
.stave-lens { font-family: "Adwaita Sans", "Inter", sans-serif; font-size: 11.5px; border-spacing: 2px; }
button.stave-lens-link { min-height: 0; min-width: 0; padding: 0 2px; font-weight: normal; border-radius: 4px; }
.stave-peek { border-radius: 10px; margin-top: 2px; }
.stave-peek-head { padding: 7px 12px; font-family: "Adwaita Sans", "Inter", sans-serif; font-size: 12px; border-radius: 10px 10px 0 0; }
.stave-peek-path { font-weight: 700; }
.stave-peek-body { padding: 6px 0; }
.stave-peek-line { min-height: 21px; }
.stave-peek-number { padding-right: 12px; font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 12.5px; }
.stave-peek-code { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 12.5px; }
.stave-hover { border-radius: 10px; padding: 10px 12px; font-family: "Adwaita Sans", "Inter", sans-serif; font-size: 13px; }
.stave-hover-kind { font-size: 11px; font-weight: 700; letter-spacing: 0.04em; }
.stave-hover-signature { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 12.5px; font-weight: 600; }
.stave-hover-where, .stave-hover-help { font-size: 12px; }
.stave-uses-head { padding: 4px 8px 4px 16px; font-size: 13px; }
.stave-uses-list { background: none; }
.stave-uses-list > row { padding: 0; }
.stave-use { padding: 6px 16px; font-size: 12.5px; }
.stave-use-place { font-weight: 600; }
.stave-use-line { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 12px; }
.stave-chip { border-radius: 16px; padding: 6px 12px 6px 8px; font-size: 12px; font-weight: 600; }
.stave-stage.page { padding: 16px; border-spacing: 14px; }
.stave-empty-title { font-size: 20px; font-weight: 700; }
.stave-empty-description { font-size: 14px; }
.stave-stage.page .stave-figure { border-radius: 14px; }
.stave-toolbar { padding: 4px; border-radius: 12px; border-spacing: 4px; }
.stave-toolbar button.stave-tool { min-height: 32px; padding: 0 12px; border-radius: 8px; font-size: 13px; font-weight: 600; }
.stave-toolbar button.stave-tool box { border-spacing: 6px; }
.stave-toolbar .stave-tool-separator { min-width: 1px; margin: 6px 0; }
.stave-toolbar button.stave-tool-play { min-width: 32px; min-height: 32px; padding: 0; }
.stave-tool-slider { padding: 4px 10px 2px 2px; }
.stave-tool-end { font-size: 11px; }
.stave-camera { padding: 4px; border-radius: 10px; }
.stave-camera button { min-width: 32px; min-height: 32px; padding: 0; border-radius: 7px; }
.stave-contact-key { padding: 6px 12px; border-radius: 14px; font-size: 12px; font-weight: 600; }
.stave-plan-switch { padding: 3px; border-radius: 10px; }
.stave-plan-switch button { min-width: 34px; min-height: 30px; padding: 0; border-radius: 7px; }
.stave-focus-card { padding: 12px 16px; border-radius: 14px; font-size: 13px; }
.stave-focus-card.stave-problem-card { padding: 14px 18px 12px 18px; }
button.stave-play { min-width: 40px; min-height: 40px; padding: 0; }
.stave-card-dim { font-size: 12px; }
.stave-card-value { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 13px; font-weight: 500; }
.stave-card-blocked { font-size: 12px; font-weight: 600; }
.stave-card-name { font-size: 13px; font-weight: 600; }
.stave-focus-caption { font-size: 13px; }
.stave-focus-caption .stave-card-dim { font-size: 13px; }
.stave-pictures button.stave-picture { padding: 10px; border-radius: 12px; font-weight: normal; }
.stave-picture-caption { font-size: 12px; }

button.stave-close { min-width: 28px; min-height: 28px; padding: 0; }
.stave-problem-assembly { padding: 5px 8px; font-weight: 600; }
.stave-problem-assembly.depth1 { padding-left: 22px; }
.stave-problem-assembly.depth2 { padding-left: 36px; }
.stave-problem-assembly .stave-detail { font-size: 12px; font-weight: 500; }
.stave-problem-part { padding: 5px 8px 5px 20px; }
.stave-problem-part.depth1 { padding-left: 30px; }
.stave-problem-part.depth2 { padding-left: 44px; }
.stave-problem-part .stave-detail { font-size: 12px; }
.stave-problem-kind { font-weight: 600; }
.stave-problem-places { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 11.5px; }
.stave-eyebrow.stave-eyebrow-bold, .stave-eyebrow.stave-problem-eyebrow { font-weight: 700; letter-spacing: 0.02em; }
.stave-problem-title { font-size: 19px; }
.stave-code.stave-diagnostic { padding: 14px; font-size: 11.5px; }
.stave-number { min-width: 22px; min-height: 22px; border-radius: 11px; font-size: 12px; font-weight: 700; }
.stave-card > .stave-card-row.stave-toggle-row { padding: 10px 14px; }
.stave-card > .stave-card-row.compact { padding: 8px 14px; }
.stave-card > .stave-card-row.compact .stave-number { min-width: 20px; min-height: 20px; border-radius: 10px; font-size: 11px; }
expander.stave-more title { font-size: 13px; font-weight: 600; }
expander.stave-more title > arrow { min-width: 12px; min-height: 12px; }

.stave-panel { border-radius: 14px; }
.stave-tree .stave-joint { padding: 4px 8px 4px 34px; border-spacing: 8px; font-size: 12.5px; }
.stave-joint-word { font-size: 10px; font-weight: 700; padding: 1px 4px; border-radius: 3px; }
.stave-mark { min-width: 8px; min-height: 8px; border-radius: 4px; }
.stave-mark.warning { border-radius: 2px; }
.stave-card-panel { border-radius: 16px; }
.stave-card-panel .stave-inspector { padding: 14px 16px; }
.stave-dock { padding: 6px; border-radius: 14px; border-spacing: 6px; font-size: 13px; }
.stave-dock button.stave-tool { min-height: 34px; padding: 0 12px; border-radius: 9px; font-weight: 600; }
.stave-dock .stave-dock-separator { min-width: 1px; margin: 6px 2px; }
.stave-dock .stave-dock-label { font-size: 12px; padding: 0 4px; }
.stave-dock button.stave-move { min-height: 30px; padding: 0 10px; border-radius: 15px; font-size: 12px; font-weight: 600; }
.stave-rail { padding: 4px; border-radius: 12px 0 0 12px; border-spacing: 4px; min-width: 36px; }
.stave-rail button.stave-rail-button { min-width: 36px; min-height: 36px; padding: 0; border-radius: 8px; }
.stave-rail button.stave-rail-tab { min-width: 36px; padding: 12px 0; border-radius: 8px; font-size: 12.5px; font-weight: 500; }
.stave-rail button.stave-rail-tab.active { font-weight: 700; }
.stave-rail-name { font-family: "JetBrains Mono", "Adwaita Mono", monospace; }
.stave-editor-panel { min-width: 420px; }
.stave-editor-head { min-height: 44px; padding: 0 8px 0 16px; }
.stave-editor-title { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 13px; font-weight: 600; }
button.stave-problem-step { min-width: 28px; min-height: 28px; padding: 0; border-radius: 14px; }
.stave-pin { min-width: 24px; min-height: 24px; padding: 0; border-radius: 12px 12px 12px 2px; font-size: 12px; font-weight: 700; }
.stave-pin.warning { border-radius: 4px; }
.stave-pin-label { padding: 3px 8px; border-radius: 8px; font-size: 12px; font-weight: 600; }
.stave-edge-pin { min-height: 32px; padding: 0 12px 0 10px; border-radius: 16px; font-size: 12px; font-weight: 600; }
.stave-edge-pin box { border-spacing: 6px; }
.stave-finder { border-radius: 16px; }
.stave-finder entry { min-height: 52px; font-size: 17px; border-radius: 16px 16px 0 0; }
.stave-finder-filters { padding: 10px 16px 6px 16px; border-spacing: 6px; font-size: 12px; font-weight: 600; }
.stave-finder-filters togglebutton { min-height: 28px; padding: 0 10px; border-radius: 14px; }
.stave-finder-list { background: none; padding: 4px 8px 8px 8px; }
.stave-finder-list > row { border-radius: 9px; min-height: 36px; padding: 0 10px; }
.stave-finder-heading { padding: 10px 8px 4px 8px; font-size: 11.5px; font-weight: 700; letter-spacing: 0.03em; }
.stave-finder-hit { border-spacing: 10px; font-size: 13.5px; }
.stave-finder-hit .stave-detail { font-size: 12px; }
.stave-finder-line { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 12.5px; }
.stave-finder-foot { padding: 10px 16px; font-size: 12px; border-spacing: 16px; border-radius: 0 0 16px 16px; }
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
.stave-header button.stave-icon-button:disabled { color: alpha(currentColor, 0.45); }
.stave-header button.stave-sidebar-toggle:checked { background: alpha(currentColor, 0.1); }
.stave-title .subtitle, .dim { color: alpha(currentColor, 0.7); }
menubutton.stave-scene > button { background: alpha(currentColor, 0.08); }
menubutton.stave-scene > button:hover { background: alpha(currentColor, 0.12); }
button.stave-pill.clean { background: alpha(@success_color, 0.15); color: @success_color; }
button.stave-pill.warnings { background: alpha(@warning_color, 0.18); color: @warning_color; }
button.stave-pill.errors { background: alpha(@error_color, 0.14); color: @error_color; }
button.stave-pill.unknown { background: alpha(currentColor, 0.08); color: alpha(currentColor, 0.7); }
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
.stave-stage { background: #e8e5df; }
.stave-stage.dark { background: #242428; }
.stave-empty { color: #3d3846; }
.stave-empty.dark { color: alpha(white, 0.9); }
.stave-empty-icon, .stave-empty-description { opacity: 0.75; }
.stave-toolbar, .stave-plan-switch { background: alpha(@view_bg_color, 0.92); color: @view_fg_color; box-shadow: 0 1px 6px alpha(black, 0.1); }
.stave-camera { background: alpha(@view_bg_color, 0.88); color: @view_fg_color; }
.stave-toolbar button.stave-tool, .stave-plan-switch button { background: transparent; color: inherit; }
.stave-toolbar button.stave-tool:hover, .stave-plan-switch button:hover { background: alpha(currentColor, 0.07); }
.stave-toolbar button.stave-tool:checked, .stave-plan-switch button:checked { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-toolbar .stave-tool-separator { background: alpha(currentColor, 0.15); }
.stave-toolbar button.stave-tool-play { color: @accent_color; }
.stave-slider { color: @accent_bg_color; }
.stave-tool-end, .stave-card-dim { color: alpha(currentColor, 0.7); }
.stave-contact-key { background: alpha(@view_bg_color, 0.9); color: @error_color; }
.stave-focus-card { background: alpha(@view_bg_color, 0.95); color: @view_fg_color; box-shadow: 0 4px 16px alpha(black, 0.12); }
button.stave-play { background: @accent_bg_color; color: @accent_fg_color; }
button.stave-play:hover { background: mix(@accent_bg_color, @accent_fg_color, 0.1); }
.stave-card-value.blocked, .stave-card-blocked { color: @error_color; }
.stave-focus-caption { color: #3d3846; }
.stave-stage.dark .stave-focus-caption { color: alpha(white, 0.85); }
.stave-pictures button.stave-picture { background: alpha(@view_bg_color, 0.45); color: @view_fg_color; }
.stave-pictures button.stave-picture:hover { background: alpha(@view_bg_color, 0.7); }
.stave-pictures button.stave-picture.current { background: @view_bg_color; box-shadow: 0 0 0 2px @accent_bg_color; }

.stave-problem-icon.error { color: @error_color; }
.stave-problem-icon.warning { color: @warning_color; }
.stave-problem-icon.note { color: alpha(currentColor, 0.7); }
.stave-problem-places { color: @accent_color; }
.stave-eyebrow.stave-problem-eyebrow.error { color: @error_color; }
.stave-eyebrow.stave-problem-eyebrow.warning { color: @warning_color; }
.stave-number { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-card .stave-value.good { color: @success_color; }
.stave-card .stave-value.bad { color: @error_color; }

.stave-panel, .stave-dock, .stave-rail, .stave-edge-pin, .stave-pin-label { background: alpha(@view_bg_color, 0.95); color: @view_fg_color; box-shadow: 0 2px 12px alpha(black, 0.12); }
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
.stave-rail { box-shadow: -2px 0 12px alpha(black, 0.14); }
.stave-rail button.stave-rail-tab { background: transparent; color: alpha(currentColor, 0.8); }
.stave-rail button.stave-rail-tab:hover { background: alpha(currentColor, 0.07); }
.stave-rail button.stave-rail-tab.active { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-editor-panel { background: @view_bg_color; box-shadow: -8px 0 28px alpha(black, 0.16); }
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

/// Installs the window's style for `display`. Every colour is one of
/// libadwaita's named colours, so nothing needs swapping when the style
/// turns light or dark.
pub fn install(display: &gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!("{SHARED}{COLOURS}"));
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    // A selectable label (a problem's `stave check` text, focus mode's
    // insert line) would otherwise select all its text whenever it takes
    // the keyboard focus, so it looks highlighted for no reason. It can
    // still be selected with the mouse to copy.
    gtk::Settings::for_display(display).set_gtk_label_select_on_focus(false);
}

#[cfg(test)]
mod tests {
    use super::{CODE, CODE_BACKGROUNDS, COLOURS};

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
}
