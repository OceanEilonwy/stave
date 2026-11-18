# Docked Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the floating V2 window with a libpanel docked window (layout B, optional right panel, Code panel decides where files open) and fix six behaviour bugs.

**Architecture:** All changes are in `crates/stave-gtk` (plus `stave-app` only if a fix needs session state). A new `ui/workbench.rs` owns the `panel::Dock`, its frames and panel widgets, the edge toggles and the saved layout; `ui/mod.rs` hands it the existing widgets (tree, inspector, editor page, problems list, 3D view + dock strip) instead of laying them over the 3D view.

**Tech Stack:** Rust 2024, gtk4-rs 0.11, libadwaita 0.9, `libpanel` 0.7 (system `libpanel-1` 1.10), GtkSourceView 5.

**Spec:** `docs/superpowers/specs/2026-11-18-docked-window-design.md`

## Global Constraints

- Only `stave-gtk` depends on `libpanel`.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` clean before each commit.
- No `unwrap`/`expect`/`panic!` outside tests.
- `tools/window-shot` output stays deterministic: screenshots never read the saved layout.
- User-facing text: plain English, woodworking words.

## Review Focus

- A saved layout naming a panel that no longer exists, or a corrupt file: start with the default layout, no crash.
- The Code panel closed by the user, then a file opened (Ctrl+K, "Defined in the text"): the Code panel comes back in its last place and the file opens in it.
- The last code tab closed: the Code frame shows its placeholder and stays where it is.
- Selecting a part in 3D while the editor shows another file: the camera does not move, the text still follows.
- Narrow window (760 px): panels still fit; the 3D view keeps a usable width.

---

### Task 1: Behaviour fixes

**Files:** `crates/stave-gtk/src/ui/mod.rs`, `ui/finder.rs`, `ui/editor.rs`, `ui/style.rs`, `ui/header.rs`

- [ ] Solid header: the header bar keeps `@headerbar_bg_color` when the window is unfocused or the content scrolls under it (remove whatever stylesheet rule or `flat` class makes it transparent).
- [ ] Esc in the finder's entry closes the finder (a key controller on the dialog's entry, capture phase, or `adw::Dialog::close` on `stop-search`).
- [ ] `Window::select_and_frame(&Rc<Self>, Option<Selection>)`: dispatch `Command::Select` then `frame_selection()`. `handle()` no longer frames on `SelectionChanged`. Callers that frame: the parts tree, the finder's Go and OpenText, the editor's "show in 3D", the command line's `--select`. A 3D pick, move chips, Back/Forward and problems do not.
- [ ] Mouse buttons 8 and 9 on the window: a `GestureClick` with button 0 in capture phase; button 8 dispatches `Command::Back`, 9 `Command::Forward`.
- [ ] Editor lenses: a click on a lens does not start a text selection (claim the click before GtkTextView's own gesture, or keep the cursor where it was).
- [ ] X-ray: remove every place the window sets the X-ray toggle or `state.xray` itself (focus mode turning it on, leaving focus mode); only the button changes it. Focus mode keeps its own option.
- [ ] Run fmt, clippy, tests; regenerate screenshots that changed; commit.

### Task 2: libpanel spike

- [ ] Add `panel = { package = "libpanel", version = "0.7" }` to `crates/stave-gtk/Cargo.toml`; build.
- [ ] Throwaway example window: `panel::Dock` with start, bottom, end areas and a centre; two `panel::Widget`s in one `panel::Frame`; a frame with a placeholder; drag a widget to another edge; find out how to add a widget to the frame another widget is in (`Widget::frame` or the frame's ancestor) and how a layout is saved and restored (`panel::Session`, or our own record).
- [ ] Note findings in this plan; delete the example.

### Task 3: Docked window

**Files:** Create `crates/stave-gtk/src/ui/workbench.rs`; modify `ui/mod.rs` (`build_widgets`, `Widgets`, `editor_changed`, `show_card`, `place_card`), `ui/files.rs` (drop the rail and "+", open files as `panel::Widget`s), `ui/tree.rs` (plain sidebar look), `ui/dock.rs` (a strip, not floating), `ui/style.rs`, `docs/gui.md`.

- [ ] `Workbench::new(parts, files_list, details, view, problems) -> Rc<Workbench>`, with `open_code(widget: &panel::Widget)` that adds it to the frame the Code panel lives in (or brings the Code panel back first), `reset_layout()`, `save_layout()` / `restore_layout()` to `glib::user_config_dir()/stave/layout`.
- [ ] Default layout B: start area Parts|Files over Details; centre the 3D view with its strip; bottom Code (placeholder) and Problems; end area empty, hidden.
- [ ] Details shows what the floating card showed (inspector pages and focus mode's panel); the card and its placement code go.
- [ ] Problems panel: the problems tree as a list (click selects the problem).
- [ ] Status bar with the edge toggles; main menu "Reset layout".
- [ ] Screenshots: `tools/gui-screenshots`, look at every changed picture; update `docs/gui.md`.
- [ ] fmt, clippy, tests; commit.
