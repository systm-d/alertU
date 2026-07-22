#!/bin/sh
# %preun. $1 is the number of versions that will remain after this operation:
# 0 on a real removal, 1 while an upgrade replaces this version.
#
# Guarding on 0 is the whole point. Stopping unconditionally -- which is what
# the Debian prerm does, because dpkg restarts the service afterwards -- would
# take the daemon down on every upgrade and never bring it back.
set -e

if [ "$1" -eq 0 ]; then
    systemctl --no-reload disable --now alertu-daemon.service >/dev/null 2>&1 || true
fi
exit 0
