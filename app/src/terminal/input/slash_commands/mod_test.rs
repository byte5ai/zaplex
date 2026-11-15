use std::path::PathBuf;
use std::sync::Arc;

use warp_util::path::LineAndColumnArg;

use super::open_file_command_path;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{Session, SessionInfo};
use crate::terminal::shell::ShellType;

fn bash_session() -> Session {
    Session::new(
        SessionInfo::new_for_test().with_shell_type(ShellType::Bash),
        Arc::new(TestCommandExecutor::default()),
    )
}

// Resolves the path only; the test never creates files in the home directory.
#[test]
fn open_file_command_expands_tilde_against_home_not_current_directory() {
    let home = dirs::home_dir().expect("the test runner has a home directory");
    let current_dir = "/tmp/zaplex-open-file-cwd";

    let (path, line_col) =
        open_file_command_path(&bash_session(), current_dir, "~/notes/todo.txt:3");

    assert_eq!(path, home.join("notes").join("todo.txt"));
    assert_eq!(
        line_col,
        Some(LineAndColumnArg {
            line_num: 3,
            column_num: None,
        })
    );
}

#[test]
fn open_file_command_joins_relative_paths_to_current_directory() {
    let (path, line_col) = open_file_command_path(
        &bash_session(),
        "/tmp/zaplex-open-file-cwd",
        "notes/todo.txt",
    );

    assert_eq!(
        path,
        PathBuf::from("/tmp/zaplex-open-file-cwd/notes/todo.txt")
    );
    assert_eq!(line_col, None);
}
