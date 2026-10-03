#!/usr/bin/env python3
"""verify-v1-release.py — V1 发布物 / 版本标识 / 依赖许可 / 未验证项 门禁（父级 checklist C11；卡片 REL1）。

用途（父规格 checklist C11 / PAC5；`docs/novel-reader-v1-development-plan.md` §15）：

  输出一份**可复核的发布清单**，而不是给出「可以发布」的结论：

  [1] 版本与构建标识：Rust workspace 版本 ↔ 桌面 crate 版本；Android `versionName/versionCode`；
      Android `BUILD_MARKER` 三方一致链（`app/build.gradle.kts` ↔ `dist/BUILD-INFO.md` ↔ APK 内
      dex 字符串，APK 侧由 `scripts/verify-android-apk.py` 独立解析）；桌面二进制 ELF 属性
      （64 位 / x86_64 / 符号剥离 / `.note.gnu.build-id`）；工具链版本（rust-toolchain、
      Gradle Wrapper、AGP、Kotlin、NDK、build-tools）；`Cargo.lock` 的**发布时哈希**登记
      （gate-1-core.md O6 裁定要求重新登记，不得沿用 RC6 旧值）；
  [2] 发布物清单 + SHA-256：桌面 Release 二进制（ELF/剥离/`ldd` 运行库）、`dist/*.apk`
      （大小 / SHA-256 / 签名状态 / 版本 / 构建标记 / APK 内 JNI `.so` 与源码树预编译 `.so` 同源），
      并用 `dist/BUILD-INFO.md` 记录的 大小/SHA-256/标记 做**跨文件逐项核对**；
      `dist/novel-reader-<版本>-linux-<arch>-<会话>/` 发布目录若存在则校验 `VERSION` 与
      `MANIFEST.sha256` 自洽，未产出时登记为「未产出/待验证」（不伪造、也不假装通过）；
  [3] 依赖与许可证清单：Rust（根 `Cargo.lock` + 桌面嵌套 workspace 的 `Cargo.lock`，逐包解析
      `license` 字段，来源=本地 registry/path crate）、Android（`build.gradle.kts` 直接声明依赖，
      按 scope 标注「随 Release 包分发 / 仅 Debug / 仅测试」，来源=Gradle 缓存的 POM），
      未解析项一律报出（不得静默视为通过）；项目自身发行许可证按规格记为**延期未定**；
  [4] 发布文档与已知限制登记：`docs/release/` 模板齐全、必需章节/占位符存在、
      `known-limitations.md` 登记了 gate-1 裁定与延期验收项，且**不得**出现
      「已真机验证」「三平台发布完成」「Android 全面通过」类未实测宣称；
  [5] 未验证项清单：Android 真机（SAF/旋转/后台/返回/JNI 取消/功耗）、Windows 10/11 与
      Linux Wayland、未签名 Release APK、Android 传递依赖许可、项目发行许可证、
      干净发行版上的桌面发布目录 —— 全部以 `待验证` 输出，且不计入通过。
  [6] 阻塞项清单（区别于「延期未验证」）：真实执行 `scripts/verify-android-boundary.py` 默认口径，
      未关闭（exit != 0）时登记为**阻塞项**并强制约束 V1 结论口径（TXT 桌面可用 + Android 可构建；
      不得宣称 Android 通过 / 全平台发布）。阻塞项不改变本脚本退出码（退出码语义 = 发布清单是否完整），
      发布裁定由 `results/gates/gate-final.md` 承担。

本脚本**不做构建**（不跑 cargo/gradle）：它只读取既有产物与配置，因此可在任意时刻重复执行。
退出码：0 = 清单完整且无未验证项被写成通过；1 = 缺失/不一致/未解析许可；2 = 前提缺失（目标平台产物目录不存在）。
`--self-test`：合成 fixture 逐条**反向**用例（跨文件核对、许可解析、文档宣称、ELF 剥离判定）。
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]

# --------------------------------------------------------------------------- #
# 冻结口径
# --------------------------------------------------------------------------- #

APK_VERIFIER = "scripts/verify-android-apk.py"           # AN6 交付：APK 内容/签名独立解析
DEVICE_READINESS = "scripts/verify-android-device-readiness.py"  # AN6 交付：真机延期就绪度
ANDROID_BOUNDARY_GATE = "scripts/verify-android-boundary.py"     # AN2 交付：Kotlin 侧锁/业务边界门禁
ANDROID_BOUNDARY_HEADER = "include/novel_reader.h"
ANDROID_BOUNDARY_KOTLIN = "android/app/src/main/java"
BUILD_INFO_REL = "dist/BUILD-INFO.md"
DIST_DIR_REL = "dist"
DESKTOP_BINARY_REL = "crates/desktop-app/target/release/desktop-app"
DESKTOP_APP_CARGO_REL = "crates/desktop-app/Cargo.toml"
DESKTOP_LOCK_REL = "crates/desktop-app/Cargo.lock"
ROOT_LOCK_REL = "Cargo.lock"
ANDROID_GRADLE_REL = "android/app/build.gradle.kts"
ANDROID_ROOT_GRADLE_REL = "android/build.gradle.kts"
ANDROID_WRAPPER_REL = "android/gradle/wrapper/gradle-wrapper.properties"
ANDROID_JNI_SO_REL = "android/app/src/main/jniLibs/{abi}/libnr_jni.so"
DESKTOP_TARGETS_REL = "desktop/packaging/targets.json"
DOCS_RELEASE_REL = "docs/release"

#: 桌面发布目录命名（desktop/packaging/targets.json 的 release_dir_name_pattern 口径）。
DESKTOP_RELEASE_DIR_GLOB = "novel-reader-*-linux-*-*"

#: 发布文档必须齐全的文件。
RELEASE_DOC_FILES: tuple[str, ...] = (
    "README.md",
    "release-notes-template.md",
    "known-limitations.md",
)

#: 发布文档必须覆盖的内容（正则；跨 `docs/release/**/*.md` 合并文本判定）。
RELEASE_DOC_PATTERNS: dict[str, str] = {
    "门禁命令（verify-v1-release.py）": r"verify-v1-release\.py",
    "门禁命令（verify-v1-structure.py）": r"verify-v1-structure\.py",
    "Android 真机待验证": r"真机[\s\S]{0,80}待验证|待验证[\s\S]{0,80}真机",
    "Windows 10/11 待验证": r"Windows\s*10/11",
    "Linux Wayland 待验证": r"Wayland",
    "未签名 Release APK 属预期": r"未签名",
    "发布前必须正式签名": r"正式签名|签名配置",
    "项目自身发行许可证延期未定": r"发行许可证|项目许可证",
    "已知限制章节": r"已知限制|已知问题",
    "延期验收项登记": r"延期",
    "未验证项清单": r"未验证",
    "回滚与数据备份": r"回滚",
    "数据备份说明": r"备份",
    "桌面发布目录未在干净发行版实测": r"干净(环境|目标发行版|发行版)",
    "文档精度项登记（gate-1 O1–O5/O7）": r"文档精度",
    "Cargo.lock 发布时哈希登记（gate-1 O6）": r"Cargo\.lock",
    "Android 传递依赖许可缺口": r"传递依赖",
    "阻塞项登记（AC-A2 边界门禁未关闭）": r"AC-A2|边界门禁",
    "发布物 SHA-256 记录要求": r"SHA-256|sha256",
}

#: 禁止事项：未实测不得写成的宣称。
FORBIDDEN_CLAIM_PATTERNS: tuple[tuple[str, str], ...] = (
    ("已真机验证", r"已真机验证"),
    ("真机验证通过", r"真机验证通过"),
    ("真机全部通过", r"真机(全部|全面)通过"),
    ("Android 全面通过", r"Android\s*(全面|全部)通过"),
    ("三平台发布完成", r"三平台(发布)?(全部)?完成"),
    ("V1 已发布", r"V1\s*已发布"),
)

#: 许可分类：宽松/许可式（可随包分发）与需人工复核的许可族。
PERMISSIVE_HINTS = ("MIT", "APACHE", "BSD", "ISC", "ZLIB", "UNICODE", "CC0", "UNLICENSE", "0BSD")
REVIEW_HINTS = ("GPL", "LGPL", "AGPL", "EPL", "ECLIPSE", "MPL", "CDDL", "CC-BY-SA", "SSPL", "BUSL")

#: Android 依赖 scope 语义（决定是否随 Release 包分发）。
ANDROID_SCOPE_MEANING = {
    "implementation": "随 Release 包分发",
    "api": "随 Release 包分发",
    "debugImplementation": "仅 Debug 包（不进 Release）",
    "releaseImplementation": "仅 Release 包（但不构建 Release 变体的 debug-only 路径）",
    "testImplementation": "仅 JVM 单测（不进包）",
    "androidTestImplementation": "仅真机 instrumentation（不进包）",
    "compileOnly": "仅编译期（不进包）",
}

MARKER_BUILDCONFIG_RE = re.compile(
    r'buildConfigField\(\s*"String"\s*,\s*"BUILD_MARKER"\s*,\s*"\\"([^"\\]+)\\""\s*\)'
)
BUILD_INFO_ROW_RE = re.compile(
    r"\|\s*`([^`]+\.apk)`\s*\|\s*(\d+)\s*\|\s*`([0-9a-fA-F]{64})`\s*\|([^|]*)\|"
)
BUILD_INFO_MARKER_RE = re.compile(r"BUILD_MARKER[`：:\s]*[`\"]?([A-Za-z0-9][A-Za-z0-9._-]{8,})[`\"]?")


class Report:
    """与仓库既有 verify-*.py 一致的成功/失败/跳过/事实四件套。"""

    def __init__(self) -> None:
        self.ok_count = 0
        self.failures: list[str] = []
        self.skips: list[str] = []
        self.facts: dict[str, object] = {}
        self.unverified: list[str] = []
        self.blockers: list[str] = []
        self.verifier_skips: list[str] = []

    def ok(self, label: str) -> None:
        self.ok_count += 1
        print(f"[ok]   {label}")

    def fail(self, label: str) -> None:
        self.failures.append(label)
        print(f"[FAIL] {label}")

    def skip(self, label: str) -> None:
        self.skips.append(label)
        print(f"[skip] {label}")

    def delegated_skip(self, label: str) -> None:
        """**被调用校验器**报告的跳过项（F-REL1b-2）。

        降级不得在发布门禁输出中不可见：既计入本门禁自己的跳过项，也单列在
        `verifier_skips` 里（摘要行与 JSON 均呈现），便于人工判定「校验器降级」。
        只影响可见性，不改变本门禁的通过/失败语义。
        """
        if label not in self.skips:
            self.skips.append(label)
        self.verifier_skips.append(label)
        print(f"[降级] {label}")

    def fact(self, key: str, value: object) -> None:
        self.facts[key] = value

    def unverified_item(self, label: str) -> None:
        if label not in self.unverified:
            self.unverified.append(label)
        print(f"[待验证] {label}")

    def blocker_item(self, label: str) -> None:
        """阻塞项：已实测取证但**尚未关闭**（区别于「延期未验证」）。

        阻塞项既不得计为通过，也不得被当作「已知限制」而滑过：它在摘要与 JSON 中单列，
        并强制约束 V1 结论口径。
        """
        if label not in self.blockers:
            self.blockers.append(label)
        print(f"[阻塞] {label}")


# --------------------------------------------------------------------------- #
# 通用工具
# --------------------------------------------------------------------------- #


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def rel(root: Path, path: Path) -> str:
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def parse_marker_from_gradle_text(text: str) -> str | None:
    m = MARKER_BUILDCONFIG_RE.search(text)
    return m.group(1) if m else None


def parse_marker_from_gradle(path: Path) -> str | None:
    if not path.is_file():
        return None
    return parse_marker_from_gradle_text(read_text(path))


def parse_workspace_version(text: str) -> str | None:
    block = text.split("[workspace.package]", 1)
    if len(block) < 2:
        return None
    m = re.search(r'^\s*version\s*=\s*"([^"]+)"', block[1], re.M)
    return m.group(1) if m else None


def parse_package_version(text: str) -> str | None:
    block = text.split("[package]", 1)
    if len(block) < 2:
        return None
    m = re.search(r'^\s*version\s*=\s*"([^"]+)"', block[1], re.M)
    return m.group(1) if m else None


def parse_android_values(gradle_text: str) -> dict[str, str]:
    """从 app/build.gradle.kts 抽取版本 / SDK / 工具链 / 构建标记。"""
    out: dict[str, str] = {}
    patterns = {
        "application_id": r'applicationId\s*=\s*"([^"]+)"',
        "version_code": r"versionCode\s*=\s*(\d+)",
        "version_name": r'versionName\s*=\s*"([^"]+)"',
        "min_sdk": r"minSdk\s*=\s*(\d+)",
        "target_sdk": r"targetSdk\s*=\s*(\d+)",
        "compile_sdk": r"compileSdk\s*=\s*(\d+)",
        "build_tools": r'buildToolsVersion\s*=\s*"([^"]+)"',
        "ndk_version": r'ndkVersion\s*=\s*"([^"]+)"',
        "abi": r'val\s+nrAbi\s*=\s*"([^"]+)"',
    }
    for key, pattern in patterns.items():
        m = re.search(pattern, gradle_text)
        if m:
            out[key] = m.group(1)
    marker = MARKER_BUILDCONFIG_RE.search(gradle_text)
    if marker:
        out["build_marker"] = marker.group(1)
    return out


def parse_android_plugin_versions(gradle_text: str) -> dict[str, str]:
    out: dict[str, str] = {}
    for plugin, key in (("com.android.application", "agp"), ("org.jetbrains.kotlin.android", "kotlin")):
        m = re.search(rf'id\("{re.escape(plugin)}"\)\s*version\s*"?([0-9][0-9.]*)"?', gradle_text)
        if m:
            out[key] = m.group(1)
    return out


def parse_gradle_wrapper_version(text: str) -> str | None:
    m = re.search(r"gradle-([0-9][0-9.]*)-(?:bin|all)\.zip", text)
    return m.group(1) if m else None


def parse_rust_toolchain(text: str) -> dict[str, object]:
    channel = re.search(r'^channel\s*=\s*"([^"]+)"', text, re.M)
    targets = re.search(r"^targets\s*=\s*\[(.*?)\]", text, re.M | re.S)
    return {
        "channel": channel.group(1) if channel else None,
        "targets": re.findall(r'"([^"]+)"', targets.group(1)) if targets else [],
    }


def parse_lock_packages(lock_text: str, lock_path: str) -> list[dict[str, str]]:
    out: list[dict[str, str]] = []
    for block in lock_text.split("[[package]]")[1:]:
        name = re.search(r'^name\s*=\s*"([^"]+)"', block.strip(), re.M)
        version = re.search(r'^version\s*=\s*"([^"]+)"', block.strip(), re.M)
        if name and version:
            out.append({"name": name.group(1), "version": version.group(1), "lock": lock_path})
    return out


# --------------------------------------------------------------------------- #
# ELF 解析（纯 Python：不依赖 readelf/file）
# --------------------------------------------------------------------------- #

ELF_MACHINES = {0x03: "i386", 0x28: "arm", 0xB7: "aarch64", 0x3E: "x86_64"}
ELF_TYPES = {1: "REL", 2: "EXEC", 3: "DYN", 4: "CORE"}


def parse_elf(data: bytes) -> dict[str, object]:
    """解析 ELF 头 / 节名 / `.note.gnu.build-id` / `DT_NEEDED`。

    返回 dict；非 ELF 或截断时抛 ValueError（调用方按失败处理，不静默通过）。
    """
    if len(data) < 64 or data[:4] != b"\x7fELF":
        raise ValueError("不是 ELF 文件")
    ei_class = data[4]
    ei_data = data[5]
    if ei_class != 2:
        raise ValueError(f"仅支持 64 位 ELF（ei_class={ei_class}）")
    endian = "<" if ei_data == 1 else ">"

    e_type = struct.unpack_from(endian + "H", data, 16)[0]
    e_machine = struct.unpack_from(endian + "H", data, 18)[0]
    e_phoff = struct.unpack_from(endian + "Q", data, 32)[0]
    e_phentsize = struct.unpack_from(endian + "H", data, 54)[0]
    e_phnum = struct.unpack_from(endian + "H", data, 56)[0]
    e_shoff = struct.unpack_from(endian + "Q", data, 40)[0]
    e_shentsize = struct.unpack_from(endian + "H", data, 58)[0]
    e_shnum = struct.unpack_from(endian + "H", data, 60)[0]
    e_shstrndx = struct.unpack_from(endian + "H", data, 62)[0]

    def section(i: int) -> dict[str, int]:
        off = e_shoff + i * e_shentsize
        name, stype = struct.unpack_from(endian + "II", data, off)
        addr, offset, size = struct.unpack_from(endian + "QQQ", data, off + 16)
        return {"name_off": name, "type": stype, "addr": addr, "offset": offset, "size": size}

    sections = [section(i) for i in range(e_shnum)]
    shstr = sections[e_shstrndx]
    shstr_data = data[shstr["offset"] : shstr["offset"] + shstr["size"]]

    def cstr(buf: bytes, off: int) -> str:
        end = buf.find(b"\x00", off)
        return buf[off : end if end != -1 else len(buf)].decode("utf-8", "replace")

    names: list[str] = []
    for s in sections:
        names.append(cstr(shstr_data, s["name_off"]) if s["name_off"] < len(shstr_data) else "")

    result: dict[str, object] = {
        "class": "ELF64",
        "endian": "little" if ei_data == 1 else "big",
        "type": ELF_TYPES.get(e_type, f"0x{e_type:x}"),
        "machine": ELF_MACHINES.get(e_machine, f"0x{e_machine:x}"),
        "sections": len(names),
        "section_names": names,
        "has_symtab": ".symtab" in names,
        "debug_sections": [n for n in names if n.startswith(".debug")],
        "build_id": None,
        "needed": [],
    }

    # .note.gnu.build-id
    if ".note.gnu.build-id" in names:
        s = sections[names.index(".note.gnu.build-id")]
        note = data[s["offset"] : s["offset"] + s["size"]]
        if len(note) >= 12:
            namesz, descsz, _ntype = struct.unpack_from(endian + "III", note, 0)
            desc_off = 12 + ((namesz + 3) // 4) * 4
            desc = note[desc_off : desc_off + descsz]
            result["build_id"] = desc.hex()

    # DT_NEEDED（经 PT_DYNAMIC + .dynstr；DT_NEEDED 的 value = 字符串表内偏移）
    if e_phoff and e_phnum:
        for i in range(e_phnum):
            off = e_phoff + i * e_phentsize
            p_type = struct.unpack_from(endian + "I", data, off)[0]
            if p_type != 2:  # PT_DYNAMIC
                continue
            p_offset, p_vaddr = struct.unpack_from(endian + "QQ", data, off + 8)
            p_filesz = struct.unpack_from(endian + "Q", data, off + 32)[0]
            dyn = data[p_offset : p_offset + p_filesz]
            entries: list[tuple[int, int]] = []
            dynstr_addr = None
            for j in range(0, len(dyn) - 16 + 1, 16):
                tag, val = struct.unpack_from(endian + "qQ", dyn, j)
                if tag == 0:
                    break
                entries.append((tag, val))
                if tag == 5:  # DT_STRTAB
                    dynstr_addr = val
            # 定位字符串表：优先用 .dynstr 节表；否则用 DT_STRTAB 的 vaddr 映射到文件偏移
            strtab_off: int | None = None
            strtab_size = 0
            if ".dynstr" in names:
                ds = sections[names.index(".dynstr")]
                strtab_off, strtab_size = ds["offset"], ds["size"]
            elif dynstr_addr is not None:
                for s in sections:
                    if s["type"] == 3 and s["addr"] <= dynstr_addr < s["addr"] + s["size"]:
                        strtab_off = s["offset"] + (dynstr_addr - s["addr"])
                        strtab_size = s["size"]
                        break
            if strtab_off is None:
                break
            buf = data[strtab_off : strtab_off + strtab_size]
            for tag, val in entries:
                if tag == 1 and 0 <= val < len(buf):  # DT_NEEDED
                    result["needed"].append(cstr(buf, val))
            break
    return result


def build_synthetic_elf(*, with_symtab: bool, build_id: bytes | None = None) -> bytes:
    """构造最小 ELF64（自测用）：验证「符号剥离判定」确有反面能力。

    注意：节头里的 `sh_name` 是 **.shstrtab 内的字节偏移**（不是节序号），本函数按字节偏移构造。
    """
    names = ["", ".shstrtab", ".dynstr"]
    if with_symtab:
        names.append(".symtab")
    if build_id is not None:
        names.append(".note.gnu.build-id")
    shstrtab = b""
    name_off: dict[str, int] = {}
    for n in names:
        name_off[n] = len(shstrtab)
        shstrtab += n.encode() + b"\x00"
    dynstr = b"\x00libc.so.6\x00"
    symtab = b"\x00" * 24
    note = b""
    if build_id is not None:
        note = struct.pack("<III", 4, len(build_id), 3) + b"GNU\x00" + build_id
        note += b"\x00" * ((4 - len(build_id) % 4) % 4)

    ehsize = 64
    off = ehsize
    offsets: dict[str, int] = {}
    for n, payload in (
        (".shstrtab", shstrtab),
        (".dynstr", dynstr),
        (".symtab", symtab),
        (".note.gnu.build-id", note),
    ):
        if n in names:
            offsets[n] = off
            off += len(payload)
    shoff = off
    shnum = len(names)
    shentsize = 64
    total = shoff + shnum * shentsize

    def shdr(name_offset: int, stype: int, s_off: int, size: int) -> bytes:
        return struct.pack("<IIQQQQIIQQ", name_offset, stype, 0, 0, s_off, size, 0, 0, 1, 0)

    hdrs = [shdr(0, 0, 0, 0)]
    hdrs.append(shdr(name_off[".shstrtab"], 3, offsets[".shstrtab"], len(shstrtab)))
    hdrs.append(shdr(name_off[".dynstr"], 3, offsets[".dynstr"], len(dynstr)))
    if ".symtab" in names:
        hdrs.append(shdr(name_off[".symtab"], 2, offsets[".symtab"], len(symtab)))
    if ".note.gnu.build-id" in names:
        hdrs.append(
            shdr(name_off[".note.gnu.build-id"], 7, offsets[".note.gnu.build-id"], len(note))
        )

    ehdr = bytearray(64)
    ehdr[0:4] = b"\x7fELF"
    ehdr[4] = 2  # ELF64
    ehdr[5] = 1  # little endian
    ehdr[6] = 1  # version
    struct.pack_into("<HHI", ehdr, 16, 3, 0x3E, 1)  # ET_DYN / x86_64 / version
    struct.pack_into("<Q", ehdr, 40, shoff)
    struct.pack_into("<HHH", ehdr, 58, shentsize, shnum, 1)

    body = bytearray(total)
    body[0:64] = ehdr
    for n, payload in (
        (".shstrtab", shstrtab),
        (".dynstr", dynstr),
        (".symtab", symtab),
        (".note.gnu.build-id", note),
    ):
        if n in names:
            body[offsets[n] : offsets[n] + len(payload)] = payload
    for i, hdr in enumerate(hdrs):
        body[shoff + i * shentsize : shoff + (i + 1) * shentsize] = hdr
    return bytes(body)


# --------------------------------------------------------------------------- #
# 依赖许可解析
# --------------------------------------------------------------------------- #


def local_registry_roots() -> list[Path]:
    cargo_home = Path(os.environ.get("CARGO_HOME") or (Path.home() / ".cargo"))
    roots: list[Path] = []
    src = cargo_home / "registry" / "src"
    if src.is_dir():
        roots.extend(sorted(d for d in src.iterdir() if d.is_dir()))
    return roots


def license_from_cargo_toml(path: Path) -> str | None:
    text = read_text(path)
    m = re.search(r'^\s*license\s*=\s*"([^"]+)"', text, re.M)
    if m:
        return m.group(1)
    if re.search(r'^\s*license-file\s*=', text, re.M):
        return "license-file（见 crate 内许可证文件）"
    return None


def compiled_crates(root: Path) -> tuple[set[tuple[str, str]], set[str]]:
    """从桌面 Release 构建的 dep-info（`target/release/deps/*.d`）提取**真实编译过**的依赖。

    依据是 Cargo 生成的依赖信息文件首行（源文件绝对路径），因此这是构建证据而不是猜测：
    - 返回 (registry 依赖 {(name, version)}, path crate 名集合)。
    - 本地 lock 里存在但**未在发布目标上编译**的包（Windows/macOS/iOS/Android 专用依赖）
      会被排除，它们不进入 Linux 发布物，因而不构成本次发布的许可缺口。
    """
    deps_dir = root / "crates/desktop-app/target/release/deps"
    registry: set[tuple[str, str]] = set()
    path_crates: set[str] = set()
    if not deps_dir.is_dir():
        return registry, path_crates
    reg_pat = re.compile(r"registry/src/[^/]+/([A-Za-z0-9_.+-]+?)-(\d[0-9A-Za-z.+-]*)/")
    path_pat = re.compile(re.escape(str(root)) + r"/(?:crates|desktop)/([A-Za-z0-9_-]+)/src/")
    dep_infos = sorted(deps_dir.glob("*.d")) + sorted((deps_dir.parent / "build").glob("*/*.d"))
    for dep_info in dep_infos:
        head = read_text(dep_info)[:4000]
        for m in reg_pat.finditer(head):
            registry.add((m.group(1), m.group(2)))
        for m in path_pat.finditer(head):
            path_crates.add(m.group(1))
    return registry, path_crates


def resolve_rust_packages(
    root: Path,
) -> tuple[list[dict[str, str]], list[dict[str, str]], dict[str, int]]:
    """逐 package 解析 license。

    返回 (resolved, unresolved, 溯源计数)。unresolved 项带 name/version/reason/lock，
    以便按「是否在发布目标上编译过」分流：编译过 → 硬失败；未编译（平台专用）→ 未验证项。
    """
    locks = [root / ROOT_LOCK_REL, root / DESKTOP_LOCK_REL]
    registry_roots = local_registry_roots()
    resolved: list[dict[str, str]] = []
    unresolved: list[dict[str, str]] = []
    sources = {"path-crate": 0, "local-registry": 0}
    seen: set[tuple[str, str]] = set()

    for lock in locks:
        if not lock.is_file():
            unresolved.append(
                {
                    "name": lock.name,
                    "version": "-",
                    "lock": rel(root, lock),
                    "reason": f"{rel(root, lock)} 不存在（无法解析该计划集依赖）",
                }
            )
            continue
        for pkg in parse_lock_packages(read_text(lock), rel(root, lock)):
            key = (pkg["name"], pkg["version"])
            if key in seen:
                continue
            seen.add(key)
            entry = dict(pkg)
            # 1) 工作区内的 path crate（内部 crate 通常 publish=false，无 license 字段）
            crate_dir = root / "crates" / pkg["name"]
            if (crate_dir / "Cargo.toml").is_file():
                lic = license_from_cargo_toml(crate_dir / "Cargo.toml")
                entry["source"] = f"path:{rel(root, crate_dir)}/Cargo.toml"
                sources["path-crate"] += 1
                entry["license"] = lic or "内部 crate（publish = false，未声明 license 字段）"
                resolved.append(entry)
                continue
            # 2) 本地 cargo registry（精确版本目录）
            found = None
            for reg in registry_roots:
                cand = reg / f"{pkg['name']}-{pkg['version']}"
                if (cand / "Cargo.toml").is_file():
                    found = cand / "Cargo.toml"
                    break
            if found is not None:
                lic = license_from_cargo_toml(found)
                entry["source"] = str(found)
                sources["local-registry"] += 1
                if lic:
                    entry["license"] = lic
                    resolved.append(entry)
                else:
                    unresolved.append(
                        {
                            "name": pkg["name"],
                            "version": pkg["version"],
                            "lock": pkg["lock"],
                            "reason": f"{found} 未声明 license 字段",
                        }
                    )
                continue
            unresolved.append(
                {
                    "name": pkg["name"],
                    "version": pkg["version"],
                    "lock": pkg["lock"],
                    "reason": "本地 registry 无对应目录（未在本机下载/编译）",
                }
            )
    return resolved, unresolved, sources


def resolve_android_dependencies(root: Path) -> tuple[list[dict[str, str]], list[str], dict[str, str]]:
    """解析 android/app/build.gradle.kts 直接声明依赖的许可证（来源：Gradle 缓存 POM）。"""
    gradle = root / ANDROID_GRADLE_REL
    if not gradle.is_file():
        return [], [f"{ANDROID_GRADLE_REL} 不存在"], {}
    deps = re.findall(r'^\s*(\w+)\(\s*"([^"]+)"\s*\)', read_text(gradle), re.M)
    cache = Path(os.environ.get("GRADLE_USER_HOME") or (Path.home() / ".gradle"))
    modules = cache / "caches" / "modules-2" / "files-2.1"
    resolved: list[dict[str, str]] = []
    unresolved: list[str] = []
    for cfg, coord in deps:
        parts = coord.split(":")
        if len(parts) < 3:
            continue  # 插件 id / 非坐标（如 getDefaultProguardFile）
        group, artifact, version = parts[0], parts[1], parts[2]
        entry = {
            "configuration": cfg,
            "scope": ANDROID_SCOPE_MEANING.get(cfg, f"未登记 scope({cfg})"),
            "coordinate": f"{group}:{artifact}:{version}",
        }
        d = modules / group / artifact / version
        poms = sorted(d.rglob("*.pom")) if d.is_dir() else []
        lic = None
        if poms:
            m = re.search(r"<license>\s*<name>([^<]+)</name>", read_text(poms[0]))
            lic = m.group(1).strip() if m else None
        entry["source"] = str(poms[0]) if poms else "本地 Gradle 缓存无该模块"
        if lic:
            entry["license"] = lic
            resolved.append(entry)
        else:
            unresolved.append(f"{entry['coordinate']}（{cfg}）：{entry['source']}")
    return resolved, unresolved, {"cache": str(modules)}


def classify_license(license_text: str) -> str:
    up = license_text.upper()
    if any(h in up for h in REVIEW_HINTS):
        return "需人工复核"
    if any(h in up for h in PERMISSIVE_HINTS):
        return "许可式（宽松）"
    if "内部 crate" in license_text:
        return "内部 crate"
    return "未分类"


# --------------------------------------------------------------------------- #
# 各节检查
# --------------------------------------------------------------------------- #


def section_versions(root: Path, rep: Report) -> dict[str, object]:
    print("[1] 版本与构建标识")
    ctx: dict[str, object] = {}

    root_cargo = root / "Cargo.toml"
    if not root_cargo.is_file():
        rep.fail("根 Cargo.toml 不存在（版本唯一来源缺失）")
        return ctx
    ws_version = parse_workspace_version(read_text(root_cargo))
    ctx["workspace_version"] = ws_version
    if ws_version:
        rep.ok(f"Rust workspace 版本 = {ws_version}")
    else:
        rep.fail("根 Cargo.toml 未声明 [workspace.package] version")

    desktop_cargo = root / DESKTOP_APP_CARGO_REL
    desktop_version = parse_package_version(read_text(desktop_cargo)) if desktop_cargo.is_file() else None
    ctx["desktop_version"] = desktop_version
    if desktop_version and ws_version:
        if desktop_version == ws_version:
            rep.ok(f"桌面 crate 版本与 workspace 一致：{desktop_version}")
        else:
            rep.fail(f"桌面 crate 版本 {desktop_version} ≠ workspace {ws_version}（发布物版本口径分叉）")
    else:
        rep.fail(f"无法解析桌面 crate 版本（{DESKTOP_APP_CARGO_REL}）")

    # Android 侧版本 / SDK / 工具链
    gradle = root / ANDROID_GRADLE_REL
    android = parse_android_values(read_text(gradle)) if gradle.is_file() else {}
    ctx["android"] = android
    if not android:
        rep.fail(f"无法解析 Android 版本与 SDK 口径（{ANDROID_GRADLE_REL}）")
    else:
        rep.ok(
            "Android 版本口径：applicationId={application_id} versionName={version_name} "
            "versionCode={version_code}".format(**android)
        )
        rep.ok(
            "Android SDK/ABI：minSdk={min_sdk} targetSdk={target_sdk} "
            "compileSdk={compile_sdk} ABI={abi}".format(**android)
        )

    wrapper = root / ANDROID_WRAPPER_REL
    gradle_version = parse_gradle_wrapper_version(read_text(wrapper)) if wrapper.is_file() else None
    plugins = (
        parse_android_plugin_versions(read_text(root / ANDROID_ROOT_GRADLE_REL))
        if (root / ANDROID_ROOT_GRADLE_REL).is_file()
        else {}
    )
    toolchain = (
        parse_rust_toolchain(read_text(root / "rust-toolchain.toml"))
        if (root / "rust-toolchain.toml").is_file()
        else {}
    )
    ctx["toolchain"] = {
        "rust": toolchain.get("channel"),
        "gradle": gradle_version,
        **plugins,
        "ndk": android.get("ndk_version"),
        "build_tools": android.get("build_tools"),
    }
    print(
        "       工具链："
        f"rust={toolchain.get('channel')} gradle={gradle_version} "
        f"agp={plugins.get('agp')} kotlin={plugins.get('kotlin')} "
        f"ndk={android.get('ndk_version')} build-tools={android.get('build_tools')}"
    )
    if toolchain.get("channel"):
        rep.ok(f"Rust 工具链锁定：{toolchain.get('channel')}（targets={toolchain.get('targets')}）")
    else:
        rep.fail("rust-toolchain.toml 未锁定 channel")

    # Cargo.lock 发布时哈希（gate-1-core.md O6：必须重新登记，不得沿用 RC6 旧值）
    lock = root / ROOT_LOCK_REL
    if lock.is_file():
        digest = sha256_file(lock)
        ctx["cargo_lock_sha256"] = digest
        ctx["cargo_lock_packages"] = len(parse_lock_packages(read_text(lock), ROOT_LOCK_REL))
        rep.ok(f"Cargo.lock 发布时哈希登记：{digest[:16]}…（{ctx['cargo_lock_packages']} 个 package）")
    else:
        rep.fail("Cargo.lock 缺失")

    marker = android.get("build_marker")
    ctx["gradle_build_marker"] = marker
    if marker:
        rep.ok(f"Android BUILD_MARKER（gradle）= {marker}")
    else:
        rep.fail("app/build.gradle.kts 未声明 BUILD_MARKER（构建标识链断裂）")
    return ctx


def section_desktop_artifacts(root: Path, rep: Report, ctx: dict[str, object]) -> None:
    print("[2] 发布物清单 —— 桌面（Linux x86_64 Release）")
    binary = root / DESKTOP_BINARY_REL
    if not binary.is_file():
        rep.fail(f"桌面 Release 二进制不存在：{DESKTOP_BINARY_REL}（先执行 cargo build --manifest-path {DESKTOP_APP_CARGO_REL} --release）")
        return
    size = binary.stat().st_size
    digest = sha256_file(binary)
    info: dict[str, object] = {
        "path": DESKTOP_BINARY_REL,
        "size": size,
        "sha256": digest,
        "mtime": __import__("datetime").datetime.fromtimestamp(binary.stat().st_mtime).isoformat(timespec="seconds"),
    }
    rep.ok(f"桌面 Release 二进制：{DESKTOP_BINARY_REL}（{size} B, sha256={digest[:16]}…）")

    try:
        elf = parse_elf(binary.read_bytes())
    except ValueError as exc:
        rep.fail(f"桌面二进制不是可解析的 ELF64：{exc}")
        return
    info["elf"] = {
        "class": elf["class"],
        "machine": elf["machine"],
        "type": elf["type"],
        "sections": elf["sections"],
        "build_id": elf["build_id"],
        "needed": elf["needed"],
        "debug_sections": elf["debug_sections"],
    }
    if elf["machine"] == "x86_64" and elf["type"] in {"EXEC", "DYN"}:
        rep.ok(f"ELF 属性：{elf['class']} / {elf['machine']} / {elf['type']} / build-id={elf['build_id']}")
    else:
        rep.fail(f"桌面二进制 ELF 目标不符：machine={elf['machine']} type={elf['type']}（期望 x86_64）")
    if elf["has_symtab"] or elf["debug_sections"]:
        rep.fail(
            "桌面 Release 未剥离符号："
            f".symtab={elf['has_symtab']} debug_sections={elf['debug_sections']}（NFR-D2 要求剥离）"
        )
    else:
        rep.ok("桌面 Release 已符号剥离（无 .symtab / .debug_*）")
    if elf["needed"]:
        rep.ok(f"DT_NEEDED 运行库 {len(elf['needed'])} 项：{', '.join(sorted(elf['needed']))}")
    else:
        rep.skip("未从 ELF 解析到 DT_NEEDED（静态链接或非常规布局）")

    ldd = shutil.which("ldd")
    if ldd:
        proc = subprocess.run([ldd, str(binary)], capture_output=True, text=True)
        if proc.returncode == 0:
            libs = sorted(
                {
                    line.split("=>")[0].strip().split("/")[-1]
                    for line in proc.stdout.splitlines()
                    if "=>" in line or "linux-vdso" in line
                }
            )
            info["ldd_build_machine"] = libs
            rep.ok(f"构建机 ldd 解析运行库 {len(libs)} 项（目标发行版仍需实测）")
        else:
            rep.skip(f"ldd 返回 {proc.returncode}，跳过运行库实测")
    else:
        rep.skip("本机无 ldd，跳过桌面运行库实测")

    # 桌面发布目录（完整发布目录口径；未产出时按「未产出」登记，不伪造）
    release_dirs = sorted(
        p for p in (root / DIST_DIR_REL).glob(DESKTOP_RELEASE_DIR_GLOB) if p.is_dir()
    ) if (root / DIST_DIR_REL).is_dir() else []
    if not release_dirs:
        command = "bash desktop/packaging/linux/package-linux.sh --backend x11 --out dist"
        rep.unverified_item(
            "桌面完整发布目录未在 dist/ 产出（口径：`"
            + command
            + "`；产出后本脚本会校验 VERSION 与 MANIFEST.sha256）"
        )
        ctx["desktop_release_dirs"] = []
        return

    ctx["desktop_release_dirs"] = [rel(root, d) for d in release_dirs]
    for d in release_dirs:
        version_file = d / "VERSION"
        manifest = d / "MANIFEST.sha256"
        if not version_file.is_file() or not manifest.is_file():
            rep.fail(f"{rel(root, d)}：缺少 VERSION 或 MANIFEST.sha256（完整发布目录口径）")
            continue
        bad: list[str] = []
        recorded = 0
        for line in read_text(manifest).splitlines():
            line = line.strip()
            if not line:
                continue
            m = re.match(r"^([0-9a-fA-F]{64})\s+\*?(.+)$", line)
            if not m:
                bad.append(f"MANIFEST 行无法解析: {line}")
                continue
            want, target = m.group(1).lower(), d / m.group(2)
            recorded += 1
            if not target.is_file():
                bad.append(f"MANIFEST 指向的文件不存在: {m.group(2)}")
            elif sha256_file(target) != want:
                bad.append(f"SHA-256 不符: {m.group(2)}")
        if bad:
            rep.fail(f"{rel(root, d)} 发布目录自校验失败：" + "；".join(bad[:5]))
        else:
            rep.ok(f"{rel(root, d)}：MANIFEST.sha256 自校验通过（{recorded} 个文件逐字节一致）")
        kv = dict(
            line.split("=", 1)
            for line in read_text(version_file).splitlines()
            if "=" in line and not line.startswith("#")
        )
        if kv.get("version"):
            rep.ok(f"{rel(root, d)} VERSION: version={kv.get('version')} target={kv.get('target_triple') or kv.get('target')}")
            if ctx.get("workspace_version") and kv["version"] != str(ctx["workspace_version"]):
                rep.fail(
                    f"{rel(root, d)} VERSION 记录 {kv['version']} ≠ workspace {ctx['workspace_version']}"
                )
        else:
            rep.fail(f"{rel(root, d)} VERSION 未记录 version 字段")


def section_android_artifacts(root: Path, rep: Report, ctx: dict[str, object]) -> None:
    print("[3] 发布物清单 —— Android（dist/*.apk）")
    dist = root / DIST_DIR_REL
    apks = sorted(p for p in dist.glob("*.apk") if p.is_file()) if dist.is_dir() else []
    if not apks:
        rep.fail(f"{DIST_DIR_REL}/ 下没有 APK 分发包（A-T6/AN6 产物缺失）")
        return

    apk_records: list[dict[str, object]] = []
    for apk in apks:
        record: dict[str, object] = {
            "path": rel(root, apk),
            "size": apk.stat().st_size,
            "sha256": sha256_file(apk),
        }
        verifier = root / APK_VERIFIER
        if verifier.is_file():
            with tempfile.TemporaryDirectory() as td:
                json_out = Path(td) / "apk.json"
                cmd = [sys.executable, str(verifier), str(apk), "--json", str(json_out)]
                abi = str((ctx.get("android") or {}).get("abi") or "arm64-v8a")  # type: ignore[union-attr]
                cmd += ["--abi", abi]
                proc = subprocess.run(cmd, capture_output=True, text=True)
                if not json_out.is_file():
                    rep.fail(f"{rel(root, apk)}：{APK_VERIFIER} 未产出 JSON（exit={proc.returncode}）")
                    apk_records.append(record)
                    continue
                facts = json.loads(json_out.read_text(encoding="utf-8"))
                # F-REL1b-2：消费被调用校验器的跳过项（含 SDK 根探测降级），
                # 不允许「校验器跳过了 N 项」在本门禁输出里不可见。
                verifier_skips = facts.get("skips") or []
                sdk_probe = (facts.get("facts") or {}).get("sdk_probe")
        else:
            rep.fail(f"缺少 {APK_VERIFIER}（无法独立解析 APK 内容与签名）")
            apk_records.append(record)
            continue

        record["verifier_skips"] = verifier_skips
        record["sdk_probe"] = sdk_probe
        # 每一条校验器跳过项都在本门禁输出中单列（含 SDK 根探测降级：此时校验器的 skip 文案
        # 已如实列出「已试位置」，此处不再重复派生，避免跳过项计数被重复放大）。
        for item in verifier_skips:
            rep.delegated_skip(f"{record['path']}：{APK_VERIFIER} 跳过项 → {item}")

        record["verifier_result"] = facts.get("result")
        record["verifier_failures"] = facts.get("failures")
        af = facts.get("facts", {})
        record["marker"] = af.get("marker")
        record["version_name"] = (af.get("manifest") or {}).get("version_name")
        record["version_code"] = (af.get("manifest") or {}).get("version_code")
        record["min_sdk"] = (af.get("manifest") or {}).get("min_sdk")
        record["target_sdk"] = (af.get("manifest") or {}).get("target_sdk")
        record["signed"] = (af.get("signature") or {}).get("signed")
        record["signer"] = af.get("signer_dn")
        record["jni_sha256"] = (af.get("jni_lib") or {}).get("sha256")
        record["dex_count"] = af.get("dex_count")
        apk_records.append(record)

        state = "已签名" if record["signed"] else "未签名"
        rep.ok(
            f"{record['path']}：{record['size']} B sha256={str(record['sha256'])[:16]}… 签名={state} "
            f"versionName={record['version_name']} marker={record['marker']}"
        )
        if facts.get("result") != "PASS":
            rep.fail(f"{record['path']}：{APK_VERIFIER} 判定 {facts.get('result')}（{facts.get('failures')}）")

        android = ctx.get("android") or {}
        if record["version_name"] and android.get("version_name") and record["version_name"] != android["version_name"]:  # type: ignore[union-attr]
            rep.fail(
                f"{record['path']} versionName {record['version_name']} ≠ gradle {android['version_name']}（产物过期）"  # type: ignore[index]
            )
        if str(record["version_code"]) != str(android.get("version_code")):  # type: ignore[union-attr]
            rep.fail(
                f"{record['path']} versionCode {record['version_code']} ≠ gradle {android.get('version_code')}"  # type: ignore[union-attr]
            )
        if record["marker"] != ctx.get("gradle_build_marker"):
            rep.fail(
                f"{record['path']} 内嵌 BUILD_MARKER {record['marker']} ≠ gradle {ctx.get('gradle_build_marker')}"
            )

        # APK 内 JNI .so 必须与源码树里的预编译 .so 同源
        abi = str(android.get("abi") or "arm64-v8a")  # type: ignore[union-attr]
        src_so = root / ANDROID_JNI_SO_REL.format(abi=abi)
        if src_so.is_file() and record["jni_sha256"]:
            if sha256_file(src_so) == record["jni_sha256"]:
                rep.ok(f"{record['path']} 内 JNI .so 与 {rel(root, src_so)} 同源（sha256 一致）")
            else:
                rep.fail(f"{record['path']} 内 JNI .so 与源码树预编译 .so sha256 不一致（包体与源码树不对应）")
        else:
            rep.skip(f"{record['path']}：未做 JNI .so 同源核对（缺源码树 .so 或解析值）")

    ctx["apks"] = apk_records

    # dist/BUILD-INFO.md 跨文件逐项核对（与自测共用同一实现）
    _verify_build_info(rep, root, ctx)
    unsigned = [r for r in apk_records if r.get("signed") is False]
    for r in unsigned:
        rep.unverified_item(
            f"{r['path']} 未签名（无正式签名配置，属当前预期；规格 §15.2 要求 Release 使用正式签名，"
            "签名完成前不得作为发布包分发）"
        )


def check_license_coverage(
    rep: Report, root: Path, resolved: list[dict[str, str]], unresolved: list[dict[str, str]]
) -> None:
    """用构建证据（dep-info 真实编译过的依赖）核对许可覆盖，并分流未解析项。

    匹配为**版本精确**（同一 crate 在 lock 里可能有多个版本，只有编译进发布物的那个版本
    才算覆盖；用名字放宽会掩盖版本级缺口）。
    """
    compiled, path_crates = compiled_crates(root)
    resolved_keys = {(e["name"], e["version"]) for e in resolved}
    resolved_names = {e["name"] for e in resolved}

    if not compiled and not path_crates:
        rep.skip("缺少桌面 Release 构建证据（target/release/deps/*.d），跳过「编译依赖许可覆盖」核对")
        return

    uncovered = [f"{n}-{v}" for n, v in sorted(compiled) if (n, v) not in resolved_keys]
    if uncovered:
        rep.fail(
            f"发布物实际编译依赖中有 {len(uncovered)} 个没有许可条目：{uncovered[:8]}（许可清单不得缺失发布物依赖）"
        )
    else:
        rep.ok(
            f"许可覆盖核对（版本精确）：Release 构建实际编译的 {len(compiled)} 个 registry 依赖全部有许可条目"
        )

    path_uncovered = sorted(n for n in path_crates if n not in resolved_names)
    if path_uncovered:
        rep.fail(f"path crate 缺许可条目：{path_uncovered}")
    else:
        rep.ok(f"许可覆盖核对：path crate {sorted(path_crates)} 均有许可条目")

    host_unresolved = [u for u in unresolved if (u["name"], u["version"]) in compiled]
    platform_unresolved = [u for u in unresolved if u not in host_unresolved]
    if host_unresolved:
        rep.fail(
            "发布目标上实际编译的依赖存在未解析许可："
            + "；".join(f"{u['name']}-{u['version']}（{u['reason']}）" for u in host_unresolved[:8])
        )
    if platform_unresolved:
        sample = "、".join(f"{u['name']}-{u['version']}" for u in platform_unresolved[:8])
        rep.unverified_item(
            f"平台专用依赖（Windows/macOS/iOS/Android）许可未在本地解析：{len(platform_unresolved)} 个"
            f"（示例：{sample} …）；依据 crates/desktop-app/target/release/deps/*.d 的构建证据，"
            "这些包**未编译进 Linux 发布物**，因此不构成本次发布的许可缺口；发布对应平台目标时必须重新解析"
        )


def section_licenses(root: Path, rep: Report, ctx: dict[str, object]) -> None:
    print("[4] 依赖与许可证清单")
    resolved, unresolved, sources = resolve_rust_packages(root)
    ctx["rust_licenses"] = resolved
    dist: dict[str, int] = {}
    for e in resolved:
        dist[e["license"]] = dist.get(e["license"], 0) + 1
    print(f"     Rust（根 + 桌面 workspace 的 Cargo.lock 全量，{len(resolved)} 个 package；溯源={sources}）：")
    for lic, count in sorted(dist.items(), key=lambda kv: -kv[1]):
        print(f"       - {lic} × {count}  [{classify_license(lic)}]")
    rep.ok(f"Rust 依赖许可解析：{len(resolved)} 个 package 有许可条目，{len(unresolved)} 个未解析")
    flagged = {lic for lic in dist if classify_license(lic) == "需人工复核"}
    if flagged:
        print(f"       ! 需人工复核的许可族：{sorted(flagged)}")
    ctx["rust_license_distribution"] = dist
    check_license_coverage(rep, root, resolved, unresolved)

    android_resolved, android_unresolved, cache_facts = resolve_android_dependencies(root)
    ctx["android_licenses"] = android_resolved
    print(f"     Android（build.gradle.kts 直接声明 {len(android_resolved) + len(android_unresolved)} 个坐标）：")
    for e in android_resolved:
        print(f"       - {e['coordinate']:55s} {e['license']:42s} [{e['scope']}]")
    if android_unresolved:
        rep.fail(
            "Android 许可清单未解析项（本地 Gradle 缓存缺 POM；来源："
            + str(cache_facts.get("cache"))
            + "）："
            + "；".join(android_unresolved[:8])
        )
    else:
        rep.ok(f"Android 直接声明依赖许可全部解析（{len(android_resolved)} 个坐标，0 未解析）")
    android_flagged = {e["license"] for e in android_resolved if classify_license(e["license"]) == "需人工复核"}
    if android_flagged:
        print(f"       ! 需人工复核的许可族：{sorted(android_flagged)}")

    # 项目自身许可证（规格延期项）
    print("     项目自身发行许可证：未定（规格「开放问题与延期决定」登记为延期项）")
    rep.unverified_item("项目自身发行许可证未定（规格延期项；发布前必须确定并写入发布说明）")
    # Android 传递依赖许可：**已闭环**（2026-10-02）。本门禁不再无条件登记该项为未验证，
    # 改为**按实际产物核对**：若 results/verify/android-transitive-licenses.md 存在且已登记，
    # 则只把「查不到许可声明」的模块作为剩余人工核对项列出（据实，不沿用旧文案）。
    _transitive = root / "results" / "verify" / "android-transitive-licenses.md"
    _transitive_text = _transitive.read_text(encoding="utf-8") if _transitive.is_file() else ""
    if _transitive_text:
        # 模块坐标：group:artifact:version。版本允许点号/连字符（1.0 / 2.8.7 / 1.9.0）。
        _mod_re = r"`([A-Za-z0-9_.\-]+:[A-Za-z0-9_.\-]+:[0-9][A-Za-z0-9_.\-]*)`"
        _all_mods = sorted(set(re.findall(_mod_re, _transitive_text)))
        # 「未声明」行判定：按**行内**许可列标记识别，不依赖行首格式
        #（实测清单表格行首带序号「| 76 | `模块` | **未声明** | …」，用 startswith("| `") 会漏掉）。
        _undeclared_mods = [
            m.group(1)
            for ln in _transitive_text.splitlines()
            if ln.startswith("|") and "**未声明**" in ln
            for m in [re.search(_mod_re, ln)]
            if m
        ]
        print(f"     Android 传递依赖许可：已闭环（清单 {_transitive.relative_to(root)}）")
        rep.ok(
            f"Android 传递依赖许可清单已登记（{len(_all_mods)} 个模块，"
            f"其中「未声明」{len(_undeclared_mods)} 个）"
        )
        if _undeclared_mods:
            rep.unverified_item(
                "Android 传递依赖许可「未声明」项需人工核对上游 LICENSE："
                + "、".join(_undeclared_mods)
                + "（构建产物中查不到许可声明；核对前不得宣称「全部 Apache-2.0」。"
                "全量清单见 results/verify/android-transitive-licenses.md）"
            )
    else:
        rep.unverified_item(
            "Android 传递依赖许可未逐项登记（本门禁只覆盖 build.gradle.kts 直接声明；"
            "闭环命令：cd android && ./gradlew :app:dependencies --configuration releaseRuntimeClasspath）"
        )


def section_release_docs(root: Path, rep: Report, ctx: dict[str, object]) -> None:
    print("[5] 发布文档与已知限制登记（docs/release/）")
    docs_dir = root / DOCS_RELEASE_REL
    if not docs_dir.is_dir():
        rep.fail(f"缺少发布文档目录 {DOCS_RELEASE_REL}/（发布说明模板与已知限制未登记）")
        return
    files = sorted(docs_dir.rglob("*.md"))
    ctx["release_docs"] = [rel(root, p) for p in files]
    _evaluate_release_docs(rep, docs_dir)
    corpus = "\n".join(read_text(p) for p in files)
    ctx["release_docs_pending_mentions"] = len(re.findall(r"待验证", corpus))


def check_android_boundary_gate(root: Path, rep: Report) -> None:
    """AC-A2 边界门禁（Kotlin 侧锁规则 / 业务边界）的**真实执行**登记。

    口径（`results/gates/gate-3-android.md` §4 的 manager 裁定 + 上游 Gate-3 HOLD 结论）：
      * 默认口径 exit 0 → AC-A2 关闭，可 `rep.ok`；
      * 默认口径 exit != 0 → AC-A2 **未关闭**，登记为阻塞项（**不得**计为通过，也**不得**让本门禁
        因此 exit 1 —— 本脚本的退出码语义是「发布清单是否完整」，发布裁定由 gate-final.md 承担）；
      * 门禁脚本缺失 → 结构性缺陷，硬失败。
    """
    gate = root / ANDROID_BOUNDARY_GATE
    if not gate.is_file():
        rep.fail(f"缺少 {ANDROID_BOUNDARY_GATE}（AC-A2 边界门禁无法执行，边界未被取证）")
        return
    proc = subprocess.run(
        [
            sys.executable,
            str(gate),
            "--header",
            ANDROID_BOUNDARY_HEADER,
            "--kotlin",
            ANDROID_BOUNDARY_KOTLIN,
        ],
        capture_output=True,
        text=True,
        cwd=str(root),
    )
    verdict = [
        ln.strip()
        for ln in (proc.stdout or "").splitlines()
        if ln.strip().startswith("RESULT:") or ln.strip().startswith("- ")
    ]
    detail = " / ".join(verdict[:4])
    if proc.returncode == 0:
        rep.ok(
            f"AC-A2 Android 边界门禁默认口径 exit 0（{ANDROID_BOUNDARY_GATE} "
            f"--header {ANDROID_BOUNDARY_HEADER} --kotlin {ANDROID_BOUNDARY_KOTLIN}）"
        )
        return
    rep.blocker_item(
        f"AC-A2 未关闭（阻塞，非「已知限制」）：{ANDROID_BOUNDARY_GATE} 默认口径 exit={proc.returncode}，"
        f"AC-A2 一律记为「未通过」，Android 结论只能是「可构建 / 可测试 / 可校验」，"
        f"不得宣称 Android 通过或全平台发布；待修复卡闭环后本项才可转为 ok。"
        + (f"门禁输出：{detail}" if detail else "")
    )


def section_unverified(root: Path, rep: Report, ctx: dict[str, object]) -> None:
    print("[6] 未验证项清单（不得计为通过）")

    # 6.1 桌面目标状态（单一事实来源：desktop/packaging/targets.json）
    targets_path = root / DESKTOP_TARGETS_REL
    if targets_path.is_file():
        try:
            targets = json.loads(read_text(targets_path))
        except json.JSONDecodeError as exc:
            rep.fail(f"{DESKTOP_TARGETS_REL} 不是合法 JSON：{exc}")
            targets = None
        if isinstance(targets, dict):
            ctx["desktop_targets"] = {
                t.get("id"): t.get("status") for t in targets.get("targets", [])
            }
            for t in targets.get("targets", []):
                if t.get("status") == "pending":
                    rep.unverified_item(
                        f"桌面目标 {t.get('id')} 状态 待验证：{t.get('blocking_reason') or '未记录原因'}"
                    )
                for item in t.get("unverified_items") or []:
                    rep.unverified_item(f"桌面目标 {t.get('id')} 未验证项：{item}")
    else:
        rep.fail(f"缺少 {DESKTOP_TARGETS_REL}（桌面目标状态无法登记）")

    # 6.2 Android 真机延期（复用 AN6 的就绪度门禁，读取真实状态而不是自述）
    readiness = root / DEVICE_READINESS
    if readiness.is_file():
        with tempfile.TemporaryDirectory() as td:
            json_out = Path(td) / "readiness.json"
            proc = subprocess.run(
                [
                    sys.executable,
                    str(readiness),
                    "--docs",
                    "docs/android",
                    "--results",
                    "results",
                    "--json",
                    str(json_out),
                ],
                capture_output=True,
                text=True,
                cwd=str(root),
            )
            if json_out.is_file():
                facts = json.loads(json_out.read_text(encoding="utf-8"))
                status = (facts.get("facts") or {}).get("evidence_status", "未知")
                ctx["android_device_evidence_status"] = status
                rep.unverified_item(
                    f"Android 真机项（SAF/旋转/后台恢复/系统返回/JNI 取消/功耗）保持 待验证；"
                    f"延期证据状态={status}（{DEVICE_READINESS} exit={proc.returncode}）"
                )
            else:
                rep.fail(f"{DEVICE_READINESS} 未产出 JSON（exit={proc.returncode}）")
    else:
        rep.fail(f"缺少 {DEVICE_READINESS}（真机延期状态无法登记）")

    # 6.3 Android 边界门禁（AC-A2）：真实执行，未关闭时登记为**阻塞项**
    check_android_boundary_gate(root, rep)

    # 6.4 记账类登记项
    rep.unverified_item(
        "Cargo.lock 记账偏差（gate-1-core.md O6）：本脚本已登记发布时哈希 "
        f"{str(ctx.get('cargo_lock_sha256'))[:16]}…，历史比对值见 results/gates/gate-1-core.md（不得沿用 RC6 旧值）"
    )
    rep.unverified_item(
        "已知文档精度项 O1–O5/O7（头文件措辞 / 存储层章节一致性裁定等）已在 known-limitations.md 登记，"
        "属不阻断放行的文档精度类，仍非「已验证修复」"
    )


# --------------------------------------------------------------------------- #
# 自测：合成 fixture 反向用例
# --------------------------------------------------------------------------- #


def self_test() -> list[str]:
    failures: list[str] = []
    print("[self-test] 合成 fixture 反向用例（跨文件核对 / 许可解析 / 文档宣称 / ELF 剥离）")

    def run(fn, *args, **kwargs) -> list[str]:
        import contextlib
        import io

        rep = Report()
        with contextlib.redirect_stdout(io.StringIO()):
            fn(rep, *args, **kwargs)
        return rep.failures

    with tempfile.TemporaryDirectory() as td:
        work = Path(td)

        # 1) BUILD-INFO ↔ 实际文件 跨文件核对
        def check_build_info(rep_: Report, dist: Path, build_info_text: str) -> None:
            ctx: dict[str, object] = {"gradle_build_marker": "an6-marker-1234567890"}
            (dist.parent / BUILD_INFO_REL).parent.mkdir(parents=True, exist_ok=True)
            (dist.parent / BUILD_INFO_REL).write_text(build_info_text, encoding="utf-8")
            root = dist.parent
            record = {
                "path": "dist/app-debug.apk",
                "size": (dist / "app-debug.apk").stat().st_size,
                "sha256": sha256_file(dist / "app-debug.apk"),
                "signed": True,
                "marker": "an6-marker-1234567890",
                "version_name": "1.0.0",
                "version_code": 1,
                "jni_sha256": None,
            }
            ctx["apks"] = [record]
            _verify_build_info(rep_, root, ctx)

        dist = work / "repo" / DIST_DIR_REL
        dist.mkdir(parents=True)
        (dist / "app-debug.apk").write_bytes(b"debug-apk")
        (dist / "app-release-unsigned.apk").write_bytes(b"release-apk")
        good_size = (dist / "app-debug.apk").stat().st_size
        good_sha = sha256_file(dist / "app-debug.apk")
        good_info = (
            "# 分发包\n\n- 构建标记 BUILD_MARKER：`an6-marker-1234567890`\n\n"
            "| 文件 | 大小(B) | sha256 | 签名状态 | 安装方式 |\n|---|---|---|---|---|\n"
            f"| `app-debug.apk` | {good_size} | `{good_sha}` | 已签名 | adb install |\n"
            f"| `app-release-unsigned.apk` | {(dist / 'app-release-unsigned.apk').stat().st_size} | "
            f"`{sha256_file(dist / 'app-release-unsigned.apk')}` | 未签名（属预期） | 需签名 |\n"
        )
        cases = [
            ("BUILD-INFO 记录一致", good_info, False),
            ("BUILD-INFO 大小不符", good_info.replace(str(good_size), str(good_size + 1)), True),
            ("BUILD-INFO SHA-256 不符", good_info.replace(good_sha, "0" * 64), True),
            ("BUILD-INFO 标记不一致", good_info.replace("an6-marker-1234567890", "an6-marker-0000000000"), True),
            ("BUILD-INFO 记录了不存在文件", good_info.replace("app-debug.apk` |", "app-ghost.apk` |", 1), True),
        ]
        for label, text, want_fail in cases:
            # 每次用全新目录避免上一次的残留
            case_dir = work / f"bi-{abs(hash(label))}" / DIST_DIR_REL
            case_dir.mkdir(parents=True)
            (case_dir / "app-debug.apk").write_bytes(b"debug-apk")
            (case_dir / "app-release-unsigned.apk").write_bytes(b"release-apk")
            found = run(check_build_info, case_dir, text)
            if bool(found) == want_fail:
                print(f"  PASS  {label} → {'报出' if found else '通过'}")
            else:
                failures.append(f"BUILD-INFO 判定错误：{label}（{found}）")
                print(f"  FAIL  {label} 判定错误（{found}）")

        # 2) 许可解析（合成 registry）
        reg = work / "registry" / "index-fixture"
        (reg / "demo-crate-1.2.3").mkdir(parents=True)
        (reg / "demo-crate-1.2.3" / "Cargo.toml").write_text(
            '[package]\nname = "demo-crate"\nversion = "1.2.3"\nlicense = "MIT OR Apache-2.0"\n',
            encoding="utf-8",
        )
        (reg / "no-license-0.1.0").mkdir(parents=True)
        (reg / "no-license-0.1.0" / "Cargo.toml").write_text(
            '[package]\nname = "no-license"\nversion = "0.1.0"\n', encoding="utf-8"
        )
        if license_from_cargo_toml(reg / "demo-crate-1.2.3" / "Cargo.toml") == "MIT OR Apache-2.0":
            print("  PASS  许可解析：读取 crate Cargo.toml 的 license 字段")
        else:
            failures.append("许可解析未读出 license 字段")
            print("  FAIL  许可解析未读出 license 字段")
        if license_from_cargo_toml(reg / "no-license-0.1.0" / "Cargo.toml") is None:
            print("  PASS  许可解析：无 license 字段返回 None（会进未解析清单）")
        else:
            failures.append("许可解析对无 license 字段的 crate 未返回 None")
            print("  FAIL  无 license 字段未返回 None")
        for lic, want in (
            ("MIT OR Apache-2.0", "许可式（宽松）"),
            ("GPL-3.0-only", "需人工复核"),
            ("Eclipse Public License 1.0", "需人工复核"),
            ("内部 crate（publish = false，未声明 license 字段）", "内部 crate"),
        ):
            got = classify_license(lic)
            if got == want:
                print(f"  PASS  许可分类：{lic} → {got}")
            else:
                failures.append(f"许可分类错误：{lic} → {got}（期望 {want}）")
                print(f"  FAIL  许可分类错误：{lic} → {got}")

        # 3) 发布文档 / 宣称
        docs = work / "docs" / "release"
        docs.mkdir(parents=True)
        body = "\n".join(
            [
                "verify-v1-release.py 与 verify-v1-structure.py 是门禁命令。",
                "真机项保持 待验证；Windows 10/11 与 Wayland 待验证。",
                "未签名 Release APK 属预期，发布前必须使用正式签名。",
                "项目发行许可证延期未定；已知限制章节见下；延期验收项见下。",
                "未验证项清单；回滚与数据备份说明；干净发行版未实测。",
                "文档精度项与 Cargo.lock 发布时哈希登记（O6）。",
                "Android 传递依赖许可缺口已登记；发布物附 SHA-256。",
                "阻塞项登记：AC-A2 边界门禁未关闭，不得计为通过。",
            ]
        )
        for f in RELEASE_DOC_FILES:
            (docs / f).write_text("{{VERSION}} placeholder\n" + body, encoding="utf-8")

        def run_docs(docs_dir: Path) -> list[str]:
            import contextlib
            import io

            rep = Report()
            with contextlib.redirect_stdout(io.StringIO()):
                _evaluate_release_docs(rep, docs_dir)
            return rep.failures

        base_docs_failures = run_docs(docs)
        if base_docs_failures:
            failures.append(f"发布文档完整 fixture 误报：{base_docs_failures}")
            print(f"  FAIL  完整发布文档 fixture 误报：{base_docs_failures}")
        else:
            print("  PASS  发布文档完整 fixture 通过")
        for label, mutate, expect in [
            ("缺少释放门禁命令", lambda t: t.replace("verify-v1-release.py", "门禁"), "门禁命令"),
            ("缺少真机待验证", lambda t: t.replace("真机", "设备"), "Android 真机待验证"),
            ("缺少回滚说明", lambda t: t.replace("回滚", "恢复"), "回滚与数据备份"),
            ("缺少传递依赖口径", lambda t: t.replace("传递依赖", "依赖"), "传递依赖"),
            (
                "缺少阻塞项登记（AC-A2 边界门禁）",
                lambda t: t.replace("AC-A2 边界门禁未关闭", "其它事项已登记"),
                "阻塞项登记",
            ),
            ("写入已真机验证", lambda t: t + "\n已真机验证通过。\n", "未实测宣称"),
            ("写入三平台发布完成", lambda t: t + "\n三平台发布完成。\n", "未实测宣称"),
            ("写入 Android 全面通过", lambda t: t + "\nAndroid 全面通过。\n", "未实测宣称"),
        ]:
            case_dir = work / f"docs-{abs(hash(label))}"
            shutil.rmtree(case_dir, ignore_errors=True)
            shutil.copytree(docs, case_dir)
            for f in RELEASE_DOC_FILES:
                (case_dir / f).write_text(mutate(read_text(case_dir / f)), encoding="utf-8")
            found = run_docs(case_dir)
            if found and any(expect in f for f in found):
                print(f"  PASS  {label} → {next(f for f in found if expect in f)}")
            else:
                failures.append(f"发布文档判定错误：{label}（{found}）")
                print(f"  FAIL  {label} 判定错误（{found}）")
        # 占位符被填成结论 → 模板检查必须报出
        case_dir = work / "docs-filled"
        shutil.rmtree(case_dir, ignore_errors=True)
        shutil.copytree(docs, case_dir)
        (case_dir / "release-notes-template.md").write_text(body, encoding="utf-8")
        found = run_docs(case_dir)
        if any("占位符" in f for f in found):
            print("  PASS  模板被填成结论 → 报出")
        else:
            failures.append("模板无占位符未被报出")
            print("  FAIL  模板无占位符未被报出")

        # 4) ELF 剥离判定（合成 ELF64）
        stripped = parse_elf(build_synthetic_elf(with_symtab=False, build_id=b"\x01\x02\x03\x04"))
        unstripped = parse_elf(build_synthetic_elf(with_symtab=True))
        if not stripped["has_symtab"] and stripped["build_id"] == "01020304":
            print(f"  PASS  合成 ELF（已剥离）判定：symtab={stripped['has_symtab']} build_id={stripped['build_id']}")
        else:
            failures.append(f"合成 ELF 剥离判定错误：{stripped}")
            print(f"  FAIL  合成 ELF（已剥离）判定错误：{stripped}")
        if unstripped["has_symtab"]:
            print("  PASS  合成 ELF（未剥离）被识别出 .symtab（判定有反面能力）")
        else:
            failures.append("合成 ELF（未剥离）未被识别出 .symtab")
            print("  FAIL  合成 ELF（未剥离）未被识别出 .symtab")

        # 4b) dep-info 构建证据 + 未解析许可分流（编译过 → 失败；平台专用 → 未验证项）
        repo = work / "coverage" / "repo"
        deps = repo / "crates/desktop-app/target/release/deps"
        deps.mkdir(parents=True)
        (deps / "demo_crate-1.2.3.d").write_text(
            f"/tmp/out.d: /home/u/.cargo/registry/src/idx/demo_crate-1.2.3/src/lib.rs\n",
            encoding="utf-8",
        )
        (deps / "platform_crate-9.9.9.d").write_text("ignored\n", encoding="utf-8")
        compiled, paths = compiled_crates(repo)
        if compiled == {("demo_crate", "1.2.3")}:
            print(f"  PASS  dep-info 构建证据解析：{sorted(compiled)}")
        else:
            failures.append(f"dep-info 解析错误：{compiled}")
            print(f"  FAIL  dep-info 解析错误：{compiled}")

        rep_cov = Report()
        resolved_fixture = [
            {"name": "demo_crate", "version": "1.2.3", "license": "MIT", "source": "fixture"}
        ]
        unresolved_fixture = [
            {"name": "demo_crate", "version": "1.2.3", "lock": "fixture", "reason": "fixture 未解析"},
            {"name": "windows_only", "version": "0.1.0", "lock": "fixture", "reason": "fixture 未解析"},
        ]
        import contextlib
        import io

        with contextlib.redirect_stdout(io.StringIO()):
            check_license_coverage(rep_cov, repo, resolved_fixture, unresolved_fixture)
        if any("实际编译的依赖存在未解析许可" in f for f in rep_cov.failures):
            print("  PASS  编译过的依赖未解析许可 → 硬失败")
        else:
            failures.append("编译过的依赖未解析许可未被报出")
            print(f"  FAIL  编译过的依赖未解析许可未被报出（{rep_cov.failures}）")
        if any("windows_only-0.1.0" in item for item in rep_cov.unverified):
            print("  PASS  未编译的平台专用依赖 → 未验证项（不计失败）")
        else:
            failures.append("未编译的平台专用依赖未被登记为未验证项")
            print(f"  FAIL  未编译的平台专用依赖未被登记（unverified={rep_cov.unverified}）")

        rep_cov2 = Report()
        with contextlib.redirect_stdout(io.StringIO()):
            check_license_coverage(rep_cov2, repo, resolved_fixture, [])
        if not rep_cov2.failures:
            print("  PASS  编译依赖全部有许可条目 → 覆盖核对通过")
        else:
            failures.append(f"许可覆盖核对误报：{rep_cov2.failures}")
            print(f"  FAIL  许可覆盖核对误报：{rep_cov2.failures}")

        # 4c) 版本 / 标记解析
        if parse_package_version('[package]\nname="x"\nversion = "0.1.0"\n') == "0.1.0":
            print("  PASS  Cargo 版本解析")
        else:
            failures.append("Cargo 版本解析失败")
            print("  FAIL  Cargo 版本解析失败")
        marker = parse_marker_from_gradle_text(
            'buildConfigField("String", "BUILD_MARKER", "\\"an6-x-1234567890\\"")'
        )
        if marker == "an6-x-1234567890":
            print("  PASS  BUILD_MARKER 解析（gradle）")
        else:
            failures.append(f"BUILD_MARKER 解析失败：{marker}")
            print(f"  FAIL  BUILD_MARKER 解析失败：{marker}")
        if parse_gradle_wrapper_version("distributionUrl=...gradle-8.11.1-bin.zip") == "8.11.1":
            print("  PASS  Gradle Wrapper 版本解析")
        else:
            failures.append("Gradle Wrapper 版本解析失败")
            print("  FAIL  Gradle Wrapper 版本解析失败")
        av = parse_android_values(
            'versionCode = 7\nversionName = "1.2.3"\nminSdk = 31\ntargetSdk = 36\n'
            'buildConfigField("String", "BUILD_MARKER", "\\"m-1234567890\\"")\n'
        )
        if av.get("version_name") == "1.2.3" and av.get("version_code") == "7" and av.get("build_marker") == "m-1234567890":
            print("  PASS  Android 版本/标记解析")
        else:
            failures.append(f"Android 版本/标记解析失败：{av}")
            print(f"  FAIL  Android 版本/标记解析失败：{av}")

        # 5) AC-A2 边界门禁：exit != 0 必须登记为**阻塞项**（不得计为通过）；exit 0 才允许 ok
        import contextlib
        import io

        for code, expect_blocker in ((1, True), (2, True), (0, False)):
            gate_root = work / f"boundary-exit-{code}"
            (gate_root / "scripts").mkdir(parents=True)
            (gate_root / "scripts" / "verify-android-boundary.py").write_text(
                "import sys\n"
                f"print('stub boundary gate exit={code}')\n"
                f"print('RESULT: {'PASS' if code == 0 else 'FAIL'}')\n"
                f"sys.exit({code})\n",
                encoding="utf-8",
            )
            rep_b = Report()
            with contextlib.redirect_stdout(io.StringIO()):
                check_android_boundary_gate(gate_root, rep_b)
            if bool(rep_b.blockers) == expect_blocker and not rep_b.failures:
                print(f"  PASS  边界门禁 exit={code} → 阻塞项={expect_blocker}")
            else:
                failures.append(
                    f"边界门禁 exit={code} 判定错误：blockers={rep_b.blockers} failures={rep_b.failures}"
                )
                print(
                    f"  FAIL  边界门禁 exit={code} 判定错误："
                    f"blockers={rep_b.blockers} failures={rep_b.failures}"
                )

        # 5b) 门禁脚本缺失 → 结构性硬失败（不得静默跳过）
        rep_c = Report()
        with contextlib.redirect_stdout(io.StringIO()):
            check_android_boundary_gate(work / "boundary-missing", rep_c)
        if rep_c.failures and not rep_c.blockers:
            print("  PASS  边界门禁脚本缺失 → 硬失败")
        else:
            failures.append(f"边界门禁脚本缺失未被报为硬失败：{rep_c.failures} / {rep_c.blockers}")
            print(f"  FAIL  边界门禁脚本缺失未被报为硬失败：{rep_c.failures} / {rep_c.blockers}")

        # 6) F-REL1b-2 反例：被调用 APK 校验器报告的 skips 必须在本门禁输出中**可见**
        #    （此前门禁只读 result/failures/facts，校验器跳过 2 项时门禁仍自报「跳过 0 项」）
        import contextlib as _contextlib
        import io as _io

        def apk_verifier_case(case_name: str, verifier_skips: list[str]) -> tuple[Report, str]:
            case_root = work / case_name
            (case_root / DIST_DIR_REL).mkdir(parents=True)
            (case_root / DIST_DIR_REL / "app-debug.apk").write_bytes(b"debug-apk")
            verifier = case_root / APK_VERIFIER
            verifier.parent.mkdir(parents=True, exist_ok=True)
            payload = {
                "apk": "dist/app-debug.apk",
                "result": "PASS",
                "ok_count": 1,
                "failures": [],
                "skips": verifier_skips,
                "facts": {
                    "marker": "an6-marker-1234567890",
                    "manifest": {"version_name": "1.0.0", "version_code": 1},
                    "signature": {"signed": True},
                    "dex_count": 6,
                    "sdk_probe": {"tried": ["PATH"], "found": {"aapt2": None, "apksigner": None}},
                },
            }
            verifier.write_text(
                "import json, sys\n"
                f"payload = {payload!r}\n"
                "payload['apk'] = sys.argv[1]\n"
                "out = sys.argv[sys.argv.index('--json') + 1]\n"
                "open(out, 'w', encoding='utf-8').write(json.dumps(payload, ensure_ascii=False))\n"
                "sys.exit(0)\n",
                encoding="utf-8",
            )
            ctx: dict[str, object] = {
                "gradle_build_marker": "an6-marker-1234567890",
                "android": {"version_name": "1.0.0", "version_code": "1", "abi": "arm64-v8a"},
            }
            rep_v = Report()
            buf = _io.StringIO()
            with _contextlib.redirect_stdout(buf):
                section_android_artifacts(case_root, rep_v, ctx)
            return rep_v, buf.getvalue()

        degraded_skip = "未找到 aapt2（已试 PATH）；包名/SDK 外部交叉校验跳过"
        rep_d, out_d = apk_verifier_case("verifier-skip-visible", [degraded_skip])
        if (
            len(rep_d.verifier_skips) == 1
            and degraded_skip in rep_d.verifier_skips[0]
            and all(item in rep_d.skips for item in rep_d.verifier_skips)
            and "[降级]" in out_d
            and degraded_skip in out_d
        ):
            print("  PASS  校验器跳过项计入本门禁并单列 [降级]（降级不再不可见）")
        else:
            failures.append(
                f"校验器跳过项未被门禁消费：verifier_skips={rep_d.verifier_skips} skips={rep_d.skips}"
            )
            print(
                f"  FAIL  校验器跳过项未被门禁消费：verifier_skips={rep_d.verifier_skips} "
                f"skips={rep_d.skips}"
            )
        rep_ok, out_ok = apk_verifier_case("verifier-skip-none", [])
        if not rep_ok.verifier_skips and "[降级]" not in out_ok:
            print("  PASS  校验器无跳过时不产生假「降级」提示（非空转判定有反面）")
        else:
            failures.append(f"校验器无跳过却报了降级：{rep_ok.verifier_skips}")
            print(f"  FAIL  校验器无跳过却报了降级：{rep_ok.verifier_skips}")

    return failures


def _verify_build_info(rep: Report, root: Path, ctx: dict[str, object]) -> None:
    """BUILD-INFO ↔ 实际分发包 的跨文件核对（抽成独立函数以便自测直接调用）。"""
    dist = root / DIST_DIR_REL
    build_info = root / BUILD_INFO_REL
    if not build_info.is_file():
        rep.fail(f"缺少 {BUILD_INFO_REL}（分发包元信息未生成，无法核对构建标记与 SHA-256）")
        return
    text = read_text(build_info)
    marker_m = BUILD_INFO_MARKER_RE.search(text)
    marker = marker_m.group(1) if marker_m else None
    ctx["build_info_marker"] = marker
    if marker == ctx.get("gradle_build_marker"):
        rep.ok(f"{BUILD_INFO_REL} 记录的 BUILD_MARKER 与 gradle 一致：{marker}")
    else:
        rep.fail(f"{BUILD_INFO_REL} 标记 {marker} ≠ gradle {ctx.get('gradle_build_marker')}")
    rows = BUILD_INFO_ROW_RE.findall(text)
    ctx["build_info_rows"] = [r[0] for r in rows]
    if not rows:
        rep.fail(f"{BUILD_INFO_REL} 未记录任何 APK 的大小/SHA-256（发布物清单不完整）")
        return
    for name, size, digest, signature_note in rows:
        target = dist / name
        if not target.is_file():
            rep.fail(f"{BUILD_INFO_REL} 记录了不存在的分发包：{name}")
            continue
        real_size = target.stat().st_size
        real_digest = sha256_file(target)
        if int(size) != real_size:
            rep.fail(f"{name}：BUILD-INFO 记录大小 {size} B ≠ 实际 {real_size} B")
        elif real_digest != digest.lower():
            rep.fail(f"{name}：BUILD-INFO 记录 SHA-256 与实际不符（分发包被替换或记录过期）")
        else:
            rep.ok(f"{name}：BUILD-INFO 大小与 SHA-256 逐项一致（签名状态：{signature_note.strip() or '未记录'}）")
    names = [r[0] for r in rows]
    if not any("debug" in n for n in names):
        rep.fail(f"{BUILD_INFO_REL} 未记录 Debug APK（发布物清单缺 Debug 分发包）")
    if not any("release" in n for n in names):
        rep.fail(f"{BUILD_INFO_REL} 未记录 Release APK（发布物清单缺 Release 分发包）")


def _evaluate_release_docs(rep: Report, docs_dir: Path) -> None:
    """docs/release 判定（抽成独立函数以便自测直接调用）。"""
    missing = [f for f in RELEASE_DOC_FILES if not (docs_dir / f).is_file()]
    if missing:
        rep.fail(f"{docs_dir} 缺少文件：{missing}")
    else:
        rep.ok(f"{docs_dir} 文件齐全：{list(RELEASE_DOC_FILES)}")
    files = sorted(docs_dir.rglob("*.md"))
    corpus = "\n".join(read_text(p) for p in files)
    for label, pattern in RELEASE_DOC_PATTERNS.items():
        if re.search(pattern, corpus, re.I):
            rep.ok(f"发布文档含「{label}」")
        else:
            rep.fail(f"发布文档缺少「{label}」（正则 /{pattern}/）")
    claims = [
        f"/{reason}/ 命中 {m.group(0)!r}"
        for reason, rx in FORBIDDEN_CLAIM_PATTERNS
        for m in [re.search(rx, corpus)]
        if m
    ]
    if claims:
        rep.fail("发布文档出现未实测宣称：" + "；".join(claims))
    else:
        rep.ok("发布文档未出现未实测宣称")
    template = docs_dir / "release-notes-template.md"
    if template.is_file():
        if re.search(r"\{\{[^}]+\}\}|<[^>]{2,}>", read_text(template)):
            rep.ok("release-notes-template.md 含占位符")
        else:
            rep.fail("release-notes-template.md 未见占位符（模板被当成结论填写）")
    pending = len(re.findall(r"待验证", corpus))
    if pending >= 6:
        rep.ok(f"发布文档「待验证」出现 {pending} 次")
    else:
        rep.fail(f"发布文档「待验证」仅 {pending} 次（< 6）")


# --------------------------------------------------------------------------- #
# CLI
# --------------------------------------------------------------------------- #


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="V1 发布物/版本/许可/未验证项门禁（父级 checklist C11）")
    p.add_argument("--desktop", action="store_true", help="校验桌面发布物")
    p.add_argument("--android", action="store_true", help="校验 Android 发布物")
    p.add_argument("--root", default=str(REPO_ROOT), help="仓库根（默认本脚本上级目录）")
    p.add_argument("--json", dest="json_out", default=None, help="把发布清单结论写入 JSON 文件")
    p.add_argument("--self-test", action="store_true", help="运行合成 fixture 反向用例")
    args = p.parse_args(argv)

    root = Path(args.root).resolve()
    do_desktop = args.desktop or not (args.desktop or args.android)
    do_android = args.android or not (args.desktop or args.android)
    print(f"verify-v1-release: repo={root} desktop={do_desktop} android={do_android}")
    if not root.is_dir():
        print(f"RESULT: FAIL — --root 不存在: {root}")
        return 2

    exit_code = 0
    if args.self_test:
        failures = self_test()
        print("-" * 68)
        if failures:
            print(f"SELF-TEST RESULT: FAIL ({len(failures)} 项)")
            for f in failures:
                print(f"  - {f}")
            exit_code = 1
        else:
            print("SELF-TEST RESULT: PASS（每条判定均有反向用例）")

    rep = Report()
    ctx = section_versions(root, rep)
    if do_desktop:
        section_desktop_artifacts(root, rep, ctx)
    if do_android:
        section_android_artifacts(root, rep, ctx)
    section_licenses(root, rep, ctx)
    section_release_docs(root, rep, ctx)
    section_unverified(root, rep, ctx)

    print("-" * 68)
    print("发布清单摘要：")
    print(f"  版本（Rust workspace / 桌面）: {ctx.get('workspace_version')} / {ctx.get('desktop_version')}")
    android = ctx.get("android") or {}
    print(f"  版本（Android）              : {android.get('version_name')} (code {android.get('version_code')})")
    print(f"  构建标记（gradle/BUILD-INFO) : {ctx.get('gradle_build_marker')} / {ctx.get('build_info_marker')}")
    print(f"  Cargo.lock 发布时哈希        : {ctx.get('cargo_lock_sha256')}")
    for r in ctx.get("apks", []) or []:
        print(
            f"  发布物: {r.get('path')} | {r.get('size')} B | sha256={str(r.get('sha256'))[:16]}… | "
            f"签名={'是' if r.get('signed') else '否'} | marker={r.get('marker')}"
        )
    print(f"  桌面发布目录                 : {ctx.get('desktop_release_dirs') or '未产出（见未验证项）'}")
    print(f"  Rust 许可条目                : {len(ctx.get('rust_licenses') or [])}")
    print(f"  Android 许可条目             : {len(ctx.get('android_licenses') or [])}")
    print(f"  未验证项                     : {len(rep.unverified)}")
    for item in rep.unverified:
        print(f"    - {item}")
    print(f"  阻塞项（未关闭）             : {len(rep.blockers)}")
    for item in rep.blockers:
        print(f"    ! {item}")
    print(
        "  V1 结论口径                  : TXT 桌面可用 + Android 可构建；"
        + ("存在未关闭阻塞项 → 不得宣称 Android 通过 / 全平台发布" if rep.blockers else "无未关闭阻塞项")
    )
    print("")

    if args.json_out:
        payload = {
            "result": "PASS" if (not rep.failures and exit_code == 0) else "FAIL",
            "root": str(root),
            "ok_count": rep.ok_count,
            "version": {
                "rust_workspace": ctx.get("workspace_version"),
                "desktop_crate": ctx.get("desktop_version"),
                "android_version_name": android.get("version_name"),
                "android_version_code": android.get("version_code"),
            },
            "build_marker": {
                "gradle": ctx.get("gradle_build_marker"),
                "build_info": ctx.get("build_info_marker"),
            },
            "toolchain": ctx.get("toolchain"),
            "cargo_lock_sha256": ctx.get("cargo_lock_sha256"),
            "artifacts": ctx.get("apks", []),
            "desktop_release_dirs": ctx.get("desktop_release_dirs", []),
            "licenses": {
                "rust": ctx.get("rust_licenses", []),
                "android": ctx.get("android_licenses", []),
            },
            "release_docs": ctx.get("release_docs", []),
            "unverified_items": rep.unverified,
            "blockers": rep.blockers,
            "v1_verdict_scope": (
                "TXT 桌面可用 + Android 可构建；存在未关闭阻塞项 → 不得宣称 Android 通过 / 全平台发布"
                if rep.blockers
                else "TXT 桌面可用 + Android 可构建；无未关闭阻塞项"
            ),
            "failures": rep.failures,
            "skips": rep.skips,
            "verifier_skips": rep.verifier_skips,
            "facts": rep.facts,
        }
        Path(args.json_out).write_text(
            json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8"
        )
        print(f"发布清单 JSON 已写入 {args.json_out}")

    if rep.failures:
        print(f"RESULT: FAIL ({len(rep.failures)} 项)")
        for item in rep.failures:
            print(f"  - {item}")
        return 1
    if exit_code != 0:
        print("RESULT: FAIL — --self-test 未通过")
        return 1
    print(
        f"RESULT: PASS — 发布清单完整（断言 {rep.ok_count} 项，跳过 {len(rep.skips)} 项"
        + (f"，其中被调用校验器报告的跳过 {len(rep.verifier_skips)} 项" if rep.verifier_skips else "")
        + f"，未验证项 {len(rep.unverified)} 项，阻塞项 {len(rep.blockers)} 项）；"
        f"本门禁不构成发布结论，未实测项一律为待验证；"
        f"V1 口径：TXT 桌面可用 + Android 可构建"
        + ("（存在未关闭阻塞项）" if rep.blockers else "")
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
