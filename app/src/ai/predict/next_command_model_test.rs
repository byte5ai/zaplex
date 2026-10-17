use crate::completer::SessionContext;
use crate::terminal::model::session::Session;
use crate::terminal::model::session::{
    command_executor::testing::TestCommandExecutor, SessionInfo,
};
use typed_path::TypedPathBuf;
use warp_completer::signatures::CommandRegistry;
use warp_completer::{meta::SpannedItem, parsers::ParsedToken};
use warpui::App;

use super::*;

fn test_session_context(cwd: TypedPathBuf, app: &App) -> SessionContext {
    let session = Session::new(
        SessionInfo::new_for_test(),
        Arc::new(TestCommandExecutor::default()),
    );
    app.read(|ctx| SessionContext::new(session, CommandRegistry::default().into(), cwd, ctx))
}

#[test]
fn test_feature_flag_arg_is_valid_with_no_whitespace_before_arg() {
    App::test((), |app| async move {
        let ctx = test_session_context(TypedPathBuf::from("/test/home/"), &app);

        let full_command = "cargo run --features=with_local_server,fast_dev";
        let with_local_server_arg = ParsedExpression::new(
            Expression::ValidatableArgument(vec![ArgType::Generator("feature_flags".into())]),
            ParsedToken::new("with_local_server".to_string()),
        )
        .spanned((21, 38));
        let fast_dev_arg = ParsedExpression::new(
            Expression::ValidatableArgument(vec![ArgType::Generator("feature_flags".into())]),
            ParsedToken::new("fast_dev".to_string()),
        )
        .spanned((39, 47));
        let is_valid = is_arg_valid(full_command, &with_local_server_arg, &ctx, None).await;
        assert!(is_valid);

        let is_valid = is_arg_valid(full_command, &fast_dev_arg, &ctx, None).await;
        assert!(is_valid);

        let is_valid = is_command_valid(full_command, Some(&ctx), None).await;
        assert!(is_valid);
    });
}
