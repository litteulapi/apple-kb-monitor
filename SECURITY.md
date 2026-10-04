# Security policy

## Supported versions

Only the latest release (the newest `apple-kb-monitor` package on the
[Releases page](https://github.com/litteulapi/apple-kb-monitor/releases)) gets
security fixes.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through GitHub:
*Security* tab → *Report a vulnerability*
([direct link](https://github.com/litteulapi/apple-kb-monitor/security/advisories/new)).

Include the version (`pacman -Q apple-kb-monitor`), what an attacker can do,
and the steps to reproduce. You will get an answer within 7 days; a fix is
released and the advisory published once users can update, crediting you
unless you prefer otherwise.

## Scope

In scope: the daemon `apple-kb-monitord`, `akmctl`, the privileged helpers
(`akm-helper`, `rssi-helper`), the polkit policy and rules, the udev rule, the
systemd units, the D-Bus interface `com.agenceapi.AppleKbMonitor1`, the Plasma
widget and the System Settings module.

Known and documented trade-offs (not vulnerabilities by themselves): the
`uaccess` udev rule gives processes of the active session read and write
access (logind rw ACL) to the keyboard's hidraw node: keystrokes are readable,
output and feature reports writable, and a descriptor opened before a fast
user switch stays usable (see [udev/README.md](udev/README.md)).
