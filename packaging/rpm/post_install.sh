#!/bin/sh
# %post. $1 is the number of versions of this package that will be installed
# once the operation finishes: 1 on a fresh install, 2 during an upgrade.
set -e

# The service account and its input/video/audio groups. Shipped as a sysusers.d
# fragment so systemd owns the definition; this applies it now instead of
# waiting for the next boot.
#
# The Debian postinst wraps the same call in `[ -x /usr/bin/systemd-sysusers ]`.
# That guard is deliberately absent here, for two reasons: this package carries
# `Requires: /usr/bin/loginctl`, so RPM will not let it install without the
# systemd package that also owns /usr/bin/systemd-sysusers; and the trailing
# `|| true` already absorbs the 127 a missing binary would produce, so the
# guard would change nothing even if the requirement were dropped.
systemd-sysusers /usr/lib/sysusers.d/alertu.conf >/dev/null 2>&1 || true

# No config file is shipped: anything under /etc would be a packaged config
# file, and the daemon rewrites this one whenever the tray or the CLI saves
# settings. It creates the file itself on first use; we only provide a
# directory it is allowed to write to.
#
# RPM downgrades a failing %post to a warning and commits the transaction
# anyway, so nothing here may abort the script: an unguarded failure on any of
# these three lines would skip `systemctl preset` and the whole first-use
# message below, and the install would still be reported as successful. The
# chown is the one that matters -- if it fails, /etc/alertu stays root:root,
# the daemon starts fine, and the user only finds out when a setting saved
# from the tray, the settings window or `alertu-ctl set-config` vanishes on
# restart. So say so, loudly, rather than hiding it behind `2>/dev/null`.
mkdir -p /etc/alertu || :
if ! chown alertu:alertu /etc/alertu; then
    echo "alertu: could not set ownership of /etc/alertu; the daemon will not be able to save its configuration" >&2
fi
chmod 0755 /etc/alertu || :

systemctl daemon-reload >/dev/null 2>&1 || true

if [ "$1" -eq 1 ]; then
    # preset, not enable: it honours the administrator's preset policy, which
    # is the Fedora convention. Fedora's default policy ends with `disable *`,
    # so this leaves the daemon disabled and the user enables it -- see the
    # message below and README.Fedora.
    systemctl preset alertu-daemon.service >/dev/null 2>&1 || true

    # Same convention for the tray, one scope out: `--global` covers systemd user
    # instances started after this point, so it needs no access to the user
    # instance of whoever is logged in. Under Fedora's default policy this leaves
    # it disabled, like the daemon — the message below says how to turn both on.
    systemctl --global preset alertu-gui.service >/dev/null 2>&1 || true
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

The tray follows the same preset policy, so turn it on for your account:

    systemctl --user enable --now alertu-gui

The rest needs no terminal:

  1. Open "AlertU -- Settings" from your applications menu. If it says this
     account may not control AlertU, click "Authorize this account" -- one
     password prompt. (The control socket is 0660 and group-owned, and no
     package may add an existing account to a group, so this is the one step
     that has to be a deliberate act. Log out and back in afterwards.)

  2. Click "Pair remote" in the tray menu or the settings window, then press a
     button on your remote. AlertU works out both the device and the key by
     itself -- a key the remote cannot send is the classic reason a paired
     remote does nothing.

  3. If you want the siren audible, set the ALSA device in the settings window.
     The daemon has no user session of its own, so it cannot reach PipeWire or
     PulseAudio; `aplay -L` lists the devices, and the built-in speaker is the
     one that protects anything.

See /usr/share/doc/alertu/config.example.toml for every setting.

EOF
fi
exit 0
