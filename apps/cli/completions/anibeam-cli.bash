# bash completion for anibeam-cli, the core from the terminal. Install it as
# anibeam-cli in a bash-completion directory, such as
# /usr/share/bash-completion/completions.

_anibeam_cli() {
  local cur prev cmd sub i
  cur=${COMP_WORDS[COMP_CWORD]}
  prev=${COMP_WORDS[COMP_CWORD-1]}
  cmd="" sub=""
  for (( i = 1; i < COMP_CWORD; i++ )); do
    case ${COMP_WORDS[i]} in
      --root) (( i++ )) ;;
      -*) ;;
      *) if [[ -z $cmd ]]; then cmd=${COMP_WORDS[i]}; elif [[ -z $sub ]]; then sub=${COMP_WORDS[i]}; fi ;;
    esac
  done
  if [[ $prev == --root ]]; then
    COMPREPLY=( $(compgen -d -- "$cur") )
    return
  fi
  case $cmd in
    "") COMPREPLY=( $(compgen -W "call events sources scan list show jobs import export usage help --root --help --version" -- "$cur") ) ;;
    call) COMPREPLY=( $(compgen -W "--json --wait" -- "$cur") ) ;;
    events)
      if [[ $prev == --level ]]; then COMPREPLY=( $(compgen -W "debug info warn error" -- "$cur") )
      else COMPREPLY=( $(compgen -W "--follow --level" -- "$cur") ); fi ;;
    scan) COMPREPLY=( $(compgen -W "--source --wait" -- "$cur") ) ;;
    list)
      if [[ $prev == --direction ]]; then COMPREPLY=( $(compgen -W "asc desc" -- "$cur") )
      else COMPREPLY=( $(compgen -W "--tab --sort --direction --query" -- "$cur") ); fi ;;
    import) COMPREPLY=( $(compgen -W "--wait" -- "$cur") $(compgen -f -- "$cur") ) ;;
    export) COMPREPLY=( $(compgen -W "--private --wait" -- "$cur") $(compgen -f -- "$cur") ) ;;
    usage)
      case $sub in
        "") COMPREPLY=( $(compgen -W "show export send clear" -- "$cur") ) ;;
        export) COMPREPLY=( $(compgen -f -- "$cur") ) ;;
        send) COMPREPLY=( $(compgen -W "--yes" -- "$cur") ) ;;
      esac ;;
  esac
}
complete -F _anibeam_cli anibeam-cli
