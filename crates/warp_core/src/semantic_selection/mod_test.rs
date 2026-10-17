use super::*;

#[test]
fn test_other_url_selection() {
    let sel = SemanticSelection::mock(true, "");
    assert_eq!(
        sel.smart_search("ssh://git@github.com:acarl005/dotfiles.git", 0.into()),
        Some(ByteOffset::range(0..42))
    );
    assert_eq!(
        sel.smart_search("data hdfs://hadoopNS/data/users.csv there", 20.into()),
        Some(ByteOffset::range(5..35))
    );
    assert_eq!(
        sel.smart_search("send files here&ftp://thing.网站/file please", 30.into()),
        Some(ByteOffset::range(16..39))
    );
    assert_eq!(
        sel.smart_search("http://[2001:db8::1]:80", 10.into()),
        Some(ByteOffset::range(0..23))
    );
}
