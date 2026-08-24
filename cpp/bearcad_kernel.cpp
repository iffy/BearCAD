// OCCT-backed implementation of the BearCAD kernel C ABI (see bearcad_kernel.hpp).
//
// Only compiled when BearCAD is built with `--features occt`; the `cc` build in
// build.rs pulls this in and links it against the OCCT static libraries.

#include "bearcad_kernel.hpp"

#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepBuilderAPI_Copy.hxx>
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
#include <BRepAlgoAPI_BooleanOperation.hxx>
#include <NCollection_List.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffsetAPI_MakeOffsetShape.hxx>
#include <ShapeFix_Shape.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepLProp_SLProps.hxx>
#include <GeomAbs_JoinType.hxx>
#include <BRepOffset_Mode.hxx>
#include <Precision.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <STEPControl_Writer.hxx>
#include <APIHeaderSection_MakeHeader.hxx>
#include <StepBasic_Product.hxx>
#include <StepBasic_ProductDefinitionFormation.hxx>
#include <StepData_StepModel.hxx>
#include <StepRepr_Representation.hxx>
#include <TCollection_HAsciiString.hxx>
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
#include <gp_Pln.hxx>
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
        // Snap every vertex onto one plane before MakeFace. Sketch frames from
        // tessellated mesh faces are f32, so a "planar" loop can miss OCCT's
        // 1e-7 confusion by ~1e-5 and MakeFace(wire) fails (#1468).
        gp_Pnt p0(xyz[0], xyz[1], xyz[2]);
        gp_Vec normal;
        bool have_n = false;
        for (unsigned long i = 1; i + 1 < n_pts; ++i) {
            gp_Pnt pi(xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]);
            gp_Pnt pj(xyz[3 * (i + 1)], xyz[3 * (i + 1) + 1], xyz[3 * (i + 1) + 2]);
            gp_Vec c = gp_Vec(p0, pi).Crossed(gp_Vec(p0, pj));
            if (c.SquareMagnitude() > 1e-24) {
                normal = c;
                have_n = true;
                break;
            }
        }
        BRepBuilderAPI_MakePolygon poly;
        if (have_n) {
            gp_Dir dn(normal);
            gp_Vec nn(dn);
            gp_Pln pln(p0, dn);
            for (unsigned long i = 0; i < n_pts; ++i) {
                gp_Pnt p(xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]);
                const double d = gp_Vec(p0, p).Dot(nn);
                poly.Add(p.Translated(-d * nn));
            }
            poly.Close();
            if (!poly.IsDone()) {
                return nullptr;
            }
            BRepBuilderAPI_MakeFace face(pln, poly.Wire());
            if (!face.IsDone()) {
                return nullptr;
            }
            BRepPrimAPI_MakePrism prism(face.Face(), gp_Vec(dx, dy, dz));
            return new BearcadShape{prism.Shape()};
        }
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
// Non-zero `pitch` (axial travel per full 2π turn) makes a helix for springs (#1242):
// intermediate profile sections are lofted with ThruSections. Signed `angle_rad` is
// allowed (negative reverses the turn).
extern "C" BearcadShape* bearcad_shape_revolve(const double* xyz, unsigned long n_pts,
                                               double ox, double oy, double oz, double ax,
                                               double ay, double az, double angle_rad,
                                               int symmetric, double pitch) {
    if (xyz == nullptr || n_pts < 3 || std::fabs(angle_rad) < 1e-12) {
        return nullptr;
    }
    try {
        gp_Pnt origin(ox, oy, oz);
        gp_Dir dir(ax, ay, az);
        // Negative angle: flip the axis so MakeRevol (which wants a positive angle) and
        // the helix sections still wind the right way.
        double signed_angle = angle_rad;
        if (signed_angle < 0.0) {
            dir.Reverse();
            signed_angle = -signed_angle;
            pitch = -pitch;
        }
        gp_Ax1 axis(origin, dir);

        // Pure revolve path (no pitch): BRepPrimAPI_MakeRevol.
        if (std::fabs(pitch) < 1e-12) {
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
            TopoDS_Shape profile = face.Face();
            if (symmetric != 0) {
                gp_Trsf pre;
                pre.SetRotation(axis, -signed_angle / 2.0);
                profile = BRepBuilderAPI_Transform(profile, pre, true).Shape();
            }
            // A full revolution must use the no-angle constructor: MakeRevol normalizes the
            // angle modulo 2*pi, so a float angle a hair over 2*pi builds a degenerate sliver.
            if (signed_angle >= 2.0 * M_PI - 1e-6) {
                BRepPrimAPI_MakeRevol revol(profile, axis);
                if (!revol.IsDone()) {
                    return nullptr;
                }
                return new BearcadShape{revol.Shape()};
            }
            BRepPrimAPI_MakeRevol revol(profile, axis, signed_angle);
            if (!revol.IsDone()) {
                return nullptr;
            }
            return new BearcadShape{revol.Shape()};
        }

        // Helical revolve (#1242/#1249): screw the profile along a smooth helix spine
        // (pipe shell with fixed BiNormal = revolve axis). That matches the lathe
        // semantics of "rotate about the axis + advance pitch per turn" while producing
        // true curved BREP for STEP and adaptive tessellation for the viewport.
        //
        // #1248's ruled ThruSections shortcut kept pan/orbit interactive by making
        // planar strips — but the solid looked faceted and STEP exported faceted
        // surfaces. A pipe along a B-spline helix keeps curved faces; the adaptive
        // deflection floor in bearcad_shape_tessellate still bounds triangle count.
        const double start =
            (symmetric != 0) ? -signed_angle / 2.0 : 0.0;
        const double end =
            (symmetric != 0) ? signed_angle / 2.0 : signed_angle;
        const double turns = std::fabs(signed_angle) / (2.0 * M_PI);

        // Profile centroid — attachment point whose screw path is the spine.
        gp_XYZ sum(0.0, 0.0, 0.0);
        for (unsigned long i = 0; i < n_pts; ++i) {
            sum.SetX(sum.X() + xyz[3 * i]);
            sum.SetY(sum.Y() + xyz[3 * i + 1]);
            sum.SetZ(sum.Z() + xyz[3 * i + 2]);
        }
        const double inv = 1.0 / static_cast<double>(n_pts);
        gp_Pnt centroid(sum.X() * inv, sum.Y() * inv, sum.Z() * inv);

        auto screw_trsf = [&](double a) -> gp_Trsf {
            const double axial = pitch * (a / (2.0 * M_PI));
            gp_Trsf tr_rot;
            tr_rot.SetRotation(axis, a);
            gp_Trsf tr_axial;
            tr_axial.SetTranslation(gp_Vec(dir) * axial);
            return tr_axial * tr_rot;
        };

        // Dense helix samples for a smooth B-spline spine (~24/turn, hard-capped).
        int n_spine = static_cast<int>(std::ceil(turns * 24.0));
        if (n_spine < 16) {
            n_spine = 16;
        }
        if (n_spine > 512) {
            n_spine = 512;
        }
        NCollection_Array1<gp_Pnt> spine_pts(1, n_spine + 1);
        for (int s = 0; s <= n_spine; ++s) {
            const double t = static_cast<double>(s) / static_cast<double>(n_spine);
            const double a = start + t * (end - start);
            spine_pts.SetValue(s + 1, centroid.Transformed(screw_trsf(a)));
        }
        GeomAPI_PointsToBSpline fit(spine_pts);
        if (!fit.IsDone()) {
            return nullptr;
        }
        BRepBuilderAPI_MakeEdge spine_edge(fit.Curve());
        if (!spine_edge.IsDone()) {
            return nullptr;
        }
        BRepBuilderAPI_MakeWire spine_wire(spine_edge.Edge());
        if (!spine_wire.IsDone()) {
            return nullptr;
        }

        // Profile wire at the start pose of the sweep (screw by `start`).
        const gp_Trsf pre = screw_trsf(start);
        BRepBuilderAPI_MakePolygon poly;
        for (unsigned long i = 0; i < n_pts; ++i) {
            gp_Pnt p(xyz[3 * i], xyz[3 * i + 1], xyz[3 * i + 2]);
            p.Transform(pre);
            poly.Add(p);
        }
        poly.Close();
        if (!poly.IsDone()) {
            return nullptr;
        }

        BRepOffsetAPI_MakePipeShell pipe(spine_wire.Wire());
        // Fixed BiNormal = revolve axis keeps the profile parallel to its start
        // orientation (screw motion), not Frenet-twisted along the helix tangent.
        pipe.SetMode(dir);
        pipe.SetTransitionMode(BRepBuilderAPI_RoundCorner);
        // WithContact=false: profile already sits at the spine start; WithCorrection=
        // false: don't re-orient onto the tangent (BiNormal mode owns orientation).
        pipe.Add(poly.Wire(), /*WithContact=*/false, /*WithCorrection=*/false);
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

namespace {

double shape_volume(const TopoDS_Shape& s) {
    if (s.IsNull()) {
        return 0.0;
    }
    GProp_GProps props;
    BRepGProp::VolumeProperties(s, props);
    return std::fabs(props.Mass());
}

// Whether the two shapes' axis-aligned bounding boxes meet at all — the cheap test for
// "these could possibly intersect", so solids that are plainly far apart never pay for the
// volume comparison below.
bool boxes_overlap(const TopoDS_Shape& a, const TopoDS_Shape& b) {
    Bnd_Box ba;
    Bnd_Box bb;
    BRepBndLib::Add(a, ba);
    BRepBndLib::Add(b, bb);
    if (ba.IsVoid() || bb.IsVoid()) {
        return false;
    }
    return !ba.IsOut(bb);
}

// One boolean attempt. `fuzzy > 0` widens OCCT's coincidence tolerance. Returns false when
// the algorithm reports a failure; `out` is the result shape otherwise.
bool run_boolean(const TopoDS_Shape& a, const TopoDS_Shape& b, int op, double fuzzy,
                 TopoDS_Shape& out) {
    BRepAlgoAPI_Fuse fuse;
    BRepAlgoAPI_Cut cut;
    BRepAlgoAPI_Common common;
    BRepAlgoAPI_BooleanOperation* algo = nullptr;
    switch (op) {
        case 0: algo = &fuse; break;
        case 1: algo = &cut; break;
        case 2: algo = &common; break;
        default: return false;
    }
    NCollection_List<TopoDS_Shape> args;
    NCollection_List<TopoDS_Shape> tools;
    args.Append(a);
    tools.Append(b);
    algo->SetArguments(args);
    algo->SetTools(tools);
    // Cached operand solids must stay intact; booleans that mutate their
    // arguments would corrupt the body-shape memo (#1337).
    algo->SetNonDestructive(true);
    algo->SetRunParallel(true);
    if (fuzzy > 0.0) {
        algo->SetFuzzyValue(fuzzy);
    }
    algo->Build();
    if (!algo->IsDone() || algo->HasErrors()) {
        return false;
    }
    out = algo->Shape();
    return !out.IsNull();
}

// Whether a completed boolean silently did nothing — the failure mode a tangential
// coincidence provokes (#1033: a snapped sphere whose surface passes exactly through the
// box corner it was snapped to, so OCCT finds no intersection at all and hands back A).
// Only meaningful when the operands' bounding boxes actually overlap.
bool boolean_is_a_no_op(const TopoDS_Shape& a, const TopoDS_Shape& b,
                        const TopoDS_Shape& result, int op) {
    const double va = shape_volume(a);
    const double vb = shape_volume(b);
    const double vr = shape_volume(result);
    // Relative to the larger operand, so the test scales with the model.
    const double eps = std::max(va, vb) * 1e-9;
    switch (op) {
        // A union can never be smaller than either operand, and can only equal the larger
        // one when the smaller sits wholly inside it.
        case 0: return vr < std::max(va, vb) - eps;
        case 1: return std::fabs(vr - va) <= eps;
        case 2: return vr <= eps;
        default: return false;
    }
}

}  // namespace

extern "C" BearcadShape* bearcad_shape_boolean(const BearcadShape* a, const BearcadShape* b,
                                               int op) {
    if (a == nullptr || b == nullptr) {
        return nullptr;
    }
    try {
        TopoDS_Shape result;
        const bool built = run_boolean(a->shape, b->shape, op, 0.0, result);
        // A clean result stands; so does a no-op between solids that genuinely don't meet.
        if (built) {
            if (!boxes_overlap(a->shape, b->shape) ||
                !boolean_is_a_no_op(a->shape, b->shape, result, op)) {
                return new BearcadShape{result};
            }
        }
        // Retry with a widening fuzzy value (#1033). The escalation is relative to the
        // model's own size, and tops out far below anything a CAD user could see: for a
        // 200 mm part the widest attempt treats points 0.02 µm apart as coincident.
        Bnd_Box whole;
        BRepBndLib::Add(a->shape, whole);
        BRepBndLib::Add(b->shape, whole);
        double diagonal = 1.0;
        if (!whole.IsVoid()) {
            double xa, ya, za, xb, yb, zb;
            whole.Get(xa, ya, za, xb, yb, zb);
            const double dx = xb - xa;
            const double dy = yb - ya;
            const double dz = zb - za;
            diagonal = std::sqrt(dx * dx + dy * dy + dz * dz);
        }
        for (const double scale : {1e-9, 1e-8, 1e-7}) {
            TopoDS_Shape retry;
            if (!run_boolean(a->shape, b->shape, op, diagonal * scale, retry)) {
                continue;
            }
            if (!boolean_is_a_no_op(a->shape, b->shape, retry, op)) {
                return new BearcadShape{retry};
            }
        }
        // Every attempt agrees the operation changes nothing: report the first result
        // rather than failing, so a genuinely empty cut still yields its input.
        if (built) {
            return new BearcadShape{result};
        }
        return nullptr;
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

extern "C" BearcadShape* bearcad_shape_sphere(double cx, double cy, double cz,
                                              double radius) {
    if (radius <= 0.0) {
        return nullptr;
    }
    try {
        TopoDS_Shape shape = BRepPrimAPI_MakeSphere(gp_Pnt(cx, cy, cz), radius).Shape();
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

// Match each requested open face (point + normal) to a TopoDS_Face on `shape`, then hollow
// with BRepOffsetAPI_MakeThickSolid (inward offset = −thickness). Empty face list → closed shell.
extern "C" BearcadShape* bearcad_shape_shell(const BearcadShape* s, const double* faces,
                                             unsigned long n_faces, double thickness) {
    if (s == nullptr || thickness <= 0.0) {
        return nullptr;
    }
    if (n_faces > 0 && faces == nullptr) {
        return nullptr;
    }
    try {
        // Gather solid faces once for matching.
        NCollection_IndexedMap<TopoDS_Shape, TopTools_ShapeMapHasher> faceMap;
        for (TopExp_Explorer ex(s->shape, TopAbs_FACE); ex.More(); ex.Next()) {
            faceMap.Add(ex.Current());
        }
        NCollection_List<TopoDS_Shape> closing;
        // Tolerance scales with the solid's size, like edge matching.
        Bnd_Box bbox;
        BRepBndLib::Add(s->shape, bbox);
        double xmin, ymin, zmin, xmax, ymax, zmax;
        bbox.Get(xmin, ymin, zmin, xmax, ymax, zmax);
        double diag = std::sqrt((xmax - xmin) * (xmax - xmin) + (ymax - ymin) * (ymax - ymin)
                                + (zmax - zmin) * (zmax - zmin));
        double tol = std::max(1e-3, diag * 1e-3);
        double cos_tol = 0.85;  // ~32° — generous for quantized mesh normals

        for (unsigned long i = 0; i < n_faces; ++i) {
            const double* f = faces + i * 6;
            gp_Pnt want_p(f[0], f[1], f[2]);
            gp_Dir want_n(f[3], f[4], f[5]);
            bool matched = false;
            for (int k = 1; k <= faceMap.Extent(); ++k) {
                const TopoDS_Face& face = TopoDS::Face(faceMap(k));
                // Skip faces already claimed as open.
                bool already = false;
                for (NCollection_List<TopoDS_Shape>::Iterator it(closing); it.More(); it.Next()) {
                    if (it.Value().IsSame(face)) {
                        already = true;
                        break;
                    }
                }
                if (already) {
                    continue;
                }
                // Project the sample point onto the face surface; accept if near and normal agrees.
                BRepAdaptor_Surface surf(face, true);
                double u0 = surf.FirstUParameter();
                double u1 = surf.LastUParameter();
                double v0 = surf.FirstVParameter();
                double v1 = surf.LastVParameter();
                // Sample the UV mid + project via surface props at a few grid points; pick nearest.
                double best_dist2 = 1e300;
                double best_u = 0.5 * (u0 + u1);
                double best_v = 0.5 * (v0 + v1);
                const int N = 5;
                for (int iu = 0; iu < N; ++iu) {
                    for (int iv = 0; iv < N; ++iv) {
                        double u = u0 + (u1 - u0) * (iu + 0.5) / N;
                        double v = v0 + (v1 - v0) * (iv + 0.5) / N;
                        gp_Pnt p = surf.Value(u, v);
                        double d2 = p.SquareDistance(want_p);
                        if (d2 < best_dist2) {
                            best_dist2 = d2;
                            best_u = u;
                            best_v = v;
                        }
                    }
                }
                if (best_dist2 > tol * tol) {
                    continue;
                }
                BRepLProp_SLProps props(surf, best_u, best_v, 1, Precision::Confusion());
                if (!props.IsNormalDefined()) {
                    continue;
                }
                gp_Dir n = props.Normal();
                if (face.Orientation() == TopAbs_REVERSED) {
                    n.Reverse();
                }
                if (std::abs(n.Dot(want_n)) < cos_tol) {
                    continue;
                }
                closing.Append(face);
                matched = true;
                break;
            }
            if (!matched) {
                return nullptr;
            }
        }

        auto thicken = [&](const TopoDS_Shape& src, GeomAbs_JoinType join,
                           bool intersect) -> TopoDS_Shape {
            BRepOffsetAPI_MakeThickSolid maker;
            maker.MakeThickSolidByJoin(src, closing, -thickness, tol, BRepOffset_Skin, intersect,
                                       false, join);
            if (!maker.IsDone()) {
                return TopoDS_Shape();
            }
            TopoDS_Shape out = maker.Shape();
            return out.IsNull() ? TopoDS_Shape() : out;
        };
        auto heal = [](const TopoDS_Shape& src) -> TopoDS_Shape {
            ShapeFix_Shape fixer(src);
            fixer.SetPrecision(0.1);
            fixer.SetMaxTolerance(1.0);
            fixer.Perform();
            TopoDS_Shape fixed = fixer.Shape();
            if (fixed.IsNull()) {
                fixed = src;
            }
            ShapeUpgrade_UnifySameDomain unify(fixed, true, true, true);
            unify.SetLinearTolerance(0.5);
            unify.SetAngularTolerance(0.1);
            unify.Build();
            TopoDS_Shape out = unify.Shape();
            return out.IsNull() ? fixed : out;
        };

        const GeomAbs_JoinType joins[] = {GeomAbs_Intersection, GeomAbs_Arc};
        const bool intersects[] = {false, true};
        TopoDS_Shape thick;
        for (bool inter : intersects) {
            for (GeomAbs_JoinType join : joins) {
                thick = thicken(s->shape, join, inter);
                if (!thick.IsNull()) {
                    break;
                }
            }
            if (!thick.IsNull()) {
                break;
            }
        }
        if (thick.IsNull()) {
            TopoDS_Shape healed = heal(s->shape);
            for (bool inter : intersects) {
                for (GeomAbs_JoinType join : joins) {
                    thick = thicken(healed, join, inter);
                    if (!thick.IsNull()) {
                        break;
                    }
                }
                if (!thick.IsNull()) {
                    break;
                }
            }
        }
        if (thick.IsNull()) {
            try {
                BRepOffsetAPI_MakeOffsetShape offsetter;
                offsetter.PerformByJoin(s->shape, -thickness, tol);
                if (offsetter.IsDone()) {
                    thick = offsetter.Shape();
                }
            } catch (const Standard_Failure&) {
            }
        }
        if (thick.IsNull()) {
            return nullptr;
        }
        // Empty closing list: MakeThickSolidByJoin returns the *cavity* (inward offset of
        // every face) rather than the wall remainder. Subtract that cavity from the
        // original solid so the preview/result is what remains (#1163).
        TopoDS_Shape result;
        if (n_faces == 0) {
            BRepAlgoAPI_Cut cut(s->shape, thick);
            if (!cut.IsDone()) {
                return nullptr;
            }
            result = cut.Shape();
            if (result.IsNull()) {
                return nullptr;
            }
        } else {
            result = thick;
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
        // Floor linear deflection at a tiny fraction of the bbox diagonal so large
        // multi-turn helical faces don't explode into hundreds of thousands of
        // triangles under a fixed 0.05 mm chord error (#1248). Small parts keep the
        // caller's absolute deflection (the floor falls below it).
        double lin_defl = deflection > 0.0 ? deflection : 0.05;
        {
            Bnd_Box box;
            BRepBndLib::Add(s, box);
            if (!box.IsVoid()) {
                double xmin, ymin, zmin, xmax, ymax, zmax;
                box.Get(xmin, ymin, zmin, xmax, ymax, zmax);
                const double dx = xmax - xmin;
                const double dy = ymax - ymin;
                const double dz = zmax - zmin;
                const double diag = std::sqrt(dx * dx + dy * dy + dz * dz);
                // ~0.05% of diagonal: a 600 mm spring floors at ~0.3 mm.
                const double floor_defl = diag * 5.0e-4;
                if (floor_defl > lin_defl) {
                    lin_defl = floor_defl;
                }
            }
        }
        BRepMesh_IncrementalMesh mesher(s, lin_defl, false, 0.5, true);
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

extern "C" BearcadShape* bearcad_shape_clone(const BearcadShape* shape) {
    if (shape == nullptr) {
        return nullptr;
    }
    try {
        // Deep copy so tessellation / fillets on a returned clone cannot
        // mutate the memoized TShape (#1337).
        BRepBuilderAPI_Copy copy(shape->shape, true, false);
        if (!copy.IsDone()) {
            return nullptr;
        }
        return new BearcadShape{copy.Shape()};
    } catch (const Standard_Failure&) {
        return nullptr;
    } catch (...) {
        return nullptr;
    }
}

// Stamp `name` on everything a receiving CAD tool shows as the part name: the
// FILE_NAME header, each PRODUCT (name and id), its formation, and the shape
// representations. Without this OCCT leaves its own defaults ("Open CASCADE STEP
// translator ...") and the part arrives nameless (#1656). OCCT's writer escapes
// the string itself (apostrophes double), so `name` is passed through as given.
static void bearcad_step_set_name(STEPControl_Writer& writer, const char* name) {
    if (name == nullptr || *name == '\0') {
        return;
    }
    Handle(TCollection_HAsciiString) label = new TCollection_HAsciiString(name);
    Handle(StepData_StepModel) model = writer.Model();
    if (model.IsNull()) {
        return;
    }
    APIHeaderSection_MakeHeader header(model);
    header.SetName(label);
    for (int i = 1; i <= model->NbEntities(); i++) {
        Handle(Standard_Transient) ent = model->Value(i);
        if (ent.IsNull()) {
            continue;
        }
        if (Handle(StepBasic_Product) product = Handle(StepBasic_Product)::DownCast(ent)) {
            product->SetId(label);
            product->SetName(label);
        } else if (Handle(StepBasic_ProductDefinitionFormation) formation =
                       Handle(StepBasic_ProductDefinitionFormation)::DownCast(ent)) {
            formation->SetId(label);
        } else if (Handle(StepRepr_Representation) repr =
                       Handle(StepRepr_Representation)::DownCast(ent)) {
            repr->SetName(label);
        }
    }
}

extern "C" int bearcad_shape_write_step(const BearcadShape* s, const char* path, const char* name) {
    if (s == nullptr || path == nullptr) {
        return 1;
    }
    try {
        STEPControl_Writer writer;
        if (writer.Transfer(s->shape, STEPControl_AsIs) != IFSelect_RetDone) {
            return 1;
        }
        bearcad_step_set_name(writer, name);
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
