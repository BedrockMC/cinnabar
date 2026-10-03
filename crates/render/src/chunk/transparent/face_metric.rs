//! Perspective face ordering shared by every ordinary terrain-blend stream.
use crate::chunk::*;

const NEAR_CAMERA_BLOCK_RADIUS: i32 = 4;
const CHUNK_SIDE: i32 = chunk_origin(SubChunkKey::new(0, 1, 0, 0))[0];

#[derive(Clone, Copy)]
pub(in crate::chunk) struct TransparentFaceMetric {
    camera: Vec3,
    camera_chunk: [i32; 3],
    near_min: [i32; 3],
    near_max: [i32; 3],
}

#[derive(Clone, Copy)]
pub(in crate::chunk) struct TransparentChunkFaceMetric {
    camera: Vec3,
    direction: Option<Vec3>,
}

impl TransparentChunkFaceMetric {
    pub(in crate::chunk) fn distance(self, centroid: Vec3) -> f32 {
        let delta = centroid - self.camera;
        self.direction
            .map_or_else(|| delta.length_squared(), |direction| delta.dot(direction))
    }
}

impl TransparentFaceMetric {
    pub(in crate::chunk) fn new(camera: Vec3) -> Self {
        let block = camera.to_array().map(|value| value.floor() as i32);
        Self {
            camera,
            camera_chunk: block.map(|value| value.div_euclid(CHUNK_SIDE)),
            near_min: block.map(|value| {
                value
                    .saturating_sub(NEAR_CAMERA_BLOCK_RADIUS)
                    .div_euclid(CHUNK_SIDE)
            }),
            near_max: block.map(|value| {
                value
                    .saturating_add(NEAR_CAMERA_BLOCK_RADIUS)
                    .div_euclid(CHUNK_SIDE)
            }),
        }
    }

    pub(in crate::chunk) fn distance(self, key: SubChunkKey, centroid: Vec3) -> f32 {
        self.for_chunk(key).distance(centroid)
    }

    pub(in crate::chunk) fn for_chunk(self, key: SubChunkKey) -> TransparentChunkFaceMetric {
        let chunk = [key.x, key.y, key.z];
        if (0..3).all(|axis| (self.near_min[axis]..=self.near_max[axis]).contains(&chunk[axis])) {
            return TransparentChunkFaceMetric {
                camera: self.camera,
                direction: None,
            };
        }
        let direction = Vec3::from_array(std::array::from_fn(|axis| {
            match chunk[axis].cmp(&self.camera_chunk[axis]) {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
            }
        }));
        TransparentChunkFaceMetric {
            camera: self.camera,
            direction: Some(direction.normalize_or_zero()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_faces_use_radial_distance_not_view_depth() {
        let metric = TransparentFaceMetric::new(Vec3::new(1.0, 1.0, 1.0));
        let key = SubChunkKey::new(0, 0, 0, 0);
        assert_eq!(metric.distance(key, Vec3::new(2.0, 3.0, 4.0)), 14.0);
        assert!(metric.distance(key, Vec3::new(5.0, 1.0, 2.0)) > 14.0);
    }

    #[test]
    fn near_interval_crosses_chunk_boundaries_including_negative_coordinates() {
        let metric = TransparentFaceMetric::new(Vec3::new(-0.1, 15.9, 15.9));
        let key = SubChunkKey::new(0, -1, 1, 1);
        let centroid = Vec3::new(-1.0, 17.0, 17.0);
        assert_eq!(
            metric.distance(key, centroid),
            (centroid - metric.camera).length_squared()
        );
    }

    #[test]
    fn distant_faces_project_on_chunk_grid_direction() {
        let metric = TransparentFaceMetric::new(Vec3::ZERO);
        let key = SubChunkKey::new(0, -2, 0, 3);
        let centroid = Vec3::new(-20.0, 500.0, 30.0);
        let expected = centroid.dot(Vec3::new(-1.0, 0.0, 1.0).normalize());
        assert!((metric.distance(key, centroid) - expected).abs() < 0.00001);
    }
}
