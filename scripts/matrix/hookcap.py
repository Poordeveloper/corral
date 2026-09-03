#!/usr/bin/env python3
"""Append one hook/notify invocation to a JSONL log: wall clock ns, argv, verbatim stdin."""
import json, os, sys, time
out = sys.argv[1]
t = time.time_ns()
data = b""
if not sys.stdin.isatty():
    try:
        data = sys.stdin.buffer.read()
    except Exception:
        data = b""
rec = {"t_ns": t, "pid": os.getpid(), "ppid": os.getppid(), "argv": sys.argv[2:], "stdin": data.decode("utf-8", "replace")}
with open(out, "a") as f:
    f.write(json.dumps(rec) + "\n")
