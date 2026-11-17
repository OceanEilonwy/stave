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
"#;

/// The light colours (Main.dc.html).
const LIGHT: &str = r#"
window.stave-window { background: #fafafb; color: #222226; }
.stave-header { background: #ffffff; box-shadow: inset 0 -1px #dededf; color: #222226; }
.stave-header button.stave-icon-button:disabled { color: #9a9996; }
.stave-header button.stave-sidebar-toggle:checked { background: #ececee; }
.stave-title .subtitle { color: #5e5c64; }
.stave-switcher { background: #ececee; }
.stave-switcher > toggle { color: #3d3846; background: transparent; }
.stave-switcher > toggle:checked { background: #ffffff; color: #222226; box-shadow: 0 1px 2px rgba(0,0,0,0.12); }
menubutton.stave-scene > button { background: #ececee; color: #222226; }
menubutton.stave-scene .dim { color: #5e5c64; }
button.stave-pill.clean { background: #e0f2e8; color: #145c3a; }
button.stave-pill.warnings { background: #fdf0d5; color: #8a5100; }
button.stave-pill.errors { background: #fbe3e4; color: #a51d2d; }
button.stave-pill.unknown { background: #ececee; color: #5e5c64; }
button.stave-build { background: #1c71d8; color: #ffffff; }
button.stave-build:hover { background: #1a65c2; }
button.stave-build:disabled { background: #ececee; color: #77767b; }
button.stave-build.building:disabled { background: #a7c6ef; color: #ffffff; }

.stave-sidebar { background: #ebebed; box-shadow: inset -1px 0 #dededf; }
.stave-sidebar entry.stave-search { background: #ffffff; color: #222226; }
.stave-tree > row:hover { background: rgba(0,0,0,0.05); }
.stave-tree > row:selected { background: #1c71d8; color: #ffffff; }
.stave-tree > row:selected .stave-detail { color: rgba(255,255,255,0.9); }
.stave-tree .stave-detail, .stave-tree .stave-heading { color: #5e5c64; }

.stave-viewport { background: #e8e5df; }
.stave-inspector-panel { background: #fafafb; box-shadow: inset 1px 0 #dededf; }
.stave-eyebrow, .stave-description, .stave-section, .stave-footnote, .stave-key { color: #5e5c64; }
.stave-card { background: #ffffff; box-shadow: 0 0 0 1px rgba(0,0,0,0.08); }
.stave-card > .stave-card-row { border-bottom: 1px solid #ededef; }
.stave-card > button.stave-card-row { background: transparent; color: #222226; }
.stave-card > button.stave-card-row:hover { background: rgba(0,0,0,0.04); }
.stave-link { color: #1c71d8; }
.stave-code { background: #1e1e24; color: #e7e5ea; }

.stave-statusbar { background: #ffffff; box-shadow: inset 0 1px #dededf; color: #5e5c64; }
.stave-floating { background: rgba(255,255,255,0.92); box-shadow: 0 2px 10px rgba(0,0,0,0.12); color: #222226; }
.stave-floating .stave-key { color: #5e5c64; }
"#;

/// The dark colours (MainDark.dc.html).
const DARK: &str = r#"
window.stave-window { background: #222226; color: #f6f5f4; }
.stave-header { background: #303034; box-shadow: inset 0 -1px #141416; color: #f6f5f4; }
.stave-header button.stave-icon-button:disabled { color: #77767b; }
.stave-header button.stave-sidebar-toggle:checked { background: #3a3a3f; }
.stave-title .subtitle { color: #c0bfc4; }
.stave-switcher { background: #3a3a3f; }
.stave-switcher > toggle { color: #deddda; background: transparent; }
.stave-switcher > toggle:checked { background: #303034; color: #f6f5f4; box-shadow: 0 1px 2px rgba(0,0,0,0.12); }
menubutton.stave-scene > button { background: #3a3a3f; color: #f6f5f4; }
menubutton.stave-scene .dim { color: #c0bfc4; }
button.stave-pill.clean { background: #1f3a2b; color: #8ff0a4; }
button.stave-pill.warnings { background: #45361a; color: #ffc96b; }
button.stave-pill.errors { background: #4a2226; color: #ff8f96; }
button.stave-pill.unknown { background: #3a3a3f; color: #c0bfc4; }
button.stave-build { background: #1c71d8; color: #ffffff; }
button.stave-build:hover { background: #2a7de1; }
button.stave-build:disabled { background: #3a3a3f; color: #77767b; }
button.stave-build.building:disabled { background: #2c4f7c; color: #ffffff; }

.stave-sidebar { background: #2a2a2e; box-shadow: inset -1px 0 #141416; }
.stave-sidebar entry.stave-search { background: #303034; color: #f6f5f4; }
.stave-tree > row:hover { background: rgba(255,255,255,0.05); }
.stave-tree > row:selected { background: #1c71d8; color: #ffffff; }
.stave-tree > row:selected .stave-detail { color: rgba(255,255,255,0.9); }
.stave-tree .stave-detail, .stave-tree .stave-heading { color: #c0bfc4; }

.stave-viewport { background: #242428; }
.stave-inspector-panel { background: #222226; box-shadow: inset 1px 0 #141416; }
.stave-eyebrow, .stave-description, .stave-section, .stave-footnote, .stave-key { color: #c0bfc4; }
.stave-card { background: #303034; box-shadow: 0 0 0 1px rgba(0,0,0,0.3); }
.stave-card > .stave-card-row { border-bottom: 1px solid #3d3d42; }
.stave-card > button.stave-card-row { background: transparent; color: #f6f5f4; }
.stave-card > button.stave-card-row:hover { background: rgba(255,255,255,0.05); }
.stave-link { color: #78aeed; }
.stave-code { background: #2a2a2e; color: #e7e5ea; }

.stave-statusbar { background: #303034; box-shadow: inset 0 1px #141416; color: #c0bfc4; }
.stave-floating { background: rgba(48,48,52,0.94); box-shadow: 0 2px 10px rgba(0,0,0,0.3); color: #f6f5f4; }
.stave-floating .stave-key { color: #c0bfc4; }
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
    /// Comments.
    pub comment: &'static str,
}

/// The snippet's colours on its dark background (both styles).
pub const CODE: CodeColours = CodeColours {
    keyword: "#f0a3c4",
    name: "#8fd0ff",
    number: "#ffcf80",
    text: "#8ff0a4",
    comment: "#9a9996",
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

/// Installs the window's style for `display`, and keeps the colours in
/// step with the light or dark style.
pub fn install(display: &gdk::Display) {
    let shared = gtk::CssProvider::new();
    shared.load_from_string(SHARED);
    gtk::style_context_add_provider_for_display(
        display,
        &shared,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let colours = gtk::CssProvider::new();
    let manager = adw::StyleManager::default();
    let load = {
        let colours = colours.clone();
        move |manager: &adw::StyleManager| {
            colours.load_from_string(if manager.is_dark() { DARK } else { LIGHT });
        }
    };
    load(&manager);
    manager.connect_dark_notify(load);
    gtk::style_context_add_provider_for_display(
        display,
        &colours,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
