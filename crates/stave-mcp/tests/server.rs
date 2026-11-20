//! The server as an AI app sees it: what `initialize` and `tools/list`
//! answer.

mod common;

#[tokio::test(flavor = "multi_thread")]
async fn initialize_lists_tools() {
    let connection = common::connect(common::options("initialize_lists_tools")).await;
    let info = connection.client.peer_info().expect("the server answered");
    assert_eq!(info.server_info.as_ref().unwrap().name, "stave");
    let mut names: Vec<String> = connection
        .client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "apply_suggestion",
            "check",
            "describe",
            "discard_suggestion",
            "fixes_for",
            "grammar",
            "list_conflicts",
            "list_projects",
            "list_suggestions",
            "list_versions",
            "new_project",
            "open_project",
            "resolve_conflict",
            "std_lookup",
            "suggest",
            "try_patch",
            "try_resolution",
            "undo",
        ]
    );
}
