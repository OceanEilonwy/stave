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

/// A host that shares one folder with Stave, as Claude Code or Codex
/// started inside a project folder does.
struct SharingHost {
    root: String,
}

#[expect(
    deprecated,
    reason = "roots are deprecated by SEP-2577; the hosts still use them"
)]
impl rmcp::ClientHandler for SharingHost {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        let mut capabilities = rmcp::model::ClientCapabilities::default();
        capabilities.roots = Some(rmcp::model::RootsCapabilities::default());
        rmcp::model::ClientInfo::new(
            capabilities,
            rmcp::model::Implementation::new("Sharing Host", "1.0"),
        )
    }

    async fn list_roots(
        &self,
        _context: rmcp::service::RequestContext<rmcp::RoleClient>,
    ) -> Result<rmcp::model::ListRootsResult, rmcp::ErrorData> {
        Ok(rmcp::model::ListRootsResult::new(vec![
            rmcp::model::Root::new(self.root.clone()),
        ]))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn list_projects_lists_a_folder_the_host_shares() {
    use rmcp::ServiceExt as _;
    let folder = common::example_copy("box", "list_projects_lists_a_shared_folder");
    let options = common::options("list_projects_lists_a_shared_folder");
    let (server_transport, client_transport) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = stave_mcp::StaveServer::new(options)
            .serve(server_transport)
            .await
        {
            let _ = running.waiting().await;
        }
    });
    let client = SharingHost {
        root: format!("file://{}", folder.display()),
    }
    .serve(client_transport)
    .await
    .unwrap();
    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("list_projects"))
        .await
        .unwrap();
    let text: String = result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.clone()))
        .collect();
    assert!(text.contains("Folders this app shares"), "{text}");
    assert!(text.contains(&folder.display().to_string()), "{text}");
}
