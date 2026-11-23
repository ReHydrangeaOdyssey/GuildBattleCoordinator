use crate::{protocol::game::privateapi::ScheduledGuildBattle, rpc::Failure};
const MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const INCREMENT: u64 = 1_442_695_040_888_963_407;
const TIME_SEED: u64 = 202_205_311_459;
const DATE_FACTOR: u64 = 100_000_000_000;
const SLOT_FACTOR: u64 = 100_000_000;
struct Random(u64);
impl Random {
    fn new(seed: u64) -> Self {
        let mut random = Self(0);
        random.next();
        random.0 = random.0.wrapping_add(seed);
        random.next();
        random
    }
    fn next(&mut self) -> u32 {
        let old = self.0;
        // PCG state' = state * multiplier + increment modulo 2^64.
        self.0 = old.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
        (((old >> 18 ^ old) >> 27) as u32).rotate_right((old >> 59) as u32)
    }
    fn bounded(&mut self, bound: u32) -> u32 {
        // Rejection of the incomplete residue range removes modulo bias.
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next();
            if value >= threshold {
                return value % bound;
            }
        }
    }
}
pub fn pairs(
    mut guilds: Vec<u64>,
    ymd: u64,
    slot: i32,
    micros: u64,
) -> Result<Vec<ScheduledGuildBattle>, Failure> {
    guilds.sort_unstable();
    if guilds.contains(&0)
        || guilds.windows(2).any(|w| w[0] == w[1])
        || guilds.len() > u32::MAX as usize
        || !(0..6).contains(&slot)
    {
        return Err(Failure::BAD_REQUEST);
    }
    let mut random = Random::new(TIME_SEED ^ micros);
    // Fisher-Yates: swap index i with an unbiased sample from [0, i].
    for i in (1..guilds.len()).rev() {
        let j = random.bounded((i + 1) as u32) as usize;
        guilds.swap(i, j);
    }
    guilds
        .chunks(2)
        .enumerate()
        .map(|(index, guilds)| {
            // Normal battle ID = YYYYMMDD * 10^11 + slot * 10^8 + pair index.
            let id = ymd
                .checked_mul(DATE_FACTOR)
                .and_then(|v| v.checked_add(slot as u64 * SLOT_FACTOR))
                .and_then(|v| v.checked_add(index as u64))
                .filter(|v| *v < (1u64 << 63))
                .ok_or(Failure::BAD_REQUEST)?;
            if index as u64 >= SLOT_FACTOR {
                return Err(Failure::BAD_REQUEST);
            }
            Ok(ScheduledGuildBattle {
                guild_battle_id: id,
                guild_id: vec![guilds[0], guilds.get(1).copied().unwrap_or(0)],
            })
        })
        .collect()
}
