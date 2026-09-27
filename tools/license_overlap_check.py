#!/usr/bin/env python3
"""许可合规 · 与上游 `SerialGUI-rs` 的代码重合度实测（对应 `NOTICE.md` §3）。

## 为什么要这个脚本

`NOTICE.md` 里的结论「当前代码在表达层面没有残留上游实现」如果只是**断言**，
下次有人（包括未来的 AI 会话）就没法复核。这个脚本把它变成**可复现的数字**：
把上游源码与本地仓库任意一个 `rev` 的源码逐行比较，报出逐字相同的行数与它们**具体是什么**。

## 口径（要改口径就改这里，别在别处口算）

* 只比较 `.rs` 文件；
* 每行 `strip()` 去首尾空白；
* **丢弃长度 < 12 的行**——`}`, `Ok(())`, `use x;` 这类短行在任何 Rust 项目里都会重复，
  算进来只会得到一堆没有判别力的数字；
* **丢弃注释行与属性行**（`//`, `#`, `/*`, `*` 开头）——注释是表达，但这里的目的是看
  "实现"有没有重合；
* 取**集合交集**（同一行出现多次只记一次）。

## 用法

    python tools/license_overlap_check.py                 # 默认比 HEAD
    python tools/license_overlap_check.py --rev ecfb6e8   # 比历史里的某个提交
    python tools/license_overlap_check.py --list          # 逐条打印重合行（默认只打印前若干条）

需要本机存在上游参考副本（默认 `SerialGUI-rs/`，可用 `--upstream` 指定）。
该目录在 `.gitignore` 里，**不随仓库分发**——所以在没有副本的机器上这个脚本会直接报错退出，
这是有意的：不要为了让它跑起来去把上游拉进来。
"""

from __future__ import annotations

import argparse
import io
import os
import subprocess
import sys

MIN_LINE_LEN = 12


def substantive_lines(text: str) -> set[str]:
    """抽出"实质代码行"：去空白、够长、非注释。"""
    out: set[str] = set()
    for line in text.splitlines():
        s = line.strip()
        if len(s) < MIN_LINE_LEN:
            continue
        if s.startswith(("//", "#", "/*", "*")):
            continue
        out.add(s)
    return out


def upstream_lines(root: str) -> tuple[set[str], int]:
    files: list[str] = []
    for dirpath, _, names in os.walk(os.path.join(root, "src")):
        files += [os.path.join(dirpath, n) for n in names if n.endswith(".rs")]
    acc: set[str] = set()
    for path in files:
        with io.open(path, encoding="utf-8", errors="replace") as fh:
            acc |= substantive_lines(fh.read())
    return acc, len(files)


def tracked_rs(rev: str) -> list[str]:
    proc = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", rev],
        capture_output=True,
        text=True,
        check=True,
    )
    return [p for p in proc.stdout.split() if p.endswith(".rs")]


def show(rev: str, path: str) -> str:
    """`git show <rev>:<path>`；路径不存在（历史里删掉的文件）时返回空串。"""
    proc = subprocess.run(
        ["git", "show", f"{rev}:{path}"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return proc.stdout


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rev", default="HEAD", help="要比对的提交（默认 HEAD）")
    ap.add_argument("--upstream", default="SerialGUI-rs", help="上游参考副本目录")
    ap.add_argument("--list", action="store_true", help="逐条打印全部重合行")
    args = ap.parse_args()

    if not os.path.isdir(args.upstream):
        print(
            f"找不到上游参考副本 {args.upstream}/。\n"
            "它是 .gitignore 里的参考目录，不随仓库分发；本脚本只在有它的机器上有意义。",
            file=sys.stderr,
        )
        return 2

    up, up_files = upstream_lines(args.upstream)
    print(f"上游 {up_files} 个 .rs 文件，去重后的实质代码行 {len(up)} 条")

    our_files = tracked_rs(args.rev)
    hits: list[tuple[str, str]] = []
    per_file: dict[str, int] = {}
    for path in our_files:
        common = substantive_lines(show(args.rev, path)) & up
        if common:
            per_file[path] = len(common)
            hits += [(path, s) for s in sorted(common)]

    print(f"--- {args.rev}：{len(our_files)} 个 .rs 文件，与上游逐字相同的实质代码行 {len(hits)} 条")
    for path, n in sorted(per_file.items(), key=lambda kv: -kv[1]):
        print(f"      {path}: {n}")

    print("--- 重合行清单" + ("" if args.list else "（默认只显示前 40 条，--list 看全部）"))
    for path, s in hits[: None if args.list else 40]:
        print(f"    [{path}] {s}")

    print(
        "\n提示：判断这些行是'框架强制唯一的写法'还是'被抄的实现'，只能人看。\n"
        "      `ui.separator();` / `fn default() -> Self {` / `use eframe::egui;` 这类是前者。"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
