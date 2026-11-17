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
.stave-sidebar entry.stave-search { min-height: 34px; border-radius: 8px; font-size: 13px; }
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

.stave-statusbar { min-height: 30px; padding: 0 14px; font-size: 12px; border-spacing: 16px; }
.stave-statusbar box { border-spacing: 6px; }

.stave-floating { border-radius: 12px; padding: 8px 14px; font-size: 13px; border-spacing: 12px; }
.stave-placeholder { font-size: 13px; }

.stave-files .stave-file { padding: 5px 8px; border-spacing: 6px; }
.stave-files .stave-file.nested { padding-left: 20px; }
.stave-files .stave-file.found { border-spacing: 0; }
.stave-files .stave-folder { padding: 9px 8px 5px 8px; font-weight: 600; }
.stave-files .stave-read-only { font-size: 11px; }
.stave-files > row:selected .stave-file { font-weight: 600; }
.stave-tabs { min-height: 38px; padding: 0 8px; }
.stave-tabs > box { border-spacing: 2px; }
button.stave-tab { min-height: 32px; padding: 0 14px; margin-top: 6px; border-radius: 8px 8px 0 0; font-size: 13px; font-weight: normal; }
button.stave-tab.active { font-weight: 600; }
.stave-mode { padding: 2px; border-radius: 8px; font-size: 12px; font-weight: 600; }
.stave-mode > toggle { min-height: 24px; padding: 0 10px; border-radius: 6px; }
.stave-mode > separator { background: none; min-width: 0; margin: 0; }
.stave-crumbs { min-height: 30px; padding: 0 16px; font-size: 12px; }
.stave-crumb.current { font-weight: 600; }
textview.stave-editor { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 13px; }
.stave-gutter { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 13px; }
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

.stave-problems-head { padding: 2px 4px; }
.stave-problems-title { font-size: 15px; font-weight: 700; }
button.stave-close { min-width: 28px; min-height: 28px; padding: 0; }
.stave-filters button.stave-filter { min-height: 28px; padding: 0 10px; border-radius: 14px; font-size: 12px; font-weight: 600; }
.stave-problems-lead { padding: 0 6px; font-size: 12px; }
.stave-problems-tree { font-size: 13px; }
.stave-problems-tree .stave-heading { padding: 12px 8px 4px 8px; font-size: 12px; font-weight: 700; letter-spacing: 0.02em; }
.stave-problems-tree .stave-heading.first { padding-top: 8px; }
.stave-problem-assembly { padding: 5px 8px; font-weight: 600; }
.stave-problem-assembly.depth1 { padding-left: 22px; }
.stave-problem-assembly.depth2 { padding-left: 36px; }
.stave-problem-assembly .stave-detail { font-size: 12px; font-weight: 500; }
.stave-problem-part { padding: 5px 8px 5px 20px; }
.stave-problem-part.depth1 { padding-left: 30px; }
.stave-problem-part.depth2 { padding-left: 44px; }
.stave-problem-part .stave-detail { font-size: 12px; }
button.stave-problem-row { min-height: 28px; padding: 0 8px; margin: 1px 3px 1px 30px; border-radius: 6px; font-size: 12.5px; font-weight: normal; }
button.stave-problem-row.depth2 { margin-left: 44px; }
.stave-problems-tree { padding: 0 0 4px 0; }
.stave-problem-kind { font-weight: 600; }
.stave-problem-places { font-family: "JetBrains Mono", "Adwaita Mono", monospace; font-size: 11.5px; }
.stave-badge { min-width: 18px; min-height: 18px; padding: 0 5px; border-radius: 9px; font-size: 11px; font-weight: 700; }
.stave-problems-about { padding: 12px; border-radius: 10px; font-size: 12px; }
.stave-eyebrow.stave-eyebrow-bold, .stave-eyebrow.stave-problem-eyebrow { font-weight: 700; letter-spacing: 0.02em; }
.stave-problem-title { font-size: 19px; }
.stave-code.stave-diagnostic { padding: 14px; font-size: 11.5px; }
.stave-number { min-width: 22px; min-height: 22px; border-radius: 11px; font-size: 12px; font-weight: 700; }
.stave-card > .stave-card-row.stave-toggle-row { padding: 10px 14px; }
.stave-card > .stave-card-row.compact { padding: 8px 14px; }
.stave-card > .stave-card-row.compact .stave-number { min-width: 20px; min-height: 20px; border-radius: 10px; font-size: 11px; }
expander.stave-more title { font-size: 13px; font-weight: 600; }
expander.stave-more title > arrow { min-width: 12px; min-height: 12px; }
"#;

/// The colours, from libadwaita's named colours, so the light and dark
/// styles (Main.dc.html, MainDark.dc.html) both come from this one sheet
/// and follow the style manager the moment it changes. Only the code
/// block is dark in both styles, as the design has it.
const COLOURS: &str = r#"
.stave-header { background: @headerbar_bg_color; color: @headerbar_fg_color; box-shadow: inset 0 -1px @headerbar_shade_color; }
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
.stave-sidebar entry.stave-search { background: @view_bg_color; color: @view_fg_color; }
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
.stave-code { background: #1e1e24; color: #e7e5ea; }

.stave-statusbar { background: @headerbar_bg_color; color: alpha(@headerbar_fg_color, 0.7); box-shadow: inset 0 1px @headerbar_shade_color; }
.stave-floating { background: alpha(@view_bg_color, 0.94); color: @view_fg_color; box-shadow: 0 2px 10px alpha(black, 0.15); }

.stave-files .stave-folder, .stave-files .stave-file.standard, .stave-files .stave-detail { color: alpha(currentColor, 0.7); }
.stave-files > row:selected .stave-detail { color: alpha(@accent_fg_color, 0.9); }
.stave-files .stave-unsaved { color: @accent_color; font-weight: 700; }
.stave-files > row:selected .stave-unsaved { color: @accent_fg_color; }
.stave-editor-column { background: @view_bg_color; color: @view_fg_color; box-shadow: inset -1px 0 mix(@view_bg_color, @view_fg_color, 0.15); }
.stave-tabs { background: mix(@view_bg_color, @window_bg_color, 0.6); box-shadow: inset 0 -1px @headerbar_shade_color; }
button.stave-tab { background: transparent; color: alpha(currentColor, 0.7); box-shadow: none; }
button.stave-tab:hover { background: alpha(currentColor, 0.05); }
button.stave-tab.active { background: @view_bg_color; color: @view_fg_color; box-shadow: inset 1px 0 @headerbar_shade_color, inset -1px 0 @headerbar_shade_color, inset 0 1px @headerbar_shade_color; }
.stave-mode { background: alpha(currentColor, 0.08); }
.stave-crumbs { color: alpha(currentColor, 0.7); box-shadow: inset 0 -1px alpha(currentColor, 0.07); }
.stave-crumb.current { color: @view_fg_color; }
textview.stave-editor, textview.stave-editor > text, .stave-gutter { background: @view_bg_color; color: @view_fg_color; }
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
.stave-files-split > separator { background: mix(@window_bg_color, @window_fg_color, 0.15); }
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

.stave-filters button.stave-filter { background: transparent; color: inherit; box-shadow: none; }
.stave-filters button.stave-filter:hover { background: alpha(currentColor, 0.07); }
.stave-filters button.stave-filter:checked { background: @view_bg_color; color: @view_fg_color; box-shadow: 0 0 0 1px alpha(black, 0.1); }
.stave-filters button.stave-filter.errors { color: @error_color; }
.stave-problems-lead, .stave-problems-tree .stave-detail, .stave-problems-tree .stave-heading { color: alpha(currentColor, 0.7); }
button.stave-problem-row { background: transparent; color: inherit; }
button.stave-problem-row:hover { background: alpha(currentColor, 0.06); }
button.stave-problem-row.selected { background: @view_bg_color; color: @view_fg_color; box-shadow: 0 0 0 2px @accent_bg_color; }
.stave-problem-icon.error { color: @error_color; }
.stave-problem-icon.warning { color: @warning_color; }
.stave-problem-icon.note { color: alpha(currentColor, 0.7); }
.stave-problem-places { color: @accent_color; }
.stave-badge.error { background: @error_bg_color; color: @error_fg_color; }
.stave-badge.warning { background: @warning_bg_color; color: @warning_fg_color; }
.stave-badge.note { background: alpha(currentColor, 0.15); }
.stave-problems-about { background: alpha(currentColor, 0.05); color: alpha(currentColor, 0.75); }
.stave-eyebrow.stave-problem-eyebrow.error { color: @error_color; }
.stave-eyebrow.stave-problem-eyebrow.warning { color: @warning_color; }
.stave-number { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.stave-number.second { background: alpha(currentColor, 0.1); color: alpha(currentColor, 0.85); }
.stave-card .stave-value.good { color: @success_color; }
.stave-card .stave-value.bad { color: @error_color; }
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

/// The snippet's colours on its dark background (both styles).
pub const CODE: CodeColours = CodeColours {
    keyword: "#f0a3c4",
    name: "#8fd0ff",
    number: "#ffcf80",
    text: "#8ff0a4",
    comment: "#8b8a93",
    error: "#ff7b85",
    warning: "#ffcf80",
};

/// The dot in the status bar: green, amber, red or grey.
pub fn status_dot(tone: stave_app::session::Tone) -> gdk::RGBA {
    use stave_app::session::Tone;
    let (red, green, blue): (u8, u8, u8) = match tone {
        Tone::Clean => (0x26, 0xa2, 0x69),
        Tone::Warnings => (0xe5, 0xa5, 0x0a),
        Tone::Errors => (0xc0, 0x1c, 0x28),
        Tone::Unknown => (0x9a, 0x99, 0x96),
    };
    gdk::RGBA::new(
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
        1.0,
    )
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
}
