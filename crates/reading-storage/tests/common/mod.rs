//! RC6 测试公共夹具：临时目录、跨语言可校验的 SHA-256、快照比对。
//!
//! 本文件被多个集成测试二进制包含，未在所有二进制中使用的辅助函数
//! 由文件级 `allow(dead_code)` 放行。
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use reading_storage::bookmarks::BookmarkRecord;
use reading_storage::books::{BookImport, BookRecord};
use reading_storage::index::{ChapterRecord, LayoutRecord};
use reading_storage::progress::ProgressRecord;
use reading_storage::settings::ReaderSettings;
use reading_storage::Storage;

// ---------------------------------------------------------------------------
// 临时目录
// ---------------------------------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 自管理临时目录（Drop 时递归删除；不依赖外部 crate）。
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// 创建唯一临时目录（进程号 + 原子计数 + 纳秒，避免并行测试互相覆盖）。
    pub fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("nr-rc6-{tag}-{}-{nanos}-{seq}", std::process::id()));
        fs::create_dir_all(&path).expect("创建临时目录");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// ---------------------------------------------------------------------------
// SHA-256（零依赖自实现，用于「用户原始 TXT 未被修改」的硬断言）
// ---------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

fn sha256_words(data: &[u8]) -> [u32; 8] {
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut message = data.to_vec();
    message.push(0x80);
    let pad = (56 + 64 - (message.len() % 64)) % 64;
    message.extend(std::iter::repeat_n(0_u8, pad));
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut start = 0;
    while start + 64 <= message.len() {
        let chunk = &message[start..start + 64];
        let mut w = [0_u32; 64];
        for (index, word) in w.iter_mut().take(16).enumerate() {
            let base = index * 4;
            *word = u32::from_be_bytes([
                chunk[base],
                chunk[base + 1],
                chunk[base + 2],
                chunk[base + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];
        for index in 0..64 {
            let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(big_s1)
                .wrapping_add(choice)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(w[index]);
            let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = big_s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        let round = [a, b, c, d, e, f, g, h];
        for (slot, added) in state.iter_mut().zip(round) {
            *slot = slot.wrapping_add(added);
        }
        start += 64;
    }
    state
}

/// SHA-256 十六进制小写摘要（与 `sha256sum` / Python `hashlib` 口径一致）。
pub fn sha256_hex(data: &[u8]) -> String {
    let words = sha256_words(data);
    let mut out = String::with_capacity(64);
    for word in words {
        out.push_str(&format!("{word:08x}"));
    }
    out
}

/// FNV-1a 64 位指纹（16 位十六进制）——与 reading-source 文件指纹同算法家族。
pub fn fnv1a64_hex(data: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// 读取仓库内夹具（不存在时返回 `None`）。
pub fn read_fixture(name: &str) -> Option<Vec<u8>> {
    fs::read(fixture_path(name)).ok()
}

// ---------------------------------------------------------------------------
// 夹具文件
// ---------------------------------------------------------------------------

/// 生成一个真实 TXT 文件并返回其字节（含中文、换行与 4 字节字符）。
pub fn write_sample_txt(dir: &TempDir, name: &str) -> (PathBuf, Vec<u8>) {
    let mut body = String::new();
    body.push_str("第一章 起始\n");
    body.push_str("这是用于 RC6 存储安全测试的正文样章，包含中文、标点与 emoji 🙂。\n");
    for index in 0..64 {
        body.push_str(&format!(
            "第 {index} 行：正文不应写入数据库，只允许元数据与短摘要。\n"
        ));
    }
    body.push_str("第二章 结束\n");
    let bytes = body.into_bytes();
    let path = dir.join(name);
    fs::write(&path, &bytes).expect("写入夹具 TXT");
    (path, bytes)
}

/// 构造导入记录（指纹用内容 FNV-1a，避免依赖外部哈希）。
pub fn sample_import(locator: &str, display_name: &str, bytes: &[u8]) -> BookImport {
    BookImport {
        display_name: display_name.to_string(),
        original_name: format!("{display_name}.txt"),
        source_kind: reading_storage::books::KIND_DESKTOP_PATH.to_string(),
        source_locator: locator.to_string(),
        source_size: bytes.len() as i64,
        source_modified_hint: Some(1_700_000_000),
        source_fingerprint: Some(fnv1a64_hex(bytes)),
        encoding: "utf-8".to_string(),
        encoding_confidence: 1000,
        encoding_user_override: false,
    }
}

/// 生成 TXT 并导入，返回 `(book_id, 文件路径, 文件字节)`。
pub fn seed_book(
    storage: &Storage,
    dir: &TempDir,
    file_name: &str,
    display_name: &str,
) -> (i64, PathBuf, Vec<u8>) {
    let (path, bytes) = write_sample_txt(dir, file_name);
    let import = sample_import(&path.to_string_lossy(), display_name, &bytes);
    let outcome =
        reading_storage::books::import_book(storage.conn(), &import, 1_000).expect("导入书籍夹具");
    (outcome.book_id, path, bytes)
}

// ---------------------------------------------------------------------------
// 数据快照（证明「清理索引不丢用户元数据」）
// ---------------------------------------------------------------------------

/// 用户可见数据的完整快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub tables: Vec<(String, i64)>,
    pub books: Vec<BookRecord>,
    pub progress: Vec<ProgressRecord>,
    pub bookmarks: Vec<BookmarkRecord>,
    pub chapters: Vec<ChapterRecord>,
    pub layouts: Vec<LayoutRecord>,
    pub settings: ReaderSettings,
}

/// 采集快照（含逐表行数，便于定位差异）。
pub fn snapshot(storage: &Storage) -> Snapshot {
    let conn = storage.conn();
    let books = reading_storage::books::list_all_books(conn).expect("读取书籍");
    let mut progress = Vec::new();
    let mut bookmarks = Vec::new();
    let mut chapters = Vec::new();
    let mut layouts = Vec::new();
    for book in &books {
        if let Some(record) =
            reading_storage::progress::load_progress(conn, book.id).expect("读取进度")
        {
            progress.push(record);
        }
        bookmarks
            .extend(reading_storage::bookmarks::list_bookmarks(conn, book.id).expect("读取书签"));
        chapters
            .extend(reading_storage::index::list_chapters(conn, book.id).expect("读取章节索引"));
        layouts.extend(
            reading_storage::index::list_layout_records(conn, book.id).expect("读取分页缓存"),
        );
    }
    Snapshot {
        tables: storage.table_row_counts().expect("逐表行数"),
        books,
        progress,
        bookmarks,
        chapters,
        layouts,
        settings: reading_storage::settings::load_settings(conn).expect("读取设置"),
    }
}

impl Snapshot {
    /// 只比对「用户元数据」部分（书籍、进度、书签、设置），忽略可重建索引与行数。
    pub fn assert_user_metadata_eq(&self, other: &Snapshot, label: &str) {
        assert_eq!(self.books, other.books, "{label}: 书籍记录发生变化");
        assert_eq!(self.progress, other.progress, "{label}: 阅读进度发生变化");
        assert_eq!(self.bookmarks, other.bookmarks, "{label}: 书签发生变化");
        assert_eq!(self.settings, other.settings, "{label}: 排版设置发生变化");
    }

    /// 全量比对（含可重建索引）。
    pub fn assert_all_eq(&self, other: &Snapshot, label: &str) {
        self.assert_user_metadata_eq(other, label);
        assert_eq!(self.chapters, other.chapters, "{label}: 章节索引发生变化");
        assert_eq!(self.layouts, other.layouts, "{label}: 分页缓存记录发生变化");
        assert_eq!(self.tables, other.tables, "{label}: 逐表行数发生变化");
    }
}

// ---------------------------------------------------------------------------
// 自检：SHA-256 实现必须与 FIPS 向量 / Python hashlib 一致
// ---------------------------------------------------------------------------

#[test]
fn sha256_matches_fips_reference_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

/// 跨语言对照：与 `sha256sum tests/fixtures/ascii-short.txt` 的输出逐字节一致。
#[test]
fn sha256_matches_python_hashlib_reference_for_repo_fixture() {
    let bytes = read_fixture("ascii-short.txt").expect("仓库夹具 tests/fixtures/ascii-short.txt");
    assert_eq!(
        sha256_hex(&bytes),
        "70ea2f9a493bfafb858a387e43151b81b037e31f41f9d035d91cf8b7bb6ef0b9"
    );
    let bom = read_fixture("utf8-bom.txt").expect("仓库夹具 tests/fixtures/utf8-bom.txt");
    assert_eq!(
        sha256_hex(&bom),
        "faae2f1a4d5edfc833704588e5b1c47c65585776a20d852e251a9b9a5005e5ce"
    );
}
