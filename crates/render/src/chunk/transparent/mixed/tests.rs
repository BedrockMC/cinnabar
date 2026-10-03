use super::plan::{MixedFace, MixedStream};
use super::*;

fn face(stream: MixedStream, index: u32, z: f32) -> MixedFace {
    MixedFace {
        stream,
        index,
        centroid: Vec3::new(0.5, 0.5, z),
        stable: [index, 0],
    }
}

#[test]
fn models_and_water_share_native_order_and_compress_contiguous_runs() {
    let faces = vec![
        face(MixedStream::Model, 0, 4.0),
        face(MixedStream::Model, 1, 3.0),
        face(MixedStream::Water, 10, 2.0),
        face(MixedStream::Model, 2, 1.0),
        face(MixedStream::Water, 11, 0.0),
    ];
    let segments = merge_faces(SubChunkKey::new(0, 0, 0, 0), Vec3::ZERO, faces, 4).unwrap();
    assert_eq!(
        segments,
        [
            MixedTerrainSegment {
                stream: MixedStream::Model,
                range: 0..2
            },
            MixedTerrainSegment {
                stream: MixedStream::Water,
                range: 10..11
            },
            MixedTerrainSegment {
                stream: MixedStream::Model,
                range: 2..3
            },
            MixedTerrainSegment {
                stream: MixedStream::Water,
                range: 11..12
            },
        ]
    );
}

#[test]
fn old_uploaded_subsequence_order_uses_actual_indices_not_current_rank() {
    let faces = vec![
        face(MixedStream::Model, 0, 1.0),
        face(MixedStream::Model, 1, 3.0),
        face(MixedStream::Water, 5, 2.0),
    ];
    let segments = merge_faces(SubChunkKey::new(0, 0, 0, 0), Vec3::ZERO, faces, 3).unwrap();
    assert_eq!(
        segments
            .iter()
            .map(|segment| (segment.stream, segment.range.clone()))
            .collect::<Vec<_>>(),
        [
            (MixedStream::Model, 1..2),
            (MixedStream::Water, 5..6),
            (MixedStream::Model, 0..1)
        ]
    );
}

#[test]
fn segment_guard_rejects_pathological_fragmentation_before_submission() {
    let faces = vec![
        face(MixedStream::Model, 0, 3.0),
        face(MixedStream::Water, 0, 2.0),
        face(MixedStream::Model, 1, 1.0),
    ];
    assert!(merge_faces(SubChunkKey::new(0, 0, 0, 0), Vec3::ZERO, faces, 2).is_none());
}
