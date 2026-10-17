#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use settings::manager::SettingsManager;
    use warpui::{App, SingletonEntity};

    use crate::auth::AuthStateProvider;
    use crate::cloud_object::model::persistence::ObjectStoreModel;
    use crate::cloud_object::model::view::ObjectStoreViewModel;
    use crate::cloud_object::update_manager::UpdateManager;
    use crate::cloud_object::{Revision, StoredObjectMetadata, StoredObjectPermissions};
    use crate::notebooks::manager::NotebookManager;
    use crate::notebooks::{NotebookObject, NotebookObjectModel};
    use crate::search::ai_context_menu::notebooks::data_source::NotebookDataSource;
    use crate::search::data_source::Query;
    use crate::search::mixer::SyncDataSource;
    use crate::server::ids::SyncId;
    use crate::settings::AISettings;
    use crate::system::SystemStats;
    use crate::workspaces::user_profiles::UserProfiles;
    use crate::workspaces::user_workspaces::UserWorkspaces;
    use crate::NetworkStatus;

    fn mock_notebook_with_revision(id: i64, title: &str, revision: Revision) -> NotebookObject {
        let sync_id = SyncId::ServerId(id.into());
        let mut metadata = StoredObjectMetadata::mock();
        metadata.revision = Some(revision);

        NotebookObject::new(
            sync_id,
            NotebookObjectModel {
                title: title.to_string(),
                data: format!("{title} content"),
                ai_document_id: None,
                conversation_id: None,
            },
            metadata,
            StoredObjectPermissions::mock_personal(),
        )
    }

    fn initialize_app(app: &mut App) {
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(|_| SystemStats::new());
        app.add_singleton_model(|ctx| UserWorkspaces::mock(vec![], ctx));
        app.add_singleton_model(ObjectStoreModel::mock);
        app.add_singleton_model(|ctx| UpdateManager::new(None, ctx));
        app.add_singleton_model(|_| UserProfiles::new(Vec::new()));
        app.add_singleton_model(ObjectStoreViewModel::new);
        app.add_singleton_model(NotebookManager::mock);
        app.add_singleton_model(|_| SettingsManager::default());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.update(crate::settings::init_and_register_user_preferences);
        app.update(AISettings::register_and_subscribe_to_events);
    }

    #[test]
    fn zero_state_scores_reflect_recency() {
        App::test((), |mut app| async move {
            initialize_app(&mut app);

            let now = Utc::now();
            ObjectStoreModel::handle(&app).update(&mut app, |model, _| {
                for notebook in [
                    mock_notebook_with_revision(1, "oldest", (now - Duration::minutes(3)).into()),
                    mock_notebook_with_revision(2, "middle", (now - Duration::minutes(2)).into()),
                    mock_notebook_with_revision(3, "newest", (now - Duration::minutes(1)).into()),
                ] {
                    model.add_object(notebook.id, notebook);
                }
            });

            let data_source = NotebookDataSource::new(false);
            let results = app.read(|app| data_source.run_query(&Query::from(""), app).unwrap());

            assert_eq!(results.len(), 3);
            // run_query sorts descending by score, so first result should be newest
            let scores: Vec<_> = results.iter().map(|r| r.score()).collect();
            assert!(
                scores[0] > scores[1] && scores[1] > scores[2],
                "Expected scores in strictly descending order (newest first), got {scores:?}"
            );
        })
    }

    #[test]
    fn filtered_state_adds_recency_bonus_to_equal_matches() {
        App::test((), |mut app| async move {
            initialize_app(&mut app);

            let now = Utc::now();
            // All titles contain "plan" so fuzzy scores should be similar
            ObjectStoreModel::handle(&app).update(&mut app, |model, _| {
                for notebook in [
                    mock_notebook_with_revision(
                        1,
                        "my first plan",
                        (now - Duration::minutes(3)).into(),
                    ),
                    mock_notebook_with_revision(
                        2,
                        "my second plan",
                        (now - Duration::minutes(2)).into(),
                    ),
                    mock_notebook_with_revision(
                        3,
                        "my third plan",
                        (now - Duration::minutes(1)).into(),
                    ),
                ] {
                    model.add_object(notebook.id, notebook);
                }
            });

            let data_source = NotebookDataSource::new(false);
            let results = app.read(|app| data_source.run_query(&Query::from("plan"), app).unwrap());

            assert_eq!(results.len(), 3);
            // All match "plan" similarly; recency bonus should make newer items score higher
            let scores: Vec<_> = results.iter().map(|r| r.score()).collect();
            assert!(
                scores[0] > scores[1] && scores[1] > scores[2],
                "Expected scores in strictly descending order (newest first), got {scores:?}"
            );
        })
    }
}
