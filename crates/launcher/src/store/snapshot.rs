//! The store's presented state: everything a screen build needs, cloneable across the menu view
//! boundary so the presentation side can bind it with its own translator.

use std::collections::HashMap;

use protocol::store_control::{StoreOffer, StoreOfferDetail};

use super::flow::{PurchaseDialog, PurchaseFlow};
use super::worker::StoreError;

/// Most thumbnails the menu artwork atlas can hold at once.
pub const MAX_VISIBLE_IMAGES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreView {
    Home,
    Search,
    Detail,
    Inventory,
}

/// One titled strip of offers as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayRow {
    pub id: Option<String>,
    pub title: String,
    /// The vanilla row factory role (`StoreRow`, `GridList`, ...).
    pub role: &'static str,
    pub offers: Vec<StoreOffer>,
    /// Token that loads this row's next offers, when it has more.
    pub continuation: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StoreSnapshot {
    pub view: StoreView,
    pub rows: Vec<DisplayRow>,
    pub detail: Option<StoreOfferDetail>,
    pub balance: Option<i64>,
    pub loading: bool,
    pub failure: Option<StoreError>,
    pub flow: PurchaseFlow,
    /// Thumbnail URL to the local file the core cached.
    pub images: HashMap<String, String>,
    pub owned_total: usize,
    pub search_term: String,
}

impl StoreSnapshot {
    /// The state before anything has loaded.
    pub fn empty() -> Self {
        Self {
            view: StoreView::Home,
            rows: Vec::new(),
            detail: None,
            balance: None,
            loading: true,
            failure: None,
            flow: PurchaseFlow::Idle,
            images: HashMap::new(),
            owned_total: 0,
            search_term: String::new(),
        }
    }

    /// Whether an overlay (progress or a modal) is up and takes the input.
    pub fn modal_active(&self) -> bool {
        match &self.flow {
            PurchaseFlow::Idle => false,
            PurchaseFlow::Done(dialog) => !matches!(dialog, PurchaseDialog::Success { .. }),
            PurchaseFlow::Confirming { .. } | PurchaseFlow::InProgress { .. } => true,
        }
    }

    /// Local files of the thumbnails currently on screen, in draw order, bounded by the artwork atlas.
    pub fn image_paths(&self) -> Vec<String> {
        let detail = self.detail.iter().flat_map(|detail| {
            detail
                .offer
                .thumbnail_url
                .iter()
                .chain(detail.screenshot_urls.iter())
        });
        let rows = self
            .rows
            .iter()
            .flat_map(|row| row.offers.iter().filter_map(|o| o.thumbnail_url.as_ref()));
        let mut seen = Vec::new();
        for url in detail.chain(rows) {
            if let Some(path) = self.images.get(url)
                && !seen.contains(path)
            {
                seen.push(path.clone());
                if seen.len() == MAX_VISIBLE_IMAGES {
                    break;
                }
            }
        }
        seen
    }
}

/// The factory role for a layout row `kind`; anything unknown is a plain offer row.
pub fn role_for(kind: Option<&str>) -> &'static str {
    match kind.unwrap_or_default() {
        "GridList" => "GridList",
        "VerticalGridList" => "VerticalGridList",
        "HeroRow" => "HeroRow",
        "CarouselRow" => "CarouselRow",
        _ => "StoreRow",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(id: &str, url: Option<&str>) -> StoreOffer {
        StoreOffer {
            id: id.into(),
            title: id.into(),
            creator: None,
            content_type: None,
            thumbnail_url: url.map(str::to_owned),
            store_id: None,
            prices: vec![],
            rating: None,
            tags: vec![],
            owned: false,
        }
    }

    #[test]
    fn image_paths_follow_draw_order_skip_unfetched_and_dedupe() {
        let snapshot = StoreSnapshot {
            view: StoreView::Home,
            rows: vec![DisplayRow {
                id: None,
                title: String::new(),
                role: "StoreRow",
                offers: vec![
                    offer("a", Some("https://x.test/a")),
                    offer("b", Some("https://x.test/missing")),
                    offer("c", Some("https://x.test/a")),
                    offer("d", Some("https://x.test/d")),
                ],
                continuation: None,
            }],
            detail: None,
            balance: None,
            loading: false,
            failure: None,
            flow: PurchaseFlow::Idle,
            images: [
                ("https://x.test/a".to_owned(), "/c/a.png".to_owned()),
                ("https://x.test/d".to_owned(), "/c/d.png".to_owned()),
            ]
            .into_iter()
            .collect(),
            owned_total: 0,
            search_term: String::new(),
        };
        assert_eq!(snapshot.image_paths(), ["/c/a.png", "/c/d.png"]);
    }

    #[test]
    fn unknown_row_kinds_fall_back_to_the_plain_offer_row() {
        assert_eq!(role_for(Some("GridList")), "GridList");
        assert_eq!(role_for(Some("Whatever")), "StoreRow");
        assert_eq!(role_for(None), "StoreRow");
    }
}
