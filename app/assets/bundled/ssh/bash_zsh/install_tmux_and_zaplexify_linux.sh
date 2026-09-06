INSTALL_TMUX='set -e

_on_error() {
    local _msg=$(printf "{\"hook\": \"TmuxInstallFailed\", \"value\": { \"line\": \"$1\", \"command\": \"$2\" } }" | command -p od -An -v -tx1 | command -p tr -d " \n")
    printf '\''\033\120\044\144%s\234'\'' "$_msg"
    rm -rf "$HOME/.warp/tmux"
}
trap "_on_error \"\${LINENO}\" \"\$BASH_COMMAND\"" ERR

mkdir -p $HOME/.warp/tmux
pushd "$HOME/.warp/tmux"

ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64) ARCH_NAME=amd64; EXPECTED_SHA256=6d52c3badd5c73ecf3e80709510fb6913fa7497873478e31d513b37ac564a0f0 ;;
    aarch64) ARCH_NAME=arm64; EXPECTED_SHA256=842f960deb5faa04e6236da727c98fc9200df346497c7950972b874b609f7ec1 ;;
    *) echo "Unsupported architecture $ARCH"; exit 1 ;;
esac

URL="https://github.com/warpdotdev/portable-tmux/releases/download/tmux-3.5a/tmux-${ARCH_NAME}.tar.gz"

(curl -fo tmux.tar.gz -L "$URL" || wget -qO tmux.tar.gz "$URL")
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_SHA256=$(sha256sum tmux.tar.gz)
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL_SHA256=$(shasum -a 256 tmux.tar.gz)
else
    echo "No SHA-256 utility available" >&2; exit 1
fi
ACTUAL_SHA256=${ACTUAL_SHA256%% *}
[ "$ACTUAL_SHA256" = "$EXPECTED_SHA256" ] || { echo "tmux archive SHA-256 mismatch" >&2; exit 1; }
tar -xf tmux.tar.gz

INSTALL_PATH="$HOME/.warp/tmux/local"
echo "TERM=tmux-256color LD_LIBRARY_PATH=\"$INSTALL_PATH/lib\" TERMINFO=\"$INSTALL_PATH/share/terminfo/\" \"$INSTALL_PATH/bin/tmux\" \"\$@\";" > ~/.warp/tmux/execute_tmux.sh
chmod +x ~/.warp/tmux/execute_tmux.sh;'

bash <<< "$INSTALL_TMUX" && ~/.warp/tmux/execute_tmux.sh -Lwarp -CC && exit
