use std::rc::Rc;

const HEADER = 4;
const BTREE_PAGE_SIZE = 4096;
const BTREE_MAX_KEY_SIZE = 1000;
const BTREE_MAX_VAL_SIZE = 3000;

pub struct b_node {
    data: Vec<u8>
}

impl b_node {
    pub fn new_node(t: i32, leaf: bool) -> Self {
        b_node {
            data: Vec::new()
        }
    }

    pub fn b_type(&self) -> u16 {
        u16::from_le_bytes([self.data[0], self.data[1]])
    }

    pub fn n_keys(&self) -> u16 {
        u16::from_le_bytes([self.data[2], self.data[3]])
    }

    pub fn set_header(&mut self, btype: u16, nkeys: u16) {
        self.data[..2].copy_from_slice(&btype.to_le_bytes());
        self.data[2..4].copy_from_slice(&nkeys.to_le_bytes());
    }
 
    fn split_child(&mut self, i: i32, mut y: Rc<b_node>) {
        let mut z = b_node::new_node(y.t, y.leaf);
        let mutref = Rc::get_mut(&mut z).unwrap();
        mutref.n = self.t -1;

        for j in 0..self.t-1 {
            mutref.keys[j as usize] = y.keys[(j+self.t) as usize];
        }

        if y.leaf == false {
            for j in 0..self.t {
                mutref.nodes[j as usize] = Rc::clone(&y.nodes[(j+self.t) as usize]);
            }
        }

        Rc::get_mut(&mut y).unwrap().n = self.t -1;

        for j in i+1..self.n {
            self.nodes[(j+1) as usize] = Rc::clone(&self.nodes[j as usize]);
        }

        self.nodes[(i+1) as usize] = z;

        for j in i..self.n -1 {
            self.keys[(j+1) as usize] = self.keys[j as usize];
        }

        self.keys[i as usize] = y.keys[(self.t-1) as usize];
        self.n = self.n + 1;
    }

    pub fn traverse(&self) {
        for (i, node) in self.nodes.iter().enumerate() {
            if self.leaf == false {
                node.traverse();
            }
            print!(" {}", self.keys[i as usize]);
        }

        if self.leaf == false {
            if let Some(last) = self.nodes.last() {
                last.traverse();
            }
        }
    }

    pub fn search(&self, k: i32) -> Option<Rc<b_node>> {
        let mut i:i32 = 0;
        while i < self.n && k > self.keys[i as usize] {
            i += 1;
        }

        if self.keys[i as usize] == k {
            return Some(Rc::new(&self));
        }

        if self.leaf == true {
            return None;
        }

        self.nodes[i as usize].search(k)
    }
}

pub struct b_tree {
    root: Option<u64>,
    t: i32
}

impl b_tree {
    pub fn new_tree(t: i32) -> b_tree {
        b_tree {
            root: None,
            t
        }
    }

    pub fn traverse(&self) {
        if let Some(root) = &self.root {
            root.traverse();
        }
    }

    pub fn search(&self, k: i32) -> Option<Rc<b_node>> {
        if let Some(root) = &self.root {
            root.search(k)
        } else {
            None
        }
    }

    pub fn insert(&mut self, k: i32) {
        if let Some(root) = &self.root {
            if self.root.as_ref().unwrap().n == 2*self.t-1 {
                let node = b_node::new_node(self.t, false);
                let mutnode = Rc::get_mut(&mut node).unwrap();
                mutnode.nodes[0] = Rc::clone(&root);
                mutnode.split_child(0, root);

                let mut i = 0;
                if node.keys[0] < k {
                    i += 1;
                }
                node.nodes[i as usize].insert_non_full(k);

                self.root = Some(node);
            } else {
                self.root.as_ref().unwrap().insert_non_full(k);
            }
        } else {
            self.root = Some(b_node::new_node(self.t, true));
            self.root.as_ref().unwrap().keys[0] = k;
            self.root.as_ref().unwrap().n = 1;
        }
    }
}