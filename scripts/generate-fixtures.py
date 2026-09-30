#!/usr/bin/env python3
"""novel-reader 确定性夹具生成器（reading-core C-T2 / 卡片 rc2）。

两档 profile：

* ``smoke``    —— 小型夹具（每个文件 ≤ 128 KiB，集合总量 ≤ 256 KiB），可直接落盘到
                 ``tests/fixtures/`` 供 Rust 单测离线使用。
* ``pressure`` —— 100 MB / 1 GB 级 TXT 压力夹具，**只允许写入临时目录**
                 （如 ``/tmp/nr-pressure``），严禁提交进版本库。

确定性保证：同一 ``(profile, seed)`` 组合总是生成逐字节一致的文件；``manifest.tsv``
只包含内容事实（名称/字节数/sha256/类型/细节），因此跨机、跨次运行逐字节一致；
``manifest.json`` 额外带 ``generated_at_unix`` 等运行期元数据。

用法::

    python3 scripts/generate-fixtures.py --profile smoke    --seed 20260912 --output /tmp/nr-smoke
    python3 scripts/generate-fixtures.py --profile pressure --seed 20260912 --output /tmp/nr-pressure

压力档可选 ``--sizes``（默认 ``100mb,1gb``，单位按 1024 的幂解释）::

    python3 scripts/generate-fixtures.py --profile pressure --seed 20260912 --sizes 100mb --output /tmp/nr-quick
"""

from __future__ import annotations

import argparse
import codecs
import hashlib
import json
import platform
import sys
import time
from pathlib import Path

GENERATOR_VERSION = 1

# 与 crates/reading-source/src/lib.rs 的冻结常量保持一致（gate-0 §2.3）。
DEFAULT_WINDOW_BYTES = 64 * 1024
MAX_WINDOW_BYTES = 256 * 1024
# 与 crates/reading-source/src/fingerprint.rs 的抽样大小保持一致（仅写入 manifest 供核对）。
FINGERPRINT_SAMPLE_BYTES = 4096

LONG_LINE_BYTES = 130 * 1024
PRESSURE_POOL_BYTES = 1 << 20
DEFAULT_PRESSURE_SIZES = "100mb,1gb"

MANIFEST_HEADER = "# novel-reader fixtures manifest\n# name\tbytes\tsha256\tkind\tdetails\n"


class XorShift64:
    """确定性 PRNG（xorshift64*），与 Python 版本无关，保证跨机一致。"""

    MASK = 0xFFFFFFFFFFFFFFFF

    def __init__(self, seed: int) -> None:
        state = seed & self.MASK
        self.state = state or 0x9E3779B97F4A7C15

    def next_u64(self) -> int:
        x = self.state
        x ^= (x << 13) & self.MASK
        x ^= x >> 7
        x ^= (x << 17) & self.MASK
        self.state = x & self.MASK
        return (self.state * 0x2545F4914F6CDD1D) & self.MASK

    def below(self, bound: int) -> int:
        if bound <= 0:
            raise ValueError("bound 必须为正数")
        return self.next_u64() % bound


CJK_POOL = [chr(cp) for cp in range(0x4E00, 0x4E00 + 512)]
PUNCT_POOL = list("，。！？；：“”‘’（）《》")
ASCII_POOL = list("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .,;:!?-()[]")


def utf8_lead_len(byte: int) -> int:
    if byte < 0x80:
        return 1
    if byte >= 0xF0:
        return 4
    if byte >= 0xE0:
        return 3
    if byte >= 0xC0:
        return 2
    return 0  # 续接字节


def snap_to_char_boundary(data: bytes, index: int) -> int:
    """把下标回退到最近的字符合法边界（不会越过 0，也不会超过数据长度）。"""
    cut = min(index, len(data))
    while 0 < cut < len(data) and utf8_lead_len(data[cut]) == 0:
        cut -= 1
    return cut


def build_text_pool(rng: XorShift64, target_bytes: int) -> bytes:
    """按目标字节数生成确定性 UTF-8 文本池（含 CJK/ASCII/标点/换行）。

    返回值保证是**完整**的 UTF-8 文本：末尾不足一个字符的字节会被裁掉。
    """
    parts: list[str] = []
    total = 0
    while total < target_bytes:
        roll = rng.below(100)
        if roll < 55:
            ch = CJK_POOL[rng.below(len(CJK_POOL))]
        elif roll < 85:
            ch = ASCII_POOL[rng.below(len(ASCII_POOL))]
        elif roll < 95:
            ch = PUNCT_POOL[rng.below(len(PUNCT_POOL))]
        else:
            ch = "\n"
        parts.append(ch)
        total += len(ch.encode("utf-8"))
    data = "".join(parts).encode("utf-8")
    return data[: snap_to_char_boundary(data, target_bytes)]


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_bytes(outdir: Path, name: str, data: bytes) -> dict:
    path = outdir / name
    path.write_bytes(data)
    return {
        "name": name,
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }


def render_lines(rng: XorShift64, lines: int, separator: bytes = b"\n") -> bytes:
    chunks: list[bytes] = []
    for _ in range(lines):
        width = 8 + rng.below(48)
        line = build_text_pool(rng, width).decode("utf-8", "ignore").strip("\n")
        chunks.append(line.encode("utf-8"))
    return separator.join(chunks) + separator


def build_smoke_fixtures(outdir: Path, seed: int) -> list[dict]:
    """生成 smoke 档小型夹具，返回 (name, bytes, sha256, kind, details) 记录。"""
    records: list[dict] = []

    def add(name: str, data: bytes, kind: str, details: str = "-") -> None:
        info = write_bytes(outdir, name, data)
        info["kind"] = kind
        info["details"] = details
        records.append(info)

    add("empty.txt", b"", "empty")

    add("ascii-short.txt", b"Hello novel reader.\n", "ascii")

    rng = XorShift64(seed ^ 0x51A5_C0DE)
    add("utf8-mixed-lines.txt", render_lines(rng, 96), "text")

    rng = XorShift64(seed ^ 0x0C7F_11EE)
    add("utf8-crlf.txt", render_lines(rng, 48, separator=b"\r\n"), "crlf")

    add(
        "utf8-bom.txt",
        codecs.BOM_UTF8 + "带 BOM 的 UTF-8 文本\n第二行\n".encode("utf-8"),
        "bom-utf8",
        "bom_len=3",
    )

    # 边界探针：4096 个 ASCII 字节之后紧跟 3 字节汉字；再跟 4 字节字符。
    prefix = b"a" * 4096
    body = "中文测试四字节".encode("utf-8")
    emoji = "🙂".encode("utf-8")
    suffix = b"z" * 4096
    probe = prefix + body + emoji + b"\n" + suffix
    add(
        "utf8-boundary-probe.txt",
        probe,
        "boundary-probe",
        (
            f"char_start={len(prefix)};char_len=3"
            f";ascii_prefix={len(prefix)}"
            f";second_char_start={len(prefix) + 3};second_char_len=3"
            f";four_byte_start={len(prefix) + len(body)};four_byte_len={len(emoji)}"
        ),
    )

    # 单行超过默认窗口（64 KiB）的长行：用于验证窗口跨越与不整本读取。
    unit = "长行测试ABC中文字符串with ASCII ".encode("utf-8")
    unit_repeats = (LONG_LINE_BYTES - 1) // len(unit)
    filler = unit * unit_repeats
    filler = filler + b" " * (LONG_LINE_BYTES - 1 - len(filler))
    long_line_data = filler + b"\n"
    add(
        "utf8-long-line.txt",
        long_line_data,
        "long-line",
        f"line_bytes={LONG_LINE_BYTES - 1};newline_count=1",
    )

    # 尾部被截断的多字节序列（文件以不完整字符结尾）。
    truncated = "截断尾部".encode("utf-8") + "中".encode("utf-8")[:2]
    add("utf8-truncated-tail.txt", truncated, "truncated-tail", "kept_bytes_of_last_char=2")

    add("utf8-four-byte.txt", (b"x" * 64) + "𠀋🙂中".encode("utf-8") + b"\n", "four-byte", "four_byte_len=4")

    return records


def build_pressure_fixtures(outdir: Path, seed: int, sizes: list[tuple[str, int]]) -> list[dict]:
    """生成压力档大文件（确定性、逐块校验 UTF-8 合法），返回记录列表。"""
    records: list[dict] = []
    pool_rng = XorShift64(seed ^ 0x9E53_0001)
    pool = build_text_pool(pool_rng, PRESSURE_POOL_BYTES)
    boundaries = [index for index, byte in enumerate(pool) if utf8_lead_len(byte) != 0]
    if not boundaries:
        raise RuntimeError("压力夹具文本池未找到字符边界")

    for label, size in sizes:
        name = f"pressure-{label}.txt"
        path = outdir / name
        rng = XorShift64(seed ^ (size & 0xFFFFFFFF))
        decoder = codecs.getincrementaldecoder("utf-8")()
        written = 0
        try:
            with path.open("wb") as handle:
                while written < size:
                    remaining = size - written
                    rotation = boundaries[rng.below(len(boundaries))]
                    block = pool[rotation:] + pool[:rotation]
                    if len(block) <= remaining:
                        chunk = block
                    else:
                        cut = snap_to_char_boundary(block, remaining)
                        chunk = block[:cut] + b" " * (remaining - cut)
                    handle.write(chunk)
                    decoder.decode(chunk)
                    written += len(chunk)
                decoder.decode(b"", final=True)
        except Exception:
            # 失败时不得留下半成品大文件。
            path.unlink(missing_ok=True)
            raise
        actual = path.stat().st_size
        if actual != size:
            raise RuntimeError(f"{name} 大小不符：期望 {size}，实际 {actual}")
        records.append(
            {
                "name": name,
                "bytes": actual,
                "sha256": sha256_of(path),
                "kind": "pressure",
                "details": f"seed={seed};pool_bytes={len(pool)};utf8=valid",
            }
        )
    return records


def write_manifest(outdir: Path, profile: str, seed: int, records: list[dict], sizes_note: str) -> Path:
    ordered = sorted(records, key=lambda item: item["name"])
    total_bytes = sum(item["bytes"] for item in ordered)

    tsv = outdir / "manifest.tsv"
    lines = [
        "# novel-reader fixtures manifest",
        f"# profile={profile} seed={seed}",
        (
            f"# default_window_bytes={DEFAULT_WINDOW_BYTES} max_window_bytes={MAX_WINDOW_BYTES}"
            f" fingerprint_sample_bytes={FINGERPRINT_SAMPLE_BYTES}"
        ),
        MANIFEST_HEADER.rstrip("\n").split("\n")[-1],
    ]
    for item in ordered:
        lines.append(
            "\t".join(
                [
                    item["name"],
                    str(item["bytes"]),
                    item["sha256"],
                    item["kind"],
                    item["details"],
                ]
            )
        )
    tsv.write_text("\n".join(lines) + "\n", encoding="utf-8")

    manifest = {
        "generator": "scripts/generate-fixtures.py",
        "generator_version": GENERATOR_VERSION,
        "profile": profile,
        "seed": seed,
        "generated_at_unix": int(time.time()),
        "python_version": platform.python_version(),
        "default_window_bytes": DEFAULT_WINDOW_BYTES,
        "max_window_bytes": MAX_WINDOW_BYTES,
        "fingerprint_sample_bytes": FINGERPRINT_SAMPLE_BYTES,
        "file_count": len(ordered),
        "total_bytes": total_bytes,
        "sizes_note": sizes_note,
        "files": [
            {
                "name": item["name"],
                "bytes": item["bytes"],
                "sha256": item["sha256"],
                "kind": item["kind"],
                "details": item["details"],
            }
            for item in ordered
        ],
        "notes": (
            "确定性：同一 (profile, seed) 生成逐字节一致的文件；manifest.tsv 只含内容事实"
            "（跨次运行逐字节一致），manifest.json 额外含 generated_at_unix 等运行期元数据。"
            "pressure 档大文件仅用于临时目录，禁止提交版本库。"
        ),
        "manifest_tsv": "manifest.tsv",
    }
    json_path = outdir / "manifest.json"
    json_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return json_path


def parse_sizes(spec: str) -> list[tuple[str, int]]:
    units = {
        "": 1,
        "b": 1,
        "kb": 1024,
        "kib": 1024,
        "mb": 1024 * 1024,
        "mib": 1024 * 1024,
        "gb": 1024 * 1024 * 1024,
        "gib": 1024 * 1024 * 1024,
    }
    parsed: list[tuple[str, int]] = []
    for raw in spec.split(","):
        token = raw.strip().lower()
        if not token:
            continue
        number = ""
        unit = ""
        for ch in token:
            if ch.isdigit():
                number += ch
            else:
                unit += ch
        if not number or unit not in units:
            raise SystemExit(f"无法解析 --sizes 项: {raw!r}")
        parsed.append((f"{number}{unit}", int(number) * units[unit]))
    if not parsed:
        raise SystemExit("--sizes 不能为空")
    return parsed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="novel-reader 确定性夹具生成器")
    parser.add_argument("--profile", choices=["smoke", "pressure"], required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--sizes", default=DEFAULT_PRESSURE_SIZES, help="pressure 档文件尺寸，如 100mb,1gb")
    args = parser.parse_args(argv)

    outdir: Path = args.output
    outdir.mkdir(parents=True, exist_ok=True)

    if args.profile == "smoke":
        sizes_note = f"smoke: 每个文件 ≤ {LONG_LINE_BYTES} 字节"
        records = build_smoke_fixtures(outdir, args.seed)
    else:
        sizes = parse_sizes(args.sizes)
        sizes_note = "pressure: " + ",".join(f"{label}={size}" for label, size in sizes)
        records = build_pressure_fixtures(outdir, args.seed, sizes)

    manifest_path = write_manifest(outdir, args.profile, args.seed, records, sizes_note)

    print(f"profile={args.profile} seed={args.seed} output={outdir}")
    print(f"files={len(records)} total_bytes={sum(item['bytes'] for item in records)}")
    for item in sorted(records, key=lambda entry: entry["name"]):
        print(f"  {item['name']}\t{item['bytes']}\t{item['sha256']}\t{item['kind']}")
    print(f"manifest={manifest_path}")
    print(f"manifest_tsv={manifest_path.with_name('manifest.tsv')}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
