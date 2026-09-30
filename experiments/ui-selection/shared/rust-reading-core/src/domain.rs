//! 模拟领域数据：6 层分类 / 1000 类 / 10000 书 / 2000 历史。
//!
//! 全部由固定种子确定性生成（S-R4），不落库、不读网络。

use crate::json;
use crate::rng::Rng;

/// 分类（书架的层级目录）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: u64,
    pub name: String,
    /// 层级，0 为顶层。
    pub level: u8,
    /// 父分类 id，顶层为 0。
    pub parent_id: u64,
}

/// 书籍。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub id: u64,
    pub title: String,
    pub author: String,
    pub category_id: u64,
    pub chapter_count: u32,
    pub total_bytes: u64,
}

/// 章节元数据（按需生成，不常驻）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub book_id: u64,
    pub index: u32,
    pub title: String,
    pub byte_offset: u64,
    pub byte_len: u64,
}

/// 阅读历史。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub book_id: u64,
    pub chapter_index: u32,
    pub byte_offset: u64,
    pub updated_at_ms: u64,
}

const NAME_POOL: [&str; 32] = [
    "星", "夜", "风", "云", "剑", "书", "海", "山", "霜", "月", "花", "雪", "江", "河", "木", "林",
    "光", "影", "雨", "雾", "春", "秋", "长", "短", "青", "玄", "白", "金", "远", "近", "行", "止",
];
const AUTHOR_TAIL: [&str; 8] = ["", "先生", "女士", "居士", "子", "翁", "客", "生"];
const CHAPTER_TITLE: [&str; 12] = [
    "序章", "归途", "夜行", "问路", "旧约", "断章", "远山", "近水", "重逢", "离别", "终章", "残页",
];
/// 分类层级规模（合计 1000）。
const LEVEL_SIZES: [usize; 6] = [6, 24, 120, 300, 400, 150];

/// 模拟书库。
#[derive(Debug, Clone)]
pub struct Library {
    pub seed: u64,
    pub categories: Vec<Category>,
    pub books: Vec<Book>,
    pub history: Vec<HistoryEntry>,
}

impl Library {
    /// 分类总数。
    pub const CATEGORY_COUNT: usize = 1000;
    /// 书籍总数。
    pub const BOOK_COUNT: usize = 10000;
    /// 历史条数。
    pub const HISTORY_COUNT: usize = 2000;

    /// 按固定种子生成完整模拟书库。
    pub fn generate(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let categories = Self::generate_categories(&mut rng);
        let books = Self::generate_books(&mut rng, &categories);
        let history = Self::generate_history(&mut rng, &books);
        Self {
            seed,
            categories,
            books,
            history,
        }
    }

    fn name_from(rng: &mut Rng, min: usize, max: usize) -> String {
        let n = rng.range_inclusive(min as u64, max as u64) as usize;
        let mut s = String::with_capacity(n * 3);
        for _ in 0..n {
            s.push_str(rng.pick(&NAME_POOL));
        }
        s
    }

    fn generate_categories(rng: &mut Rng) -> Vec<Category> {
        let mut out = Vec::with_capacity(Self::CATEGORY_COUNT);
        let mut next_id: u64 = 1;
        let mut parents: Vec<u64> = Vec::new();
        for (level, size) in LEVEL_SIZES.iter().enumerate() {
            let mut level_ids = Vec::with_capacity(*size);
            for i in 0..*size {
                let id = next_id;
                next_id += 1;
                let parent_id = if level == 0 || parents.is_empty() {
                    0
                } else {
                    parents[(i * parents.len()) / (*size).max(1)]
                };
                out.push(Category {
                    id,
                    name: Self::name_from(rng, 2, 4),
                    level: level as u8,
                    parent_id,
                });
                level_ids.push(id);
            }
            parents = level_ids;
        }
        out
    }

    fn generate_books(rng: &mut Rng, categories: &[Category]) -> Vec<Book> {
        // 只把书挂到最内层分类，形成可虚拟化的深目录。
        let leaves: Vec<u64> = categories
            .iter()
            .filter(|c| c.level as usize == LEVEL_SIZES.len() - 1)
            .map(|c| c.id)
            .collect();
        let mut out = Vec::with_capacity(Self::BOOK_COUNT);
        for i in 0..Self::BOOK_COUNT {
            let id = (i as u64) + 1;
            let chapter_count = rng.range_inclusive(8, 48) as u32;
            let chapter_bytes = rng.range_inclusive(6 * 1024, 24 * 1024);
            let category_id = if leaves.is_empty() {
                1
            } else {
                leaves[i % leaves.len()]
            };
            let mut title = Self::name_from(rng, 2, 5);
            if rng.below(3) == 0 {
                title.push('·');
                title.push_str(rng.pick(&CHAPTER_TITLE));
            }
            let mut author = Self::name_from(rng, 2, 3);
            author.push_str(rng.pick(&AUTHOR_TAIL));
            out.push(Book {
                id,
                title,
                author,
                category_id,
                chapter_count,
                total_bytes: chapter_bytes * chapter_count as u64,
            });
        }
        out
    }

    fn generate_history(rng: &mut Rng, books: &[Book]) -> Vec<HistoryEntry> {
        let mut out = Vec::with_capacity(Self::HISTORY_COUNT);
        for i in 0..Self::HISTORY_COUNT {
            let book = &books[rng.below(books.len() as u64) as usize];
            let chapter_index = rng.below(book.chapter_count as u64) as u32;
            let offset = Self::chapter_byte_offset(book, chapter_index);
            out.push(HistoryEntry {
                book_id: book.id,
                chapter_index,
                byte_offset: offset,
                updated_at_ms: 1_760_000_000_000 + (i as u64) * 37_000,
            });
        }
        out
    }

    /// 章节在书籍正文中的起始字节偏移（均分）。
    pub fn chapter_byte_offset(book: &Book, chapter_index: u32) -> u64 {
        if book.chapter_count == 0 {
            return 0;
        }
        let idx = chapter_index.min(book.chapter_count - 1) as u64;
        (book.total_bytes * idx) / book.chapter_count as u64
    }

    /// 章节长度（字节）。
    pub fn chapter_byte_len(book: &Book, chapter_index: u32) -> u64 {
        if book.chapter_count == 0 {
            return 0;
        }
        let idx = chapter_index.min(book.chapter_count - 1) as u64;
        let end = (book.total_bytes * (idx + 1)) / book.chapter_count as u64;
        end.saturating_sub(Self::chapter_byte_offset(book, chapter_index))
    }

    /// 查询分类。
    pub fn category(&self, id: u64) -> Option<&Category> {
        self.categories.iter().find(|c| c.id == id)
    }

    /// 查询书籍。
    pub fn book(&self, id: u64) -> Option<&Book> {
        self.books.iter().find(|b| b.id == id)
    }

    /// 生成章节元数据。
    pub fn chapter(&self, book_id: u64, index: u32) -> Option<Chapter> {
        let book = self.book(book_id)?;
        if index >= book.chapter_count {
            return None;
        }
        let title = format!(
            "{} {}",
            CHAPTER_TITLE[(index as usize) % CHAPTER_TITLE.len()],
            index + 1
        );
        Some(Chapter {
            book_id,
            index,
            title,
            byte_offset: Self::chapter_byte_offset(book, index),
            byte_len: Self::chapter_byte_len(book, index),
        })
    }

    /// 分类分页批次。
    pub fn categories_batch(&self, start: usize, count: usize) -> &[Category] {
        slice_batch(&self.categories, start, count)
    }

    /// 书籍分页批次。
    pub fn books_batch(&self, start: usize, count: usize) -> &[Book] {
        slice_batch(&self.books, start, count)
    }

    /// 历史分页批次。
    pub fn history_batch(&self, start: usize, count: usize) -> &[HistoryEntry] {
        slice_batch(&self.history, start, count)
    }

    /// 子分类（供书架的虚拟化目录树使用）。
    pub fn children_of(&self, category_id: u64) -> Vec<&Category> {
        self.categories
            .iter()
            .filter(|c| c.parent_id == category_id)
            .collect()
    }

    /// 分类下的书籍（直达 + 子分类直挂）。
    pub fn books_of_category(&self, category_id: u64) -> Vec<&Book> {
        self.books
            .iter()
            .filter(|b| b.category_id == category_id)
            .collect()
    }

    /// 标题/作者模糊搜索。
    pub fn search_books(&self, query: &str, limit: usize) -> Vec<&Book> {
        let q = query.trim();
        if q.is_empty() {
            return self.books.iter().take(limit).collect();
        }
        self.books
            .iter()
            .filter(|b| b.title.contains(q) || b.author.contains(q))
            .take(limit)
            .collect()
    }

    /// 书籍批次 JSON（跨语言边界的批量传输形态）。
    pub fn books_batch_json(&self, start: usize, count: usize) -> String {
        let items = self.books_batch(start, count);
        let mut out = String::from("{\"start\":");
        out.push_str(&start.to_string());
        out.push_str(",\"count\":");
        out.push_str(&items.len().to_string());
        out.push_str(",\"total\":");
        out.push_str(&self.books.len().to_string());
        out.push_str(",\"items\":[");
        for (i, b) in items.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"id\":");
            out.push_str(&b.id.to_string());
            out.push_str(",\"title\":");
            out.push_str(&json::quoted(&b.title));
            out.push_str(",\"author\":");
            out.push_str(&json::quoted(&b.author));
            out.push_str(",\"categoryId\":");
            out.push_str(&b.category_id.to_string());
            out.push_str(",\"chapterCount\":");
            out.push_str(&b.chapter_count.to_string());
            out.push_str(",\"totalBytes\":");
            out.push_str(&b.total_bytes.to_string());
            out.push('}');
        }
        out.push_str("]}");
        out
    }

    /// 章节批次 JSON。
    pub fn chapters_batch_json(&self, book_id: u64, start: u32, count: u32) -> Option<String> {
        let book = self.book(book_id)?;
        let start = start.min(book.chapter_count);
        let end = (start + count).min(book.chapter_count);
        let mut out = String::from("{\"bookId\":");
        out.push_str(&book_id.to_string());
        out.push_str(",\"start\":");
        out.push_str(&start.to_string());
        out.push_str(",\"items\":[");
        for (i, idx) in (start..end).enumerate() {
            let ch = self.chapter(book_id, idx)?;
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"index\":");
            out.push_str(&ch.index.to_string());
            out.push_str(",\"title\":");
            out.push_str(&json::quoted(&ch.title));
            out.push_str(",\"byteOffset\":");
            out.push_str(&ch.byte_offset.to_string());
            out.push_str(",\"byteLen\":");
            out.push_str(&ch.byte_len.to_string());
            out.push('}');
        }
        out.push_str("]}");
        Some(out)
    }
}

fn slice_batch<T>(all: &[T], start: usize, count: usize) -> &[T] {
    if start >= all.len() {
        return &[];
    }
    let end = start.saturating_add(count).min(all.len());
    &all[start..end]
}
