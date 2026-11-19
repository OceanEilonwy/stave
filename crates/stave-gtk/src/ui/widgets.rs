//! Building the window's widgets: the header, the panels in the dock,
//! the 3D view with its floating controls, and the breakpoints.

use stave_app::i18n::{fill_in, tr};
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use anyhow::Result;
use stave_app::session::View;

use super::actions::main_menu;
use super::chrome::MotionBar;
use super::dock::Dock;
use super::files::FilesPage;
use super::finder::Finder;
use super::focus_stage::{Caption, Pictures, ProblemCard, StepCard, SwingCard};
use super::header::Header;
use super::inspector::Inspector;
use super::options::Options;
use super::tree::TreeSidebar;
use super::view_tools::{CameraToolbar, PlanSwitch, ViewToolbar};
use super::viewport::Viewport;
use super::{Widgets, app, chrome, floating, output, panels, view_layout, view_tools, workbench};

impl Widgets {
    /// Offers the view's everyday controls (the scene menu, X-ray, Pull
    /// apart and the moving assemblies' chips), or takes them away while
    /// a problem is posed or a joint is in focus.
    pub(super) fn offer_view_tools(&self, offered: bool) {
        self.dock.offer(offered);
        self.view_toolbar.offer(offered);
    }
}

pub(super) fn build_widgets(
    application: &adw::Application,
    folder: &Path,
    shell: &app::Shell,
    options: &Options,
) -> Result<Widgets> {
    let folder_name = app::folder_name(folder);
    let header = Header::new(&folder_name, &main_menu(shell.recent_menu()));

    // The 3D view, its controls floating on it (`view_layout` keeps them
    // apart): the scene menu at the top left; the camera buttons, X-ray
    // and Pull apart down the top right, Pull apart's slider popping out
    // to the left of its button; a short line and the shared-wood key
    // along the top; the moving assemblies' chips at the bottom left and
    // the selection's motion slider at the bottom right; a joint's step
    // player, a problem's motion card and the off-screen chip above the
    // foot, in the middle; focus mode's From above / In 3D switch at the
    // top right.
    let viewport = Viewport::default();
    let motion = MotionBar::new();
    let view_toolbar = ViewToolbar::new();
    let camera = CameraToolbar::new(&view_toolbar);
    let contact_key = view_tools::contact_key();
    let plan_switch = PlanSwitch::new();
    let step_card = StepCard::new();
    let problem_card = ProblemCard::new();
    let dock = Dock::new(&header.scene, &motion);
    let hint = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(48)
        .build();
    let hint_box = gtk::Box::builder()
        .css_classes(["stave-chip"])
        .visible(false)
        .build();
    hint_box.append(&hint);
    let edge_label = gtk::Label::new(None);
    edge_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    edge_label.set_max_width_chars(60);
    let edge_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    edge_content.append(&gtk::Image::from_icon_name("go-down-symbolic"));
    edge_content.append(&edge_label);
    let edge_pin = gtk::Button::builder()
        .child(&edge_content)
        .css_classes(["stave-edge-pin"])
        .tooltip_text(tr("Show the problem off screen"))
        .visible(false)
        .build();
    let figure = gtk::Overlay::builder()
        .child(&viewport)
        .vexpand(true)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["stave-viewport", "stave-figure"])
        .build();
    // Hardware focus lays the view out as a page (Hinge.dc.html): a line
    // over it, three small views and the swing card under it.
    let caption = Caption::new();
    let pictures = Pictures::new();
    let swing_card = SwingCard::new();
    let stage = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-stage"])
        .build();
    stage.append(&caption.root);
    stage.append(&figure);
    stage.append(&pictures.root);
    stage.append(&swing_card.root);
    let empty = view_tools::EmptyView::new();
    let view_stack = gtk::Stack::new();
    view_stack.add_named(&stage, Some("view"));
    view_stack.add_named(&empty.root, Some("status"));
    view_stack.set_visible_child_name("status");

    // The panels round the view: the parts tree, the files, the
    // selection's account, the open files and the problems.
    let tree = TreeSidebar::new();
    let inspector = Inspector::new();
    let details_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    details_box.append(&inspector.root);
    let details = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&details_box)
        .css_classes(["stave-details"])
        .build();
    // The Edit panel's content comes with the editing (`edit_view`).
    let edit_holder = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .vexpand(true)
        .build();
    let files = FilesPage::new();
    let file_list = panels::FilesList::new();
    let problems = panels::ProblemsPanel::new();
    let output = output::OutputPanel::new();

    // The 3D View panel: the view, with the problems pinned on the model
    // (they come later) over it.
    let design = gtk::Overlay::builder()
        .child(&view_stack)
        .hexpand(true)
        .vexpand(true)
        .overflow(gtk::Overflow::Hidden)
        .build();
    let view_panel = design.clone();

    let workbench = workbench::Workbench::new(
        vec![
            (workbench::PanelId::View, view_panel.upcast()),
            (workbench::PanelId::Parts, tree.root.clone().upcast()),
            (workbench::PanelId::Files, file_list.root.clone().upcast()),
            (workbench::PanelId::Details, details.upcast()),
            (workbench::PanelId::Edit, edit_holder.clone().upcast()),
            (workbench::PanelId::Code, files.root.clone().upcast()),
            (workbench::PanelId::Problems, problems.root.clone().upcast()),
            (workbench::PanelId::Output, output.root.clone().upcast()),
        ],
        match &options.layout {
            Some(path) => workbench::Remember::From(path.clone()),
            None if options.window_screenshot.is_some() => workbench::Remember::Nothing,
            None => workbench::Remember::Yours,
        },
    )?;

    // The view's controls float on it; the tool groups dock where the
    // layout says, and a group dragged to a new spot is remembered with
    // the layout.
    let floating: Vec<(view_layout::Floating, gtk::Widget)> = vec![
        (view_layout::Floating::Camera, camera.root.clone().upcast()),
        (
            view_layout::Floating::PullSlider,
            view_toolbar.popout.clone().upcast(),
        ),
        (
            view_layout::Floating::PlanSwitch,
            plan_switch.root.clone().upcast(),
        ),
        (view_layout::Floating::Scene, dock.scene.clone().upcast()),
        (view_layout::Floating::Hint, hint_box.clone().upcast()),
        (
            view_layout::Floating::ContactKey,
            contact_key.clone().upcast(),
        ),
        (view_layout::Floating::Moves, dock.moves.clone().upcast()),
        (view_layout::Floating::Motion, motion.root.clone().upcast()),
        (
            view_layout::Floating::StepCard,
            step_card.root.clone().upcast(),
        ),
        (
            view_layout::Floating::ProblemCard,
            problem_card.root.clone().upcast(),
        ),
        (view_layout::Floating::EdgePin, edge_pin.clone().upcast()),
    ];
    let orient_camera = camera.orientation_setter(&view_toolbar);
    let orient_moves = dock.moves_orientation_setter();
    let orient_motion = motion.orientation_setter();
    let saver = Rc::downgrade(&workbench);
    let floats = floating::Floats::new(
        &figure,
        floating,
        workbench.tools(),
        Box::new(move |group, orientation| match group {
            view_layout::Group::Scene => {}
            view_layout::Group::Camera => orient_camera(orientation),
            view_layout::Group::Moves => orient_moves(orientation),
            view_layout::Group::Motion => orient_motion(orientation),
        }),
        Box::new(move |group, spot| {
            if let Some(workbench) = saver.upgrade() {
                workbench.dock_tool(group, spot);
            }
        }),
    );
    let refresher = Rc::downgrade(&floats);
    workbench.connect_tools(move || {
        if let Some(floats) = refresher.upgrade() {
            floats.refresh();
        }
    });

    let views = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();
    views.add_named(&workbench.root, Some(View::Design.word()));
    views.add_named(&chrome::cut_list_page(), Some(View::CutList.word()));
    views.set_visible_child_name(View::Design.word());

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&views);
    // Over the whole window: the search over a setting's options.
    let sheet = gtk::Overlay::builder().child(&content).build();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&sheet));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header.bar);
    toolbar.set_content(Some(&toasts));

    let (width, height) = options.size.unwrap_or((1200, 800));
    let width = i32::try_from(width).unwrap_or(1200);
    let height = i32::try_from(height).unwrap_or(800);
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title(fill_in(
            &tr("{project} — Stave"),
            &[("project", &folder_name)],
        ))
        .default_width(width)
        .default_height(height)
        .width_request(360)
        .height_request(400)
        .content(&toolbar)
        .css_classes(["stave-window"])
        .build();
    if options.window_screenshot.is_some() {
        // A screenshot is exactly the size asked for, whatever the
        // display's own size.
        window.set_size_request(width, height);
    }

    // Narrower windows drop the switcher's words, then fold the menu bar
    // into one menu button; the project's name in the middle shortens
    // itself. Only the last breakpoint that matches applies, so each
    // repeats the wider ones. The side edges narrow too, then fold away,
    // so the Model keeps room.
    for (width, level) in [(1000.0, 0), (860.0, 1), (760.0, 2), (600.0, 3)] {
        let narrow = breakpoint(width);
        workbench.narrow_edges(&narrow, width);
        if level >= 2 {
            for label in header.switcher_labels() {
                narrow.add_setter(label, "visible", Some(&false.to_value()));
            }
        }
        if level >= 3 {
            narrow.add_setter(&header.menu_bar, "visible", Some(&false.to_value()));
            narrow.add_setter(&header.menu_button, "visible", Some(&true.to_value()));
            narrow.add_setter(
                &header.switcher,
                "css-classes",
                Some(&["stave-switcher", "compact"].to_value()),
            );
        }
        window.add_breakpoint(narrow);
    }

    let finder = Finder::new();

    Ok(Widgets {
        window,
        header,
        toasts,
        views,
        design,
        tree,
        inspector,
        details: details_box,
        edit_holder,
        sheet,
        dock,
        workbench,
        problems,
        file_list,
        hint,
        hint_box,
        edge_pin,
        edge_label,
        finder,
        view_stack,
        empty,
        viewport,
        motion,
        stage,
        view_toolbar,
        camera,
        contact_key,
        plan_switch,
        step_card,
        swing_card,
        caption,
        pictures,
        problem_card,
        files,
        output,
        _floats: floats,
    })
}

/// A breakpoint for windows at most `width` sp wide.
pub(super) fn breakpoint(width: f64) -> adw::Breakpoint {
    adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        width,
        adw::LengthUnit::Sp,
    ))
}

/// A control's `f64` value as the `f32` the scene works in.
pub(super) fn narrow(value: f64) -> f32 {
    crate::convert::f32_of(value)
}
