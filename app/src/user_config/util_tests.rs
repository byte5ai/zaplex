use super::*;

#[test]
fn file_name_to_human_readable_name_test() {
    assert_eq!(
        "Solarized Dark",
        file_name_to_human_readable_name("solarized_dark")
    );
    assert_eq!(
        "Solarized Dark",
        file_name_to_human_readable_name("solarized_dark.yaml")
    );
    assert_eq!(
        "Solarized Dark",
        file_name_to_human_readable_name("SOLARIZED_DARK.yaml")
    );
    assert_eq!(
        "Solarizeddark",
        file_name_to_human_readable_name("solarizeddark.yaml")
    );
}
