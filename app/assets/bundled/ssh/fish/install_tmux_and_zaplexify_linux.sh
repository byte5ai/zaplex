set INSTALL_TMUX 'set -e

_json_escape() {
    local _text=$1 _char _code _index
    for ((_index=0; _index<${#_text}; _index++)); do
        _char=${_text:_index:1}
        case "$_char" in
            \'"\') printf \'\\\\"\' ;;
            \'\\\') printf \'\\\\\\\\\' ;;
            *) printf -v _code \'%d\' "\'$_char"
                if [ "$_code" -lt 32 ]; then
                    printf \'\\\\u%04x\' "$_code"
                else
                    printf \'%s\' "$_char"
                fi ;;
        esac
    done
}

_on_error() {
    local _msg
    _msg=$(printf \'{"hook":"TmuxInstallFailed","value":{"line":"%s","command":"%s"}}\' "$1" "$(_json_escape "$2")" | command -p od -An -v -tx1 | command -p tr -d " \\n")
    printf \'\\033\\120\\044\\144%s\\234\' "$_msg"
    rm -rf "$HOME/.warp/tmux"
}
trap "_on_error \\"\\${LINENO}\\" \\"\\$BASH_COMMAND\\"" ERR

mkdir -p "$HOME/.warp/tmux"
pushd "$HOME/.warp/tmux"

ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64) ARCH_NAME=amd64; EXPECTED_SHA256=6d52c3badd5c73ecf3e80709510fb6913fa7497873478e31d513b37ac564a0f0 ;;
    aarch64) ARCH_NAME=arm64; EXPECTED_SHA256=842f960deb5faa04e6236da727c98fc9200df346497c7950972b874b609f7ec1 ;;
    *) _on_error "$LINENO" "Unsupported architecture $ARCH"; exit 1 ;;
esac

URL="https://github.com/warpdotdev/portable-tmux/releases/download/tmux-3.5a/tmux-${ARCH_NAME}.tar.gz"

(curl -fo tmux.tar.gz -L "$URL" || wget -qO tmux.tar.gz "$URL")
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_SHA256=$(sha256sum tmux.tar.gz)
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL_SHA256=$(shasum -a 256 tmux.tar.gz)
else
    _on_error "$LINENO" "No SHA-256 utility available"; exit 1
fi
ACTUAL_SHA256=${ACTUAL_SHA256%% *}
[ "$ACTUAL_SHA256" = "$EXPECTED_SHA256" ] || { _on_error "$LINENO" "tmux archive SHA-256 mismatch"; exit 1; }
tar -xf tmux.tar.gz

printf \'%s\\n\' \'#!/bin/sh\' \'INSTALL_PATH="$HOME/.warp/tmux/local"\' \'TERM=tmux-256color LD_LIBRARY_PATH="$INSTALL_PATH/lib" TERMINFO="$INSTALL_PATH/share/terminfo/" exec "$INSTALL_PATH/bin/tmux" "$@"\' > "$HOME/.warp/tmux/execute_tmux.sh"
chmod +x "$HOME/.warp/tmux/execute_tmux.sh";'

bash -c "$INSTALL_TMUX" && "$HOME/.warp/tmux/execute_tmux.sh" -Lwarp -CC && exit
