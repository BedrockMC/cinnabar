//! The purchase flow as the player sees it: an affordability check, an optional confirmation, a
//! progress modal, then a result. Only a confirmed purchase is ever handed to the core, and a second
//! press while one is running is ignored.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use json_ui::ModalForm;
use protocol::store_control::{
    ConfirmedPurchase, PendingPurchase, PurchaseOutcome, PurchaseStatus, StoreBalance, StoreOffer,
    StorePrice,
};
use sha2::{Digest, Sha256};

use super::worker::StoreError;

/// A modal or toast the flow wants shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PurchaseDialog {
    /// "You just bought: %s".
    Success { title: String },
    /// The balance cannot cover the price; offers the coin top-up.
    InsufficientFunds { missing: i64 },
    /// The service refused the shown price.
    PriceMismatch,
    /// No definitive answer yet; balance and entitlements must be re-read before trying again.
    Pending,
    /// The service refused the purchase; codes help support find the attempt.
    Failed {
        marketplace_error_code: Option<u32>,
        correlation_id: Option<String>,
    },
    /// The account is no longer signed in.
    SignedOut,
    /// Purchases are switched off until the owner verifies them; nothing was sent.
    Disabled,
}

/// Shown when a confirmed purchase is stopped by the `store_purchases_enabled` setting; not a
/// vanilla string.
pub(crate) const DISABLED_TITLE: &str = "Purchases disabled";
pub(crate) const DISABLED_BODY: &str = "Purchases disabled until verified";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PurchaseFlow {
    Idle,
    /// Waiting for the player to confirm (bundles ask first); the texts are already localized.
    Confirming {
        pending: PendingPurchase,
        title: String,
        body: String,
        offer_title: String,
    },
    InProgress {
        purchase_id: String,
        offer_title: String,
    },
    Done(PurchaseDialog),
}

/// What pressing the purchase button did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Begin {
    /// Ignored: a purchase is already active or the price is not payable.
    Ignored,
    /// A confirmation or result modal is now showing.
    Showing,
    /// Send this to the core.
    Send(ConfirmedPurchase),
}

/// Whether `balances` cover `amount` of `currency`; `None` when that balance is unknown.
pub(crate) fn balance_covers(
    balances: &[StoreBalance],
    currency: &str,
    amount: i64,
) -> Option<bool> {
    balances
        .iter()
        .find(|balance| balance.currency == currency)
        .map(|balance| balance.amount >= amount)
}

impl PurchaseFlow {
    pub(crate) fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// The player pressed the purchase button for `price`. `confirm` carries the bundle confirmation
    /// texts (`title`, `body`) when vanilla asks first; otherwise the press itself is the confirmation.
    pub(crate) fn begin(
        &mut self,
        offer: &StoreOffer,
        price: &StorePrice,
        balances: &[StoreBalance],
        confirm: Option<(String, String)>,
        purchase_id: String,
        purchases_enabled: bool,
    ) -> Begin {
        if !self.is_idle() {
            return Begin::Ignored;
        }
        let Some(pending) = PendingPurchase::for_offer(offer, price) else {
            return Begin::Ignored;
        };
        match balance_covers(balances, &price.currency, price.amount) {
            Some(true) => {}
            Some(false) => {
                let have = balances
                    .iter()
                    .find(|balance| balance.currency == price.currency)
                    .map_or(0, |balance| balance.amount);
                *self = Self::Done(PurchaseDialog::InsufficientFunds {
                    missing: price.amount - have,
                });
                return Begin::Showing;
            }
            None => {
                *self = Self::Done(PurchaseDialog::Failed {
                    marketplace_error_code: None,
                    correlation_id: None,
                });
                return Begin::Showing;
            }
        }
        if let Some((title, body)) = confirm {
            *self = Self::Confirming {
                pending,
                title,
                body,
                offer_title: offer.title.clone(),
            };
            return Begin::Showing;
        }
        if !purchases_enabled {
            *self = Self::Done(PurchaseDialog::Disabled);
            return Begin::Showing;
        }
        Begin::Send(self.start(pending, offer.title.clone(), purchase_id))
    }

    /// The player confirmed the modal; returns the purchase to send, or `None` when nothing is
    /// awaiting confirmation or purchases are disabled.
    pub(crate) fn confirm(
        &mut self,
        purchase_id: String,
        purchases_enabled: bool,
    ) -> Option<ConfirmedPurchase> {
        let Self::Confirming {
            pending,
            offer_title,
            ..
        } = std::mem::replace(self, Self::Idle)
        else {
            return None;
        };
        if !purchases_enabled {
            *self = Self::Done(PurchaseDialog::Disabled);
            return None;
        }
        Some(self.start(pending, offer_title, purchase_id))
    }

    fn start(
        &mut self,
        pending: PendingPurchase,
        offer_title: String,
        purchase_id: String,
    ) -> ConfirmedPurchase {
        let confirmed = pending.confirm(purchase_id.clone());
        *self = Self::InProgress {
            purchase_id,
            offer_title,
        };
        confirmed
    }

    /// The player backed out of a confirmation or dismissed a result.
    pub(crate) fn dismiss(&mut self) {
        if matches!(self, Self::Confirming { .. } | Self::Done(_)) {
            *self = Self::Idle;
        }
    }

    /// The core answered `purchase_id`; answers for any other attempt are dropped.
    pub(crate) fn finish(
        &mut self,
        purchase_id: &str,
        result: Result<PurchaseOutcome, StoreError>,
    ) {
        let Self::InProgress {
            purchase_id: active,
            offer_title,
        } = self
        else {
            return;
        };
        if active.as_str() != purchase_id {
            return;
        }
        let dialog = match result {
            Ok(outcome) => match outcome.status {
                PurchaseStatus::Purchased => PurchaseDialog::Success {
                    title: std::mem::take(offer_title),
                },
                PurchaseStatus::PriceMismatch => PurchaseDialog::PriceMismatch,
                PurchaseStatus::Unknown => PurchaseDialog::Pending,
                PurchaseStatus::PreconditionFailed | PurchaseStatus::Failed => {
                    PurchaseDialog::Failed {
                        marketplace_error_code: Some(outcome.marketplace_error_code)
                            .filter(|code| *code != 0),
                        correlation_id: Some(outcome.correlation_id).filter(|id| !id.is_empty()),
                    }
                }
            },
            Err(StoreError::SignedOut) => PurchaseDialog::SignedOut,
            Err(StoreError::Busy) => PurchaseDialog::Pending,
            Err(_) => PurchaseDialog::Failed {
                marketplace_error_code: None,
                correlation_id: None,
            },
        };
        *self = Self::Done(dialog);
    }

    /// The modal for the current state; `tr` maps a vanilla lang key to text. Success shows as a toast.
    pub(crate) fn modal(&self, tr: &dyn Fn(&str) -> String) -> Option<ModalForm> {
        match self {
            Self::Idle => None,
            Self::Confirming { title, body, .. } => Some(ModalForm {
                title: title.clone(),
                body: body.clone(),
                button1: tr("store.purchase.bundle.confirm"),
                button2: tr("gui.cancel"),
            }),
            Self::InProgress { .. } => Some(ModalForm {
                title: tr("store.popup.purchaseInProgress.title"),
                body: tr("store.popup.purchaseInProgress.msg"),
                ..ModalForm::default()
            }),
            Self::Done(dialog) => dialog.modal(tr),
        }
    }
}

impl PurchaseDialog {
    fn modal(&self, tr: &dyn Fn(&str) -> String) -> Option<ModalForm> {
        let close = tr("gui.close");
        Some(match self {
            Self::Success { .. } => return None,
            Self::InsufficientFunds { .. } => ModalForm {
                title: tr("store.popup.purchaseFailedInsufficientFunds.title"),
                body: tr("store.popup.purchaseFailedInsufficientFunds.msg"),
                button1: tr("store.popup.purchaseFailedInsufficientFunds.buyButton"),
                button2: close,
            },
            Self::PriceMismatch => ModalForm {
                title: tr("store.popup.purchaseFailed.title"),
                body: tr("store.popup.purchasePriceMismatch.msg"),
                button1: close,
                ..ModalForm::default()
            },
            Self::Pending => ModalForm {
                title: tr("store.popup.purchasePending.title"),
                body: tr("store.popup.purchasePending.msg"),
                button1: close,
                ..ModalForm::default()
            },
            Self::Failed {
                marketplace_error_code,
                correlation_id,
            } => {
                let mut body = tr("store.popup.purchaseFailed.msg");
                if let Some(code) = marketplace_error_code {
                    body.push_str(&format!(
                        "\n{}",
                        tr("store.csb.purchaseErrorDialog.errorCode")
                            .replace("%s", &code.to_string())
                    ));
                }
                if let Some(id) = correlation_id {
                    body.push_str(&format!(
                        "\n{}",
                        tr("store.csb.purchaseErrorDialog.correlationId").replace("%s", id)
                    ));
                }
                ModalForm {
                    title: tr("store.popup.purchaseFailed.title"),
                    body,
                    button1: close,
                    ..ModalForm::default()
                }
            }
            Self::Disabled => ModalForm {
                title: DISABLED_TITLE.to_owned(),
                body: DISABLED_BODY.to_owned(),
                button1: close,
                ..ModalForm::default()
            },
            Self::SignedOut => ModalForm {
                title: tr("store.popup.xblRequired.title"),
                body: tr("store.popup.xblRequired.message"),
                button1: tr("store.popup.xblRequired.button1"),
                button2: tr("store.popup.xblRequired.button2"),
            },
        })
    }

    /// The success toast (`store.purchase.success`).
    pub(crate) fn toast(&self, tr: &dyn Fn(&str) -> String) -> Option<String> {
        match self {
            Self::Success { title } => Some(tr("store.purchase.success").replace("%s", title)),
            _ => None,
        }
    }
}

static PURCHASE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh idempotency key: 32 hex characters, unique per call within and across runs.
pub(crate) fn new_purchase_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let count = PURCHASE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(count.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut out, byte| {
            out.push_str(&format!("{byte:02x}"));
            out
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> StoreOffer {
        StoreOffer {
            id: "o1".into(),
            title: "Castle".into(),
            creator: None,
            content_type: None,
            thumbnail_url: None,
            store_id: Some("s1".into()),
            prices: vec![],
            rating: None,
            tags: vec![],
            owned: false,
        }
    }

    fn price(amount: i64) -> StorePrice {
        StorePrice {
            currency: "mc".into(),
            amount,
        }
    }

    fn balances(amount: i64) -> Vec<StoreBalance> {
        vec![StoreBalance {
            currency: "mc".into(),
            amount,
        }]
    }

    fn outcome(status: PurchaseStatus) -> PurchaseOutcome {
        PurchaseOutcome {
            status,
            http_status: 200,
            marketplace_error_code: 0,
            correlation_id: "c1".into(),
            inventory_version: None,
            replayed: false,
        }
    }

    fn tr(key: &str) -> String {
        format!("<{key}>")
    }

    #[test]
    fn a_covered_price_sends_once_and_ignores_a_second_press() {
        let mut flow = PurchaseFlow::Idle;
        let sent = flow.begin(
            &offer(),
            &price(320),
            &balances(500),
            None,
            "id-1".into(),
            true,
        );
        let Begin::Send(purchase) = sent else {
            panic!("expected a send, got {sent:?}");
        };
        assert_eq!(purchase.purchase_id(), "id-1");
        assert_eq!(
            flow.begin(
                &offer(),
                &price(320),
                &balances(500),
                None,
                "id-2".into(),
                true
            ),
            Begin::Ignored
        );
        assert!(matches!(flow, PurchaseFlow::InProgress { .. }));
    }

    #[test]
    fn an_uncovered_or_unknown_balance_never_sends() {
        let mut flow = PurchaseFlow::Idle;
        assert_eq!(
            flow.begin(
                &offer(),
                &price(320),
                &balances(100),
                None,
                "id".into(),
                true
            ),
            Begin::Showing
        );
        assert_eq!(
            flow,
            PurchaseFlow::Done(PurchaseDialog::InsufficientFunds { missing: 220 })
        );
        flow.dismiss();
        assert_eq!(
            flow.begin(&offer(), &price(320), &[], None, "id".into(), true),
            Begin::Showing
        );
        assert!(matches!(
            flow,
            PurchaseFlow::Done(PurchaseDialog::Failed { .. })
        ));
        flow.dismiss();
        assert_eq!(
            flow.begin(&offer(), &price(0), &balances(9), None, "id".into(), true),
            Begin::Ignored
        );
    }

    #[test]
    fn a_bundle_waits_for_confirmation_and_cancel_sends_nothing() {
        let confirm = Some(("Unlock 2 of 3 Packs?".to_owned(), "You'll get".to_owned()));
        let mut flow = PurchaseFlow::Idle;
        assert_eq!(
            flow.begin(
                &offer(),
                &price(320),
                &balances(500),
                confirm.clone(),
                "id".into(),
                true
            ),
            Begin::Showing
        );
        let modal = flow.modal(&tr).expect("confirmation modal");
        assert_eq!(modal.button1, "<store.purchase.bundle.confirm>");
        flow.dismiss();
        assert!(flow.is_idle() && flow.confirm("x".into(), true).is_none());

        flow.begin(
            &offer(),
            &price(320),
            &balances(500),
            confirm,
            "id".into(),
            true,
        );
        let purchase = flow.confirm("id-2".into(), true).expect("confirmed");
        assert_eq!(purchase.purchase_id(), "id-2");
        assert!(matches!(flow, PurchaseFlow::InProgress { .. }));
    }

    #[test]
    fn outcomes_map_to_the_vanilla_dialogs() {
        let cases = [
            (
                PurchaseStatus::PriceMismatch,
                "<store.popup.purchasePriceMismatch.msg>",
            ),
            (PurchaseStatus::Unknown, "<store.popup.purchasePending.msg>"),
        ];
        for (status, body) in cases {
            let mut flow = PurchaseFlow::Idle;
            flow.begin(&offer(), &price(1), &balances(5), None, "id".into(), true);
            flow.finish("id", Ok(outcome(status)));
            assert_eq!(flow.modal(&tr).expect("modal").body, body);
        }
        let mut flow = PurchaseFlow::Idle;
        flow.begin(&offer(), &price(1), &balances(5), None, "id".into(), true);
        flow.finish("stale", Ok(outcome(PurchaseStatus::Purchased)));
        assert!(
            matches!(flow, PurchaseFlow::InProgress { .. }),
            "another attempt's answer is dropped"
        );
        flow.finish("id", Ok(outcome(PurchaseStatus::Purchased)));
        let PurchaseFlow::Done(dialog) = &flow else {
            panic!("not done: {flow:?}");
        };
        assert_eq!(
            dialog.toast(&tr).as_deref(),
            Some("<store.purchase.success>")
        );
        assert!(flow.modal(&tr).is_none());
    }

    #[test]
    fn failures_carry_codes_and_a_dead_session_asks_to_sign_in() {
        let mut flow = PurchaseFlow::Idle;
        flow.begin(&offer(), &price(1), &balances(5), None, "id".into(), true);
        let mut failed = outcome(PurchaseStatus::Failed);
        failed.marketplace_error_code = 1502;
        flow.finish("id", Ok(failed));
        let body = flow.modal(&tr).expect("modal").body;
        assert!(
            body.contains("<store.csb.purchaseErrorDialog.errorCode>")
                && body.contains("<store.popup.purchaseFailed.msg>")
        );
        flow.dismiss();
        flow.begin(&offer(), &price(1), &balances(5), None, "id2".into(), true);
        flow.finish("id2", Err(StoreError::SignedOut));
        assert_eq!(flow, PurchaseFlow::Done(PurchaseDialog::SignedOut));
    }

    #[test]
    fn disabled_purchases_show_the_notice_and_never_send() {
        let mut flow = PurchaseFlow::Idle;
        let begun = flow.begin(&offer(), &price(5), &balances(10), None, "id".into(), false);
        assert_eq!(begun, Begin::Showing);
        assert_eq!(flow, PurchaseFlow::Done(PurchaseDialog::Disabled));
        assert_eq!(flow.modal(&tr).expect("modal").body, DISABLED_BODY);
        flow.dismiss();

        // A confirmation that is answered after purchases were switched off is stopped too.
        let confirm = Some(("t".to_owned(), "b".to_owned()));
        flow.begin(
            &offer(),
            &price(5),
            &balances(10),
            confirm,
            "id".into(),
            true,
        );
        assert!(flow.confirm("id".into(), false).is_none());
        assert_eq!(flow, PurchaseFlow::Done(PurchaseDialog::Disabled));
    }

    #[test]
    fn purchase_ids_are_unique_hex() {
        let a = new_purchase_id();
        let b = new_purchase_id();
        assert_ne!(a, b);
        assert!(a.len() == 32 && a.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}
