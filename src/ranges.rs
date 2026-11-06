use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    #[serde(rename = "Start", alias = "start", alias = "s")]
    pub start: u64,
    #[serde(rename = "End", alias = "end", alias = "e")]
    pub end: u64,
}
impl Range {
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }
    pub fn len(self) -> u64 {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}
#[derive(Debug, Default, Clone)]
pub struct RangeSet(pub Vec<Range>);
impl RangeSet {
    pub fn add(&mut self, r: Range) {
        if r.is_empty() {
            return;
        }
        self.0.push(r);
        self.0.sort_by_key(|x| x.start);
        let mut out: Vec<Range> = Vec::new();
        for x in &self.0 {
            if let Some(last) = out.last_mut() {
                if x.start <= last.end {
                    last.end = last.end.max(x.end);
                    continue;
                }
            }
            out.push(*x);
        }
        self.0 = out;
    }
    pub fn remove(&mut self, r: Range) {
        let mut out = Self::default();
        for x in &self.0 {
            if x.end <= r.start || x.start >= r.end {
                out.add(*x);
                continue;
            }
            if x.start < r.start {
                out.add(Range::new(x.start, r.start));
            }
            if x.end > r.end {
                out.add(Range::new(r.end, x.end));
            }
        }
        *self = out;
    }
    pub fn contains(&self, r: Range) -> bool {
        r.is_empty() || self.0.iter().any(|x| x.start <= r.start && x.end >= r.end)
    }
    pub fn covered(&self) -> u64 {
        self.0.iter().map(|x| x.len()).sum()
    }
    pub fn missing(&self, total: u64) -> Vec<Range> {
        let mut out = Vec::new();
        let mut pos = 0;
        for r in &self.0 {
            if r.start > pos {
                out.push(Range::new(pos, r.start.min(total)));
            }
            pos = pos.max(r.end);
            if pos >= total {
                break;
            }
        }
        if pos < total {
            out.push(Range::new(pos, total));
        }
        out
    }
}
