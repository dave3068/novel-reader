#!/usr/bin/env python3
"""RC3 编码夹具与频率表生成器（reading-source / encoding.rs 配套）。

用途
====
1. `tables`  子命令：从两个**外部频率表**推导 GBK/Big5「常用字码表」并打印可直接
   粘进 `crates/reading-source/src/encoding.rs` 的 Rust 常量数组（升序 u16，供
   `binary_search` 命中判定）。
2. `fixtures` 子命令：写出 RC3 的编码夹具到 `tests/fixtures/encoding/`，并同步写出
   `manifest.tsv`（名称 / 字节数 / sha256 / 期望编码 / 期望可信度 / 细节）。

输入（外部频率表，需先用 curl 取到本地；本脚本不联网）
----------------------------------------------------
- 简体（Jun Da / MTSU 现代汉语字频表，按频次降序，5000 字）：
  `https://raw.githubusercontent.com/tuokaikyle/word-frequency-lists/main/zh/mtsu_5000.tsv`
  sha256 = 52cbfeb94386ac62da872d152f76c380eeb68e1a370234b8b902fa90be808512
  每行格式：`rank<TAB>char<TAB>count<TAB>percent<TAB>pinyin<TAB>definition`
- 繁体（繁体字频表，按频次降序，13060 行）：
  `https://cdn.jsdelivr.net/gh/agj/3000-traditional-hanzi@master/data/external/frequency.txt`
  sha256 = ab47e6c79126bd98b5f716e7fdc41e00d67cfb265eed79a08ac836ef84e68c62
  每行格式：`char<TAB>count<TAB>strokes`

命令示例
--------
    python3 crates/reading-source/tests/generate-encoding-fixtures.py tables \
        --simplified /tmp/mtsu_5000.tsv --traditional /tmp/agj_freq2.txt \
        --limit 512 > /tmp/nr-encoding-tables.rs

    python3 crates/reading-source/tests/generate-encoding-fixtures.py fixtures \
        --simplified /tmp/mtsu_5000.tsv --traditional /tmp/agj_freq2.txt \
        --output tests/fixtures/encoding

确定性
======
输出只依赖：两个输入文件的**前 limit 个可编码字符**、本文件内置的夹具文本与常量、
以及固定顺序的构造规则；不含时间戳、随机数或环境信息（manifest 里也不写生成时间，
以便逐字节复现）。
"""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

# --------------------------------------------------------------------------------------
# 常量（与 encoding.rs 保持一致；manifest 里做跨语言校验）
# --------------------------------------------------------------------------------------

DETECTOR_VERSION = 1
DETECT_HEAD_BYTES = 32 * 1024
DETECT_SAMPLE_BYTES = 16 * 1024
DETECT_MAX_SAMPLED_BYTES = DETECT_HEAD_BYTES + 2 * DETECT_SAMPLE_BYTES
SAMPLE_BYTES_DEFAULT = DETECT_SAMPLE_BYTES

# 期望编码的稳定标识（与 TextEncoding::as_str 一致）。
E_UTF8 = "UTF-8"
E_UTF8_BOM = "UTF-8-BOM"
E_UTF16LE = "UTF-16LE"
E_UTF16BE = "UTF-16BE"
E_GB18030 = "GB18030"
E_GBK = "GBK"
E_BIG5 = "Big5"

# --------------------------------------------------------------------------------------
# 内置夹具文本（简体 / 繁体各一段；均为原创小说风格段落，只使用目标编码可表示的字符）
# --------------------------------------------------------------------------------------

SIMPLIFIED_SENTENCES = [
    "第一章 夜雨孤灯",
    "雨点敲在青石板上，发出细碎的声响。少年把油纸伞收在门边，抖落肩上的水珠，抬脚走进那间窄小的书铺。",
    "掌柜正伏在柜台上打盹，听见动静，才慢慢抬起头来，眯着眼打量来人：“客官要寻什么书？”",
    "“我想找一本旧书。”少年把手伸进怀里，摸出半枚铜钱，“书名叫《山河录》，掌柜可有印象？”",
    "掌柜的脸色微微一变，随即又恢复如常：“《山河录》……二十年前便已绝版了。”",
    "少年并不意外，只是点了点头，把铜钱轻轻放在柜面上。",
    "屋外的雨越下越密，檐下挂着一盏孤灯，灯光落在水洼里，被风一吹，散成一片碎金。",
    "“绝版的书，未必就找不到。”少年说，“我父亲临终前嘱咐我，一定要把这本书寻回来。”",
    "掌柜沉默良久，终于从柜台底下抽出一个落满灰尘的木匣。",
    "“书是不在了。”他低声道，“可留着一张纸，写着当年那位抄书人的去处。”",
    "少年接过那张薄纸，只见上面写着十个字：向北三里，问渡口老翁。",
    "他收起纸，向掌柜深深鞠了一躬，重新撑开油纸伞，走进了雨里。",
]

TRADITIONAL_SENTENCES = [
    "第一章 夜雨孤燈",
    "雨點敲在青石板上，發出細碎的聲響。少年把油紙傘收在門邊，抖落肩上的水珠，抬腳走進那間窄小的書鋪。",
    "掌櫃正伏在櫃檯上打盹，聽見動靜，才慢慢抬起頭來，瞇著眼打量來人：「客官要尋什麼書？」",
    "「我想找一本舊書。」少年把手伸進懷裡，摸出半枚銅錢，「書名叫《山河錄》，掌櫃可有印象？」",
    "掌櫃的臉色微微一變，隨即又恢復如常：「《山河錄》……二十年前便已絕版了。」",
    "少年並不意外，只是點了點頭，把銅錢輕輕放在櫃面上。",
    "屋外的雨越下越密，簷下掛著一盞孤燈，燈光落在水窪裡，被風一吹，散成一片碎金。",
    "「絕版的書，未必就找不到。」少年說，「我父親臨終前囑咐我，一定要把這本書尋回來。」",
    "掌櫃沉默良久，終於從櫃檯底下抽出一個落滿灰塵的木匣。",
    "「書是不在了。」他低聲道，「可留著一張紙，寫著當年那位抄書人的去處。」",
    "少年接過那張薄紙，只見上面寫著十個字：向北三里，問渡口老翁。",
    "他收起紙，向掌櫃深深鞠了一躬，重新撐開油紙傘，走進了雨裡。",
]

ASCII_TEXT = (
    "Chapter 1 - A Rainy Night\n"
    "It was a dark and stormy night; the rain fell in torrents, except at occasional\n"
    "intervals, when it was checked by a violent gust of wind which swept up the\n"
    "streets, rattling along the house-tops and fiercely agitating the scanty flame\n"
    "of the lamps that struggled against the darkness.\n"
)
ASCII_TEXT = ASCII_TEXT * 4

# 垃圾字节：控制字符 + 非法序列，任何候选编码都不成立。
GARBAGE_BYTES = bytes(
    [0x00, 0x01, 0x02, 0x03, 0x1F, 0x7F, 0xFF, 0xFE, 0x80, 0x81, 0x9F, 0x0B, 0x0C, 0xBF, 0xC0, 0xED]
) * 12

# GB18030 四字节专用样本（这些字符不在 GBK 中，只能由 GB18030 表示）。
GB18030_ONLY_CHARS = ["\U0002000B", "\U0002003E", "\U0002004D", "\U0002006F", "\U0002008D"]


def repeated_block(sentences: list[str], target_chars: int) -> str:
    """按固定顺序重复句子直到达到目标字符数（确定性）。"""
    out: list[str] = []
    total = 0
    index = 0
    while total < target_chars:
        line = sentences[index % len(sentences)]
        out.append(line)
        total += len(line)
        index += 1
    return "\n".join(out) + "\n"


# --------------------------------------------------------------------------------------
# 外部频率表读取
# --------------------------------------------------------------------------------------


def read_simplified_chars(path: Path, limit: int) -> list[str]:
    chars: list[str] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        fields = line.split("\t")
        if len(fields) < 2 or len(fields[1]) != 1:
            continue
        char = fields[1]
        try:
            char.encode("gb18030")
        except UnicodeEncodeError:
            continue
        chars.append(char)
        if len(chars) >= limit:
            break
    return chars


def read_traditional_chars(path: Path, limit: int) -> list[str]:
    chars: list[str] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        fields = line.split("\t")
        if not fields or len(fields[0]) != 1:
            continue
        char = fields[0]
        try:
            char.encode("big5")
        except UnicodeEncodeError:
            continue
        chars.append(char)
        if len(chars) >= limit:
            break
    return chars


def common_codes(chars: list[str], codec: str) -> list[int]:
    """字符 -> 两字节码点（大端 u16，升序去重）。"""
    codes: set[int] = set()
    for char in chars:
        raw = char.encode(codec)
        if len(raw) != 2:
            continue
        codes.add((raw[0] << 8) | raw[1])
    return sorted(codes)


def rust_u16_array(name: str, codes: list[int], doc: str, per_line: int = 8) -> str:
    lines = [f"/// {doc}", f"pub const {name}: [u16; {len(codes)}] = ["]
    for start in range(0, len(codes), per_line):
        chunk = ", ".join(f"0x{code:04X}" for code in codes[start : start + per_line])
        lines.append(f"    {chunk},")
    lines.append("];")
    return "\n".join(lines)


# --------------------------------------------------------------------------------------
# 模糊（GBK/Big5 近似并列）夹具构造
# --------------------------------------------------------------------------------------


def gbk_structurally_valid_big5(code: int) -> bool:
    lead = code >> 8
    trail = code & 0xFF
    return 0xA1 <= lead <= 0xF9 and (0x40 <= trail <= 0x7E or 0xA1 <= trail <= 0xFE)


def gbk_structurally_valid(code: int) -> bool:
    lead = code >> 8
    trail = code & 0xFF
    return 0x81 <= lead <= 0xFE and 0x40 <= trail <= 0xFE and trail != 0x7F


def ambiguous_block(gbk_codes: list[int], big5_codes: list[int], pairs: int = 72) -> bytes:
    """构造 GBK 与 Big5 结构均合法、且两边命中数完全相等的字节块。

    取 GBK 码表中「不是 Big5 码表成员」的码、与 Big5 码表中「不是 GBK 码表成员」的码
    各 `pairs` 个，交替拼接：两种解释下的多字节序列数、命中数与结构合法率都相同，
    因此统计评分并列 —— 用来构造「低可信度、需要用户指定编码」的场景。
    """
    gbk_set = set(gbk_codes)
    big5_set = set(big5_codes)
    only_gbk = [
        code
        for code in gbk_codes
        if code not in big5_set and gbk_structurally_valid_big5(code)
    ]
    only_big5 = [
        code
        for code in big5_codes
        if code not in gbk_set and gbk_structurally_valid(code)
    ]
    if len(only_gbk) < pairs or len(only_big5) < pairs:
        raise SystemExit(
            f"码表不足以构造模糊夹具: only_gbk={len(only_gbk)} only_big5={len(only_big5)} need={pairs}"
        )
    out = bytearray()
    for index in range(pairs):
        for code in (only_gbk[index], only_big5[index]):
            out.append(code >> 8)
            out.append(code & 0xFF)
    return bytes(out)


# --------------------------------------------------------------------------------------
# fixtures 子命令
# --------------------------------------------------------------------------------------


def encode_or_fail(text: str, codec: str, label: str) -> bytes:
    """逐字符校验可编码性（失败时点名具体字符，便于修正夹具文本）。"""
    for char in text:
        try:
            char.encode(codec)
        except UnicodeEncodeError as error:
            raise SystemExit(f"{label}: 字符 {char!r} (U+{ord(char):04X}) 无法用 {codec} 编码: {error}")
    raw = text.encode(codec)
    assert raw.decode(codec) == text, f"{label}: {codec} 往返不一致"
    return raw


def build_fixtures(simplified: Path, traditional: Path, limit: int) -> dict[str, bytes]:
    simplified_chars = read_simplified_chars(simplified, limit)
    traditional_chars = read_traditional_chars(traditional, limit)
    gbk_codes = common_codes(simplified_chars, "gb18030")
    big5_codes = common_codes(traditional_chars, "big5")

    simplified_text = repeated_block(SIMPLIFIED_SENTENCES, 1400)
    traditional_text = repeated_block(TRADITIONAL_SENTENCES, 1400)

    fixtures: dict[str, bytes] = {}

    # 1) UTF-8 BOM
    fixtures["utf8-bom.txt"] = b"\xef\xbb\xbf" + encode_or_fail(simplified_text, "utf-8", "utf8-bom")
    # 2) UTF-8 无 BOM
    fixtures["utf8-plain.txt"] = encode_or_fail(simplified_text, "utf-8", "utf8-plain")
    # 3/4) UTF-16 LE / BE（含 BOM）
    utf16le = encode_or_fail(simplified_text, "utf-16-le", "utf16le")
    fixtures["utf16le-bom.txt"] = b"\xff\xfe" + utf16le

    # BE 必须逐码元翻转（不能整体翻转字节，否则码元顺序也反了）。
    utf16be = b"".join(utf16le[index : index + 2][::-1] for index in range(0, len(utf16le), 2))
    fixtures["utf16be-bom.txt"] = b"\xfe\xff" + utf16be
    assert fixtures["utf16be-bom.txt"][2:].decode("utf-16-be") == simplified_text
    assert fixtures["utf16le-bom.txt"][2:].decode("utf-16-le") == simplified_text

    # 5) GBK（两字节，不含 GB18030 四字节序列）
    gbk_bytes = encode_or_fail(simplified_text, "gbk", "gbk")
    fixtures["gbk.txt"] = gbk_bytes

    # 6) GB18030（简体正文 + 只能在 GB18030 表示的四字节字符）
    extra = "".join(GB18030_ONLY_CHARS)
    for char in GB18030_ONLY_CHARS:
        try:
            char.encode("gbk")
        except UnicodeEncodeError:
            continue
        raise SystemExit(f"GB18030 专用字符 {char!r} 竟然可以被 GBK 编码，夹具前提不成立")
    gb18030_text = simplified_text + extra + "\n" + extra + "\n"
    gb18030_bytes = encode_or_fail(gb18030_text, "gb18030", "gb18030")
    assert gb18030_bytes.count(GB18030_ONLY_CHARS[0].encode("gb18030")) >= 1
    fixtures["gb18030.txt"] = gb18030_bytes

    # 7) Big5（繁体正文）
    fixtures["big5.txt"] = encode_or_fail(traditional_text, "big5", "big5")

    # 8) GBK/Big5 近似并列（低可信度）
    fixtures["ambiguous-gbk-big5.bin"] = ambiguous_block(gbk_codes, big5_codes)

    # 9) 纯 ASCII（任何候选编码解码结果一致）
    fixtures["ascii-only.txt"] = ASCII_TEXT.encode("ascii")

    # 10) 垃圾字节（无候选成立）
    fixtures["garbage.bin"] = GARBAGE_BYTES

    # 11) UTF-16LE BOM + 实际 BE 载荷（BOM 与内容矛盾 → 低可信度并给出替代）
    fixtures["utf16le-bom-be-payload.txt"] = b"\xff\xfe" + utf16be

    # 12) 空文件
    fixtures["empty.txt"] = b""

    # 前提自检：进入统计评分的夹具必须**不是**合法 UTF-8（否则走不到 GBK/Big5 评分）。
    for name in ("gbk.txt", "gb18030.txt", "big5.txt", "ambiguous-gbk-big5.bin", "garbage.bin"):
        try:
            fixtures[name].decode("utf-8")
        except UnicodeDecodeError:
            continue
        raise SystemExit(f"{name} 竟是合法 UTF-8，无法用于统计评分夹具")
    # 空文件与 ASCII 夹具必须是合法 UTF-8。
    for name in ("utf8-bom.txt", "utf8-plain.txt", "ascii-only.txt", "empty.txt"):
        fixtures[name].decode("utf-8")
    return fixtures


EXPECTATIONS = [
    # (文件名, 期望编码, 期望可信度, 是否要求提示确认, details)
    ("utf8-bom.txt", E_UTF8_BOM, "high", "false", "bom_len=3"),
    ("utf8-plain.txt", E_UTF8, "high", "false", "bom_len=0"),
    ("utf16le-bom.txt", E_UTF16LE, "high", "false", "bom_len=2"),
    ("utf16be-bom.txt", E_UTF16BE, "high", "false", "bom_len=2"),
    ("gbk.txt", E_GBK, "high", "false", "bom_len=0;compatible=GB18030"),
    ("gb18030.txt", E_GB18030, "high", "false", "bom_len=0;four_byte=1"),
    ("big5.txt", E_BIG5, "high", "false", "bom_len=0"),
    ("ambiguous-gbk-big5.bin", E_GBK, "low", "true", "runner_up=Big5"),
    ("ascii-only.txt", E_UTF8, "high", "false", "bom_len=0;ascii_only=1"),
    ("garbage.bin", E_UTF8, "low", "true", "no_candidate_valid=1"),
    ("utf16le-bom-be-payload.txt", E_UTF16LE, "low", "true", "alternative=UTF-16BE"),
    ("empty.txt", E_UTF8, "low", "false", "empty=1"),
]


def write_fixtures(simplified: Path, traditional: Path, limit: int, output: Path) -> int:
    fixtures = build_fixtures(simplified, traditional, limit)
    output.mkdir(parents=True, exist_ok=True)
    rows: list[str] = []
    for name, expected, confidence, confirm, details in EXPECTATIONS:
        if name not in fixtures:
            raise SystemExit(f"缺少夹具 {name}")
        raw = fixtures[name]
        path = output / name
        path.write_bytes(raw)
        rows.append(
            "\t".join(
                [
                    name,
                    str(len(raw)),
                    hashlib.sha256(raw).hexdigest(),
                    expected,
                    confidence,
                    confirm,
                    details,
                ]
            )
        )
    header = [
        "# novel-reader encoding fixtures manifest",
        "# generator=crates/reading-source/tests/generate-encoding-fixtures.py",
        f"# detector_version={DETECTOR_VERSION}",
        f"# detect_head_bytes={DETECT_HEAD_BYTES} detect_sample_bytes={DETECT_SAMPLE_BYTES}"
        f" detect_max_sampled_bytes={DETECT_MAX_SAMPLED_BYTES}",
        f"# gbk_table_limit={limit} big5_table_limit={limit}",
        "# name\tbytes\tsha256\texpected_encoding\texpected_confidence\texpects_confirmation\tdetails",
    ]
    manifest = "\n".join(header + rows) + "\n"
    (output / "manifest.tsv").write_text(manifest, encoding="utf-8")
    print(f"wrote {len(rows)} fixtures + manifest.tsv to {output}")
    for row in rows:
        print("  " + row)
    return 0


def print_tables(simplified: Path, traditional: Path, limit: int) -> int:
    simplified_chars = read_simplified_chars(simplified, limit)
    traditional_chars = read_traditional_chars(traditional, limit)
    print(f"// simplified chars={len(simplified_chars)} gbk codes={len(common_codes(simplified_chars, 'gb18030'))}")
    print(f"// traditional chars={len(traditional_chars)} big5 codes={len(common_codes(traditional_chars, 'big5'))}")
    print(render_tables(simplified, traditional, limit))
    print()
    print(f"// sha256(simplified input) = {hashlib.sha256(simplified.read_bytes()).hexdigest()}")
    print(f"// sha256(traditional input) = {hashlib.sha256(traditional.read_bytes()).hexdigest()}")
    return 0


TABLE_BEGIN = "// @@begin generated tables@@"
TABLE_END = "// @@end generated tables@@"


def inject_tables(path: Path, payload: str) -> int:
    """把码表常量写入目标文件的两个标记之间（幂等，可重复执行）。"""
    text = path.read_text(encoding="utf-8")
    if TABLE_BEGIN not in text or TABLE_END not in text:
        raise SystemExit(f"{path} 缺少码表标记 {TABLE_BEGIN!r} / {TABLE_END!r}")
    start = text.index(TABLE_BEGIN) + len(TABLE_BEGIN)
    end = text.index(TABLE_END)
    if end < start:
        raise SystemExit(f"{path} 码表标记顺序错误")
    body = payload.strip("\n")
    new_text = f"{text[:start]}\n{body}\n{text[end:]}"
    path.write_text(new_text, encoding="utf-8")
    print(f"injected tables ({len(body.splitlines())} lines) into {path}")
    return 0


def render_tables(simplified: Path, traditional: Path, limit: int) -> str:
    simplified_chars = read_simplified_chars(simplified, limit)
    traditional_chars = read_traditional_chars(traditional, limit)
    gbk_codes = common_codes(simplified_chars, "gb18030")
    big5_codes = common_codes(traditional_chars, "big5")
    return "\n".join(
        [
            rust_u16_array(
                "GBK_COMMON_CODES",
                gbk_codes,
                "GBK/GB18030 常用字两字节码表（大端 u16，升序；由字频表前 N 个可编码字符推导）。",
            ),
            "",
            rust_u16_array(
                "BIG5_COMMON_CODES",
                big5_codes,
                "Big5 常用字两字节码表（大端 u16，升序；由繁体字频表前 N 个可编码字符推导）。",
            ),
        ]
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="RC3 编码夹具与频率表生成器")
    sub = parser.add_subparsers(dest="command", required=True)

    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--simplified", type=Path, required=True, help="简体字频表（MTSU 5000）")
    common.add_argument("--traditional", type=Path, required=True, help="繁体字频表")
    common.add_argument("--limit", type=int, default=512, help="每套码表取前 N 个可编码字符")

    tables = sub.add_parser("tables", parents=[common], help="打印/注入 Rust 码表常量")
    tables.add_argument(
        "--inject",
        type=Path,
        default=None,
        help="把码表写入目标文件 `@@begin generated tables@@` / `@@end generated tables@@` 之间",
    )

    fixtures = sub.add_parser("fixtures", parents=[common], help="写出编码夹具与 manifest")
    fixtures.add_argument("--output", type=Path, required=True, help="输出目录")

    args = parser.parse_args(argv)
    for path in (args.simplified, args.traditional):
        if not path.is_file():
            raise SystemExit(f"缺少输入文件: {path}")
    if args.command == "tables":
        if args.inject is not None:
            return inject_tables(args.inject, render_tables(args.simplified, args.traditional, args.limit))
        return print_tables(args.simplified, args.traditional, args.limit)
    return write_fixtures(args.simplified, args.traditional, args.limit, args.output)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
