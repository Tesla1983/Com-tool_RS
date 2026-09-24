#!/usr/bin/env python3
"""**不是被测程序** —— 它是 `tools/backpressure_check.py` 的替身工装。

存在的唯一理由：让那个脚本的判定逻辑（内存判据、退出码、断开检测）能在**两个方向**
上被验证一遍，而不必去动真程序。真程序有运行禁令、也可能有别的毛病会盖住脚本自身的问题。

它刻意复刻被测程序发送侧的**结构**（这是它第一版错的地方）：

  * **UI 线程**：每 10 ms 把一包新分配的 32 KB 投进**无界的 mpsc**
    （`TcpChannel::write` 就是 `commands.send(...)`，从不看队列满不满，直接返回 `Ok`）；
  * **worker 线程**：每轮 read（超时 10 ms）→ 最多推进一次 write（阻塞 socket、无写超时）
    → 一次排空 mpsc，逐条走 `push_bounded`（**按总字节**上限 1 MiB，满了拒绝并记一条
    `Notice` 对应）。
  * 两者**互相独立** —— 真实程序里也如此：worker 卡在 `write` 上时，UI 线程照常入队。

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
LIFE = float(os.environ.get("FAKE_LIFE", "60"))


def main() -> None:
    if len(sys.argv) < 2:
        raise SystemExit("用法：backpressure_dummy_sender.py <端口>")
    port = int(sys.argv[1])
    stop = threading.Event()
    inbox: collections.deque = collections.deque()       # 无界 mpsc
    lock = threading.Lock()
    st = {"accepted": 0, "rejected": 0, "counted": 0}

    def ui_thread() -> None:
        """= UI 线程的 transmit()：从不看队列满不满，照着间隔投。

        每包**新分配**一个 32 KB 缓冲（真实程序每次 send 都是一条新的 `Vec<u8>`）。
        第一版这里复用了同一个 bytes 对象，于是 64 MB 的队列只占几百字节内存，
        把脚本的内存判据骗成了绿色 —— 工装自己的假绿，值得记住。
        """
        next_at = time.monotonic()
        while not stop.is_set():
            with lock:
                inbox.append(bytearray(PAYLOAD))
            st["counted"] += len(PAYLOAD)
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

    s.settimeout(0.010)                 # 只有读超时，写侧保持阻塞（同 TcpChannel）
    print(f"[dummy] 已连 127.0.0.1:{port}，开始按 32KB/10ms 灌数据", flush=True)
    threading.Thread(target=ui_thread, daemon=True).start()

    outgoing: collections.deque = collections.deque()    # 【bytes, sent】
    peak_pending = 0
    while time.monotonic() < deadline:
        try:
            s.recv(4096)
        except (TimeoutError, socket.timeout):
            pass
        except OSError:
            break

        if outgoing:
            data, sent = outgoing[0]
            try:
                n = s.send(data[sent:])          # 阻塞；没有写超时
            except (TimeoutError, socket.timeout):
                n = 0
            except OSError:
                break
            sent += n
            if sent == len(data):
                outgoing.popleft()
            else:
                outgoing[0][1] = sent

        # 一次排空 mpsc，逐条带背压入队
        while True:
            with lock:
                if not inbox:
                    break
                item = inbox.popleft()
            pending = sum(len(b) for b, _ in outgoing)
            peak_pending = max(peak_pending, pending)
            if pending + len(item) > CAP:
                st["rejected"] += 1              # = accept_transmit 报 Notice
            else:
                outgoing.append([item, 0])
                st["accepted"] += 1

    stop.set()
    pending = sum(len(b) for b, _ in outgoing)
    print(f"[dummy] 结束：接受 {st['accepted']} 包 / 拒绝 {st['rejected']} 包"
          f"（对应 Notice {st['rejected']} 条）；在队 {pending} 字节（峰值 {peak_pending}）；"
          f"界面会显示「已发送」{st['counted']} 字节，"
          f"而真正入队的只有 {st['accepted'] * len(PAYLOAD)} 字节", flush=True)
    os._exit(0)


main()
