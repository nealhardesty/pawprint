//! The frozen hash used for fallback mascots and accents, plus the
//! animation PRNG (SPEC §5.3, §7.1). No dependencies.
//!
//! **`fnv1a64` and `splitmix64` must never change.** Fresh installs agree on
//! a host's fallback mascot only while they stay frozen; the test vectors
//! below enforce that.

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Rendezvous score of mascot `id` for host `short`.
pub fn score(short: &str, id: &str) -> u64 {
    let mut bytes = Vec::with_capacity(short.len() + 1 + id.len());
    bytes.extend_from_slice(short.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(id.as_bytes());
    splitmix64(fnv1a64(&bytes))
}

/// Highest-random-weight pick: the id with the highest score, ties broken
/// by the lexicographically smallest id. `None` if there are no ids.
pub fn hrw_pick<'a>(short: &str, ids: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    ids.into_iter()
        .map(|id| (score(short, id), id))
        .max_by(|(sa, ia), (sb, ib)| sa.cmp(sb).then_with(|| ib.cmp(ia)))
        .map(|(_, id)| id)
}

/// xorshift64* — animation timing only; reproducibility is not required.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// Any seed works; zero is remapped because xorshift can't leave zero.
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 {
            0x9e37_79b9_7f4a_7c15
        } else {
            seed
        })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in `lo..=hi` (modulo bias is irrelevant at these ranges).
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            lo
        } else {
            lo + self.next_u64() % (hi - lo + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC §5.3 frozen vectors. If this test fails, the change is wrong.
    #[test]
    fn frozen_vectors() {
        let cases: [(&[u8], u64, Option<u64>); 5] = [
            (b"", 0xcbf29ce484222325, None),
            (b"a", 0xaf63dc4c8601ec8c, None),
            (
                b"buildbox\0owl",
                0x3251b145c0609532,
                Some(0x959e9e0228cae98d),
            ),
            (
                b"buildbox\0otter",
                0x46a1d27c6d1c72b2,
                Some(0xc9611fca31e71b6f),
            ),
            (
                b"prod-db-01\0owl",
                0x2412edb9d3386759,
                Some(0xfa6c62cfbcb0e79d),
            ),
        ];
        for (input, fnv, mixed) in cases {
            assert_eq!(fnv1a64(input), fnv, "fnv1a64({input:?})");
            if let Some(mixed) = mixed {
                assert_eq!(splitmix64(fnv), mixed, "splitmix64 of {input:?}");
            }
        }
        assert_eq!(score("buildbox", "owl"), 0x959e9e0228cae98d);
        assert_eq!(score("prod-db-01", "owl"), 0xfa6c62cfbcb0e79d);
    }

    #[test]
    fn hrw_picks_highest_score() {
        // owl 0x959e... < otter 0xc961... for buildbox.
        assert_eq!(hrw_pick("buildbox", ["owl", "otter"]), Some("otter"));
        assert_eq!(
            hrw_pick("buildbox", ["otter", "owl"]),
            Some("otter"),
            "order-independent"
        );
        assert_eq!(hrw_pick("buildbox", []), None);
        // A tie (same id twice) resolves to that id.
        assert_eq!(hrw_pick("x", ["b", "b"]), Some("b"));
    }

    /// SPEC §16.1: 10,000 hosts over 100 mascots, no mascot above 3× its
    /// expected share.
    #[test]
    fn hrw_distribution() {
        let ids: Vec<String> = (0..100).map(|i| format!("mascot-{i:03}")).collect();
        let mut counts = vec![0u32; ids.len()];
        for h in 0..10_000 {
            let host = format!("host-{h}");
            let pick = hrw_pick(&host, ids.iter().map(String::as_str)).unwrap();
            counts[ids.iter().position(|i| i == pick).unwrap()] += 1;
        }
        let max = *counts.iter().max().unwrap();
        assert!(max <= 300, "a mascot got {max} of 10000 (expected ~100)");
    }

    #[test]
    fn rng_range_is_inclusive_and_varies() {
        let mut r = Rng::new(0);
        let draws: Vec<u64> = (0..1000).map(|_| r.range(8, 20)).collect();
        assert!(draws.iter().all(|&d| (8..=20).contains(&d)));
        assert!(draws.contains(&8) && draws.contains(&20));
        assert_eq!(Rng::new(5).range(3, 3), 3);
    }
}
