# Moving stave-gtk to relm4

This is the contract for moving `crates/stave-gtk` from hand-written
gtk-rs plumbing to [relm4](https://relm4.org) 0.11. Every lane codes
against it: the pattern, the rules, the test convention, and the table
of each leaf file's callbacks and setters with the message names they
become. Phase 0 (this document, the `relm4` dependency, the shared
`Bridge` and the slider) is on branch `relm4/base`.

## Why

The window's recent bugs all came from Rust's `Rc<RefCell<..>>` meeting
GTK's synchronous, re-entrant signals and widget-owned closures:

| Commit | What went wrong |
|---|---|
| 960f06d | About 50 handlers held a strong `Rc<Window>`: a reference cycle, so closed windows were never freed. |
| 0603e47 | Once they held it weakly, nothing owned the window: it was freed at once. |
| bc79bd8 | A `borrow_mut()` was held while `close()` emitted `close-request`, whose handler borrowed again: a panic across GTK's C trampoline, so an abort. |
| 079a512 | Callbacks ran while their `RefCell` was borrowed: `BorrowMutError`. |
| b7d26f2 | `try_borrow` silently dropped what people typed; hand-rolled `updating_controls` and `syncing` guards. |

relm4 removes these classes of bug by construction. One component owns
its state. Only `update(&mut self, message)` changes it. Widgets hold a
`ComponentSender` and send messages, which are queued on the main loop
and handled one at a time, never re-entered. Programmatic changes that
would fire a handler are wrapped in `#[block_signal]` instead of a
hand-rolled guard.

## The stack

`relm4 = "0.11"` with `default-features = false` and the features
`macros`, `libadwaita`, `libpanel` and `gnome_47`:

- `gnome_47` is the level of our `gtk4` feature (`v4_16`). Features only
  add, so our own `libadwaita` and `libpanel` features (`v1_8`) stay as
  they are; a higher `gnome_N` would raise gtk's minimum for no gain.
- `cargo tree -p stave-gtk` shows one version each of gtk4 (0.11.5), gdk4,
  glib and gio (0.22.11), libadwaita (0.9.2), libpanel (0.7.0) and
  sourceview5 (0.11.3). Keep it that way; check with
  `cargo tree -p stave-gtk -d`.
- `relm4-css` (a default feature) is left out: we have our own style.
- We do not use `RelmApp`. The application, its windows and its main loop
  stay as they are; components are launched inside them
  (`Component::builder().launch(..)`), and their runtimes are futures on
  the default `glib::MainContext`.

## The pattern

A leaf (a widget group with no children of its own that matter, such as
the slider, the problems panel or the header) becomes a relm4
`Component`. While its callers are still the old code, a thin shim keeps
its old public API, so the coordinators (`mod.rs`, `edit_view.rs`,
`app.rs`) need no change in the same commit. When a coordinator itself
moves to relm4, it owns the leaves' `Controller`s directly and the shims
go.

### The worked example: `ui/slider.rs`

The slider used to keep an `Rc<RefCell<State>>` shared by its draw
function, its gesture and key handlers and the window, with a
`WeakSlider` so its own handlers would not keep it alive.

**The component.** `SliderComponent` is the model: range, value, blocked
range and handle size, small and `Copy`. Its messages:

```rust
pub enum SliderInput {
    PointedAt { x: f64, width: f64, right_to_left: bool }, // people pressed or dragged
    Stepped(Step),                      // people pressed an arrow, Page, Home or End key
    Scrolled { dx: f64, dy: f64, right_to_left: bool },    // people scrolled over it
    SetValue(f64),                      // the owner moves it: no output
    SetRange { lower: f64, upper: f64 },
    SetBlocked(Option<(f64, String)>),
    SetValueText(String),
}

pub enum SliderOutput {
    Changed(f64),                       // only what people do
}
```

- **Widgets only send.** The drag handlers read the pointer and the
  widget's width and send `PointedAt`; the key handler turns the key into
  a `Step` and sends `Stepped`. None of them reads or writes the model.
- **Only `update` changes the model**, and it alone decides whether to
  send `SliderOutput::Changed`: people's input does when the value moves,
  `SetValue` and the other setters never do. "Set it without hearing
  back" is now a property of the message, not of a guard.
- **The view is derived.** `#[watch] set_draw_func: model.painter()`
  hands the drawing area a *copy* of the model after every update, so the
  draw function shares nothing; `#[watch] update_property` keeps the
  accessible value up to date.
- **The root** is built with a builder (`gtk::DrawingArea::builder()
  .accessible_role(..).build() -> gtk::DrawingArea { .. }`), because the
  accessible role can only be set at construction.

**The handle.** `Slider` is what owners hold: the component's root and
its `Controller`, launched with its output forwarded straight to the
owner's input as a message the owner chose (rule 6). Its setters
(`set_value`, `set_range`, `set_blocked`, `set_value_text`) only send
inputs; `value()` reads the model, for tests:

```rust
pub struct Slider {
    pub root: gtk::DrawingArea,                // the component's root widget
    controller: Controller<SliderComponent>,   // the one owner
}

impl Slider {
    pub fn launch<M: 'static>(
        label: &str, height: i32, handle: f64, ring: f64,
        owner: &relm4::Sender<M>,
        changed: impl Fn(f64) -> M + 'static,  // a message constructor
    ) -> Slider {
        let controller = SliderComponent::builder()
            .launch(SliderInit { /* .. */ })
            .forward(owner, move |output| match output {
                SliderOutput::Changed(value) => changed(value),
            });
        Slider { root: controller.widget().clone(), controller }
    }

    pub fn set_value(&self, value: f64) {
        let _ = self.controller.sender().send(SliderInput::SetValue(value));
    }
}
```

It is not `Clone`: a `Controller` has one owner. The view's tools launch
theirs into `ViewToolbarInput::Moved`, and the focus cards (`focus_stage.rs`)
theirs into the card's own input (`StepCardInput::Scrubbed` and so on).

**What changed for callers.** Setters are queued, so they take effect
when the main loop next runs, after the caller has returned; and the
owner hears people's change as a message after the slider's update has
finished, never from inside it. `value()` is the value as of the last
handled message.

### The shared helper: `ui/relm4_shim.rs`

`Bridge<C: Component>` is the shim pattern for every lane:

| Method | What it does |
|---|---|
| `Bridge::launch(init)` | Launches the component and forwards its outputs to the callback slot. |
| `bridge.widget()` | The root widget (the shim keeps a clone as its old `root` field). |
| `bridge.send(input)` | Queues an input message; ignores the error after shutdown. |
| `bridge.connect(callback)` | Outputs go to `callback` from now on (replacing the last one); before any is connected they are dropped, as with the old slots. |
| `bridge.model()` | The model as of the last handled message, for old getters. |

The callback slot is a `Cell<Option<Rc<dyn Fn(Output)>>>`, cloned out
before each call, so a callback can call `connect`, `send` or `model` on
its own bridge. Nothing is borrowed across a callback or a GTK call.

Every owner now holds its components' `Controller`s itself, outputs
forwarded at launch, so `relm4_shim.rs` is compiled for tests only:
some tests still launch a component through a `Bridge` to hear its
outputs.

## The rules

1. **No `RefCell` borrow is held across any widget call or signal
   emission.** GTK emits signals synchronously from inside widget calls
   (`set_active`, `close`, `set_text`, `grab_focus`, `select_row`, ...).
   Inside a component this is automatic: the state is `&mut self` in
   `update`. In code not yet moved, copy what you need out of the borrow
   and drop it first.
2. **No strong capture of `self`, the window or a component in a widget
   closure.** Widget closures capture a `ComponentSender` (the view
   macro's `[sender]`) or plain copied values, nothing else. A closure
   that needs the model's values receives a copy (as the slider's draw
   function does).
3. **Widgets send messages only.** A handler reads what the event gives
   it (pointer, key, the widget's own property) and sends one input. It
   never reads or changes the model, never calls another component, never
   calls back into the window.
4. **Send without panicking.** In widget closures use
   `sender.input_sender().emit(message)`. `sender.input(..)`, and the
   macro's `connect_x => Message` shorthand which calls it, panic if the
   component has shut down while its widget lives on (still packed in a
   parent): that panic would cross GTK's C trampoline and abort. Outputs:
   `let _ = sender.output(..)` (no listener is not a mistake). From
   outside, `Bridge::send` or `controller.sender().send(..)`, never
   `controller.emit(..)` (it unwraps).
5. **`#[block_signal]` for programmatic updates.** When the view sets a
   property whose change fires a handler (a toggle's `active`, a scale's
   `value`, a list's selection, a switcher's active name), name the
   handler and block it in the `#[watch]` line:

   ```rust
   gtk::ToggleButton {
       #[watch]
       #[block_signal(pull_toggled)]
       set_active: model.pull_apart,
       connect_toggled[sender] => move |button| {
           sender.input_sender().emit(ToolbarInput::Pull(button.is_active()));
       } @pull_toggled,
   }
   ```

   This replaced `updating_controls` in `mod.rs` and `syncing` in
   `tree.rs`. Setters that did not call back (`set_value` and friends)
   become inputs whose `update` sends no output. A component that fills
   a list imperatively (in `update_with_view`) calls
   `list.block_signal(&handler)` / `unblock_signal` on the view's
   `@name` handler id itself, which is what `#[block_signal]` expands to
   (`tree.rs` does this).
6. **Who owns each `Controller`.** Exactly one owner, never shared, never
   in an `Rc`:
   - while a leaf keeps its old API, the leaf's shim owns it through a
     `Bridge`, and the shim is owned where the old struct was (for
     example `Widgets` in `mod.rs` owns `ViewToolbar`, which owns the
     slider's shim);
   - once a coordinator is a component, its model owns its children's
     `Controller`s as fields (launched in its `init` with `.forward(..)`
     to its own input), and the window component is owned by the
     application for as long as the window is open.
7. **Dropping a `Controller` shuts its component down.** The shutdown
   happens the next time the main loop runs: relm4 calls `shutdown`,
   drops the model and widgets, and the output forwarding ends once the
   widget's handlers are freed. Never call `detach_runtime` (it leaks the
   component for the life of the process). A widget may outlive its
   component while a parent still holds it; rule 4 keeps that harmless.
8. **`model()` is not for handlers.** relm4 borrows the model mutably
   while `update` runs, so calling `controller.model()` (or
   `Bridge::model`) from a handler of the same component's widgets can
   panic. Old getters may use it from outside; new code asks with a
   message or keeps the value in the owner's model.
9. **Callbacks that return a value** (`connect_search`, `connect_actions`,
   `connect_unsaved`, `set_page_tabs`) become a request and a reply: an
   output asking (`FinderOutput::Search(query)`) and an input answering
   (`FinderInput::Hits(hits)`). Never call the owner synchronously from
   `update`.
10. **Borrowed arguments become owned messages.** Setters that take
    `&CardModel<'_>`, `&[MenuItem<'_>]` or `&dyn Words` get an owned
    message type (`EditCardInput::Show(CardView)`), built by the caller.
    Messages must be `Debug`.
11. **Shared state across leaves is a smell to remove, not to wrap.**
    `Rc<RefCell<Spots>>` (shared by `Workbench` and `Floats`) and the
    `Rc<Cell<..>>` flags passed into `BufferWriter` move into the one
    component that owns them, and the others hear about changes by
    message.
12. **A known relm4 cost.** Each launched component leaves a small
    shutdown sender in a process-wide list (relm4 drains it only when a
    `RelmApp` quits). Launch components for things that live a while
    (panels, cards, sliders), not one per list row on every refresh; use
    a relm4 factory, or plain widgets inside a component, for rows.

## The test convention

- GTK tests run on the one shared GTK thread:
  `crate::ui::gtk_test::on_gtk_thread(body)`. Without a display the body
  is skipped, saying so.
- relm4 does its work on the main loop, so after sending messages,
  firing signals or dropping a controller, call
  `crate::ui::gtk_test::settle()` (runs the loop until nothing is
  waiting) before asserting.
- Fire people's input as GTK would: `button.emit_clicked()`,
  `toggle.set_active(..)`, `gesture.emit_by_name::<()>("drag-begin",
  &[&x, &y])`, and assert on the model (`bridge.model()`), on widgets, or
  on what a connected callback recorded.
- Every component keeps or gains:
  1. a freeing test: drop the shim or controller, `settle()`, and assert
     the root widget's `WeakRef` is gone, the input sender's `send`
     fails (the runtime and model are gone) and the callback's captures
     are freed (see `slider::tests::a_dropped_slider_is_freed`);
  2. a setters test: setters change the model and send no output;
  3. a queueing test where re-entrancy was possible: input is only
     queued until `settle()`, and a callback or signal fired from inside
     a callback is queued too
     (`slider::tests::a_signal_fired_while_handling_an_output_is_queued_not_re_entered`).
- The baseline is in `target/relm4-baseline/`: `tests.txt` (every
  passing test before the migration, `binary::path::name`, sorted),
  `test-output.txt`, PNGs and `screenshots.txt` (the exact commands, run
  headless through Broadway with `tools/window-shot`). A lane's change is
  done when every test in `tests.txt` still passes (renamed tests
  listed in the commit), and its screenshots match the baseline or the
  differences are explained.

## The leaf table

For each leaf file in `crates/stave-gtk/src/ui`: its callback slots
(`connect_*`, `Rc<dyn Fn>` / `Box<dyn Fn>` fields and constructor
arguments, and the raw widget signals a coordinator connects on its
public fields) become **Output** variants; its setters become **Input**
variants. Names are `<Type>Output::<Variant>` and `<Type>Input::<Variant>`.
Getters that only read widgets for tests (`rows()`, `buttons()`,
`entries()`, `words()`) stay as plain methods on the shim, reading the
model or walking `bridge.widget()` (there is no `Bridge::widgets()`;
`tree.rs`'s tests find the list by walking the root). Queries the window calls synchronously
are marked *query*: keep them on the shim while the caller is old code,
and move the answer into the owner's model when the caller moves.

The coordinators are not in the table: `mod.rs` (`Window`), `app.rs`
(`Shell`), `edit_view.rs`, `edit_script.rs`, `edit_writer.rs` and
`edit_demo.rs` (all `impl Window`), and the `impl Window` / `impl Shell`
parts of `new_project.rs`, `start_page.rs` and the `edit_view/` files.
Files with no widgets need nothing: `view_layout.rs` (layout arithmetic),
`remember.rs` (saved window state), `style.rs` (CSS and colours),
`gtk_test.rs`, `relm4_shim.rs`.

| File | Type | Callback slot → Output | Setter → Input | Notes |
|---|---|---|---|---|
| `slider.rs` | `Slider` | `SliderOutput::Changed(f64)`, forwarded at launch: `Slider::launch(.., owner, changed)` sends `changed(value)` to the owner | `set_value` → `SetValue(f64)`; `set_range` → `SetRange { lower, upper }`; `set_blocked` → `SetBlocked(Option<(f64, String)>)`; `set_value_text` → `SetValueText(String)`; `set_label` → `SetLabel(String)` (what screen readers call it: the swing and problem cards rename their slider by message, never through its root) | **Done**. People's input: `PointedAt { x, width }`, `Stepped(Step)`, `Scrolled`. `value()` reads the model (tests). Every owner launches it with `Slider::launch`: the view's tools and `focus_stage.rs`'s three cards; there is no callback form. |
| `chrome.rs` | `MotionBar` | `MotionBarOutput::Moved(f64)`, forwarded at launch (`MotionBar::new(owner, message)`) → `SelectionInput::MotionMoved` | `show(Option<MotionShown>)` → `Show(Option<MotionShown>)`; `set_orientation` → `SetOrientation(gtk::Orientation)` | **Done**. `block_signal` on the scale's value when `Show` moves it. Its root has no alignment or margins of its own: the floats place it exactly (the dock no longer zeroes its margin). |
| `dock.rs` | `Dock` | `DockOutput::Move(AssemblyId)` → `SelectionInput::Select`; `DockOutput::Scene(usize)` (the scene menu's choice, after the dock has closed its own menu) → `RenderingInput::SceneChosen`; forwarded at launch (`Dock::new(owner, message)`) | `offer(bool)` → `Offer(bool)`; `set_moves_orientation` → `SetMovesOrientation(gtk::Orientation)`; `show_moves` → `ShowMoves { chips, current }`; `show_scenes(&[String], usize)` → `ShowScenes(ScenesShown)` | **Done**. No longer in an `Rc`: `Widgets` owns it. Owns the scene menu (it was the header's, placed in the dock's box): its button, label and list are the dock's own; the window places its two roots, `scene` and `moves`, in the floats. |
| `edit_card.rs` | `EditCardComponent` | `connect_action(CardAction)` → `EditCardOutput::Action(CardAction)`; its links → `Link(LinkTarget)`, `Hover(Option<LinkTarget>)`; the open short list → `OptionsOpen(Option<String>)` | `show(&CardModel, &dyn Words)` → `Show(Box<CardView>)` (owned); `update_options(&Candidate)` → `UpdateOptions(Box<Candidate>)`; `open_options(&str) -> bool` → `OpenOptions(String)` (the bool is the window's own `CardOffers`); `scroll_to_open_options_now()` → `ScrollToOpenOptions` | **Done**: the shim is gone; the window owns the `Controller` (see "Edit card and menu: answers by message"). `Init` is `CardSlots` (the repeat and Pending boxes the window makes and others fill: an owner-created slot, see "Accepted exceptions"). Test getters on the model (`entries`, `option_lists`, `fix_buttons`, `put_back_button`) and the widgets (`buttons`, `name_field`, `scrolled`). |
| `edit_menu.rs` | `EditMenuComponent` | `connect_chosen(Declaration)` → `EditMenuOutput::Chosen(Declaration)`; `EditMenuOutput::Drawn` after each `Show`; its own "Hold a distance from this space…" → `EditMenuOutput::HoldFromSpace` | `show(Heading, &[MenuItem])` → `Show { heading, items: Vec<MenuEntry>, from_space: bool }` (owned; `from_space` offers the hold button, for a space); the window's placing (margins from its measured height) → `Place { point, view }` | **Done**: the shim is gone; the window owns the `Controller`. The menu owns the hold button and places itself; the window never measures it or sets its margins. Test getter `rows()` on the model. |
| `edit_panel.rs` | `EditPanel` | — | — (`update() -> bool` stays a plain method; no message) | **Done** as a plain widget helper of `EditView`: it has no state of its own, no callbacks and no cells; `update()` reads its cards' visibility and picks the page, synchronously, from the window's update. A component would add nothing. |
| `editor.rs` | `Editor` | a lens's links → a `LensClick` sent on the `relm4::Sender<LensClick>` given to `Editor::new`; `FilesComponent` forwards that channel's receiver into its own input as `FilesInput::ListUses` / `ShowIn3d` | — (called by its owner's `update`) | **Done** (with `files.rs`): no `Cell`s: its state is plain fields changed through `&mut self` (only `FilesComponent::update` calls it); a lens row's link handlers are made again with the declaration it shows (no shared symbol cell); a go-to's scroll keeps its frame callback and a newer go-to removes it (no shared ticket); the text view and its gutter marks read the dark style from their own CSS class; its search bar is a `Controller` (detached). the view of `FilesComponent`, owned by its model and called only from its `update`, so `show_buffer`, `go_to` and the cursor queries take effect at once inside that update; its search bar is the component `EditorSearch`. GObject subclasses `Marks`, `CodeView`, `BlockIndenter` stay widgets. `EscapeProbe` answers "is anything open over the text?" from the widgets alone, for the Escape key handler. `new_buffer(table, text)` makes a buffer sharing the tags. |
| `empty_state.rs` | `PageFit` (`fit`, `fit_every`) | — | — | **Done** as a GObject subclass of `gtk::Adjustment`, set as a docked status page's vertical adjustment: it hears the page's frame and words change size in its own `changed` method, and hides or shows the page's icon in an idle that holds it weakly. `fit(page)` is called once per page by the component that owns it, on its own page (`panels.rs`'s two panels, `files.rs`, `inspector.rs`, `edit_panel.rs`); `fit_every`, with which the workbench walked into every panel's widgets when it docked them, is gone. No closures share Rust state. Its instance state is an accepted exception (below). |
| `files.rs` | `FilesPage` / `FilesComponent<Then>` | `FilesOutput::Link(CursorName)`, `Go { from, to }`, `Check`, `ShowIn3d { analysis, symbol }`, `Open`, `Notice(String)`; replies `Saved { ok, then }`, `UnsavedCounted { count, then }`; publications `Documents(Vec<(Document, TextBuffer)>)`, `Analysed(Rc<Analysis>)`, `Unsaved(BTreeSet<PathBuf>)`, `Tabs(Vec<PageTab>)`; `NotInText` | `FocusText`, `SetFolder(PathBuf)`, `Save`, `Format`, `SaveAll { then }`, `CountUnsaved { then }`, `ForgetUnsaved`, `Search`, `ShowMarks`, `ShowResult`, `ShowLocation`, `Reveal`, `Wish`, `Restyle { dark }`, `OpenDocument`, `ShowTab(usize)`, `Adopt { document, buffer }` | **Done**. No `RefCell`, no `Rc<Self>`, no `Weak`, no `Cell`: timers and the worker thread send messages with tickets (a stale timer's message is ignored, so no fired timer is ever removed), the overlays' settling counts its frames in the model and removes its frame callback, programmatic cursor moves block the buffer's cursor handler. `FilesPage` is the window's handle: the one owner of the `Controller` (forwarded at launch, `update/project.rs`, `files_said`) and the window's own copies of what it published, which the window's synchronous reads (`edit_files`, `overlay_for_check`, `buffer_for`, `analysis`) use; see "Request and reply". The analysis is handed over by the window when `FilesOutput::Analysed` reaches it (`ProjectInput::Analysed`); the shared `LatestAnalysis` cell is gone. |
| `finder.rs` | `Finder` | `connect_search(&str) -> Vec<Hit>` → request `FinderOutput::Search(String)` / reply `FinderInput::Hits(Vec<Hit>)`; `connect_actions(&str) -> Vec<Action>` → `FinderOutput::SearchActions(String)` / `FinderInput::Actions(Vec<Action>)`; `connect_hit(Hit, Choice)` → `FinderOutput::Hit { hit, choice }`; `connect_action(usize)` → `FinderOutput::Action(usize)` | `present(&impl IsA<Widget>, &str)` → `Present { parent: gtk::Widget, query: String }`; `force_close()` → `ForceClose`, answered by `FinderOutput::ClosedForGood` (→ `ProjectInput::FinderClosedForGood`) | **Done**: rule 9 for the two searches: `FinderOutput::Search` / `SearchActions` are answered from the window's update with `Finder::show_hits` / `show_actions` (`FinderInput::Hits` / `Actions`). Forwarded at launch (`finder_said`); `ListenForHits` is gone, since the owner hears hits from launch. Root is an `adw::Dialog`, no longer a public field: a screenshot's closing asks the finder to close (`ForceClose`) and closes the window when it answers, instead of calling `dialog.force_close()`. Test getter `dialog()`. |
| `floating.rs` | `Floats` | `FloatsOutput::Orient { group, orientation }`, `Docked { group, spot }`, forwarded at launch (`Floats::new(.., owner, message)`) → `RenderingInput::ToolOriented`, `ToolDocked` | `refresh(Spots)` → `Refresh(Spots)` | **Done**. Rule 11: the tool groups' spots have one keeper, the workbench; the floats hear them as `Refresh`, first when the window has made its workbench, then at each move (the window passes on `WorkbenchOutput::ToolsChanged`). Its root is the view's overlay (`Floats::root`, which the window places in the stage), with the 3D view as its child. Takes the 3D view and the floating controls at launch (`Init`) and places each control in a *float* of its own (an `adw::Bin`): the float, not the control, gets the gestures, the "Press and hold to move it" tooltip and `stave-dragging` while dragged (its shadow drawn on the group inside, `.stave-dragging > *`); the controls (other components' roots) are only placed, and lay themselves out with no alignment or margins of their own. The overlay's `get-child-position` handler gets a copy (`Placer`) after every update. |
| `focus_panel.rs` | `FocusPanel` | `connect_source(SourceLocation)` → `FocusPanelOutput::Source(SourceLocation)`; (mod.rs) `done.connect_clicked` → `Done`; `xray`, `tint`, `fade` `connect_active_notify` → `Xray(bool)`, `Tint(bool)`, `Fade(bool)` | `show(Option<&Focus>)` → `Show(Option<Focus>)` | **Done**: `#[block_signal]` on the three checks when `Show` sets them. Owned by the `Inspector` handle, launched with `.forward()` into the window's leaf channel (`focus_panel_said`). |
| `focus_stage.rs` | `StepCard` | (mod.rs) `play.connect_clicked` → `StepCardOutput::Play`; `scrub` `Changed` → `StepCardOutput::Scrubbed(f64)` | `show_frame(Option<&str>, &str)` → `ShowFrame { step: Option<String>, status: String }`; (mod.rs) `scrub.set_value` → `SetProgress(f64)`; (selection.rs) `show_playing(&play, &play_icon, ..)` → `show_playing(bool)` / `ShowPlaying(bool)`, the icon, tooltip and screen-reader label drawn in the card's own view (the play button is private) | **Done**: the window owns the handle, the one owner of the component's `Controller`, launched in `build_widgets` with `.forward()` into the window's leaf channel (`update/selection.rs`, `step_card_said`); no `connect_outputs`, no `Bridge`. Its slider is `Slider::launch`ed into the card's own input (`StepCardInput::Scrubbed`). |
| `focus_stage.rs` | `SwingCard` | (mod.rs) `play.connect_clicked` → `SwingCardOutput::Play`; `scrub` `Changed` → `Scrubbed(f64)` | `show_name(&str, bool)` → `ShowName { name, turns }`; `show_value(&str)` → `ShowValue(String)`; (mod.rs) `scrub.set_value` → `SetProgress(f64)`; (selection.rs) `show_playing(&play, &play_icon, ..)` → `show_playing(bool)` / `ShowPlaying(bool)`, drawn in the card's own view | **Done**: the window owns the handle, the one owner of the component's `Controller`, launched in `build_widgets` with `.forward()` into the window's leaf channel (`update/selection.rs`, `swing_card_said`); no `connect_outputs`, no `Bridge`. Its slider is `Slider::launch`ed into the card's own input. |
| `focus_stage.rs` | `Caption` | — | `show(&str, &str)` → `CaptionInput::Show { text, hint }` | **Done**: owns a detached `Controller` (no outputs). |
| `focus_stage.rs` | `Pictures` | `connect_resized()` → `PicturesOutput::Resized(Option<(u32, u32)>)` (the size to render at, `None` while laid out afresh with no room yet); the `on_choose` closure given to `show` → `PicturesOutput::Chosen(usize)` | `show(&[String], on_choose)` → `Show(Vec<String>)`; `show_textures(Vec<gdk::Texture>)` → `ShowTextures(Vec<gdk::Texture>)`; `show_current(Option<usize>)` → `ShowCurrent(Option<usize>)` | **Done**: the window owns the handle, the one owner of the component's `Controller`, launched in `build_widgets` with `.forward()` into the window's leaf channel (`update/selection.rs`, `pictures_said`); no `connect_outputs`, no `Bridge`. The *query* `device_size()` is gone: the window keeps the published size (`Pictures::keep_size`, read back with `size()`), never the row's model or slots. |
| `focus_stage.rs` | `ProblemCard` | `scrub` `Changed` (mod.rs) → `ProblemCardOutput::Moved(f64)` | `show(Option<&MotionCard>, f32)` → `Show { card: Option<MotionCard>, amount: f32 }`; `show_value(&MotionCard, f32)` → `ShowValue { card, amount }` | **Done**: the window owns the handle, the one owner of the component's `Controller`, launched in `build_widgets` with `.forward()` into the window's leaf channel (`update/selection.rs`, `problem_card_said`); no `connect_outputs`, no `Bridge`. Its slider is `Slider::launch`ed into the card's own input. |
| `header.rs` | `Header` | `HeaderOutput::View(View)`, forwarded at launch (`Header::new(.., owner, message)`) → `RenderingInput::ShowView` | `show_title(&str, &str)` → `ShowTitle { title, folder }`; `show_view(View)` → `ShowView(View)`; `show_pill(&Pill, usize, &str)` → `ShowPill(PillShown)`; `show_build(Build, &str)` → `ShowBuild { build, tooltip }`; `narrow(&adw::Breakpoint, f64)` → `Narrow { breakpoint, width }` (the bar adds its own setters to the window's breakpoint, as the workbench's `NarrowEdges` does) | **Done**. `block_signal` on the switcher in `ShowView`. Pill, back, forward and build use window actions. The scene menu moved to the dock. The window places only `bar`: `menu_bar`, `menu_button`, `switcher` and `switcher_labels()` are no longer public (the window used them to add breakpoint setters; the table called `switcher_labels()` a test getter, wrongly). Tests read the bar's widgets through a `#[cfg(test)]` copy of its parts. |
| `icons.rs` | `PathIcon` | — | `set_shape(Shape)` (widget method) | A GObject subclass: stays a widget, used inside components' views. |
| `inspector.rs` | `Inspector` | `connect_focus(Target)` → `InspectorOutput::Focus(Target)`; `connect_source(SourceLocation)` → `Source(SourceLocation)`; its name links' clicks and hovers → `Link(LinkTarget)`, `Hover(Option<LinkTarget>)` (the window's `connect_links` is gone); `connect_unsaved` is gone: the window sends `show_unsaved(set)` → `InspectorInput::Unsaved(BTreeSet<PathBuf>)` when `FilesOutput::Unsaved` says the set changed | `show_focus()` → `ShowFocus`; `show_idle(Option<Note>)` → `ShowIdle(Option<Note>)`; `show(Option<&Inspection>)` → `Show(Option<Inspection>)` | **Done**: the window owns the handle, the one owner of the component's `Controller`, launched in `build_widgets` with `.forward()` into the window's leaf channel (`update/selection.rs`, `inspector_said`); no `connect_outputs`, no `Bridge`. Owns `FocusPanel` (forwarded too, `focus_panel_said`); its name links send its own input (`LinkHandlers<InspectorInput>`). |
| `new_project.rs` | `NewProjectDialog` | `cancel`, `form.folder_button`, `form.create` `connect_clicked` and `form.dialog.connect_closed` → `NewProjectOutput::Cancel`, `ChooseFolder`, `Create(NewProject)`, `Closed` | — | **Done**: owned by whoever shows it (the window's `State::new_project`, the start page's model) while it shows, forwarded at launch, let go on `NewProjectOutput::Closed`; the self-keeping `Rc<Cell<Option<NewProjectDialog>>>` and `ask` are gone. *query* `answers()` (tests). |
| `option_search.rs` | `OptionSearchComponent` | `connect_closed()` → `OptionSearchOutput::Closed`; `connect_chosen(String, String)` → `Chosen { setting, value }`; `connect_search(&str) -> Vec<SearchGroup>` is gone: the search holds the window's copy of the setting and searches it itself | `present(&str, &str)` → `Present { name, setting: Option<Box<SettingParam>>, query }`; `refresh()` → `Update(Box<SettingParam>)` (a fresh copy); `close()` → `Close`; typing → `QueryChanged`; `choose_selected()` → `ChooseSelected`; a row's click → `Activated(i32)`; the arrows → `Step(i32)` | **Done**: owned by the window, forwarded at launch. *query* `setting()` is the window's own `EditView::search_open` (set when it presents, cleared on `Closed`). Test getters `setting()`, `rows()` on the model, `entry()`, `selected(..)` on the widgets. |
| `output.rs` | `OutputPanel` | — | `add_line(&str)` → `OutputPanelInput::AddLine(String)` | **Done**: owns a detached `Controller` (no outputs). |
| `panels.rs` | `ProblemsPanel` | `connect_choose(usize)` → `ProblemsOutput::Choose(usize)`; `connect_source(SourceLocation)` → `Source(SourceLocation)` | `show(&[Report], Option<usize>, bool)` → `Show { reports: Vec<Report>, selected, stale }`; `show_selected(Option<usize>)` → `ShowSelected(Option<usize>)` | **Done**: the window owns the handle, the one owner of the component's `Controller`, launched in `build_widgets` with `.forward()` into the window's leaf channel (`update/selection.rs`, `problems_said`); no `connect_outputs`, no `Bridge`. `#[block_signal]` on row selection in `ShowSelected`. |
| `panels.rs` | `FilesList` | `connect_open(PathBuf)` → `FilesListOutput::Open(PathBuf)` | `set_folder(&Path)` → `SetFolder(PathBuf)` | **Done**: the window owns the handle, the one owner of the component's `Controller`, launched in `build_widgets` with `.forward()` into the window's leaf channel (`update/selection.rs`, `file_list_said`); no `connect_outputs`, no `Bridge`. |
| `sections.rs` | `LinkHandlers<Message>` | the old `connect_click` / `connect_hover` slots are gone: a link sends its owner's own input (`LinkHandlers::new(input, click, hover)`, two message constructors), and the owner tells its owner `<Owner>Output::Link(LinkTarget)` / `Hover(Option<LinkTarget>)` | — | **Done**. Not a component: its widget builders (`link`, `links`, `set_button_words`) are used inside `Inspector`'s and `EditCard`'s views. No `Cell`: the handlers are a sender and two `fn`s, cloned into each link. The stylesheet goes on the display once per process (`std::sync::Once`). |
| `start_page.rs` | `StartPage` | `open` / `create` actions, recent `row.connect_activated`, `window.connect_close_request` → `StartPageOutput::Open`, `Create`, `OpenRecent(PathBuf)`, `CloseRequest` | `ask_new_project()` → `AskNewProject` | **Done**: the application's handle owns the `Controller`, its outputs forwarded at launch as `ShellInput`s (`OpenFolder`, `OpenNewProject`, `StartClosed`). The component asks for a folder itself (`Init::choose_folder`, a `fn` a test replaces) and keeps the New Project dialog in its model while it shows (`NewProjectWritten`, `NewProjectClosed`), so the output callback holding the window weakly is gone. Outputs are now `OpenFolder(PathBuf)`, `OpenNewProject(PathBuf)`, `CloseRequest`. |
| `tab_strip.rs` | `TabStrip`, `TabStripComponent` | `TabStripOutput::Show { panel: PanelId, index }`, forwarded at launch (`TabStrip::new(own, owner, message)`) to the workbench → `WorkbenchInput::OwnTabChosen` → `WorkbenchOutput::ShowOwnTab` → `ProjectInput::ShowOwnTab` | the panels' own tabs as data, from the workbench (`TabStrip::show_own_tabs(OwnTabs)` → `SetOwnTabs(OwnTabs)`); the frame's pages coming or going → `Gather` (the pages' handler only sends); the frame's `page_changed` → `PageChanged` (restyle in place); Ctrl+Page Up/Down on the frame → `Step(Step)` | **Done**. No thread-local registry, no list of strips, no closures: the window's `Workbench` keeps the own tabs (`OwnTabs`, by `PanelId`, read from each page's id) for its dock, gives every frame its strip with a copy (at the end of the update that made the frame) and hands every strip a new copy when they change (`Workbench::show_own_tabs`, from `FilesOutput::Tabs`). The GObject subclass owns the component's controller. |
| `tree.rs` | `TreeSidebar` | `connect_select(Selection)` → `TreeOutput::Select(Selection)`; `connect_source(SourceLocation)` → `Source(SourceLocation)`; `connect_focus(Target)` → `Focus(Target)`; `connect_hover(Vec<Item>)` → `Hover(Vec<Item>)` | `show_tree(PartsTree, PartJoints)` → `ShowTree { tree, joints }`; `show_empty(EmptyState)` → `ShowEmpty(EmptyState)`; `set_placeholder(Option<&str>)` → `SetPlaceholder(Option<String>)`; `show_marks(Rc<Marks>)` → `ShowMarks(Rc<Marks>)`; `show_selection(Option<&Selection>)` → `ShowSelection(Option<Selection>)` | **Done**: the window owns the handle, the one owner of the component's `Controller`, launched in `build_widgets` with `.forward()` into the window's leaf channel (`update/selection.rs`, `tree_said`); no `connect_outputs`, no `Bridge`. Its `syncing` flag and `try_borrow_mut` go: `#[block_signal]` on row selection while filling and syncing. |
| `viewport.rs` | `Viewport` | the GObject signal `resized(u32, u32)` (`connect_resized`, a signal handler like `unmap`'s, which only sends `RenderingInput::Resized`) | `show_frame(gdk::Texture)`; `show_live(LiveFrame) -> Shown` | **Done**. A GObject subclass (the wgpu surface): stays a widget. The Rust callback slot (`on_resize`) is gone. *queries* `scale()`, `device_size()`. Its instance state is an accepted exception (below). |
| `view_tools.rs` | `ViewToolbar` | `ViewToolbarOutput::Xray(bool)`, `Pull(bool)`, `Play`, `Apart(f64)`, forwarded at launch (`ViewToolbar::new(owner, message)`) → `RenderingInput::Xray`, `PullApart`, `PlayExplode`, `ExplodeSlid`; the publication `PoppedOut(bool)` (the slider popped out or went, whoever turned Pull apart on) → `RenderingInput::PullSliderShown` → `CameraToolbar::set_joined` | `show(bool, f32, bool)` → `Show { on, amount, playing }`; `show_xray(bool)` → `SetXray(bool)` (for `--xray`; the window sets its own X-ray with `RenderingInput::Xray`); `offer(bool)` → `Offer(bool)`; `set_orientation` → `SetOrientation(gtk::Orientation)` (the window tells it with the camera column's) | **Done**. `block_signal` on Pull apart and on X-ray. Its slider is launched with `Slider::launch`, forwarded to `ViewToolbarInput::Moved`. Two roots for others to place: `popout` (in the floats) and `section` (X-ray and Pull apart, at the camera column's foot); it turns `section` and its line itself. |
| `view_tools.rs` | `CameraToolbar` | `CameraOutput::Front`, `Turn`, forwarded at launch (`CameraToolbar::new(tools, owner, message)`) → `RenderingInput::LookFront`, `TurnAround` | `set_orientation` → `SetOrientation(gtk::Orientation)`; `set_joined(bool)` → `Joined(bool)` | **Done**. `fit` uses the `win.frame` action, set by the toolbar itself. Places the view tools' `section` at its foot and nothing more: it no longer turns the tools' section and line, nor watches their pop-out's `visible` (the window tells it, from `PoppedOut`). |
| `view_tools.rs` | `PlanSwitch` | `PlanSwitchOutput::FromAbove(bool)`, forwarded at launch (`PlanSwitch::new(owner, message)`) → `SelectionInput::FocusPlan` | `show(bool)` → `Show { plan }` | **Done**. `block_signal` on the toggle. No alignment or margins of its own: the floats place it (as `contact_key()`'s box). |
| `view_tools.rs` | `EmptyView` | — (its `open` button uses the `win.open` action) | `show(&str, &str, bool)` → `EmptyViewInput::Show { title, description, offers_open }`; `show_dark(bool)` → `Dark(bool)` (the window's backdrop no longer styles its root) | **Done**. |
| `workbench.rs` | `Workbench` | `WorkbenchOutput::ToolsChanged(Spots)`, `SaveFailed(String)`, `ShowOwnTab { panel, index }`, `SavedForClosing`, forwarded at launch (`Workbench::new(.., owner, message)`) → `RenderingInput::ToolsMoved`, `ProjectInput::Notice`, `ProjectInput::ShowOwnTab`, `ProjectInput::LayoutSavedForClosing` | `apply(Layout)` → `Apply(Layout)`; `narrow_edges(&adw::Breakpoint, f64)` → `NarrowEdges { breakpoint, width }` (its apply/unapply send `Fold`/`Unfold`); `dock_tool(Group, Spot)` → `DockTool { group, spot }`; `show_problem_count(usize)` → `ShowProblemCount(usize)`; `show_own_tabs(PanelId, Vec<PageTab>)` → `ShowOwnTabs { panel, tabs }`; `save_for_closing()` → `SaveForClosing`, answered by `WorkbenchOutput::SavedForClosing` (→ `ProjectInput::LayoutSavedForClosing`, which goes on closing); a dragged panel → `Save`; `reset()` → `Reset`; `present(PanelId)` → `Present(PanelId)`; `bring_forward_beside_view(PanelId)` → `BringForwardBesideView(PanelId)`; `bring_forward_if_open(PanelId)` → `BringForwardIfOpen(PanelId)`; `edit_panel_hidden()` → `EditPanelHidden` (present Details if Edit shares its frame and shows); `show_status(&str)` → `ShowStatus(String)`; `move_to(PanelId, Side)` → `MoveTo { panel, side }`; a strip's chosen own tab → `OwnTabChosen { panel, index }` | **Done**. libpanel dock. The one keeper of the tool groups' spots and of the panels' own tabs (see `floating.rs`, `tab_strip.rs`). Decisions read from where the panels are ("is Code open?", "do Edit and Details share a frame, and does Edit show?") are messages decided in its own update, after the moves queued before them; the queries `is_open`, `is_shown`, `share_a_frame` and `layout()` are for tests only. *query* `tool_spots()` (a copy, read once by `build_widgets`). The status bar's label is its own (no longer a public field). It places each panel's content and never reaches into it (no `fit_every`). |
| `edit_view/grip_panel.rs` | `GripPanelComponent` (in `GripSection`) | `connect_action(PanelAction)` → `GripPanelOutput::Action(PanelAction)`; (`grips.rs`) `panel.value.connect_activate` → folds into `Action` | `show(&PanelModel)` → `Show(Box<PanelModel>, Option<GripOptions>)` (the grip's solver switches come with it); the window's popdown of the options' menu → `CloseOptions`; the options' switch and "Make it solved" → `PanelAction::Switch { index, ask: SwitchAsk }` | **Done**: the shim is gone. The window's `GripView` owns a `GripSection` (the section around the content and the `Controller`), launched in `EditView::new` with its output forwarded to `EditingInput::GripPanel`. The options (⋮) are the panel's own, made again only for another grip (`options_for` is its model's); the menu is built by `switch_menu`, shared with the grips' own marks on the model. Test helpers `panel_words`, `panel_radios` read the controller's widgets. |
| `edit_view/grip_panel.rs` | `ReadoutComponent` | — | `show(Option<&(String, String)>)` → `ReadoutInput::Show(Option<(String, String)>)` | **Done**: the shim is gone; `GripView` owns the `Controller` (no outputs, so launched with `detach()`). Test helper `readout_words`. |
| `edit_view/changes.rs` | `ChangeCardComponent` | `connect_action(ChangeAction)` → `ChangeCardOutput::Action(ChangeAction)` | `show(&ChangeWords)` → `Show(Box<ChangeWords>)`; people's Enter → `Enter`, presses → `Pressed(ChangeAction)` | **Done**: the shim is gone; the window owns the `Controller` (`ChangeCard` in `EditView::changes`, with the `grip_slot` and `pending_slot` it fills), forwarded at launch to `EditingInput::ChangeAction`. |
| `edit_view/changes.rs` | `DependantsComponent` | `include.connect_clicked` → `DependantsOutput::Include`; `cancel.connect_clicked` → `Cancel` | `show(&str, &[String])` → `Show { item, dependants }`; buttons and Escape → `Answer(bool)` | **Done**: the window owns the `Controller` (`DependantsDialog` in `EditView::changes`), forwarded at launch to `EditingInput::DependantsAnswered`; it shows and hides the backdrop itself. |
| `edit_view/item_tools.rs` | `RepeatRowComponent` | `connect_changed(usize, Axis)` → `RepeatRowOutput::Changed { count, axis, shows }` | `show(usize, Axis, bool)` → `Show { count, axis, axis_shown }`; `shorten()` → `Shorten` | **Done**: the `RepeatRow` shim is gone; each `RepeatFoldComponent` owns its row's `Controller`, forwarded to its own input. `#[block_signal]` on the count spin button and axis drop-down in `Show`. |
| `edit_view/item_tools.rs` | `RepeatFoldComponent` | `connect_unfold()` → `RepeatFoldOutput::Unfold`; its row's `Changed` → `RepeatFoldOutput::Changed { count, axis, shows }` | `show(RepeatFold)` → `Show(RepeatFold)`; the row's `show` → `ShowCount { count, axis, axis_shown }`; `place_apart(&gtk::Box, &gtk::Box)` → `RepeatFoldInit::apart` (`RepeatFoldSlots`); the classes the item's tools put on its root → `RepeatFoldInit::classes` | **Done**: the `RepeatFoldRow` shim is gone. *queries* `asked()`, `is_open()`, `press()`, the row's `count()` and `axis()` are the owner's `RepeatFoldShown`, a plain copy kept beside the controller (its setters return the message to send; `take_change` takes people's change unless a later `ShowCount` replaced it). Owned by the item's tools (`ItemTools`) and the candidate card's repeat controls (`Repeats`). |
| `edit_view/item_tools.rs` | `ItemToolsComponent` | (`connect_item_tools`) `name.connect_editing_notify` (when editing ends) → `ItemToolsOutput::Rename(String)`; `mirror.connect_active_notify` (when switched on) → `Mirror`; `hold.connect_clicked` → `Hold`; `remove.connect_clicked` → `Remove`; its repeat controls' outputs → `EditingInput::ItemRepeat(RepeatFoldOutput)` | what `Window::update_item_tools` fills in → `Show(ItemToolsShown)`; `focus_item_name` → `StartEditing` | **Done**: the window owns both `Controller`s (`ItemTools` in `EditView::tools`: the tools' and their `RepeatFoldComponent`'s, forwarded at launch to `EditingInput::ItemTools` / `ItemRepeat`) and the `RepeatFoldShown` copy; a script's "Repeat…" is `Window::press_item_repeat`. `#[block_signal]` on the name and Mirror while filling. |
| `edit_view/hold.rs` | `HoldCardComponent` | (`connect_hold`) `try_it.connect_clicked` and `reason.connect_activate` → `HoldCardOutput::TryIt(HoldFields)`; `cancel.connect_clicked` → `Cancel` | `show(&Measured, &Measured, &Result<Length, String>)` → `Show { title, now }` (`HoldCardInput::show`); a script's fields → `Fill(HoldFields)` | **Done**: the shim is gone; the window owns the `Controller` (`Hold::card`), forwarded at launch to `EditingInput::HoldCard`. `TryIt` carries what the fields hold. |
| `edit_view/hold.rs` | `Hold` | (`connect_hold`) `from_space.connect_clicked` → `EditMenuOutput::HoldFromSpace` → `EditingInput::HoldFromSpace` | — | **Done**: not a component but the window's state for a hold (`step`, the card's `Controller`). The "Hold a distance from this space…" button is the edit menu's own (`connect_hold` and `Hold::from_space` are gone). |
| `edit_view/repeat_handles.rs` | `Repeats` | the card's stepper (a `RepeatFoldComponent` the window owns, forwarded at launch): `Unfold` → `EditingInput::CardRepeatUnfolded`, `Changed { count, axis, shows }` → `CardRepeatChanged { count, axis, shows }` | `RepeatFoldShown::show`/`show_count` return the `RepeatFoldInput` the window sends | **Done**: plain model state of the window, not a component. The stepper's controller and its `RepeatFoldShown` copy (read at once by scripts) are fields of `Repeats`. The count and boundary handles on the model are gone: boundaries are rows of the Sizes card. A script's `Repeat…` marks the copy asked and queues `CardRepeatUnfolded`, as a click does. |
| `edit_view/grip_layer.rs` | `GripLayer` | — | — | A GObject subclass (layout manager): stays a widget. |

## Phase 2: the application and the window

`app.rs` and `mod.rs` are relm4 components now; their leaves still sit
behind their shims.

### Who owns what

- **The application, `app::Shell`**, is a component with no widget
  (`type Root = ()`). `ui::run` launches it, keeps its controller until
  `run_with_args` returns, and connects GApplication's `startup`,
  `activate` and `open` to `ShellInput`s. `activate` and `open` carry an
  `ApplicationHoldGuard`, dropped once the message is handled, so the
  application can't end between the signal and its first window.
- **The project windows**: the shell's model holds an
  `OpenWindow { id, folder, controller: Controller<Window> }` for each.
  When GTK takes a window out of the application (GtkApplication's
  `window-removed`, which closing emits; a window's `destroy` comes only
  once its last reference goes, so it can't be the signal) the shell
  drops that controller, which shuts the window's component down. Nothing else holds a window, so a closed window is
  freed and an open one can't be (the 960f06d and 0603e47 classes).
  The root is made by `Window::init_root`; the shell takes it from
  `Window::builder().widget()`, fills it with the fallible
  `build_widgets(&root, ..)` and only then launches, destroying the root
  if building failed.
- **The window, `ui::Window`**, is the model: `widgets: Widgets`,
  `session: Session`, `state: State`, `pins: Pins`,
  `watcher: Option<Watcher>`, `editing: EditView`, plus its own
  `ComponentSender`. No `Rc<Window>`, `Weak<Window>` or `RefCell` round
  them; every handler is `&mut self`.
- The window talks to the shell with `WindowOutput`: `GoToFolder` (the
  shell brings the window showing it forward, or answers
  `ProjectInput::OpenHere`), `Opened` (recent projects; which window
  shows what), `Closing { size, maximized, hold }` (remembered; the hold
  keeps the application up until it is saved). The shell
  talks to a window with `WindowInput`s (`ProjectInput::Notice`,
  `ProjectInput::Welcome`).

### The window's groups

`WindowInput` wraps one input enum per group. Each group is a file in
`ui/update/` with its enum, a `wire(window)` called from `Window::init`,
and an `update_<group>` method that `Window::update` hands its messages
to. The handlers themselves are `impl Window` methods in the group's
file (for editing, in the `edit_view` files).

| Group | File | Input | Handlers | Leaf outputs and signals it wires |
|---|---|---|---|---|
| Selection and focus | `update/selection.rs` | `SelectionInput` | `select_and_frame`, `selection_changed`, `show_card`, `show_hint`, `show_moves`, focus mode (`focus_changed`, `open_focus`, `leave_focus`, `update_focus`, `toggle_play_focus`, `focus_tick`, the progress, caption and pictures), posed problems and openings (`pose_problem`, `update_opening`, `motion_moved`, `problem_moved`), `finder_search`, `finder_hit` | actions `back`, `forward`, `next-problem`, `previous-problem`, `find`, `escape`, `show-problem`; `ProblemsPanel` choose/source; `FilesList` open; `TreeSidebar` select/source/focus/hover; `Dock` move and scene (the scene as `RenderingInput::SceneChosen`); `Inspector` focus/source; `Finder` search request and hit; `FocusPanel` done/source/X-ray/tint/fade; the step and swing cards' scrubs and play; `ProblemCard`'s scrub; `Pictures` resized and chosen; `PlanSwitch`; `MotionBar`'s scale |
| Project | `update/project.rs` | `ProjectInput` | `run_check` (and `Checked`), `show_folder`, `show_title`, `result_changed`, `apply_wanted`, `editor_changed`, `sync_files`, `text_linked`, `capture_if_asked`, `check_status_changed`, `build_changed`, `choose_folder`, `go_to_folder`, `ask_about_unsaved`, `open_folder`, `watch`, closing | actions `new-project`, `check`, `search`, `editor`, `show-panel`, `move-panel`, `reset-layout`, `model`, `save`, `format`, `open`, `open-recent`; the folder drop; `FilesPage` link/go/check/show-in-3D/open/notice and its tabs; `Workbench` save failed and saved for closing; `Finder` closed for good (a screenshot's closing); `Inspector` unsaved; the root's `close-request` |
| Editing | `update/editing.rs` | `EditingInput` | in `edit_view.rs` and `edit_view/*.rs`: the menu, the card, the option search, links, grips, changes, holds, item tools, repeats, verbs | `EditMenuComponent` chosen and drawn, `EditCardComponent` action, open short list and links (forwarded at launch, `EditView::new`); the face button; the pointer over the view; `OptionSearchComponent` chosen/closed (forwarded at launch); `Inspector`'s links; `DependantsComponent`, `ChangeCardComponent`, `HoldCardComponent`, `ItemToolsComponent` and the `RepeatFoldComponent`s (forwarded at launch); the space menu's Hold button; the repeat count and boundary drags; `Finder` actions request and action; `GripPanel`; the grips' handles, entries and range menus; the edit menu's, options' and stops' timers and worker threads; the Delete key |
| Rendering | `update/rendering.rs` | `RenderingInput` | `view_changed`, framing and the camera (`frame`, `frame_selection`, `look_from`, `update_orbit`, `pick`, `resized`), drawing (`request_render`, `render_now`, `poll_frame`, `looks`), pins, `pull_apart`, `toggle_play_explode`, `explode_tick`, `export_view`, `scene_changed`, `explode_changed`, `repose`, the backdrop | actions `frame`, `export-view`, `view-*`; `Header`'s view; `ViewToolbar` X-ray, Pull apart, play, slider and the slider popping out; `CameraToolbar` front and turn; the style manager's dark (disconnected in `shutdown`); `Viewport` resized; the view's drags, scroll and keys; the mouse's back and forward buttons |

`dispatch`, `handle` and `toast` stay in `mod.rs`: every group uses
them.

### Conventions inside the window

- A widget closure captures `window.input()` (a clone of the input
  sender) and does `input.emit(GroupInput::X(..).into())`; background
  work uses `input.send(..)` and stops when it fails. Nothing captures
  the window.
- `self.send(message)` queues a message for after the current one (what
  the old code did with `idle_add` or a short timeout "once the signal is
  done").
- Animation and drawing use the view's frame clock through
  `self.on_next_frame(|frame_time| message)`: one tick, one message.
  Playing (`FocusTick`, `ExplodeTick`) asks for the next frame from its
  handler while it plays; `Clock` in `State` keeps the last frame time
  and whether a tick is already asked for, so pausing and playing again
  never runs two clocks.
- Each leaf the window owns is launched in `build_widgets` with
  `.forward(&to_window, <group>::<leaf>_said)`: its outputs go through
  the window's one leaf channel (`Widgets::leaf_outputs`, passed on to
  the window's input by `Window::init`), each made a group's message by
  a mapping function in the group's file. There are no `connect_outputs`
  callbacks and no `Bridge`s in front of them.
- The window's own chrome (the header, the motion bar, the view tools,
  the camera column, the plan switch, the dock's chips, the floating
  controls and the workbench) is launched by `build_widgets` with its
  outputs forwarded (rule 6), each variant mapped there to a group's
  input. They are launched before the window, so they forward into a
  channel of the window's own (`Widgets::leaf_outputs`), which
  `Window::init` passes on to the window's input, in order; the mapping
  functions live in each group's file (`update::rendering::header_said`,
  `view_tools_said`, `camera_said`, `floats_said`;
  `update::selection::motion_said`, `plan_switch_said`, `dock_said`;
  `update::project::workbench_said`). Nothing in front of them takes a
  callback.
- Controls the window also sets (Pull apart, From above, focus mode's
  checks, the motion slider) are set by their own components with the
  handler blocked (`#[block_signal]`, or `block_signal` in
  `update_with_view`), so their outputs are people's changes only. The
  window shows focus mode's options and From above from its model after
  opening focus mode and after each of people's changes, so a change made
  in the same turn as the window's own ends as the model has it.
- Leaf-to-leaf links go through the window too: the floating controls'
  `Orient`/`Docked` and the workbench's `ToolsChanged` are
  `RenderingInput::ToolOriented`, `ToolDocked` and `ToolsMoved`; the
  camera column's orientation goes to the view tools at its foot too
  (`ViewToolbar::set_orientation`), and the view tools' `PoppedOut` is
  `RenderingInput::PullSliderShown`, which tells the camera column to meet
  the slider (`CameraToolbar::set_joined`).
- Owners keep to their children's roots (the definition of done's
  fourth rule): an owner never calls a method on another component's
  child widgets or changes its widget tree; it sends the component a
  message.
  It may place a component's root (or each of its roots: the dock's
  `scene` and `moves`, the view tools' `popout` and `section`), show or
  hide it, and read its own size or visibility (`right_column` reads the
  camera column's width). A decision that depends on a component's
  widgets is made by that component, in order, as a message
  (`WorkbenchInput::BringForwardIfOpen`, `EditPanelHidden`); a breakpoint
  is handed to each component that changes with it, which adds its own
  setters (`HeaderInput::Narrow`, `WorkbenchInput::NarrowEdges`).
- Request and reply (rule 9): the finder's searches go out as
  `SelectionInput::FinderSearch` / `EditingInput::FinderActions` and are
  answered with `Finder::show_hits` / `show_actions`. The option search
  needs no request: the window sends it a copy of the setting it searches
  when it opens it (`Present`) and again as each option's fit is found
  (`Update`), and the dialog lists matches from that copy itself.
- A key handler that must answer GTK at once (take the key or pass it
  on) decides from the widgets' own state, never by asking the model.
  Delete outside a text field is taken only when there is something to
  do, as on main: the window keeps a `win.remove-selected` action
  (`REMOVE_SELECTED`, activating it sends `EditingInput::RemoveSelected`)
  and at the end of every message (`Window::offer_removal`) enables it
  while there is something to remove or the text doesn't read (then
  Delete says why editing in 3D is off), and disables it otherwise. The
  handler takes the key and activates the action when it is enabled, and
  passes the key on to other widgets when it isn't. Whether the text
  reads is worked out when the editor's analysis reaches the window as a
  message (`FilesOutput::Analysed` → `ProjectInput::Analysed`, which
  sets `State::text_blocked`), so no analysis arrives without one. No
  Rust state is shared with the handler.
- Every message payload has `Debug` of its own (`files::CursorName`,
  `stave_ide::Analysis`, `stave_app`'s `FoundFit` and `FoundStops` all
  derive it), so there is no wrapper that hides one.

### Edit card and menu: answers by message

The window owns `Controller<EditMenuComponent>` and
`Controller<EditCardComponent>` as fields of `EditView`, launched in
`EditView::new` with `.forward(window_input, ..)` (rule 6), so their
outputs arrive as `EditingInput`s; `edit_demo.rs` keeps its showcase's
controller in `EditView::showcase` the same way, its outputs forwarded
into the window's input as `EditingInput::Showcase`, whose handler only
prints them (the fixture's ids are illustrative, so nothing acts on them). Nothing reads a card or
a menu back after showing it. Every question the old synchronous
`EditCard` answered became one of two things:

- **State the window already decides.** `draw_edit` builds the
  `CardView` it sends, and `CardView::offers()` works out from it, with
  the same functions the card draws with, what that card offers: which
  settings have a short list and whether the put-back button shows
  (`CardOffers`). `draw_edit` returns it (`Drawn::Card(offers)`) and the
  window keeps it in `EditView::card_offers`. So `open_options(..) ->
  bool` (an edit script's `choices`) is answered by the window from
  `card_offers` before it sends `OpenOptions` and
  `ScrollToOpenOptions`, and `put_back_button()` (the script's `put
  back`) by `card_offers.put_back()`, after which the window queues the
  same `CardAction::PutBack` the button sends. Which card or menu shows
  is the window's too: it sets the roots' visibility itself, at once,
  because `EditPanel::update` and the option search read it straight
  away.
- **A message from the card when people change it.** Which short list is
  open is people's to change, so the card says so whenever it changes
  (`EditCardOutput::OptionsOpen(Option<String>)`, also when the window
  opened it) and the window keeps the copy in
  `EditView::card_open_options`, read when it starts the options' check
  (the open setting's options are checked first). The menu says when it
  has drawn what it was shown (`EditMenuOutput::Drawn`): the window places
  the menu by its height then, which it can't measure before the menu has
  drawn.

The option search went the same way: the window remembers which setting
it opened it on (`EditView::search_open`, cleared on
`OptionSearchOutput::Closed`) instead of asking it, and gives it a copy
of the setting to search instead of answering its queries.

`EditCardOutput::OptionsOpened(bool)` is gone: nobody needed the answer
once the window knew it. Tests launch the components with
`Component::builder().launch(..).connect_receiver(..)`, call the same
`draw_edit`, `settle()` and then read the components' models and widgets
(`edit_view/tests.rs`, `Launched`).

### Request and reply

`FilesPage` was the window's biggest synchronous partner: it asked it
questions whose answers changed what it did next (`save_all`'s success,
`has_unsaved`), read its open buffers straight after writing to them (a
check reads `overlay_for_check`; an edit's commit opened tabs through
`buffer_for`), and the inspector and the tab strip asked it from inside
their own drawing. Each kind of question has its own answer; reuse them
for later conversions, in this order of preference.

1. **The answer comes back as a later message, carrying the asker's
   continuation.** When the answer depends on the order of what came
   before (a change typed just before Save; Don't Save just before
   closing), ask the component by message and let it hand back what was
   waiting. The component is generic over the owner's continuation,
   `FilesComponent<Then>`, so it never names the window's types; the
   window uses `Then = Box<WindowInput>`:

   ```text
   close-request ─▶ CloseAsked ─▶ FilesInput::CountUnsaved { then: CloseNow }
       ◀─ FilesOutput::UnsavedCounted { count, then } ─▶ ProjectInput::UnsavedCounted
          count == 0 ─▶ send(then)                         (CloseNow: destroy)
          else the question ─▶ UnsavedAnswered { answer, then }
             Save    ─▶ FilesInput::SaveAll { then: Some(then) }
                 ◀─ FilesOutput::Saved { ok, then } ─▶ ProjectInput::FilesSaved
                    ok ─▶ send(then); not ok ─▶ a dialog says why, nothing goes on
             Discard ─▶ send(then)                         (CloseNow: forget, destroy)
             Stay    ─▶ nothing
   ```

   Requests go through the component's own input queue, which is FIFO, so
   the answer reflects every message sent before it, including a
   `TextChanged` a buffer's `changed` signal queued during the window's
   own update. The window keeps a `closing` flag so a close asked again
   while the first waits for its answer does nothing (the old code closed
   synchronously, so the second request found the window gone).

2. **Immutable, reference-counted objects the component publishes and
   the owner keeps.** What the window must read inside one of its own
   updates (synchronously, in the middle of `session.apply`) comes from
   its own copies, never from the component's model:
   - `FilesOutput::Documents(Vec<(Document, gtk::TextBuffer)>)` when a tab
     opens or they all close; GObjects are reference-counted, so the shim
     keeps clones. The component keeps each buffer's own `modified` flag
     equal to "has unsaved changes" (it clears it on open, save, reload
     and forget, and sets it to the dirty state after each change; GTK
     sets it the instant text changes). So `FilesPage::edit_files` (the
     overlay a check or an edit reads) is built from the live text of the
     modified buffers, exactly what `Buffers::overlay` would give, with
     no lag.
   - `FilesOutput::Analysed(Rc<Analysis>)`: it reaches the window as
     `ProjectInput::Analysed`, whose handler gives it to `FilesPage`
     (`set_analysis`; the window reads it with `analysis()`) and works out
     whether the text reads (`State::text_blocked`, for Delete).
   - The tag table is given to the component at launch (`FilesInit`), so
     the owner can make a buffer itself.

   When the owner needs a buffer that doesn't exist yet (an edit's commit
   writing a file with no tab while the editor shows; `reveal` followed
   at once by marking the revealed file's lines), the owner makes it
   (`editor::new_buffer`), keeps it, and hands it over with
   `FilesInput::Adopt`. The edit writer (`edit_writer.rs`) is rebuilt by
   `Window::dispatch` before every Add from the shim's buffers and
   whether the editor shows (no shared cell), and reports buffers it made
   through an `mpsc` channel that `dispatch` drains straight after
   `session.apply` (`FilesPage::take_written`), before the events are
   handled.

3. **Changes pushed, not asked for.** A query another leaf made from
   inside its own drawing becomes a message sent when the answer changes:
   `FilesOutput::Unsaved(set)` → `Inspector::show_unsaved` →
   `InspectorInput::Unsaved`; `FilesOutput::Tabs(tabs)` →
   `Workbench::show_own_tabs(PanelId::Code, tabs)`, which hands every
   strip of the window's dock a copy, each gathering its tabs in its own
   update. A strip's "show this tab" is its output, forwarded to the
   workbench, which tells the window (`WorkbenchOutput::ShowOwnTab` →
   `ProjectInput::ShowOwnTab`).

Never call `bridge.model()` from the owner's handlers to answer one of
these (rule 8), and never put a `RefCell` round the shared answer.

### Behaviour that changed on purpose

- Everything the window does in answer to a widget now happens when the
  main loop next runs, after the signal has returned.
- A frame ready on the GPU is shown from a message after the frame-clock
  tick that found it, so it reaches the screen one frame later than
  before. A frame asked for only by `on_next_frame` therefore draws
  nothing; GTK 4.24's Broadway backend warns "Duplicate call to
  gdk_frame_timings_submitted()" about such a frame, so on Broadway the
  tick also redraws the view (`frames_must_draw` in
  `update/rendering.rs`).
- A grip's typed value, a pick from a grip's sizes and a drag's next
  value were sent after a short timeout or an idle; they are queued
  messages now.
- The tab strip restyles its tabs for another page shown (main's
  `mark_shown`) from a `PageChanged` message, not inside the frame's
  `page_changed`; the tabs are not made again, so the focused tab keeps
  the keyboard as on main. Ctrl+Page Up/Down decide at once, from the
  frame's page count, whether to take the key (fewer than two pages
  pass it on, so the editor keeps its own); the step itself is a
  `Step` message.
- The option search opens, lists, moves its selection and chooses from
  queued messages; an edit script's `options` and `search` steps open it
  before the picture is taken. Its arrow keys and Escape are still taken
  at once. Opened by a script, its field now keeps the keyboard in the
  picture (the field shows focused, as when people open it); on `main` it
  had lost the keyboard again by the time the picture was taken. Now the
  search opens after the card's redraws the same step queued.
- The edit menu and card draw what they are shown when the main loop
  next runs, after the window has already shown or hidden them; the menu
  is placed by its new height once it has drawn (`EditMenuOutput::Drawn`),
  a turn of the main loop after it appears where it will be placed. It
  places itself: the window sends `EditMenuInput::Place` (where it was
  asked for, and the view's size), which the menu handles after any
  `Show` sent before it, so each placing now lands when the main loop
  next runs rather than at once. An
  edit script's `choices` opens the short list, and its `put back`
  presses the button, from queued messages, both before the picture is
  taken.
- The Code panel's setters are queued. The window's close, open-folder
  and new-project flows ask how many files are unsaved and save them by
  message ("Request and reply"); closing after Don't Save tells the
  application the size once per close asked, not once more as it goes
  on. `--edit-script`'s `save` step queues the saving and goes on; a
  failure says so in its dialog rather than stopping the script. The
  hidden row of tab buttons inside the Code panel (never shown since the
  tabs moved to the strip) is gone.
- A narrow window's side edges fold and unfold from `Fold`/`Unfold`
  messages that the breakpoint's `apply`/`unapply` send, so they move
  when the main loop next runs rather than inside the breakpoint's
  signal; `layout()` still reports a folded edge as it was before.
- Focus mode's step and swing cards show play or pause from a queued
  `ShowPlaying` (with their slider's progress, also queued), not from the
  window setting the buttons' icons and tooltips at once.
- The window's chrome (header, view tools, camera column, plan switch,
  motion bar, dock, floating controls, workbench) reaches the window
  through `Widgets::leaf_outputs`, one more step on the main loop than
  a callback that sent the window a message; the order is kept. An
  output sent before the window is launched waits there rather than
  being dropped.
- A tab strip gathers its tabs in its own update, at the main loop's
  default priority (still before GTK lays out and draws), instead of in
  a high-priority idle shared by every strip; the panels' own tabs are
  kept per window, by the window's workbench, rather than in one
  registry for the process. A frame the workbench makes gets its strip
  at the end of the update that made it, as frames libpanel makes did
  already.
- `Viewport::connect_resized` connects a handler to the `resized`
  signal instead of replacing the one handler; each caller connects
  once.
- The editor's analysis reaches the window as a message
  (`ProjectInput::Analysed`) instead of being stored by the Code panel's
  shim as the output was delivered, so a window message already queued
  before it still sees the analysis before. Delete's take-or-pass is read
  from the `remove-selected` action's state, set at the end of each
  message.
- A name link's click and the pointer on it, in the Details panel, go to
  the inspector and from it to the window, so the window hears them a
  turn of the main loop later than when the link called the window's
  callback.
- The question about unsaved changes keeps what waits on it in the
  window's state; the dialog is modal, so there is never more than one.
- A go-to's scroll gives up after `SCROLL_WAIT_FRAMES` counted on the
  view's frame clock from the go-to; asked while the view had no frame
  clock yet (never shown), it waits until the text is laid out or a
  newer go-to replaces it.
- The New Project action while its dialog shows does nothing (the
  dialog showing is the one people answer); before, a second dialog
  could not be asked for either, since the first was modal.
- The start page's Open and New Project are done by the page itself
  rather than by the application's callback; the application hears
  only which folder to open.
- Closing asks the workbench to save the layout by message
  (`WorkbenchInput::SaveForClosing`), so the layout saved includes every
  change the workbench was told before; the window asks about unsaved
  changes when the workbench answers (`SavedForClosing`), two turns of
  the main loop later than before. A screenshot's closing saves no layout
  (its workbench never wrote one).
- A lens's link in the Code panel reaches the component through the
  editor's `LensClick` channel, one turn of the main loop later than when
  the link called the component's sender itself.
- `--xray` sets the X-ray toggle through the toolbar's input and the
  window's X-ray by its own queued message, instead of toggling the
  button and hearing it back.
- The selected item's "Repeat…" counts as asked for when the window
  handles its `RepeatFoldOutput::Unfold`, not inside the button's click;
  the window redraws the tools from that same message, as before. A
  script's press (`Window::press_item_repeat`) marks it at once and
  queues the same message.
- Whether the Code panel is open before it is brought forward (a part
  chosen), and whether the Edit panel shares a frame with Details and
  shows when its card goes, are decided by the workbench when it handles
  `BringForwardIfOpen` / `EditPanelHidden`, after the moves queued before
  them. The window used to ask the dock straight away, before a `Present`
  or `MoveTo` it had just queued was made, and could act on the dock as
  it was.
- The header's breakpoint setters are added by the header when it
  handles `HeaderInput::Narrow`, as the workbench's were already, rather
  than by the window as it built the breakpoints; both are in place
  before the window is first laid out.
- The camera column meets Pull apart's slider square (`joined`) when the
  window passes on the view tools' `PoppedOut`, a turn or two of the main
  loop after the slider shows, instead of from a `visible` handler the
  column put on the tools' pop-out.
- A `--window-screenshot` with the finder open closes the finder by
  message (`FinderInput::ForceClose`) and closes the window when the
  finder answers (`ProjectInput::FinderClosedForGood`), one turn of the
  main loop after the picture is saved, instead of at once.
- The floating controls get the tool groups' spots from the window's
  `Floats::refresh` once the workbench is made, in the same turn of the
  main loop as before, instead of at launch. A dragged group's float is
  faded and the group inside it takes the shadow, so the shadow keeps the
  group's rounded corners; the pictures are the same.

### The grips, the readout and the repeat handles

- `GripView` owns the size section's `Controller<GripPanelComponent>`
  (inside a `GripSection`, launched in `EditView::new` with its output
  forwarded to `EditingInput::GripPanel`) and the readout's
  `Controller<ReadoutComponent>`; the `GripPanel` and `Readout` shims and
  their `connect_action` are gone.
- The grip's options (⋮) in the size section are the panel's own: the
  window sends the grip's solver switches with `Show`
  (`GripOptions { index, switches }`), the panel makes the menu again
  only for another grip, and what it asks comes back as
  `PanelAction::Switch { index, ask }` (`SwitchAsk::MakePlain` /
  `MakeSolved`). Once the window has read the range it closes that menu
  with `GripPanelInput::CloseOptions` (`SwitchMenu::Panel`); a mark's own
  menu on the model, the window's widget, it pops down itself
  (`SwitchMenu::Mark`). Both menus are built by `grip_panel::switch_menu`.
- The grips' lines are drawn from a copy: each time the grips are placed,
  the window hands the drawing area a new draw function owning that
  frame's `Vec<Drawn>` (the old `Rc<RefCell<Vec<Drawn>>>` is gone).
- `Repeats` holds the card's `RepeatFoldComponent` and its
  `RepeatFoldShown`; its drag and the handles' nudges are plain fields.
- `connect_editor_link` (empty) is gone; `connect_grips` and
  `connect_repeats` only put their overlays and gestures over the 3D
  view, with no output callbacks.

### The change card, the hold card and the item's tools

- `EditView` owns their controllers, launched in `EditView::new` with
  `.forward(window_input, ..)`: `Changes` holds the change card's
  (`ChangeCard`, beside the `grip_slot` and `pending_slot` the window
  fills) and the removal dialog's (`DependantsDialog`, beside its root);
  `Hold::card` is the hold card's; `ItemTools` holds the tools' and their
  `RepeatFoldComponent`'s, with the window's `RepeatFoldShown` copy. Their
  `Bridge` shims and `connect_action`, `connect_answered`, `connect`,
  `connect_unfold` and `connect_changed` are gone; setters are messages
  the window sends to the controllers, and it shows and hides the roots
  itself, at once.
- The space menu's "Hold a distance from this space…" is the edit
  menu's own button: `Show { from_space, .. }` shows it for a space,
  and its press is `EditMenuOutput::HoldFromSpace`, which the window
  hears as `EditingInput::HoldFromSpace`.
- The changes' and the grips' stylesheets are put on the display once,
  by the application at startup (`Shell::startup`, beside
  `style::install`), so they need no thread-local guard. Tests put the
  grips' on the GTK test thread's display once, when that thread starts
  (`gtk_test::on_gtk_thread`).

### Done by the group lanes and the integration

- Every handler is an `impl Window` method on `&mut self`: closing, the
  unsaved-changes question and `--window-screenshot` answer with messages
  (`ProjectInput::CloseAsked`, `UnsavedAnswered`, `CaptureTick`).
- `Window::controls`, `block_controls` and `unblock_controls` are gone;
  the motion bar is a component (`chrome::MotionBarComponent`).
- The selection, rendering and project leaves reach the window by their
  outputs (forwarded at launch); the `Slot`s and callback structs of
  `tree.rs`, `focus_stage.rs`, `panels.rs`, `inspector.rs`, `finder.rs`
  and `workbench.rs` are gone, as are `ToolSpots`, the Finder's
  `Rc::new_cyclic` and the weak links between the workbench and the
  floating controls.
- `EditView`'s state, `pending_marked`, `verbs`, `immediate`,
  `panel_shown` and `options`, the grips' state, the hold's step, the
  changes' asked item and the item tools' offer are plain fields of the
  model.

### Leaks found at the end, and fixed

All three were on `main` before the move, not caused by it.

- The centre's dock frame (and with it the Model and Code panels, about
  260 widgets) outlived every closed window. A `create-frame` handler
  returned a frame made in Rust, which is already sunk; libpanel sinks
  it again when it places it, so one reference was never dropped. The
  dock now lets libpanel make its own frames and gives them their tabs
  afterwards (`Dockyard::give_tabs`). `closed_window_is_freed` now
  checks that every widget in the window is freed.
- A grip's options menu was held by its own switch's and button's
  handlers, which live in its popover: a cycle, so every menu that was
  replaced stayed alive. The handlers hold it weakly now.
- The view switcher's two toggle children (an icon and a label each)
  outlived every closed window. libadwaita's `AdwToggle` takes a
  reference to its child and never drops it (its finalize leaves `child`
  alone; 1.10.0 and `main`), as a plain `adw::ToggleGroup` shows
  (`libadwaita_leaks_a_toggles_child`). The header keeps its toggles and
  unsets their children when the switcher is destroyed
  (`free_children_on_destroy`), and `closed_window_is_freed` no longer
  lets them off. A draft upstream report is in
  `docs/reviews/upstream-adw-toggle-leak.md`.

### Accepted exceptions

State that stays outside a component's model, each for a reason the
rules can't remove:

- `viewport.rs`, `imp::Viewport`'s `frame`, `sizing`, `reported` and
  `settling` (`Cell`s): the instance state of a GObject subclass. GTK
  calls `snapshot`, `measure` and `size_allocate` with the instance
  shared (`&self`), and the 3D view must be a widget that draws in
  `snapshot`, so its state lives in the instance, read and written only
  by the widget's own methods and its own settle timer (which holds the
  widget weakly). Nothing else shares it, and nothing is borrowed while
  it emits `resized`.
- `empty_state.rs`, `imp::PageFit`'s `page`, `hidden_icon`,
  `icon_room` and `queued` (a `WeakRef` and `Cell`s): the instance
  state of a GObject subclass, the same kind as `Viewport`'s. A status
  page's icon must go when its frame is too short and come back when
  it grows, which needs the hidden icon's name and the height it took
  kept between layouts. `PageFit` is the page's own scrolled window's
  vertical adjustment, so the widget tree owns it; GTK calls its
  `changed` method with the instance shared (`&self`), and the refit it
  waits to run after the layout pass holds it only weakly. Only its own
  methods read or write the cells, and nothing else shares them.
- `tab_strip.rs`, `imp::TabStrip::tabs` (a `OnceCell` holding the
  strip's `Controller<TabStripComponent>`): libpanel takes a frame's
  header as a GObject implementing `PanelFrameHeader`, whose methods
  (`page_changed`, the `frame` property) take the instance shared. The
  cell is set once, by `TabStrip::new`, and only read to send the
  component a message. The component holds its frame and its pages as
  `glib::WeakRef`s (GObject widgets, not Rust structs): the frame owns
  the strip, which owns the component, so a strong reference would be a
  cycle.
- `icons.rs`, `imp::PathIcon`'s `shape` and `size` (`Cell`s): the
  instance state of a GObject subclass, for the same reason as the
  viewport's: GTK calls `snapshot` and `measure` with the instance
  shared, and the icon is a widget used inside components' views (set
  through its own `set_shape`, often from a `#[watch]` line). Nothing
  else holds it.
- Owner-created slots (editing and its panels): an empty `gtk::Box` (or
  another component's root) that an owner makes and hands to a component
  in its `Init`. The component only places it in its view and never
  calls anything on it or reads it; the owner (or the component it gives
  the slot to) fills it. This is how two components share a spot in one
  card without either reaching into the other, and one of the two
  exceptions to "owners never touch a component's widget tree" (the
  other is the window-level slots, below): what is filled in was the
  owner's from the start. Each one:
  - `edit_card.rs`, `CardSlots` (`EditCardComponent`'s `Init`): made by
    `EditView::new`. `repeat_button_slot` and `repeat_slot` are handed on
    as the candidate card's `RepeatFoldSlots`, which that
    `RepeatFoldComponent` fills with its own button and row;
    `pending_slot` is filled by the window with the Pending section
    (`editor_link.rs`, `PendingPanel::move_into`).
  - `edit_view/changes.rs`, `ChangeCardInit`'s `grip_slot` and
    `pending_slot` (clones kept in `ChangeCard`): the window puts the size
    section (`GripSection`'s own root, `grips.rs`, `show_grip_section`)
    and the Pending section (`editor_link.rs`) in them. The Pending
    section moves between this slot and the edit card's.
  - `edit_view/item_tools.rs`, `RepeatFoldSlots`
    (`RepeatFoldInit::apart`): the candidate card's slots above, which
    the fold fills with its own widgets.
  - `inspector.rs`, `InspectorInit::focus`: focus mode's panel's root
    (`FocusPanel`, owned by the `Inspector` handle beside the inspector's
    controller), placed on the focus page.
  - `edit_view/item_tools.rs`, `ItemToolsInit::repeat`: the item's
    tools' `RepeatFoldComponent` root (owned by the `ItemTools` handle
    beside the tools' controller), placed after the name.
- Window-level slots: a component given another component's root at
  launch places it in a container of its own, and does nothing else
  with it. The window launches its chrome before itself and hands these
  roots over in `build_widgets`: `Floats` takes the 3D view and every
  floating control (`FloatsInit`) and puts each control in a float of
  its own (`adw::Bin`), which it alone places, styles, drags and gives
  the "Press and hold" tooltip; `CameraToolbar` takes the view tools'
  `section` (`CameraInit::section`) and appends it at its foot;
  `Workbench` takes each panel's content and sets it as its panel's
  child. The component taking a root never sets its alignment, margins,
  classes, controllers, tooltip or orientation, never reaches into it,
  and doesn't show or hide it: that stays with the root's own component
  (each floating control has no alignment or margins of its own, so the
  float's place is its place). The root's owner keeps the component
  (the window keeps both), so a root may outlive the slot's component
  only while the window holds it. This is placing, not reaching in: it
  is how relm4's own `#[local_ref]` children and `ComponentController`
  roots are put in a parent, done across two components because the
  window, not the slot, owns both.
- The static `OnceLock`s holding the `ParamSpec`s and signal definitions
  of the GObject subclasses (`viewport.rs`, `tab_strip.rs`): GObject's
  own class data, made once per process, holding no state. Likewise the
  `std::sync::Once` guards that put a stylesheet or style schemes on the
  default display once a process (`edit_card.rs`, `sections.rs`,
  `tree.rs`, `editor.rs`).
- `edit_view.rs`, `EditView::options` (the options' check's cancel flag
  with its ticket), the `stop` flag of the edit menu's check and the
  options' `cancel` flag, and `edit_view/grips.rs`, `GripView::cancel`
  (the stops' search) (`Arc<AtomicBool>`s): cross-thread stop tokens.
  `stave_app`'s jobs (`check_edit_menu`, the options' and the stops'
  `job.run`) take `&AtomicBool` and poll it to stop early. The options'
  and stops' jobs run on a worker thread and the window keeps its side,
  raised by its own update when a newer job replaces the running one;
  the edit menu's check runs in the window's update and a timer thread
  raises its flag when its time budget is spent. A flag says only
  "stop"; it is not UI state, and what the jobs find comes back as
  messages (`OptionFitFound`, `GripStopsFound`), the options' with a
  ticket, so a stale answer is ignored.
- `render/` (the wgpu renderer below the 3D view, not UI state), all of
  it shared across threads with GPU callbacks: `render/mod.rs`'s
  `Arc<Mutex<Trouble>>` (wgpu's device-lost and error callbacks, which
  run on wgpu's threads, record the trouble the renderer reports next
  frame); `render/dmabuf.rs`'s `Pool` (`Arc<Mutex<VecDeque<Image>>>`, the
  exported images a `gdk::Texture`'s release callback hands back) and
  `refused` (`Arc<AtomicBool>`, set when GTK refuses a frame as a
  dmabuf, so later frames take the readback path) and its static `OnceLock` (the environment's choice, read once);
  `render/readback.rs`'s `Spares` (`Arc<Mutex<Vec<wgpu::Buffer>>>`, the
  readback buffers a `map_async` callback returns); `render/profile.rs`'s
  `static EDIT: Mutex<Option<Edit>>` and `ENABLED` `OnceLock` (one
  process-wide slot timing an edit from its check's start to the first
  frame showing it, only with `STAVE_PROFILE`; the renderer and the check
  note their times without a window to hand). These are the GPU,
  threading and profiling boundary: the callbacks are wgpu's and GDK's,
  `Send`, and can't be messages to a component. The scope is exactly `crates/stave-gtk/src/render/`; nothing
  under `ui/` shares state this way.

### Still to do

Nothing.
