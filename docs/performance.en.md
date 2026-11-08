# Performance

[← Back to README](../README.en.md) · [简体中文](performance.md) | English

Measured with the bundled load generator `vivant-bench`: everyone in the same room, 25 people each posting every 2 seconds (about 12.5 messages a second for the room), the server pinned to **1 CPU core** (Intel Xeon 2.1 GHz) with `taskset`, the load generator on another core:

| Sessions online | Mode | Delivered | Latency p50 / p99 | Memory (peak RSS) |
| --- | --- | --- | --- | --- |
| 1000 | full interface | 100% | 12 ms / 33 ms | 74 MB |
| 2000 | full interface | 100% | 24 ms / 66 ms | 140 MB |
| 3000 | full interface | 100% | 33 ms / 88 ms | 232 MB |
| 4000 | line mode | 100% | 74 ms / 210 ms | 239 MB |

Bottom line: **a 1-core, 1 GB VPS handles 3000+ people online at once in the full interface**, with latency still under 0.1 s. For more people, add CPU cores (all of them are used). The same number of people spread over several rooms costs far less.

For reference, the Go version (v0.9) in its documented run (another machine, also one core): 2000 people 30 ms / 148 ms, 355 MB; 3000 people 91 ms / 478 ms, 532 MB; 4000 in line mode 423 ms / 1.4 s, 360 MB.

Key design choices:

- **A small hand-written terminal renderer.** It uses the terminal's scroll region, so each new message sends only its new lines, and the fixed lines are redrawn only when they change.
- **One lock for the whole chat state, never held while waiting on the network.** Delivery never blocks: a slow reader only loses its own messages instead of stalling the room.
- **Messages are shared between sessions.** Each session keeps pointers to shared messages and renders scrollback only when you scroll.
- **No general styling engine on the hot path.** Colours are precomputed escape codes and the width of ASCII text is computed directly.
- **Ordinary chat messages don't refresh the status bar**, so they don't contend for the global lock.
- **Static musl binaries use the mimalloc allocator**, so musl's own allocator doesn't slow down many threads.

Try it yourself: `make bench`, or see the comment at the top of `src/bin/vivant-bench.rs`.
