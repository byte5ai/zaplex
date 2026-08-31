use std::collections::HashMap;

use warpui::{Entity, ModelContext, SingletonEntity};

use crate::{
    cloud_object::{
        model::persistence::ObjectStoreModel,
        update_manager::{
            InitiatedBy, ObjectOperation, OperationSuccessType, UpdateManager, UpdateManagerEvent,
        },
        Owner, StoredObjectEventEntrypoint,
    },
    drive::folders::FolderId,
    notebooks::NotebookObjectModel,
    server::ids::{ClientId, SyncId},
    workflows::{workflow::Workflow, workflow_enum::WorkflowEnum},
};

use super::nodes::{self, FileId};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct ImportGeneration(ClientId);

impl ImportGeneration {
    pub(super) fn new() -> Self {
        Self(ClientId::new())
    }
}

impl Default for ImportGeneration {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) enum ImportQueueEvent {
    FileCompleted {
        generation: ImportGeneration,
        file_id: FileId,
        server_id: Option<String>,
    },
    FolderCompleted {
        generation: ImportGeneration,
        folder_id: nodes::FolderId,
        server_id: Option<String>,
    },
    FileSavedLocally {
        generation: ImportGeneration,
        file_id: FileId,
    },
}

impl ImportQueueEvent {
    pub(super) fn generation(&self) -> ImportGeneration {
        match self {
            Self::FileCompleted { generation, .. }
            | Self::FolderCompleted { generation, .. }
            | Self::FileSavedLocally { generation, .. } => *generation,
        }
    }
}

#[derive(Debug)]
pub(super) enum ParentId {
    FolderToUpload(ClientId),
    InitialFolder(Option<SyncId>),
}

#[derive(Debug)]
pub(super) struct ImportQueueArgs {
    pub(super) generation: ImportGeneration,
    pub(super) owner: Owner,
    pub(super) parent_id: ParentId,
    pub(super) content: RequestContent,
}

#[derive(Debug)]
pub(super) enum RequestContent {
    Folder {
        name: String,
        client_id: ClientId,
        folder_id: nodes::FolderId,
    },
    Notebook {
        title: String,
        data: String,
        client_id: ClientId,
        file_id: FileId,
    },
    Workflow {
        workflows: Vec<(Workflow, ClientId)>,
        workflow_enums: HashMap<ClientId, WorkflowEnum>,
        file_id: FileId,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TrackedFile {
    generation: ImportGeneration,
    file_id: FileId,
}

#[derive(Default)]
struct FileCompletionCounter {
    client_id_to_file: HashMap<ClientId, TrackedFile>,
    file_to_counter: HashMap<TrackedFile, usize>,
}

impl FileCompletionCounter {
    fn request_completed(&mut self, client_id: ClientId) -> Option<TrackedFile> {
        let tracked_file = self.client_id_to_file.remove(&client_id)?;
        let Some(counter) = self.file_to_counter.get_mut(&tracked_file) else {
            log::error!("File completion counter should exist but it doesn't");
            return None;
        };
        *counter = counter.saturating_sub(1);
        if *counter == 0 {
            self.file_to_counter.remove(&tracked_file);
            Some(tracked_file)
        } else {
            None
        }
    }

    fn add_entry(&mut self, client_id: ClientId, generation: ImportGeneration, file_id: FileId) {
        let tracked_file = TrackedFile {
            generation,
            file_id,
        };
        self.client_id_to_file.insert(client_id, tracked_file);
        *self.file_to_counter.entry(tracked_file).or_insert(0) += 1;
    }

    fn cancel_generation(&mut self, generation: ImportGeneration) {
        self.client_id_to_file
            .retain(|_, tracked| tracked.generation != generation);
        self.file_to_counter
            .retain(|tracked, _| tracked.generation != generation);
    }
}

pub(super) struct ImportQueue {
    queue: Vec<ImportQueueArgs>,
    client_to_server_id: HashMap<ClientId, (ImportGeneration, Option<FolderId>)>,
    client_to_node_folder_id: HashMap<ClientId, (ImportGeneration, nodes::FolderId)>,
    file_completion: FileCompletionCounter,
}

impl ImportQueue {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let update_manager = UpdateManager::handle(ctx);
        ctx.subscribe_to_model(&update_manager, |me, event, ctx| {
            me.handle_update_manager_event(event, ctx);
        });

        Self {
            queue: Vec::new(),
            client_to_server_id: HashMap::default(),
            file_completion: Default::default(),
            client_to_node_folder_id: HashMap::default(),
        }
    }

    // Whether all dependencies of an item has been sync-ed.
    fn dependency_synced(&self, item: &ImportQueueArgs) -> bool {
        match &item.parent_id {
            ParentId::FolderToUpload(id) => {
                self.client_to_server_id
                    .get(id)
                    .is_some_and(|(generation, server_id)| {
                        *generation == item.generation && server_id.is_some()
                    })
            }
            ParentId::InitialFolder(_) => true,
        }
    }

    pub fn cancel_generation(&mut self, generation: ImportGeneration) {
        self.queue.retain(|item| item.generation != generation);
        self.client_to_server_id
            .retain(|_, (tracked_generation, _)| *tracked_generation != generation);
        self.client_to_node_folder_id
            .retain(|_, (tracked_generation, _)| *tracked_generation != generation);
        self.file_completion.cancel_generation(generation);
    }

    // Enqueue a new request to the import queue.
    pub fn enqueue(&mut self, arg: ImportQueueArgs, ctx: &mut ModelContext<Self>) {
        // Update internal tracker of the object.
        match &arg.content {
            RequestContent::Folder {
                client_id,
                folder_id,
                ..
            } => {
                self.client_to_server_id
                    .insert(*client_id, (arg.generation, None));
                self.client_to_node_folder_id
                    .insert(*client_id, (arg.generation, *folder_id));
            }
            RequestContent::Notebook {
                client_id, file_id, ..
            } => self
                .file_completion
                .add_entry(*client_id, arg.generation, *file_id),
            RequestContent::Workflow {
                workflows, file_id, ..
            } => {
                for (_, client_id) in workflows {
                    self.file_completion
                        .add_entry(*client_id, arg.generation, *file_id);
                }
            }
        }

        self.queue.push(arg);
        self.dequeue(ctx);
    }

    // Dequeue a new request from the import queue.
    pub fn dequeue(&mut self, ctx: &mut ModelContext<Self>) {
        if self.queue.is_empty() {
            return;
        }

        if let Some(idx) = self
            .queue
            .iter()
            .position(|item| self.dependency_synced(item))
        {
            let dequeued_item = self.queue.remove(idx);
            let parent_id = match dequeued_item.parent_id {
                ParentId::FolderToUpload(client_id) => Some(SyncId::ServerId(
                    self.client_to_server_id
                        .get(&client_id)
                        .expect("Client id entry should exist")
                        .1
                        .expect("Server id entry should exist")
                        .into(),
                )),
                ParentId::InitialFolder(folder_id) => folder_id,
            };

            match dequeued_item.content {
                RequestContent::Folder {
                    name, client_id, ..
                } => {
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.create_folder(
                            name,
                            dequeued_item.owner,
                            client_id,
                            parent_id,
                            false,
                            InitiatedBy::User,
                            ctx,
                        );
                    });
                }
                RequestContent::Notebook {
                    title,
                    data,
                    client_id,
                    file_id,
                } => {
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        update_manager.create_notebook(
                            client_id,
                            dequeued_item.owner,
                            parent_id,
                            NotebookObjectModel {
                                title,
                                data,
                                ai_document_id: None,
                                conversation_id: None,
                            },
                            StoredObjectEventEntrypoint::ImportModal,
                            false,
                            ctx,
                        );
                    });
                    ctx.emit(ImportQueueEvent::FileSavedLocally {
                        generation: dequeued_item.generation,
                        file_id,
                    });
                }
                RequestContent::Workflow {
                    workflows,
                    workflow_enums,
                    file_id,
                } => {
                    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                        // Create any new workflow enums
                        for (client_id, workflow_enum) in workflow_enums {
                            update_manager.create_workflow_enum(
                                workflow_enum,
                                dequeued_item.owner,
                                client_id,
                                StoredObjectEventEntrypoint::ImportModal,
                                false,
                                ctx,
                            );
                        }

                        // Create the workflow
                        for (workflow, client_id) in workflows {
                            update_manager.create_workflow(
                                workflow,
                                dequeued_item.owner,
                                parent_id,
                                client_id,
                                StoredObjectEventEntrypoint::ImportModal,
                                false,
                                ctx,
                            );
                        }
                    });
                    ctx.emit(ImportQueueEvent::FileSavedLocally {
                        generation: dequeued_item.generation,
                        file_id,
                    });
                }
            }
            self.dequeue(ctx);
        }
    }

    fn handle_update_manager_event(
        &mut self,
        event: &UpdateManagerEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        let UpdateManagerEvent::ObjectOperationComplete { result } = event else {
            return;
        };

        if matches!(&result.operation, ObjectOperation::Create { .. }) {
            let Some(client_id) = result.client_id else {
                return;
            };

            let is_successful = matches!(&result.success_type, OperationSuccessType::Success);
            let server_id = result.server_id;
            if let Some(tracked_file) = self.file_completion.request_completed(client_id) {
                ctx.emit(ImportQueueEvent::FileCompleted {
                    generation: tracked_file.generation,
                    file_id: tracked_file.file_id,
                    server_id: server_id.map(|server_id| server_id.uid()),
                });
                return;
            }

            let Some((generation, node_id)) =
                self.client_to_node_folder_id.get(&client_id).copied()
            else {
                return;
            };

            // Return early if we are not successfully uploading a folder.
            if !is_successful {
                self.client_to_node_folder_id.remove(&client_id);
                self.client_to_server_id.remove(&client_id);
                ctx.emit(ImportQueueEvent::FolderCompleted {
                    generation,
                    folder_id: node_id,
                    server_id: server_id.map(|server_id| server_id.uid()),
                });
                return;
            }

            let object_store_model = ObjectStoreModel::as_ref(ctx);

            let Some(folder_id) = object_store_model
                .get_folder_by_uid(&result.server_id.expect("Expect id").uid())
                .and_then(|folder| folder.id.into_server())
            else {
                return;
            };

            let replaced = match self.client_to_server_id.get_mut(&client_id) {
                Some((tracked_generation, value))
                    if *tracked_generation == generation && value.is_none() =>
                {
                    *value = Some(folder_id.into());
                    true
                }
                Some(_) | None => false,
            };

            if replaced {
                self.client_to_node_folder_id.remove(&client_id);
                ctx.emit(ImportQueueEvent::FolderCompleted {
                    generation,
                    folder_id: node_id,
                    server_id: server_id.map(|server_id| server_id.uid()),
                });
                self.dequeue(ctx);
            }
        }
    }
}

impl Entity for ImportQueue {
    type Event = ImportQueueEvent;
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
