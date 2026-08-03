use super::OSS_APPIMAGE_ASSET_NAME;

#[test]
fn oss_appimage_asset_name_matches_linux_bundle() {
    assert_eq!(OSS_APPIMAGE_ASSET_NAME, "Zaplex-x86_64.AppImage");
}
