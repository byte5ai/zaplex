use super::should_open_notebook_in_new_window;

#[test]
fn external_markdown_document_uses_dedicated_viewer_window() {
    assert!(should_open_notebook_in_new_window(true, true));
}

#[test]
fn internal_markdown_open_reuses_existing_window() {
    assert!(!should_open_notebook_in_new_window(false, true));
}

#[test]
fn internal_markdown_open_without_existing_window_creates_one() {
    assert!(should_open_notebook_in_new_window(false, false));
}
