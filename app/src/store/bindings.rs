//! Data sources for the vanilla store screens (`store_item_list`, `common_store`): the binding names
//! are the ones those layouts read.

use json_ui::{CollectionItem, DataSource, Scalar};
use protocol::store_control::StoreOffer;

use super::worker::StoreError;

/// The `collection_name` the offer grids read.
const OFFER_COLLECTION: &str = "list_collection";
/// The vanilla `texture_file_system` value for a path outside the packs.
const RAW_PATH: &str = "RawPath";

fn text(value: impl Into<String>) -> Scalar {
    Scalar::Text(value.into())
}

/// "You Have %s Minecoins".
pub(crate) fn balance_text(amount: i64, tr: &dyn Fn(&str) -> String) -> String {
    tr("store.coins.currentCoins").replace("%s", &amount.to_string())
}

/// One grid entry per offer. `image_path` maps a thumbnail URL to the decoded file the renderer can
/// load, when there is one; `owned` is the store's ownership view.
pub(crate) fn offer_items(
    offers: &[StoreOffer],
    owned: &dyn Fn(&StoreOffer) -> bool,
    image_path: &dyn Fn(&str) -> Option<String>,
    tr: &dyn Fn(&str) -> String,
) -> Vec<CollectionItem> {
    offers
        .iter()
        .map(|offer| {
            let price = offer.prices.first().filter(|price| price.amount > 0);
            let is_owned = owned(offer);
            let is_free = price.is_none() && !is_owned;
            let path = offer
                .thumbnail_url
                .as_deref()
                .and_then(image_path)
                .unwrap_or_default();
            let prompt = if is_owned {
                tr("store.owned")
            } else if is_free {
                tr("store.free")
            } else {
                String::new()
            };
            let amount = price.map_or_else(String::new, |price| price.amount.to_string());
            let creator = offer.creator.clone().unwrap_or_default();
            let ratings = offer.rating.as_ref().map_or(0, |rating| rating.count);
            CollectionItem::default()
                .with("#title_label", text(offer.title.clone()))
                .with(
                    "#is_creator_label_visible",
                    Scalar::Bool(!creator.is_empty()),
                )
                .with("#creator_label", text(creator))
                .with(
                    "#offer_coin_visible",
                    Scalar::Bool(price.is_some() && !is_owned),
                )
                .with("#offer_minecoin_text", text(amount.clone()))
                .with("#offer_full_price", text(amount))
                .with(
                    "#offer_prompt_text_visibility",
                    Scalar::Bool(!prompt.is_empty()),
                )
                .with("#offer_prompt_text", text(prompt))
                .with("#ratings_visible", Scalar::Bool(ratings > 0))
                .with("#ratings_count_text", text(ratings.to_string()))
                .with("#offer_markdown_visible", Scalar::Bool(false))
                .with("#new_offer_icon_visible", Scalar::Bool(false))
                .with("#valid_offer_index", Scalar::Bool(true))
                .with(
                    "#thumbnail_texture_file_system",
                    text(if path.is_empty() { "" } else { RAW_PATH }),
                )
                .with("#thumbnail_texture_path", text(path))
        })
        .collect()
}

/// A grid of `offers` plus the loading and failure state the list screens bind.
pub(crate) fn offer_list_source(
    items: Vec<CollectionItem>,
    loading: bool,
    failure: Option<StoreError>,
    tr: &dyn Fn(&str) -> String,
) -> DataSource {
    let mut data = DataSource::new();
    data.set_global("#max_grid_offers", Scalar::Num(items.len() as f64));
    data.set_global("#page_loading_visible", Scalar::Bool(loading));
    data.set_global("#store_error_visible", Scalar::Bool(failure.is_some()));
    data.set_global("#store_failure_text", text(failure_text(failure, tr)));
    data.set_collection(OFFER_COLLECTION, items);
    data
}

fn failure_text(failure: Option<StoreError>, tr: &dyn Fn(&str) -> String) -> String {
    match failure {
        None => String::new(),
        Some(StoreError::SignedOut) => tr("store.popup.xblRequired.message"),
        Some(_) => tr("store.connection.failed.body"),
    }
}

#[cfg(test)]
mod tests {
    use protocol::store_control::{StorePrice, StoreRating};

    use super::*;

    fn tr(key: &str) -> String {
        match key {
            "store.owned" => "Owned".into(),
            "store.free" => "Free".into(),
            "store.coins.currentCoins" => "You Have %s Minecoins".into(),
            other => format!("<{other}>"),
        }
    }

    fn offer(title: &str, price: Option<i64>) -> StoreOffer {
        StoreOffer {
            id: title.into(),
            title: title.into(),
            creator: Some("Studio".into()),
            content_type: None,
            thumbnail_url: Some(format!("https://x.test/{title}.png")),
            store_id: None,
            prices: price
                .map(|amount| {
                    vec![StorePrice {
                        currency: "mc".into(),
                        amount,
                    }]
                })
                .unwrap_or_default(),
            rating: Some(StoreRating {
                average: 4.5,
                count: 12,
            }),
            tags: vec![],
            owned: false,
        }
    }

    fn value<'a>(item: &'a CollectionItem, name: &str) -> &'a Scalar {
        item.values
            .get(name)
            .unwrap_or_else(|| panic!("missing {name}"))
    }

    #[test]
    fn priced_owned_and_free_offers_bind_the_vanilla_price_and_prompt_fields() {
        let offers = [
            offer("paid", Some(320)),
            offer("mine", Some(320)),
            offer("gift", None),
        ];
        let items = offer_items(
            &offers,
            &|offer| offer.title == "mine",
            &|url| {
                url.ends_with("paid.png")
                    .then(|| "/cache/paid.png".to_owned())
            },
            &tr,
        );
        assert_eq!(value(&items[0], "#offer_minecoin_text"), &text("320"));
        assert_eq!(value(&items[0], "#offer_coin_visible"), &Scalar::Bool(true));
        assert_eq!(
            value(&items[0], "#thumbnail_texture_path"),
            &text("/cache/paid.png")
        );
        assert_eq!(
            value(&items[0], "#thumbnail_texture_file_system"),
            &text("RawPath")
        );
        assert_eq!(value(&items[1], "#offer_prompt_text"), &text("Owned"));
        assert_eq!(
            value(&items[1], "#offer_coin_visible"),
            &Scalar::Bool(false)
        );
        assert_eq!(value(&items[2], "#offer_prompt_text"), &text("Free"));
        assert_eq!(
            value(&items[2], "#thumbnail_texture_file_system"),
            &text("")
        );
        assert_eq!(value(&items[0], "#ratings_count_text"), &text("12"));
    }

    #[test]
    fn balance_and_failure_texts_use_vanilla_keys() {
        assert_eq!(balance_text(1500, &tr), "You Have 1500 Minecoins");
        assert_eq!(failure_text(None, &tr), "");
        assert_eq!(
            failure_text(Some(StoreError::Unavailable), &tr),
            "<store.connection.failed.body>"
        );
        assert_eq!(
            failure_text(Some(StoreError::SignedOut), &tr),
            "<store.popup.xblRequired.message>"
        );
    }
}
