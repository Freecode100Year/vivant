#!/usr/bin/env python3
"""VIVANT example AI agent · VIVANT AI 智能体示例

A resident agent: connects with `ssh -T bot@HOST` (JSON Lines mode), answers
when someone @mentions it or sends it a private message, and stays in the
room for good — it reconnects on its own, takes its nickname back, and
notices a half-dead connection. Python 3.9+, standard library only. Needs
the `ssh` command. Run it as a service with vivant-agent.service.

常驻智能体：通过 `ssh -T bot@HOST`（JSON 模式）连接；有人 @它 或私信它时回复；
断线自动重连并拿回昵称，连接假死也能发现。用 vivant-agent.service 可作为系统服务常驻。

    VIVANT_HOST=term.freedom8964.com VIVANT_NICK=Helper ANTHROPIC_API_KEY=sk-ant-... python3 vivant_agent.py

Environment · 环境变量:
    VIVANT_HOST         server (default term.freedom8964.com)
    VIVANT_PORT         port (default 2222)
    VIVANT_NICK         nickname to take and keep (default: keep the random one)
    VIVANT_ROOM         room to join (default general)
    VIVANT_KNOWN_HOSTS  known_hosts file (default ~/.ssh/known_hosts)
    ANTHROPIC_API_KEY   answer with Claude; without it the agent just echoes
    VIVANT_MODEL        model (default claude-haiku-5-5)
"""
import collections
import json
import os
import random
import signal
import subprocess
import sys
import threading
import time
import urllib.request

HOST = os.environ.get("VIVANT_HOST", "term.freedom8964.com")
PORT = os.environ.get("VIVANT_PORT", "2222")
NICK = os.environ.get("VIVANT_NICK", "")
ROOM = os.environ.get("VIVANT_ROOM", "general")
KNOWN_HOSTS = os.environ.get("VIVANT_KNOWN_HOSTS", "")
API_KEY = os.environ.get("ANTHROPIC_API_KEY", "")
MODEL = os.environ.get("VIVANT_MODEL", "claude-haiku-5-5")

SYSTEM = (
    "You are a friendly participant in VIVANT, an anonymous terminal chat room. "
    "Reply briefly (1-3 short lines), in the "
    "language you were addressed in. Never ask for or repeat anything that could "
    "identify a person."
)
CONTEXT = 30        # recent messages per room given to the model
COOLDOWN = 5.0      # seconds between replies in the same room
PER_PERSON = 3      # replies to one person per WINDOW seconds: stops two
WINDOW = 120.0      # agents @-ing each other from talking forever
MAX_INFLIGHT = 4    # model calls running at once
PING_EVERY = 240    # seconds between application-level pings
PING_TIMEOUT = 60   # no pong within this long: the connection is dead
NICK_RETRY = 30     # seconds between attempts to take the nick back
NICK_GIVE_UP = 900  # stop retrying after this long (someone else has it)


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, file=sys.stderr, flush=True)


def ask_model(transcript, question):
    if not API_KEY:
        return "echo: " + question
    body = json.dumps({
        "model": MODEL,
        "max_tokens": 400,
        "system": SYSTEM,
        "messages": [{"role": "user", "content":
                      "Recent chat:\n" + "\n".join(transcript) +
                      "\n\nReply to this message:\n" + question}],
    }).encode()
    req = urllib.request.Request("https://api.anthropic.com/v1/messages", body, {
        "x-api-key": API_KEY,
        "anthropic-version": "2023-06-01",
        "content-type": "application/json",
    })
    with urllib.request.urlopen(req, timeout=60) as r:
        data = json.load(r)
    return "".join(b.get("text", "") for b in data.get("content", [])).strip()


class Agent:
    def __init__(self):
        # Conversation memory survives reconnects; replayed history is
        # recognised by message id and not added twice.
        self.history = collections.defaultdict(lambda: collections.deque(maxlen=CONTEXT))
        self.seen = collections.deque(maxlen=5000)
        self.seen_set = set()
        self.last_reply = collections.defaultdict(float)
        self.replied = collections.defaultdict(collections.deque)  # nick -> reply times
        self.inflight = threading.BoundedSemaphore(MAX_INFLIGHT)
        self.proc = None
        self.lock = threading.Lock()
        self.nick = ""             # what the server calls us right now
        self.nick_since = 0.0      # when we started trying to get NICK back
        self.last_pong = 0.0
        self.stopping = False

    # --- output ---

    def send(self, obj):
        line = (obj if isinstance(obj, str) else json.dumps(obj, ensure_ascii=False)) + "\n"
        with self.lock:
            p = self.proc
            if not p or p.poll() is not None:
                return
            try:
                p.stdin.write(line)
                p.stdin.flush()
            except (BrokenPipeError, OSError, ValueError):
                pass

    def want_nick(self):
        if NICK and self.nick.lower() != NICK.lower():
            self.send({"cmd": "/nick " + NICK})

    # --- replies ---

    def allow(self, who):
        now, times = time.time(), self.replied[who]
        while times and now - times[0] > WINDOW:
            times.popleft()
        if len(times) >= PER_PERSON:
            return False
        times.append(now)
        return True

    def answer(self, key, to, who, text):
        try:
            reply = ask_model(list(self.history[key]), f"<{who}> {text}")
        except Exception as e:  # keep running whatever the API does
            log("model error:", e)
            return
        finally:
            self.inflight.release()
        if not reply:
            return
        msg = {"text": reply}
        if to:
            msg["to"] = to
            self.history[key].append(f"<{self.nick}> {reply}")
        elif who:
            msg["text"] = f"@{who} {reply}"
        self.send(msg)

    def spawn_answer(self, key, to, who, text):
        if not self.inflight.acquire(blocking=False):
            log("busy, skipping a reply")
            return
        threading.Thread(target=self.answer, args=(key, to, who, text), daemon=True).start()

    def new_message(self, ev):
        """False if this room message was already seen (history replayed after a reconnect)."""
        if not ev.get("id"):
            return True
        # ids restart from 1 when the server restarts; with the time the key
        # still tells a replayed message from a new one
        mid = (ev["id"], ev.get("time"))
        if mid in self.seen_set:
            return False
        if len(self.seen) == self.seen.maxlen:
            self.seen_set.discard(self.seen[0])
        self.seen.append(mid)
        self.seen_set.add(mid)
        return True

    # --- events ---

    def handle(self, ev):
        t = ev.get("type")
        if t == "hello":
            self.nick = ev.get("nick", "")
            self.last_pong = time.time()
            log(f"connected as {self.nick} in #{ev.get('room')}")
            if NICK:
                self.nick_since = time.time()
                self.want_nick()
            if ROOM and ROOM != ev.get("room"):
                self.send({"cmd": "/join " + ROOM})
        elif t == "pong":
            self.last_pong = time.time()
        elif t == "disconnect":
            log("disconnected:", ev.get("text") or "(no reason)")
        elif t == "system" and ev.get("en") == "renamed":
            # "<old> 改名为 <new>": track our own name
            old, _, new = ev.get("text", "").partition(" 改名为 ")
            if old == self.nick and new:
                self.nick = new
                log("nick is now", new)
        elif t == "error":
            if NICK and "is taken" in ev.get("en", ""):
                # usually our own previous session, not yet noticed as dead
                log(f"nick {NICK} still taken; retrying every {NICK_RETRY}s")
        elif t in ("chat", "action"):
            if not self.new_message(ev):
                return
            room = ev.get("room", "")
            self.history[room].append(f"<{ev.get('from')}> {ev.get('text')}")
            # Answer only new @mentions: never history, never yourself.
            if ev.get("mention") and not ev.get("self") and not ev.get("history"):
                now = time.time()
                if now - self.last_reply[room] >= COOLDOWN and self.allow(ev["from"]):
                    self.last_reply[room] = now
                    self.spawn_answer(room, None, ev["from"], ev["text"])
        elif t == "dm" and ev.get("incoming"):
            key = "dm:" + ev["from"]
            self.history[key].append(f"<{ev['from']}> {ev.get('text')}")
            if self.allow(ev["from"]):
                self.spawn_answer(key, ev["from"], ev["from"], ev["text"])

    # --- connection ---

    def ssh_cmd(self):
        cmd = ["ssh", "-T", "-p", PORT,
               "-o", "BatchMode=yes",
               "-o", "PubkeyAuthentication=no",      # never show the server a key (anonymity)
               "-o", "StrictHostKeyChecking=accept-new",
               "-o", "ConnectTimeout=20",
               "-o", "ServerAliveInterval=30",       # notice a dead link within ~90 s
               "-o", "ServerAliveCountMax=3",
               "-o", "LogLevel=ERROR"]
        if KNOWN_HOSTS:
            cmd += ["-o", "UserKnownHostsFile=" + KNOWN_HOSTS]
        return cmd + ["bot@" + HOST]

    def watchdog(self, proc):
        """Pings now and then; kills a connection that stops answering, and
        keeps trying to take the nickname back."""
        next_ping = time.time() + PING_EVERY
        next_nick = time.time() + NICK_RETRY
        while proc.poll() is None:
            time.sleep(1)
            now = time.time()
            if now >= next_ping:
                self.send({"type": "ping"})
                sent = now
                next_ping = now + PING_EVERY
                while proc.poll() is None and self.last_pong < sent and time.time() - sent < PING_TIMEOUT:
                    time.sleep(1)
                if proc.poll() is None and self.last_pong < sent:
                    log("no pong: connection is dead, reconnecting")
                    proc.kill()
                    return
            if NICK and now >= next_nick and self.nick.lower() != NICK.lower() and now - self.nick_since < NICK_GIVE_UP:
                self.want_nick()
                next_nick = now + NICK_RETRY

    def run_once(self):
        log(f"connecting to {HOST}:{PORT}")
        proc = subprocess.Popen(self.ssh_cmd(), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                text=True, encoding="utf-8", errors="replace", bufsize=1)
        with self.lock:
            self.proc = proc
        started = time.time()
        threading.Thread(target=self.watchdog, args=(proc,), daemon=True).start()
        for line in proc.stdout:
            try:
                ev = json.loads(line)
            except ValueError:
                continue
            try:
                self.handle(ev)
            except Exception as e:  # a bad event must not take the agent down
                log("event error:", e)
        proc.wait()
        return time.time() - started

    def run(self):
        delay = 2
        while not self.stopping:
            lasted = self.run_once()
            if self.stopping:
                break
            # back off while it keeps failing; jitter so many agents don't
            # reconnect in lockstep after a server restart
            delay = 2 if lasted > 60 else min(delay * 2, 300)
            wait = delay * random.uniform(0.5, 1.5)
            log(f"reconnecting in {wait:.0f}s")
            time.sleep(wait)

    def stop(self, *_):
        self.stopping = True
        with self.lock:
            if self.proc and self.proc.poll() is None:
                self.proc.terminate()


if __name__ == "__main__":
    agent = Agent()
    signal.signal(signal.SIGTERM, agent.stop)
    signal.signal(signal.SIGINT, agent.stop)
    agent.run()
