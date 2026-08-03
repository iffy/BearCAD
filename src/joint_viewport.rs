//! Joint icons in the 3D view (#899): each joint draws its kind's icon at its mating
//! frame and is selectable there — exactly how sketch constraints surface as pickable
//! badges ([`crate::constraint_viewport`]). Clicking one selects the joint; hovering
//! highlights the parts it joins.

use crate::icons::{paint_icon, IconId};
use crate::model::Document;
use egui::{Color32, Pos2, Rect};
use glam::Vec3;

pub const JOINT_ICON_SCREEN_SIZE: f32 = 22.0;
pub const JOINT_ICON_HIT_PADDING: f32 = 4.0;

/// One joint's badge: where it sits in the world and which icon it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct JointIconPlacement {
    pub joint: crate::model::JointKey,
    pub world: Vec3,
    pub icon: IconId,
}

/// A badge's clickable screen rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointIconHit {
    pub joint: crate::model::JointKey,
    pub rect: Rect,
}

/// Every live joint's badge (#899), anchored at its posed mating frame — or, for a
/// frameless joint, at the centre of the first driven part.
pub fn build_joint_icon_placements(doc: &Document) -> Vec<JointIconPlacement> {
    let mut out = Vec::new();
    for (ji, joint) in doc.joints.iter() {
        if joint.members.len() < 2 {
            continue;
        }
        let world = crate::joints::posed_joint_frame(doc, ji)
            .map(|(origin, _, _, _)| origin)
            .or_else(|| {
                let driven = joint.driven_members().next()?;
                let bodies = crate::joints::member_bodies(doc, driven);
                let mut bounds: Option<(Vec3, Vec3)> = None;
                for bi in bodies {
                    if let Some((min, max)) =
                        crate::extrude::body_solid_mesh(doc, bi).and_then(|m| m.bounds())
                    {
                        bounds = Some(match bounds {
                            Some((lo, hi)) => (lo.min(min), hi.max(max)),
                            None => (min, max),
                        });
                    }
                }
                bounds.map(|(lo, hi)| (lo + hi) * 0.5)
            });
        if let Some(world) = world {
            out.push(JointIconPlacement {
                joint: ji,
                world,
                icon: crate::icons::icon_for_joint_kind(&joint.kind),
            });
        }
    }
    out
}

/// Screen-space hitboxes for the badges, in draw order.
pub fn build_joint_icon_hits(
    project: &impl Fn(Vec3) -> Option<Pos2>,
    placements: &[JointIconPlacement],
) -> Vec<JointIconHit> {
    placements
        .iter()
        .filter_map(|p| {
            let screen = project(p.world)?;
            let half = JOINT_ICON_SCREEN_SIZE * 0.5 + JOINT_ICON_HIT_PADDING;
            Some(JointIconHit {
                joint: p.joint,
                rect: Rect::from_center_size(screen, egui::vec2(half * 2.0, half * 2.0)),
            })
        })
        .collect()
}

/// The badge under the pointer, nearest centre first so overlapping badges don't flicker.
pub fn pointer_over_joint_icon(
    hits: &[JointIconHit],
    pointer: Pos2,
) -> Option<crate::model::JointKey> {
    hits.iter()
        .filter(|h| h.rect.contains(pointer))
        .min_by(|a, b| {
            let da = a.rect.center().distance(pointer);
            let db = b.rect.center().distance(pointer);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|h| h.joint)
}

/// Draw the badges: a rounded highlight behind a selected or hovered one, the health
/// tint on a broken one, the kind's icon on all.
#[allow(clippy::too_many_arguments)]
pub fn draw_joint_icons(
    painter: &egui::Painter,
    ctx: &egui::Context,
    project: &impl Fn(Vec3) -> Option<Pos2>,
    health: &crate::document_health::DocumentHealth,
    selection: &crate::selection::SceneSelection,
    placements: &[JointIconPlacement],
    hovered: Option<crate::model::JointKey>,
    base_color: Color32,
    selected_color: Color32,
) {
    for placement in placements {
        let Some(screen) = project(placement.world) else { continue };
        let rect = Rect::from_center_size(
            screen,
            egui::vec2(JOINT_ICON_SCREEN_SIZE, JOINT_ICON_SCREEN_SIZE),
        );
        let selected = selection
            .iter()
            .any(|e| e == crate::hierarchy::SceneElement::Joint(placement.joint));
        let is_hovered = hovered == Some(placement.joint);
        if selected || is_hovered {
            painter.rect_filled(
                rect.expand(3.0),
                4.0,
                selected_color.gamma_multiply(if selected { 0.45 } else { 0.25 }),
            );
        }
        let status = health
            .elements
            .get(&crate::hierarchy::SceneElement::Joint(placement.joint))
            .copied()
            .unwrap_or_default();
        let tint = match status {
            crate::document_health::HealthStatus::Invalid => Color32::from_rgb(220, 80, 80),
            crate::document_health::HealthStatus::Unstable => Color32::from_rgb(255, 180, 60),
            crate::document_health::HealthStatus::Healthy => {
                if selected || is_hovered {
                    selected_color
                } else {
                    base_color
                }
            }
        };
        paint_icon(painter, ctx, placement.icon, rect, tint);
    }
}

#[cfg(test)]
mod tests {
    use crate::model::body_key_for_slot as bkey;
    use crate::model::joint_key_for_slot as jkey;
    use super::*;
    use crate::model::{Body, BodySource, ImportedMesh, Joint, JointKind, JointRef};

    fn doc_with_joint(kind: JointKind) -> Document {
        let mut doc = Document::default();
        for i in 0..2 {
            let origin = Vec3::new(i as f32 * 10.0, 0.0, 0.0);
            let mesh = doc.imported_meshes.insert(ImportedMesh {
                triangles: vec![
                    [origin, origin + Vec3::X, origin + Vec3::Y],
                    [origin, origin + Vec3::X, origin + Vec3::Z],
                    [origin, origin + Vec3::Y, origin + Vec3::Z],
                    [origin + Vec3::X, origin + Vec3::Y, origin + Vec3::Z],
                ],
                source_name: format!("p{i}"),
                step_bytes: None,
            });
            doc.bodies.insert(Body {
                source: BodySource::Imported(mesh),
                name: None,
                material: None,
                shadow: false,
            });
        }
        doc.joints.insert(Joint {
            members: vec![JointRef::Body(bkey(0)), JointRef::Body(bkey(1))],
            base: 0,
            kind,
            placement: Default::default(),
            position: String::new(),
            position2: String::new(),
            position3: String::new(),
            rest: String::new(),
            rest2: String::new(),
            rest3: String::new(),
            limits: Default::default(),
            name: None,
            frame: Default::default(),
        });
        doc
    }

    /// #899: a frameless joint's badge anchors at the driven part's centre, carries the
    /// kind's icon, and its hitbox takes the pointer.
    #[test]
    fn badge_anchors_hits_and_picks() {
        let doc = doc_with_joint(JointKind::Revolute);
        let placements = build_joint_icon_placements(&doc);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].icon, IconId::JointRevolute);
        // The driven tetra spans 10..11 in x: its centre is the anchor.
        assert!((placements[0].world.x - 10.5).abs() < 0.1, "at {}", placements[0].world);
        let project = |w: Vec3| Some(Pos2::new(w.x * 10.0, w.y * 10.0));
        let hits = build_joint_icon_hits(&project, &placements);
        assert_eq!(hits.len(), 1);
        let centre = hits[0].rect.center();
        assert_eq!(pointer_over_joint_icon(&hits, centre), Some(jkey(0)));
        assert_eq!(
            pointer_over_joint_icon(&hits, centre + egui::vec2(100.0, 0.0)),
            None
        );
    }

    /// #899: every kind maps to its own icon.
    #[test]
    fn each_kind_has_its_own_icon() {
        let kinds = [
            JointKind::Rigid,
            JointKind::Slider,
            JointKind::Revolute,
            JointKind::Cylindrical,
            JointKind::Planar,
            JointKind::Ball,
            JointKind::PinSlot,
            JointKind::Screw { lead: String::new() },
        ];
        let icons: std::collections::HashSet<_> = kinds
            .iter()
            .map(|k| crate::icons::icon_for_joint_kind(k) as u32)
            .collect();
        assert_eq!(icons.len(), kinds.len(), "no two kinds share an icon");
    }
}
