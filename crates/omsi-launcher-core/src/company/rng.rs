//! A predictable chance: the same company on the same day draws the same numbers, also after
//! a restart (Omsi-Hub's `reeks`: a market that changed every time the page was opened would
//! be shopped until it offered the best bus). SplitMix64, seeded from a hash of words.

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    /// Seeded from words and a number (the company's id, the day, what is drawn for).
    pub fn of(parts: &[&str], n: i64) -> Rng {
        // (FNV-1a)
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for p in parts {
            for b in p.bytes().chain(std::iter::once(0xff)) {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        for b in n.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        Rng::new(h)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// 0 ≤ x < 1.
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f64()
    }

    /// A whole number in `a..=b`.
    pub fn int(&mut self, a: i64, b: i64) -> i64 {
        if b <= a {
            return a;
        }
        a + (self.next_u64() % (b - a + 1) as u64) as i64
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            items.get((self.next_u64() % items.len() as u64) as usize)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_draws_the_same() {
        let a: Vec<f64> = (0..5).map({
            let mut r = Rng::of(&["firm", "2024-01-01"], 3);
            move |_| r.f64()
        }).collect();
        let b: Vec<f64> = (0..5).map({
            let mut r = Rng::of(&["firm", "2024-01-01"], 3);
            move |_| r.f64()
        }).collect();
        assert_eq!(a, b);
        assert!(a.iter().all(|x| (0.0..1.0).contains(x)));
        let mut c = Rng::of(&["firm", "2024-01-02"], 3);
        assert_ne!(a[0], c.f64());
        let mut r = Rng::new(1);
        for _ in 0..100 {
            let k = r.int(3, 5);
            assert!((3..=5).contains(&k));
        }
    }
}
