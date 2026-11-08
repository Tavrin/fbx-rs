//! Element connections as compressed sparse rows.
//!
//! One table holds both directions: for each element, the connections it is
//! the source of (`connections_src`) and the destination of
//! (`connections_dst`), each in file order. A row is an element index plus an
//! interned destination-property name, so the per-element lists and the
//! per-connection name copies of a naive representation are not needed.
use crate::api_alloc::Budget;
use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    /// The element at the other end of the connection.
    other: u32,
    /// Interned destination-property name; 0 is the empty name.
    prop: u32,
}

/// Connections of every element in both directions, in file order.
#[derive(Debug, Clone, Default)]
pub struct Connections {
    /// `src_rows[src_start[e]..src_start[e + 1]]`: connections from `e`.
    src_start: Vec<u32>,
    src_rows: Vec<Row>,
    /// `dst_rows[dst_start[e]..dst_start[e + 1]]`: connections into `e`.
    dst_start: Vec<u32>,
    dst_rows: Vec<Row>,
    /// Name `k >= 1` is `names[name_end[k - 2]..name_end[k - 1]]` (from 0 for k = 1).
    name_end: Vec<u32>,
    names: Vec<u8>,
}

fn too_many() -> Error {
    Error::LimitExceeded(crate::Limit::Allocation)
}

impl Connections {
    /// Builds the table for `elements` elements. `edges` yields every
    /// connection as `(source, destination, destination property)` in file
    /// order; it is called twice (count, then fill).
    pub(crate) fn build<'p, I>(
        elements: usize,
        edges: impl Fn() -> I,
        a: &Budget,
    ) -> Result<Self, Error>
    where
        I: Iterator<Item = (usize, usize, &'p [u8])>,
    {
        let slots = elements.checked_add(1).ok_or_else(too_many)?;
        u32::try_from(elements).map_err(|_| too_many())?;
        let mut src_start = a.filled(slots, 0u32)?;
        let mut dst_start = a.filled(slots, 0u32)?;
        let mut total = 0u32;
        let mut ids = crate::fallible_map::Map::<&'p [u8], u32>::new();
        let mut name_end = Vec::new();
        let mut names = Vec::new();
        for (src, dst, prop) in edges() {
            let bump = |c: &mut u32| -> Result<(), Error> {
                *c = c.checked_add(1).ok_or_else(too_many)?;
                Ok(())
            };
            bump(src_start.get_mut(src + 1).ok_or_else(too_many)?)?;
            bump(dst_start.get_mut(dst + 1).ok_or_else(too_many)?)?;
            bump(&mut total)?;
            if !prop.is_empty() && !ids.contains_key(prop) {
                a.reserve(&mut names, prop.len())?;
                names.extend_from_slice(prop);
                a.push(
                    &mut name_end,
                    u32::try_from(names.len()).map_err(|_| too_many())?,
                )?;
                let id = u32::try_from(name_end.len()).map_err(|_| too_many())?;
                ids.try_insert(prop, id, a)?;
            }
        }
        for start in [&mut src_start, &mut dst_start] {
            for i in 1..start.len() {
                start[i] += start[i - 1];
            }
        }
        let empty = Row { other: 0, prop: 0 };
        let mut src_rows = a.filled(total as usize, empty)?;
        let mut dst_rows = a.filled(total as usize, empty)?;
        // Fill cursors start at each element's first row.
        let mut src_next = a.collect(src_start.iter().copied())?;
        let mut dst_next = a.collect(dst_start.iter().copied())?;
        for (src, dst, prop) in edges() {
            let prop = if prop.is_empty() {
                0
            } else {
                *ids.get(prop).ok_or(Error::InvalidData("connection name"))?
            };
            let row = |other: usize| -> Result<Row, Error> {
                Ok(Row {
                    other: u32::try_from(other).map_err(|_| too_many())?,
                    prop,
                })
            };
            let s = &mut src_next[src];
            src_rows[*s as usize] = row(dst)?;
            *s += 1;
            let d = &mut dst_next[dst];
            dst_rows[*d as usize] = row(src)?;
            *d += 1;
        }
        Ok(Self {
            src_start,
            src_rows,
            dst_start,
            dst_rows,
            name_end,
            names,
        })
    }

    fn list<'a>(&'a self, start: &'a [u32], rows: &'a [Row], e: usize) -> List<'a> {
        let range = match (start.get(e), e.checked_add(1).and_then(|n| start.get(n))) {
            (Some(&a), Some(&b)) => a as usize..b as usize,
            _ => 0..0,
        };
        List {
            rows: rows.get(range).unwrap_or(&[]),
            table: self,
        }
    }
    /// `connections_src` of element `e`: (destination, destination property).
    pub(crate) fn src(&self, e: usize) -> List<'_> {
        self.list(&self.src_start, &self.src_rows, e)
    }
    /// `connections_dst` of element `e`: (source, destination property).
    pub(crate) fn dst(&self, e: usize) -> List<'_> {
        self.list(&self.dst_start, &self.dst_rows, e)
    }
    fn name(&self, prop: u32) -> &[u8] {
        let k = prop as usize;
        if k == 0 {
            return b"";
        }
        let end = self.name_end.get(k - 1).map_or(0, |&e| e as usize);
        let begin = if k >= 2 {
            self.name_end.get(k - 2).map_or(0, |&e| e as usize)
        } else {
            0
        };
        self.names.get(begin..end).unwrap_or(b"")
    }
    /// Every connection as `(source, destination, destination property)`,
    /// grouped by source in element order, file order within a source.
    #[cfg(test)]
    pub(crate) fn edges(&self) -> impl Iterator<Item = (usize, usize, &[u8])> + '_ {
        (0..self.src_start.len().saturating_sub(1))
            .flat_map(move |e| self.src(e).into_iter().map(move |(d, p)| (e, d, p)))
    }
}

/// One element's connections in one direction, in file order.
#[derive(Clone, Copy)]
pub(crate) struct List<'a> {
    rows: &'a [Row],
    table: &'a Connections,
}
impl<'a> List<'a> {
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    /// The `i`th connection: (other element, destination property).
    pub(crate) fn get(&self, i: usize) -> Option<(usize, &'a [u8])> {
        let table = self.table;
        self.rows
            .get(i)
            .map(|r| (r.other as usize, table.name(r.prop)))
    }
    pub(crate) fn iter(&self) -> Iter<'a> {
        Iter {
            rows: self.rows.iter(),
            table: self.table,
        }
    }
}
impl<'a> IntoIterator for List<'a> {
    type Item = (usize, &'a [u8]);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// Iterator over a [`List`].
#[derive(Clone)]
pub(crate) struct Iter<'a> {
    rows: std::slice::Iter<'a, Row>,
    table: &'a Connections,
}
impl<'a> Iterator for Iter<'a> {
    type Item = (usize, &'a [u8]);
    fn next(&mut self) -> Option<Self::Item> {
        let table = self.table;
        self.rows
            .next()
            .map(|r| (r.other as usize, table.name(r.prop)))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
    }
}
impl ExactSizeIterator for Iter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_file_order_per_direction_and_intern_names() {
        let edges: [(usize, usize, &[u8]); 5] = [
            (1, 0, b""),
            (2, 1, b"Lcl Translation"),
            (1, 2, b"DeformPercent"),
            (2, 1, b""),
            (3, 1, b"Lcl Translation"),
        ];
        let a = Budget::new(usize::MAX);
        let c = Connections::build(4, || edges.iter().copied(), &a).unwrap();
        let src = |e| c.src(e).into_iter().collect::<Vec<_>>();
        let dst = |e| c.dst(e).into_iter().collect::<Vec<_>>();
        assert_eq!(src(1), [(0, &b""[..]), (2, b"DeformPercent")]);
        assert_eq!(src(2), [(1, &b"Lcl Translation"[..]), (1, b"")]);
        assert_eq!(src(0), []);
        assert_eq!(
            dst(1),
            [
                (2, &b"Lcl Translation"[..]),
                (2, b""),
                (3, b"Lcl Translation")
            ]
        );
        assert_eq!(c.dst(2).get(0), Some((1, &b"DeformPercent"[..])));
        assert!(c.src(9).is_empty());
        assert_eq!(c.names, b"Lcl TranslationDeformPercent");
        assert_eq!(c.edges().count(), 5);
    }
}
