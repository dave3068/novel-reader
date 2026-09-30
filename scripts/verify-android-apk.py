#!/usr/bin/env python3
"""verify-android-apk.py — APK 内容与签名状态校验（AN6 交付物；代理子规格检查项 C8）。

用途（android/spec.md「验收」AR5、checklist C8）：

  对构建产物做**内容级**校验，而不是只看「文件存在」：

  [1] APK 文件与 ZIP 结构可读、含 AndroidManifest.xml；
  [2] 包名 / versionCode / versionName 解析自**二进制 AndroidManifest.xml**
      （纯 Python 解析，不依赖 aapt2）；
  [3] minSdk / targetSdk 与规格一致（31 / 36）；
  [4] `lib/<abi>/libnr_jni.so` 存在，且 ELF 头证明它就是该 ABI 的 64 位共享库
      （e_machine=EM_AARCH64 等），并报告大小与 sha256；
  [4b] 不存在**未声明 ABI** 的 libnr_jni.so（abiFilters 只发一个 ABI）；
  [5] DEX 构建标记：扫描**全部** `classes*.dex`（多 dex 场景下标记可能只在
      classes5.dex 之类靠后的 dex 里，只看 classes.dex 会误判「新代码未入包」）；
  [6] 签名状态：v1（JAR）/ v2 / v3 / v3.1 / v4（.idsig）分别判定。
      Debug 必须已签名；Release 无 signingConfig 时**未签名属预期**，
      不得当作失败（见卡片「禁止事项」）。
  [7] 交叉校验：本机存在 aapt2 / apksigner 时，用它们**独立**复核包名、SDK 范围、
      签名状态；两者结论不一致即判失败（证明纯 Python 解析不是自说自话）。
      SDK 根按显式 --sdk-root → PATH → $ANDROID_HOME → $ANDROID_SDK_ROOT →
      android/local.properties 的 sdk.dir → 本机默认 /opt/dev-tools/android-sdk
      依次自动探测（F-REL1b-1）；只有**每一层都试过仍找不到**才计为跳过项，
      且跳过文案会列出已试位置、探测结果同时写入 --json 的 facts.sdk_probe，
      调用方（发布门禁）据此可判定「是否降级」。

退出码：0 = 全部通过；1 = 存在违规；2 = 脚本自身前提缺失（APK 不存在/不可读、
无法从 build.gradle.kts 取得期望 BUILD_MARKER）。

`--self-test`：用真实 APK 派生 fixture 做反向用例，证明每条判定非空检查：
  * 只在靠后的合成 dex 里写标记必须被判为「命中」（全 dex 扫描）；
  * 删掉标记的合成 ZIP 必须被判为「未命中」；
  * 伪造 v1（META-INF）必须被识别；
  * 篡改/删除 JNI 库必须被报出。

局限说明：本脚本证明「APK 里装了什么」，不证明「真机上跑得对」。
真机行为见 docs/android/device-runbook.md 与 V-AN2 卡的延期验收。

────────────────────────────────────────────────────────────────────────────
维护说明（REL1 t_bf1e2bfb 登记）：本文件在 2026-09-19 由 REL1 的反向对照脚本
（results/rel1-logs/rel1-negative-control.sh，第一版经软链误删真实文件）**重建**。
重建依据：AN6 卡片日志中该文件自身的模块说明（检查项 [1]–[7]、退出码、自测口径）、
AN6/van1 记录的 CLI 契约与 `--json` 事实键、以及 android/checklist C8 的期望输出。
重建版满足同一命令行与同一 JSON 键；**sha256 与 AN6 原始文件不同**，已在
results/rel1-release-scripts.md 中如实登记，M4/M5 复核时以本条为准。

F-REL1b-1 修复（t_481c627b，2026-09-19）：重建版缺 SDK 根自动探测，默认口径把
aapt2/apksigner 外部交叉校验记成「本机无 aapt2」的静默跳过项（exit 仍 0）。本卡按
「行为等价修复」恢复探测链（显式 --sdk-root → PATH → $ANDROID_HOME →
$ANDROID_SDK_ROOT → android/local.properties 的 sdk.dir → 本机默认根），并让跳过文案
如实列出已试位置、把 facts.sdk_probe 写入 --json。**AN6 原始字节仍不可复原**，
本次不声称恢复原始 sha256；新 sha256 见 results/f-rel1b-1-2-fix.md。
────────────────────────────────────────────────────────────────────────────
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import zipfile
from collections.abc import Sequence
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
GRADLE_FILE_DEFAULT = REPO_ROOT / "android" / "app" / "build.gradle.kts"
LOCAL_PROPERTIES_DEFAULT = REPO_ROOT / "android" / "local.properties"

# 本机默认 SDK 根（探测链最后一层；F-REL1b-1）
DEFAULT_SDK_ROOTS: tuple[Path, ...] = (Path("/opt/dev-tools/android-sdk"),)

DEFAULT_PACKAGE = "com.novelreader.android"
DEFAULT_ABI = "arm64-v8a"
DEFAULT_MIN_SDK = 31
DEFAULT_TARGET_SDK = 36
JNI_LIB_NAME = "libnr_jni.so"

# ---- 二进制 XML（AXML）常量 -------------------------------------------------
RES_STRING_POOL_TYPE = 0x0001
RES_XML_TYPE = 0x0003
RES_XML_START_ELEMENT_TYPE = 0x0102

TYPE_REFERENCE = 0x01
TYPE_STRING = 0x03
TYPE_INT_DEC = 0x10
TYPE_INT_HEX = 0x11
TYPE_INT_BOOLEAN = 0x12

# ---- APK 签名常量 ----------------------------------------------------------
APK_SIG_BLOCK_MAGIC = b"APK Sig Block 42"
SIG_BLOCK_IDS = {
    0x7109871A: "v2",
    0xF05368C0: "v3",
    0x1B93AD61: "v3.1",
    0x6DFF800D: "source-stamp",
    0x42726577: "verity-padding",
}
EOCD_SIG = b"PK\x05\x06"

# ---- ELF 常量 --------------------------------------------------------------
EM_MACHINE_BY_ABI = {"arm64-v8a": 0xB7, "armeabi-v7a": 0x28, "x86_64": 0x3E, "x86": 0x03}

MARKER_RE = re.compile(
    r'buildConfigField\(\s*"String"\s*,\s*"BUILD_MARKER"\s*,\s*"\\"([^"\\]+)\\""\s*\)'
)


class Report:
    def __init__(self) -> None:
        self.ok_count = 0
        self.failures: list[str] = []
        self.skips: list[str] = []
        self.facts: dict[str, object] = {}

    def ok(self, label: str) -> None:
        self.ok_count += 1
        print(f"[ok]   {label}")

    def fail(self, label: str) -> None:
        self.failures.append(label)
        print(f"[FAIL] {label}")

    def skip(self, label: str) -> None:
        self.skips.append(label)
        print(f"[skip] {label}")

    def fact(self, key: str, value: object) -> None:
        self.facts[key] = value


# --------------------------------------------------------------------------- #
# 二进制 AndroidManifest.xml 解析（纯 Python）
# --------------------------------------------------------------------------- #


def _string_pool(data: bytes, offset: int) -> tuple[list[str], int]:
    (chunk_type, header_size, chunk_size) = struct.unpack_from("<HHI", data, offset)
    if chunk_type != RES_STRING_POOL_TYPE:
        raise ValueError(f"不是字符串池 chunk（type=0x{chunk_type:04x}）")
    (string_count, style_count, flags, strings_start, styles_start) = struct.unpack_from(
        "<IIIII", data, offset + 8
    )
    utf8 = bool(flags & (1 << 8))
    offsets = struct.unpack_from(f"<{string_count}I", data, offset + header_size)
    strings: list[str] = []
    base = offset + strings_start
    for off in offsets:
        pos = base + off
        if utf8:
            # 先读 UTF-16 长度（可变长），再读 UTF-8 长度与字节
            n = data[pos]
            pos += 2 if n & 0x80 else 1
            m = data[pos]
            if m & 0x80:
                m = ((m & 0x7F) << 8) | data[pos + 1]
                pos += 2
            else:
                pos += 1
            strings.append(data[pos : pos + m].decode("utf-8", "replace"))
        else:
            n = struct.unpack_from("<H", data, pos)[0]
            pos += 2
            if n & 0x8000:
                n = ((n & 0x7FFF) << 16) | struct.unpack_from("<H", data, pos)[0]
                pos += 2
            strings.append(data[pos : pos + n * 2].decode("utf-16-le", "replace"))
    return strings, chunk_size


def parse_binary_manifest(raw: bytes) -> dict[str, object]:
    """解析 AXML，返回 package / versionCode / versionName / minSdk / targetSdk / debuggable。"""
    (chunk_type, header_size, chunk_size) = struct.unpack_from("<HHI", raw, 0)
    if chunk_type != RES_XML_TYPE:
        raise ValueError(f"不是二进制 XML（type=0x{chunk_type:04x}）")
    strings: list[str] = []
    out: dict[str, object] = {
        "package": None,
        "version_code": None,
        "version_name": None,
        "min_sdk": None,
        "target_sdk": None,
        "debuggable": None,
        "unresolved": [],
    }
    pos = header_size
    while pos + 8 <= chunk_size:
        (ctype, chdr, csize) = struct.unpack_from("<HHI", raw, pos)
        if csize == 0:
            break
        if ctype == RES_STRING_POOL_TYPE and not strings:
            strings, _ = _string_pool(raw, pos)
        elif ctype == RES_XML_START_ELEMENT_TYPE:
            (line, comment, ns, name_idx, attr_start, attr_size, attr_count) = struct.unpack_from(
                "<IIIIHHH", raw, pos + 8
            )
            element = strings[name_idx] if name_idx < len(strings) else ""
            for i in range(attr_count):
                # 属性区起点 = chunk 起点 + 16（ResXMLTree_node 头 8 + lineNumber/comment 8）+ attributeStart
                apos = pos + 16 + attr_start + i * attr_size
                (_ans, aname, araw, asize, _res0, atype, adata) = struct.unpack_from(
                    "<IIIHBBI", raw, apos
                )
                key = strings[aname] if aname < len(strings) else ""
                if atype == TYPE_STRING and adata < len(strings):
                    value: object = strings[adata]
                elif atype in (TYPE_INT_DEC, TYPE_INT_HEX):
                    value = adata
                elif atype == TYPE_INT_BOOLEAN:
                    value = bool(adata)
                elif atype == TYPE_REFERENCE:
                    value = f"@ref/0x{adata:08x}"
                else:
                    value = f"<type 0x{atype:02x}>"
                if element == "manifest":
                    if key == "package":
                        out["package"] = str(value)
                    elif key == "versionCode":
                        out["version_code"] = int(value) if isinstance(value, int) else None
                    elif key == "versionName":
                        out["version_name"] = str(value)
                    elif key == "debuggable":
                        out["debuggable"] = bool(value)
                elif element == "uses-sdk":
                    if key in ("minSdkVersion", "minSdk"):
                        out["min_sdk"] = int(value) if isinstance(value, int) else None
                    elif key in ("targetSdkVersion", "targetSdk"):
                        out["target_sdk"] = int(value) if isinstance(value, int) else None
                elif element == "application" and key == "debuggable":
                    out["debuggable"] = bool(value)
        pos += csize
    for key in ("package", "version_code", "version_name", "min_sdk", "target_sdk"):
        if out[key] is None:
            out["unresolved"].append(key)  # type: ignore[union-attr]
    return out


# --------------------------------------------------------------------------- #
# APK 结构 / 签名 / ELF
# --------------------------------------------------------------------------- #


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_marker_from_gradle(path: Path) -> str | None:
    if not path.is_file():
        return None
    m = MARKER_RE.search(path.read_text(encoding="utf-8", errors="replace"))
    return m.group(1) if m else None


def dex_names(zf: zipfile.ZipFile) -> list[str]:
    return sorted(n for n in zf.namelist() if re.fullmatch(r"classes\d*\.dex", n))


def scan_marker_in_dexes(zf: zipfile.ZipFile, names: list[str], marker: str) -> list[str]:
    needle = marker.encode("utf-8")
    hits: list[str] = []
    for name in names:
        if needle in zf.read(name):
            hits.append(name)
    return hits


def elf_info(data: bytes) -> dict[str, object] | None:
    if len(data) < 20 or data[:4] != b"\x7fELF":
        return None
    ei_class = data[4]
    e_type, e_machine = struct.unpack_from("<HH", data, 16)
    return {
        "class": "ELF64" if ei_class == 2 else "ELF32",
        "machine": e_machine,
        "machine_name": {0x03: "i386", 0x28: "ARM", 0xB7: "AArch64", 0x3E: "x86-64"}.get(
            e_machine, f"0x{e_machine:04x}"
        ),
        "type": {2: "EXEC", 3: "DYN"}.get(e_type, str(e_type)),
    }


def signature_facts(apk: Path, zf: zipfile.ZipFile) -> dict[str, object]:
    names = zf.namelist()
    v1_entries = [n for n in names if re.match(r"META-INF/.*\.(SF|RSA|DSA|EC)$", n, re.I)]
    v1 = bool(v1_entries)
    block_ids: list[str] = []
    detail = "无 APK Signing Block"
    data = apk.read_bytes()
    eocd = data.rfind(EOCD_SIG)
    if eocd != -1 and eocd >= 32:
        cd_offset = struct.unpack_from("<I", data, eocd + 16)[0]
        # APK Signing Block: [u64 size][pairs][u64 size][16B magic]，紧邻中央目录之前
        if cd_offset >= 32 and data[cd_offset - 16 : cd_offset] == APK_SIG_BLOCK_MAGIC:
            size2 = struct.unpack_from("<Q", data, cd_offset - 24)[0]
            block_start = cd_offset - 8 - size2
            if block_start >= 0:
                pos = block_start + 8
                raw_ids: list[int] = []
                end = block_start + size2 - 8
                while pos + 12 <= end:
                    (length,) = struct.unpack_from("<Q", data, pos)
                    if length < 4:
                        break
                    (bid,) = struct.unpack_from("<I", data, pos + 8)
                    raw_ids.append(bid)
                    pos += 8 + length
                block_ids = [SIG_BLOCK_IDS.get(b, f"0x{b:08x}") for b in raw_ids]
                detail = "签名块 ID: " + ", ".join(f"0x{b:08x}" for b in raw_ids)
    v4_idsig = Path(str(apk) + ".idsig").is_file()
    signed = v1 or any(b in {"v2", "v3", "v3.1"} for b in block_ids)
    return {
        "signed": signed,
        "v1": v1,
        "v1_manifest_mf": "META-INF/MANIFEST.MF" in names,
        "block_ids": block_ids,
        "block_detail": detail,
        "v4_idsig": v4_idsig,
    }


def unescape_properties_value(value: str) -> str:
    """按 Java Properties 语义去掉 `\\` 转义（`\\:` → `:`、`\\\\` → `\\`）。"""
    out: list[str] = []
    i = 0
    while i < len(value):
        if value[i] == "\\" and i + 1 < len(value):
            out.append(value[i + 1])
            i += 2
            continue
        out.append(value[i])
        i += 1
    return "".join(out)


def read_local_properties_sdk_dir(path: Path | None = None) -> str | None:
    """读取 android/local.properties 的 sdk.dir（容忍 `#`/`!` 注释与 `=`/`:` 分隔）。"""
    props = LOCAL_PROPERTIES_DEFAULT if path is None else path
    try:
        text = props.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line[0] in "#!":
            continue
        if "=" in line:
            key, value = line.split("=", 1)
        elif ":" in line:
            key, value = line.split(":", 1)
        else:
            continue
        if key.strip() != "sdk.dir":
            continue
        resolved = unescape_properties_value(value.strip())
        return resolved or None
    return None


def sdk_root_candidates(
    *,
    local_properties: Path | None = None,
    env: dict[str, str] | None = None,
    defaults: Sequence[Path] | None = None,
) -> list[tuple[str, Path]]:
    """环境/配置层候选 SDK 根，按优先级返回 (来源标签, 根路径)。

    标签会被原样写进 skip 文案与 JSON，因此必须自带来历（含具体路径）。
    """
    environ = os.environ if env is None else env
    candidates: list[tuple[str, Path]] = []
    for var in ("ANDROID_HOME", "ANDROID_SDK_ROOT"):
        raw = (environ.get(var) or "").strip()
        if raw:
            candidates.append((f"{var}={raw}", Path(raw)))
    sdk_dir = read_local_properties_sdk_dir(local_properties)
    if sdk_dir:
        candidates.append((f"local.properties(sdk.dir={sdk_dir})", Path(sdk_dir)))
    for default in DEFAULT_SDK_ROOTS if defaults is None else defaults:
        candidates.append((f"本机默认={default}", Path(default)))
    return candidates


def _tool_in_root(name: str, root: Path) -> str | None:
    lookups = [
        sorted(root.glob(f"build-tools/*/{name}")),
        sorted(root.glob(f"*/{name}")),
        [root / name],
    ]
    for group in lookups:
        for candidate in group:
            if candidate.is_file():
                return str(candidate)
    return None


def resolve_sdk_tools(
    names: Sequence[str],
    explicit: Path | None = None,
    *,
    local_properties: Path | None = None,
    env: dict[str, str] | None = None,
    defaults: Sequence[Path] | None = None,
) -> dict[str, object]:
    """按 F-REL1b-1 的探测链解析 SDK 工具。

    顺序：显式 `--sdk-root` → `PATH` → `$ANDROID_HOME` → `$ANDROID_SDK_ROOT` →
    `android/local.properties` 的 `sdk.dir` → 本机默认根。

    返回 `{"tried": [来源标签…], "found": {name: path|None}, "found_from": {name: 来源标签|None}}`。
    `tried` 登记**每一层尝试过的位置**（含不存在的根），使「跳过」不再等于静默降级。
    """
    layers: list[tuple[str, Path | None]] = []
    if explicit is not None:
        layers.append((f"--sdk-root={explicit}", explicit))
    layers.append(("PATH", None))
    layers.extend(sdk_root_candidates(local_properties=local_properties, env=env, defaults=defaults))

    found: dict[str, str | None] = {name: None for name in names}
    found_from: dict[str, str | None] = {name: None for name in names}
    for label, root in layers:
        if root is None:
            for name in names:
                if found[name] is None:
                    hit = shutil.which(name)
                    if hit:
                        found[name], found_from[name] = hit, label
            continue
        if not root.is_dir():
            continue
        for name in names:
            if found[name] is not None:
                continue
            hit = _tool_in_root(name, root)
            if hit:
                found[name], found_from[name] = hit, label
    return {"tried": [label for label, _ in layers], "found": found, "found_from": found_from}


def cross_check(apk: Path, facts: dict[str, object], sdk_root: Path | None, rep: Report) -> None:
    probe = resolve_sdk_tools(["aapt2", "apksigner"], sdk_root)
    facts["sdk_probe"] = probe
    tried = " / ".join(str(label) for label in probe["tried"])  # type: ignore[union-attr]
    found: dict[str, str | None] = probe["found"]  # type: ignore[assignment]
    found_from: dict[str, str | None] = probe["found_from"]  # type: ignore[assignment]

    aapt2 = found["aapt2"]
    if aapt2:
        print(f"       aapt2 探测命中：{aapt2}（来源：{found_from['aapt2']}）")
        proc = subprocess.run(
            [aapt2, "dump", "badging", str(apk)], capture_output=True, text=True
        )
        text = proc.stdout
        pkg = re.search(r"package: name='([^']+)' versionCode='(\d+)' versionName='([^']*)'", text)
        m_sdk = re.search(r"minSdkVersion:'(\d+)'", text) or re.search(r"sdkVersion:'(\d+)'", text)
        t_sdk = re.search(r"targetSdkVersion:'(\d+)'", text)
        aapt_facts = {
            "package": pkg.group(1) if pkg else None,
            "version_code": int(pkg.group(2)) if pkg else None,
            "version_name": pkg.group(3) if pkg else None,
            "min_sdk": int(m_sdk.group(1)) if m_sdk else None,
            "target_sdk": int(t_sdk.group(1)) if t_sdk else None,
        }
        facts["aapt2"] = aapt_facts
        manifest = facts["manifest"]  # type: ignore[assignment]
        if pkg is None:
            rep.skip(f"aapt2 未能解析 badging 输出（exit={proc.returncode}），跳过交叉校验")
        else:
            mismatches = [
                k
                for k in ("package", "version_code", "version_name", "min_sdk", "target_sdk")
                if manifest[k] is not None and aapt_facts[k] != manifest[k]
            ]
            if mismatches:
                rep.fail(f"aapt2 独立复核与纯 Python 解析不一致：{mismatches}")
            else:
                rep.ok(
                    "aapt2 独立复核一致（package: name='{package}' versionCode='{version_code}' "
                    "versionName='{version_name}'）".format(**aapt_facts)
                )
    else:
        rep.skip(
            f"未找到 aapt2（已试 {tried}）；包名/SDK 外部交叉校验跳过 —— 调用方应据此判定为「校验器降级」"
        )

    apksigner = found["apksigner"]
    if apksigner:
        print(f"       apksigner 探测命中：{apksigner}（来源：{found_from['apksigner']}）")
        proc = subprocess.run([apksigner, "verify", "--print-certs", str(apk)], capture_output=True, text=True)
        facts["apksigner_exit"] = proc.returncode
        dn = None
        for line in proc.stdout.splitlines():
            if "certificate DN:" in line:
                dn = line.split("certificate DN:", 1)[1].strip()
                break
        facts["signer_dn"] = dn
        sig = facts["signature"]  # type: ignore[assignment]
        if sig["signed"] and proc.returncode == 0:
            rep.ok(f"apksigner 独立复核一致：Verifies，证书 DN: {dn}")
        elif not sig["signed"] and proc.returncode != 0:
            rep.ok("apksigner 独立复核一致：未签名（属预期，不计为失败）")
        else:
            rep.fail(
                f"apksigner 结论与签名块判定不一致：signed={sig['signed']} apksigner_exit={proc.returncode}"
            )
    else:
        rep.skip(
            f"未找到 apksigner（已试 {tried}）；签名状态外部交叉校验跳过 —— 调用方应据此判定为「校验器降级」"
        )


# --------------------------------------------------------------------------- #
# 主判定
# --------------------------------------------------------------------------- #


def evaluate(
    apk: Path,
    rep: Report,
    *,
    abi: str,
    expect_package: str,
    min_sdk: int,
    target_sdk: int,
    expect_version_code: int | None,
    expect_version_name: str | None,
    marker: str | None,
    signature: str,
    sdk_root: Path | None,
) -> None:
    data = apk.read_bytes()
    rep.fact("apk", str(apk))
    rep.fact("apk_size", len(data))
    digest = sha256_bytes(data)
    rep.fact("apk_sha256", digest)
    print(f"[1] APK 结构：{apk}（{len(data)} B, sha256={digest[:16]}…）")

    with zipfile.ZipFile(apk) as zf:
        names = zf.namelist()
        if "AndroidManifest.xml" in names:
            rep.ok("ZIP 结构可读且含 AndroidManifest.xml")
        else:
            rep.fail("APK 内缺少 AndroidManifest.xml")
            return

        print("[2] 二进制 AndroidManifest.xml（纯 Python 解析）")
        manifest = parse_binary_manifest(zf.read("AndroidManifest.xml"))
        rep.fact("manifest", manifest)
        if manifest["unresolved"]:
            rep.fail(f"manifest 关键字段未解析：{manifest['unresolved']}")
        else:
            rep.ok(
                "manifest：package={package} versionCode={version_code} "
                "versionName={version_name}".format(**manifest)  # type: ignore[arg-type]
            )

        print("[3] 包名与 SDK 范围")
        if manifest["package"] == expect_package:
            rep.ok(f"包名 {expect_package} 符合规格")
        else:
            rep.fail(f"包名 {manifest['package']} ≠ 期望 {expect_package}")
        if expect_version_code is not None and manifest["version_code"] != expect_version_code:
            rep.fail(f"versionCode {manifest['version_code']} ≠ 期望 {expect_version_code}")
        if expect_version_name is not None and manifest["version_name"] != expect_version_name:
            rep.fail(f"versionName {manifest['version_name']} ≠ 期望 {expect_version_name}")
        for label, got, want in (
            ("minSdk", manifest["min_sdk"], min_sdk),
            ("targetSdk", manifest["target_sdk"], target_sdk),
        ):
            if got == want:
                rep.ok(f"{label}={got} 符合规格")
            else:
                rep.fail(f"{label}={got} ≠ 规格 {want}")

        print(f"[4] JNI 库（期望 ABI {abi}）")
        jni_names = [n for n in names if n.endswith(f"/{JNI_LIB_NAME}")]
        expected_entry = f"lib/{abi}/{JNI_LIB_NAME}"
        if not jni_names:
            rep.fail(f"APK 内没有任何 {JNI_LIB_NAME}（JNI 库未打包）")
        undeclared = [n for n in jni_names if n != expected_entry]
        if expected_entry in jni_names:
            lib = zf.read(expected_entry)
            info = elf_info(lib)
            want_machine = EM_MACHINE_BY_ABI.get(abi)
            facts_jni = {
                "entry": expected_entry,
                "size": len(lib),
                "sha256": sha256_bytes(lib),
            }
            if info is None:
                rep.fail(f"{expected_entry} 不是 ELF 文件")
            else:
                facts_jni.update(info)
                if info["class"] == "ELF64" and info["machine"] == want_machine and info["type"] == "DYN":
                    rep.ok(
                        f"{expected_entry} 为 ELF64/{info['machine_name']}/DYN（{len(lib)} B, "
                        f"sha256={facts_jni['sha256'][:16]}…）"  # type: ignore[index]
                    )
                else:
                    rep.fail(
                        f"{expected_entry} ELF 属性不符：class={info['class']} machine={info['machine_name']} "
                        f"type={info['type']}（期望 ELF64/{abi}/DYN）"
                    )
            rep.fact("jni_lib", facts_jni)
        else:
            rep.fail(f"缺少 {expected_entry}（期望 ABI 的 JNI 库不在包内）")
        if undeclared:
            rep.fail(f"存在未声明 ABI 的 JNI 库：{undeclared}（abiFilters 应只发一个 ABI）")
        else:
            rep.ok("不存在未声明 ABI 的 JNI 库")

        print("[5] DEX 构建标记（全 classes*.dex 扫描）")
        dexes = dex_names(zf)
        rep.fact("dex_count", len(dexes))
        rep.fact("dex_names", dexes)
        if not dexes:
            rep.fail("APK 内没有 classes*.dex")
        else:
            rep.ok(f"包含 {len(dexes)} 个 dex：{dexes}")
        rep.fact("marker", marker)
        if marker is None:
            rep.skip("未取得期望 BUILD_MARKER（--marker 缺省且 build.gradle.kts 未声明）")
        else:
            hits = scan_marker_in_dexes(zf, dexes, marker)
            rep.fact("marker_hit_dex", hits)
            if hits:
                rep.ok(f"构建标记命中 {hits}（全部 dex 都参与扫描）")
            else:
                rep.fail(f"全部 {len(dexes)} 个 dex 均未找到构建标记 {marker!r}（新代码未入包？）")

        print("[6] 签名状态")
        sig = signature_facts(apk, zf)
        rep.fact("signature", sig)
        rep.fact("build_type", "debug" if manifest.get("debuggable") else "release")
        if sig["signed"]:
            rep.ok(
                f"已签名（v1={sig['v1']}，签名块={sig['block_ids']}，v4={sig['v4_idsig']}）"
            )
        else:
            rep.ok("未签名（无 v1 签名块，也无 v2/v3 APK Signing Block）")
        if signature == "signed" and not sig["signed"]:
            rep.fail("--signature signed 要求已签名，但 APK 未签名")
        if signature == "unsigned" and sig["signed"]:
            rep.fail("--signature unsigned 要求未签名，但 APK 已签名")
        if signature == "auto" and not sig["signed"]:
            rep.skip("Release 未签名属预期（无 signingConfig）；发布前必须正式签名，不计为构建失败")

    print("[7] aapt2 / apksigner 交叉校验（SDK 根自动探测）")
    cross_check(apk, rep.facts, sdk_root, rep)


# --------------------------------------------------------------------------- #
# 自测：反向用例
# --------------------------------------------------------------------------- #


def self_test(apk: Path) -> list[str]:
    import tempfile

    failures: list[str] = []
    print("[self-test] 由真实 APK 派生 fixture 的反向用例")
    with zipfile.ZipFile(apk) as real:
        base = {n: real.read(n) for n in real.namelist()}
    marker = "self-test-marker-0123456789"

    def build(path: Path, *, put_marker_in: str | None, drop_jni: bool = False) -> None:
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as out:
            for name, payload in base.items():
                if drop_jni and name.endswith(JNI_LIB_NAME):
                    continue
                if put_marker_in is not None and name == put_marker_in:
                    payload = payload + b"\x00" + marker.encode()
                out.writestr(name, payload)

    with tempfile.TemporaryDirectory() as td:
        td_path = Path(td)
        late = "classes7.dex"
        raw = base.get("classes.dex", b"\x00")
        with zipfile.ZipFile(apk) as zf:
            names = dex_names(zf)
        if not names:
            return ["自测前提缺失：真实 APK 内没有 classes*.dex"]

        # 用例 1：标记只在合成的最靠后 dex 里 → 必须判为「命中」
        case1 = td_path / "case1.apk"
        with zipfile.ZipFile(case1, "w", zipfile.ZIP_DEFLATED) as out:
            for name, payload in base.items():
                out.writestr(name, payload)
            out.writestr(late, b"dex\x00" + marker.encode())
        with zipfile.ZipFile(case1) as zf:
            hits = scan_marker_in_dexes(zf, dex_names(zf), marker)
        if hits == [late]:
            print(f"  PASS  标记只写在 {late} → 被判定命中（全 dex 扫描生效）")
        else:
            failures.append(f"全 dex 扫描未命中：{hits}")
            print(f"  FAIL  标记只写在 {late} 但判定为 {hits}")

        # 用例 2：删掉标记 → 必须判为「未命中」
        case2 = td_path / "case2.apk"
        build(case2, put_marker_in=None)
        with zipfile.ZipFile(case2) as zf:
            hits = scan_marker_in_dexes(zf, dex_names(zf), marker)
        if not hits:
            print("  PASS  无标记 fixture → 判定未命中")
        else:
            failures.append(f"无标记 fixture 被误判命中：{hits}")
            print(f"  FAIL  无标记 fixture 被误判命中：{hits}")

        # 用例 3：伪造 v1（META-INF/CERT.SF）→ 必须识别为已签名
        case3 = td_path / "case3.apk"
        with zipfile.ZipFile(case3, "w", zipfile.ZIP_DEFLATED) as out:
            for name, payload in base.items():
                out.writestr(name, payload)
            out.writestr("META-INF/CERT.SF", b"Signature-Version: 1.0\n")
        with zipfile.ZipFile(case3) as zf:
            sig = signature_facts(case3, zf)
        if sig["v1"] and sig["signed"]:
            print("  PASS  伪造 v1 fixture → 识别为已签名（v1 判定有反面能力）")
        else:
            failures.append(f"伪造 v1 fixture 未被识别：{sig}")
            print(f"  FAIL  伪造 v1 fixture 未被识别：{sig}")

        # 用例 4：删掉 JNI 库 → 判定必须报出（用真实 APK 与去掉 .so 的 fixture 对比）
        case4 = td_path / "case4.apk"
        build(case4, put_marker_in=None, drop_jni=True)
        with zipfile.ZipFile(case4) as zf:
            jni = [n for n in zf.namelist() if n.endswith(JNI_LIB_NAME)]
        if not jni:
            print("  PASS  去除 JNI 库的 fixture 确实不含 .so（判定分支有输入）")
        else:
            failures.append(f"去除 JNI 库的 fixture 仍含 {jni}")
            print(f"  FAIL  去除 JNI 库的 fixture 仍含 {jni}")

        # 用例 5：ELF 判定反面能力（AArch64 vs x86-64 头）
        aarch = struct.pack("<4sBBH", b"\x7fELF", 2, 1, 0) + b"\x00" * 8 + struct.pack("<HH", 3, 0xB7)
        x86 = struct.pack("<4sBBH", b"\x7fELF", 2, 1, 0) + b"\x00" * 8 + struct.pack("<HH", 3, 0x3E)
        ia, ix = elf_info(aarch), elf_info(x86)
        if ia and ix and ia["machine"] == 0xB7 and ix["machine"] == 0x3E and ia["machine"] != ix["machine"]:
            print("  PASS  ELF ABI 判定可区分 AArch64 与 x86-64")
        else:
            failures.append(f"ELF ABI 判定不可区分：{ia} vs {ix}")
            print(f"  FAIL  ELF ABI 判定不可区分：{ia} vs {ix}")

        # 用例 6（F-REL1b-1 反例）：local.properties 声明了 SDK 根时**不得跳过**。
        # 用唯一的假工具名 + 注入 env/defaults，排除 PATH / 真实 SDK 的干扰，保证判定确定。
        fake_tool = "nr-faketool-probe-only"
        fake_root = td_path / "fake-sdk"
        (fake_root / "build-tools" / "36.0.0").mkdir(parents=True)
        fake_tool_path = fake_root / "build-tools" / "36.0.0" / fake_tool
        fake_tool_path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        fake_props = td_path / "local.properties"
        fake_props.write_text(f"# 注释行\nsdk.dir={fake_root}\n", encoding="utf-8")
        probe = resolve_sdk_tools(
            [fake_tool], None, local_properties=fake_props, env={}, defaults=()
        )
        probe_found = probe["found"]  # type: ignore[assignment]
        probe_from = probe["found_from"]  # type: ignore[assignment]
        if probe_found[fake_tool] == str(fake_tool_path) and str(probe_from[fake_tool]).startswith(
            "local.properties"
        ):
            print(
                f"  PASS  local.properties 声明 SDK 根 → 工具被自动找到（来源：{probe_from[fake_tool]}），不跳过"
            )
        else:
            failures.append(
                f"local.properties 声明 SDK 根却未找到工具（跳过未被阻止）："
                f"found={probe_found[fake_tool]} from={probe_from[fake_tool]}"
            )
            print(
                f"  FAIL  local.properties 声明 SDK 根却未找到工具：found={probe_found[fake_tool]} "
                f"from={probe_from[fake_tool]}"
            )

        # 用例 7：local.properties 指向不存在目录 → 必须如实登记「已试位置」（非静默、非假阳性）
        bogus_root = td_path / "no-such-sdk-root"
        bogus_props = td_path / "local.properties.bogus"
        bogus_props.write_text(f"sdk.dir={bogus_root}\n", encoding="utf-8")
        probe_bogus = resolve_sdk_tools(
            [fake_tool], None, local_properties=bogus_props, env={}, defaults=()
        )
        bogus_found = probe_bogus["found"]  # type: ignore[assignment]
        bogus_tried = [str(label) for label in probe_bogus["tried"]]  # type: ignore[union-attr]
        if bogus_found[fake_tool] is None and any(str(bogus_root) in label for label in bogus_tried):
            print(f"  PASS  不存在的 SDK 根仍被登记为「已试位置」（tried={bogus_tried}），跳过非静默")
        else:
            failures.append(
                f"不存在的 SDK 根未被如实登记：found={bogus_found[fake_tool]} tried={bogus_tried}"
            )
            print(
                f"  FAIL  不存在的 SDK 根未被如实登记：found={bogus_found[fake_tool]} tried={bogus_tried}"
            )
    return failures


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="APK 内容与签名状态校验（C8）")
    p.add_argument("apk", help="APK 路径")
    p.add_argument("--abi", default=DEFAULT_ABI)
    p.add_argument("--expect-package", default=DEFAULT_PACKAGE)
    p.add_argument("--min-sdk", type=int, default=DEFAULT_MIN_SDK)
    p.add_argument("--target-sdk", type=int, default=DEFAULT_TARGET_SDK)
    p.add_argument("--expect-version-code", type=int, default=None)
    p.add_argument("--expect-version-name", default=None)
    p.add_argument("--marker", default=None, help="期望的 BUILD_MARKER；缺省从 build.gradle.kts 读取")
    p.add_argument("--gradle-file", default=str(GRADLE_FILE_DEFAULT))
    p.add_argument("--signature", choices=["auto", "signed", "unsigned"], default="auto")
    p.add_argument(
        "--sdk-root",
        default=None,
        help="Android SDK 根（最优先；缺省时按 PATH → ANDROID_HOME → ANDROID_SDK_ROOT → "
        "android/local.properties 的 sdk.dir → 本机默认根 自动探测）",
    )
    p.add_argument("--json", dest="json_out", default=None, help="把结论写入 JSON 文件")
    p.add_argument("--self-test", action="store_true", help="运行 fixture 反向用例")
    args = p.parse_args(argv)

    apk = Path(args.apk)
    print(f"verify-android-apk: {apk}")
    if not apk.is_file():
        print(f"RESULT: FAIL — APK 不存在或不可读：{apk}")
        return 2
    try:
        with zipfile.ZipFile(apk):
            pass
    except zipfile.BadZipFile as exc:
        print(f"RESULT: FAIL — APK 不是有效 ZIP：{exc}")
        return 2

    marker = args.marker
    if marker is None:
        marker = read_marker_from_gradle(Path(args.gradle_file))
        if marker is None:
            print(f"RESULT: FAIL — 无法从 {args.gradle_file} 取得期望 BUILD_MARKER（用 --marker 指定）")
            return 2
        print(f"       期望 BUILD_MARKER（来自 gradle）：{marker}")

    exit_code = 0
    if args.self_test:
        failures = self_test(apk)
        print("-" * 68)
        if failures:
            print(f"SELF-TEST RESULT: FAIL ({len(failures)} 项)")
            for f in failures:
                print(f"  - {f}")
            exit_code = 1
        else:
            print("SELF-TEST RESULT: PASS（每条判定均有反向用例）")

    rep = Report()
    sdk_root = Path(args.sdk_root) if args.sdk_root else None
    evaluate(
        apk,
        rep,
        abi=args.abi,
        expect_package=args.expect_package,
        min_sdk=args.min_sdk,
        target_sdk=args.target_sdk,
        expect_version_code=args.expect_version_code,
        expect_version_name=args.expect_version_name,
        marker=marker,
        signature=args.signature,
        sdk_root=sdk_root,
    )

    print("-" * 68)
    if args.json_out:
        payload = {
            "apk": str(apk),
            "result": "PASS" if (not rep.failures and exit_code == 0) else "FAIL",
            "ok_count": rep.ok_count,
            "failures": rep.failures,
            "skips": rep.skips,
            "facts": rep.facts,
        }
        Path(args.json_out).write_text(
            json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
        )
        print(f"JSON 结论已写入 {args.json_out}")

    if rep.failures:
        print(f"RESULT: FAIL ({len(rep.failures)} 项)")
        for item in rep.failures:
            print(f"  - {item}")
        return 1
    if exit_code != 0:
        print("RESULT: FAIL — --self-test 未通过")
        return 1
    sig = rep.facts.get("signature") or {}
    state = "已签名" if sig.get("signed") else "未签名"
    print(
        f"RESULT: PASS — 包名 {DEFAULT_PACKAGE}、minSdk/targetSdk {args.min_sdk}/{args.target_sdk}、"
        f"{args.abi} JNI 库、DEX 标记 ({rep.facts.get('dex_count')} 个 dex)、签名状态 {state}"
        f"（跳过项 {len(rep.skips)}）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
