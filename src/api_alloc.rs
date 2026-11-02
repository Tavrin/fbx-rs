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
        let original = std::mem::take(v);
        let bytes = original
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::LimitExceeded(crate::Limit::Allocation))?;
        let release = crate::allocation::Release::new(bytes);
        let items = self.collect(original.into_iter().enumerate())?;
        drop(release);
        let mut items = self.scratch(items)?;
        items
            .values
            .sort_unstable_by(|a, b| cmp(&a.1, &b.1).then(a.0.cmp(&b.0)));
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
        // During loading the shared ledger already owns all earlier stages'
        // buffers. The local helper only checks this reservation's old + new
        // capacity. Standalone evaluation/baking retains its stage counter.
        let old = v
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::ResourceLimit("public allocation"))?;
        let shared = crate::allocation::load_active();
        let mut used = if shared {
            old
        } else {
            self.used
                .get()
                .checked_add(old)
                .ok_or(Error::ResourceLimit("public allocation"))?
        };
        crate::allocation::reserve(v, n, &mut used, self.limit).map_err(|e| match e {
            crate::allocation::Error::Failed => Error::Allocation,
            crate::allocation::Error::Limit if crate::allocation::load_active() => {
                Error::LimitExceeded(crate::Limit::Allocation)
            }
            crate::allocation::Error::Limit => Error::ResourceLimit("public allocation"),
        })?;
        if !shared {
            self.used.set(used);
        }
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

// Scratch ownership refunds both the shared load envelope and this allocator's
// conservative stage counter after the buffer has actually been dropped.
pub(crate) struct Scratch<'a, T> {
    values: Vec<T>,
    budget: &'a Budget,
    bytes: usize,
    release: crate::allocation::Release,
}
impl<T> std::ops::Deref for Scratch<'_, T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
impl<T> Drop for Scratch<'_, T> {
    fn drop(&mut self) {
        if !crate::allocation::load_active() {
            if let Some(left) = self.budget.used.get().checked_sub(self.bytes) {
                self.budget.used.set(left);
            }
        }
    }
}
impl Budget {
    pub(crate) fn scratch<T>(&self, values: Vec<T>) -> Result<Scratch<'_, T>, Error> {
        let bytes = values
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .ok_or(Error::LimitExceeded(crate::Limit::Allocation))?;
        Ok(Scratch {
            values,
            budget: self,
            bytes,
            release: crate::allocation::Release::new(bytes),
        })
    }
    pub(crate) fn scratch_collect<T>(
        &self,
        iter: impl IntoIterator<Item = T>,
    ) -> Result<Scratch<'_, T>, Error> {
        self.scratch(self.collect(iter)?)
    }
}
pub(crate) struct ScratchIter<'a, T> {
    values: std::vec::IntoIter<T>,
    budget: &'a Budget,
    bytes: usize,
    _release: crate::allocation::Release,
}
impl<T> Iterator for ScratchIter<'_, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.values.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}
impl<T> ExactSizeIterator for ScratchIter<'_, T> {}
impl<T> Drop for ScratchIter<'_, T> {
    fn drop(&mut self) {
        if !crate::allocation::load_active() {
            if let Some(left) = self.budget.used.get().checked_sub(self.bytes) {
                self.budget.used.set(left);
            }
        }
    }
}
impl<'a, T> IntoIterator for Scratch<'a, T> {
    type Item = T;
    type IntoIter = ScratchIter<'a, T>;
    fn into_iter(mut self) -> Self::IntoIter {
        let bytes = self.bytes;
        self.bytes = 0;
        ScratchIter {
            values: std::mem::take(&mut self.values).into_iter(),
            budget: self.budget,
            bytes,
            _release: std::mem::take(&mut self.release),
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[test]
    fn load_envelope_spans_stages_and_refunds_scratch() {
        let _scope = crate::allocation::LoadScope::new(40);
        let first_stage = Budget::new(1000);
        let scratch = first_stage.scratch_collect([0u8; 24]).unwrap();
        let second_stage = Budget::new(1000);
        assert!(matches!(
            second_stage.collect([0u8; 24]),
            Err(Error::LimitExceeded(crate::Limit::Allocation))
        ));
        drop(scratch);
        assert!(second_stage.collect([0u8; 24]).is_ok());
    }
    #[test]
    fn failed_reservation_refunds_shared_charge() {
        let _scope = crate::allocation::LoadScope::new(16);
        let budget = Budget::new(16);
        FAIL_AFTER.with(|f| f.set(Some(0)));
        let result = budget.collect([0u8; 16]);
        FAIL_AFTER.with(|f| f.set(None));
        assert!(matches!(result, Err(Error::Allocation)));
        assert!(budget.collect([0u8; 16]).is_ok());
    }
    #[test]
    fn stable_sort_refunds_replaced_and_temporary_buffers() {
        let _scope = crate::allocation::LoadScope::new(300);
        let budget = Budget::new(4096);
        let mut values = budget.collect([0u8; 16]).unwrap();
        budget.stable_sort(&mut values, u8::cmp).unwrap();
        assert!(budget.collect([0u8; 280]).is_ok());
    }
    #[test]
    fn temporary_iterator_retains_capacity_until_drop() {
        let _scope = crate::allocation::LoadScope::new(20);
        let budget = Budget::new(1000);
        let mut iter = crate::allocation::Temporary::new(budget.collect([0u8; 16]).unwrap())
            .unwrap()
            .into_iter();
        assert_eq!(iter.next(), Some(0));
        assert!(budget.collect([0u8; 8]).is_err());
        drop(iter);
        assert!(budget.collect([0u8; 16]).is_ok());
    }
    #[test]
    fn consuming_scratch_holds_charge_until_iterator_drops() {
        let _scope = crate::allocation::LoadScope::new(20);
        let budget = Budget::new(1000);
        let mut iter = budget.scratch_collect([0u8; 16]).unwrap().into_iter();
        assert_eq!(iter.next(), Some(0));
        assert!(budget.collect([0u8; 8]).is_err());
        drop(iter);
        assert!(budget.collect([0u8; 16]).is_ok());
    }
}
