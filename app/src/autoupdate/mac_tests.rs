use super::{app_name, dmg_name_for_arch, executable_name};
use crate::channel::Channel;

#[test]
fn oss_release_names_match_macos_bundle_artifacts() {
    assert_eq!(app_name(Channel::Oss), "Zaplex.app");
    assert_eq!(dmg_name_for_arch(Channel::Oss, true), "Zaplex-arm64.dmg");
    assert_eq!(dmg_name_for_arch(Channel::Oss, false), "Zaplex-intel.dmg");
    assert_eq!(executable_name(Channel::Oss), "zaplex");
}
