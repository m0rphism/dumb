//! An abstraction for applying the usual motions on list views (go up, go down, go to first, ...)
//! to the current item index.

#[derive(Debug, Clone, Copy)]
pub enum Motion {
    First,
    Last,
    Up,
    Down,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    Abs(usize),
    Rel(isize),
}

impl Motion {
    pub fn apply(self, cur: usize, len: usize, height: usize) -> usize {
        let last = len.saturating_sub(1);
        match self {
            Motion::First => 0,
            Motion::Last => last,
            Motion::Up => cur.saturating_sub(1),
            Motion::Down => (cur + 1).min(last),
            Motion::PageUp => cur.saturating_sub(height),
            Motion::PageDown => (cur + height).min(last),
            Motion::HalfPageUp => cur.saturating_sub(height / 2),
            Motion::HalfPageDown => (cur + height / 2).min(last),
            Motion::Abs(j) => j.min(last),
            Motion::Rel(j) => (cur.saturating_add_signed(j)).min(last),
        }
    }

    pub fn apply_check(self, cur: &mut usize, len: usize, height: usize) -> bool {
        let new = self.apply(*cur, len, height);
        if *cur == new {
            false
        } else {
            *cur = new;
            true
        }
    }
}
