use std::collections::{HashSet, VecDeque};
use std::ops::Bound::Included;

mod types {
    pub struct Text(String);
    pub struct Number(f64);

    pub struct List<T>{
        list: VecDeque<T>
    }

    impl<T> List<T>{
        pub fn new() -> Self {
            List<T> {
                list: VecDeque::new()
            }
        }

        pub fn push_front(&mut self, val: T) {
            self.list.push_front(val);
        }

        pub fn pop_back() -> Option<T> {
            self.list.pop_back()
        }

        pub fn push_back(val: T) {
            self.list.push_back(val);
        }

        pub fn pop_front() -> Option<T> {
            self.list.pop_front()
        }
    }
    
    pub struct Set<T>{
        set: HashSet<T>
    }

    impl<T> Set<T> {
        pub fn new() -> Self {
            Set<T> {
                set: HashSet::new()
            }
        }

        pub fn insert(&mut self, val: T) {
            self.set.insert(val);
        }

        pub fn contains(&mut self, val: &T) -> bool {
            self.set.contains(val)
        }

        pub fn remove(&mut self, val: &T) {
            self.set.remove(val);
        }

        pub fn iter(&self) -> Iter<'_, T> {
            self.set.iter()
        }
    }
    
    pub struct SortedSet<K, V>{
        set: BTreeMap<K, V>
    }
    
    impl<K, V> SortedSet<K, V> {
        pub fn new() -> Self {
            SortedSet<T> {
                set: BTreeMap::new()
            }
        }

        pub fn insert<K: Ord, V>(&mut self, score: K, val: T) {
            self.set.insert(score, val);
        }

        pub fn pop(&mut self) -> Option<(K, V)> {
            self.set.pop_first()
        }

        pub fn range(&self, start: &K, end: &K) -> Iter<'_, T> {
            self.set.range(Included(end), Included(start)).map(|curr| curr.1).into_iter()
        }
    }
}
