use super::*;
use crate::RuntimeAudioCatalog;

pub(super) fn fixture() -> (AudioPcmExpectedIdentity, RuntimeAudioCatalog, Vec<u8>) {
    let samples = [1_i16, -2, 3, -4];
    let pcm: Vec<_> = samples.into_iter().flat_map(i16::to_le_bytes).collect();
    let definition = crate::AudioDefinition {
        identifier: "test:finite".into(),
        category: Some("ambient".into()),
        subtitle: None,
        min_distance: None,
        max_distance: None,
        volume: None,
        pitch: None,
        use_legacy_max_distance: Some("true".into()),
        alternatives: vec![crate::AudioAlternative {
            object_form: true,
            name: "sounds/test/finite".into(),
            weight: 1,
            volume: None,
            pitch: None,
            is_3d: Some(false),
            stream: Some(true),
            load_on_low_memory: None,
        }]
        .into_boxed_slice(),
    };
    let catalog_bytes = crate::encode_audio_catalog([1; 32], [2; 32], &[definition]).unwrap();
    let catalog = RuntimeAudioCatalog::decode(&catalog_bytes).unwrap();
    let expected = AudioPcmExpectedIdentity::new(
        "test:finite",
        "sounds/test/finite.fsb",
        Sha256::digest(&catalog_bytes).into(),
        [1; 32],
        [2; 32],
        [3; 32],
        Sha256::digest(&pcm).into(),
        140,
        2,
        48000,
        2,
    )
    .unwrap();
    let bytes = encode_audio_pcm(&expected, &samples).unwrap();
    (expected, catalog, bytes)
}

#[test]
fn synthetic_pcm_roundtrip_requires_independent_expected_identity() {
    let (expected, catalog, bytes) = fixture();
    let decoded =
        RuntimeAudioPcm::decode(&bytes, &catalog, expected.catalog_sha256(), &expected).unwrap();
    assert_eq!(decoded.samples(), &[1, -2, 3, -4]);
    assert_eq!(decoded.channels(), 2);
    assert_eq!(decoded.sample_rate(), 48000);
    assert_eq!(decoded.frames(), 2);
    assert_eq!(decoded.identifier(), "test:finite");
    assert_eq!(decoded.mode(), AudioPcmMode::FinitePredecodedNoLoop);
    assert_eq!(
        encode_audio_pcm(&expected, decoded.samples()).unwrap(),
        bytes
    );
    assert!(RuntimeAudioPcm::decode(&bytes, &catalog, [9; 32], &expected).is_err());
}

fn rehash(bytes: &mut [u8]) {
    let split = bytes.len() - 32;
    let hash = Sha256::digest(&bytes[..split]);
    bytes[split..].copy_from_slice(&hash);
}

#[test]
fn recomputed_carrier_hashes_cannot_authorize_another_pcm_or_source() {
    let (expected, catalog, bytes) = fixture();
    for offset in [
        8,
        12,
        14,
        16,
        20,
        24,
        28,
        30,
        32,
        36,
        68,
        100,
        132,
        164,
        HEADER_BYTES,
    ] {
        let mut forged = bytes.clone();
        forged[offset] ^= 1;
        rehash(&mut forged);
        assert!(
            RuntimeAudioPcm::decode(&forged, &catalog, expected.catalog_sha256(), &expected)
                .is_err(),
            "offset {offset}"
        );
    }
    let mut forged = bytes.clone();
    let pcm_start = HEADER_BYTES + expected.identifier().len() + expected.source_path().len();
    forged[pcm_start] ^= 1;
    let hash = Sha256::digest(&forged[pcm_start..forged.len() - 32]);
    forged[164..196].copy_from_slice(&hash);
    rehash(&mut forged);
    assert!(
        RuntimeAudioPcm::decode(&forged, &catalog, expected.catalog_sha256(), &expected).is_err()
    );
}

#[test]
fn all_truncations_trailing_data_and_size_overflows_reject() {
    let (expected, catalog, bytes) = fixture();
    for length in 0..bytes.len() {
        assert!(
            RuntimeAudioPcm::decode(
                &bytes[..length],
                &catalog,
                expected.catalog_sha256(),
                &expected
            )
            .is_err()
        );
    }
    let mut overflow = bytes.clone();
    overflow[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    rehash(&mut overflow);
    assert!(
        RuntimeAudioPcm::decode(&overflow, &catalog, expected.catalog_sha256(), &expected).is_err()
    );
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(
        RuntimeAudioPcm::decode(&extra, &catalog, expected.catalog_sha256(), &expected).is_err()
    );
    assert!(
        RuntimeAudioPcm::decode(
            &vec![0; MAX_AUDIO_PCM_CARRIER_BYTES + 1],
            &catalog,
            expected.catalog_sha256(),
            &expected
        )
        .is_err()
    );
}

#[test]
fn expected_profile_rejects_paths_empty_hashes_and_arithmetic() {
    let (good, _, _) = fixture();
    for path in [
        "../finite.fsb",
        "sounds/../finite.fsb",
        "/sounds/finite.fsb",
        "sounds\\finite.fsb",
        "sounds//finite.fsb",
        "sounds/finite.ogg",
    ] {
        assert!(
            AudioPcmExpectedIdentity::new(
                "test:finite",
                path,
                [1; 32],
                [2; 32],
                [3; 32],
                [4; 32],
                [5; 32],
                140,
                2,
                48000,
                2
            )
            .is_err()
        );
    }
    assert!(
        AudioPcmExpectedIdentity::new(
            "",
            good.source_path(),
            [1; 32],
            [2; 32],
            [3; 32],
            [4; 32],
            [5; 32],
            140,
            2,
            48000,
            2
        )
        .is_err()
    );
    for (source_bytes, channels, rate, frames) in [
        (0, 2, 48000, 2),
        (u32::MAX, 2, 48000, 2),
        (140, 6, 48000, 2),
        (140, 2, 0, 2),
        (140, 2, 48000, 0),
        (140, 2, 48000, u32::MAX),
    ] {
        assert!(
            AudioPcmExpectedIdentity::new(
                "test:finite",
                good.source_path(),
                [1; 32],
                [2; 32],
                [3; 32],
                [4; 32],
                [5; 32],
                source_bytes,
                channels,
                rate,
                frames
            )
            .is_err()
        );
    }
    assert!(
        AudioPcmExpectedIdentity::new(
            "test:finite",
            good.source_path(),
            [0; 32],
            [2; 32],
            [3; 32],
            [4; 32],
            [5; 32],
            140,
            2,
            48000,
            2
        )
        .is_err()
    );
}
