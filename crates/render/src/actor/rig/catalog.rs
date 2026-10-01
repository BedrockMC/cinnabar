//! Every registered rig geometry's vertices, laid out as append-only segments so registering a
//! skin model or item mesh copies and uploads only that geometry, not the whole catalog.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use super::{
    ActorRigGeometry, ActorRigGeometryError, ActorRigGeometrySpan, ActorRigVertex, EntityRigId,
    MAX_ACTOR_RIG_VERTICES,
};

/// Segments beyond which a registration compacts the catalog back into one.
const MAX_SEGMENTS: usize = 32;

/// Distinct for every full layout, so a renderer never appends to another layout's buffer.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Rig vertices as segments: within one `epoch` existing segments never change and new ones are
/// only appended, so a mirror uploads just the segments it has not seen.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorRigVertexSegments {
    pub epoch: u64,
    pub segments: Arc<[Arc<[ActorRigVertex]>]>,
    len: usize,
}

impl Default for ActorRigVertexSegments {
    fn default() -> Self {
        Self {
            epoch: 0,
            segments: Arc::from([]),
            len: 0,
        }
    }
}

impl ActorRigVertexSegments {
    #[must_use]
    pub fn from_vertices(vertices: impl Into<Arc<[ActorRigVertex]>>) -> Self {
        let vertices = vertices.into();
        Self {
            epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
            len: vertices.len(),
            segments: Arc::from([vertices]),
        }
    }

    /// Vertices across every segment.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The vertices of `span`, which never crosses a segment boundary.
    #[must_use]
    pub fn span(&self, span: ActorRigGeometrySpan) -> Option<&[ActorRigVertex]> {
        let mut start = span.first_vertex as usize;
        for segment in self.segments.iter() {
            if start < segment.len() {
                return segment.get(start..start + span.vertex_count as usize);
            }
            start -= segment.len();
        }
        None
    }
}

#[derive(Debug)]
pub(super) struct GeometryCatalog {
    pub(super) geometries: BTreeMap<EntityRigId, ActorRigGeometry>,
    /// Span index of each geometry; stable while the geometry stays registered.
    pub(super) indices: BTreeMap<EntityRigId, u32>,
    spans: Vec<ActorRigGeometrySpan>,
    pub(super) published_spans: Arc<[ActorRigGeometrySpan]>,
    pub(super) vertices: ActorRigVertexSegments,
    /// Vertices of replaced geometries still occupying the segments.
    dead: usize,
    pub(super) revision: u64,
}

impl GeometryCatalog {
    /// Lays out every geometry in one segment.
    pub(super) fn layout(
        geometries: BTreeMap<EntityRigId, ActorRigGeometry>,
    ) -> Result<Self, ActorRigGeometryError> {
        let mut indices = BTreeMap::new();
        let mut vertices = Vec::new();
        let mut spans = Vec::with_capacity(geometries.len());
        for (id, geometry) in &geometries {
            let first_vertex = u32::try_from(vertices.len())
                .map_err(|_| ActorRigGeometryError::CatalogCapacity)?;
            if vertices.len() + geometry.vertices.len() > MAX_ACTOR_RIG_VERTICES {
                return Err(ActorRigGeometryError::CatalogCapacity);
            }
            indices.insert(*id, spans.len() as u32);
            vertices.extend_from_slice(&geometry.vertices);
            spans.push(ActorRigGeometrySpan {
                first_vertex,
                vertex_count: geometry.vertices.len() as u32,
            });
        }
        let revision = content_revision(&vertices, &spans);
        Ok(Self {
            geometries,
            indices,
            published_spans: Arc::from(spans.as_slice()),
            spans,
            vertices: ActorRigVertexSegments::from_vertices(vertices),
            dead: 0,
            revision,
        })
    }

    /// Adds or replaces `added` in one new segment; a replaced geometry's old vertices stay as
    /// dead space until a compaction. On error the catalog is unchanged.
    pub(super) fn append(
        &mut self,
        added: Vec<ActorRigGeometry>,
        revision: u64,
    ) -> Result<(), ActorRigGeometryError> {
        let added_vertices: usize = added.iter().map(|geometry| geometry.vertices.len()).sum();
        let live = self.vertices.len() - self.dead;
        if self.vertices.len() + added_vertices > MAX_ACTOR_RIG_VERTICES
            || self.vertices.segments.len() >= MAX_SEGMENTS
            || self.dead > live
        {
            let mut geometries = self.geometries.clone();
            geometries.extend(added.into_iter().map(|geometry| (geometry.id, geometry)));
            *self = Self {
                revision,
                ..Self::layout(geometries)?
            };
            return Ok(());
        }
        let mut segment = Vec::with_capacity(added_vertices);
        for geometry in added {
            let span = ActorRigGeometrySpan {
                first_vertex: (self.vertices.len() + segment.len()) as u32,
                vertex_count: geometry.vertices.len() as u32,
            };
            segment.extend_from_slice(&geometry.vertices);
            match self.indices.get(&geometry.id) {
                Some(&index) => {
                    self.dead += self.spans[index as usize].vertex_count as usize;
                    self.spans[index as usize] = span;
                }
                None => {
                    self.indices.insert(geometry.id, self.spans.len() as u32);
                    self.spans.push(span);
                }
            }
            self.geometries.insert(geometry.id, geometry);
        }
        let segments: Vec<_> = self
            .vertices
            .segments
            .iter()
            .cloned()
            .chain([Arc::from(segment)])
            .collect();
        self.vertices = ActorRigVertexSegments {
            epoch: self.vertices.epoch,
            segments: segments.into(),
            len: self.vertices.len() + added_vertices,
        };
        self.published_spans = Arc::from(self.spans.as_slice());
        self.revision = revision;
        Ok(())
    }
}

fn content_revision(vertices: &[ActorRigVertex], spans: &[ActorRigGeometrySpan]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytemuck::cast_slice::<ActorRigVertex, u8>(vertices)
        .iter()
        .chain(bytemuck::cast_slice::<ActorRigGeometrySpan, u8>(spans))
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cuboid(id: u32) -> ActorRigGeometry {
        ActorRigGeometry::synthetic_cuboid(EntityRigId(id), [0.0; 3], [1.0; 3], 1).unwrap()
    }

    /// A registration appends one segment to the same epoch and leaves earlier segments shared.
    #[test]
    fn registering_appends_a_segment_without_copying_the_catalog() {
        let mut catalog =
            GeometryCatalog::layout([(EntityRigId(1), cuboid(1))].into_iter().collect()).unwrap();
        let before = catalog.vertices.clone();
        catalog.append(vec![cuboid(2), cuboid(1)], 7).unwrap();
        assert_eq!(catalog.vertices.epoch, before.epoch);
        assert_eq!(catalog.vertices.segments.len(), 2);
        assert!(Arc::ptr_eq(
            &catalog.vertices.segments[0],
            &before.segments[0]
        ));
        // The replaced geometry now draws from the new segment.
        let span = catalog.published_spans[catalog.indices[&EntityRigId(1)] as usize];
        assert_eq!(span.first_vertex, 72);
        assert_eq!(catalog.vertices.span(span), Some(&cuboid(1).vertices[..]));
    }

    /// Too many segments or too much dead space compacts into a fresh single-segment epoch.
    #[test]
    fn churn_compacts_into_a_new_epoch() {
        let mut catalog =
            GeometryCatalog::layout([(EntityRigId(1), cuboid(1))].into_iter().collect()).unwrap();
        let epoch = catalog.vertices.epoch;
        for _ in 0..3 {
            catalog.append(vec![cuboid(1)], 9).unwrap();
        }
        assert_ne!(catalog.vertices.epoch, epoch);
        assert_eq!((catalog.vertices.segments.len(), catalog.dead), (1, 0));
        assert_eq!(catalog.revision, 9);
    }
}
