#!/bin/sh
# %postun. $1 is the number of versions remaining: 0 after a real removal,
# 1 or more after an upgrade.
set -e

systemctl daemon-reload >/dev/null 2>&1 || true

if [ "$1" -ge 1 ]; then
    # Upgrade: pick up the new binary. try-restart is a no-op when the unit is
    # not running, which is the common case here since Fedora leaves it
    # disabled unless the user enabled it.
    systemctl try-restart alertu-daemon.service >/dev/null 2>&1 || true
fi

if [ -x /usr/bin/gtk-update-icon-cache ]; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi

# /etc/alertu and /var/lib/alertu are deliberately left alone: the first holds
# the user's configuration, the second their captured snapshots. Neither is in
# the package manifest, so RPM does not touch them either.
exit 0
