# bash completion for scripts/dev-endpoint. Install as
# ~/.local/share/bash-completion/completions/dev-endpoint.

_dev_endpoint() {
    local cur prev words cword
    _init_completion || return

    local i group="" cmd=""
    for ((i = 1; i < cword; i++)); do
        case "${words[i]}" in
        --endpoint) ((i++)) ;;
        -*) ;;
        *)
            if [[ -z "$group" ]]; then group="${words[i]}"
            elif [[ -z "$cmd" ]]; then cmd="${words[i]}"
            fi
            ;;
        esac
    done

    case "$prev" in
    --endpoint) COMPREPLY=($(compgen -W "https://dev.anibeam.app http://127.0.0.1:8796" -- "$cur")); return ;;
    --since|--before|--top) return ;;
    esac

    if [[ -z "$group" ]]; then
        COMPREPLY=($(compgen -W "usage token --endpoint --no-keyring --help" -- "$cur"))
        return
    fi
    case "$group" in
    usage)
        case "$cmd" in
        "") COMPREPLY=($(compgen -W "list pull summary prune --help" -- "$cur")) ;;
        list) COMPREPLY=($(compgen -W "--since --help" -- "$cur")) ;;
        pull) [[ "$cur" == -* ]] && COMPREPLY=($(compgen -W "--help" -- "$cur")) || _filedir -d ;;
        summary) [[ "$cur" == -* ]] && COMPREPLY=($(compgen -W "--top --help" -- "$cur")) || _filedir -d ;;
        prune) COMPREPLY=($(compgen -W "--before --help" -- "$cur")) ;;
        esac
        ;;
    token)
        case "$cmd" in
        "") COMPREPLY=($(compgen -W "new set show --help" -- "$cur")) ;;
        new) COMPREPLY=($(compgen -W "--force --help" -- "$cur")) ;;
        *) COMPREPLY=($(compgen -W "--help" -- "$cur")) ;;
        esac
        ;;
    esac
}
complete -F _dev_endpoint dev-endpoint
