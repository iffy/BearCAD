//! 3MF export of solid meshes (SPEC §9.2).
//!
//! A 3MF package is a ZIP with OPC content types, a root relationship, and a
//! `3D/3dmodel.model` mesh document. We write store-method (uncompressed) ZIP
//! entries so the encoder needs no deflate crate — slicers accept store fine.
//!
//! Multi-body documents export as separate `<object>` entries sharing a
//! materials-extension `<m:colorgroup>` (one `<m:color>` per distinct body
//! color). Bambu Studio maps each unique color to a filament slot and sets
//! the object's extruder from `pid`/`pindex` (#1294 / #1299). Core
//! `<basematerials>` is also written so generic 3MF viewers show colors.

use crate::extrude::SolidMesh;
use glam::Vec3;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Default body color when no material is known (matches Unobtainium / SOLID_FILL).
pub const DEFAULT_BODY_COLOR: [u8; 3] = [150, 168, 196];

/// Materials extension namespace (3MF Materials Spec 1.1) — required for `m:colorgroup`.
const MATERIALS_NS: &str = "http://schemas.microsoft.com/3dmanufacturing/material/2015/02";

/// One mesh object in a multi-part 3MF package.
#[derive(Clone, Copy, Debug)]
pub struct ThreeMfPart<'a> {
    pub name: &'a str,
    pub mesh: &'a SolidMesh,
    /// sRGB body color.
    pub color: [u8; 3],
    /// Label written into `<basematerials>` (material name).
    pub material_name: &'a str,
}

/// Serialize one or more colored mesh parts as a 3MF package (#1284 / #1294 / #1299).
///
/// Coordinates are millimetres. Each part becomes its own `<object>` with
/// `pid`/`pindex` into a shared materials-extension `<m:colorgroup>`. Bambu Studio
/// assigns a filament slot per distinct color; identical colors share a slot.
pub fn write_3mf_parts(parts: &[ThreeMfPart<'_>]) -> Vec<u8> {
    let model = model_xml_parts(parts);
    zip_store(&[
        ("[Content_Types].xml", CONTENT_TYPES_XML.as_bytes()),
        ("_rels/.rels", ROOT_RELS_XML.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
    ])
}

const CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>
"#;

const ROOT_RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>
"#;

fn model_xml_parts(parts: &[ThreeMfPart<'_>]) -> String {
    // Distinct colors (order of first appearance) → filament slots / pindex.
    // Also keep a display name for basematerials (first material name per color).
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let mut color_names: Vec<&str> = Vec::new();
    let mut pindex_for: Vec<u32> = Vec::with_capacity(parts.len());
    for part in parts {
        let idx = colors.iter().position(|&c| c == part.color).unwrap_or_else(|| {
            let i = colors.len();
            colors.push(part.color);
            color_names.push(part.material_name);
            i
        });
        pindex_for.push(idx as u32);
    }

    let mut capacity = 512 + colors.len() * 100;
    for part in parts {
        capacity += part.mesh.triangles.len() * 40;
    }
    let mut out = String::with_capacity(capacity);
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    // Materials extension `m:` is what Bambu Studio reads for filament mapping (#1299).
    let _ = write!(
        out,
        r#"<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:m="{MATERIALS_NS}">"#
    );
    out.push('\n');
    out.push_str("  <resources>\n");

    // id=1: basematerials (standard 3MF viewers). id=2: m:colorgroup (Bambu Studio).
    // Objects `pid` into the colorgroup so Bambu assigns extruders from color.
    const BASEMATERIALS_ID: u32 = 1;
    const COLORGROUP_ID: u32 = 2;
    if !colors.is_empty() {
        let _ = write!(out, "    <basematerials id=\"{BASEMATERIALS_ID}\">\n");
        for (name, color) in color_names.iter().zip(colors.iter()) {
            let _ = write!(
                out,
                "      <base name=\"{}\" displaycolor=\"{}\"/>\n",
                xml_escape(name),
                display_color(*color)
            );
        }
        out.push_str("    </basematerials>\n");
        let _ = write!(out, "    <m:colorgroup id=\"{COLORGROUP_ID}\">\n");
        for color in &colors {
            let _ = write!(
                out,
                "      <m:color color=\"{}\"/>\n",
                display_color(*color)
            );
        }
        out.push_str("    </m:colorgroup>\n");
    }

    // Object resource ids: 3, 4, … when materials present (1=basematerials, 2=colorgroup).
    let first_object_id: u32 = if colors.is_empty() { 1 } else { 3 };
    let mut object_ids: Vec<u32> = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        let object_id = first_object_id + i as u32;
        object_ids.push(object_id);
        let (vertices, triangles) = dedupe_mesh(part.mesh);
        if colors.is_empty() {
            let _ = write!(
                out,
                "    <object id=\"{object_id}\" name=\"{}\" type=\"model\">\n",
                xml_escape(part.name)
            );
        } else {
            // pid → m:colorgroup so Bambu Studio sets extruder from pindex (#1299).
            let _ = write!(
                out,
                "    <object id=\"{object_id}\" name=\"{}\" type=\"model\" pid=\"{COLORGROUP_ID}\" pindex=\"{}\">\n",
                xml_escape(part.name),
                pindex_for[i]
            );
        }
        out.push_str("      <mesh>\n");
        out.push_str("        <vertices>\n");
        for v in &vertices {
            let _ = write!(
                out,
                "          <vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n",
                v.x, v.y, v.z
            );
        }
        out.push_str("        </vertices>\n");
        out.push_str("        <triangles>\n");
        for [a, b, c] in &triangles {
            let _ = write!(
                out,
                "          <triangle v1=\"{a}\" v2=\"{b}\" v3=\"{c}\"/>\n"
            );
        }
        out.push_str("        </triangles>\n");
        out.push_str("      </mesh>\n");
        out.push_str("    </object>\n");
    }

    out.push_str("  </resources>\n");
    out.push_str("  <build>\n");
    for id in object_ids {
        let _ = write!(out, "    <item objectid=\"{id}\"/>\n");
    }
    out.push_str("  </build>\n");
    out.push_str("</model>\n");
    out
}

/// 3MF color: `#RRGGBBAA` (opaque). Used for both basematerials displaycolor and m:color.
fn display_color(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}FF", rgb[0], rgb[1], rgb[2])
}

/// Collapse repeated triangle corners into a vertex table + index triples.
fn dedupe_mesh(mesh: &SolidMesh) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut index: HashMap<[u32; 3], u32> = HashMap::new();
    let mut triangles: Vec<[u32; 3]> = Vec::with_capacity(mesh.triangles.len());
    for tri in &mesh.triangles {
        let mut ids = [0u32; 3];
        for (i, v) in tri.iter().enumerate() {
            let key = [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
            ids[i] = *index.entry(key).or_insert_with(|| {
                let id = vertices.len() as u32;
                vertices.push(*v);
                id
            });
        }
        triangles.push(ids);
    }
    (vertices, triangles)
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Build a ZIP archive with store (method 0) entries.
fn zip_store(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for &(name, data) in files {
        let name_bytes = name.as_bytes();
        let crc = crc32(data);
        let size = data.len() as u32;
        let local_offset = out.len() as u32;

        // Local file header.
        out.extend_from_slice(&0x04034b50u32.to_le_bytes()); // signature
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: store
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes()); // compressed
        out.extend_from_slice(&size.to_le_bytes()); // uncompressed
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // Central directory header.
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central.extend_from_slice(&0u16.to_le_bytes()); // mod date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk start
        central.extend_from_slice(&0u16.to_le_bytes()); // int attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // ext attrs
        central.extend_from_slice(&local_offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }

    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);

    // End of central directory.
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // start disk
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    out
}

/// ISO HDLC CRC-32 (ZIP/PNG polynomial).
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in data {
        let idx = ((crc ^ u32::from(b)) & 0xff) as usize;
        crc = CRC_TABLE[idx] ^ (crc >> 8);
    }
    !crc
}

static CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 != 0 {
                c = 0xedb8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// Pull a stored ZIP entry's payload by path (test helper).
#[cfg(test)]
fn zip_entry(archive: &[u8], want: &str) -> Option<Vec<u8>> {
    let want = want.as_bytes();
    let mut pos = 0;
    while pos + 30 <= archive.len() {
        let sig = u32::from_le_bytes(archive[pos..pos + 4].try_into().ok()?);
        if sig != 0x04034b50 {
            break;
        }
        let method = u16::from_le_bytes(archive[pos + 8..pos + 10].try_into().ok()?);
        let comp = u32::from_le_bytes(archive[pos + 18..pos + 22].try_into().ok()?) as usize;
        let name_len = u16::from_le_bytes(archive[pos + 26..pos + 28].try_into().ok()?) as usize;
        let extra_len = u16::from_le_bytes(archive[pos + 28..pos + 30].try_into().ok()?) as usize;
        let name_start = pos + 30;
        let name_end = name_start + name_len;
        let data_start = name_end + extra_len;
        let data_end = data_start + comp;
        if data_end > archive.len() {
            return None;
        }
        let name = &archive[name_start..name_end];
        if name == want {
            if method != 0 {
                return None; // only store
            }
            return Some(archive[data_start..data_end].to_vec());
        }
        pos = data_end;
    }
    None
}

#[cfg(test)]
fn box_mesh() -> SolidMesh {
    // Unit cube: 12 triangles, 8 unique corners.
    let p = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(1.0, 0.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(0.0, 1.0, 1.0),
    ];
    let faces: [[usize; 3]; 12] = [
        [0, 1, 2],
        [0, 2, 3],
        [4, 6, 5],
        [4, 7, 6],
        [0, 4, 5],
        [0, 5, 1],
        [1, 5, 6],
        [1, 6, 2],
        [2, 6, 7],
        [2, 7, 3],
        [3, 7, 4],
        [3, 4, 0],
    ];
    SolidMesh {
        triangles: faces.iter().map(|[a, b, c]| [p[*a], p[*b], p[*c]]).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_part(name: &str, mesh: &SolidMesh) -> Vec<u8> {
        write_3mf_parts(&[ThreeMfPart {
            name,
            mesh,
            color: DEFAULT_BODY_COLOR,
            material_name: "Default",
        }])
    }

    #[test]
    fn write_3mf_is_a_zip_package() {
        let bytes = one_part("part", &box_mesh());
        assert!(bytes.len() > 100, "package too small: {}", bytes.len());
        assert_eq!(&bytes[0..4], b"PK\x03\x04", "local file header magic");
    }

    #[test]
    fn write_3mf_contains_required_parts() {
        let bytes = one_part("Block", &box_mesh());
        let types = zip_entry(&bytes, "[Content_Types].xml").expect("content types");
        let types = String::from_utf8(types).unwrap();
        assert!(types.contains("3dmanufacturing-3dmodel+xml"));

        let rels = zip_entry(&bytes, "_rels/.rels").expect("rels");
        let rels = String::from_utf8(rels).unwrap();
        assert!(rels.contains("3D/3dmodel.model"));

        let model = zip_entry(&bytes, "3D/3dmodel.model").expect("model");
        let model = String::from_utf8(model).unwrap();
        assert!(model.contains("unit=\"millimeter\""));
        assert!(model.contains("name=\"Block\""));
        assert!(model.contains("<vertex "), "vertices");
        assert!(model.contains("<triangle "), "triangles");
        // basematerials id=1, colorgroup id=2; mesh object is id=3 (pid into colorgroup).
        assert!(model.contains("<basematerials id=\"1\">"));
        assert!(model.contains("<m:colorgroup id=\"2\">"));
        assert!(model.contains("objectid=\"3\""));
        assert!(model.contains("pid=\"2\""));
        assert!(model.contains("pindex=\"0\""));
    }

    #[test]
    fn write_3mf_dedupes_shared_vertices() {
        let bytes = one_part("cube", &box_mesh());
        let model = String::from_utf8(zip_entry(&bytes, "3D/3dmodel.model").unwrap()).unwrap();
        let n_verts = model.matches("<vertex ").count();
        let n_tris = model.matches("<triangle ").count();
        assert_eq!(n_verts, 8, "unit cube has 8 unique corners, got {n_verts}");
        assert_eq!(n_tris, 12, "unit cube has 12 triangles, got {n_tris}");
    }

    #[test]
    fn write_3mf_escapes_object_name() {
        let bytes = one_part("a&b<c>", &box_mesh());
        let model = String::from_utf8(zip_entry(&bytes, "3D/3dmodel.model").unwrap()).unwrap();
        assert!(model.contains("name=\"a&amp;b&lt;c&gt;\""));
    }

    #[test]
    fn write_3mf_empty_mesh_is_still_valid_package() {
        let bytes = one_part("empty", &SolidMesh::default());
        assert_eq!(&bytes[0..4], b"PK\x03\x04");
        let model = String::from_utf8(zip_entry(&bytes, "3D/3dmodel.model").unwrap()).unwrap();
        assert!(model.contains("<vertices>"));
        assert!(model.contains("<triangles>"));
        assert_eq!(model.matches("<vertex ").count(), 0);
        assert_eq!(model.matches("<triangle ").count(), 0);
    }

    /// #1294 / #1299: each colored body is its own object; basematerials for viewers and
    /// m:colorgroup (materials extension) so Bambu Studio maps colors → filament slots.
    #[test]
    fn write_3mf_parts_emits_colorgroup_and_per_color_objects() {
        let red = box_mesh();
        let yellow = {
            let mut m = box_mesh();
            for tri in &mut m.triangles {
                for v in tri {
                    v.z += 2.0;
                }
            }
            m
        };
        let bytes = write_3mf_parts(&[
            ThreeMfPart {
                name: "Body 0",
                mesh: &yellow,
                color: [0xe8, 0xc9, 0x4a],
                material_name: "Yellow",
            },
            ThreeMfPart {
                name: "Body 1",
                mesh: &red,
                color: [0xe8, 0x61, 0x5c],
                material_name: "Red",
            },
        ]);
        let model = String::from_utf8(zip_entry(&bytes, "3D/3dmodel.model").unwrap()).unwrap();

        assert!(
            model.contains("xmlns:m=\"http://schemas.microsoft.com/3dmanufacturing/material/2015/02\""),
            "materials extension namespace:\n{model}"
        );
        assert!(
            model.contains("<basematerials id=\"1\">"),
            "basematerials group:\n{model}"
        );
        assert!(
            model.contains("name=\"Yellow\" displaycolor=\"#E8C94AFF\""),
            "yellow basematerial:\n{model}"
        );
        assert!(
            model.contains("name=\"Red\" displaycolor=\"#E8615CFF\""),
            "red basematerial:\n{model}"
        );
        // Bambu Studio reads m:colorgroup / m:color (not basematerials) for extruders.
        assert!(
            model.contains("<m:colorgroup id=\"2\">"),
            "colorgroup:\n{model}"
        );
        assert!(
            model.contains("<m:color color=\"#E8C94AFF\"/>"),
            "yellow m:color:\n{model}"
        );
        assert!(
            model.contains("<m:color color=\"#E8615CFF\"/>"),
            "red m:color:\n{model}"
        );
        assert!(model.contains("name=\"Body 0\""), "body 0 object");
        assert!(model.contains("name=\"Body 1\""), "body 1 object");
        // Objects pid into colorgroup (id=2), distinct pindex → filament 1 and 2.
        assert!(model.contains("pid=\"2\" pindex=\"0\""), "fila 1:\n{model}");
        assert!(model.contains("pid=\"2\" pindex=\"1\""), "fila 2:\n{model}");
        assert!(model.contains("objectid=\"3\""));
        assert!(model.contains("objectid=\"4\""));
        // Two cubes → 16 unique verts, 24 triangles.
        assert_eq!(model.matches("<vertex ").count(), 16);
        assert_eq!(model.matches("<triangle ").count(), 24);
    }

    /// Bodies that share a color share one colorgroup entry (same pindex / filament).
    #[test]
    fn write_3mf_parts_dedupes_shared_colors() {
        let a = box_mesh();
        let b = box_mesh();
        let bytes = write_3mf_parts(&[
            ThreeMfPart {
                name: "a",
                mesh: &a,
                color: [0xff, 0x00, 0x00],
                material_name: "Red",
            },
            ThreeMfPart {
                name: "b",
                mesh: &b,
                color: [0xff, 0x00, 0x00],
                material_name: "Also red", // different name, same color → same filament
            },
        ]);
        let model = String::from_utf8(zip_entry(&bytes, "3D/3dmodel.model").unwrap()).unwrap();
        assert_eq!(
            model.matches("<m:color ").count(),
            1,
            "one shared m:color:\n{model}"
        );
        assert_eq!(
            model.matches("<base ").count(),
            1,
            "one shared basematerial:\n{model}"
        );
        assert_eq!(model.matches("pindex=\"0\"").count(), 2);
        assert!(!model.contains("pindex=\"1\""));
    }

    #[test]
    fn display_color_is_rrggbbaa() {
        assert_eq!(display_color([0xe8, 0xc9, 0x4a]), "#E8C94AFF");
        assert_eq!(display_color([0, 0, 0]), "#000000FF");
    }

    #[test]
    fn crc32_matches_known_value() {
        // "123456789" → 0xCBF43926 (ISO HDLC).
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }
}
