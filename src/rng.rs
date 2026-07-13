// 外部クレートに依存しない小さな乱数生成器(xorshift64)。
// 決定的なので、テストではシードを固定して挙動を検証できる。

#[derive(Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // シード0は xorshift が縮退するので避ける
        Rng {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    // 現在時刻からシードを作る(起動時用)
    pub fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678);
        Rng::new(nanos)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    // 0.0..1.0 の一様乱数
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    // lo..hi の一様乱数
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    // -1.0..1.0 の一様乱数
    pub fn signed(&mut self) -> f64 {
        self.next_f64() * 2.0 - 1.0
    }

    // lo..=hi の整数
    pub fn range_usize(&mut self, lo: usize, hi: usize) -> usize {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() as usize) % (hi - lo + 1)
    }
}
