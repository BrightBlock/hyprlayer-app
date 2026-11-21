#!/usr/bin/env python3
"""Raw PTY bytes of a real claude's picker: `capture.py <claude> <cols>x<rows> model|effort|session <out.bin>`.
A throwaway, logged-out HOME and CLAUDE_CONFIG_DIR; no Enter or digit reaches a picker, since
claude 2.1.330's Enter saves the default model."""
import json, os, pty, select, shutil, signal, struct, sys, tempfile, termios, fcntl, time

binary, size, kind, out = sys.argv[1:5]
cols, rows = (int(n) for n in size.split("x"))
sandbox = os.path.realpath(tempfile.mkdtemp(prefix="claude-tui-"))
home, config = os.path.join(sandbox, "home"), os.path.join(sandbox, "config")
work = os.path.join(home, "demo")
os.makedirs(work)
os.makedirs(config)
with open(os.path.join(config, ".claude.json"), "w") as f:
    json.dump({"hasCompletedOnboarding": True, "theme": "dark", "numStartups": 3,
               "projects": {work: {"hasTrustDialogAccepted": True}}}, f)
# Else claude registers the sandbox's URL handler app with LaunchServices, which is per user.
with open(os.path.join(config, "settings.json"), "w") as f:
    json.dump({"disableDeepLinkRegistration": "disable"}, f)
env = {"HOME": home, "CLAUDE_CONFIG_DIR": config, "PATH": "/usr/bin:/bin", "TERM": "xterm-256color",
       "LANG": "en_US.UTF-8", "DISABLE_AUTOUPDATER": "1", "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1"}

pid, fd = pty.fork()
if pid == 0:
    os.chdir(work)
    os.execve(binary, [binary], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
raw = bytearray()


def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        if select.select([fd], [], [], 0.05)[0]:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            raw.extend(chunk)
            if b"\x1b[6n" in chunk:
                os.write(fd, b"\x1b[1;1R")


def stop():
    # claude writes its transcript as it exits, and blocks on a PTY nobody reads.
    os.kill(pid, signal.SIGTERM)
    end = time.time() + 5
    while os.waitpid(pid, os.WNOHANG) == (0, 0):
        if time.time() > end:
            os.kill(pid, signal.SIGKILL)
        pump(0.1)
    shutil.rmtree(sandbox)


def send(keys, settle):
    os.write(fd, keys)
    pump(settle)


pump(8)
# The Enter below must land on the input box, not on a login or trust dialog.
if b"shift+tab to cycle" not in raw:
    stop()
    sys.exit("claude isn't showing its input box; nothing was typed")
for ch in "/effort" if kind == "effort" else "/model":
    send(ch.encode(), 0.08)
pump(0.8)
send(b"\r", 2.5)
if kind != "effort":
    send(b"\x1b[B", 0.8)
if kind == "session":
    send(b"s", 2.5)
cut = len(raw)
if kind != "session":
    send(b"\x1b", 1.5)
stop()
with open(out, "wb") as f:
    f.write(raw[:cut])
print(f"{out}: {cut} bytes")
