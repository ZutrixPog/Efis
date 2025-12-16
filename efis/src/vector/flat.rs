use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap};

use crate::vector::{DistanceFn, Index, Vector, F32};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlatIndex {
    dim: usize,
    vecs: Vec<f32>,
    ids: Vec<String>,
    idset: HashMap<String, usize>,
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
    fn insert(&mut self, id: String, v: Vector) -> anyhow::Result<()> {
        if v.len() != self.dim {
            return Err(anyhow::format_err!(
                "vector size is not equal to the original size"
            ));
        }
        if let Some(i) = self.idset.get(&id) {
            let start = self.dim * i;
            let end = start + self.dim;
            self.vecs.splice(start..end, v.into_iter().map(|e| *e));
        } else {
            self.vecs.extend_from_slice(v);
            self.idset.insert(id.clone(), self.ids.len());
            self.ids.push(id);
        }
        Ok(())
    }

    fn search(&self, query: Vector, k: usize, df: DistanceFn) -> Vec<(String, f32)> {
        if k == 0 {
            return Vec::new();
        }

        let mut heap: BinaryHeap<(F32, String)> = BinaryHeap::new();

        for (i, id) in self.ids.iter().enumerate() {
            let start = self.dim * i;
            let end = start + self.dim;
            let v = &self.vecs[start..end];
            let d = F32(df(v, query));

            if heap.len() < k {
                heap.push((d, id.clone()));
            } else if heap.peek().unwrap().0 > d {
                heap.pop();
                heap.push((d, id.clone()));
            }
        }

        let mut topk = Vec::with_capacity(heap.len());
        while let Some((d, id)) = heap.pop() {
            topk.push((id, d.0));
        }
        topk.reverse();
        topk
    }

    fn delete(&mut self, id: String) {
        if let Ok(i) = self.ids.binary_search(&id) {
            self.ids.remove(i);
            let start = self.dim * i;
            self.vecs.drain(start..start + self.dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::vector::{cosine_sim, l2_distance};

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

        assert!(idx.insert("10".to_string(), &[1.0, 2.0, 3.0]).is_ok());
        assert!(idx.insert("20".to_string(), &[4.0, 5.0, 6.0]).is_ok());

        assert_eq!(idx.ids, vec!["10", "20"]);
        assert_eq!(idx.vecs, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_insert_dimension_mismatch() {
        let mut idx = new_index(3);
        assert!(idx.insert("1".to_string(), &[1.0, 2.0]).is_err());
    }

    #[test]
    fn test_search_exact_match() {
        let mut idx = new_index(3);
        assert!(idx.insert("1".to_string(), &[0.0, 0.0, 0.0]).is_ok());
        assert!(idx.insert("2".to_string(), &[1.0, 0.0, 0.0]).is_ok());
        assert!(idx.insert("3".to_string(), &[2.0, 0.0, 0.0]).is_ok());
        let res = idx.search(&[0.0, 0.0, 0.0], 1, l2_distance);

        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, "1".to_string()); // closest ID
        assert!((res[0].1 - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_search_k_neighbors() {
        let mut idx = new_index(2);
        assert!(idx.insert("1".to_string(), &[0.0, 0.0]).is_ok());
        assert!(idx.insert("2".to_string(), &[1.0, 0.0]).is_ok());
        assert!(idx.insert("3".to_string(), &[2.0, 0.0]).is_ok());

        let res = idx.search(&[0.0, 0.0], 2, l2_distance);

        assert_eq!(res.len(), 2);

        let ids: Vec<&str> = res.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"1"));
        assert!(ids.contains(&"2"));
    }

    #[test]
    fn test_search_with_cosine_product() {
        let mut idx = new_index(3);

        assert!(idx.insert("10".to_string(), &[1.0, 0.0, 0.0]).is_ok());
        assert!(idx.insert("20".to_string(), &[0.5, 0.0, 0.0]).is_ok());

        let res = idx.search(&[1.0, 0.0, 0.0], 1, cosine_sim);

        assert_eq!(res[0].0, "10".to_string());
    }

    #[test]
    fn test_search_empty_index_returns_empty() {
        let idx = new_index(3);
        let res = idx.search(&[1.0, 2.0, 3.0], 5, l2_distance);

        assert!(res.is_empty());
    }

    #[test]
    fn test_delete_removes_id() {
        let mut idx = new_index(3);

        assert!(idx.insert("10".to_string(), &[1.0, 1.0, 1.0]).is_ok());
        assert!(idx.insert("20".to_string(), &[2.0, 2.0, 2.0]).is_ok());
        assert!(idx.insert("30".to_string(), &[3.0, 3.0, 3.0]).is_ok());

        idx.delete("20".to_string());

        assert_eq!(idx.ids, vec!["10".to_string(), "30".to_string()]);

        assert_eq!(idx.vecs, vec![1.0, 1.0, 1.0, 3.0, 3.0, 3.0]);
    }

    #[test]
    fn test_delete_nonexistent_id_does_nothing() {
        let mut idx = new_index(3);
        assert!(idx.insert("1".to_string(), &[9.0, 9.0, 9.0]).is_ok());

        idx.delete("999".to_string());

        assert_eq!(idx.ids, vec!["1".to_string()]);
        assert_eq!(idx.vecs, vec![9.0, 9.0, 9.0]);
    }
}
