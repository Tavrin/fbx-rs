//! Allocation-fallible ordered maps backed by a balanced index tree.
//! Rotations and iteration use indices rather than pointers or scratch buffers.
use crate::api_alloc::Budget;
use crate::Error;
use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::ops::{Index, RangeFrom};

#[derive(Clone)]
struct Node<K, V> {
    pair: Option<(K, V)>,
    left: Option<usize>,
    right: Option<usize>,
    parent: Option<usize>,
    height: usize,
}
#[derive(Clone)]
pub(crate) struct Map<K, V> {
    nodes: Vec<Node<K, V>>,
    root: Option<usize>,
}
impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self::new()
    }
}
impl<K, V> Map<K, V> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: None,
        }
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    fn height(&self, i: Option<usize>) -> usize {
        i.map_or(0, |i| self.nodes[i].height)
    }
    fn update(&mut self, i: usize) {
        self.nodes[i].height = 1 + self
            .height(self.nodes[i].left)
            .max(self.height(self.nodes[i].right));
    }
    fn connect(&mut self, parent: Option<usize>, old: usize, new: usize) {
        match parent {
            None => self.root = Some(new),
            Some(p) if self.nodes[p].left == Some(old) => self.nodes[p].left = Some(new),
            Some(p) => self.nodes[p].right = Some(new),
        }
        self.nodes[new].parent = parent;
    }
    fn left(&mut self, x: usize) -> usize {
        let y = self.nodes[x].right.expect("right child for left rotation");
        let middle = self.nodes[y].left;
        let parent = self.nodes[x].parent;
        self.connect(parent, x, y);
        self.nodes[y].left = Some(x);
        self.nodes[x].parent = Some(y);
        self.nodes[x].right = middle;
        if let Some(m) = middle {
            self.nodes[m].parent = Some(x);
        }
        self.update(x);
        self.update(y);
        y
    }
    fn right(&mut self, x: usize) -> usize {
        let y = self.nodes[x].left.expect("left child for right rotation");
        let middle = self.nodes[y].right;
        let parent = self.nodes[x].parent;
        self.connect(parent, x, y);
        self.nodes[y].right = Some(x);
        self.nodes[x].parent = Some(y);
        self.nodes[x].left = middle;
        if let Some(m) = middle {
            self.nodes[m].parent = Some(x);
        }
        self.update(x);
        self.update(y);
        y
    }
    fn rebalance(&mut self, mut at: Option<usize>) {
        while let Some(i) = at {
            self.update(i);
            let left = self.height(self.nodes[i].left);
            let right = self.height(self.nodes[i].right);
            let top = if left > right + 1 {
                let child = self.nodes[i].left.expect("left-heavy child");
                if self.height(self.nodes[child].right) > self.height(self.nodes[child].left) {
                    self.left(child);
                }
                self.right(i)
            } else if right > left + 1 {
                let child = self.nodes[i].right.expect("right-heavy child");
                if self.height(self.nodes[child].left) > self.height(self.nodes[child].right) {
                    self.right(child);
                }
                self.left(i)
            } else {
                i
            };
            at = self.nodes[top].parent;
        }
    }
    fn minimum(&self, mut at: Option<usize>) -> Option<usize> {
        while let Some(i) = at {
            match self.nodes[i].left {
                Some(left) => at = Some(left),
                None => return Some(i),
            }
        }
        None
    }
    fn successor(&self, mut i: usize) -> Option<usize> {
        if self.nodes[i].right.is_some() {
            return self.minimum(self.nodes[i].right);
        }
        while let Some(p) = self.nodes[i].parent {
            if self.nodes[p].left == Some(i) {
                return Some(p);
            }
            i = p;
        }
        None
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|(_, v)| v)
    }
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            cursor: Cursor {
                map: self,
                at: self.minimum(self.root),
            },
            remaining: self.len(),
        }
    }
}
impl<K: Ord, V> Map<K, V> {
    fn find<Q: Ord + ?Sized>(&self, key: &Q) -> Option<usize>
    where
        K: Borrow<Q>,
    {
        let mut at = self.root;
        while let Some(i) = at {
            let stored: &Q = self.nodes[i]
                .pair
                .as_ref()
                .expect("occupied node")
                .0
                .borrow();
            match key.cmp(stored) {
                Ordering::Less => at = self.nodes[i].left,
                Ordering::Greater => at = self.nodes[i].right,
                Ordering::Equal => return Some(i),
            }
        }
        None
    }
    pub fn get<Q: Ord + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.find(key)
            .map(|i| &self.nodes[i].pair.as_ref().expect("occupied node").1)
    }
    pub fn get_mut<Q: Ord + ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        self.find(key)
            .map(|i| &mut self.nodes[i].pair.as_mut().expect("occupied node").1)
    }
    pub fn contains_key<Q: Ord + ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.find(key).is_some()
    }
    fn insert_index(
        &mut self,
        key: K,
        value: V,
        budget: &Budget,
    ) -> Result<(usize, Option<V>), Error> {
        let mut at = self.root;
        let mut parent = None;
        let mut left = false;
        while let Some(i) = at {
            match key.cmp(&self.nodes[i].pair.as_ref().expect("occupied node").0) {
                Ordering::Equal => {
                    return Ok((
                        i,
                        Some(std::mem::replace(
                            &mut self.nodes[i].pair.as_mut().expect("occupied node").1,
                            value,
                        )),
                    ))
                }
                cmp => {
                    parent = Some(i);
                    left = cmp == Ordering::Less;
                    at = if left {
                        self.nodes[i].left
                    } else {
                        self.nodes[i].right
                    };
                }
            }
        }
        let i = self.nodes.len();
        budget.push(
            &mut self.nodes,
            Node {
                pair: Some((key, value)),
                left: None,
                right: None,
                parent,
                height: 1,
            },
        )?;
        match parent {
            None => self.root = Some(i),
            Some(p) if left => self.nodes[p].left = Some(i),
            Some(p) => self.nodes[p].right = Some(i),
        }
        self.rebalance(parent);
        Ok((i, None))
    }
    pub fn try_insert(&mut self, key: K, value: V, budget: &Budget) -> Result<Option<V>, Error> {
        self.insert_index(key, value, budget).map(|(_, old)| old)
    }
    pub fn try_get_or_insert_with(
        &mut self,
        key: K,
        budget: &Budget,
        make: impl FnOnce() -> Result<V, Error>,
    ) -> Result<&mut V, Error> {
        let i = match self.find(&key) {
            Some(i) => i,
            None => self.insert_index(key, make()?, budget)?.0,
        };
        Ok(&mut self.nodes[i].pair.as_mut().expect("occupied node").1)
    }
    pub fn try_from_iter(
        iter: impl IntoIterator<Item = (K, V)>,
        budget: &Budget,
    ) -> Result<Self, Error> {
        let mut map = Self::new();
        for (key, value) in iter {
            map.try_insert(key, value, budget)?;
        }
        Ok(map)
    }
    pub fn range(&self, range: RangeFrom<K>) -> Range<'_, K, V> {
        let mut at = self.root;
        let mut start = None;
        while let Some(i) = at {
            let key = &self.nodes[i].pair.as_ref().expect("occupied node").0;
            if *key < range.start {
                at = self.nodes[i].right;
            } else {
                start = Some(i);
                at = self.nodes[i].left;
            }
        }
        Range {
            cursor: Cursor {
                map: self,
                at: start,
            },
        }
    }
}
impl<K: Ord, V> Index<&K> for Map<K, V> {
    type Output = V;
    fn index(&self, key: &K) -> &V {
        self.get(key).expect("present key")
    }
}
struct Cursor<'a, K, V> {
    map: &'a Map<K, V>,
    at: Option<usize>,
}
impl<'a, K, V> Cursor<'a, K, V> {
    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        let i = self.at?;
        let map = self.map;
        self.at = map.successor(i);
        let pair = map.nodes[i].pair.as_ref().expect("occupied node");
        Some((&pair.0, &pair.1))
    }
}
pub(crate) struct Iter<'a, K, V> {
    cursor: Cursor<'a, K, V>,
    remaining: usize,
}
impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        let item = self.cursor.next()?;
        self.remaining -= 1;
        Some(item)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl<K, V> ExactSizeIterator for Iter<'_, K, V> {}
pub(crate) struct Range<'a, K, V> {
    cursor: Cursor<'a, K, V>,
}
impl<'a, K, V> Iterator for Range<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        self.cursor.next()
    }
}
impl<'a, K, V> IntoIterator for &'a Map<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
pub(crate) struct IntoIter<K, V> {
    map: Map<K, V>,
    at: Option<usize>,
    remaining: usize,
}
impl<K, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);
    fn next(&mut self) -> Option<Self::Item> {
        let i = self.at?;
        self.at = self.map.successor(i);
        self.remaining -= 1;
        self.map.nodes[i].pair.take()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl<K, V> ExactSizeIterator for IntoIter<K, V> {}
impl<K, V> IntoIterator for Map<K, V> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;
    fn into_iter(self) -> Self::IntoIter {
        let at = self.minimum(self.root);
        let remaining = self.len();
        IntoIter {
            map: self,
            at,
            remaining,
        }
    }
}
impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for Map<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}
impl<K: PartialEq, V: PartialEq> PartialEq for Map<K, V> {
    fn eq(&self, b: &Self) -> bool {
        self.len() == b.len() && self.iter().eq(b.iter())
    }
}
impl<K: Eq, V: Eq> Eq for Map<K, V> {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Set<K>(Map<K, ()>);
impl<K> Default for Set<K> {
    fn default() -> Self {
        Self(Map::new())
    }
}
impl<K: Ord> Set<K> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn try_insert(&mut self, key: K, budget: &Budget) -> Result<bool, Error> {
        Ok(self.0.try_insert(key, (), budget)?.is_none())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_rotations_preserve_parent_links_and_order() {
        let budget = Budget::new(1 << 20);
        for order in [[2, 0, 1], [0, 2, 1]] {
            let map = Map::try_from_iter(order.map(|k| (k, k * 2)), &budget).unwrap();
            assert_eq!(
                map.into_iter().collect::<Vec<_>>(),
                vec![(0, 0), (1, 2), (2, 4)]
            );
        }
        let mut map = Map::new();
        // A permutation modulo a prime exercises both rotations on mixed paths.
        for i in 0..1009 {
            let key = i * 493 % 1009;
            map.try_insert(key, key * 2, &budget).unwrap();
        }
        for i in 0..1009 {
            assert_eq!(map.get(&i), Some(&(i * 2)));
        }
        assert_eq!(
            map.range(500..).map(|(&k, _)| k).collect::<Vec<_>>(),
            (500..1009).collect::<Vec<_>>()
        );
        assert_eq!(
            map.into_iter().collect::<Vec<_>>(),
            (0..1009).map(|i| (i, i * 2)).collect::<Vec<_>>()
        );
    }
    #[test]
    fn balanced_ordering_duplicates_and_borrowed_lookup() {
        for n in [1000usize, 8000] {
            let budget = Budget::new(768 << 20);
            let mut map = Map::new();
            for i in (0..n).rev() {
                assert_eq!(map.try_insert(i, i * 2, &budget).unwrap(), None);
            }
            for i in 0..n {
                assert_eq!(map.get(&i), Some(&(i * 2)));
                assert_eq!(map.try_insert(i, i * 3, &budget).unwrap(), Some(i * 2));
            }
            assert!(
                map.height(map.root)
                    <= 2 * usize::try_from(usize::BITS - n.leading_zeros()).unwrap()
            );
            assert_eq!(
                map.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>(),
                (0..n).map(|i| (i, i * 3)).collect::<Vec<_>>()
            );
            assert_eq!(
                map.range(n / 2..).map(|(&k, _)| k).collect::<Vec<_>>(),
                (n / 2..n).collect::<Vec<_>>()
            );
            assert_eq!(
                map.into_iter().collect::<Vec<_>>(),
                (0..n).map(|i| (i, i * 3)).collect::<Vec<_>>()
            );
        }
        let budget = Budget::new(10000);
        let mut words = Map::new();
        words.try_insert(b"key".to_vec(), 7, &budget).unwrap();
        assert_eq!(words.get(b"key".as_slice()), Some(&7));
    }
}
