#!/usr/bin/env bash
# Checks that an .rpm runs exactly the repository's maintainer scripts as
# root, and nothing else:
#
#   tools/ci/check-rpm-scriptlets.sh <package.rpm> <postinst.sh> <postrm.sh>
#
# - %post and %postun are present, their text is exactly <postinst.sh> and
#   <postrm.sh> (up to the final newline), their interpreter is absent
#   (rpm's default, /bin/sh: the Tauri bundler's rpm crate stores none) or
#   exactly the single entry /bin/sh (rpmbuild stores it), never another
#   program or arguments, and their flags are absent or 0 (no macro
#   expansion);
# - every other scriptlet is absent: %pre, %preun, %pretrans, %posttrans,
#   %preuntrans, %postuntrans, %verifyscript, triggers and file triggers.
#
# A scriptlet tag this rpm does not know (older rpm, e.g. %preuntrans
# before rpm 4.20) cannot be read here; it is named in the output, and the
# Fedora install check (check-linux-install.sh rpm), whose rpm knows every
# tag, runs this script on the same package.
#
# Uses only rpm queries on the package file; installs and runs nothing.
set -euo pipefail

rpm_file="${1:?usage: check-rpm-scriptlets.sh <package.rpm> <postinst.sh> <postrm.sh>}"
postinst="${2:?}"
postrm="${3:?}"
fail() {
  echo "::error::$rpm_file: $*" >&2
  exit 1
}
query() { rpm -qp --queryformat "$1" "$rpm_file"; }
known_tags=$(rpm --querytags)
known() { grep -q -x -F "$1" <<< "$known_tags"; }

for pair in "POSTIN:$postinst" "POSTUN:$postrm"; do
  tag="${pair%%:*}" script="${pair#*:}"
  [ "$(query "%|${tag}?{present}:{absent}|")" = present ] || fail "it has no %{$tag} scriptlet"
  # Every element of the interpreter list, one per line; "<absent>" when
  # no interpreter is stored (a stored "(none)" is a program, and refused).
  prog=$(query "%|${tag}PROG?{[%{${tag}PROG}\n]}:{<absent>\n}|")
  case "$prog" in
    "<absent>" | /bin/sh) ;;
    *) fail "its %{$tag} scriptlet runs with '$(tr '\n' ' ' <<< "$prog")', expected /bin/sh (or rpm's default)" ;;
  esac
  flags=$(query "%|${tag}FLAGS?{%{${tag}FLAGS}}:{0}|")
  [ "$flags" = 0 ] || fail "its %{$tag} scriptlet has flags $flags (e.g. macro expansion), expected none"
  [ "$(printf '%s\n' "$(query "%{$tag}")")" = "$(printf '%s\n' "$(cat "$script")")" ] \
    || fail "its %{$tag} scriptlet is not $script"
done

unknown=""
for tag in PREIN PREUN PRETRANS POSTTRANS PREUNTRANS POSTUNTRANS VERIFYSCRIPT \
  TRIGGERSCRIPTS FILETRIGGERSCRIPTS TRANSFILETRIGGERSCRIPTS; do
  if ! known "$tag"; then
    unknown="$unknown $tag"
    continue
  fi
  [ "$(query "%|${tag}?{present}:{absent}|")" = absent ] || fail "it has a %{$tag} scriptlet; only %post and %postun are allowed"
done
echo "$rpm_file: %post and %postun are the repository's scripts (interpreter: $(query "%|POSTINPROG?{[%{POSTINPROG} ]}:{rpm's default /bin/sh}|"), no flags); no other scriptlet${unknown:+ (not known to $(rpm --version), so not checked here:$unknown)}"
