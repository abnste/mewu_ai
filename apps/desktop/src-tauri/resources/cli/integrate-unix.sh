#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
# Noninteractive, user-level integration; safe for a native host to invoke.
# Never persist APPDIR or a FUSE mount. Shell setup is explicit and reversible.
set -eu
umask 077

fail() { printf '%s\n' "$1" >&2; exit 1; }
action=install
kind=
source=
configure_shell=false
bin_dir=${HOME:?HOME is required}/.local/bin
while [ "$#" -gt 0 ]; do
  case "$1" in
    --install) action=install; shift ;;
    --uninstall) action=uninstall; shift ;;
    --configure-shell) configure_shell=true; shift ;;
    --macos-app|--appimage)
      [ "$#" -ge 2 ] || fail MEWU_CLI_INVALID_ARGUMENTS
      kind=$1; source=$2; shift 2 ;;
    --bin-dir)
      [ "$#" -ge 2 ] || fail MEWU_CLI_INVALID_ARGUMENTS
      bin_dir=$2; shift 2 ;;
    *) fail MEWU_CLI_INVALID_ARGUMENTS ;;
  esac
done
[ "$(id -u)" != 0 ] || fail MEWU_CLI_USER_INSTALL_ONLY
case "$bin_dir" in /*) ;; *) fail MEWU_CLI_ABSOLUTE_DIRECTORY_REQUIRED ;; esac
carriage_return=$(printf '\r')
case "$bin_dir$source" in *'
'*|*"$carriage_return"*) fail MEWU_CLI_INVALID_PATH ;; esac
mkdir -p "$bin_dir"
bin_dir=$(CDPATH= cd -P "$bin_dir" && pwd -P)
case "$bin_dir" in *:*) fail MEWU_CLI_INVALID_PATH ;; esac
target=$bin_dir/mewu
lock=$bin_dir/.mewu-cli-install.lock
mkdir "$lock" 2>/dev/null || fail MEWU_CLI_INTEGRATION_BUSY
temporary=$bin_dir/.mewu-cli-$$.tmp
profile_lock=
profile_backup=
profile_candidate=
profile_block=
profile_found=
cleanup() {
  rm -f "$temporary"
  for scratch in "$profile_backup" "$profile_candidate" "$profile_block" "$profile_found"; do
    if [ -n "$scratch" ]; then rm -f "$scratch"; fi
  done
  if [ -n "$profile_lock" ]; then rmdir "$profile_lock" 2>/dev/null || :; fi
  rmdir "$lock" 2>/dev/null || :
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

# Quote a literal path; neither launchers nor startup files evaluate input.
quote() { printf "'"; printf '%s' "$1" | sed "s/'/'\\\\''/g"; printf "'"; }
file_stamp() {
  stat -c '%d:%i:%a:%u:%s:%Y:%Z' "$1" 2>/dev/null || stat -f '%d:%i:%Lp:%u:%z:%m:%c' "$1" 2>/dev/null
}
file_mode() { stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1" 2>/dev/null; }
file_owner() { stat -c '%u' "$1" 2>/dev/null || stat -f '%u' "$1" 2>/dev/null; }
profile_changed=false
configure_profile() {
  user_shell=${SHELL-}
  case "${user_shell##*/}" in
    zsh) profile_dir=${ZDOTDIR:-$HOME}; profile_name=.zshrc ;;
    bash)
      profile_dir=$HOME; profile_name=.bashrc
      if [ "$(uname -s)" = Darwin ]; then
        profile_name=.bash_profile
        for candidate_name in .bash_profile .bash_login .profile; do
          if [ -e "$profile_dir/$candidate_name" ] || [ -L "$profile_dir/$candidate_name" ]; then
            profile_name=$candidate_name; break
          fi
        done
      fi ;;
    *) return 1 ;;
  esac
  case "$profile_dir" in /*) ;; *) return 1 ;; esac
  case "$profile_dir" in *'
'*|*"$carriage_return"*) return 1 ;; esac
  [ -d "$profile_dir" ] || return 1
  profile_dir=$(CDPATH= cd -P "$profile_dir" && pwd -P) || return 1
  profile=$profile_dir/$profile_name
  [ ! -L "$profile" ] || return 1
  if [ -e "$profile" ]; then [ -f "$profile" ] && [ -r "$profile" ] && [ -w "$profile" ] || return 1; fi
  # Also serialize installations choosing different bin directories. Never take
  # ownership of a pre-existing lock or source/evaluate the user's startup file.
  mkdir "$profile.mewu-cli.lock" 2>/dev/null || return 1
  profile_lock=$profile.mewu-cli.lock
  profile_backup=$(mktemp "$profile_dir/.mewu-cli-original.XXXXXXXX") || return 1
  profile_candidate=$(mktemp "$profile_dir/.mewu-cli-replacement.XXXXXXXX") || return 1
  profile_block=$(mktemp "$profile_dir/.mewu-cli-block.XXXXXXXX") || return 1
  profile_found=$(mktemp "$profile_dir/.mewu-cli-found.XXXXXXXX") || return 1
  original_exists=false
  original_stamp=
  original_mode=600
  if [ -e "$profile" ]; then
    [ ! -L "$profile" ] && [ -f "$profile" ] || return 1
    [ "$(file_owner "$profile")" = "$(id -u)" ] || return 1
    original_exists=true
    original_stamp=$(file_stamp "$profile") || return 1
    original_mode=$(file_mode "$profile") || return 1
    case "$original_mode" in ''|*[!0-7]*) return 1 ;; esac
    # Read at most one byte beyond the limit, even for a huge or growing file.
    head -c 1048577 "$profile" > "$profile_backup" || return 1
    [ "$(wc -c < "$profile_backup")" -le 1048576 ] || return 1
    [ "$(file_stamp "$profile")" = "$original_stamp" ] || return 1
  fi
  {
    # The first LF belongs to our block. Removing the complete block therefore
    # restores even a user's missing final LF, CRLF or non-UTF-8 bytes exactly.
    printf '\n# >>> mewu CLI PATH v1 >>>\ncase ":${PATH-}:" in\n  *:'
    quote "$bin_dir"
    printf ':*) ;;\n  *) PATH='
    quote "$bin_dir"
    printf '${PATH:+":$PATH"}; export PATH ;;\nesac\n# <<< mewu CLI PATH v1 <<<\n'
  } > "$profile_block" || return 1
  # Only locate our two literal lines; compare the complete bytes before editing.
  # LC_ALL=C makes offsets bytes, including multibyte or invalid UTF-8 comments.
  marker_positions=$(LC_ALL=C awk -v BINMODE=3 '
    BEGIN { offset = 0; begins = 0; ends = 0; start = 0; finish = 0 }
    $0 == "# >>> mewu CLI PATH v1 >>>" { begins++; start = offset - 1 }
    $0 == "# <<< mewu CLI PATH v1 <<<" { ends++; finish = offset + length($0) + 1 }
    { offset += length($0) + 1 }
    END { printf "%d %d %d %d\n", begins, ends, start, finish }
  ' "$profile_backup") || return 1
  set -- $marker_positions
  if [ "$1" -eq 0 ] && [ "$2" -eq 0 ]; then
    if [ "$action" = uninstall ]; then return 0; fi
    cat "$profile_backup" "$profile_block" > "$profile_candidate" || return 1
  else
    [ "$1" -eq 1 ] && [ "$2" -eq 1 ] && [ "$3" -ge 0 ] && [ "$4" -gt "$3" ] || return 1
    start=$3; finish=$4
    head -c "$finish" "$profile_backup" | tail -c "$((finish - start))" > "$profile_found" || return 1
    cmp -s "$profile_found" "$profile_block" || return 1
    if [ "$action" = install ]; then return 0; fi
    head -c "$start" "$profile_backup" > "$profile_candidate" || return 1
    tail -c "+$((finish + 1))" "$profile_backup" >> "$profile_candidate" || return 1
  fi
  [ "$(wc -c < "$profile_candidate")" -le 1048576 ] || return 1
  chmod "$original_mode" "$profile_candidate" || return 1
  if [ "$original_exists" = true ]; then
    [ ! -L "$profile" ] && [ -f "$profile" ] || return 1
    [ "$(file_stamp "$profile")" = "$original_stamp" ] && cmp -s "$profile" "$profile_backup" || return 1
    # The directory lock covers our writers. The final comparison rejects an
    # observed external edit; POSIX rename cannot provide a cross-editor CAS.
    mv -f "$profile_candidate" "$profile" || return 1
  else
    # Atomic no-clobber publication: do not replace a newly created user file.
    ln "$profile_candidate" "$profile" 2>/dev/null || return 1
    rm -f "$profile_candidate" || return 1
  fi
  profile_changed=true
  return 0
}
path_configured=false
case ":${MEWU_CLI_ORIGINAL_PATH-${PATH-}}:" in *":$bin_dir:"*) path_configured=true ;; esac
shell_setup=true
if [ "$path_configured" = true ]; then shell_setup=false; fi

managed() {
  [ -f "$target" ] && [ ! -L "$target" ] || return 1
  prefix=$(LC_ALL=C head -c 256 "$target")
  case "$prefix" in '#!/bin/sh
# mewu-managed-cli-v1
'*) return 0 ;; *) return 1 ;; esac
}
if [ -e "$target" ] || [ -L "$target" ]; then
  managed || fail MEWU_CLI_EXISTING_COMMAND_PRESERVED
fi
if [ "$action" = uninstall ]; then
  changed=false
  if managed; then rm "$target"; changed=true; fi
  shell_setup=false
  if ! configure_profile; then shell_setup=true; fi
  if [ "$profile_changed" = true ]; then changed=true; fi
  printf '{"success":true,"changed":%s,"pathConfigured":%s,"shellSetupRequired":%s,"newShellRequired":%s}\n' "$changed" "$path_configured" "$shell_setup" "$profile_changed"
  exit 0
fi

case "$kind" in
  --macos-app)
    [ -d "$source" ] && [ ! -L "$source" ] || fail MEWU_CLI_APP_NOT_INSTALLED
    source=$(CDPATH= cd -P "$source" && pwd -P)
    case "$source" in /Volumes/*) fail MEWU_CLI_MOVE_APP_TO_APPLICATIONS_FIRST ;; *.app) ;; *) fail MEWU_CLI_APP_NOT_INSTALLED ;; esac
    executable=$source/Contents/MacOS/mewu
    [ -f "$source/Contents/MacOS/MewuAI" ] && [ -x "$source/Contents/MacOS/MewuAI" ] || fail MEWU_CLI_PACKAGE_INCOMPLETE
    [ -f "$executable" ] && [ -x "$executable" ] && [ ! -L "$executable" ] || fail MEWU_CLI_PACKAGE_INCOMPLETE
    ;;
  --appimage)
    [ -f "$source" ] && [ -x "$source" ] && [ ! -L "$source" ] || fail MEWU_CLI_APPIMAGE_NOT_INSTALLED
    source_dir=$(CDPATH= cd -P "$(dirname "$source")" && pwd -P)
    executable=$source_dir/$(basename "$source")
    case "$executable" in /tmp/*|/var/tmp/*|/run/*|*/.mount_*/*|*/AppImageLauncherFS/*) fail MEWU_CLI_APPIMAGE_PATH_NOT_PERSISTENT ;; esac
    ;;
  *) fail MEWU_CLI_SOURCE_REQUIRED ;;
esac

(
  set -C
  {
    printf '#!/bin/sh\n# mewu-managed-cli-v1\n# SPDX-License-Identifier: MPL-2.0\nexec '
    quote "$executable"
    if [ "$kind" = --appimage ]; then printf ' --cli'; fi
    printf ' "$@"\n'
  } > "$temporary"
)
chmod 700 "$temporary"
changed=true
if [ -f "$target" ] && cmp -s "$temporary" "$target"; then
  rm "$temporary"
  changed=false
else
  mv -f "$temporary" "$target"
fi
if [ "$configure_shell" = true ]; then
  if configure_profile; then shell_setup=false; else shell_setup=true; fi
fi
if [ "$profile_changed" = true ]; then changed=true; fi
new_shell=true
if [ "$path_configured" = true ]; then new_shell=false; fi
printf '{"success":true,"changed":%s,"pathConfigured":%s,"shellSetupRequired":%s,"newShellRequired":%s}\n' "$changed" "$path_configured" "$shell_setup" "$new_shell"
