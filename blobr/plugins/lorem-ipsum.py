#!/usr/bin/env python3
"""Blobr plugin: fills the selected text layers with placeholder text.

Copy it into the plugins folder (Plugins > Open Plugins Folder...) and make it executable.
"""

import json
import os
import re
import socket
import sys

LOREM = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua."


def call(conn, name, args):
    """One tool call: a JSON line out, a JSON line back. Returns the text parts."""
    conn.sendall((json.dumps({"name": name, "args": args}) + "\n").encode())
    reply = json.loads(conn.makefile().readline())
    if "err" in reply:
        sys.exit(reply["err"])
    return "".join(p.get("text", "") for p in reply["ok"])


conn = socket.socket(socket.AF_UNIX)
conn.connect(os.environ["BLOBR_SOCKET"])
# Each selected layer is a line like: #12 text "Title" 120x24 at 0,0
texts = [int(m[1]) for m in re.finditer(r"^#(\d+) text ", call(conn, "get_selection", {}), re.M)]
if not texts:
    sys.exit("Select some text layers first")
ops = [{"op": "set", "id": i, "patch": {"text": LOREM}} for i in texts]
call(conn, "apply_ops", {"ops": ops, "label": "Lorem ipsum"})
