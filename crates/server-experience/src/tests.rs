use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::collections::BTreeSet;

/// Signs canonical fixtures using an isolated deterministic test key.
fn signed<T: serde::Serialize>(value: &T, domain: &[u8], key: &Ed25519KeyPair) -> crypto::SignedDocument {
    let payload = serde_json::to_vec(value).unwrap();
    let mut message = domain.to_vec();
    message.extend_from_slice(&payload);
    crypto::SignedDocument { payload: crypto::hex(&payload), signature: crypto::hex(key.sign(&message).as_ref()) }
}

/// Builds a minimal valid advertisement without any network or local assets.
fn offer(key: &Ed25519KeyPair) -> manifest::Offer {
    manifest::Offer {
        version: policy::WIRE_VERSION,
        audience: "example.org:19132".into(),
        server_key: crypto::hex(key.public_key().as_ref()),
        revision: 3,
        expires_unix: 2000,
        scope: manifest::Scope {
            permissions: BTreeSet::new(),
            origins: BTreeSet::from(["https://example.org".into()]),
            memory_bytes: 0,
            gpu_bytes: 0,
        },
        packages: vec![manifest::PackageOffer {
            id: "example:cinema".into(), publisher_key: crypto::hex(key.public_key().as_ref()),
            digest: crypto::digest(b"bundle"), bytes: 6, url: "https://example.org/bundle".into(),
        }],
        fallback: "Use the normal lobby and poster".into(),
        carrier: protocol::EXPERIENCE_CHANNEL.into(),
    }
}

#[test]
fn signed_offer_checks_audience_expiry_key_and_canonical_bytes() {
    let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let value = offer(&key);
    let marker = negotiation::Marker { server_key: value.server_key.clone(), offer: signed(&value, crypto::OFFER_DOMAIN, &key) };
    let bytes = serde_json::to_vec(&marker).unwrap();
    let verified = negotiation::VerifiedOffer::read(&bytes, &value.audience, 1000).unwrap();
    assert_eq!(verified.offer, value);
    assert!(negotiation::VerifiedOffer::read(&bytes, "elsewhere:19132", 1000).is_err());
    assert!(negotiation::VerifiedOffer::read(&bytes, &value.audience, value.expires_unix).is_err());
    assert!(marker.offer.verify::<manifest::Offer>(&crypto::hex(&[0; 32]), crypto::OFFER_DOMAIN, policy::MAX_MARKER_BYTES).is_err());
    assert!(marker.offer.verify::<manifest::Offer>(&value.server_key, crypto::ACCEPT_DOMAIN, policy::MAX_MARKER_BYTES).is_err());
}

#[test]
fn accept_is_bound_to_the_fresh_connection_and_exact_offer() {
    let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let value = offer(&key);
    let verified = negotiation::VerifiedOffer { digest: crypto::digest(&serde_json::to_vec(&value).unwrap()), offer: value };
    let pending = negotiation::Pending::approve(verified.clone(), 0, 0).unwrap();
    let accept = negotiation::Accept {
        hello: pending.hello().clone(), server_challenge: crypto::hex(&[2; 32]), session: crypto::hex(&[3; 32]),
        audience: verified.offer.audience.clone(), offer_digest: verified.digest.clone(), revision: verified.offer.revision, expires_unix: 1500,
    };
    let document = signed(&accept, crypto::ACCEPT_DOMAIN, &key);
    assert!(pending.accept(&document, 1000, 1).is_ok());
    let second = negotiation::Pending::approve(verified, 0, 0).unwrap();
    assert!(second.accept(&document, 1000, 1).is_err());
}

#[test]
fn ssrf_and_rate_limits_are_conservative() {
    for address in ["127.0.0.1", "10.0.0.1", "169.254.169.254", "100.64.0.1", "::1", "::ffff:8.8.8.8", "2002:0808:0808::1"] {
        assert!(!fetch::public_address(address.parse().unwrap()), "{address}");
    }
    let mut rate = wire::RateLimit::new(1000);
    for _ in 0..policy::MAX_MESSAGES_PER_SECOND { rate.charge(1, 1000).unwrap(); }
    assert!(rate.charge(1, 999).is_err());
    rate.charge(1, 2000).unwrap();
    assert!(rate.charge(policy::MAX_BYTES_PER_SECOND as usize, 2000).is_err());
}

#[test]
fn cache_revalidates_corruption_and_never_uses_a_url_as_a_path() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache::BundleCache::open(root.path()).unwrap();
    let hash = crypto::digest(b"bundle");
    cache.publish(&hash, b"bundle").unwrap();
    assert_eq!(cache.read(&hash).unwrap().unwrap(), b"bundle");
    std::fs::write(root.path().join(format!("{hash}.cxb")), b"modified").unwrap();
    assert!(cache.read(&hash).unwrap().is_none());
    assert!(cache.read("../../token").is_err());
    assert!(cache.publish(&hash, b"modified").is_err());
}
