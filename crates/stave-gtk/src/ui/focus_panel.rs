//! The focus panel: one joint or piece of hardware, in words, with its
//! controls. Everything it shows comes from `stave_app::focus::Focus`;
//! this module only lays it out.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::gdk;
use stave_app::focus::Focus;

/// The focus panel's widgets.
pub struct FocusPanel {
    /// The panel, ready to put in the side panel.
    pub root: gtk::Box,
    about: adw::PreferencesGroup,
    description: gtk::Label,
    rows: RefCell<Vec<gtk::Widget>>,
    /// Leaves focus mode.
    pub done: gtk::Button,
    /// Plays the joint going together, or the door opening.
    pub play: gtk::ToggleButton,
    /// Scrubs from 0 to 1: apart to together, or closed to open.
    pub scrub: gtk::Scale,
    scrub_label: gtk::Label,
    step: gtk::Label,
    status: gtk::Label,
    /// Draws the receiving part see-through.
    pub xray: adw::SwitchRow,
    /// Tints the faces cuts made.
    pub tint: adw::SwitchRow,
    /// Fades the other parts.
    pub fade: adw::SwitchRow,
    /// Shows the hinge from above, flat, through a section.
    pub from_above: gtk::ToggleButton,
    view_row: adw::ActionRow,
    members: adw::PreferencesGroup,
    member_rows: RefCell<Vec<gtk::Widget>>,
    pictures: adw::PreferencesGroup,
    picture_box: gtk::Box,
    insert: adw::PreferencesGroup,
    insert_text: gtk::Label,
    insert_source: gtk::Label,
}

fn label(classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .css_classes(classes.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>())
        .build()
}

fn boxed(child: &impl IsA<gtk::Widget>) -> adw::PreferencesRow {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 4);
    holder.set_margin_top(8);
    holder.set_margin_bottom(8);
    holder.set_margin_start(12);
    holder.set_margin_end(12);
    holder.append(child);
    adw::PreferencesRow::builder()
        .child(&holder)
        .activatable(false)
        .build()
}

impl FocusPanel {
    /// An empty, hidden panel.
    pub fn new() -> FocusPanel {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 18);
        root.set_visible(false);
        let about = adw::PreferencesGroup::builder().title("Focus").build();
        let done = gtk::Button::builder()
            .label("Done")
            .tooltip_text("Leave focus mode")
            .valign(gtk::Align::Center)
            .build();
        about.set_header_suffix(Some(&done));
        let description = label(&[]);
        about.add(&boxed(&description));

        let controls = adw::PreferencesGroup::new();
        let play = gtk::ToggleButton::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text("Play")
            .valign(gtk::Align::Center)
            .build();
        let scrub = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        scrub.set_hexpand(true);
        scrub.set_draw_value(false);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.append(&play);
        line.append(&scrub);
        let scrub_label = label(&["dim-label"]);
        let step = label(&[]);
        let status = label(&["dim-label"]);
        let stack = gtk::Box::new(gtk::Orientation::Vertical, 4);
        stack.append(&scrub_label);
        stack.append(&line);
        stack.append(&step);
        stack.append(&status);
        controls.add(&boxed(&stack));

        let display = adw::PreferencesGroup::builder().title("Show").build();
        let switch = |title: &str, subtitle: &str| {
            adw::SwitchRow::builder()
                .title(title)
                .subtitle(subtitle)
                .active(true)
                .build()
        };
        let xray = switch("X-ray", "see through the part that receives it");
        let tint = switch("Tint cut faces", "faces a cut made are darker");
        let fade = switch("Fade other parts", "everything else as a pale ghost");
        display.add(&xray);
        display.add(&tint);
        display.add(&fade);
        let from_above = gtk::ToggleButton::builder()
            .label("From above")
            .tooltip_text("Look straight down, through a cut just above the hinge")
            .valign(gtk::Align::Center)
            .build();
        let view_row = adw::ActionRow::builder()
            .title("View")
            .subtitle("in 3D, or flat from above")
            .build();
        view_row.add_suffix(&from_above);
        display.add(&view_row);

        let members = adw::PreferencesGroup::builder()
            .title("Its parts")
            .visible(false)
            .build();
        let pictures = adw::PreferencesGroup::builder()
            .title("At a glance")
            .visible(false)
            .build();
        let picture_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        picture_box.set_homogeneous(true);
        pictures.add(&boxed(&picture_box));
        let insert = adw::PreferencesGroup::builder()
            .title("How the joint is written")
            .visible(false)
            .build();
        let insert_text = label(&["monospace"]);
        let insert_source = label(&["dim-label"]);
        let insert_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        insert_box.append(&insert_text);
        insert_box.append(&insert_source);
        insert.add(&boxed(&insert_box));

        root.append(&about);
        root.append(&controls);
        root.append(&members);
        root.append(&pictures);
        root.append(&display);
        root.append(&insert);
        FocusPanel {
            root,
            about,
            description,
            rows: RefCell::new(Vec::new()),
            done,
            play,
            scrub,
            scrub_label,
            step,
            status,
            xray,
            tint,
            fade,
            from_above,
            view_row,
            members,
            member_rows: RefCell::new(Vec::new()),
            pictures,
            picture_box,
            insert,
            insert_text,
            insert_source,
        }
    }

    /// Shows a focus, or hides the panel (`None`).
    pub fn show(&self, focus: Option<&Focus>) {
        for row in self.rows.take() {
            self.about.remove(&row);
        }
        for row in self.member_rows.take() {
            self.members.remove(&row);
        }
        let Some(focus) = focus else {
            self.root.set_visible(false);
            return;
        };
        self.root.set_visible(true);
        self.about.set_title(&focus.heading);
        self.about.set_description(Some(&focus.title));
        self.description
            .set_label(focus.description.as_deref().unwrap_or(""));
        let mut rows = Vec::new();
        for entry in &focus.rows {
            let row = adw::ActionRow::builder()
                .title(entry.label)
                .subtitle(&entry.value)
                .use_markup(false)
                .subtitle_selectable(true)
                .build();
            row.add_css_class("property");
            self.about.add(&row);
            rows.push(row.upcast());
        }
        self.rows.replace(rows);
        let mut member_rows = Vec::new();
        for member in &focus.members {
            let row = adw::ActionRow::builder()
                .title(format!("{}  {}", member.number, member.name))
                .subtitle(&member.caption)
                .use_markup(false)
                .subtitle_lines(0)
                .build();
            self.members.add(&row);
            member_rows.push(row.upcast());
        }
        self.members.set_visible(!focus.members.is_empty());
        self.member_rows.replace(member_rows);
        self.scrub_label.set_label(if focus.assembles() {
            "apart → together"
        } else {
            "closed → fully open"
        });
        self.view_row.set_visible(focus.plan().is_some());
        self.from_above.set_active(false);
        match &focus.insert {
            Some(insert) => {
                self.insert.set_visible(true);
                self.insert_text.set_label(&insert.text);
                self.insert_source
                    .set_label(&format!("defined in {}", insert.source));
            }
            None => self.insert.set_visible(false),
        }
    }

    /// Shows where focus mode is: the step and how far from home.
    pub fn show_frame(&self, step: Option<&str>, status: &str) {
        self.step.set_visible(step.is_some());
        self.step.set_label(step.unwrap_or(""));
        self.status.set_label(status);
    }

    /// Shows small pictures with their captions.
    pub fn show_pictures(&self, pictures: Vec<(gdk::Texture, String)>) {
        while let Some(child) = self.picture_box.first_child() {
            self.picture_box.remove(&child);
        }
        self.pictures.set_visible(!pictures.is_empty());
        for (texture, caption) in pictures {
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_can_shrink(true);
            cell.append(&picture);
            cell.append(
                &gtk::Label::builder()
                    .label(caption)
                    .css_classes(["caption", "dim-label"])
                    .build(),
            );
            self.picture_box.append(&cell);
        }
    }
}
