//! "Export View as glTF…": the view as posed, saved as binary glTF.

use std::path::Path;

use stave_app::check::{self, Outcome};
use stave_app::export_view;

#[test]
fn the_view_can_be_saved_as_gltf() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stool");
    let checked = match check::run(&folder) {
        Outcome::Checked(checked) => *checked,
        Outcome::CantOpen { message, .. } => panic!("can't open {}: {message}", folder.display()),
    };
    let (_, graph) = checked.drawable().expect("the design solves");
    let posed = graph.pose_with(graph.scenes.first().expect("a scene"), &[], 1.0);
    let path = std::env::temp_dir().join(format!("stave-view-{}.glb", std::process::id()));
    export_view::save_glb(graph, &posed, &path).expect("saved");
    let bytes = std::fs::read(&path).expect("read back");
    std::fs::remove_file(&path).ok();
    assert_eq!(bytes.get(..4), Some(&b"glTF"[..]));
}
