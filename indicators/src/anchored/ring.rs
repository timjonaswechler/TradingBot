//! Bounded ring buffer used by anchored evaluators to keep the last K events.

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct SegmentState<T> {
    buf: VecDeque<T>,
    cap: usize,
}

impl<T> SegmentState<T> {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "SegmentState capacity must be > 0");
        Self { buf: VecDeque::with_capacity(capacity), cap: capacity }
    }

    pub fn capacity(&self) -> usize { self.cap }
    pub fn len(&self) -> usize { self.buf.len() }
    pub fn is_empty(&self) -> bool { self.buf.is_empty() }
    pub fn is_full(&self) -> bool { self.buf.len() == self.cap }

    /// Push a new event. If full, the oldest is dropped.
    pub fn push(&mut self, item: T) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(item);
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> { self.buf.iter() }
    pub fn last(&self) -> Option<&T> { self.buf.back() }
    pub fn first(&self) -> Option<&T> { self.buf.front() }

    pub fn clear(&mut self) { self.buf.clear(); }

    /// Snapshot the current contents as a Vec (oldest first).
    pub fn snapshot(&self) -> Vec<T> where T: Clone {
        self.buf.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_below_capacity_keeps_all() {
        let mut s = SegmentState::new(3);
        s.push(1); s.push(2);
        assert_eq!(s.snapshot(), vec![1, 2]);
    }

    #[test]
    fn push_over_capacity_drops_oldest() {
        let mut s = SegmentState::new(3);
        for i in 1..=5 { s.push(i); }
        assert_eq!(s.snapshot(), vec![3, 4, 5]);
        assert!(s.is_full());
    }

    #[test]
    fn last_first() {
        let mut s = SegmentState::new(3);
        s.push(10); s.push(20); s.push(30);
        assert_eq!(s.first(), Some(&10));
        assert_eq!(s.last(), Some(&30));
    }

    #[test]
    #[should_panic]
    fn zero_capacity_panics() { let _: SegmentState<i32> = SegmentState::new(0); }
}
