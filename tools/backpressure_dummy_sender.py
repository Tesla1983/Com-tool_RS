#!/usr/bin/env python3
"""**不是被测程序** —— 它是 `tools/backpressure_check.py` 的替身工装。

存在的唯一理由：让那个脚本的判定逻辑（内存判据、退出码、断开检测）能在**两个方向**
上被验证一遍，而不必去动真程序。真程序有运行禁令、也可能有别的毛病会盖住脚本自身的问题。

它刻意复刻被测程序发送侧的**结构**：

  * **共享账本**（`ledger["bytes"]`）：数"交出去、但一个字节都还没写出去"的那一段
    （= `communicationtrait::worker::PendingLedger`）；
  * **UI 线程**（= `TcpChannel::write` → `worker::submit`）：每 10 ms 拿一包**新分配**的
    32 KB，**先在调用线程预约额度**；预约不上就**当场拒绝**（对应 `write()` 返回 `Err`
    和界面上那条「本条未发送」），**一个字节都不入队**；预约上了才投进 mpsc。
  * **worker 线程**：每轮 read → 最多推进一次 write → 一次排空 mpsc（只 `push_back`，
    不再自己判上限）；队首整段写完后**出队并归还额度**（= `retire_finished_chunk`）。
  * 两者**互相独立** —— 真实程序里也如此：worker 卡在 `write` 上时，UI 线程照常试投。

⚠️ **上一版这里是错的，别改回去**：它把上限判在 worker 侧（`push_bounded`），UI 线程
"从不看队列满不满、直接投"。那是 2026-09-24 之前的旧结构，也是"状态栏计数照涨、输入框
照清"那个缺陷的来源。结构变了，替身必须跟着变，否则它验证的是另一个程序。

⚠️ **写侧非阻塞**（= `TcpChannel::open` 的 `set_nonblocking(true)`）：这一条不是装饰。
`--read-rate 0`（对端完全不读）那一档下，阻塞 socket 会让 worker 永远卡在 `send` 上，
而 UI 线程照常入队 —— 于是**替身的内存会一直涨**，脚本得出"不 OOM 判据 ❌"，
与我们刚修好的行为正好相反。非阻塞后才与真程序一致。

用法（配合 `backpressure_check.py --mode tcp`）：

    # 情形一：上限 1 MiB —— 脚本应当报 ✅、退出码 0
    python tools/backpressure_dummy_sender.py 23232

    # 情形二：把上限放开 —— 脚本应当报 ❌、退出码 1（证明判据不是空转）
    FAKE_CAP=1000000000000 python tools/backpressure_dummy_sender.py 23232

环境变量：`FAKE_CAP`（待发字节上限，默认 1048576）、`FAKE_LIFE`（存活秒数，默认 60）。
"""

from __future__ import annotations

import collections
import os
import socket
import sys
import threading
import time

CAP = int(os.environ.get("FAKE_CAP", 1024 * 1024))   # = worker::MAX_PENDING_BYTES
PAYLOAD = b"A" * 32_768        # 32 KB / 10 ms => 3.2 MB/s 入队
INTERVAL = 0.010
IDLE_SLEEP = 0.001             # = net_impl::tcp::IDLE_SLEEP
LIFE = float(os.environ.get("FAKE_LIFE", "60"))


def main() -> None:
    if len(sys.argv) < 2:
        raise SystemExit("用法：backpressure_dummy_sender.py <端口>")
    port = int(sys.argv[1])
    stop = threading.Event()
    inbox: collections.deque = collections.deque()       # mpsc
    lock = threading.Lock()
    # 共享账本：UI 线程加、worker 线程减。两个线程都读它 ⇒ 必须加锁。
    ledger = {"bytes": 0}
    st = {"attempts": 0, "accepted": 0, "rejected": 0, "counted": 0}

    def ui_thread() -> None:
        """= UI 线程的 `transmit()` → `write()` → `worker::submit`。

        **预约在前、入队在后**：预约失败就当场回绝，入队 0 字节。这就是"同步 `Err`"。
        每包**新分配**一个 32 KB 缓冲（真实程序每次 send 都是一条新的 `Vec<u8>`）。
        更早的一版在这里复用了同一个 bytes 对象，于是 64 MB 的队列只占几百字节内存，
        把脚本的内存判据骗成了绿色 —— 工装自己的假绿，值得记住。
        """
        next_at = time.monotonic()
        while not stop.is_set():
            item = bytearray(PAYLOAD)       # 先分配：真程序也是先有字节再谈能不能发
            with lock:
                st["attempts"] += 1
                if ledger["bytes"] + len(item) > CAP:
                    st["rejected"] += 1
                    item = None             # 一个字节都不入队
                else:
                    ledger["bytes"] += len(item)
                    st["accepted"] += 1
                    st["counted"] += len(item)      # = 界面 `tx_bytes += len`（只在 Ok 时）
                    inbox.append(item)
            del item
            next_at += INTERVAL
            time.sleep(max(0.0, next_at - time.monotonic()))

    print(f"[dummy] pid={os.getpid()} 上限={CAP:,} 字节；等待 127.0.0.1:{port} 有人监听…",
          flush=True)

    s = None
    deadline = time.monotonic() + LIFE
    while time.monotonic() < deadline:
        try:
            s = socket.create_connection(("127.0.0.1", port))
            break
        except OSError:
            time.sleep(0.3)
    if s is None:
        print("[dummy] 一直没人监听，退出", flush=True)
        os._exit(2)

    s.setblocking(False)                # 写侧非阻塞（同 TcpChannel）
    print(f"[dummy] 已连 127.0.0.1:{port}，开始按 32KB/10ms 试投（上限 {CAP:,}）",
          flush=True)
    threading.Thread(target=ui_thread, daemon=True).start()

    outgoing: collections.deque = collections.deque()    # 【bytearray, sent】
    peak_pending = 0
    idle = False
    while time.monotonic() < deadline:
        if idle:
            time.sleep(IDLE_SLEEP)
        idle = True

        try:
            s.recv(4096)
            idle = False
        except (BlockingIOError, InterruptedError):
            pass
        except OSError:
            break

        # 最多推进一次 write：非阻塞，写不动就留到下一轮（不再阻塞在这儿）
        if outgoing:
            data, sent = outgoing[0]
            try:
                n = s.send(memoryview(data)[sent:])
            except (BlockingIOError, InterruptedError):
                n = 0
            except OSError:
                break
            if n:
                idle = False
                sent += n
                if sent == len(data):
                    # 整段写完 ⇒ 出队 + 归还额度（= retire_finished_chunk，两件事一起做）
                    outgoing.popleft()
                    with lock:
                        ledger["bytes"] -= len(data)
                else:
                    outgoing[0][1] = sent

        # 一次排空 mpsc：**只 push_back**，上限已在调用线程判过
        while True:
            with lock:
                if not inbox:
                    break
                outgoing.append([inbox.popleft(), 0])
                idle = False
            pending = sum(len(b) for b, _ in outgoing)
            peak_pending = max(peak_pending, pending)

    stop.set()
    pending = sum(len(b) for b, _ in outgoing)
    print(f"[dummy] 结束：试投 {st['attempts']} 包 = 接受 {st['accepted']} / 拒绝 {st['rejected']}"
          f"（= 界面上的提示 {st['rejected']} 条）；在队 {pending} 字节（峰值 {peak_pending}）",
          flush=True)
    print(f"[dummy] 记账应为 {st['counted']:,} 字节 = 被接受的那些；"
          f"若界面显示得比这更大，说明「被拒也记账」那个缺陷又回来了", flush=True)
    os._exit(0)


main()
