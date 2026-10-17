use std::sync::Arc;
use warpui::App;

use super::*;

fn initialize_app(app: &App) {
    app.add_singleton_model(WarpConfig::mock);
}

#[test]
fn test_filter_global_workflows_for_session() {
    App::test((), |app| async move {
        initialize_app(&app);

        let local_workflows = app.add_singleton_model(LocalWorkflows::new);

        // Create a session that has "tr" as the only available command.
        let session = Session::test();
        session.set_external_commands(["tr"]);

        local_workflows.read(&app, move |local_workflows, _| {
            // Verify that all workflows either start with "tr" or punctuation.
            local_workflows
                .global_workflows(Some(Arc::new(session)))
                .for_each(|workflow| {
                    let command = workflow.command().expect("Workflow is Command Workflow");
                    assert!(
                        command.starts_with("tr")
                            || command.starts_with(|c: char| c.is_ascii_punctuation()),
                        "found workflow that should have been filtered: {command}"
                    );
                });
        });
    });
}
