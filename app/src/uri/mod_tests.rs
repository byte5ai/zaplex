use super::should_open_notebook_in_new_window;

#[test]
fn internal_markdown_open_reuses_existing_window() {
    assert!(!should_open_notebook_in_new_window(false, true));
}
