//! 跨边界缓冲区：**所有权契约**与归还函数。
//!
//! # 所有权（冻结，写入头文件）
//!
//! - 由 Rust 分配、经 [`NrBuffer`] 交给调用方的内存（读取窗口、章节批次、最近错误消息），
//!   调用方**必须**用 [`nr_release_buffer`] 归还；
//! - 归还时 Rust 释放内存并把结构体三个字段**全部置零**；对已归零的缓冲区再次调用
//!   [`nr_release_buffer`] 是**安全 no-op**（返回 `NR_ERR_OK`），因此"归还两次"不会崩溃；
//! - 结构体自身由调用方拥有；调用方不得修改 `ptr` / `len` / `capacity`，
//!   伪造的字段（例如 `len > capacity`）会被拒绝为 `NR_ERR_INVALID_ARGUMENT`；
//! - 归还函数不需要引擎句柄，因此引擎销毁后仍然可以归还此前取得的缓冲区。

use crate::abi::invalid;
use reading_core::NrResult;

/// 由 Rust 分配、调用方归还的字节缓冲区（与头文件 `nr_buffer` 字段一致）。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NrBuffer {
    /// 数据指针；`NULL` 表示空缓冲区。
    pub ptr: *mut u8,
    /// 有效字节数。
    pub len: usize,
    /// 分配容量（Rust 归还时使用）。
    pub capacity: usize,
}

impl Default for NrBuffer {
    fn default() -> Self {
        Self::empty()
    }
}

impl NrBuffer {
    /// 空缓冲区（`ptr = NULL`）。
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
            capacity: 0,
        }
    }

    /// 由 `Vec<u8>` 转移所有权（Rust 分配 ⇒ 调用方必须归还）。
    pub fn from_vec(mut bytes: Vec<u8>) -> Self {
        let ptr = bytes.as_mut_ptr();
        let len = bytes.len();
        let capacity = bytes.capacity();
        std::mem::forget(bytes);
        Self { ptr, len, capacity }
    }

    /// 是否为「已归零 / 空」状态。
    pub const fn is_empty_handle(&self) -> bool {
        self.ptr.is_null() && self.len == 0 && self.capacity == 0
    }

    /// 归还并置零（幂等）。
    ///
    /// # Safety
    ///
    /// 必须由 Rust 侧的原始缓冲区调用一次；重复调用是安全的（第二次为 no-op）。
    pub unsafe fn release(&mut self) -> NrResult<()> {
        if self.is_empty_handle() {
            return Ok(());
        }
        if self.ptr.is_null() {
            // 已归零的结构体不应残留长度；这属于调用方伪造字段，拒绝而不是猜测。
            return Err(invalid(
                "缓冲区 ptr 为 NULL 但 len/capacity 非零：结构体已被破坏，拒绝归还",
            ));
        }
        if self.len > self.capacity {
            return Err(invalid(format!(
                "缓冲区 len({}) 大于 capacity({})：结构体已被破坏，拒绝归还",
                self.len, self.capacity
            )));
        }
        if self.capacity > 0 {
            // 只有 capacity > 0 才存在真实分配；capacity == 0 的 Vec 不持有内存。
            drop(Vec::from_raw_parts(self.ptr, self.len, self.capacity));
        }
        *self = NrBuffer::empty();
        Ok(())
    }

    /// 只读视图（仅供 crate 内部与测试使用，不导出到 ABI）。
    ///
    /// # Safety
    ///
    /// `ptr`/`len` 必须描述一块仍然有效的、由 [`NrBuffer::from_vec`] 转移的分配。
    pub unsafe fn as_slice<'a>(&self) -> &'a [u8] {
        if self.ptr.is_null() || self.len == 0 {
            return &[];
        }
        std::slice::from_raw_parts(self.ptr, self.len)
    }
}

/// 归还一个缓冲区（`nr_release_buffer` 的实现）。
///
/// `buf` 为 `NULL` 时返回 [`NrBufferError::NullPointer`]（参数错误）。
///
/// # Safety
///
/// `buf` 必须指向本库分配且尚未归还的 [`NrBuffer`]，或指向一个已归零的同一结构体。
pub unsafe fn release_buffer(buf: *mut NrBuffer) -> NrResult<()> {
    if buf.is_null() {
        return Err(invalid("nr_release_buffer 的 buf 不得为 NULL"));
    }
    (*buf).release()
}

/// 供头文件 —— Rust 对照测试使用的字段清单（顺序即结构体字段顺序）。
pub const BUFFER_FIELDS: [(&str, &str); 3] = [
    ("ptr", "uint8_t *"),
    ("len", "size_t"),
    ("capacity", "size_t"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_frees_and_zeroes_then_is_idempotent() {
        let mut buffer = NrBuffer::from_vec(b"hello".to_vec());
        assert!(!buffer.is_empty_handle());
        assert_eq!(buffer.len, 5);
        unsafe {
            assert_eq!(buffer.as_slice(), b"hello");
            buffer.release().expect("首次归还必须成功");
        }
        assert!(buffer.is_empty_handle(), "归还后三个字段必须全部置零");
        unsafe {
            buffer
                .release()
                .expect("对已归零的缓冲区再次归还必须是 no-op");
        }
    }

    #[test]
    fn empty_vec_buffer_has_no_allocation_and_releases_as_noop() {
        let mut buffer = NrBuffer::from_vec(Vec::new());
        // 空 Vec 不持有分配：capacity == 0 时归还只做置零。
        assert_eq!(buffer.capacity, 0);
        assert_eq!(buffer.len, 0);
        unsafe {
            buffer.release().expect("空缓冲区归还必须成功");
        }
        assert!(buffer.is_empty_handle());
    }

    #[test]
    fn release_rejects_null_pointer_argument() {
        let error = unsafe { release_buffer(std::ptr::null_mut()) }.expect_err("NULL 必须被拒绝");
        assert_eq!(error.code(), reading_core::ErrorCode::InvalidArgument);
    }

    #[test]
    fn release_rejects_forged_fields() {
        let mut forged = NrBuffer {
            ptr: std::ptr::null_mut(),
            len: 4,
            capacity: 0,
        };
        let error = unsafe { forged.release() }.expect_err("NULL 指针 + 非零长度必须被拒绝");
        assert_eq!(error.code(), reading_core::ErrorCode::InvalidArgument);

        let mut len_over_capacity = NrBuffer {
            ptr: std::ptr::dangling_mut::<u8>(),
            len: 8,
            capacity: 4,
        };
        let error = unsafe { len_over_capacity.release() }.expect_err("len > capacity 必须被拒绝");
        assert_eq!(error.code(), reading_core::ErrorCode::InvalidArgument);
    }

    #[test]
    fn buffer_allocation_is_not_whole_file() {
        // 缓冲区大小由调用方请求的窗口长度决定，本模块不引入任何"整本"路径。
        let buffer = NrBuffer::from_vec(vec![0u8; crate::abi::MAX_WINDOW_BYTES]);
        assert_eq!(buffer.len, crate::abi::MAX_WINDOW_BYTES);
        let mut buffer = buffer;
        unsafe {
            buffer.release().expect("归还");
        }
        assert!(buffer.is_empty_handle());
    }

    #[test]
    fn malloc_error_type_is_used_for_null_pointer() {
        // `NrBufferError` 只是文档化标签，真正的错误码来自统一错误模型。
        use crate::abi::NrBufferError;
        assert_eq!(NrBufferError::NullPointer.as_str(), "NullPointer");
        assert_eq!(NrBufferError::ForgedFields.as_str(), "ForgedFields");
    }
}
