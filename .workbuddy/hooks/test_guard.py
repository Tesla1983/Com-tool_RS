#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""comtool-run-guard.sh 的行为测试。

对每一条构造好的 hook payload，断言守卫脚本是「放行」还是「拒绝」。
只读写 ~/.workbuddy 下的批准文件，不启动本项目，不接触任何串口。
"""
import json
import os
import shutil
import subprocess
import sys
import time

GUARD = os.path.join(os.path.expanduser("~"), ".workbuddy", "hooks", "comtool-run-guard.sh")
CONFIG_DIR = os.environ.get("WORKBUDDY_CONFIG_DIR") or os.path.join(os.path.expanduser("~"), ".workbuddy")
GRANT = os.path.join(CONFIG_DIR, "combot-run-approval")


def find_bash():
    for cand in (
        shutil.which("bash"),
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
        os.path.join(os.path.expanduser("~"), "scoop", "apps", "git", "current", "bin", "bash.exe"),
    ):
        if cand and os.path.exists(cand):
            return cand
    return None


BASH = find_bash()


def run(payload_raw: str) -> str:
    """返回 'ALLOW' 或 'DENY'。"""
    proc = subprocess.run(
        [BASH, GUARD],
        input=payload_raw.encode("utf-8"),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    out = proc.stdout.decode("utf-8", "replace")
    if '"permissionDecision":"deny"' in out:
        return "DENY"
    if out.strip():
        return "UNEXPECTED:" + out.strip()[:200]
    return "ALLOW"


def bash_payload(cmd: str) -> str:
    return json.dumps(
        {
            "session_id": "test",
            "transcript_path": "/tmp/t.jsonl",
            "cwd": "E:\\Downloads\\Zed\\AI_comtool-rs",
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": cmd, "description": "test"},
        },
        ensure_ascii=False,
    )


def ps_payload(cmd: str) -> str:
    return json.dumps(
        {
            "hook_event_name": "PreToolUse",
            "tool_name": "PowerShell",
            "tool_input": {"command": cmd},
        },
        ensure_ascii=False,
    )


def write_payload(path: str, content: str) -> str:
    return json.dumps(
        {
            "hook_event_name": "PreToolUse",
            "tool_name": "Write",
            "tool_input": {"file_path": path, "content": content},
        },
        ensure_ascii=False,
    )


CASES = [
    # --- 必须拒绝：运行本项目 -------------------------------------------------
    ("cargo run", bash_payload("cargo run"), "DENY"),
    ("cargo run --release", bash_payload("cargo run --release"), "DENY"),
    (
        "cd 项目后 cargo run",
        bash_payload("cd E:/Downloads/Zed/AI_comtool-rs && cargo run"),
        "DENY",
    ),
    ("cargo run 后台运行", bash_payload("cargo run > /dev/null 2>&1 &"), "DENY"),
    ("直接执行 release exe", bash_payload("./target/release/comtool-rs.exe"), "DENY"),
    ("执行 debug exe", bash_payload("target/debug/comtool-rs.exe --help"), "DENY"),
    (
        "反斜杠路径启动 exe",
        bash_payload("powershell -c \"Start-Process 'target\\release\\comtool-rs.exe'\""),
        "DENY",
    ),
    ("taskkill 本项目进程", bash_payload("taskkill /IM comtool-rs.exe /F"), "DENY"),
    ("cargo run（大小写）", bash_payload("CARGO RUN"), "DENY"),
    # --- 必须拒绝：COM3 / 串口 ------------------------------------------------
    ("mode COM3", bash_payload("mode COM3: baud=115200"), "DENY"),
    ("echo 到 COM3", bash_payload("echo AT > COM3"), "DENY"),
    ("pyserial 打开 COM3", bash_payload("python -c \"import serial; serial.Serial('COM3')\""), "DENY"),
    ("pyserial 打开 COM4（不点名 COM3，仍拒）", bash_payload("python -c \"import serial; s=serial.Serial('COM4')\""), "DENY"),
    (
        ".NET SerialPort 打开",
        ps_payload("[System.IO.Ports.SerialPort]::new('COM3',9600)"),
        "DENY",
    ),
    ("禁用设备", ps_payload("Disable-PnpDevice -InstanceId 'USB\\VID_1A86' -Confirm:$false"), "DENY"),
    ("pnputil", ps_payload("pnputil /disable-device /instanceid X"), "DENY"),
    # --- 必须拒绝：批准文件与批准脚本 -----------------------------------------
    (
        "写批准文件",
        write_payload(os.path.expanduser("~") + "/.workbuddy/combot-run-approval", "99999999999"),
        "DENY",
    ),
    ("调用批准脚本", bash_payload("bash ~/.workbuddy/hooks/comtool-approve.sh 30"), "DENY"),
    ("echo 写批准文件", bash_payload("echo 9999999999 > ~/.workbuddy/combot-run-approval"), "DENY"),
    # --- 必须放行：正常开发命令 -----------------------------------------------
    ("cargo check", bash_payload("cargo check --all-targets"), "ALLOW"),
    ("cargo test", bash_payload("cargo test"), "ALLOW"),
    ("cargo fmt + clippy", bash_payload("cargo fmt && cargo clippy --all-targets"), "ALLOW"),
    (
        "git 状态",
        bash_payload("cd E:/Downloads/Zed/AI_comtool-rs && git status --short"),
        "ALLOW",
    ),
    ("列 target/release 目录", bash_payload("ls E:/Downloads/Zed/AI_comtool-rs/target/release"), "ALLOW"),
    ("读 Cargo.toml", bash_payload("cat E:/Downloads/Zed/AI_comtool-rs/Cargo.toml"), "ALLOW"),
    ("cargo test --test serial", bash_payload("cargo test --test serial"), "ALLOW"),
    (
        "只读端口枚举（GetPortNames）",
        ps_payload("[System.IO.Ports.SerialPort]::GetPortNames()"),
        "ALLOW",
    ),
    (
        "写文档（内容提到 cargo run / COM3）",
        write_payload(
            "E:/Downloads/Zed/AI_comtool-rs/AGENTS.md",
            "规则：未经批准不得 cargo run；不得占用 COM3。",
        ),
        "ALLOW",
    ),
    ("写普通源文件", write_payload("E:/Downloads/Zed/AI_comtool-rs/src/app.rs", "fn main() {}"), "ALLOW"),
    ("无关网络命令", bash_payload("curl -s http://localhost:8080/health"), "ALLOW"),
    # --- fail-closed ----------------------------------------------------------
    ("非法 JSON payload", "这不是 JSON，但里面写着 cargo run", "DENY"),
    ("空 payload", "", "DENY"),
    ("仅空白 payload", "   \n\t ", "DENY"),
]


def main() -> int:
    if not BASH:
        print("找不到 bash.exe（Git Bash）")
        return 2
    if not os.path.exists(GUARD):
        print("找不到守卫脚本：" + GUARD)
        return 2

    print("bash  : " + BASH)
    print("guard : " + GUARD)
    print("-" * 78)

    failed = []

    def check(name, raw, expect):
        got = run(raw)
        ok = got == expect
        print("%-4s %-36s expect=%-6s got=%s" % ("ok" if ok else "FAIL", name, expect, got))
        if not ok:
            failed.append((name, expect, got))

    # 无批准文件的状态
    if os.path.exists(GRANT):
        os.remove(GRANT)
    print("[批准文件不存在]")
    for name, raw, expect in CASES:
        check(name, raw, expect)

    # 有效批准：cargo run 放行，COM3 仍然拒绝
    print("[批准文件有效（+30 分钟）]")
    with open(GRANT, "w", encoding="utf-8") as fh:
        fh.write("%d\n" % (int(time.time()) + 1800))
    check("cargo run（已批准）", bash_payload("cargo run"), "ALLOW")
    check("COM3（已批准，仍拒）", bash_payload("mode COM3:"), "DENY")

    # 过期批准：回到拒绝
    print("[批准文件已过期]")
    with open(GRANT, "w", encoding="utf-8") as fh:
        fh.write("%d\n" % (int(time.time()) - 60))
    check("cargo run（已过期）", bash_payload("cargo run"), "DENY")

    # 无法解析的批准文件：拒绝
    print("[批准文件损坏]")
    with open(GRANT, "w", encoding="utf-8") as fh:
        fh.write("not-a-timestamp\n")
    check("cargo run（批准损坏）", bash_payload("cargo run"), "DENY")

    os.remove(GRANT)

    print("-" * 78)
    if failed:
        print("失败 %d 条：" % len(failed))
        for name, expect, got in failed:
            print("  - %s: expect=%s got=%s" % (name, expect, got))
        return 1
    print("全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
