//! Pagination math. Page numbers are 1-indexed.

/// Zero-based skip offset for the given 1-indexed page and page size.
pub fn offset(page: u64, page_size: u64) -> u64 {
    page.max(1).saturating_sub(1).saturating_mul(page_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_page_has_no_offset() {
        assert_eq!(offset(1, 100), 0);
        assert_eq!(offset(0, 100), 0);
    }

    #[test]
    fn later_pages() {
        assert_eq!(offset(2, 50), 50);
        assert_eq!(offset(3, 25), 50);
    }
}
