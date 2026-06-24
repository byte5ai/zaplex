use crate::cloud_object::ObjectType;
use crate::code_review::diff_state::DiffMode;
use crate::search::mixer::SearchMixer;

pub type AIContextMenuMixer = SearchMixer<AIContextMenuSearchableAction>;

#[derive(Debug, Clone, PartialEq)]
pub enum AIContextMenuSearchableAction {
    InsertFilePath {
        /// This is the file path relative to the root of the current git
        /// repository. If this changes, this could break how we resolve
        /// the file path outside of AI mode, so just note the downstream
        /// dependencies.
        file_path: String,
    },
    InsertText {
        /// Text to insert into the input buffer.
        text: String,
    },
    InsertDriveObject {
        /// Drive object type (Workflow, Notebook, etc.).
        object_type: ObjectType,
        /// Drive object UID to attach.
        object_uid: String,
        /// @ name displayed in Agent Mode input box.
        display_name: String,
    },
    InsertPlan {
        /// AI document UID to attach.
        ai_document_uid: String,
        /// @ name displayed in Agent Mode input box.
        display_name: String,
    },
    InsertDiffSet {
        /// The diff mode indicating what base to compare against
        diff_mode: DiffMode,
    },
    InsertConversation {
        /// Conversation ID to attach.
        conversation_id: String,
        /// @ title displayed in Agent Mode input box.
        title: String,
    },
    InsertSkill {
        /// The skill name to insert as /{name} into the buffer.
        name: String,
    },
}
