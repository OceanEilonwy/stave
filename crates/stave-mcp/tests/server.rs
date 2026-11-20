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
            "preview",
            "render_view",
            "resolve_conflict",
            "show_in_stave",
            "std_lookup",
            "suggest",
            "try_patch",
            "try_resolution",
            "undo",
        ]
    );
}

/// Applying, and making a project (which counts as an apply), are marked
/// as changing the project, so under the `ask` policy the host asks first.
#[tokio::test(flavor = "multi_thread")]
async fn tools_that_apply_are_marked_as_changing_the_project() {
    let connection = common::connect(common::options("tools_that_apply")).await;
    let tools = connection.client.list_all_tools().await.unwrap();
    for name in ["apply_suggestion", "new_project", "resolve_conflict"] {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} is listed"));
        let annotations = tool.annotations.as_ref().expect("annotated");
        assert_eq!(annotations.destructive_hint, Some(true), "{name}");
    }
}
