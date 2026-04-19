use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
};

use crate::vector::{cosine_sim, l2_distance, Distance, Index, Vector, F32};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlatIndex {
    dim: usize,
    vecs: Vec<f32>,
    ids: Vec<u64>,
    idset: HashMap<u64, usize>,
}

impl FlatIndex {
    pub fn new(n: usize) -> FlatIndex {
        FlatIndex {
            dim: n,
            vecs: Vec::new(),
            ids: Vec::new(),
            idset: HashMap::new(),
        }
    }
}

impl Index for FlatIndex {
    fn insert(&mut self, id: u64, v: Vector) -> anyhow::Result<()> {
        if v.len() != self.dim {
            return Err(anyhow::format_err!(
                "vector size is not equal to the original size"
            ));
        }
        if let Some(i) = self.idset.get(&id) {
            let start = self.dim * i;
            let end = start + self.dim;
            for j in start..end {
                self.vecs[j] = v[j - start];
            }
        } else {
            self.vecs.extend_from_slice(v);
            self.idset.insert(id, self.ids.len());
            self.ids.push(id);
        }
        Ok(())
    }

    fn search(&self, query: Vector, k: usize, df: Distance) -> Vec<(u64, f32)> {
        if k == 0 {
            return Vec::new();
        }

        let dist_fn = match df {
            Distance::L2 => l2_distance,
            Distance::Cosine => cosine_sim,
        };

        let mut distances: Vec<(u64, f32)> = self
            .vecs
            .par_chunks(self.dim)
            .zip(self.ids.par_iter())
            .map(|(chunk, &id)| (id, dist_fn(chunk, query)))
            .collect();

        let k = k.min(distances.len());
        distances.select_nth_unstable_by(k - 1, |a, b| a.1.partial_cmp(&b.1).unwrap());

        distances.truncate(k);
        distances.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        distances
    }

    fn delete(&mut self, id: u64) {
        let mut index = None;
        for (i, &vid) in self.ids.iter().enumerate() {
            if id == vid {
                index = Some(i);
                break;
            }
        }
        if let Some(i) = index {
            let start = self.dim * i;
            let end = start + self.dim;
            let new_len = self.vecs.len() - self.dim;
            for i in start..end {
                self.vecs[i] = self.vecs[new_len + i - start];
            }
            self.vecs.truncate(new_len);
            self.ids.swap_remove(i);
            self.idset.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_index(dim: usize) -> FlatIndex {
        FlatIndex {
            dim,
            vecs: Vec::new(),
            ids: Vec::new(),
            idset: HashMap::new(),
        }
    }

    #[test]
    fn test_insert_and_internal_storage() {
        let mut idx = new_index(3);

        assert!(idx.insert(10, &[1.0, 2.0, 3.0]).is_ok());
        assert!(idx.insert(20, &[4.0, 5.0, 6.0]).is_ok());

        assert_eq!(idx.ids, vec![10, 20]);
        assert_eq!(idx.vecs, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_insert_dimension_mismatch() {
        let mut idx = new_index(3);
        assert!(idx.insert(1, &[1.0, 2.0]).is_err());
    }

    #[test]
    fn test_search_exact_match() {
        let mut idx = new_index(3);
        assert!(idx.insert(1, &[0.0, 0.0, 0.0]).is_ok());
        assert!(idx.insert(2, &[1.0, 0.0, 0.0]).is_ok());
        assert!(idx.insert(3, &[2.0, 0.0, 0.0]).is_ok());
        let res = idx.search(&[0.0, 0.0, 0.0], 1, Distance::L2);

        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, 1);
        assert!((res[0].1 - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_search_k_neighbors() {
        let mut idx = new_index(2);
        assert!(idx.insert(1, &[0.0, 0.0]).is_ok());
        assert!(idx.insert(2, &[1.0, 0.0]).is_ok());
        assert!(idx.insert(3, &[2.0, 0.0]).is_ok());

        let res = idx.search(&[0.0, 0.0], 2, Distance::L2);

        assert_eq!(res.len(), 2);

        let ids: Vec<u64> = res.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
    }

    #[test]
    fn test_search_with_cosine_product() {
        let mut idx = new_index(3);

        assert!(idx.insert(10, &[1.0, 0.0, 0.0]).is_ok());
        assert!(idx.insert(20, &[0.5, 0.0, 0.0]).is_ok());

        let res = idx.search(&[1.0, 0.0, 0.0], 1, Distance::Cosine);

        assert_eq!(res[0].0, 10);
    }

    #[test]
    fn test_search_empty_index_returns_empty() {
        let idx = new_index(3);
        let res = idx.search(&[1.0, 2.0, 3.0], 5, Distance::L2);

        assert!(res.is_empty());
    }

    #[test]
    fn test_delete_removes_id() {
        let mut idx = new_index(3);

        assert!(idx.insert(10, &[1.0, 1.0, 1.0]).is_ok());
        assert!(idx.insert(20, &[2.0, 2.0, 2.0]).is_ok());
        assert!(idx.insert(30, &[3.0, 3.0, 3.0]).is_ok());

        idx.delete(20);

        assert_eq!(idx.ids, vec![10, 30]);

        assert_eq!(idx.vecs, vec![1.0, 1.0, 1.0, 3.0, 3.0, 3.0]);
    }

    #[test]
    fn test_delete_nonexistent_id_does_nothing() {
        let mut idx = new_index(3);
        assert!(idx.insert(1, &[9.0, 9.0, 9.0]).is_ok());

        idx.delete(999);

        assert_eq!(idx.ids, vec![1]);
        assert_eq!(idx.vecs, vec![9.0, 9.0, 9.0]);
    }
}
