#!/usr/bin/env python3
"""PR8 matrix driver: run a provider under a real PTY, record the raw byte stream with
timestamps, drive it with a scripted keyboard, and mark checkpoints. Screens are rendered
later by replaying the stream through Corral's own emulator; here decisions use regexes
over the raw bytes and the hook/notify capture files."""
import fcntl, json, os, pty, re, select, signal, struct, subprocess, sys, termios, time

KEYS = {"Enter": "\r", "Esc": "\x1b", "Tab": "\t", "Up": "\x1b[A", "Down": "\x1b[B",
        "CtrlC": "\x03", "CtrlD": "\x04", "CtrlU": "\x15", "Backspace": "\x7f", "Space": " "}

class Run:
    def __init__(self, outdir, argv, cwd, env, rows, cols, hooks_file=None):
        self.outdir = outdir; os.makedirs(outdir, exist_ok=True)
        self.argv, self.cwd, self.env, self.rows, self.cols = argv, cwd, env, rows, cols
        self.hooks_file = hooks_file
        self.stream = open(os.path.join(outdir, "stream.bin"), "wb")
        self.input = open(os.path.join(outdir, "input.bin"), "wb")
        self.marks = open(os.path.join(outdir, "marks.jsonl"), "a")
        self.log = open(os.path.join(outdir, "driver.log"), "a")
        self.buf = bytearray(); self.last_output_ns = None; self.t0 = time.time_ns()
        self.hook_lines_seen = 0; self.reaped = False
        meta = {"argv": argv, "cwd": cwd, "rows": rows, "cols": cols, "t0_ns": self.t0,
                "env": {k: env[k] for k in sorted(env)}}
        with open(os.path.join(outdir, "meta.json"), "w") as f: json.dump(meta, f, indent=1)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(cwd)
            os.execvpe(argv[0], argv, env)
        self.pid, self.fd = pid, fd
        self.resize(rows, cols)
        self.note("started pid=%d" % pid)

    def now_ns(self): return time.time_ns()
    def note(self, s):
        self.log.write("%d %s\n" % (self.now_ns() - self.t0, s)); self.log.flush()
    def mark(self, name, **extra):
        rec = dict(extra); rec.update({"name": name, "t_ns": self.now_ns(), "offset": len(self.buf)})
        self.marks.write(json.dumps(rec) + "\n"); self.marks.flush()
        self.note("mark %s" % name)
    def resize(self, rows, cols):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        self.rows, self.cols = rows, cols
        self.mark("resize", rows=rows, cols=cols)
    def _pump(self, timeout):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if not r: return False
        try: data = os.read(self.fd, 65536)
        except OSError: return None
        if not data: return None
        t = self.now_ns(); self.last_output_ns = t
        self.stream.write(struct.pack("<QI", t, len(data)) + data); self.stream.flush()
        self.buf.extend(data); return True
    def send(self, text, per_char_delay=0.0):
        data = text.encode("utf-8")
        if per_char_delay:
            for b in data:
                self._write(bytes([b])); self.sleep(per_char_delay)
        else:
            self._write(data)
    def _write(self, data):
        t = self.now_ns()
        self.input.write(struct.pack("<QI", t, len(data)) + data); self.input.flush()
        os.write(self.fd, data)
    def key(self, name): self.send(KEYS[name])
    def submit(self, text, pause=0.3, per_char=0.025):
        # Typed, not pasted: a burst of characters reads as a paste to Codex's composer
        # (and to Claude's for long prompts), and an Enter riding that burst inserts a
        # newline instead of submitting.
        self.send(text, per_char_delay=per_char); self.sleep(pause); self.key("Enter")
    def paste(self, text): self.send("\x1b[200~" + text + "\x1b[201~")
    def sleep(self, seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if self._pump(min(0.05, max(0.0, end - time.monotonic()))) is None: return
    def wait_for(self, pattern, timeout=60, since=None):
        rx = re.compile(pattern.encode("utf-8") if isinstance(pattern, str) else pattern)
        start = 0 if since is None else since
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            m = rx.search(bytes(self.buf[start:]))
            if m:
                self.note("wait_for %r matched at +%d" % (pattern, start + m.start()))
                return start + m.start()
            if self._pump(0.1) is None: break
        self.note("wait_for %r TIMEOUT" % pattern); self.mark("timeout", pattern=str(pattern))
        return None
    def hook_events(self):
        if not self.hooks_file or not os.path.exists(self.hooks_file): return []
        with open(self.hooks_file) as f: lines = f.read().splitlines()
        out = []
        for ln in lines:
            try: out.append(json.loads(ln))
            except Exception: pass
        return out
    def wait_hook(self, predicate, timeout=60):
        """predicate(record) -> bool over records not yet seen."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            evs = self.hook_events()
            for i in range(self.hook_lines_seen, len(evs)):
                if predicate(evs[i]):
                    self.hook_lines_seen = i + 1
                    self.note("wait_hook matched record %d" % i); return evs[i]
            self.hook_lines_seen = max(self.hook_lines_seen, 0)
            if self._pump(0.1) is None: break
        self.note("wait_hook TIMEOUT"); self.mark("hook_timeout"); return None
    def quiet(self, quiet_s, timeout=120):
        """Wait until no output for quiet_s; record when the last byte arrived."""
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            r = self._pump(quiet_s)
            if r is None: break
            if r is False:
                self.mark("quiet", quiet_s=quiet_s, last_output_ns=self.last_output_ns); return True
        self.mark("quiet_timeout"); return False
    def alive(self):
        if self.reaped: return False
        try: pid, _ = os.waitpid(self.pid, os.WNOHANG)
        except ChildProcessError: self.reaped = True; return False
        if pid == self.pid: self.reaped = True; return False
        return True
    def end(self, grace=10):
        deadline = time.monotonic() + grace
        while time.monotonic() < deadline:
            if self._pump(0.2) is None: break
            if not self.alive(): break
        try:
            if self.alive(): os.kill(self.pid, signal.SIGHUP); self.sleep(1)
            if self.alive(): os.kill(self.pid, signal.SIGKILL); self.sleep(0.5); self.alive()
        except ProcessLookupError: pass
        self.mark("end"); self.stream.close(); self.input.close(); self.marks.close(); self.log.close()

def hook_event_name(rec):
    try: return json.loads(rec["stdin"]).get("hook_event_name")
    except Exception: return None
def hook_is(name):
    return lambda rec: hook_event_name(rec) == name

def base_env():
    # The updater is off because a provider that replaces itself mid-run makes
    # the capture's version a guess: the first matrix run recorded Claude Code
    # 2.1.258 and the updater had removed that binary by the next day.
    return {"PATH": "/root/.local/bin:/root/.cargo/bin:/usr/local/bin:/usr/bin:/bin",
            "HOME": "/root", "TERM": "xterm-256color", "LANG": "C.UTF-8", "USER": "root",
            "SHELL": "/bin/bash", "COLORTERM": "truecolor",
            "DISABLE_AUTOUPDATER": "1"}

CLAUDE_HOOK_EVENTS = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse",
                      "Notification", "Stop", "SubagentStop", "PreCompact", "SessionEnd",
                      "PermissionRequest"]

def claude_settings(outdir, events=CLAUDE_HOOK_EVENTS):
    os.makedirs(outdir, exist_ok=True)
    hooks_file = os.path.join(outdir, "hooks.jsonl")
    cmd = "python3 /matrix/hookcap.py %s" % hooks_file
    settings = {"hooks": {ev: [{"hooks": [{"type": "command", "command": cmd}]}] for ev in events}}
    path = os.path.join(outdir, "settings.json")
    with open(path, "w") as f: json.dump(settings, f, indent=1)
    return path, hooks_file

def claude(outdir, extra_args=(), cwd="/root/proj", rows=40, cols=120, events=CLAUDE_HOOK_EVENTS):
    settings, hooks_file = claude_settings(outdir, events)
    argv = ["claude", "--model", "haiku", "--settings", settings] + list(extra_args)
    return Run(outdir, argv, cwd, base_env(), rows, cols, hooks_file)

def codex(outdir, extra_args=(), cwd="/root/proj", rows=40, cols=120):
    os.makedirs(outdir, exist_ok=True)
    notify_file = os.path.join(outdir, "notify.jsonl")
    argv = ["codex", "-c", 'notify=["python3","/matrix/hookcap.py","%s"]' % notify_file] + list(extra_args)
    return Run(outdir, argv, cwd, base_env(), rows, cols, notify_file)

def codex_notify_type(rec):
    try: return json.loads(rec["argv"][-1]).get("type")
    except Exception: return None
def notify_is(t): return lambda rec: codex_notify_type(rec) == t

# ---------------------------------------------------------------- scenarios
S = {}
def scenario(f): S[f.__name__] = f; return f

READY = "❯|manual mode on|auto mode on"

@scenario
def c01_startup_idle_ready(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready"); r.quiet(2); r.mark("idle_prompt")
    r.submit("Reply with exactly: ok"); r.mark("submitted")
    r.wait_hook(hook_is("UserPromptSubmit"), 30)
    stop = r.wait_hook(hook_is("Stop"), 120); r.mark("stop_hook", stop_t_ns=stop and stop["t_ns"])
    r.quiet(3, 60); r.mark("ready_after_stop")
    n = r.wait_hook(hook_is("Notification"), 120); r.mark("notification", rec=n and n["stdin"])
    r.quiet(2, 30); r.mark("idle_after_notification")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c02_permission_approve(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Create a file named probe.txt in this directory containing the word hello, using a shell command."); r.mark("submitted")
    pre = r.wait_hook(hook_is("PreToolUse"), 120)
    r.wait_hook(hook_is("PermissionRequest"), 120); r.sleep(1.5); r.mark("permission_prompt_visible")
    r.quiet(2, 30); r.mark("permission_prompt_settled")
    r.key("Enter"); r.mark("approved")
    r.wait_hook(hook_is("PostToolUse"), 60); r.sleep(0.5); r.mark("after_post_tool_use")
    r.wait_hook(hook_is("Stop"), 120); r.mark("stop_hook"); r.quiet(3, 60); r.mark("ready_after_stop")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c03_permission_reject(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Create a file named probe.txt in this directory containing the word hello, using a shell command."); r.mark("submitted")
    r.wait_hook(hook_is("PreToolUse"), 120)
    r.wait_hook(hook_is("PermissionRequest"), 120); r.sleep(1.5); r.mark("permission_prompt_visible"); r.quiet(2, 30)
    r.key("Esc"); r.mark("rejected_esc"); r.sleep(2); r.mark("after_reject")
    r.wait_hook(hook_is("Stop"), 60); r.mark("stop_hook"); r.quiet(3, 60); r.mark("ready_after_reject")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c04_ask_user_question(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Use the AskUserQuestion tool to ask me whether I prefer red or blue, offering exactly those two options. Then just say which I chose."); r.mark("submitted")
    r.wait_hook(hook_is("PreToolUse"), 120)
    r.wait_for(r"(?i)red", 60); r.sleep(2); r.mark("question_prompt_visible"); r.quiet(2, 30); r.mark("question_settled")
    perm = r.wait_hook(hook_is("PermissionRequest"), 3); r.mark("permission_request_hook", present=perm is not None)
    r.key("Enter"); r.mark("answered"); r.sleep(1); r.mark("after_answer")
    r.wait_hook(hook_is("Stop"), 120); r.mark("stop_hook"); r.quiet(3, 60); r.mark("ready_after_stop")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c05_plan_mode_approval(out):
    r = claude(out, extra_args=["--permission-mode", "plan"])
    r.wait_for(READY, 90); r.mark("prompt_ready_plan_mode"); r.quiet(2, 30); r.mark("plan_mode_idle")
    r.submit("Write a two-line plan to add one line to README.md, then call ExitPlanMode."); r.mark("submitted")
    r.wait_hook(hook_is("PreToolUse"), 180)
    perm = r.wait_hook(hook_is("PermissionRequest"), 60); r.mark("plan_permission_request_hook", present=perm is not None); r.sleep(2); r.mark("plan_approval_visible"); r.quiet(2, 60); r.mark("plan_approval_settled")
    perm = r.wait_hook(hook_is("PermissionRequest"), 3); r.mark("permission_request_hook", present=perm is not None)
    r.key("Esc"); r.mark("plan_rejected_esc"); r.sleep(2)
    r.wait_hook(hook_is("Stop"), 120); r.mark("stop_hook"); r.quiet(3, 60); r.mark("ready_after")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c06_thinking_working(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Think step by step, briefly, about what 17 times 23 is, then answer with the number only."); r.mark("submitted")
    r.wait_hook(hook_is("UserPromptSubmit"), 30)
    for i in range(6):
        r.sleep(0.5); r.mark("working_%d" % i, last_output_ns=r.last_output_ns)
    r.wait_hook(hook_is("Stop"), 120); r.mark("stop_hook"); r.quiet(3, 60); r.mark("ready_after_stop")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c07_silent_long_tool(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Run this exact shell command: sleep 8"); r.mark("submitted")
    r.wait_hook(hook_is("PreToolUse"), 120); r.wait_hook(hook_is("PermissionRequest"), 120); r.sleep(1.5); r.mark("permission_prompt_visible")
    r.key("Enter"); r.mark("approved")
    for i in range(8):
        r.sleep(1.0); r.mark("tool_running_%d" % i, last_output_ns=r.last_output_ns)
    r.wait_hook(hook_is("PostToolUse"), 60); r.mark("post_tool_use")
    r.wait_hook(hook_is("Stop"), 120); r.mark("stop_hook"); r.quiet(3, 60); r.mark("ready_after_stop")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c08_compact_resume_help_resize_typing_paste(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Reply with exactly: ok"); r.wait_hook(hook_is("Stop"), 120); r.quiet(3, 60); r.mark("ready")
    r.submit("/compact"); r.mark("compact_submitted")
    r.wait_hook(hook_is("PreCompact"), 60); r.mark("pre_compact_hook"); r.sleep(1); r.mark("compacting")
    r.quiet(3, 120); r.mark("after_compact")
    r.submit("/resume"); r.sleep(2); r.mark("resume_picker"); r.quiet(1, 10); r.mark("resume_picker_settled")
    r.key("Esc"); r.sleep(1); r.quiet(1, 10); r.mark("after_picker_esc")
    r.send("?"); r.sleep(1); r.quiet(1, 10); r.mark("help_overlay")
    r.key("Esc"); r.sleep(1); r.quiet(1, 10); r.mark("after_help_esc")
    r.resize(30, 100); r.sleep(1); r.quiet(1, 10); r.mark("after_resize_small")
    r.resize(40, 120); r.sleep(1); r.quiet(1, 10); r.mark("after_resize_back")
    r.send("hello there this is a person typing at the prompt", per_char_delay=0.06); r.mark("typed_no_enter"); r.quiet(1, 10); r.mark("typed_settled")
    r.key("CtrlU"); r.sleep(0.5); r.quiet(1, 10); r.mark("cleared")
    r.paste("pasted line\n" * 60); r.mark("pasted"); r.quiet(1, 10); r.mark("paste_settled")
    r.key("CtrlU"); r.sleep(0.5); r.quiet(1, 10); r.mark("cleared_2")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c09_permission_like_output(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Run this exact shell command and show me its full output verbatim: printf 'Do you want to proceed?\\n1. Yes\\n2. No\\nAllow Bash(ls)?\\n'"); r.mark("submitted")
    r.wait_hook(hook_is("PreToolUse"), 120); r.wait_hook(hook_is("PermissionRequest"), 120); r.sleep(1.5); r.mark("real_permission_prompt")
    r.key("Enter"); r.mark("approved")
    r.wait_hook(hook_is("Stop"), 120); r.mark("stop_hook"); r.quiet(3, 60); r.mark("ready_with_permission_like_output")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c10_subagent_and_background(out):
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Use a subagent (the Task tool) to count the files in the current directory and report the number."); r.mark("submitted_subagent")
    r.wait_hook(hook_is("Stop"), 240); r.mark("stop_hook_1"); r.quiet(3, 60); r.mark("ready_1")
    r.sleep(5); r.mark("five_s_after_ready_1")
    r.submit("Run this exact shell command in the background: sleep 15. Tell me once it is started, do not wait for it."); r.mark("submitted_background")
    r.wait_hook(hook_is("PreToolUse"), 120); r.wait_hook(hook_is("PermissionRequest"), 120); r.sleep(1.5); r.mark("permission_prompt"); r.key("Enter")
    r.wait_hook(hook_is("Stop"), 120); r.mark("stop_hook_2"); r.quiet(3, 60); r.mark("ready_2")
    r.sleep(20); r.mark("twenty_s_after_ready_2")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c11_trust_dialog_fresh_dir(out):
    os.makedirs("/root/proj-fresh-pr8", exist_ok=True)
    r = claude(out, cwd="/root/proj-fresh-pr8")
    r.wait_for(r"(?i)trust", 60); r.sleep(1.5); r.mark("trust_dialog_visible"); r.quiet(2, 30); r.mark("trust_dialog_settled")
    r.key("Enter"); r.mark("trusted")
    r.wait_for(READY, 90); r.quiet(2, 30); r.mark("prompt_ready")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c12_external_no_settings(out):
    """Global hooks only (Corral's PR7 entries): does anything reach the relay? Diagnostic; no capture file."""
    argv = ["claude", "--model", "haiku"]
    r = Run(out, argv, "/root/proj", base_env(), 40, 120)
    r.wait_for(READY, 90); r.mark("prompt_ready"); r.submit("Reply with exactly: ok")
    r.wait_for(r"\bok\b", 120); r.quiet(3, 60); r.mark("ready"); r.submit("/exit"); r.end()

CODEX_READY = r"gpt-\S+ (low|medium|high|minimal|xhigh)"  # the header says loading until the model resolves; the effort label appears only then

@scenario
def x01_startup_idle_ready(out):
    r = codex(out)
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    r.submit("Reply with exactly: ok"); r.mark("submitted")
    for i in range(4):
        r.sleep(0.5); r.mark("working_%d" % i, last_output_ns=r.last_output_ns)
    n = r.wait_hook(notify_is("agent-turn-complete"), 180); r.mark("notify_turn_complete", t_ns=n and n["t_ns"])
    r.quiet(3, 60); r.mark("ready_after_turn")
    r.sleep(65); r.mark("idle_65s")
    r.submit("/quit"); r.sleep(2); r.end()

@scenario
def x02_approval_approve(out):
    r = codex(out, extra_args=["-a", "on-request", "-s", "read-only"])
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    r.submit("Create a file named probe.txt in this directory containing the word hello, using a shell command."); r.mark("submitted")
    r.wait_for(r"(?i)approv|allow|permission|Yes", 240); r.sleep(2); r.mark("approval_prompt_visible"); r.quiet(2, 30); r.mark("approval_settled")
    r.key("Enter"); r.mark("approved"); r.sleep(1); r.mark("after_approve")
    r.wait_hook(notify_is("agent-turn-complete"), 180); r.mark("notify_turn_complete"); r.quiet(3, 60); r.mark("ready_after")
    r.submit("/quit"); r.sleep(2); r.end()

@scenario
def x03_approval_reject(out):
    r = codex(out, extra_args=["-a", "on-request", "-s", "read-only"])
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    r.submit("Create a file named probe.txt in this directory containing the word hello, using a shell command."); r.mark("submitted")
    r.wait_for(r"(?i)approv|allow|permission|Yes", 240); r.sleep(2); r.mark("approval_prompt_visible"); r.quiet(2, 30)
    r.key("Esc"); r.mark("rejected_esc"); r.sleep(2); r.mark("after_reject")
    r.wait_hook(notify_is("agent-turn-complete"), 180); r.mark("notify_turn_complete"); r.quiet(3, 60); r.mark("ready_after")
    r.submit("/quit"); r.sleep(2); r.end()

@scenario
def x04_tui_notifications(out):
    r = codex(out, extra_args=["-a", "on-request", "-s", "read-only", "-c", "tui.notifications=true"])
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    r.submit("Create a file named probe.txt in this directory containing the word hello, using a shell command."); r.mark("submitted")
    r.send("\x1b[O"); r.mark("focus_out")  # terminal-notification emitters fire only while unfocused
    r.wait_for(r"(?i)approv|allow|permission|Yes", 240); r.sleep(2); r.mark("approval_prompt_visible"); r.quiet(2, 30)
    r.key("Enter"); r.mark("approved")
    r.wait_hook(notify_is("agent-turn-complete"), 180); r.mark("notify_turn_complete"); r.quiet(3, 60); r.mark("ready_after")
    r.send("\x1b[I"); r.mark("focus_in"); r.submit("/quit"); r.sleep(2); r.end()

@scenario
def x05_question_prompt(out):
    r = codex(out)
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    r.submit("Before answering, ask me one clarifying question using whatever user-input tool you have: do I prefer red or blue? Wait for my answer."); r.mark("submitted")
    r.wait_for(r"(?i)red", 180); r.sleep(2); r.mark("question_visible"); r.quiet(3, 60); r.mark("question_settled")
    r.submit("blue"); r.mark("answered")
    r.wait_hook(notify_is("agent-turn-complete"), 180); r.mark("notify_turn_complete"); r.quiet(3, 60); r.mark("ready_after")
    r.submit("/quit"); r.sleep(2); r.end()

@scenario
def x06_help_resize_typing_paste_compact(out):
    r = codex(out)
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    r.submit("Reply with exactly: ok"); r.wait_hook(notify_is("agent-turn-complete"), 180); r.quiet(3, 60); r.mark("ready")
    r.submit("/help"); r.sleep(1); r.quiet(1, 10); r.mark("help")
    r.key("Esc"); r.sleep(1); r.quiet(1, 10); r.mark("after_help")
    r.resize(30, 100); r.sleep(1); r.quiet(1, 10); r.mark("after_resize_small")
    r.resize(40, 120); r.sleep(1); r.quiet(1, 10); r.mark("after_resize_back")
    r.send("hello there this is a person typing at the prompt", per_char_delay=0.06); r.mark("typed_no_enter"); r.quiet(1, 10); r.mark("typed_settled")
    r.key("CtrlU"); r.sleep(0.5); r.quiet(1, 10); r.mark("cleared")
    r.paste("pasted line\n" * 60); r.mark("pasted"); r.quiet(1, 10); r.mark("paste_settled")
    r.key("CtrlU"); r.sleep(0.5); r.quiet(1, 10); r.mark("cleared_2")
    r.submit("/compact"); r.mark("compact_submitted"); r.quiet(3, 120); r.mark("after_compact")
    r.submit("/quit"); r.sleep(2); r.end()

@scenario
def x07_permission_like_output(out):
    r = codex(out, extra_args=["-a", "never", "-s", "danger-full-access"])
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    r.submit("Run this exact shell command and show its full output verbatim: printf 'Allow command?\\nApprove running ls?\\n> Yes\\n  No\\n'"); r.mark("submitted")
    r.wait_hook(notify_is("agent-turn-complete"), 180); r.mark("notify_turn_complete"); r.quiet(3, 60); r.mark("ready_with_approval_like_output")
    r.submit("/quit"); r.sleep(2); r.end()

@scenario
def x08_resume_picker(out):
    argv = ["codex", "resume"]
    r = Run(out, argv, "/root/proj", base_env(), 40, 120)
    r.sleep(3); r.quiet(1, 20); r.mark("resume_picker"); r.key("Esc"); r.sleep(1); r.key("CtrlC"); r.sleep(1); r.end(5)

# ------------------------------------------- the scenarios the first run missed

ERROR_SERVER = r"""
import http.server, json
class H(http.server.BaseHTTPRequestHandler):
    def respond(self):
        body = json.dumps({"type": "error", "error": {"type": "api_error",
                           "message": "Internal server error"}}).encode()
        self.send_response(500); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers()
        self.wfile.write(body)
    do_POST = respond
    do_GET = respond
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", 8787), H).serve_forever()
"""

@scenario
def c13_compaction(out):
    """Compaction itself, which C8 could not reach: its answer there was "Not
    enough messages to compact", so this builds a conversation first."""
    r = claude(out)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    for i in range(6):
        r.submit("List %d common English verbs, one per line, no commentary." % (20 + i))
        r.wait_hook(hook_is("Stop"), 180); r.quiet(3, 60); r.mark("turn_%d_ready" % i)
    r.submit("/compact"); r.mark("compact_submitted")
    r.wait_hook(hook_is("PreCompact"), 60); r.mark("pre_compact_hook")
    for i in range(12):
        r.sleep(1.0); r.mark("compacting_%d" % i, last_output_ns=r.last_output_ns)
    r.quiet(3, 240); r.mark("after_compact")
    r.submit("Reply with exactly: ok"); r.wait_hook(hook_is("Stop"), 180)
    r.quiet(3, 60); r.mark("ready_after_compact")
    r.submit("/exit"); r.wait_hook(hook_is("SessionEnd"), 30); r.end()

@scenario
def c14_api_error_500(out):
    """A turn the API refuses, inside an otherwise healthy session. The base URL
    is a local server that answers every request 500, so no account request is
    made and the screen is the one a person sees when a turn fails."""
    settings, hooks_file = claude_settings(out)
    server = subprocess.Popen([sys.executable, "-c", ERROR_SERVER])
    try:
        time.sleep(1)
        env = base_env(); env["ANTHROPIC_BASE_URL"] = "http://127.0.0.1:8787"
        r = Run(out, ["claude", "--model", "haiku", "--settings", settings],
                "/root/proj", env, 40, 120, hooks_file)
        r.wait_for(READY, 90); r.mark("prompt_ready"); r.quiet(2, 30); r.mark("idle_before_error")
        r.submit("Reply with exactly: ok"); r.mark("submitted")
        for i in range(15):
            r.sleep(1.0); r.mark("after_submit_%d" % i, last_output_ns=r.last_output_ns)
        r.quiet(5, 240); r.mark("error_settled")
        r.sleep(30); r.mark("thirty_s_after_error", last_output_ns=r.last_output_ns)
        r.submit("/exit"); r.sleep(3); r.end()
    finally:
        server.terminate()

@scenario
def c15_api_unreachable(out):
    """The other failure a person meets: nothing answering at all."""
    settings, hooks_file = claude_settings(out)
    env = base_env(); env["ANTHROPIC_BASE_URL"] = "http://127.0.0.1:9"
    r = Run(out, ["claude", "--model", "haiku", "--settings", settings],
            "/root/proj", env, 40, 120, hooks_file)
    r.wait_for(READY, 90); r.mark("prompt_ready")
    r.submit("Reply with exactly: ok"); r.mark("submitted")
    for i in range(15):
        r.sleep(1.0); r.mark("after_submit_%d" % i, last_output_ns=r.last_output_ns)
    r.quiet(5, 240); r.mark("error_settled")
    r.submit("/exit"); r.sleep(3); r.end()

@scenario
def x09_slash_popup(out):
    """The popup `/` opens, which X6 could not capture: it is also this
    version's command inventory, and tells x10 whether /compact exists."""
    r = codex(out)
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    r.send("/"); r.sleep(1.5); r.quiet(1, 20); r.mark("slash_popup")
    r.send("com"); r.sleep(1.0); r.quiet(1, 20); r.mark("slash_popup_filtered")
    r.key("Esc"); r.sleep(1); r.quiet(1, 15); r.mark("after_esc")
    r.key("CtrlU"); r.sleep(0.5); r.quiet(1, 10); r.mark("cleared")
    r.submit("/quit"); r.sleep(2); r.end()

@scenario
def x10_compaction(out):
    """Codex compaction on its own line: X6's `/compact` was appended to a
    paste and submitted as one message."""
    r = codex(out)
    r.wait_for(CODEX_READY, 120); r.sleep(1.5); r.quiet(2, 30); r.mark("prompt_ready")
    for i in range(5):
        r.submit("List %d common English verbs, one per line, no commentary." % (20 + i))
        r.wait_hook(notify_is("agent-turn-complete"), 240); r.quiet(3, 60); r.mark("turn_%d_ready" % i)
    r.submit("/compact"); r.mark("compact_submitted")
    for i in range(15):
        r.sleep(1.0); r.mark("compacting_%d" % i, last_output_ns=r.last_output_ns)
    r.quiet(3, 240); r.mark("after_compact")
    r.submit("Reply with exactly: ok")
    r.wait_hook(notify_is("agent-turn-complete"), 240); r.quiet(3, 60); r.mark("ready_after_compact")
    r.submit("/quit"); r.sleep(2); r.end()


if __name__ == "__main__":
    name = sys.argv[1]; root = sys.argv[2] if len(sys.argv) > 2 else "/matrix/out"
    out = os.path.join(root, name)
    if os.path.exists(out):
        import shutil; shutil.rmtree(out)
    os.makedirs(out, exist_ok=True)
    try:
        S[name](out)
    except Exception as e:
        with open(os.path.join(out, "driver.log"), "a") as f: f.write("EXCEPTION %r\n" % (e,))
        raise
    print("done", name)
