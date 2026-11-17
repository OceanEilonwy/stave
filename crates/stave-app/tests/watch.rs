//! The folder watcher calls back once after a `.stave` file is saved, and
//! not for other files.

use std::sync::mpsc;
use std::time::Duration;

use stave_app::check::{RECHECK_DELAY, Watcher};

#[test]
fn saving_a_stave_file_calls_back_once_the_folder_is_quiet() {
    let folder = std::env::temp_dir().join(format!("stave-watch-test-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("project.stave");
    std::fs::write(&file, "stave 1\n").unwrap();

    let (sender, receiver) = mpsc::channel();
    let watcher = Watcher::start(&folder, move || {
        let _ = sender.send(());
    })
    .unwrap();

    // Something that isn't a .stave file: no call.
    std::fs::write(folder.join("notes.txt"), "hello").unwrap();
    assert!(
        receiver
            .recv_timeout(RECHECK_DELAY + Duration::from_millis(500))
            .is_err()
    );

    // An editor's save, in a few writes: one call.
    for text in ["stave 1\n", "stave 1\n\n", "stave 1\n\n\n"] {
        std::fs::write(&file, text).unwrap();
    }
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("a save is noticed");
    assert!(
        receiver
            .recv_timeout(RECHECK_DELAY + Duration::from_millis(300))
            .is_err(),
        "a burst of writes is one call"
    );

    drop(watcher);
    std::fs::remove_dir_all(&folder).unwrap();
}
