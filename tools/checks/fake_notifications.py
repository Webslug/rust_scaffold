#!/usr/bin/env python3
"""Stand-in desktop notification service for hidden-screen checks.

Owns org.freedesktop.Notifications on the bus given (the check's private
bus, never the desktop's) and appends each notification it receives to a
file as one JSON line: app, title, text, urgency, sound. Exits when the bus
goes away.

Usage: fake_notifications.py BUS_ADDRESS OUTPUT_FILE
"""
import json
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

DEBUG_MODE = False

NAME = "org.freedesktop.Notifications"
PATH = "/org/freedesktop/Notifications"
XML = """
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="Notify">
      <arg type="s" direction="in"/><arg type="u" direction="in"/>
      <arg type="s" direction="in"/><arg type="s" direction="in"/>
      <arg type="s" direction="in"/><arg type="as" direction="in"/>
      <arg type="a{sv}" direction="in"/><arg type="i" direction="in"/>
      <arg type="u" direction="out"/>
    </method>
    <method name="GetCapabilities"><arg type="as" direction="out"/></method>
  </interface>
</node>
"""


def debug(text):
    if DEBUG_MODE:
        print("fake_notifications:", text, file=sys.stderr)


def main():
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    address, out_path = sys.argv[1], sys.argv[2]
    flags = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION
    conn = Gio.DBusConnection.new_for_address_sync(address, flags, None, None)
    loop = GLib.MainLoop()
    count = [0]

    def on_call(_conn, _sender, _path, _iface, method, params, invocation):
        if method == "GetCapabilities":
            invocation.return_value(GLib.Variant("(as)", (["body", "sound"],)))
            return
        app, _replaces, _icon, title, text, _actions, hints, _timeout = params.unpack()
        count[0] += 1
        line = {"app": app, "title": title, "text": text, "urgency": hints.get("urgency"), "sound": hints.get("sound-name")}
        with open(out_path, "a", encoding="utf-8") as f:
            f.write(json.dumps(line) + "\n")
        debug(f"received {line}")
        invocation.return_value(GLib.Variant("(u)", (count[0],)))

    info = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object(PATH, info.interfaces[0], on_call, None, None)
    conn.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName",
                   GLib.Variant("(su)", (NAME, 4)), None, Gio.DBusCallFlags.NONE, -1, None)
    conn.connect("closed", lambda *_: loop.quit())
    debug("ready")
    loop.run()
    return 0


if __name__ == "__main__":
    sys.exit(main())
