use sha2::{Digest, Sha256};

/// Pricing: 2 quota units per page.
pub const QUOTA_PER_PAGE: u64 = 2;
pub const MAX_PDF_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_PAGES: usize = 20;

/// Very small PDF page counter – counts "/Type /Page" not "/Type /Pages".
/// Good enough for the technical MVP; a real PDF parser can replace it later.
pub fn count_pdf_pages(pdf: &[u8]) -> usize {
    let needle = b"/Type /Page";
    if pdf.is_empty() {
        return 0;
    }
    let mut count = 0usize;
    let mut i = 0;
    while i + needle.len() <= pdf.len() {
        if &pdf[i..i + needle.len()] == needle {
            // Avoid counting "/Type /Pages"
            let after = i + needle.len();
            if after >= pdf.len() || pdf[after] != b's' {
                count += 1;
            }
            i += needle.len();
        } else {
            i += 1;
        }
    }
    // Fallback: many minimal PDFs have exactly one page and may omit the marker.
    // If we saw a PDF header but counted 0, treat it as 1 page for the MVP.
    if count == 0 && pdf.starts_with(b"%PDF") {
        return 1;
    }
    count
}

pub fn evidence_id_for(summary: &str, pages: usize, job_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(summary.as_bytes());
    h.update(pages.to_le_bytes());
    h.update(job_id.as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pages() {
        let pdf = b"%PDF-1.4\n1 0 obj << /Type /Pages >>\n2 0 obj << /Type /Page >>\n3 0 obj << /Type /Page >>";
        assert_eq!(count_pdf_pages(pdf), 2);
    }

    #[test]
    fn fallback_single_page() {
        assert_eq!(count_pdf_pages(b"%PDF-1.4 minimal"), 1);
    }
}
