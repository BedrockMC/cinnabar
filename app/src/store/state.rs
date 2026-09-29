//! What the store screens show: the newest page, search, offer, balances and owned ids, folded from
//! worker events. Every event may ask for follow-up requests; nothing here does I/O.

use std::collections::HashSet;

use bevy::prelude::Resource;
use protocol::store_control::{
    ConfirmedPurchase, PurchaseStatus, StoreBalance, StoreOffer, StoreOfferDetail, StorePage,
    StorePrice, StoreSearchResults,
};

use super::flow::{Begin, PurchaseFlow, new_purchase_id};
use super::images::TextureCache;
use super::worker::{StoreError, StoreEvent, StoreRequest, StoreWorker};

const MAX_IMAGE_REQUESTS_PER_EVENT: usize = 64;
const MAX_FAILED_IMAGES: usize = 512;
const MAX_OWNED: usize = 100_000;
const TEXTURE_ENTRIES: usize = 128;
const TEXTURE_BYTES: usize = 96 << 20;

#[derive(Resource)]
pub(crate) struct StoreState {
    pub(crate) page: Option<StorePage>,
    pub(crate) search: Option<StoreSearchResults>,
    pub(crate) offer: Option<StoreOfferDetail>,
    pub(crate) balances: Vec<StoreBalance>,
    pub(crate) flow: PurchaseFlow,
    /// The newest failure of a read call; cleared by the next success.
    pub(crate) failure: Option<StoreError>,
    pub(crate) textures: TextureCache,
    owned: HashSet<String>,
    page_name: Option<String>,
    pending_images: HashSet<String>,
    failed_images: HashSet<String>,
}

impl Default for StoreState {
    fn default() -> Self {
        Self::new()
    }
}

impl StoreState {
    pub(crate) fn new() -> Self {
        Self {
            page: None,
            search: None,
            offer: None,
            balances: Vec::new(),
            flow: PurchaseFlow::Idle,
            failure: None,
            textures: TextureCache::new(TEXTURE_ENTRIES, TEXTURE_BYTES),
            owned: HashSet::new(),
            page_name: None,
            pending_images: HashSet::new(),
            failed_images: HashSet::new(),
        }
    }

    /// The requests that load a store session from scratch: page, balance and owned content.
    pub(crate) fn open(&mut self, page: Option<String>) -> Vec<StoreRequest> {
        self.page_name = page.clone();
        vec![
            StoreRequest::Home(page),
            StoreRequest::Balance,
            StoreRequest::Entitlements { offset: 0 },
        ]
    }

    /// Whether the core reported `id` as owned (lowercase compare).
    pub(crate) fn is_owned(&self, id: &str) -> bool {
        self.owned.contains(&id.to_ascii_lowercase())
    }

    /// Whether an offer shows as owned: the core's flag or the entitlement list.
    pub(crate) fn offer_owned(&self, offer: &StoreOffer) -> bool {
        offer.owned || self.is_owned(&offer.id)
    }

    /// Drain the worker, fold every event in and queue the follow-ups. A refused follow-up is dropped;
    /// the next [`StoreState::open`] reloads it.
    pub(crate) fn pump(&mut self, worker: &StoreWorker) {
        for event in worker.poll() {
            for request in self.apply(event) {
                let _ = worker.send(request);
            }
        }
    }

    /// The player pressed the purchase button for `price`; `confirm` carries the bundle confirmation texts.
    pub(crate) fn press_purchase(
        &mut self,
        worker: &StoreWorker,
        offer: &StoreOffer,
        price: &StorePrice,
        confirm: Option<(String, String)>,
    ) {
        let id = new_purchase_id();
        if let Begin::Send(purchase) = self.flow.begin(offer, price, &self.balances, confirm, id) {
            self.dispatch_purchase(worker, purchase);
        }
    }

    /// The player confirmed the bundle modal.
    pub(crate) fn confirm_purchase(&mut self, worker: &StoreWorker) {
        if let Some(purchase) = self.flow.confirm(new_purchase_id()) {
            self.dispatch_purchase(worker, purchase);
        }
    }

    fn dispatch_purchase(&mut self, worker: &StoreWorker, purchase: ConfirmedPurchase) {
        let id = purchase.purchase_id().to_owned();
        if !worker.send(StoreRequest::Purchase(purchase)) {
            self.flow.finish(&id, Err(StoreError::Unavailable));
        }
    }

    /// Fold one worker event in; returns the follow-up requests it calls for.
    pub(crate) fn apply(&mut self, event: StoreEvent) -> Vec<StoreRequest> {
        match event {
            StoreEvent::Page(Ok(page)) => {
                self.failure = None;
                let requests = self.image_requests(page.rows.iter().flat_map(|row| &row.offers));
                self.page = Some(page);
                requests
            }
            StoreEvent::Search(Ok(results)) => {
                self.failure = None;
                let requests = self.image_requests(results.offers.iter());
                self.search = Some(results);
                requests
            }
            StoreEvent::Offer(Ok(detail)) => {
                self.failure = None;
                let requests = self.image_requests(std::iter::once(&detail.offer));
                self.offer = Some(detail);
                requests
            }
            StoreEvent::Balance(Ok(balances)) => {
                self.balances = balances;
                Vec::new()
            }
            StoreEvent::Entitlements {
                offset,
                result: Ok(window),
            } => {
                if offset == 0 {
                    self.owned.clear();
                }
                let taken = window.owned.len() as u32;
                for id in window.owned {
                    if self.owned.len() < MAX_OWNED {
                        self.owned.insert(id.to_ascii_lowercase());
                    }
                }
                if taken > 0 && window.offset + taken < window.total {
                    vec![StoreRequest::Entitlements {
                        offset: window.offset + taken,
                    }]
                } else {
                    Vec::new()
                }
            }
            StoreEvent::Purchase {
                purchase_id,
                result,
            } => {
                let refresh = matches!(
                    &result,
                    Ok(outcome) if !matches!(outcome.status, PurchaseStatus::PriceMismatch)
                );
                self.flow.finish(&purchase_id, result);
                if refresh {
                    // Re-read what the purchase may have changed instead of assuming it.
                    self.open(self.page_name.clone())
                } else {
                    vec![StoreRequest::Balance]
                }
            }
            StoreEvent::Image { url, result } => {
                self.pending_images.remove(&url);
                match result {
                    Ok(path) => {
                        if self.textures.load_file(&url, &path).is_err() {
                            self.fail_image(url);
                        }
                    }
                    Err(_) => self.fail_image(url),
                }
                Vec::new()
            }
            StoreEvent::Page(Err(error))
            | StoreEvent::Search(Err(error))
            | StoreEvent::Offer(Err(error))
            | StoreEvent::Balance(Err(error))
            | StoreEvent::Entitlements {
                result: Err(error), ..
            } => {
                self.failure = Some(error);
                Vec::new()
            }
        }
    }

    fn fail_image(&mut self, url: String) {
        if self.failed_images.len() >= MAX_FAILED_IMAGES {
            self.failed_images.clear();
        }
        self.failed_images.insert(url);
    }

    /// Image fetches for offers whose thumbnail is neither cached, in flight nor known bad.
    fn image_requests<'a>(
        &mut self,
        offers: impl Iterator<Item = &'a StoreOffer>,
    ) -> Vec<StoreRequest> {
        let mut requests = Vec::new();
        for offer in offers {
            let Some(url) = offer.thumbnail_url.as_deref() else {
                continue;
            };
            if requests.len() >= MAX_IMAGE_REQUESTS_PER_EVENT
                || self.textures.contains(url)
                || self.pending_images.contains(url)
                || self.failed_images.contains(url)
            {
                continue;
            }
            self.pending_images.insert(url.to_owned());
            requests.push(StoreRequest::Image(url.to_owned()));
        }
        requests
    }
}

#[cfg(test)]
mod tests {
    use protocol::store_control::{PurchaseOutcome, StoreEntitlements, StorePrice, StoreRow};

    use super::*;

    fn offer(id: &str, thumbnail: Option<&str>) -> StoreOffer {
        StoreOffer {
            id: id.into(),
            title: id.into(),
            creator: None,
            content_type: None,
            thumbnail_url: thumbnail.map(str::to_owned),
            store_id: None,
            prices: vec![],
            rating: None,
            tags: vec![],
            owned: false,
        }
    }

    fn page(offers: Vec<StoreOffer>) -> StorePage {
        StorePage {
            id: "store".into(),
            rows: vec![StoreRow {
                id: None,
                title: Some("Featured".into()),
                kind: None,
                offers,
            }],
            inventory_version: None,
            truncated: false,
        }
    }

    #[test]
    fn a_page_asks_for_each_thumbnail_once() {
        let mut state = StoreState::new();
        let shown = page(vec![
            offer("a", Some("https://x.test/a.png")),
            offer("b", Some("https://x.test/a.png")),
            offer("c", None),
        ]);
        let requests = state.apply(StoreEvent::Page(Ok(shown.clone())));
        assert_eq!(
            requests.len(),
            1,
            "duplicate and missing thumbnails are skipped"
        );
        assert!(
            state.apply(StoreEvent::Page(Ok(shown))).is_empty(),
            "in-flight images are not re-requested"
        );
    }

    #[test]
    fn a_failed_image_is_not_retried_every_page() {
        let mut state = StoreState::new();
        let shown = page(vec![offer("a", Some("https://x.test/a.png"))]);
        state.apply(StoreEvent::Page(Ok(shown.clone())));
        state.apply(StoreEvent::Image {
            url: "https://x.test/a.png".into(),
            result: Err(StoreError::Rejected),
        });
        assert!(state.apply(StoreEvent::Page(Ok(shown))).is_empty());
    }

    #[test]
    fn entitlement_windows_chain_until_the_total_is_reached() {
        let mut state = StoreState::new();
        let next = state.apply(StoreEvent::Entitlements {
            offset: 0,
            result: Ok(StoreEntitlements {
                owned: vec!["AA".into(), "bb".into()],
                total: 3,
                offset: 0,
                inventory_version: None,
            }),
        });
        assert!(matches!(
            next.as_slice(),
            [StoreRequest::Entitlements { offset: 2 }]
        ));
        let done = state.apply(StoreEvent::Entitlements {
            offset: 2,
            result: Ok(StoreEntitlements {
                owned: vec!["cc".into()],
                total: 3,
                offset: 2,
                inventory_version: None,
            }),
        });
        assert!(done.is_empty());
        assert!(state.is_owned("aa") && state.is_owned("CC") && !state.is_owned("dd"));
        // A fresh first window replaces the set rather than growing it.
        state.apply(StoreEvent::Entitlements {
            offset: 0,
            result: Ok(StoreEntitlements {
                owned: vec![],
                total: 0,
                offset: 0,
                inventory_version: None,
            }),
        });
        assert!(!state.is_owned("aa"));
    }

    #[test]
    fn a_completed_purchase_rereads_state_and_a_price_refusal_only_the_balance() {
        let outcome = |status| PurchaseOutcome {
            status,
            http_status: 200,
            marketplace_error_code: 0,
            correlation_id: "c".into(),
            inventory_version: None,
            replayed: false,
        };
        let price = StorePrice {
            currency: "mc".into(),
            amount: 5,
        };
        for (status, expected) in [
            (PurchaseStatus::Purchased, 3),
            (PurchaseStatus::PriceMismatch, 1),
        ] {
            let mut state = StoreState::new();
            let begun = state.flow.begin(
                &offer("o", None),
                &price,
                &[StoreBalance {
                    currency: "mc".into(),
                    amount: 10,
                }],
                None,
                "id".into(),
            );
            assert!(matches!(begun, super::super::flow::Begin::Send(_)));
            let follow = state.apply(StoreEvent::Purchase {
                purchase_id: "id".into(),
                result: Ok(outcome(status)),
            });
            assert_eq!(follow.len(), expected);
            assert!(matches!(state.flow, PurchaseFlow::Done(_)));
        }
    }

    #[test]
    fn read_failures_surface_and_clear_on_success() {
        let mut state = StoreState::new();
        state.apply(StoreEvent::Search(Err(StoreError::SignedOut)));
        assert_eq!(state.failure, Some(StoreError::SignedOut));
        state.apply(StoreEvent::Search(Ok(StoreSearchResults {
            offers: vec![],
            continuation: None,
            truncated: false,
        })));
        assert_eq!(state.failure, None);
    }
}
