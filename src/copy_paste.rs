//! Copy / Paste / Paste Linked (#1236).
//!
//! The clipboard holds a snapshot of the selection at copy time so independent paste still
//! works if the originals are deleted. Paste Linked needs live source keys (bodies /
//! components) and fails if those are gone. Interactive paste is a cyan preview constrained
//! to the six world-axis directions from the originals' centroid; commit is click or Enter.

use crate::extrude::SolidMesh;
use crate::hierarchy::SceneElement;
use crate::model::{BodyKey, ComponentKey, ConstructionPlaneKey, Document};
use glam::Vec3;

/// One item on the clipboard.
#[derive(Clone, Debug)]
pub enum ClipboardItem {
    /// A body: mesh snapshot for independent paste; `source` for Paste Linked.
    Body {
        source: BodyKey,
        mesh: SolidMesh,
        name: Option<String>,
        /// World centroid of the mesh at copy time (for multi-item relative layout).
        #[allow(dead_code)]
        centroid: Vec3,
    },
    /// A component: its bodies (and nested components' bodies) as snapshots.
    Component {
        /// Source component at copy time (for linked paste of component membership).
        #[allow(dead_code)]
        source: ComponentKey,
        name: Option<String>,
        bodies: Vec<ClipboardBody>,
    },
    /// A construction plane (independent paste only).
    Plane {
        /// Source plane at copy time (independent paste does not re-link).
        #[allow(dead_code)]
        source: ConstructionPlaneKey,
        /// Snapshot of the plane's frame at copy time.
        origin: Vec3,
        normal: Vec3,
        u_axis: Vec3,
        v_axis: Vec3,
        extent: crate::model::PlaneExtent,
        name: Option<String>,
    },
}

/// One body nested under a component clipboard entry.
#[derive(Clone, Debug)]
pub struct ClipboardBody {
    pub source: BodyKey,
    pub mesh: SolidMesh,
    pub name: Option<String>,
    /// World centroid at copy time (layout / multi-item offset).
    #[allow(dead_code)]
    pub centroid: Vec3,
}

/// Session clipboard (not persisted).
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    pub items: Vec<ClipboardItem>,
    /// World-space origin of the 6-axis paste offset (centroid of all copied geometry).
    pub origin: Vec3,
}

impl Clipboard {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether Paste Linked applies to anything on the clipboard.
    pub fn has_linkable(&self) -> bool {
        self.items.iter().any(|i| {
            matches!(
                i,
                ClipboardItem::Body { .. } | ClipboardItem::Component { .. }
            )
        })
    }
}

/// Interactive paste placement (#1236): cyan preview, 6-axis offset from [`Clipboard::origin`].
#[derive(Clone, Debug, PartialEq)]
pub struct CreatingPaste {
    pub linked: bool,
    pub offset: Vec3,
}

impl CreatingPaste {
    pub fn new(linked: bool) -> Self {
        Self {
            linked,
            offset: Vec3::ZERO,
        }
    }
}

/// Project `cursor - origin` onto the dominant world axis (one of ±X/±Y/±Z).
pub fn six_axis_offset(origin: Vec3, cursor: Vec3) -> Vec3 {
    let d = cursor - origin;
    let ax = d.x.abs();
    let ay = d.y.abs();
    let az = d.z.abs();
    if ax >= ay && ax >= az {
        Vec3::new(d.x, 0.0, 0.0)
    } else if ay >= az {
        Vec3::new(0.0, d.y, 0.0)
    } else {
        Vec3::new(0.0, 0.0, d.z)
    }
}

/// Translate a mesh by `offset`.
pub fn translate_mesh(mesh: &SolidMesh, offset: Vec3) -> SolidMesh {
    if offset == Vec3::ZERO {
        return mesh.clone();
    }
    SolidMesh {
        triangles: mesh
            .triangles
            .iter()
            .map(|tri| [tri[0] + offset, tri[1] + offset, tri[2] + offset])
            .collect(),
    }
}

/// Centroid of a solid mesh, or `None` when empty.
pub fn mesh_centroid(mesh: &SolidMesh) -> Option<Vec3> {
    let mut sum = Vec3::ZERO;
    let mut n = 0usize;
    for tri in &mesh.triangles {
        for p in tri {
            sum += *p;
            n += 1;
        }
    }
    (n > 0).then_some(sum / n as f32)
}

/// Promote a selected element to the thing copy actually pastes (body face → body, etc.).
pub fn copyable_element(element: &SceneElement) -> Option<SceneElement> {
    match element {
        SceneElement::Body(b)
        | SceneElement::BodyEdge { body: b, .. }
        | SceneElement::BodyVertex { body: b, .. }
        | SceneElement::BodyFace { body: b, .. }
        | SceneElement::BodyCylinder { body: b, .. }
        | SceneElement::BodyAxis { body: b, .. } => Some(SceneElement::Body(*b)),
        SceneElement::Component(c) => Some(SceneElement::Component(*c)),
        SceneElement::ConstructionPlane(p) => Some(SceneElement::ConstructionPlane(*p)),
        // Unit instance body is owned by the instance row; copy the body if present.
        SceneElement::UnitInstance(_) => None,
        _ => None,
    }
}

/// Build clipboard items from the current selection. Returns `None` when nothing copyable.
pub fn clipboard_from_selection(
    doc: &Document,
    selection: &crate::selection::SceneSelection,
    component_bodies: impl Fn(ComponentKey) -> Vec<BodyKey>,
) -> Option<Clipboard> {
    let mut items = Vec::new();
    let mut seen_bodies = std::collections::HashSet::new();
    let mut seen_components = std::collections::HashSet::new();
    let mut seen_planes = std::collections::HashSet::new();
    let mut centroids = Vec::new();

    for element in selection.ordered() {
        let Some(el) = copyable_element(&element) else {
            continue;
        };
        match el {
            SceneElement::Body(bi) => {
                if !seen_bodies.insert(bi) {
                    continue;
                }
                let Some(body) = doc.bodies.get(bi) else {
                    continue;
                };
                if body.shadow {
                    continue;
                }
                let Some(mesh) = crate::extrude::body_solid_mesh(doc, bi) else {
                    continue;
                };
                if mesh.is_empty() {
                    continue;
                }
                let centroid = mesh_centroid(&mesh).unwrap_or(Vec3::ZERO);
                centroids.push(centroid);
                items.push(ClipboardItem::Body {
                    source: bi,
                    mesh,
                    name: body.name.clone(),
                    centroid,
                });
            }
            SceneElement::Component(ci) => {
                if !seen_components.insert(ci) {
                    continue;
                }
                let Some(comp) = doc.components.get(ci) else {
                    continue;
                };
                let mut bodies = Vec::new();
                for bi in component_bodies(ci) {
                    if !seen_bodies.insert(bi) {
                        continue;
                    }
                    let Some(body) = doc.bodies.get(bi) else {
                        continue;
                    };
                    let Some(mesh) = crate::extrude::body_solid_mesh(doc, bi) else {
                        continue;
                    };
                    if mesh.is_empty() {
                        continue;
                    }
                    let centroid = mesh_centroid(&mesh).unwrap_or(Vec3::ZERO);
                    centroids.push(centroid);
                    bodies.push(ClipboardBody {
                        source: bi,
                        mesh,
                        name: body.name.clone(),
                        centroid,
                    });
                }
                if bodies.is_empty() {
                    continue;
                }
                items.push(ClipboardItem::Component {
                    source: ci,
                    name: comp.name.clone(),
                    bodies,
                });
            }
            SceneElement::ConstructionPlane(pi) => {
                if !seen_planes.insert(pi) {
                    continue;
                }
                // Datum planes 0..2 are the fixed world triad — don't copy them.
                if pi.index() < 3 {
                    continue;
                }
                let Some(plane) = doc.construction_planes.get(pi) else {
                    continue;
                };
                centroids.push(plane.origin);
                items.push(ClipboardItem::Plane {
                    source: pi,
                    origin: plane.origin,
                    normal: plane.normal,
                    u_axis: plane.u_axis,
                    v_axis: plane.v_axis,
                    extent: plane.extent,
                    name: plane.name.clone(),
                });
            }
            _ => {}
        }
    }

    if items.is_empty() {
        return None;
    }
    let origin = if centroids.is_empty() {
        Vec3::ZERO
    } else {
        centroids.iter().copied().sum::<Vec3>() / centroids.len() as f32
    };
    Some(Clipboard { items, origin })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_axis_picks_dominant_direction() {
        let o = Vec3::ZERO;
        assert_eq!(six_axis_offset(o, Vec3::new(10.0, 1.0, 0.5)), Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(six_axis_offset(o, Vec3::new(1.0, -8.0, 2.0)), Vec3::new(0.0, -8.0, 0.0));
        assert_eq!(six_axis_offset(o, Vec3::new(1.0, 2.0, 9.0)), Vec3::new(0.0, 0.0, 9.0));
    }

    #[test]
    fn translate_mesh_moves_all_vertices() {
        let mesh = SolidMesh {
            triangles: vec![[
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ]],
        };
        let moved = translate_mesh(&mesh, Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(moved.triangles[0][0], Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(moved.triangles[0][1], Vec3::new(6.0, 0.0, 0.0));
    }
}
