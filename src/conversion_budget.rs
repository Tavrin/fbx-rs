//! Accounting shared by bounded graph conversion and its scaling tests.
use std::cell::Cell;
#[derive(Clone, Copy)]
pub(crate) enum Path {
    Deformers,
    Poses,
    Curves,
    Layers,
    Helpers,
}
pub(crate) struct Budget {
    used: Cell<usize>,
    work: Cell<usize>,
    allocation_limit: usize,
    work_limit: usize,
    pub path: Cell<Path>,
}
impl Budget {
    pub fn new(allocation_limit: usize, work_limit: usize, path: Path) -> Self {
        Self {
            used: Cell::new(0),
            work: Cell::new(0),
            allocation_limit,
            work_limit,
            path: Cell::new(path),
        }
    }
    pub fn charge(&self, count: usize, width: usize) -> Result<(), String> {
        let used = count
            .checked_mul(width)
            .and_then(|n| self.used.get().checked_add(n))
            .ok_or("conversion: allocation limit")?;
        if used > self.allocation_limit {
            return Err("conversion: allocation limit".into());
        }
        self.used.set(used);
        Ok(())
    }
    pub fn work(&self, count: usize) -> Result<(), String> {
        let work = self
            .work
            .get()
            .checked_add(count)
            .ok_or("conversion: work limit")?;
        if work > self.work_limit {
            return Err("conversion: work limit".into());
        }
        self.work.set(work);
        record(self.path.get(), count);
        Ok(())
    }
    pub fn sort_work(&self, count: usize) -> Result<(), String> {
        self.work(
            count
                .checked_mul(usize::try_from(usize::BITS - count.leading_zeros()).unwrap() + 1)
                .ok_or("conversion: work limit")?,
        )
    }
}
pub(crate) fn record(path: Path, count: usize) {
    #[cfg(test)]
    COUNTERS.with(|c| c.borrow_mut()[path as usize] += count);
    #[cfg(not(test))]
    let _ = (path, count);
}
#[cfg(test)]
thread_local! { pub(crate) static COUNTERS: std::cell::RefCell<[usize; 5]> = const { std::cell::RefCell::new([0; 5]) }; }
