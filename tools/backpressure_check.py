#!/usr/bin/env python3
"""待发队列背压 · 现场验证脚本（对应 HANDOVER.md §4② F5 第 71 项）。

判据来自 `src/communicationtrait.rs::worker`：
  * 上限 `MAX_PENDING_BYTES = 1 MiB`（按**在队字节合计**，不按条数）；
  * 满了**拒绝入队**并向界面报一条 `Notice`（非致命），文案
    「待发队列已满（在队 {pending} 字节，上限 1048576 字节），本条未发送」；
  * `Notice` **不动** `connected`、也**不停**定时发送（这是刻意选它而非 `Failed` 的理由）。

# 为什么脚本能"造"出这个场景（不需要真设备）

队列的**排空速度是有限的**，只要入队比排空快，队列必然堆起来：

  * 串口：`open_port` 用 `.timeout(10ms)`，Windows 上它同时落成
    `WriteTotalTimeoutConstant`（已读 serialport-4.10.0 源码确认）⇒ 单次 `write`
    最坏阻塞 10 ms 就返回 `TimedOut`，worker 才能一轮轮回到 `try_recv` 继续入队；
    而真正的排空上限是**波特率**（1200 波特 ≈ 120 字节/秒）。
  * TCP：`TcpChannel::open` 只设了读超时，写侧是**阻塞** socket。实测（`--self-test`
    可复现）：对端**慢读**时 `send` 每次都返回（偶尔阻塞 1 秒多后仍返回）⇒ worker
    照常转圈；而对端**完全不读**时 `send` 灌满约 2.7 MB 内核缓冲后**永不返回** ⇒
    worker 卡在 `write` 上，`accept_transmit` 再也不会被调用（见文末「已知缺口」）。

⇒ 所以默认模式用纯 TCP + 一个**慢读对端**，零硬件即可验证。

# 这个脚本不做什么

**不会启动被测程序**（项目有明确的运行禁令），也不会替你点界面。它只做三件事：
1. 当对端，按指定速率把数据读走（把排空速度掐在入队速度以下）；
2. 每 0.5 s 采样被测进程的内存（提交量 / 工作集），作为「不 OOM」的判据；
3. 结束时打印一份判定表 + 需要你目视的清单。

用法见 `--help`，或直接看 HANDOVER.md §4② 第 71 项。
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes as wt
import os
import socket
import sys
import time

TICK = 0.05                 # 主循环节拍（秒）
SAMPLE_EVERY = 0.5          # 内存采样间隔（秒）
STATUS_EVERY = 2.0          # 状态行间隔（秒）
MIB = 1024 * 1024


# ─────────────────────────── 被测进程：定位与内存采样 ───────────────────────────

TH32CS_SNAPPROCESS = 0x00000002
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_VM_READ = 0x0010
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value


class _PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wt.DWORD),
        ("cntUsage", wt.DWORD),
        ("th32ProcessID", wt.DWORD),
        ("th32DefaultHeapID", ctypes.c_void_p),
        ("th32ModuleID", wt.DWORD),
        ("cntThreads", wt.DWORD),
        ("th32ParentProcessID", wt.DWORD),
        ("pcPriClassBase", wt.LONG),
        ("dwFlags", wt.DWORD),
        ("szExeFile", wt.WCHAR * 260),
    ]


class _PROCESS_MEMORY_COUNTERS(ctypes.Structure):
    _fields_ = [
        ("cb", wt.DWORD),
        ("PageFaultCount", wt.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),      # 提交量（≈ 私有字节）
        ("PeakPagefileUsage", ctypes.c_size_t),
    ]


_K32 = None


def _kernel32():
    """拿到配置好 argtypes/restype 的 kernel32。

    必须显式声明：ctypes 对未声明参数的 Python int 默认按 **c_int（32 位）** 转，
    64 位下句柄会被截断，`CloseHandle` 会失败、`Process32NextW` 会静默停住。
    """
    global _K32
    if _K32 is not None:
        return _K32
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    k32.CreateToolhelp32Snapshot.restype = wt.HANDLE
    k32.CreateToolhelp32Snapshot.argtypes = [wt.DWORD, wt.DWORD]
    k32.Process32FirstW.argtypes = [wt.HANDLE, ctypes.POINTER(_PROCESSENTRY32W)]
    k32.Process32NextW.argtypes = [wt.HANDLE, ctypes.POINTER(_PROCESSENTRY32W)]
    k32.OpenProcess.restype = wt.HANDLE
    k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
    k32.CloseHandle.argtypes = [wt.HANDLE]
    k32.CloseHandle.restype = wt.BOOL
    _K32 = k32
    return k32


def iter_processes() -> list[tuple[int, str]]:
    """枚举 (pid, exe 名)。只用 ctypes，不依赖 psutil，也不受中文 Windows 影响。"""
    k32 = _kernel32()
    snap = k32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snap == INVALID_HANDLE_VALUE:
        raise OSError("CreateToolhelp32Snapshot 失败")
    out: list[tuple[int, str]] = []
    try:
        entry = _PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(entry)
        ok = k32.Process32FirstW(snap, ctypes.byref(entry))
        while ok:
            out.append((entry.th32ProcessID, entry.szExeFile))
            ok = k32.Process32NextW(snap, ctypes.byref(entry))
    finally:
        k32.CloseHandle(snap)
    return out


def process_memory(pid: int) -> tuple[int, int] | None:
    """返回 (工作集字节, 提交字节)；拿不到就返回 None。"""
    k32 = _kernel32()
    rights = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ
    handle = k32.OpenProcess(rights, False, pid)
    if not handle:
        return None
    try:
        try:
            psapi = ctypes.WinDLL("psapi", use_last_error=True)
            fn = psapi.GetProcessMemoryInfo
        except OSError:
            fn = k32.K32GetProcessMemoryInfo
        fn.argtypes = [wt.HANDLE, ctypes.POINTER(_PROCESS_MEMORY_COUNTERS), wt.DWORD]
        fn.restype = wt.BOOL
        pmc = _PROCESS_MEMORY_COUNTERS()
        pmc.cb = ctypes.sizeof(pmc)
        if not fn(handle, ctypes.byref(pmc), pmc.cb):
            return None
        return int(pmc.WorkingSetSize), int(pmc.PagefileUsage)
    finally:
        k32.CloseHandle(handle)


def find_process(substr: str, explicit_pid: int | None) -> tuple[int, str]:
    procs = iter_processes()
    if explicit_pid:
        for pid, name in procs:
            if pid == explicit_pid:
                return pid, name
        raise SystemExit(f"找不到 PID {explicit_pid}")
    needle = substr.lower()
    hits = [(pid, name) for pid, name in procs if needle in name.lower()]
    if not hits:
        raise SystemExit(
            f"没找到名字里含「{substr}」的进程。\n"
            f"  * 本脚本**不会**替你启动程序（项目有运行禁令），请先自己启动它；\n"
            f"  * 名字不同就用 --proc 指定一段子串，或用 --pid 直接给 PID。"
        )
    if len(hits) > 1:
        pretty = "、".join(f"{n}({p})" for p, n in hits)
        raise SystemExit(f"「{substr}」匹配到多个进程：{pretty}。请用 --pid 指定一个。")
    return hits[0][1], hits[0][0]


# ─────────────────────────── 对端：慢读 / 完全停读 ───────────────────────────


class TcpPeer:
    """监听回环端口，接受程序连上来，然后**按速率节流地读**。

    节流是整件事的关键：排空速度必须低于入队速度，队列才会堆到 1 MiB。
    """

    def __init__(self, host: str, port: int, rate_bps: float):
        self.host, self.port, self.rate = host, port, rate_bps
        self.srv: socket.socket | None = None
        self.conn: socket.socket | None = None
        self.received = 0
        self.peer_closed = False

    def start(self) -> None:
        self.srv = socket.socket()
        self.srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.srv.bind((self.host, self.port))
        self.srv.listen(1)
        real = self.srv.getsockname()
        print(f"[对端] 已在 {real[0]}:{real[1]} 上监听"
              f"（节流速率 {'不限' if self.rate <= 0 else f'{self.rate / 1024:.0f} KB/s'}）")

    def wait_peer(self, timeout: float) -> bool:
        assert self.srv is not None
        self.srv.settimeout(min(0.5, timeout))
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                conn, addr = self.srv.accept()
            except (TimeoutError, socket.timeout):
                continue
            conn.settimeout(TICK)
            self.conn = conn
            print(f"[对端] 程序已连上来（来自 {addr[0]}:{addr[1]}）")
            return True
        return False

    def pump(self, dt: float) -> int:
        """读走至多 `rate * dt` 字节。rate <= 0 表示**完全不读**。"""
        if self.conn is None or self.rate <= 0:
            return 0
        budget = max(1, int(self.rate * dt))
        got = 0
        while budget > 0:
            try:
                data = self.conn.recv(min(4096, budget))
            except (TimeoutError, socket.timeout):
                break
            except OSError:
                self.peer_closed = True
                return got
            if not data:
                self.peer_closed = True
                break
            got += len(data)
            budget -= len(data)
        self.received += got
        return got

    def drain(self, seconds: float) -> int:
        """不限速地把剩下的全读走（用于收尾与统计）。"""
        if self.conn is None:
            return 0
        got, deadline = 0, time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                data = self.conn.recv(1 << 16)
            except (TimeoutError, socket.timeout):
                continue
            except OSError:
                self.peer_closed = True
                break
            if not data:
                self.peer_closed = True
                break
            got += len(data)
        self.received += got
        return got

    def alive(self) -> bool:
        return self.conn is not None and not self.peer_closed

    def close(self) -> None:
        for s in (self.conn, self.srv):
            if s is not None:
                try:
                    s.close()
                except OSError:
                    pass
        self.conn = self.srv = None


class SerialPeer:
    """串口对端（需要一对虚拟串口，例如 com0com；或者程序连另一端、脚本持这一端）。

    `rate = 0`（完全不读）在这里是**可行**的：串口 `write` 有 10 ms 超时，
    驱动缓冲满了之后 worker 照样能转圈——这正是它比 TCP 稳的地方。
    """

    def __init__(self, port: str, baud: int, rate_bps: float):
        self.port, self.baud, self.rate = port, baud, rate_bps
        self.ser = None
        self.received = 0

    def start(self) -> None:
        try:
            import serial  # pyserial
        except ImportError:
            raise SystemExit(
                "串口模式需要 pyserial：\n"
                "  <隔离 venv>/python -m pip install pyserial"
            )
        self.ser = serial.Serial(self.port, self.baud, timeout=0)
        print(f"[对端] 已独占 {self.port} @ {self.baud}，"
              f"节流速率 {'不限' if self.rate <= 0 else f'{self.rate / 1024:.0f} KB/s'}；"
              f"缓冲现有 {self.ser.in_waiting} 字节")

    def wait_peer(self, timeout: float) -> bool:
        # 串口没有"握手"，占住端口就算就绪；给对方一点时间把程序连上
        time.sleep(min(2.0, timeout))
        return True

    def pump(self, dt: float) -> int:
        if self.ser is None or self.rate <= 0:
            return 0
        budget = max(1, int(self.rate * dt))
        got = 0
        while budget > 0 and self.ser.in_waiting > 0:
            n = min(4096, budget, self.ser.in_waiting)
            data = self.ser.read(n)
            if not data:
                break
            got += len(data)
            budget -= len(data)
        self.received += got
        return got

    def drain(self, seconds: float) -> int:
        if self.ser is None:
            return 0
        got, deadline = 0, time.monotonic() + seconds
        while time.monotonic() < deadline:
            data = self.ser.read(min(1 << 16, max(1, self.ser.in_waiting)))
            if data:
                got += len(data)
            else:
                time.sleep(0.02)
        self.received += got
        return got

    def alive(self) -> bool:
        return self.ser is not None

    def close(self) -> None:
        if self.ser is not None:
            try:
                self.ser.close()
            except Exception:
                pass
            self.ser = None


class NullPeer:
    """只当观众：不碰任何端口（用于"程序独占一个真实串口适配器"的场合）。"""

    def start(self) -> None:
        print("[对端] 观察模式：脚本不持有任何端口（内存判据照常可用，"
              "背压提示请在程序里目视确认）")

    def wait_peer(self, timeout: float) -> bool:
        return True

    def pump(self, dt: float) -> int:
        return 0

    def drain(self, seconds: float) -> int:
        return 0

    def alive(self) -> bool:
        return True

    def close(self) -> None:
        pass

    received = 0


# ─────────────────────────── 判定 ───────────────────────────


def tail_slope(samples: list[tuple[float, int]], window: float) -> float:
    """末尾 `window` 秒的提交量线性斜率（字节/秒）；样本不足返回 0。"""
    if len(samples) < 2:
        return 0.0
    t_end = samples[-1][0]
    pts = [(t, m) for t, m in samples if t >= t_end - window]
    if len(pts) < 3:
        pts = samples[-3:]
    n = len(pts)
    mean_t = sum(t for t, _ in pts) / n
    mean_m = sum(m for _, m in pts) / n
    denom = sum((t - mean_t) ** 2 for t, _ in pts)
    if denom <= 0:
        return 0.0
    return sum((t - mean_t) * (m - mean_m) for t, m in pts) / denom


def fmt_mb(x: int | float) -> str:
    return f"{x / MIB:.1f} MB"


def run_verification(args) -> int:
    pid, name = find_process(args.proc, args.pid)
    print(f"[被测] {name} (PID {pid})")

    enqueue_bps = (args.payload_bytes / (args.interval_ms / 1000.0)) if args.mode != "watch" else 0
    rate_bps = args.read_rate * 1024

    if args.mode == "watch":
        peer: object = NullPeer()
    elif args.mode == "serial":
        if not args.serial_port:
            raise SystemExit("--mode serial 需要 --serial-port")
        peer = SerialPeer(args.serial_port, args.baud, rate_bps)
    else:
        peer = TcpPeer(args.host, args.port, rate_bps)
    peer.start()

    print()
    print("=" * 78)
    print("现场操作（请在程序里照做，脚本不会替你操作界面）：")
    if args.mode == "tcp":
        print(f"  1. 连接类型选「TCP 客户端」；「远端地址」填 {args.host}；"
              f"「远端端口」填 {peer.srv.getsockname()[1]}")
        print("  2. 点「连接」。本脚本会同步告诉你是否连上。")
    elif args.mode == "serial":
        print(f"  1. 连接类型选「串口」，端口选**另一端**（脚本已独占 {args.serial_port}）；")
        print(f"  2. 波特率填 {args.baud}（要和脚本一致），点「打开串口」。")
    else:
        print("  1. 用你平时的串口连接（适配器上不要接会把它读干的设备，或干脆空着）。")
    print(f"  3. 往发送框里粘一大段载荷（{args.payload_bytes} 字节左右；"
          f"把脚本生成的载荷文件全选复制即可）。")
    print(f"  4. 「定时发送间隔」拉到最小 {args.interval_ms} ms，勾上「定时发送」。")
    print(f"  ⇒ 预期入队速率 ≈ {enqueue_bps / 1024:.0f} KB/s"
          f"（{args.payload_bytes} B / {args.interval_ms} ms），"
          f"对端排空 ≈ {rate_bps / 1024:.0f} KB/s。")
    if rate_bps > 0 and enqueue_bps <= rate_bps:
        print("  ⚠️ 入队速率没有超过排空速率 —— 队列不会涨，先把载荷调大或间隔调小。")
    if rate_bps == 0:
        print("  ⚠️ 当前是完全停读（--read-rate 0）。TCP 下 worker 会卡死在 write 上、"
              "预期**看不到**背压提示（见文末已知缺口）；串口下这一档是正常可测的。")
    print("=" * 78)
    print()

    if not peer.wait_peer(30.0):
        peer.close()
        raise SystemExit("等待程序连接超时（30 秒）。请确认端口填对了、并点了「连接」。")

    if args.no_input:
        print(f"[同步] {args.countdown} 秒后开始测量（--no-input）。"
              f"请在这段时间里完成上面的第 3、4 步。")
        time.sleep(args.countdown)
    else:
        try:
            input("[同步] 完成第 3、4 步后按回车开始测量（Ctrl+C 可随时中止）… ")
        except EOFError:
            time.sleep(args.countdown)

    # ── 测量阶段 ──
    print(f"\n[测量] 开始，持续 {args.run_seconds:.0f} 秒；每 {STATUS_EVERY:.0f} 秒一行状态。\n")
    t0 = time.monotonic()
    last_sample = last_status = t0
    samples: list[tuple[float, int]] = []
    commit0 = None
    peak_commit = 0
    drained_by_peer = 0

    while True:
        now = time.monotonic()
        elapsed = now - t0
        if elapsed >= args.run_seconds:
            break
        step0 = time.monotonic()
        drained_by_peer += peer.pump(TICK)
        time.sleep(max(0.0, TICK - (time.monotonic() - step0)))

        if now - last_sample >= SAMPLE_EVERY:
            mem = process_memory(pid)
            if mem is not None:
                ws, commit = mem
                samples.append((elapsed, commit))
                if commit0 is None:
                    commit0 = commit
                peak_commit = max(peak_commit, commit)
            last_sample = now

        if now - last_status >= STATUS_EVERY:
            mem = process_memory(pid)
            commit = mem[1] if mem else 0
            growth = (commit - (commit0 or commit)) if commit0 else 0
            print(f"  t={elapsed:5.1f}s  提交 {fmt_mb(commit):>9}  "
                  f"(Δ{growth / MIB:+7.1f} MB)  对端已读 "
                  f"{drained_by_peer / 1024:8.0f} KB  "
                  f"{'⚠️ 程序已断开' if not peer.alive() else ''}")
            last_status = now

        if not peer.alive():
            print("  ⚠️ 对端连接已断开（程序那边可能点了断开或崩了），提前结束测量。")
            break

    # ── 收尾：不限速读干净，统计真正送达的字节 ──
    print(f"\n[收尾] 请现在取消勾选「定时发送」。{args.drain_seconds:.0f} 秒内不限速读完剩余数据…")
    tail_received = peer.drain(args.drain_seconds)
    total_received = peer.received
    peer.close()

    # ── 判定 ──
    print()
    print("=" * 78)
    print("判定")
    print("=" * 78)
    if not samples or commit0 is None:
        print("✗ 没能采到内存样本（OpenProcess 被拒？），内存判据无法给出。")
        mem_ok = None
        growth = peak = slope = 0
    else:
        growth = peak_commit - commit0
        slope = tail_slope(samples, window=10.0)
        mem_ok = (growth <= args.max_growth_mb * MIB) and (slope <= args.max_slope_mb * MIB)
        print(f"  起始提交量      : {fmt_mb(commit0)}")
        print(f"  峰值提交量      : {fmt_mb(peak_commit)}")
        print(f"  增长            : {fmt_mb(growth)}"
              f"（判据 ≤ {args.max_growth_mb:.0f} MB）")
        print(f"  末 10 秒斜率    : {slope / MIB:+.2f} MB/s"
              f"（判据 ≤ {args.max_slope_mb:.2f} MB/s）")
        print(f"  → 不 OOM 判据   : {'✅ 通过' if mem_ok else '❌ 不通过'}")
    print(f"  对端实际读到     : {total_received:,} 字节"
          f"（其中收尾阶段 {tail_received:,}）")

    print()
    print("需要你目视确认的三条（脚本看不到程序界面）：")
    print("  [ ] 接收区出现下面这条提示，并且会**反复**出现（每个被拒的定时包各一条）：")
    print("        「…: 待发队列已满（在队 … 字节，上限 1048576 字节），本条未发送」")
    print("  [ ] 界面**仍显示「已连接」**（绿点），没有跳「未连接」")
    print("  [ ] 「定时发送」**没有被自动取消**（复选框还勾着）")
    print()
    print("对照项（本次读码发现的既有缺口，不是本项判据）：")
    print("  * 状态栏「发送字节数」会把**被拒的那些包也记上**，与「本条未发送」自相矛盾；")
    print("    「发送后清空」若开着，输入框也会被清空。原因：通道的 `write()` 只把命令投进")
    print("    mpsc 就返回 `Ok`，判断队列满不满的是后台线程。")
    print(f"  * 可拿上面的「对端实际读到 {total_received:,} 字节」和状态栏数字对照："
          f"状态栏明显更大，差额就是被背压拒掉的量。")
    print()
    if rate_bps == 0 and args.mode == "tcp":
        print("已知缺口（完全停读这一档）：TCP 的写侧没有写超时，对端彻底停读时 `send` 会")
        print("永不返回，worker 卡在 `write` 上，`accept_transmit` 再也不会被调用 —— 于是")
        print("上限 1 MiB **不生效**，真正无限增长的是无界的 mpsc 通道。串口没有这个问题")
        print("（写超时 10 ms 保证 worker 能回到入队循环）。")
        print()

    if mem_ok is False:
        return 1
    return 0


# ─────────────────────────── 自检 ───────────────────────────


def self_test() -> int:
    """不碰被测程序、不碰串口，先证明脚本自己的观测与节流逻辑是有效的。

    这也把「TCP 慢读 vs 完全不读」的 socket 语义钉成可复现的输出 —— 脚本默认走
    TCP 的全部依据就在这里。
    """
    print("=" * 78)
    print("自检 1/3：进程枚举与内存采样（目标=本进程）")
    print("=" * 78)
    me = os.getpid()
    procs = dict(iter_processes())
    print(f"  枚举到 {len(procs)} 个进程；本进程 PID {me} = {procs.get(me)!r}")
    mem = process_memory(me)
    if mem is None:
        print("  ✗ 采不到自己的内存 —— 内存判据不可用")
        return 1
    ws, commit = mem
    blob = bytearray(32_000_000)      # 故意分配 32 MB，看采样器能不能看见
    time.sleep(0.3)
    mem2 = process_memory(me)
    delta = (mem2[1] - commit) if mem2 else 0
    print(f"  工作集 {ws / MIB:.1f} MB / 提交 {commit / MIB:.1f} MB")
    print(f"  故意分配 32 MB 后：提交 {mem2[1] / MIB:.1f} MB（Δ{delta / MIB:+.1f} MB）")
    del blob
    ok1 = delta > 16 * MIB           # 分配了 32 MB，涨幅必须看得出来
    print(f"  → {'✅ 采样器能看见内存变化' if ok1 else '❌ 采样器看不见变化（判据会假绿）'}")

    print()
    print("=" * 78)
    print("自检 2/3：对端节流是否真的把排空速度压住了")
    print("=" * 78)
    ok2, rate_seen = _self_test_throttle()

    print()
    print("=" * 78)
    print("自检 3/3：TCP「慢读」与「完全不读」下发送侧的行为差异")
    print("=" * 78)
    ok3 = _self_test_socket_semantics()

    print()
    print(f"自检结果：采样器 {'OK' if ok1 else 'FAIL'} / "
          f"节流 {'OK' if ok2 else 'FAIL'} / socket 语义 {'OK' if ok3 else 'FAIL'}")
    return 0 if (ok1 and ok2 and ok3) else 1


def _self_test_throttle() -> tuple[bool, float]:
    """起一对回环连接，让发送端拼命写、脚本按 40 KB/s 读，量实际读速。"""
    peer = TcpPeer("127.0.0.1", 0, 40 * 1024)
    peer.start()
    import threading

    def writer():
        try:
            for _ in range(400):
                peer_cli.send(b"y" * 8192)
        except OSError:
            pass

    peer_cli = socket.create_connection(peer.srv.getsockname())
    peer.conn, _ = peer.srv.accept()
    peer.conn.settimeout(TICK)
    th = threading.Thread(target=writer, daemon=True)
    th.start()

    t0, got = time.monotonic(), 0
    while time.monotonic() - t0 < 2.0:
        step0 = time.monotonic()
        got += peer.pump(TICK)
        time.sleep(max(0.0, TICK - (time.monotonic() - step0)))
    elapsed = time.monotonic() - t0
    seen = got / elapsed
    peer.close()
    print(f"  2 秒内对端按 40 KB/s 的设定只读到 {got:,} 字节 = {seen / 1024:.1f} KB/s")
    # 宽容一点：节拍误差下允许 ±40%
    ok = 24 * 1024 <= seen <= 56 * 1024
    print(f"  → {'✅ 节流生效（排空被压住了）' if ok else '❌ 节流没生效，队列涨不起来'}")
    return ok, seen


def _self_test_socket_semantics() -> bool:
    """对端不读 => 阻塞 send 不返回；对端慢读 => send 持续返回。"""
    import threading

    def make():
        srv = socket.socket()
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        cli = socket.create_connection(srv.getsockname())
        conn, _ = srv.accept()
        return srv, conn, cli

    slice_bytes = 64 * 1024

    def send_loop(cli, state, seconds):
        payload = b"z" * slice_bytes
        while not state["stop"] and time.monotonic() < seconds:
            t0 = time.monotonic()
            try:
                n = cli.send(payload)
            except OSError as e:
                state["error"] = str(e)
                return
            state["bytes"] += n
            state["calls"] += 1
            state["worst"] = max(state["worst"], time.monotonic() - t0)
            state["last_progress"] = time.monotonic()
            time.sleep(0.010)

    # A：完全不读
    srv, conn, cli = make()
    state = {"bytes": 0, "calls": 0, "worst": 0.0, "last_progress": time.monotonic(),
             "stop": False, "error": None}
    threading.Thread(target=send_loop, args=(cli, state, time.monotonic() + 30), daemon=True).start()
    time.sleep(3.0)
    stuck = time.monotonic() - state["last_progress"]
    blocked = stuck > 1.0
    print(f"  A 对端完全不读：3 秒写入 {state['bytes']:,} 字节 / {state['calls']} 次 send，"
          f"之后 {stuck:.1f}s 无进展 → {'send 已被堵住（预期）' if blocked else '仍在推进（与预期不符）'}")
    srv2, conn2, cli2 = srv, conn, cli
    state["stop"] = True

    # B：慢读 40 KB/s
    srv, conn, cli = make()
    state_b = {"bytes": 0, "calls": 0, "worst": 0.0, "last_progress": time.monotonic(),
               "stop": False, "error": None}
    threading.Thread(target=send_loop, args=(cli, state_b, time.monotonic() + 30), daemon=True).start()
    got, t0 = 0, time.monotonic()
    while time.monotonic() - t0 < 3.0:
        try:
            conn.settimeout(0.05)
            chunk = conn.recv(4096)
        except OSError:
            continue
        if not chunk:
            break
        got += len(chunk)
        time.sleep(4096 / (40 * 1024))
    progressed = time.monotonic() - state_b["last_progress"]
    state_b["stop"] = True
    cycling = progressed <= 1.0 and state_b["bytes"] > 0
    print(f"  B 对端慢读 40 KB/s：对端读走 {got:,} 字节；发送侧推进 {state_b['bytes']:,} 字节"
          f" / {state_b['calls']} 次，最慢一次 {state_b['worst']:.3f}s")
    print(f"    → {'✅ send 持续返回，worker 能一轮轮回到入队循环（TCP 可测背压）' if cycling else '❌ send 也被堵死了（TCP 不可测）'}")
    for s in (conn2, cli2, srv2, conn, cli, srv):
        try:
            s.close()
        except OSError:
            pass
    return blocked and cycling


# ─────────────────────────── 载荷文件 / CLI ───────────────────────────


def write_payload(path: str, size: int) -> None:
    line = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz\n"
    with open(path, "w", encoding="utf-8", newline="") as f:
        written = 0
        while written < size:
            chunk = line[: max(1, min(len(line), size - written))]
            f.write(chunk)
            written += len(chunk)
    print(f"[载荷] 已写出 {path}（{written} 字节）。"
          f"请用记事本打开 → 全选 → 复制 → 粘进程序的发送框。")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description="待发队列背压（F5 第 71 项）现场验证脚本",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "典型用法（零硬件，TCP 慢读对端）：\n"
            "  1) python tools/backpressure_check.py --write-payload payload.txt\n"
            "  2) 启动被测程序（脚本不会替你启动）\n"
            "  3) python tools/backpressure_check.py --mode tcp --port 23232\n"
            "  4) 按脚本提示在程序里连 127.0.0.1:23232、粘贴载荷、间隔 10ms、开定时发送\n"
            "\n"
            "串口路线（需要一对虚拟串口，例如 com0com）：\n"
            "  python tools/backpressure_check.py --mode serial --serial-port COM7 --baud 115200\n"
            "\n"
            "只开内存看门狗（程序独占一个真实适配器时）：\n"
            "  python tools/backpressure_check.py --mode watch\n"
            "\n"
            "先自检脚本自身：python tools/backpressure_check.py --self-test\n"
        ),
    )
    p.add_argument("--mode", choices=["tcp", "serial", "watch"], default="tcp",
                   help="tcp=脚本监听并慢读（默认，零硬件）；serial=脚本持有一端串口；watch=只看内存")
    p.add_argument("--host", default="127.0.0.1", help="TCP 监听地址（默认 127.0.0.1）")
    p.add_argument("--port", type=int, default=23232, help="TCP 监听端口（默认 23232）")
    p.add_argument("--serial-port", help="串口模式：脚本这一端占用的端口，如 COM7")
    p.add_argument("--baud", type=int, default=115200, help="串口模式的波特率（要和程序里一致）")
    p.add_argument("--read-rate", type=float, default=40.0,
                   help="对端读取速率 KB/s（默认 40；填 0 = 完全不读，用于对照实验）")
    p.add_argument("--proc", default="comtool", help="被测进程名里含的子串（默认 comtool）")
    p.add_argument("--pid", type=int, help="直接指定 PID（多个同名进程时用）")
    p.add_argument("--payload-bytes", type=int, default=32768,
                   help="发送框里载荷的字节数（默认 32768，用于估算入队速率）")
    p.add_argument("--interval-ms", type=int, default=10,
                   help="定时发送间隔（默认 10，程序允许的最小值）")
    p.add_argument("--run-seconds", type=float, default=45.0, help="测量时长（默认 45 秒）")
    p.add_argument("--drain-seconds", type=float, default=5.0, help="收尾不限速读的时长（默认 5 秒）")
    p.add_argument("--countdown", type=float, default=20.0,
                   help="--no-input 时的同步等待秒数（默认 20）")
    p.add_argument("--no-input", action="store_true", help="不等回车，改用固定倒计时")
    p.add_argument("--max-growth-mb", type=float, default=32.0,
                   help="整个测量期内提交量的允许增长上限（默认 32 MB）")
    p.add_argument("--max-slope-mb", type=float, default=1.0,
                   help="末段斜率上限 MB/s（默认 1.0）")
    p.add_argument("--write-payload", metavar="PATH",
                   help="写出载荷文件后退出（不进入验证流程）")
    p.add_argument("--self-test", action="store_true",
                   help="自检脚本自身的采样器 / 节流 / socket 语义，不碰被测程序与串口")
    return p


def main() -> int:
    args = build_parser().parse_args()
    if args.write_payload:
        write_payload(args.write_payload, args.payload_bytes)
        return 0
    if args.self_test:
        return self_test()
    return run_verification(args)


if __name__ == "__main__":
    sys.exit(main())
