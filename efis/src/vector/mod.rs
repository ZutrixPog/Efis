use std::cmp::Ordering;

use rayon::prelude::*;

pub mod flat;

type Vector<'a> = &'a [f32];

pub trait Index {
    fn insert(&mut self, id: u64, v: Vector) -> anyhow::Result<()>;
    fn search(&self, query: Vector, k: usize, df: Distance) -> Vec<(u64, f32)>;
    fn delete(&mut self, id: u64);
}

pub enum Distance {
    L2,
    Cosine,
}

#[derive(PartialEq)]
struct F32(f32);

impl Eq for F32 {}

impl PartialOrd for F32 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for F32 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.partial_cmp(&other.0).unwrap_or(Ordering::Equal)
    }
}

#[inline]
pub fn l2_distance(a: Vector, b: Vector) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum()
}

// NOTE: expects vectors to be normalized
#[inline]
pub fn cosine_sim(a: Vector, b: Vector) -> f32 {
    -a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
}
