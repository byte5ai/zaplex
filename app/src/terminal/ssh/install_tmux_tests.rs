const AMD64_SHA256: &str = "6d52c3badd5c73ecf3e80709510fb6913fa7497873478e31d513b37ac564a0f0";
const ARM64_SHA256: &str = "842f960deb5faa04e6236da727c98fc9200df346497c7950972b874b609f7ec1";

#[test]
fn portable_tmux_archives_are_verified_before_extraction() {
    let scripts = [
        include_str!("../../../assets/bundled/ssh/bash_zsh/install_tmux_and_zaplexify_linux.sh"),
        include_str!("../../../assets/bundled/ssh/fish/install_tmux_and_zaplexify_linux.sh"),
    ];

    for script in scripts {
        assert!(script.contains(AMD64_SHA256));
        assert!(script.contains(ARM64_SHA256));
        assert!(script.contains("No SHA-256 utility available"));
        let verification = script.find("tmux archive SHA-256 mismatch").unwrap();
        let extraction = script.find("tar -xf tmux.tar.gz").unwrap();
        assert!(verification < extraction);
    }
}
