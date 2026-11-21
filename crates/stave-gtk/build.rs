//! Builds Stave's icons (the files `data/icons/hicolor` installs) into a
//! GResource bundle that the binary carries, so the window can show the
//! application icon without Stave being installed.

fn main() {
    glib_build_tools::compile_resources(
        &["../../data/icons/hicolor"],
        "icons.gresource.xml",
        "icons.gresource",
    );
}
