#!/bin/bash
# Prepare the files that cannot be packaged as they are committed.
#
# Two of them:
#   * the systemd units hardcode /usr/local/bin, correct for a manual install
#     under the FHS and wrong in a package, which installs to /usr/bin;
#   * the sounds are generated, not committed.
#
# The system unit and the user unit go to *separate* directories on purpose.
# cargo-deb scans one directory for systemd units and handles system units
# only; a user unit found there would be installed and enabled system-wide.
# cargo-generate-rpm does not scan at all — it takes an explicit asset list —
# but the split costs nothing and keeps one staging layout for both formats.
set -euo pipefail

root="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
stage="$root/target/pkg-staging"

rm -rf "$stage"
mkdir -p "$stage/system" "$stage/user" "$stage/sounds"

sed 's|/usr/local/bin|/usr/bin|g' \
    "$root/packaging/alertu-daemon.service" > "$stage/system/alertu-daemon.service"
sed 's|/usr/local/bin|/usr/bin|g' \
    "$root/packaging/alertu-gui.service" > "$stage/user/alertu-gui.service"

# Generated rather than committed, so dpkg owns them and removal takes them away.
"$root/target/release/alertu-ctl" gen-sounds --dir "$stage/sounds" --force

grep -q /usr/local/bin "$stage"/system/*.service "$stage"/user/*.service &&
  { echo "stage.sh: a unit still points at /usr/local/bin" >&2; exit 1; }

echo "staged into $stage"
