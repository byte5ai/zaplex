INSTALL_TMUX='set -e

_json_escape() {
    local _text=$1 _char _code _index
    for ((_index=0; _index<${#_text}; _index++)); do
        _char=${_text:_index:1}
        case "$_char" in
            '\''"'\'') printf '\''\\"'\'' ;;
            '\''\'\'') printf '\''\\\\'\'' ;;
            *) printf -v _code '\''%d'\'' "'\''$_char"
                if [ "$_code" -lt 32 ]; then
                    printf '\''\\u%04x'\'' "$_code"
                else
                    printf '\''%s'\'' "$_char"
                fi ;;
        esac
    done
}

_on_error() {
    local _msg
    _msg=$(printf '\''{"hook":"TmuxInstallFailed","value":{"line":"%s","command":"%s"}}'\'' "$1" "$(_json_escape "$2")" | command -p od -An -v -tx1 | command -p tr -d " \n")
    printf '\''\033\120\044\144%s\234'\'' "$_msg"
    rm -rf "$HOME/.warp/tmux"
}
trap "_on_error \"\${LINENO}\" \"\$BASH_COMMAND\"" ERR

sudo zypper refresh
sudo zypper install -y tmux'

bash <<< "$INSTALL_TMUX" && _check_tmux && command tmux -Lwarp -CC && exit
