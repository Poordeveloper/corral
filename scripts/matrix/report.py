#!/usr/bin/env python3
"""Summarize one capture: events and marks on one timeline, OSC titles over time,
redraw-after-turn-end windows, and the screen at chosen marks."""
import json, os, re, struct, sys

def frames(path):
    raw = open(path, "rb").read(); i = 0; out = []
    while i + 12 <= len(raw):
        t, n = struct.unpack("<QI", raw[i:i+12]); i += 12; out.append((t, raw[i:i+n])); i += n
    return out

def events(d):
    evs = []
    for file, kind in (("hooks.jsonl", "hook"), ("notify.jsonl", "notify")):
        p = os.path.join(d, file)
        if not os.path.exists(p): continue
        for l in open(p):
            r = json.loads(l)
            if kind == "hook":
                try: payload = json.loads(r["stdin"])
                except Exception: payload = {}
                name = payload.get("hook_event_name") or "?"
                detail = {k: payload.get(k) for k in ("notification_type", "message", "reason", "source", "tool_name", "trigger", "stop_hook_active") if payload.get(k) is not None}
            else:
                try: payload = json.loads(r["argv"][-1])
                except Exception: payload = {}
                name = payload.get("type") or "?"
                detail = {k: payload.get(k) for k in ("thread-id", "turn-id", "client", "last-assistant-message") if payload.get(k) is not None}
            evs.append({"t_ns": r["t_ns"], "kind": kind, "name": name, "detail": detail, "payload": payload})
    return sorted(evs, key=lambda e: e["t_ns"])

def main(d):
    meta = json.load(open(os.path.join(d, "meta.json"))); t0 = meta["t0_ns"]
    marks = [json.loads(l) for l in open(os.path.join(d, "marks.jsonl"))]
    marks = [m for m in marks if isinstance(m.get("t_ns"), int)]
    evs = events(d); fr = frames(os.path.join(d, "stream.bin"))
    print("## %s\nargv: %s  geometry %dx%d" % (os.path.basename(d), " ".join(meta["argv"]), meta["cols"], meta["rows"]))
    print("\n### timeline (s from start)")
    timeline = [(m["t_ns"], "mark", m["name"], {k: v for k, v in m.items() if k not in ("name", "t_ns", "offset")}) for m in marks]
    timeline += [(e["t_ns"], e["kind"], e["name"], e["detail"]) for e in evs]
    for t, kind, name, detail in sorted(timeline, key=lambda x: x[0]):
        print("%9.3f  %-6s %-28s %s" % ((t - t0) / 1e9, kind, name, json.dumps(detail, ensure_ascii=False)[:110] if detail else ""))
    # OSC titles over time
    print("\n### OSC titles (first seen)")
    seen = set(); buf = b""
    for t, b in fr:
        buf += b
        for m in re.finditer(rb"\x1b\]0;([^\x07\x1b]*)(\x07|\x1b\\)", buf):
            title = m.group(1).decode("utf-8", "replace")
            if title not in seen:
                seen.add(title); print("%9.3f  %s" % ((t - t0) / 1e9, title))
        buf = buf[-4096:]
    # other OSC / notification-ish sequences
    all_bytes = b"".join(b for _, b in fr)
    oscs = set(m.group(1)[:60] for m in re.finditer(rb"\x1b\]([0-9]+;[^\x07\x1b]*)", all_bytes))
    kinds = sorted(set(o.split(b";")[0].decode() for o in oscs))
    print("\n### OSC kinds present:", kinds)
    for o in sorted(oscs):
        if not o.startswith(b"0;"): print("   ", o.decode("utf-8", "replace"))
    # redraw windows after each turn-end-ish event
    print("\n### output activity after events (ms until output stops for >=2s)")
    ts = [t for t, _ in fr]
    for e in evs:
        if e["name"] in ("Stop", "agent-turn-complete", "Notification", "PostToolUse", "SubagentStop", "SessionEnd"):
            after = [t for t in ts if t >= e["t_ns"]]
            last = e["t_ns"]
            for t in after:
                if t - last > 2_000_000_000: break
                last = t
            print("  %-20s last output %6.0f ms after" % (e["name"], (last - e["t_ns"]) / 1e6))

if __name__ == "__main__":
    for d in sys.argv[1:]: main(d); print()
