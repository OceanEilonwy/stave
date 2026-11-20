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
    PointedAt { x: f64, width: f64 },   // people pressed or dragged
    Stepped(Step),                      // people pressed an arrow, Page, Home or End key
    SetValue(f64),                      // the window moves it: no output
    SetRange { lower: f64, upper: f64 },
    SetBlockedFrom(Option<f64>),
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

**The shim.** `Slider` keeps the old API (`new`, `root`,
`connect_changed`, `value`, `set_value`, `set_range`, `set_blocked_from`,
`set_value_text`):

```rust
pub struct Slider {
    pub root: gtk::DrawingArea,          // the component's root widget
    bridge: Bridge<SliderComponent>,     // owns the Controller
}

impl Slider {
    pub fn connect_changed(&self, callback: impl Fn(f64) + 'static) {
        self.bridge.connect(move |output| match output {
            SliderOutput::Changed(value) => callback(value),
        });
    }

    pub fn set_value(&self, value: f64) {
        self.bridge.send(SliderInput::SetValue(value));
    }
}
```

It is no longer `Clone` (nothing cloned it): a `Controller` has one owner.

**What changed for callers.** Nothing in their code. Two behaviours did,
on purpose: setters are queued, so they take effect when the main loop
next runs, after the caller has returned; and the change callback runs
from the main loop after the slider's update has finished, never from
inside it. `value()` is the value as of the last handled message.

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
| `slider.rs` | `Slider` | `connect_changed(f64)` → `SliderOutput::Changed(f64)` | `set_value` → `SetValue(f64)`; `set_range` → `SetRange { lower, upper }`; `set_blocked_from` → `SetBlockedFrom(Option<f64>)`; `set_value_text` → `SetValueText(String)` | **Done** (Phase 0). People's input: `PointedAt { x, width }`, `Stepped(Step)`. `value()` reads the model. |
| `chrome.rs` | `MotionBar` | (mod.rs) `scale.connect_value_changed` → `MotionBarOutput::Moved(f64)` | `show(Option<(&str, &str, &str, f64, f64)>)` → `Show(Option<MotionShown>)`; `orientation_setter()` → `SetOrientation(gtk::Orientation)` | `#[block_signal]` on the scale's value when `Show` moves it. |
| `dock.rs` | `Dock` | `connect_move(AssemblyId)` → `DockOutput::Move(AssemblyId)` | `offer(bool)` → `Offer(bool)`; `moves_orientation_setter()` → `SetMovesOrientation(gtk::Orientation)` | Built from the header's scene button and the `MotionBar`: launch after them, or own both. |
| `edit_card.rs` | `EditCardComponent` | `connect_action(CardAction)` → `EditCardOutput::Action(CardAction)`; its links → `Link(LinkTarget)`, `Hover(Option<LinkTarget>)`; the open short list → `OptionsOpen(Option<String>)` | `show(&CardModel, &dyn Words)` → `Show(Box<CardView>)` (owned); `update_options(&Candidate)` → `UpdateOptions(Box<Candidate>)`; `open_options(&str) -> bool` → `OpenOptions(String)` (the bool is the window's own `CardOffers`); `scroll_to_open_options_now()` → `ScrollToOpenOptions` | **Done**: the shim is gone; the window owns the `Controller` (see "Edit card and menu: answers by message"). `Init` is `CardSlots` (the repeat and Pending boxes the window fills). Test getters on the model (`entries`, `option_lists`, `fix_buttons`, `put_back_button`) and the widgets (`buttons`, `name_field`, `scrolled`). |
| `edit_menu.rs` | `EditMenuComponent` | `connect_chosen(Declaration)` → `EditMenuOutput::Chosen(Declaration)`; `EditMenuOutput::Drawn` after each `Show` | `show(Heading, &[MenuItem])` → `Show { heading, items: Vec<MenuEntry> }` (owned) | **Done**: the shim is gone; the window owns the `Controller`. Test getter `rows()` on the model. |
| `edit_panel.rs` | `EditPanel` | — | `update() -> bool` → `Update`, the bool becomes `EditPanelOutput::Shown(bool)` | *query* `is_empty()`. Built from other cards' widgets: becomes part of its owner's view. |
| `editor.rs` | `Editor` | a lens's links → the one `Fn(LensClick)` given to `Editor::new`, which sends `FilesInput::ListUses` / `ShowIn3d` | — (called by its owner's `update`) | **Done** (with `files.rs`): the view of `FilesComponent`, owned by its model and called only from its `update`, so `show_buffer`, `go_to` and the cursor queries take effect at once inside that update; its search bar is the component `EditorSearch`. GObject subclasses `Marks`, `CodeView`, `BlockIndenter` stay widgets. `EscapeProbe` answers "is anything open over the text?" from the widgets alone, for the Escape key handler. `new_buffer(table, text)` makes a buffer sharing the tags. |
| `files.rs` | `FilesPage` (shim) / `FilesComponent<Then>` | `FilesOutput::Link(CursorName)`, `Go { from, to }`, `Check`, `ShowIn3d { analysis, symbol }`, `Open`, `Notice(String)`; replies `Saved { ok, then }`, `UnsavedCounted { count, then }`; publications `Documents(Vec<(Document, TextBuffer)>)`, `Analysed(Rc<Analysis>)`, `Unsaved(BTreeSet<PathBuf>)`, `Tabs(Vec<PageTab>)`; `NotInText` | `FocusText`, `SetFolder(PathBuf)`, `Save`, `Format`, `SaveAll { then }`, `CountUnsaved { then }`, `ForgetUnsaved`, `Search`, `ShowMarks`, `ShowResult`, `ShowLocation`, `Reveal`, `Wish`, `Restyle { dark }`, `OpenDocument`, `ShowTab(usize)`, `Adopt { document, buffer }` | **Done**. No `RefCell`, no `Rc<Self>`, no `Weak`: timers and the worker thread send messages with tickets (a stale timer's message is ignored, so no fired timer is ever removed), programmatic cursor moves block the buffer's cursor handler. The window's synchronous reads (`edit_files`, `overlay_for_check`, `buffer_for`, `analysis`) are answered by the shim from what the component published; see "Request and reply". |
| `finder.rs` | `Finder` | `connect_search(&str) -> Vec<Hit>` → request `FinderOutput::Search(String)` / reply `FinderInput::Hits(Vec<Hit>)`; `connect_actions(&str) -> Vec<Action>` → `FinderOutput::SearchActions(String)` / `FinderInput::Actions(Vec<Action>)`; `connect_hit(Hit, Choice)` → `FinderOutput::Hit { hit, choice }`; `connect_action(usize)` → `FinderOutput::Action(usize)` | `present(&impl IsA<Widget>, &str)` → `Present { parent: gtk::Widget, query: String }` | Rule 9 for the two searches: the shim's `connect_search_request` / `show_hits` and `connect_actions_request` / `show_actions` are that request and reply, and the window uses them. `FinderInput::ListenForHits`: until it arrives, choosing a hit does nothing (`connect_hit` sends it); a parent that forwards `FinderOutput::Hit` itself sends it once at launch. Root is an `adw::Dialog`. |
| `floating.rs` | `Floats` | constructor `orient: Orient` → `FloatsOutput::Orient { group, orientation }`; constructor `on_dock: OnDock` → `FloatsOutput::Docked { group, spot }` | `refresh()` → `Refresh` | Shares `Rc<RefCell<Spots>>` with `Workbench`: rule 11. Takes the view's overlay and the floating widgets at launch (`Init`). |
| `focus_panel.rs` | `FocusPanel` | `connect_source(SourceLocation)` → `FocusPanelOutput::Source(SourceLocation)`; (mod.rs) `done.connect_clicked` → `Done`; `xray`, `tint`, `fade` `connect_active_notify` → `Xray(bool)`, `Tint(bool)`, `Fade(bool)` | `show(Option<&Focus>)` → `Show(Option<Focus>)` | `#[block_signal]` on the three checks when `Show` sets them. Owned by `Inspector`. |
| `focus_stage.rs` | `StepCard` | (mod.rs) `play.connect_clicked` → `StepCardOutput::Play`; `scrub` `Changed` → `StepCardOutput::Scrubbed(f64)` | `show_frame(Option<&str>, &str)` → `ShowFrame { step: Option<String>, status: String }`; (mod.rs) `scrub.set_value` → `SetProgress(f64)` | Owns a slider: forward `SliderOutput::Changed`. |
| `focus_stage.rs` | `SwingCard` | (mod.rs) `play.connect_clicked` → `SwingCardOutput::Play`; `scrub` `Changed` → `Scrubbed(f64)` | `show_name(&str, bool)` → `ShowName { name, turns }`; `show_value(&str)` → `ShowValue(String)`; (mod.rs) `scrub.set_value` → `SetProgress(f64)` | Owns a slider. |
| `focus_stage.rs` | `Caption` | — | `show(&str, &str)` → `CaptionInput::Show { text, hint }` | |
| `focus_stage.rs` | `Pictures` | `connect_resized()` → `PicturesOutput::Resized`; the `on_choose` closure given to `show` → `PicturesOutput::Chosen(usize)` | `show(&[String], on_choose)` → `Show(Vec<String>)`; `show_textures(Vec<gdk::Texture>)` → `ShowTextures(Vec<gdk::Texture>)`; `show_current(Option<usize>)` → `ShowCurrent(Option<usize>)` | *query* `device_size()`. |
| `focus_stage.rs` | `ProblemCard` | `scrub` `Changed` (mod.rs) → `ProblemCardOutput::Moved(f64)` | `show(Option<&MotionCard>, f32)` → `Show { card: Option<MotionCard>, amount: f32 }`; `show_value(&MotionCard, f32)` → `ShowValue { card, amount }` | Owns a slider. |
| `header.rs` | `Header` | (mod.rs) `switcher.connect_active_name_notify` → `HeaderOutput::View(View)`; `scenes.connect_row_activated` → `HeaderOutput::Scene(usize)` | `show_title(&str, &str)` → `ShowTitle { title, folder }`; `show_view(View)` → `ShowView(View)`; `show_scenes(&[String], usize)` → `ShowScenes { titles, current }`; `show_pill(&Pill, usize, &str)` → `ShowPill { pill, problems, line }`; `show_build(Build, &str)` → `ShowBuild { build, tooltip }` | `#[block_signal]` on the switcher in `ShowView`. Pill, back, forward and build use window actions: keep them. Test getter `switcher_labels()`. |
| `icons.rs` | `PathIcon` | — | `set_shape(Shape)` (widget method) | A GObject subclass: stays a widget, used inside components' views. |
| `inspector.rs` | `Inspector` | `connect_focus(Target)` → `InspectorOutput::Focus(Target)`; `connect_source(SourceLocation)` → `Source(SourceLocation)`; its name links' clicks and hovers → `Link(LinkTarget)`, `Hover(Option<LinkTarget>)` (the window's `connect_links` is gone); `connect_unsaved` is gone: the window sends `show_unsaved(set)` → `InspectorInput::Unsaved(BTreeSet<PathBuf>)` when `FilesOutput::Unsaved` says the set changed | `show_focus()` → `ShowFocus`; `show_idle(Option<Note>)` → `ShowIdle(Option<Note>)`; `show(Option<&Inspection>)` → `Show(Option<Inspection>)` | Owns `FocusPanel` and `LinkHandlers` (their outputs forwarded). |
| `new_project.rs` | `NewProjectDialog` | `cancel`, `form.folder_button`, `form.create` `connect_clicked` and `form.dialog.connect_closed` → `NewProjectOutput::Cancel`, `ChooseFolder`, `Create(NewProject)`, `Closed` | — | *queries* `answers()`, `ToolChoice::project()`. `ask_new_project` / `welcome_new_project` are `impl Shell` (coordinator). |
| `option_search.rs` | `OptionSearchComponent` | `connect_closed()` → `OptionSearchOutput::Closed`; `connect_chosen(String, String)` → `Chosen { setting, value }`; `connect_search(&str) -> Vec<SearchGroup>` is gone: the search holds the window's copy of the setting and searches it itself | `present(&str, &str)` → `Present { name, setting: Option<Box<SettingParam>>, query }`; `refresh()` → `Update(Box<SettingParam>)` (a fresh copy); `close()` → `Close`; typing → `QueryChanged`; `choose_selected()` → `ChooseSelected`; a row's click → `Activated(i32)`; the arrows → `Step(i32)` | **Done**: owned by the window, forwarded at launch. *query* `setting()` is the window's own `EditView::search_open` (set when it presents, cleared on `Closed`). Test getters `setting()`, `rows()` on the model, `entry()`, `selected(..)` on the widgets. |
| `output.rs` | `OutputPanel` | — | `add_line(&str)` → `OutputPanelInput::AddLine(String)` | |
| `panels.rs` | `ProblemsPanel` | `connect_choose(usize)` → `ProblemsOutput::Choose(usize)`; `connect_source(SourceLocation)` → `Source(SourceLocation)` | `show(&[Report], Option<usize>, bool)` → `Show { reports: Vec<Report>, selected, stale }`; `show_selected(Option<usize>)` → `ShowSelected(Option<usize>)` | `#[block_signal]` on row selection in `ShowSelected`. |
| `panels.rs` | `FilesList` | `connect_open(PathBuf)` → `FilesListOutput::Open(PathBuf)` | `set_folder(&Path)` → `SetFolder(PathBuf)` | |
| `sections.rs` | `LinkHandlers<Message>` | the old `connect_click` / `connect_hover` slots are gone: a link sends its owner's own input (`LinkHandlers::new(input, click, hover)`, two message constructors), and the owner tells its owner `<Owner>Output::Link(LinkTarget)` / `Hover(Option<LinkTarget>)` | — | **Done**. Not a component: its widget builders (`link`, `links`, `set_button_words`) are used inside `Inspector`'s and `EditCard`'s views. No `Cell`: the handlers are a sender and two `fn`s, cloned into each link. The stylesheet goes on the display once per process (`std::sync::Once`). |
| `start_page.rs` | `StartPage` | `open` / `create` actions, recent `row.connect_activated`, `window.connect_close_request` → `StartPageOutput::Open`, `Create`, `OpenRecent(PathBuf)`, `CloseRequest` | `ask_new_project()` → `AskNewProject` | Owned by `Shell`; a window component of its own. |
| `tab_strip.rs` | `TabStrip`, `set_page_tabs`, `show_page_tabs` | `TabStripOutput::Show { page: glib::WeakRef<panel::Widget>, index }` (a frame can hold several panels with tabs of their own) goes to the route the panel's owner registered once with `set_page_tabs(panel, Fn(usize))` (it only sends a message) | the owner hands its tabs over as data (`show_page_tabs(panel, Vec<PageTab>)`, from `FilesOutput::Tabs`); every strip gathers them into `TabStripInput::SetTabs(Vec<PageTabLabel>)` when the main loop next runs; the frame's `page_changed` → `PageChanged` (restyle in place); Ctrl+Page Up/Down on the frame → `Step(Step)` | A GObject subclass holding closures: rules 2 and 9. |
| `tree.rs` | `TreeSidebar` | `connect_select(Selection)` → `TreeOutput::Select(Selection)`; `connect_source(SourceLocation)` → `Source(SourceLocation)`; `connect_focus(Target)` → `Focus(Target)`; `connect_hover(Vec<Item>)` → `Hover(Vec<Item>)` | `show_tree(PartsTree, PartJoints)` → `ShowTree { tree, joints }`; `show_empty(EmptyState)` → `ShowEmpty(EmptyState)`; `set_placeholder(Option<&str>)` → `SetPlaceholder(Option<String>)`; `show_marks(Rc<Marks>)` → `ShowMarks(Rc<Marks>)`; `show_selection(Option<&Selection>)` → `ShowSelection(Option<Selection>)` | Its `syncing` flag and `try_borrow_mut` go: `#[block_signal]` on row selection while filling and syncing. |
| `viewport.rs` | `Viewport` | `connect_resized(u32, u32)` → `ViewportOutput::Resized { width, height }` | `show_frame(gdk::Texture)` → `ShowFrame(gdk::Texture)`; `show_live(LiveFrame) -> bool` → `ShowLive(LiveFrame)` | A GObject subclass (the wgpu surface): stays a widget, wrapped by the view's component. *queries* `scale()`, `device_size()`. Frame timing is sensitive: measure before and after. |
| `view_tools.rs` | `ViewToolbar` | (mod.rs) `xray.connect_toggled` → `ViewToolbarOutput::Xray(bool)`; `pull.connect_toggled` → `Pull(bool)`; `play.connect_clicked` → `Play`; `slider` `Changed` → `Apart(f64)` | `show(bool, f32, bool)` → `Show { on, amount, playing }`; `offer(bool)` → `Offer(bool)` | `#[block_signal]` on Pull apart and X-ray (replaces `updating_controls`). Owns a slider. |
| `view_tools.rs` | `CameraToolbar` | (mod.rs) `front.connect_clicked` → `CameraOutput::Front`; `turn.connect_clicked` → `Turn` | `orientation_setter(&ViewToolbar)` → `SetOrientation(gtk::Orientation)` | `fit` uses the `win.frame` action: keep it. |
| `view_tools.rs` | `PlanSwitch` | (mod.rs) `from_above.connect_toggled` → `PlanSwitchOutput::FromAbove(bool)` | `show(bool)` → `Show { plan }` | `#[block_signal]` on the toggle. |
| `view_tools.rs` | `EmptyView` | — (its `open` button uses the `win.open` action: keep it) | `show(&str, &str, bool)` → `EmptyViewInput::Show { title, description, offers_open }` | |
| `workbench.rs` | `Workbench` | `connect_tools()` → `WorkbenchOutput::ToolsChanged`; `connect_save_failed(&str)` → `SaveFailed(String)` | `apply(Layout)` → `Apply(Layout)`; `narrow_edges(&adw::Breakpoint, f64)` → `NarrowEdges { breakpoint, width }`, whose breakpoint's apply/unapply send `Fold(Vec<Side>)`/`Unfold(Vec<Side>)`; `dock_tool(Group, Spot)` → `DockTool { group, spot }`; `show_problem_count(usize)` → `ShowProblemCount(usize)`; `save()` → `Save`; `reset()` → `Reset`; `present(PanelId)` → `Present(PanelId)`; `bring_forward_beside_view(PanelId)` → `BringForwardBesideView(PanelId)`; `move_to(PanelId, Side)` → `MoveTo { panel, side }` | libpanel dock. *queries* `layout()`, `panel(PanelId)`, `is_open`, `is_shown`, `share_a_frame`; `tools()` hands out the shared `Rc<RefCell<Spots>>`: rule 11. |
| `edit_view/grip_panel.rs` | `GripPanel` | `connect_action(PanelAction)` → `GripPanelOutput::Action(PanelAction)`; (`grips.rs`) `panel.value.connect_activate` → folds into `Action` | `show(&PanelModel)` → `Show(PanelModel)` | Test getters `words()`, `radios()`. |
| `edit_view/grip_panel.rs` | `Readout` | — | `show(Option<&(String, String)>)` → `ReadoutInput::Show(Option<(String, String)>)` | Test getter `words()`. |
| `edit_view/changes.rs` | `ChangeCard` | `connect_action(ChangeAction)` → `ChangeCardOutput::Action(ChangeAction)` | `show(&ChangeWords)` → `Show(ChangeWords)` | |
| `edit_view/changes.rs` | `DependantsDialog` | `include.connect_clicked` → `DependantsOutput::Include`; `cancel.connect_clicked` → `Cancel` | `show(&str, &[String])` → `Show { item, dependants }` | |
| `edit_view/item_tools.rs` | `RepeatRow` | `connect_changed(usize, Axis)` → `RepeatRowOutput::Changed { count, axis }` | `show(usize, Axis, bool)` → `Show { count, axis, axis_shown }` | `#[block_signal]` on the count spin button and axis drop-down in `Show`. |
| `edit_view/item_tools.rs` | `RepeatFoldRow` | `connect_unfold()` → `RepeatFoldOutput::Unfold`; its inner `row` (a `RepeatRow`) `Changed` → `RepeatFoldOutput::Changed { count, axis }` | `show(RepeatFold)` → `Show(RepeatFold)`; `place_apart(&gtk::Box, &gtk::Box)` → launch-time `Init` (where its button and row go) | |
| `edit_view/item_tools.rs` | `ItemTools` | (`connect_item_tools`) `name.connect_editing_notify` (when editing ends) → `ItemToolsOutput::Rename(String)`; `mirror.connect_active_notify` (when switched on) → `Mirror`; `hold.connect_clicked` → `Hold`; `remove.connect_clicked` → `Remove`; `repeat` `Unfold` → `Unfold`; `repeat.row` `Changed` → `Repeat { count, axis }` | what `Window::update_item_tools` fills in → `Show(ItemToolsShown)` | Its `filling: Rc<Cell<bool>>` guard goes: `#[block_signal]` on the name and Mirror while filling. |
| `edit_view/hold.rs` | `HoldCard` | (`connect_hold`) `try_it.connect_clicked` and `reason.connect_activate` → `HoldCardOutput::TryIt`; `cancel.connect_clicked` → `Cancel` | `show(&Measured, &Measured, &Result<Length, String>)` → `Show { first, second, now }` | *query*: the reason entry's text, read by `try_hold`: carry it in `TryIt { reason: String }`. |
| `edit_view/hold.rs` | `Hold` | (`connect_hold`) `from_space.connect_clicked` → `HoldOutput::FromSpace` | — | Owns the `HoldCard`; `from_space` sits in the edit menu's root. |
| `edit_view/repeat_handles.rs` | `Repeats`, `Handle` | (`connect_repeats`) `row.connect_unfold` → `RepeatsOutput::Unfold`; `row.row.connect_changed` → `Changed { count, axis }`; the count overlay's drag and the boundary drags (`connect_boundary_drag`) → `Dragged { index, to }` | handle placement (from `shown_boundaries`) → `Show(Vec<Boundary>)` | Mostly `impl Window` today: split the widget part out first. Its `row` is a `RepeatFoldRow`. |
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
| Selection and focus | `update/selection.rs` | `SelectionInput` | `select_and_frame`, `selection_changed`, `show_card`, `show_hint`, `show_moves`, focus mode (`focus_changed`, `open_focus`, `leave_focus`, `update_focus`, `toggle_play_focus`, `focus_tick`, the progress, caption and pictures), posed problems and openings (`pose_problem`, `update_opening`, `motion_moved`, `problem_moved`), `finder_search`, `finder_hit` | actions `back`, `forward`, `next-problem`, `previous-problem`, `find`, `escape`, `show-problem`; `ProblemsPanel` choose/source; `FilesList` open; `TreeSidebar` select/source/focus/hover; `Dock` move; `Inspector` focus/source; `Finder` search request and hit; `FocusPanel` done/source/X-ray/tint/fade; the step and swing cards' scrubs and play; `ProblemCard`'s scrub; `Pictures` resized and chosen; `PlanSwitch`; `MotionBar`'s scale |
| Project | `update/project.rs` | `ProjectInput` | `run_check` (and `Checked`), `show_folder`, `show_title`, `result_changed`, `apply_wanted`, `editor_changed`, `sync_files`, `text_linked`, `capture_if_asked`, `check_status_changed`, `build_changed`, `choose_folder`, `go_to_folder`, `ask_about_unsaved`, `open_folder`, `watch`, closing | actions `new-project`, `check`, `search`, `editor`, `show-panel`, `move-panel`, `reset-layout`, `model`, `save`, `format`, `open`, `open-recent`; the folder drop; `FilesPage` link/go/check/show-in-3D/open/notice and its tabs; `Workbench` save failed; `Inspector` unsaved; the root's `close-request` |
| Editing | `update/editing.rs` | `EditingInput` | in `edit_view.rs` and `edit_view/*.rs`: the menu, the card, the option search, links, grips, changes, holds, item tools, repeats, verbs | `EditMenuComponent` chosen and drawn, `EditCardComponent` action, open short list and links (forwarded at launch, `EditView::new`); the face button; the pointer over the view; `OptionSearchComponent` chosen/closed (forwarded at launch); `Inspector`'s links; `DependantsDialog`; `ChangeCard`; `Hold`/`HoldCard`; `ItemTools`; `RepeatFoldRow`s; the repeat count and boundary drags; `Finder` actions request and action; `GripPanel`; the grips' handles, entries and range menus; the edit menu's, options' and stops' timers and worker threads; the Delete key |
| Rendering | `update/rendering.rs` | `RenderingInput` | `view_changed`, framing and the camera (`frame`, `frame_selection`, `look_from`, `update_orbit`, `pick`, `resized`), drawing (`request_render`, `render_now`, `poll_frame`, `looks`), pins, `pull_apart`, `toggle_play_explode`, `explode_tick`, `export_view`, `scene_changed`, `explode_changed`, `repose`, the backdrop | actions `frame`, `export-view`, `view-*`; `Header` outputs (view, scene); `ViewToolbar` X-ray, Pull apart, play and slider; `CameraToolbar` front and turn; the style manager's dark (disconnected in `shutdown`); `Viewport` resized; the view's drags, scroll and keys; the mouse's back and forward buttons |

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
- Each leaf the window wires hands it its component's own Output enum
  through `connect_outputs(impl Fn(<Leaf>Output))` (the shim's
  `Bridge::connect`), and `wire` maps each variant to a group's input.
  There are no per-output callback slots in front of them any more.
- Controls the window also sets (Pull apart, From above, focus mode's
  checks, the motion slider) are set by their own components with the
  handler blocked (`#[block_signal]`, or `block_signal` in
  `update_with_view`), so their outputs are people's changes only. The
  window shows focus mode's options and From above from its model after
  opening focus mode and after each of people's changes, so a change made
  in the same turn as the window's own ends as the model has it.
- Leaf-to-leaf links go through the window too: the floating controls'
  `Orient`/`Docked` and the workbench's `ToolsChanged` are
  `RenderingInput::ToolOriented`, `ToolDocked` and `ToolsMoved`.
- Request and reply (rule 9): the finder's searches go out as
  `SelectionInput::FinderSearch` / `EditingInput::FinderActions` and are
  answered with `Finder::show_hits` / `show_actions`. The option search
  needs no request: the window sends it a copy of the setting it searches
  when it opens it (`Present`) and again as each option's fit is found
  (`Update`), and the dialog lists matches from that copy itself.
- A key handler that must answer GTK at once (take the key or pass it
  on) decides from a copy the window keeps, never by asking the model.
  Delete outside a text field is taken only when there is something to
  remove, as on main: the window sets `State::removable` (an
  `Rc<Cell<bool>>`) at the end of every `update`, and the handler reads
  it, together with whether the editor's analysis reads (while it
  doesn't, Delete says so; a new analysis arrives with no message, so it
  is read at the press); the removal itself is a `RemoveSelected`
  message. With nothing to remove the key goes on to other widgets. The
  copy is written only by `update` and read only by that handler, so it
  is not the shared state rule 11 warns about.
- Every message payload has `Debug` of its own (`files::CursorName`,
  `stave_ide::Analysis`, `stave_app`'s `FoundFit` and `FoundStops` all
  derive it), so there is no wrapper that hides one.

### Edit card and menu: answers by message

The window owns `Controller<EditMenuComponent>` and
`Controller<EditCardComponent>` as fields of `EditView`, launched in
`EditView::new` with `.forward(window_input, ..)` (rule 6), so their
outputs arrive as `EditingInput`s; `edit_demo.rs` keeps its showcase's
controller in `EditView::showcase` the same way. Nothing reads a card or
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
   - `FilesOutput::Analysed(Rc<Analysis>)`: the shim keeps it in a
     `LatestAnalysis` handle as the output is delivered (before the
     window's own message), which the window reads (`analysis()`) and can
     give a copy of to a handler that must answer GTK at once.
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
   `tab_strip::show_page_tabs`, the strips gathering them when the main
   loop next runs. The strip's "show this tab" goes to a route registered
   once (`set_page_tabs(panel, Fn(usize))`), which only sends
   `ProjectInput::ShowFileTab`.

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
  a turn of the main loop after it appears where it will be placed. An
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

### Done by the group lanes and the integration

- Every handler is an `impl Window` method on `&mut self`: closing, the
  unsaved-changes question and `--window-screenshot` answer with messages
  (`ProjectInput::CloseAsked`, `UnsavedAnswered`, `CaptureTick`).
- `Window::controls`, `block_controls` and `unblock_controls` are gone;
  the motion bar is a component (`chrome::MotionBarComponent`).
- The selection, rendering and project leaves reach the window by their
  outputs (`connect_outputs`); the `Slot`s and callback structs of
  `tree.rs`, `focus_stage.rs`, `panels.rs`, `inspector.rs`, `finder.rs`
  and `workbench.rs` are gone, as are `ToolSpots`, the Finder's
  `Rc::new_cyclic` and the weak links between the workbench and the
  floating controls.
- `EditView`'s state, `pending_marked`, `verbs`, `immediate`,
  `panel_shown` and `options`, the grips' state, the hold's step, the
  changes' asked item, the item tools' offer and the boundary handles are
  plain fields of the model.

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

### Still to do

- The tab strip still keeps its registry of panels' own tabs and its
  strips in thread-local `RefCell`s (borrowed only to copy in or out,
  never across a call); they hold data and a message-sending route now,
  not closures that read the Code panel.
- The editing leaves `ChangeCard`, `GripPanel`, `RepeatRow`, `ItemTools`
  and `HoldCard` still take per-output callbacks.
- `Repeats` keeps its drag in `Cell`s, and the grips' drawing is an
  `Rc<RefCell<..>>` shared with their draw function: give the draw
  function a copy, as the slider does.
