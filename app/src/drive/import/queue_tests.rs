use super::*;

#[test]
fn cancelled_generation_cannot_complete_reused_file_id() {
    let generation_a = ImportGeneration::default();
    let generation_b = ImportGeneration::new();
    let file_id = FileId::first_id();
    let client_a = ClientId::new();
    let client_b = ClientId::new();
    let mut counter = FileCompletionCounter::default();

    counter.add_entry(client_a, generation_a, file_id);
    counter.cancel_generation(generation_a);
    counter.add_entry(client_b, generation_b, file_id);

    assert_eq!(counter.request_completed(client_a), None);
    assert_eq!(counter.file_to_counter.len(), 1);
    assert_eq!(
        counter.request_completed(client_b),
        Some(TrackedFile {
            generation: generation_b,
            file_id,
        })
    );
    assert!(counter.client_id_to_file.is_empty());
    assert!(counter.file_to_counter.is_empty());
}

#[test]
fn completion_mapping_is_removed_after_first_delivery() {
    let generation = ImportGeneration::default();
    let file_id = FileId::first_id();
    let client_id = ClientId::new();
    let mut counter = FileCompletionCounter::default();
    counter.add_entry(client_id, generation, file_id);

    assert!(counter.request_completed(client_id).is_some());
    assert_eq!(counter.request_completed(client_id), None);
    assert!(counter.client_id_to_file.is_empty());
    assert!(counter.file_to_counter.is_empty());
}

#[test]
fn cancelling_generations_removes_all_queue_tracking() {
    let generation_a = ImportGeneration::default();
    let generation_b = ImportGeneration::new();
    let folder_client_a = ClientId::new();
    let folder_client_b = ClientId::new();
    let file_client_a = ClientId::new();
    let file_client_b = ClientId::new();
    let file_id = FileId::first_id();
    let mut queue = ImportQueue {
        queue: vec![
            ImportQueueArgs {
                generation: generation_a,
                owner: Owner::mock_current_user(),
                parent_id: ParentId::InitialFolder(None),
                content: RequestContent::Notebook {
                    title: "a".to_string(),
                    data: String::new(),
                    client_id: file_client_a,
                    file_id,
                },
            },
            ImportQueueArgs {
                generation: generation_b,
                owner: Owner::mock_current_user(),
                parent_id: ParentId::InitialFolder(None),
                content: RequestContent::Notebook {
                    title: "b".to_string(),
                    data: String::new(),
                    client_id: file_client_b,
                    file_id,
                },
            },
        ],
        client_to_server_id: HashMap::from([
            (folder_client_a, (generation_a, None)),
            (folder_client_b, (generation_b, None)),
        ]),
        client_to_node_folder_id: HashMap::from([
            (folder_client_a, (generation_a, nodes::FolderId::root_id())),
            (folder_client_b, (generation_b, nodes::FolderId::root_id())),
        ]),
        file_completion: FileCompletionCounter::default(),
    };
    queue
        .file_completion
        .add_entry(file_client_a, generation_a, file_id);
    queue
        .file_completion
        .add_entry(file_client_b, generation_b, file_id);

    queue.cancel_generation(generation_a);

    assert_eq!(queue.queue.len(), 1);
    assert_eq!(queue.queue[0].generation, generation_b);
    assert_eq!(queue.client_to_server_id.len(), 1);
    assert!(queue.client_to_server_id.contains_key(&folder_client_b));
    assert_eq!(queue.client_to_node_folder_id.len(), 1);
    assert!(queue
        .client_to_node_folder_id
        .contains_key(&folder_client_b));
    assert_eq!(queue.file_completion.client_id_to_file.len(), 1);
    assert!(queue
        .file_completion
        .client_id_to_file
        .contains_key(&file_client_b));

    queue.cancel_generation(generation_b);

    assert!(queue.queue.is_empty());
    assert!(queue.client_to_server_id.is_empty());
    assert!(queue.client_to_node_folder_id.is_empty());
    assert!(queue.file_completion.client_id_to_file.is_empty());
    assert!(queue.file_completion.file_to_counter.is_empty());
}
