#!/usr/bin/env bash
# Tests check-rpm-scriptlets.sh on small .rpm packages built with
# rpmbuild: the repository's %post and %postun pass, and each way a package
# could run something else as root is refused (a missing or altered
# scriptlet, another interpreter, interpreter arguments, macro expansion
# flags, any other scriptlet, triggers and file triggers).
#
#   tools/ci/test-rpm-scriptlets.sh            (any host with docker: Fedora container)
#   tools/ci/test-rpm-scriptlets.sh --here     (a host with rpmbuild, as root or not)
#
# Environment: FEDORA_IMAGE (default fedora:42).
# ok and bad never fail, so `check && ok || bad` is an if-then-else here.
# shellcheck disable=SC2015
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"

if [ "${1-}" != --here ]; then
  exec docker run --rm -v "$root:/repo:ro" "${FEDORA_IMAGE:-fedora:42}" bash -euo pipefail -c "
    dnf -y -q install rpm-build > /dev/null 2>&1
    /repo/tools/ci/test-rpm-scriptlets.sh --here
  "
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
postinst="$root/packaging/linux/postinst.sh"
postrm="$root/packaging/linux/postrm.sh"
failures=0
ok() { echo "ok   $*"; }
bad() {
  echo "FAIL $*"
  failures=$((failures + 1))
}

# The spec text of a script: % doubled, so rpmbuild stores it unchanged.
body() { sed 's/%/%%/g' "$1"; }

# build <name> <spec sections>: a noarch package with one file; prints its path.
build() {
  local name="$1" sections="$2" spec="$work/$1.spec"
  {
    printf 'Name: t-%s\nVersion: 1\nRelease: 1\nSummary: t\nLicense: MIT\nBuildArch: noarch\n' "$name"
    printf '%%description\nt\n%%install\nmkdir -p %%{buildroot}/usr/share/t-%s\ntouch %%{buildroot}/usr/share/t-%s/f\n' "$name" "$name"
    printf '%%files\n/usr/share/t-%s/f\n' "$name"
    printf '%s\n' "$sections"
  } > "$spec"
  rpmbuild -bb --define "_topdir $work/top" "$spec" > "$work/$name.log" 2>&1 \
    || { echo "rpmbuild failed for $name:" >&2; tail -n 5 "$work/$name.log" >&2; return 1; }
  find "$work/top/RPMS" -name "t-$name-*.rpm"
}

post="%post
$(body "$postinst")"
postun="%postun
$(body "$postrm")"

# expect <name> <pass|fail> <message part> <sections>
expect() {
  local name="$1" expected="$2" message="$3" sections="$4" package out status=0
  package=$(build "$name" "$sections") || { bad "$name: could not be built"; return; }
  out=$("$here/check-rpm-scriptlets.sh" "$package" "$postinst" "$postrm" 2>&1) || status=$?
  if [ "$expected" = pass ]; then
    [ "$status" -eq 0 ] && ok "$name passes" || bad "$name: refused: $out"
  else
    [ "$status" -ne 0 ] && grep -q -F -- "$message" <<< "$out" && ok "$name is refused ($message)" \
      || bad "$name: status $status, expected a refusal with '$message': $out"
  fi
}

echo "== $(rpm --version)"
expect exact pass "" "$post
$postun"
expect no-post fail "no %{POSTIN} scriptlet" "$postun"
expect no-postun fail "no %{POSTUN} scriptlet" "$post"
expect altered fail "%{POSTIN} scriptlet is not" "$post
echo something else
$postun"
expect bash fail "runs with '/bin/bash" "%post -p /bin/bash
$(body "$postinst")
$postun"
expect arguments fail "runs with '/bin/sh -x" "%post -p \"/bin/sh -x\"
$(body "$postinst")
$postun"
expect postun-arguments fail "%{POSTUN} scriptlet runs with '/bin/sh -e" "$post
%postun -p \"/bin/sh -e\"
$(body "$postrm")"
expect expanded fail "%{POSTIN} scriptlet has flags" "%post -e
$(body "$postinst")
$postun"
for section in pre:PREIN preun:PREUN pretrans:PRETRANS posttrans:POSTTRANS verifyscript:VERIFYSCRIPT \
  preuntrans:PREUNTRANS postuntrans:POSTUNTRANS; do
  name="${section%%:*}" tag="${section#*:}"
  expect "extra-$name" fail "a %{$tag} scriptlet" "$post
$postun
%$name
true"
done
expect trigger fail "a %{TRIGGERSCRIPTS} scriptlet" "$post
$postun
%triggerin -- bash
true"
expect file-trigger fail "a %{FILETRIGGERSCRIPTS} scriptlet" "$post
$postun
%filetriggerin -- /usr/share
true"
expect transfile-trigger fail "a %{TRANSFILETRIGGERSCRIPTS} scriptlet" "$post
$postun
%transfiletriggerin -- /usr/share
true"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "all rpm scriptlet checks passed"
