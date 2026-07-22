#!/bin/sh
# %post. $1 is the number of versions of this package that will be installed
# once the operation finishes: 1 on a fresh install, 2 during an upgrade.
set -e

# The service account and its input/video groups. Shipped as a sysusers.d
# fragment so systemd owns the definition; this applies it now instead of
# waiting for the next boot.
systemd-sysusers /usr/lib/sysusers.d/alertu.conf >/dev/null 2>&1 || true

# No config file is shipped: anything under /etc would be a packaged config
# file, and the daemon rewrites this one whenever the tray or the CLI saves
# settings. It creates the file itself on first use; we only provide a
# directory it is allowed to write to.
mkdir -p /etc/alertu
chown alertu:alertu /etc/alertu 2>/dev/null || true
chmod 0755 /etc/alertu

systemctl daemon-reload >/dev/null 2>&1 || true

if [ "$1" -eq 1 ]; then
    # preset, not enable: it honours the administrator's preset policy, which
    # is the Fedora convention. Fedora's default policy ends with `disable *`,
    # so this leaves the daemon disabled and the user enables it -- see the
    # message below and README.Fedora.
    systemctl preset alertu-daemon.service >/dev/null 2>&1 || true
fi

if [ -x /usr/bin/gtk-update-icon-cache ]; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true
fi

if [ "$1" -eq 1 ]; then
    cat <<'EOF'

AlertU has been installed.

The daemon is not running yet. Fedora applies preset policy rather than
enabling third-party units, so start it yourself:

    sudo systemctl enable --now alertu-daemon

Its control socket is mode 0660, owned by the `alertu` group. Connecting to it
grants full command of the alarm, so membership is a privilege grant -- which is
also why this package cannot grant it for you. Add yourself:

    sudo usermod -aG alertu <user>

then log out and back in. Until you do, the tray, the settings window and
alertu-ctl will not be able to reach the daemon.

The tray is shipped but not started: it is a user service, and a package must
not touch your systemd user instance. Once logged back in, run:

    systemctl --user enable --now alertu-gui

No remote is configured yet. Run `alertu-ctl list-devices`, then set
`remote_name_hint` to part of your remote's name. See
/usr/share/doc/alertu/config.example.toml.

EOF
fi
exit 0
