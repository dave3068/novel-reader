//! 确定性伪随机数（xorshift64*）。用于模拟领域数据与合成文本生成，
//! 保证同种子同结果（S-R4）。

/// 固定种子确定性 PRNG。
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// 由种子创建；种子为 0 时替换为常量，避免退化。
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    /// 下一个 64 位随机数。
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// `[0, n)` 内的随机数；`n == 0` 时返回 0。
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    /// `[lo, hi]` 闭区间内的随机数。
    pub fn range_inclusive(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + self.below(hi - lo + 1)
    }

    /// 从切片中确定性挑选一项。
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        let idx = self.below(items.len() as u64) as usize;
        &items[idx.min(items.len().saturating_sub(1))]
    }
}
