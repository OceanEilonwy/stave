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

   This replaces `updating_controls` in `mod.rs` and `syncing` in
   `tree.rs`. Setters that did not call back (`set_value` and friends)
   become inputs whose `update` sends no output.
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
`entries()`, `words()`) stay as plain methods on the shim, reading
`bridge.widgets()` or the model. Queries the window calls synchronously
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
| `edit_card.rs` | `EditCard` | `connect_action(CardAction)` → `EditCardOutput::Action(CardAction)` | `show(&CardModel, &dyn Words)` → `Show(CardView)` (owned); `update_options(&Candidate)` → `UpdateOptions(Candidate)`; `open_options(&str) -> bool` → `OpenOptions(String)`, the bool becomes `EditCardOutput::OptionsOpened(bool)`; `scroll_to_open_options_now()` → `ScrollToOpenOptions` | Its `links: LinkHandlers` fold into `Action` (see `sections.rs`). Test getters: `key_controller`, `buttons`, `name_field`, `entries`, `option_lists`, `scrolled`, `put_back_button`, `fix_buttons`; *query* `open_setting()`. |
| `edit_menu.rs` | `EditMenu` | `connect_chosen(Declaration)` → `EditMenuOutput::Chosen(Declaration)` | `show(Heading, &[MenuItem])` → `Show { heading, items: Vec<MenuEntry> }` (owned) | Test getter `rows()`. |
| `edit_panel.rs` | `EditPanel` | — | `update() -> bool` → `Update`, the bool becomes `EditPanelOutput::Shown(bool)` | *query* `is_empty()`. Built from other cards' widgets: becomes part of its owner's view. |
| `editor.rs` | `Editor` | `connect_lens_uses(SymbolId)` → `EditorOutput::LensUses(SymbolId)`; `connect_lens_show(SymbolId)` → `EditorOutput::LensShow(SymbolId)` | `set_lenses(&[LensLine])` → `SetLenses(Vec<LensLine>)`; `set_peek(Option<(gtk::Widget, i32)>)` → `SetPeek(Option<(gtk::Widget, i32)>)`; `show_buffer(&TextBuffer, bool)` → `ShowBuffer { buffer, editable }`; `colour(bool)` → `Colour { dark }`; `paint(&TextBuffer, &LineStarts, &[(Run, TokenClass)])` → `Paint { buffer, lines, runs }`; `start_search()` → `StartSearch`; `find(&str)` → `Find(String)`; `close_search()` → `CloseSearch`; `show_card(&Widget, i32, i32)` → `ShowCard { card, x, y }`; `hide_card()` → `HideCard`; `place_overlays()` → `PlaceOverlays`; `go_to(&LineStarts, u32)` → `GoTo { lines, offset }` | GObject subclasses `Marks`, `CodeView`, `BlockIndenter` stay widgets. *queries*: `new_buffer`, `vadjustment`, `has_focus`, `has_search`, `has_peek`, `has_card`, `cursor`, `offset_at`; `LineStarts` is plain data. Owned by `FilesPage`. |
| `files.rs` | `FilesPage` | `connect_link(CursorName)` → `FilesOutput::Link(CursorName)`; `connect_go(Option<SourceLocation>, SourceLocation)` → `Go { from, to }`; `connect_check()` → `Check`; `connect_show_in_3d(Rc<Analysis>, SymbolId)` → `ShowIn3d { analysis, symbol }`; `connect_open()` → `Open`; `connect_notice(String)` → `Notice(String)` | `focus_text()` → `FocusText`; `set_folder(&Path)` → `SetFolder(PathBuf)`; `save_all() -> bool` → `SaveAll`, answered by `FilesOutput::Saved(bool)`; `save()` → `Save`; `format()` → `Format`; `forget_unsaved()` → `ForgetUnsaved`; `focus_search() -> bool` → `FocusSearch`; `show_marks(Rc<Marks>)` → `ShowMarks(Rc<Marks>)`; `show_result(Option<Rc<Checked>>)` → `ShowResult(Option<Rc<Checked>>)`; `show_location(&SourceLocation)` → `ShowLocation(SourceLocation)`; `reveal(&SourceLocation)` → `Reveal(SourceLocation)`; `wish(Wish)` → `Wish(Wish)`; `restyle(bool)` → `Restyle { dark }`; `open_document(&Document)` → `OpenDocument(Document)` | The biggest leaf (own `RefCell` state, `Rc<Self>` methods); owns `Editor`. *queries*: `buffer_for`, `has_unsaved`, `is_unsaved`, `unsaved_count`, `analysis`, `overlay_for_check`, `edit_files`, `page_tabs`. `BufferWriter` (`edit_writer.rs`) writes through it with an `Rc<Cell<bool>>`: rule 11. |
| `finder.rs` | `Finder` | `connect_search(&str) -> Vec<Hit>` → request `FinderOutput::Search(String)` / reply `FinderInput::Hits(Vec<Hit>)`; `connect_actions(&str) -> Vec<Action>` → `FinderOutput::SearchActions(String)` / `FinderInput::Actions(Vec<Action>)`; `connect_hit(Hit, Choice)` → `FinderOutput::Hit { hit, choice }`; `connect_action(usize)` → `FinderOutput::Action(usize)` | `present(&impl IsA<Widget>, &str)` → `Present { parent: gtk::Widget, query: String }` | Rule 9 for the two searches. Root is an `adw::Dialog`. |
| `floating.rs` | `Floats` | constructor `orient: Orient` → `FloatsOutput::Orient { group, orientation }`; constructor `on_dock: OnDock` → `FloatsOutput::Docked { group, spot }` | `refresh()` → `Refresh` | Shares `Rc<RefCell<Spots>>` with `Workbench`: rule 11. Takes the view's overlay and the floating widgets at launch (`Init`). |
| `focus_panel.rs` | `FocusPanel` | `connect_source(SourceLocation)` → `FocusPanelOutput::Source(SourceLocation)`; (mod.rs) `done.connect_clicked` → `Done`; `xray`, `tint`, `fade` `connect_active_notify` → `Xray(bool)`, `Tint(bool)`, `Fade(bool)` | `show(Option<&Focus>)` → `Show(Option<Focus>)` | `#[block_signal]` on the three checks when `Show` sets them. Owned by `Inspector`. |
| `focus_stage.rs` | `StepCard` | (mod.rs) `play.connect_clicked` → `StepCardOutput::Play`; `scrub` `Changed` → `StepCardOutput::Scrubbed(f64)` | `show_frame(Option<&str>, &str)` → `ShowFrame { step: Option<String>, status: String }`; (mod.rs) `scrub.set_value` → `SetProgress(f64)` | Owns a slider: forward `SliderOutput::Changed`. |
| `focus_stage.rs` | `SwingCard` | (mod.rs) `play.connect_clicked` → `SwingCardOutput::Play`; `scrub` `Changed` → `Scrubbed(f64)` | `show_name(&str, bool)` → `ShowName { name, turns }`; `show_value(&str)` → `ShowValue(String)`; (mod.rs) `scrub.set_value` → `SetProgress(f64)` | Owns a slider. |
| `focus_stage.rs` | `Caption` | — | `show(&str, &str)` → `CaptionInput::Show { text, hint }` | |
| `focus_stage.rs` | `Pictures` | `connect_resized()` → `PicturesOutput::Resized`; the `on_choose` closure given to `show` → `PicturesOutput::Chosen(usize)` | `show(&[String], on_choose)` → `Show(Vec<String>)`; `show_textures(Vec<gdk::Texture>)` → `ShowTextures(Vec<gdk::Texture>)`; `show_current(Option<usize>)` → `ShowCurrent(Option<usize>)` | *query* `device_size()`. |
| `focus_stage.rs` | `ProblemCard` | `scrub` `Changed` (mod.rs) → `ProblemCardOutput::Moved(f64)` | `show(Option<&MotionCard>, f32)` → `Show { card: Option<MotionCard>, amount: f32 }`; `show_value(&MotionCard, f32)` → `ShowValue { card, amount }` | Owns a slider. |
| `header.rs` | `Header` | (mod.rs) `switcher.connect_active_name_notify` → `HeaderOutput::View(View)`; `scenes.connect_row_activated` → `HeaderOutput::Scene(usize)` | `show_title(&str, &str)` → `ShowTitle { title, folder }`; `show_view(View)` → `ShowView(View)`; `show_scenes(&[String], usize)` → `ShowScenes { titles, current }`; `show_pill(&Pill, usize, &str)` → `ShowPill { pill, problems, line }`; `show_build(Build, &str)` → `ShowBuild { build, tooltip }` | `#[block_signal]` on the switcher in `ShowView`. Pill, back, forward and build use window actions: keep them. Test getter `switcher_labels()`. |
| `icons.rs` | `PathIcon` | — | `set_shape(Shape)` (widget method) | A GObject subclass: stays a widget, used inside components' views. |
| `inspector.rs` | `Inspector` | `connect_focus(Target)` → `InspectorOutput::Focus(Target)`; `connect_source(SourceLocation)` → `Source(SourceLocation)`; `connect_unsaved(&str) -> bool` → *query*: becomes `InspectorInput::Unsaved(BTreeSet<PathBuf>)`, sent when it changes | `show_focus()` → `ShowFocus`; `show_idle(Option<Note>)` → `ShowIdle(Option<Note>)`; `show(Option<&Inspection>)` → `Show(Option<Inspection>)` | Owns `FocusPanel` and `LinkHandlers` (their outputs forwarded). |
| `new_project.rs` | `NewProjectDialog` | `cancel`, `form.folder_button`, `form.create` `connect_clicked` and `form.dialog.connect_closed` → `NewProjectOutput::Cancel`, `ChooseFolder`, `Create(NewProject)`, `Closed` | — | *queries* `answers()`, `ToolChoice::project()`. `ask_new_project` / `welcome_new_project` are `impl Shell` (coordinator). |
| `option_search.rs` | `OptionSearch` | `connect_closed()` → `OptionSearchOutput::Closed`; `connect_search(&str) -> Vec<SearchGroup>` → request `Search(String)` / reply `OptionSearchInput::Results(Vec<SearchGroup>)`; `connect_chosen(String, String)` → `Chosen { setting, value }` | `present(&str, &str)` → `Present { setting, query }`; `close()` → `Close`; `refresh()` → `Refresh`; `type_query(&str)` → `TypeQuery(String)`; `choose_selected()` → `ChooseSelected`; `press_arrow(bool)` → `PressArrow { down }` | Rule 9. *queries* `setting()`, `selected()`; test getter `rows()`. |
| `output.rs` | `OutputPanel` | — | `add_line(&str)` → `OutputPanelInput::AddLine(String)` | |
| `panels.rs` | `ProblemsPanel` | `connect_choose(usize)` → `ProblemsOutput::Choose(usize)`; `connect_source(SourceLocation)` → `Source(SourceLocation)` | `show(&[Report], Option<usize>, bool)` → `Show { reports: Vec<Report>, selected, stale }`; `show_selected(Option<usize>)` → `ShowSelected(Option<usize>)` | `#[block_signal]` on row selection in `ShowSelected`. |
| `panels.rs` | `FilesList` | `connect_open(PathBuf)` → `FilesListOutput::Open(PathBuf)` | `set_folder(&Path)` → `SetFolder(PathBuf)` | |
| `sections.rs` | `LinkHandlers` | `connect_click(LinkTarget)` → `<Owner>Output::Link(LinkTarget)`; `connect_hover(Option<LinkTarget>)` → `<Owner>Output::Hover(Option<LinkTarget>)` | `click_target(LinkTarget)` → `<Owner>Input::Click(LinkTarget)` | Not a component: its widget builders (`link`, `links`, `set_button_words`) are used inside `Inspector`'s and `EditCard`'s views, and the two slots become their owner's outputs. |
| `start_page.rs` | `StartPage` | `open` / `create` actions, recent `row.connect_activated`, `window.connect_close_request` → `StartPageOutput::Open`, `Create`, `OpenRecent(PathBuf)`, `CloseRequest` | `ask_new_project()` → `AskNewProject` | Owned by `Shell`; a window component of its own. |
| `tab_strip.rs` | `TabStrip`, `set_page_tabs` | `set_page_tabs(panel, Fn() -> Vec<PageTab>)` and `PageTab::show: Rc<dyn Fn()>` → `TabStripOutput::Show(usize)` | the tabs closure (a *query*) → `TabStripInput::SetTabs(Vec<PageTabLabel>)`, sent when the tabs change | A GObject subclass holding closures: rules 2 and 9. |
| `tree.rs` | `TreeSidebar` | `connect_select(Selection)` → `TreeOutput::Select(Selection)`; `connect_source(SourceLocation)` → `Source(SourceLocation)`; `connect_focus(Target)` → `Focus(Target)`; `connect_hover(Vec<Item>)` → `Hover(Vec<Item>)` | `show_tree(PartsTree, PartJoints)` → `ShowTree { tree, joints }`; `show_empty(EmptyState)` → `ShowEmpty(EmptyState)`; `set_placeholder(Option<&str>)` → `SetPlaceholder(Option<String>)`; `show_marks(Rc<Marks>)` → `ShowMarks(Rc<Marks>)`; `show_selection(Option<&Selection>)` → `ShowSelection(Option<Selection>)` | Its `syncing` flag and `try_borrow_mut` go: `#[block_signal]` on row selection while filling and syncing. |
| `viewport.rs` | `Viewport` | `connect_resized(u32, u32)` → `ViewportOutput::Resized { width, height }` | `show_frame(gdk::Texture)` → `ShowFrame(gdk::Texture)`; `show_live(LiveFrame) -> bool` → `ShowLive(LiveFrame)` | A GObject subclass (the wgpu surface): stays a widget, wrapped by the view's component. *queries* `scale()`, `device_size()`. Frame timing is sensitive: measure before and after. |
| `view_tools.rs` | `ViewToolbar` | (mod.rs) `xray.connect_toggled` → `ViewToolbarOutput::Xray(bool)`; `pull.connect_toggled` → `Pull(bool)`; `play.connect_clicked` → `Play`; `slider` `Changed` → `Apart(f64)` | `show(bool, f32, bool)` → `Show { on, amount, playing }`; `offer(bool)` → `Offer(bool)` | `#[block_signal]` on Pull apart and X-ray (replaces `updating_controls`). Owns a slider. |
| `view_tools.rs` | `CameraToolbar` | (mod.rs) `front.connect_clicked` → `CameraOutput::Front`; `turn.connect_clicked` → `Turn` | `orientation_setter(&ViewToolbar)` → `SetOrientation(gtk::Orientation)` | `fit` uses the `win.frame` action: keep it. |
| `view_tools.rs` | `PlanSwitch` | (mod.rs) `from_above.connect_toggled` → `PlanSwitchOutput::FromAbove(bool)` | `show(bool)` → `Show { plan }` | `#[block_signal]` on the toggle. |
| `view_tools.rs` | `EmptyView` | — (its `open` button uses the `win.open` action: keep it) | `show(&str, &str, bool)` → `EmptyViewInput::Show { title, description, offers_open }` | |
| `workbench.rs` | `Workbench` | `connect_tools()` → `WorkbenchOutput::ToolsChanged`; `connect_save_failed(&str)` → `SaveFailed(String)` | `apply(Layout)` → `Apply(Layout)`; `narrow_edges(&adw::Breakpoint)` → `NarrowEdges(adw::Breakpoint)`; `dock_tool(Group, Spot)` → `DockTool { group, spot }`; `show_problem_count(usize)` → `ShowProblemCount(usize)`; `save()` → `Save`; `reset()` → `Reset`; `present(PanelId)` → `Present(PanelId)`; `bring_forward_beside_view(PanelId)` → `BringForwardBesideView(PanelId)`; `move_to(PanelId, Side)` → `MoveTo { panel, side }` | libpanel dock. *queries* `layout()`, `panel(PanelId)`, `is_shown`, `share_a_frame`; `tools()` hands out the shared `Rc<RefCell<Spots>>`: rule 11. |
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
