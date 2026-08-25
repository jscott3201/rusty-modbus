//! Unit ID → backend routing table.

use std::net::SocketAddr;

use crate::config::RouteEntry;

/// Route table for resolving unit IDs to backend addresses.
#[derive(Debug, Clone)]
pub struct RouteTable {
    entries: Vec<RouteEntry>,
}

impl RouteTable {
    /// Create a new route table from the given entries.
    ///
    /// # Panics
    ///
    /// Panics if any entries have overlapping unit ID ranges.
    #[must_use]
    pub fn new(entries: Vec<RouteEntry>) -> Self {
        // Validate no overlaps.
        for (i, a) in entries.iter().enumerate() {
            for b in &entries[i + 1..] {
                assert!(
                    !ranges_overlap(&a.unit_id_range, &b.unit_id_range),
                    "overlapping unit ID ranges: {:?} and {:?}",
                    a.unit_id_range,
                    b.unit_id_range,
                );
            }
        }
        Self { entries }
    }

    /// Look up the backend address for a given unit ID.
    #[must_use]
    pub fn resolve(&self, unit_id: u8) -> Option<SocketAddr> {
        self.resolve_route(unit_id).map(|entry| entry.backend_addr)
    }

    pub(crate) fn resolve_route(&self, unit_id: u8) -> Option<&RouteEntry> {
        self.entries
            .iter()
            .find(|entry| entry.unit_id_range.contains(&unit_id))
    }

    /// Return all backend addresses (for broadcast forwarding).
    pub fn all_backends(&self) -> impl Iterator<Item = SocketAddr> + '_ {
        // Deduplicate addresses.
        let mut seen = Vec::new();
        self.entries.iter().filter_map(move |e| {
            if seen.contains(&e.backend_addr) {
                None
            } else {
                seen.push(e.backend_addr);
                Some(e.backend_addr)
            }
        })
    }
}

fn ranges_overlap(a: &std::ops::RangeInclusive<u8>, b: &std::ops::RangeInclusive<u8>) -> bool {
    a.start() <= b.end() && b.start() <= a.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(range: std::ops::RangeInclusive<u8>, port: u16) -> RouteEntry {
        RouteEntry::new(range, format!("127.0.0.1:{port}").parse().unwrap())
    }

    #[test]
    fn resolve_finds_matching_route() {
        let table = RouteTable::new(vec![entry(1..=10, 5001), entry(11..=20, 5002)]);
        assert_eq!(table.resolve(5), Some("127.0.0.1:5001".parse().unwrap()));
        assert_eq!(table.resolve(15), Some("127.0.0.1:5002".parse().unwrap()));
    }

    #[test]
    fn resolve_returns_none_for_unrouted() {
        let table = RouteTable::new(vec![entry(1..=10, 5001)]);
        assert!(table.resolve(11).is_none());
        assert!(table.resolve(0).is_none());
    }

    #[test]
    #[should_panic(expected = "overlapping")]
    fn overlapping_ranges_panic() {
        let _ = RouteTable::new(vec![entry(1..=10, 5001), entry(5..=15, 5002)]);
    }

    #[test]
    fn all_backends_deduplicates() {
        let table = RouteTable::new(vec![entry(1..=10, 5001), entry(11..=20, 5001)]);
        let addrs: Vec<_> = table.all_backends().collect();
        assert_eq!(addrs.len(), 1);
    }
}
