//! Fallible storage and ownership conversion for loading, evaluation and baking.
use crate::Error;
use std::cell::Cell;

pub(crate) struct Budget {
    used: Cell<usize>,
    limit: usize,
}
impl Budget {
    pub fn new(limit: usize) -> Self {
        Self {
            used: Cell::new(0),
            limit,
        }
    }
    pub fn format(&self, args: std::fmt::Arguments<'_>) -> Result<String, Error> {
        struct Writer<'a> {
            budget: &'a Budget,
            bytes: Vec<u8>,
            error: Option<Error>,
        }
        impl std::fmt::Write for Writer<'_> {
            fn write_str(&mut self, s: &str) -> std::fmt::Result {
                if let Err(e) = self.budget.reserve(&mut self.bytes, s.len()) {
                    self.error = Some(e);
                    return Err(std::fmt::Error);
                }
                self.bytes.extend_from_slice(s.as_bytes());
                Ok(())
            }
        }
        let mut w = Writer {
            budget: self,
            bytes: Vec::new(),
            error: None,
        };
        if std::fmt::write(&mut w, args).is_err() {
            return Err(w.error.unwrap_or(Error::ResourceLimit("format")));
        }
        Ok(String::from_utf8(w.bytes).expect("formatted UTF-8"))
    }
    pub fn stable_sort<T>(
        &self,
        v: &mut Vec<T>,
        mut cmp: impl FnMut(&T, &T) -> std::cmp::Ordering,
    ) -> Result<(), Error> {
        let mut items = self.collect(std::mem::take(v).into_iter().enumerate())?;
        items.sort_unstable_by(|a, b| cmp(&a.1, &b.1).then(a.0.cmp(&b.0)));
        *v = self.collect(items.into_iter().map(|(_, v)| v))?;
        Ok(())
    }
    pub fn reserve<T>(&self, v: &mut Vec<T>, n: usize) -> Result<(), Error> {
        if v.len()
            .checked_add(n)
            .ok_or(Error::ResourceLimit("public allocation"))?
            <= v.capacity()
        {
            return Ok(());
        }
        // A vector may have been handed over by an earlier conversion stage.
        // Conservatively charge its old capacity again before growth. The
        // shared helper can then refund that charge without assuming this
        // Budget originally allocated the vector. Geometric growth keeps the
        // cumulative charge linear in the resulting capacity.
        let old = v
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::ResourceLimit("public allocation"))?;
        let mut used = self
            .used
            .get()
            .checked_add(old)
            .ok_or(Error::ResourceLimit("public allocation"))?;
        crate::allocation::reserve(v, n, &mut used, self.limit).map_err(|e| match e {
            crate::allocation::Error::Failed => Error::Allocation,
            crate::allocation::Error::Limit => Error::ResourceLimit("public allocation"),
        })?;
        self.used.set(used);
        Ok(())
    }
    pub fn push<T>(&self, v: &mut Vec<T>, value: T) -> Result<(), Error> {
        if v.len() == v.capacity() {
            self.reserve(v, v.len().max(4))?;
        }
        v.push(value);
        Ok(())
    }
    pub fn collect<T>(&self, iter: impl IntoIterator<Item = T>) -> Result<Vec<T>, Error> {
        self.try_collect(iter.into_iter().map(Ok))
    }
    pub fn try_collect<T>(
        &self,
        iter: impl IntoIterator<Item = Result<T, Error>>,
    ) -> Result<Vec<T>, Error> {
        let iter = iter.into_iter();
        let mut out = Vec::new();
        self.reserve(&mut out, iter.size_hint().0)?;
        for v in iter {
            self.push(&mut out, v?)?;
        }
        Ok(out)
    }
    pub fn filled<T: Clone>(&self, n: usize, value: T) -> Result<Vec<T>, Error> {
        let mut out = Vec::new();
        self.reserve(&mut out, n)?;
        out.resize(n, value);
        Ok(out)
    }
    pub fn bytes(&self, b: &[u8]) -> Result<Vec<u8>, Error> {
        self.collect(b.iter().copied())
    }
    pub fn text(&self, b: &[u8]) -> Result<String, Error> {
        // Precompute the exact replacement length so many malformed bytes
        // cannot cause repeated reallocations of an ever-growing prefix.
        let length = b.utf8_chunks().try_fold(0usize, |length, chunk| {
            length
                .checked_add(chunk.valid().len())
                .and_then(|n| n.checked_add(if chunk.invalid().is_empty() { 0 } else { 3 }))
                .ok_or(Error::ResourceLimit("text length"))
        })?;
        let mut out = Vec::new();
        self.reserve(&mut out, length)?;
        for chunk in b.utf8_chunks() {
            out.extend_from_slice(chunk.valid().as_bytes());
            if !chunk.invalid().is_empty() {
                out.extend_from_slice(b"\xef\xbf\xbd");
            }
        }
        Ok(String::from_utf8(out).expect("valid UTF-8 prefixes and replacement characters"))
    }
    pub fn owned_text(&self, b: Vec<u8>) -> Result<String, Error> {
        match String::from_utf8(b) {
            Ok(s) => Ok(s),
            Err(e) => self.text(e.as_bytes()),
        }
    }
}
// Thread-local reservation seam: fails before the Nth reservation and never
// affects the test harness allocator or another concurrently running test.
#[cfg(test)]
thread_local! { pub(crate) static FAIL_AFTER: Cell<Option<usize>> = const { Cell::new(None) }; }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lossy_text_uses_one_checked_reservation() {
        for bytes in [
            b"valid\0".as_slice(),
            b"\xff\xfe",
            b"\xe2\x82",
            b"\xf0\x80\x80\x80",
            b"a\xed\xa0\x80z",
        ] {
            let budget = Budget::new(1000);
            assert_eq!(budget.text(bytes).unwrap(), String::from_utf8_lossy(bytes));
        }
        let input = vec![0xff; 100_000];
        FAIL_AFTER.with(|f| f.set(Some(1)));
        let result = Budget::new(300_000).text(&input);
        FAIL_AFTER.with(|f| f.set(None));
        assert_eq!(result.unwrap(), String::from_utf8_lossy(&input));
    }
}
