// OCCT-backed implementation of the BearCAD kernel C ABI (see bearcad_kernel.hpp).
//
// Only compiled when BearCAD is built with `--features occt`; the `cc` build in
// build.rs pulls this in and links it against the OCCT static libraries.

#include "bearcad_kernel.hpp"

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <gp_Ax1.hxx>
#include <gp_Trsf.hxx>
#include <gp_Ax2.hxx>
#include <gp_Dir.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <GeomAPI_PointsToBSpline.hxx>
#include <NCollection_Array1.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <STEPControl_Writer.hxx>
#include <STEPControl_Reader.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <BRep_Tool.hxx>
#include <GeomAPI_ProjectPointOnCurve.hxx>
#include <Geom_Curve.hxx>
#include <Geom_Circle.hxx>
#include <Geom_Ellipse.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <BRepGProp.hxx>
#include <GProp_GProps.hxx>
#include <Poly_Triangulation.hxx>
#include <Bnd_Box.hxx>
#include <BRepBndLib.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <NCollection_IndexedMap.hxx>
#include <TopTools_ShapeMapHasher.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Edge.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Solid.hxx>
#include <TopoDS_Vertex.hxx>
#include <TopoDS_Wire.hxx>
#include <TopLoc_Location.hxx>
#include <TopAbs_Orientation.hxx>
#include <gp_Pnt.hxx>
#include <gp_Vec.hxx>
#include <Standard_Failure.hxx>
#include <Standard_Version.hxx>

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <vector>

// Opaque owned BREP shape handle exposed across the C ABI.
struct BearcadShape {
    TopoDS_Shape shape;
};

extern "C" double bearcad_kernel_box_volume(double dx, double dy, double dz) {
    try {
        BRepPrimAPI_MakeBox mk(dx, dy, dz);
        TopoDS_Solid solid = mk.Solid();
        GProp_GProps props;
        BRepGProp::VolumeProperties(solid, props);
        return props.Mass();
    } catch (const Standard_Failure&) {
        // Surface OCCT failures as a sentinel the Rust side treats as "kernel error"
        // rather than letting a C++ exception unwind across the FFI boundary (UB).
        return -1.0;
    } catch (...) {
        return -1.0;
    }
}

extern "C" const char* bearcad_kernel_occt_version(void) {
    return OCC_VERSION_STRING_EXT;
}

extern "C" BearcadShape* bearcad_shape_prism(const double* xyz, unsigned long n_pts,
                                             double dx, double dy, double dz) {
    if (xyz == nullptr || n_pts < 3) {
        return nullptr;
    }
    try {
        BRepBuilderAPI_MakePolygon poly;
        for (unsigned long i = 0; i < n_pts; ++i) {
            poly.Add(gp_Pnt(xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]));
        }
        poly.Close();
        if (!poly.IsDone()) {
            return nullptr;
        }
        BRepBuilderAPI_MakeFace face(poly.Wire());
        if (!face.IsDone()) {
            return nullptr;
        }
        BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(dx, dy, dz));
        return new BearcadShape{prism.Shape()};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

// Revolve a closed planar profile (world-space points, first point not repeated) around
// the axis through (ox,oy,oz) with direction (ax,ay,az) by `angle_rad`. When `symmetric`
// is nonzero the profile is pre-rotated by -angle/2 so the sweep straddles its plane.
extern "C" BearcadShape* bearcad_shape_revolve(const double* xyz, unsigned long n_pts,
                                               double ox, double oy, double oz, double ax,
                                               double ay, double az, double angle_rad,
                                               int symmetric) {
    if (xyz == nullptr || n_pts < 3 || angle_rad <= 0.0) {
        return nullptr;
    }
    try {
        BRepBuilderAPI_MakePolygon poly;
        for (unsigned long i = 0; i < n_pts; ++i) {
            poly.Add(gp_Pnt(xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]));
        }
        poly.Close();
        if (!poly.IsDone()) {
            return nullptr;
        }
        BRepBuilderAPI_MakeFace face(poly.Wire());
        if (!face.IsDone()) {
            return nullptr;
        }
        gp_Ax1 axis(gp_Pnt(ox, oy, oz), gp_Dir(ax, ay, az));
        TopoDS_Shape profile = face.Face();
        if (symmetric != 0) {
            gp_Trsf pre;
            pre.SetRotation(axis, -angle_rad / 2.0);
            profile = BRepBuilderAPI_Transform(profile, pre, true).Shape();
        }
        // A full revolution must use the no-angle constructor: MakeRevol normalizes the
        // angle modulo 2*pi, so a float angle a hair over 2*pi builds a degenerate sliver.
        if (angle_rad >= 2.0 * M_PI - 1e-6) {
            BRepPrimAPI_MakeRevol revol(profile, axis);
            if (!revol.IsDone()) {
                return nullptr;
            }
            return new BearcadShape{revol.Shape()};
        }
        BRepPrimAPI_MakeRevol revol(profile, axis, angle_rad);
        if (!revol.IsDone()) {
            return nullptr;
        }
        return new BearcadShape{revol.Shape()};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

extern "C" BearcadShape* bearcad_shape_loft(const double* bottom_xyz, const double* top_xyz,
                                            unsigned long n_pts) {
    if (bottom_xyz == nullptr || top_xyz == nullptr || n_pts < 3) {
        return nullptr;
    }
    try {
        BRepBuilderAPI_MakePolygon bottom;
        BRepBuilderAPI_MakePolygon top;
        for (unsigned long i = 0; i < n_pts; ++i) {
            bottom.Add(gp_Pnt(bottom_xyz[3 * i], bottom_xyz[3 * i + 1], bottom_xyz[3 * i + 2]));
            top.Add(gp_Pnt(top_xyz[3 * i], top_xyz[3 * i + 1], top_xyz[3 * i + 2]));
        }
        bottom.Close();
        top.Close();
        if (!bottom.IsDone() || !top.IsDone()) {
            return nullptr;
        }
        // isSolid = true (cap the ends), ruled = true (planar strips between
        // corresponding edges rather than a smooth interpolation).
        BRepOffsetAPI_ThruSections gen(true, true);
        gen.AddWire(bottom.Wire());
        gen.AddWire(top.Wire());
        gen.Build();
        if (!gen.IsDone()) {
            return nullptr;
        }
        return new BearcadShape{gen.Shape()};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

// Sweep a closed planar profile along a path polyline (#sweep). The profile is
// swept with BRepOffsetAPI_MakePipeShell (WithCorrection keeps it normal to the spine);
// a `smooth` path interpolates its points with a B-spline so curved sketch segments
// sweep as curves, an all-straight path keeps sharp right-corner transitions.
extern "C" BearcadShape* bearcad_shape_sweep(const double* profile_xyz, unsigned long n_profile,
                                             const double* path_xyz, unsigned long n_path,
                                             int smooth) {
    if (profile_xyz == nullptr || n_profile < 3 || path_xyz == nullptr || n_path < 2) {
        return nullptr;
    }
    try {
        BRepBuilderAPI_MakePolygon poly;
        for (unsigned long i = 0; i < n_profile; ++i) {
            poly.Add(gp_Pnt(profile_xyz[3 * i], profile_xyz[3 * i + 1], profile_xyz[3 * i + 2]));
        }
        poly.Close();
        if (!poly.IsDone()) {
            return nullptr;
        }
        TopoDS_Wire spine;
        if (smooth != 0) {
            NCollection_Array1<gp_Pnt> pts(1, static_cast<int>(n_path));
            for (unsigned long i = 0; i < n_path; ++i) {
                pts.SetValue(static_cast<int>(i + 1),
                             gp_Pnt(path_xyz[3 * i], path_xyz[3 * i + 1], path_xyz[3 * i + 2]));
            }
            GeomAPI_PointsToBSpline fit(pts);
            if (!fit.IsDone()) {
                return nullptr;
            }
            BRepBuilderAPI_MakeWire wire(BRepBuilderAPI_MakeEdge(fit.Curve()).Edge());
            if (!wire.IsDone()) {
                return nullptr;
            }
            spine = wire.Wire();
        } else {
            BRepBuilderAPI_MakePolygon path;
            for (unsigned long i = 0; i < n_path; ++i) {
                path.Add(gp_Pnt(path_xyz[3 * i], path_xyz[3 * i + 1], path_xyz[3 * i + 2]));
            }
            if (!path.IsDone()) {
                return nullptr;
            }
            spine = path.Wire();
        }
        BRepOffsetAPI_MakePipeShell pipe(spine);
        pipe.SetTransitionMode(BRepBuilderAPI_RightCorner);
        // WithContact = false (the profile stays where it is relative to the spine start),
        // WithCorrection = true (the profile is rotated normal to the spine tangent).
        pipe.Add(poly.Wire(), false, true);
        pipe.Build();
        if (!pipe.IsDone()) {
            return nullptr;
        }
        if (!pipe.MakeSolid()) {
            return nullptr;
        }
        return new BearcadShape{pipe.Shape()};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

extern "C" BearcadShape* bearcad_shape_boolean(const BearcadShape* a, const BearcadShape* b,
                                               int op) {
    if (a == nullptr || b == nullptr) {
        return nullptr;
    }
    try {
        TopoDS_Shape result;
        switch (op) {
            case 0: result = BRepAlgoAPI_Fuse(a->shape, b->shape).Shape(); break;
            case 1: result = BRepAlgoAPI_Cut(a->shape, b->shape).Shape(); break;
            case 2: result = BRepAlgoAPI_Common(a->shape, b->shape).Shape(); break;
            default: return nullptr;
        }
        return new BearcadShape{result};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

namespace {

// Build a planar TopoDS_Face on z=0 from a closed 2D loop (x,y pairs, first
// point not repeated). Returns a null face on failure.
TopoDS_Face make_planar_face(const double* xy, unsigned long n) {
    BRepBuilderAPI_MakePolygon poly;
    for (unsigned long i = 0; i < n; ++i) {
        poly.Add(gp_Pnt(xy[2 * i], xy[2 * i + 1], 0.0));
    }
    poly.Close();
    if (!poly.IsDone()) {
        return TopoDS_Face();
    }
    BRepBuilderAPI_MakeFace face(poly.Wire());
    if (!face.IsDone()) {
        return TopoDS_Face();
    }
    return face.Face();
}

}  // namespace

extern "C" double* bearcad_face_boolean_loop(const double* a_xy, unsigned long a_n,
                                             const double* b_xy, unsigned long b_n,
                                             int op, unsigned long* out_n) {
    if (out_n != nullptr) {
        *out_n = 0;
    }
    if (a_xy == nullptr || b_xy == nullptr || a_n < 3 || b_n < 3 || out_n == nullptr) {
        return nullptr;
    }
    try {
        TopoDS_Face fa = make_planar_face(a_xy, a_n);
        TopoDS_Face fb = make_planar_face(b_xy, b_n);
        if (fa.IsNull() || fb.IsNull()) {
            return nullptr;
        }
        TopoDS_Shape result;
        switch (op) {
            case 1: result = BRepAlgoAPI_Cut(fa, fb).Shape(); break;
            case 2: result = BRepAlgoAPI_Common(fa, fb).Shape(); break;
            default: return nullptr;
        }
        if (result.IsNull()) {
            return nullptr;
        }

        // Strictness contract (#88, mirrors the Rust fallback clipper): the result
        // must be exactly ONE face...
        TopoDS_Face face;
        int face_count = 0;
        for (TopExp_Explorer ex(result, TopAbs_FACE); ex.More(); ex.Next()) {
            face = TopoDS::Face(ex.Current());
            ++face_count;
        }
        if (face_count != 1) {
            return nullptr;  // empty (disjoint common, consumed cut) or multi-part
        }
        // ...with exactly ONE wire (no holes — e.g. an annulus from subtracting a
        // strictly-interior shape has an outer and an inner wire).
        TopoDS_Wire wire;
        int wire_count = 0;
        for (TopExp_Explorer wx(face, TopAbs_WIRE); wx.More(); wx.Next()) {
            wire = TopoDS::Wire(wx.Current());
            ++wire_count;
        }
        if (wire_count != 1) {
            return nullptr;
        }

        // Walk the wire in connection order (BRepTools_WireExplorer yields edges in
        // loop order, unlike TopExp_Explorer). All edges of a polygon-face boolean
        // are straight lines, so one vertex per edge — CurrentVertex() is the vertex
        // the current edge shares with the previous one, i.e. the current edge's
        // start point in loop order — reproduces the boundary exactly.
        std::vector<double> pts;
        for (BRepTools_WireExplorer wex(wire, face); wex.More(); wex.Next()) {
            gp_Pnt p = BRep_Tool::Pnt(wex.CurrentVertex());
            pts.push_back(p.X());
            pts.push_back(p.Y());
        }
        if (pts.size() < 6) {
            return nullptr;  // degenerate (fewer than 3 vertices)
        }
        *out_n = static_cast<unsigned long>(pts.size() / 2);
        double* out = new double[pts.size()];
        std::copy(pts.begin(), pts.end(), out);
        return out;
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

extern "C" void bearcad_pts_free(double* pts) {
    delete[] pts;
}

namespace {

// Match each requested edge (endpoint pair) to one of the shape's OCCT edges by
// world-space endpoints, add it to `maker` with its per-edge amount, then build.
// Returns the resulting shape, or an empty/null shape (via IsNull) on any failure.
// `Maker` is BRepFilletAPI_MakeFillet or BRepFilletAPI_MakeChamfer — both expose
// Add(Standard_Real, const TopoDS_Edge&), Build(), IsDone(), Shape().
template <typename Maker>
TopoDS_Shape apply_edge_treatment(const TopoDS_Shape& shape, const double* edges,
                                  const double* amounts, unsigned long n) {
    // Tolerance scaled to the shape's bounding box (min 1e-6) so endpoint matching
    // is robust across model sizes without matching unrelated nearby vertices.
    double tol = 1e-6;
    {
        Bnd_Box bb;
        BRepBndLib::Add(shape, bb);
        if (!bb.IsVoid()) {
            double xmin, ymin, zmin, xmax, ymax, zmax;
            bb.Get(xmin, ymin, zmin, xmax, ymax, zmax);
            double dx = xmax - xmin, dy = ymax - ymin, dz = zmax - zmin;
            double diag = std::sqrt(dx * dx + dy * dy + dz * dz);
            tol = std::max(1e-4 * diag, 1e-6);
        }
    }

    // Dedupe: TopExp::MapShapes visits each shared edge once.
    NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> edgeMap;
    TopExp::MapShapes(shape, TopAbs_EDGE, edgeMap);

    Maker maker(shape);
    auto near = [tol](const gp_Pnt& p, double x, double y, double z) {
        return p.SquareDistance(gp_Pnt(x, y, z)) <= tol * tol;
    };

    // Whether both request points lie on the edge's own curve (within tol) — matches
    // CLOSED edges (a cylinder cap's rim circle has a seam vertex, so endpoint matching
    // can't see it); callers request such an edge as two distinct points on the curve.
    auto on_curve = [tol](const TopoDS_Edge& edge, const gp_Pnt& a, const gp_Pnt& b) {
        double f, l;
        Handle(Geom_Curve) curve = BRep_Tool::Curve(edge, f, l);
        if (curve.IsNull()) {
            return false;
        }
        for (const gp_Pnt& p : {a, b}) {
            GeomAPI_ProjectPointOnCurve proj(p, curve, f, l);
            if (proj.NbPoints() == 0 || proj.LowerDistance() > tol) {
                return false;
            }
        }
        return true;
    };

    for (unsigned long i = 0; i < n; ++i) {
        const double* e = edges + 6 * i;
        gp_Pnt ra(e[0], e[1], e[2]);
        gp_Pnt rb(e[3], e[4], e[5]);
        bool matched = false;
        // Pass 1: exact endpoint matching (open edges).
        for (int k = 1; k <= edgeMap.Extent(); ++k) {
            const TopoDS_Edge& edge = TopoDS::Edge(edgeMap(k));
            TopoDS_Vertex v1, v2;
            TopExp::Vertices(edge, v1, v2);
            if (v1.IsNull() || v2.IsNull()) {
                continue;
            }
            gp_Pnt p1 = BRep_Tool::Pnt(v1);
            gp_Pnt p2 = BRep_Tool::Pnt(v2);
            bool fwd = near(p1, e[0], e[1], e[2]) && near(p2, e[3], e[4], e[5]);
            bool rev = near(p1, e[3], e[4], e[5]) && near(p2, e[0], e[1], e[2]);
            if (fwd || rev) {
                maker.Add(amounts[i], edge);
                matched = true;
                break;
            }
        }
        // Pass 2: closed/seamed edges (circular rims), matched by both request points
        // lying on the edge's curve. Restricted to edges whose two vertices coincide
        // (i.e. actually closed), so a long straight edge that happens to pass through
        // both points can't shadow an exact endpoint match from pass 1.
        if (!matched) {
            for (int k = 1; k <= edgeMap.Extent(); ++k) {
                const TopoDS_Edge& edge = TopoDS::Edge(edgeMap(k));
                TopoDS_Vertex v1, v2;
                TopExp::Vertices(edge, v1, v2);
                bool closed = (!v1.IsNull() && !v2.IsNull()
                               && BRep_Tool::Pnt(v1).SquareDistance(BRep_Tool::Pnt(v2))
                                      <= tol * tol)
                              || (v1.IsNull() && v2.IsNull());
                if (!closed) {
                    continue;
                }
                if (on_curve(edge, ra, rb)) {
                    maker.Add(amounts[i], edge);
                    matched = true;
                    break;
                }
            }
        }
        // Pass 3: the rim survived a boolean as one or more ARCS of the requested
        // circle. A coplanar-face cut (a hole drilled flush from a face) often splits
        // the rim circle at the tool's seam, leaving open arc edges that neither pass
        // above can see. The two request points are diametrical, so reconstruct the
        // circle they describe and add every edge whose underlying curve is that
        // circle — chamfering/filleting the arcs piecewise is the same ring treatment.
        if (!matched) {
            gp_Pnt center((ra.X() + rb.X()) / 2.0, (ra.Y() + rb.Y()) / 2.0,
                          (ra.Z() + rb.Z()) / 2.0);
            double radius = ra.Distance(rb) / 2.0;
            for (int k = 1; k <= edgeMap.Extent(); ++k) {
                const TopoDS_Edge& edge = TopoDS::Edge(edgeMap(k));
                double f, l;
                Handle(Geom_Curve) curve = BRep_Tool::Curve(edge, f, l);
                if (curve.IsNull()) {
                    continue;
                }
                Handle(Geom_TrimmedCurve) trimmed = Handle(Geom_TrimmedCurve)::DownCast(curve);
                if (!trimmed.IsNull()) {
                    curve = trimmed->BasisCurve();
                }
                // Accept circles and near-circular ellipses alike: a hole drilled
                // flush from an f32-precision sketch face meets that face a hair off
                // perpendicular, so OCCT sections the rim as a Geom_Ellipse whose two
                // radii differ from the hole radius by well under the tolerance.
                gp_Pnt loc;
                double r_major, r_minor;
                if (Handle(Geom_Circle) circ = Handle(Geom_Circle)::DownCast(curve)) {
                    loc = circ->Location();
                    r_major = r_minor = circ->Radius();
                } else if (Handle(Geom_Ellipse) ell = Handle(Geom_Ellipse)::DownCast(curve)) {
                    loc = ell->Location();
                    r_major = ell->MajorRadius();
                    r_minor = ell->MinorRadius();
                } else {
                    continue;
                }
                if (loc.SquareDistance(center) > tol * tol
                    || std::abs(r_major - radius) > tol
                    || std::abs(r_minor - radius) > tol) {
                    continue;
                }
                maker.Add(amounts[i], edge);
                matched = true;
            }
        }
        if (!matched) {
            return TopoDS_Shape();  // requested edge not found -> caller falls back
        }
    }

    maker.Build();
    if (!maker.IsDone()) {
        return TopoDS_Shape();
    }
    return maker.Shape();
}

}  // namespace

// True cylinder (#177): a circle-profile extrusion built as real BREP (circular wall +
// circular rim edges), so rim chamfers/fillets and countersinks are exact cones — a
// faceted prism has no circular edge to treat.
extern "C" BearcadShape* bearcad_shape_cylinder(double cx, double cy, double cz, double ax,
                                                double ay, double az, double radius,
                                                double height) {
    if (radius <= 0.0 || height <= 0.0) {
        return nullptr;
    }
    try {
        gp_Dir dir(ax, ay, az);
        gp_Ax2 frame(gp_Pnt(cx, cy, cz), dir);
        TopoDS_Shape shape = BRepPrimAPI_MakeCylinder(frame, radius, height).Shape();
        if (shape.IsNull()) {
            return nullptr;
        }
        return new BearcadShape{shape};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

extern "C" BearcadShape* bearcad_shape_fillet(const BearcadShape* s, const double* edges,
                                              const double* radii, unsigned long n) {
    if (s == nullptr || edges == nullptr || radii == nullptr || n == 0) {
        return nullptr;
    }
    try {
        TopoDS_Shape result =
            apply_edge_treatment<BRepFilletAPI_MakeFillet>(s->shape, edges, radii, n);
        if (result.IsNull()) {
            return nullptr;
        }
        return new BearcadShape{result};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

extern "C" BearcadShape* bearcad_shape_chamfer(const BearcadShape* s, const double* edges,
                                               const double* dists, unsigned long n) {
    if (s == nullptr || edges == nullptr || dists == nullptr || n == 0) {
        return nullptr;
    }
    try {
        TopoDS_Shape result =
            apply_edge_treatment<BRepFilletAPI_MakeChamfer>(s->shape, edges, dists, n);
        if (result.IsNull()) {
            return nullptr;
        }
        return new BearcadShape{result};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

extern "C" double bearcad_shape_volume(const BearcadShape* shape) {
    if (shape == nullptr) {
        return -1.0;
    }
    try {
        GProp_GProps props;
        BRepGProp::VolumeProperties(shape->shape, props);
        return props.Mass();
    } catch (const Standard_Failure&) {
        return -1.0;
    } catch (...) {
        return -1.0;
    }
}

extern "C" double* bearcad_shape_tessellate(const BearcadShape* shape, double deflection,
                                            unsigned long* out_tri_count) {
    if (out_tri_count != nullptr) {
        *out_tri_count = 0;
    }
    if (shape == nullptr || out_tri_count == nullptr) {
        return nullptr;
    }
    try {
        // Mutating meshing is stored on the shape's TShape; work on a copy of the
        // handle (cheap, shares the underlying TShape) so the const contract holds
        // at the Rust boundary while OCCT attaches its triangulation.
        TopoDS_Shape s = shape->shape;
        BRepMesh_IncrementalMesh mesher(s, deflection, false, 0.5, true);
        mesher.Perform();

        std::vector<double> tris;
        for (TopExp_Explorer ex(s, TopAbs_FACE); ex.More(); ex.Next()) {
            const TopoDS_Face& face = TopoDS::Face(ex.Current());
            TopLoc_Location loc;
            Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(face, loc);
            if (tri.IsNull()) {
                continue;
            }
            const gp_Trsf& trsf = loc.Transformation();
            const bool reversed = face.Orientation() == TopAbs_REVERSED;
            for (int t = 1; t <= tri->NbTriangles(); ++t) {
                int n1, n2, n3;
                tri->Triangle(t).Get(n1, n2, n3);
                if (reversed) {
                    std::swap(n2, n3);
                }
                const int idx[3] = {n1, n2, n3};
                for (int k = 0; k < 3; ++k) {
                    gp_Pnt p = tri->Node(idx[k]).Transformed(trsf);
                    tris.push_back(p.X());
                    tris.push_back(p.Y());
                    tris.push_back(p.Z());
                }
            }
        }
        if (tris.empty()) {
            return nullptr;
        }
        *out_tri_count = static_cast<unsigned long>(tris.size() / 9);
        double* out = new double[tris.size()];
        std::copy(tris.begin(), tris.end(), out);
        return out;
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

extern "C" void bearcad_tri_free(double* tris) {
    delete[] tris;
}

// Split a shape into its individual SOLIDs (a boolean between disjoint bodies can yield
// several disconnected pieces). Returns a malloc'd array of owned shape handles and writes
// its length to `out_count`; free the array itself with `bearcad_handles_free` (each handle
// is then owned by the caller and freed individually with `bearcad_shape_free`). A shape
// with no solids returns null with count 0.
extern "C" BearcadShape** bearcad_shape_split_solids(const BearcadShape* shape,
                                                     unsigned long* out_count) {
    if (out_count != nullptr) {
        *out_count = 0;
    }
    if (shape == nullptr || out_count == nullptr) {
        return nullptr;
    }
    try {
        std::vector<BearcadShape*> solids;
        for (TopExp_Explorer exp(shape->shape, TopAbs_SOLID); exp.More(); exp.Next()) {
            solids.push_back(new BearcadShape{exp.Current()});
        }
        if (solids.empty()) {
            return nullptr;
        }
        BearcadShape** out =
            static_cast<BearcadShape**>(std::malloc(solids.size() * sizeof(BearcadShape*)));
        for (size_t i = 0; i < solids.size(); ++i) {
            out[i] = solids[i];
        }
        *out_count = solids.size();
        return out;
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

// Rigid-transform a shape: `m` is a row-major 3x4 matrix (rotation columns + translation),
// the same layout gp_Trsf::SetValues takes. Returns a new owned shape.
extern "C" BearcadShape* bearcad_shape_transform(const BearcadShape* shape, const double* m) {
    if (shape == nullptr || m == nullptr) {
        return nullptr;
    }
    try {
        gp_Trsf trsf;
        trsf.SetValues(m[0], m[1], m[2], m[3],
                       m[4], m[5], m[6], m[7],
                       m[8], m[9], m[10], m[11]);
        BRepBuilderAPI_Transform op(shape->shape, trsf, /*Copy=*/true);
        if (!op.IsDone()) {
            return nullptr;
        }
        return new BearcadShape{op.Shape()};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

extern "C" void bearcad_handles_free(BearcadShape** handles) {
    std::free(handles);
}

extern "C" void bearcad_shape_free(BearcadShape* shape) {
    delete shape;
}

extern "C" int bearcad_shape_write_step(const BearcadShape* s, const char* path) {
    if (s == nullptr || path == nullptr) {
        return 1;
    }
    try {
        STEPControl_Writer writer;
        if (writer.Transfer(s->shape, STEPControl_AsIs) != IFSelect_RetDone) {
            return 1;
        }
        IFSelect_ReturnStatus status = writer.Write(path);
        return status == IFSelect_RetDone ? 0 : 1;
    } catch (const Standard_Failure&) {
        return 1;
    } catch (...) {
        return 1;
    }
}

extern "C" BearcadShape* bearcad_read_step(const char* path) {
    if (path == nullptr) {
        return nullptr;
    }
    try {
        STEPControl_Reader reader;
        if (reader.ReadFile(path) != IFSelect_RetDone) {
            return nullptr;
        }
        reader.TransferRoots();
        TopoDS_Shape shape = reader.OneShape();
        if (shape.IsNull()) {
            return nullptr;
        }
        return new BearcadShape{shape};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}
