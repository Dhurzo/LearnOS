#!/usr/bin/env python3
"""Scripted keyboard session against LearnOS in QEMU.

Boots the kernel with a monitor unix socket, types shell commands with the
QEMU monitor `sendkey` (real PS/2 scancodes through the i8042), and asserts
on serial output.

Covers: A1 (scancodes drained from port 0x60 in ring 3), C2 (ls/cat/echo/
clear/run/kill/stop/cont/ps), C1 (sys_exec via `run runme`), C3 (heap buffer
inside `run`), C4 (stop/cont/kill with kernel trace lines).

Usage: ./tools/interact.py  (runs from the repo root)
Exit 0 when every expectation is met, 1 otherwise.
"""

import os
import shutil
import socket
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = os.path.join(REPO, "target")
DISK = os.path.join(TARGET, "disk.img")
KERNEL = os.path.join(TARGET, "pvh-kernel")
HELLO_ELF = os.path.join(TARGET, "hello.elf")
MON_SOCK = os.path.join(TARGET, "mon.sock")
SER_LOG = os.path.join(TARGET, "interact-serial.log")

KEY_DELAY = 0.35  # seconds between keystrokes (TCG + polling servers)
STEP_TIMEOUT = 60  # max seconds waiting for one expectation
BOOT_TIMEOUT = 120  # max seconds waiting for the first shell prompt


def sh(cmd, **kw):
    return subprocess.run(cmd, cwd=REPO, check=True, **kw)


def build_all():
    print("[1/4] building kernel")
    sh(["./build.sh"], stdout=subprocess.DEVNULL)
    print("[2/4] building hello user program")
    tmp = os.path.join(TARGET, "hello-tmp")
    shutil.rmtree(tmp, ignore_errors=True)
    sh(["./userspace/build.sh", "hello", "release", "--out", tmp],
       stdout=subprocess.DEVNULL)
    shutil.copy(os.path.join(tmp, "hello.elf"), HELLO_ELF)
    shutil.rmtree(tmp, ignore_errors=True)
    print("[3/4] creating FAT16 disk image")
    sh(["python3", "tools/make-fat16.py", DISK],
       stdout=subprocess.DEVNULL)


def launch_qemu():
    for f in (MON_SOCK, SER_LOG):
        try:
            os.unlink(f)
        except FileNotFoundError:
            pass
    qemu_args = [
        "qemu-system-x86_64",
        "-kernel", "/work/target/pvh-kernel",
        "-m", "128M",
        "-machine", "pc,accel=tcg",
        "-cpu", "qemu64",
        "-display", "none",
        "-serial", "stdio",
        "-monitor", "unix:/work/target/mon.sock,server,nowait",
        "-hda", "/work/target/disk.img",
    ]
    if shutil.which("qemu-system-x86_64"):
        cmd = ["timeout", "-s", "INT", "400"] + [
            a.replace("/work", REPO) if a.startswith("/work") else a
            for a in qemu_args
        ]
    else:
        uid = os.getuid()
        gid = os.getgid()
        cmd = ["docker", "run", "--rm", "--user", "%d:%d" % (uid, gid),
               "-v", f"{REPO}:/work",
               "learnos-qemu", "sh", "-c",
               "cd /work && timeout -s INT 400 " +
               " ".join(qemu_args)]
    log = open(SER_LOG, "wb")
    print("[4/4] booting QEMU (serial -> %s)" % SER_LOG)
    proc = subprocess.Popen(cmd, cwd=REPO, stdout=log, stderr=subprocess.STDOUT)
    return proc, log


def wait_for(marker, timeout, since=0):
    """Poll the serial log for `marker` (bytes after file offset `since`)."""
    end = time.time() + timeout
    while time.time() < end:
        with open(SER_LOG, "rb") as f:
            f.seek(since)
            if marker in f.read():
                return True
        time.sleep(1.0)
    return False


def log_size():
    return os.path.getsize(SER_LOG)


class Monitor:
    def __init__(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(10)
        self.sock.connect(MON_SOCK)
        self._drain_banner()

    def _drain_banner(self):
        self.sock.settimeout(2)
        try:
            while self.sock.recv(4096):
                pass
        except socket.timeout:
            pass
        finally:
            self.sock.settimeout(10)

    def sendkey(self, key):
        self.sock.sendall(("sendkey %s\n" % key).encode())
        time.sleep(0.15)
        # Drain the (qemu) prompt echo so the socket never blocks.
        self.sock.settimeout(0.3)
        try:
            while self.sock.recv(4096):
                pass
        except socket.timeout:
            pass
        finally:
            self.sock.settimeout(10)

    def close(self):
        try:
            self.sock.sendall(b"quit\n")
        except OSError:
            pass
        self.sock.close()


def type_line(mon, line):
    for ch in line:
        if ch == " ":
            mon.sendkey("spc")
        elif ch == "\n":
            mon.sendkey("ret")
        else:
            mon.sendkey(ch)
        time.sleep(KEY_DELAY)


def main():
    build_all()
    assert os.path.exists(KERNEL), "kernel image missing"
    assert os.path.exists(DISK), "disk image missing"
    proc, log = launch_qemu()
    failures = []
    try:
        print("waiting for shell prompt ...")
        if not wait_for(b"Shell> ", BOOT_TIMEOUT):
            print("BOOT TIMEOUT — no shell prompt")
            return 1
        print("shell is up, connecting monitor ...")
        mon = Monitor()

        steps = [
            # (typed line, expected serial bytes or None)
            ("ps\n", b"self pid="),
            ("echo hello\n", b"\nhello\n"),
            ("ls\n", b"README"),
            ("cat readme\n", b"LearnOS FAT16"),
            ("clear\n", None),
            ("stop 4\n", b"signal stop"),
            ("cont 4\n", b"cont sent"),
            ("kill 4\n", b"signal kill"),
            ("run runme\n", b"Hello, LearnOS user space!"),
        ]
        for line, expect in steps:
            mark = log_size()
            print("typing %r ..." % line.strip())
            type_line(mon, line)
            if expect is None:
                time.sleep(3)
                print("  sent (no assertion)")
                continue
            if wait_for(expect, STEP_TIMEOUT, since=mark):
                print("  PASS: %r" % expect.decode())
            else:
                print("  FAIL: %r" % expect.decode())
                failures.append((line.strip(), expect.decode()))
        mon.close()
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            proc.kill()
        log.close()

    if failures:
        print("INTERACT: FAIL (%d)" % len(failures))
        for line, expect in failures:
            print("  %r missing %r" % (line, expect))
        return 1
    print("INTERACT: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
