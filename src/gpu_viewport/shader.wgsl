struct Uniforms {
    view_proj: mat4x4<f32>,
    // xyz: the scene's fixed light direction, normalized. w: unused.
    light_dir: vec4f,
    // xyz: camera eye in world space, for the view-dependent terms. w: unused.
    eye: vec4f,
    // Ground grid (#1073): x fine step, y coarse step (world mm), z how far the fine level
    // has faded in with zoom (0..1), w fade-start distance (world mm from the eye's ground
    // projection — past this the lattice softens, #1123).
    grid_steps: vec4f,
    // Line widths in **pixels**: x fine, y coarse, z the x=0/y=0 axis lines; w fade-end
    // distance (world mm) where the lattice is fully transparent (#1123).
    grid_widths: vec4f,
    grid_fine_color: vec4f,
    grid_coarse_color: vec4f,
    grid_axis_color: vec4f,
    // xy: the render target's size in pixels, for the screen-space line widening in
    // `vs_axis` (#1072). zw: unused.
    viewport_px: vec4f,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// Lighting models, matching `ShadingModel` in scene.rs. Carried in normal.w.
const MODE_UNLIT: f32 = 0.0;
const MODE_LAMBERT: f32 = 1.0;
const MODE_REALISTIC: f32 = 2.0;
const MODE_CONTACT_SHADOW: f32 = 3.0;

// Slope-scaled polygon offset for body contact shadows (#1480/#1493). Ground
// shadows stay MODE_UNLIT so they keep sitting on z = 0 with no bias.
const CONTACT_SHADOW_SLOPE: f32 = 2.0;
const CONTACT_SHADOW_BIAS: f32 = 0.0002;

// Solid-mode Lambert terms. Like the realistic weights below, these are **linear-space**
// (#1038): 0.1332 and 0.8668 re-encode to the 0.40/0.60 the sRGB-space maths used before,
// so the endpoints are unchanged and only the curve between them is now correct.
const SOLID_AMBIENT: f32 = 0.1332;
const SOLID_DIFFUSE: f32 = 0.8668;

// Realistic-mode terms, matching the REALISTIC_* constants in scene.rs. Weights are in
// linear space: 0.0707 and 0.6287 re-encode to the 0.30 floor and 0.85 ceiling the
// sRGB-space maths produced, so adopting linear lighting did not brighten every shadow.
const REALISTIC_AMBIENT: f32 = 0.0707;
const REALISTIC_DIFFUSE: f32 = 0.6287;
const REALISTIC_SPECULAR: f32 = 0.35;
const REALISTIC_SHININESS: f32 = 24.0;
const REALISTIC_HEADLIGHT: f32 = 0.70;

// The render target is a plain UNORM format, so nothing encodes for us: colours arrive
// sRGB-encoded and whatever this shader writes is what reaches the screen. Lighting has to
// decode to linear, do its arithmetic there, and re-encode (#1038).
fn srgb_to_linear(c: vec3f) -> vec3f {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3f(2.4));
    return select(hi, lo, c <= vec3f(0.04045));
}

fn linear_to_srgb(c: vec3f) -> vec3f {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3f(0.0)), vec3f(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3f(0.0031308));
}

// Narkowicz's ACES fit — a filmic shoulder, so a specular that overshoots rolls off
// instead of clipping to a flat white disc.
fn aces(x: vec3f) -> vec3f {
    return (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
}

// aces(1.0), so the curve is normalized to map full white back to full white. Without this
// the whole image would darken by ~20% purely from adopting a tonemap.
const ACES_WHITE: f32 = 0.8037036;

fn tonemap(x: vec3f) -> vec3f {
    return clamp(aces(max(x, vec3f(0.0))) / ACES_WHITE, vec3f(0.0), vec3f(1.0));
}

struct VertexInput {
    @location(0) position: vec3f,
    @location(1) color: vec4f,
    // xyz: world-space normal. w: which lighting model to apply.
    @location(2) normal: vec4f,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) color: vec4f,
    @location(1) normal: vec3f,
    @location(2) world_pos: vec3f,
    // Flat: the model is a per-vertex constant, and interpolating it would invent
    // fractional modes along every triangle edge.
    @location(3) @interpolate(flat) mode: f32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.view_proj * vec4f(input.position, 1.0);
    out.color = input.color;
    out.normal = input.normal.xyz;
    out.world_pos = input.position;
    out.mode = input.normal.w;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4f {
    // 2D chrome, lines, fills, text, gizmos: the colour is already final.
    if (input.mode < 0.5) {
        return input.color;
    }
    // Interpolating unit normals across a triangle shortens them; renormalize per pixel.
    // A degenerate normal would divide by zero, so fall back to passing the colour through.
    let len = length(input.normal);
    if (len < 1e-6) {
        return input.color;
    }
    let n = input.normal / len;
    let light = normalize(uniforms.light_dir.xyz);

    // Colours arrive premultiplied. Lighting is a property of the surface, not of how
    // transparent it is, so undo that before decoding and restore it at the end.
    let alpha = input.color.a;
    let unassociated = select(input.color.rgb / alpha, input.color.rgb, alpha <= 0.0);
    let base = srgb_to_linear(unassociated);

    if (input.mode < 1.5) {
        // Lambert (Solid mode): two-sided, so a face is lit whichever way it is wound.
        let shade = SOLID_AMBIENT + SOLID_DIFFUSE * abs(dot(n, light));
        let lit = linear_to_srgb(tonemap(base * shade));
        return vec4f(lit * alpha, alpha);
    }

    // Realistic (#83): ambient + diffuse + Blinn-Phong specular. Flip the normal toward the
    // viewer so back-facing geometry is lit rather than black.
    let view = normalize(uniforms.eye.xyz - input.world_pos);
    var nf = n;
    if (dot(nf, view) < 0.0) {
        nf = -nf;
    }
    let fixed_diffuse = max(dot(nf, light), 0.0);
    let headlight = REALISTIC_HEADLIGHT * max(dot(nf, view), 0.0);
    let diffuse = max(fixed_diffuse, headlight);
    let half_vec = normalize(light + view);
    let specular = pow(max(dot(nf, half_vec), 0.0), REALISTIC_SHININESS);
    // In linear space a highlight is light *added* to the surface, not a lerp toward white:
    // it keeps the material's colour underneath and lets the tonemap's shoulder roll off
    // the overshoot, instead of clipping to a flat white disc.
    let shaded = base * (REALISTIC_AMBIENT + REALISTIC_DIFFUSE * diffuse)
        + vec3f(REALISTIC_SPECULAR * specular);
    let lit = linear_to_srgb(tonemap(shaded));
    return vec4f(lit * alpha, alpha);
}

// Body contact shadows (#1480/#1493): unlit colour plus a slope-scaled depth
// offset so the overlay still wins on a wall viewed end-on. Ground shadows
// stay MODE_UNLIT and keep their unbiased z = 0 depth. A dedicated entry
// point so the main scene pass does not write frag_depth (and lose early-z).
struct ShadowFsOut {
    @location(0) color: vec4f,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs_contact_shadow(input: VertexOutput) -> ShadowFsOut {
    var out: ShadowFsOut;
    out.color = input.color;
    out.depth = input.clip_position.z;
    if (input.mode > 2.5) {
        let dz = max(abs(dpdx(input.clip_position.z)), abs(dpdy(input.clip_position.z)));
        out.depth = input.clip_position.z - CONTACT_SHADOW_SLOPE * dz - CONTACT_SHADOW_BIAS;
    }
    return out;
}

// ---- Screen-space lines: origin axes (#1072) and sketch strokes (#1157) ----
//
// A quad of fixed *world* width is only the right thickness at one depth: under perspective
// the near end of an axis swells and the far end thins away — and on a body face viewed at a
// grazing angle a camera-facing world ribbon reads as a freestanding 3D rectangle (#1157).
// So each corner arrives with its own world endpoint in `position`, the segment's other
// endpoint in `normal.xyz`, and a signed half-width in **pixels** in `normal.w`. Both ends
// are projected here and the corner steps sideways in screen space, which is the only place
// a pixel means anything. Depth stays on the endpoints, so a face-sketched stroke paints on
// the face rather than out of it.
//
// Ends are **round** (#1202): the quad extends past each geometric endpoint by half-width so
// the round cap has coverage, and `fs_axis` clips every fragment to a capsule of that radius.
// Square (butt) ends made coincident joints look like each line overshot the shared point.

struct AxisVertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) color: vec4f,
    // Fragment position in true screen pixels (center origin, y-up), same space as a_px/b_px.
    @location(1) pos_px: vec2f,
    // Geometric segment endpoints in that space. Flat so the whole quad shares one segment.
    @location(2) @interpolate(flat) a_px: vec2f,
    @location(3) @interpolate(flat) b_px: vec2f,
    @location(4) @interpolate(flat) half_px: f32,
}

/// Distance from `p` to the segment `a`→`b`, in the same units as the inputs.
fn dist_to_segment(p: vec2f, a: vec2f, b: vec2f) -> f32 {
    let ab = b - a;
    let denom = dot(ab, ab);
    var t = 0.0;
    if (denom > 1e-12) {
        t = clamp(dot(p - a, ab) / denom, 0.0, 1.0);
    }
    return length(p - (a + ab * t));
}

@vertex
fn vs_axis(input: VertexInput) -> AxisVertexOutput {
    var out: AxisVertexOutput;
    let own = uniforms.view_proj * vec4f(input.position, 1.0);
    let other = uniforms.view_proj * vec4f(input.normal.xyz, 1.0);
    let px = max(uniforms.viewport_px.xy, vec2f(1.0));

    // True screen pixels from the viewport centre (y-up, matching NDC). `ndc * 0.5 * px` so
    // a 1-pixel move is length 1 — the old `ndc * px` space was 2 units per pixel and made a
    // capsule SDF anisotropic. `abs(w)` keeps a behind-camera vertex's direction usable.
    let own_px = own.xy / max(abs(own.w), 1e-6) * px * 0.5;
    let other_px = other.xy / max(abs(other.w), 1e-6) * px * 0.5;

    // Canonical order so every corner of the quad writes the same flat a_px/b_px.
    if (own_px.x < other_px.x || (own_px.x == other_px.x && own_px.y <= other_px.y)) {
        out.a_px = own_px;
        out.b_px = other_px;
    } else {
        out.a_px = other_px;
        out.b_px = own_px;
    }

    var dir = other_px - own_px;
    if (length(dir) < 1e-6) {
        dir = vec2f(1.0, 0.0);
    }
    dir = normalize(dir);
    let half = input.normal.w;
    let half_abs = abs(half);
    out.half_px = half_abs;

    let side = vec2f(-dir.y, dir.x) * half;
    // Extend past the geometric endpoint so the round cap has raster coverage (#1202).
    let along = -dir * half_abs;
    let offset = side + along;
    out.pos_px = own_px + offset;

    // Back to clip space. True-pixel offset → NDC is `offset / (0.5 * px)` = `offset * 2 / px`.
    out.clip_position = vec4f(own.xy + offset / px * 2.0 * own.w, own.z, own.w);
    out.color = input.color;
    return out;
}

@fragment
fn fs_axis(input: AxisVertexOutput) -> @location(0) vec4f {
    let d = dist_to_segment(input.pos_px, input.a_px, input.b_px);
    // One-pixel AA ramp at the capsule boundary.
    let alpha = 1.0 - smoothstep(input.half_px - 0.5, input.half_px + 0.5, d);
    if (alpha <= 0.001) {
        discard;
    }
    // Premultiplied, matching the rest of the viewport pipelines.
    return vec4f(input.color.rgb * alpha, input.color.a * alpha);
}

// ---- Ground plane (#1073 / #159 / #1300 / #1301) ----
//
// Grid and solid ground share one footprint-quad path: depth-tested, never depth-writing,
// so bodies and translucent construction planes composite without coplanar z-fighting
// (#1301 — no geometric or pipeline bias). Both discard when the eye is under z = 0 so
// looking up from underneath never paints a floor through the scene (#1300).

struct GridVertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) world_xy: vec2f,
    @location(1) color: vec4f,
}

@vertex
fn vs_grid(input: VertexInput) -> GridVertexOutput {
    var out: GridVertexOutput;
    out.clip_position = uniforms.view_proj * vec4f(input.position, 1.0);
    out.world_xy = input.position.xy;
    out.color = input.color;
    return out;
}

/// Solid ground fill (#159/#1295/#1301): flat colour, hidden from below (#1300).
@fragment
fn fs_solid_ground(input: GridVertexOutput) -> @location(0) vec4f {
    if (uniforms.eye.z <= 0.0) {
        discard;
    }
    // Premultiplied, matching every other pipeline's blend state.
    return input.color;
}

/// Coverage of a lattice of lines every `step` world-mm, `width_px` pixels wide, at `p`.
/// `duv` is world-mm per pixel along each axis at this fragment.
fn lattice_coverage(p: vec2f, duv: vec2f, step: f32, width_px: f32) -> f32 {
    // Distance to the nearest multiple of `step`, per axis, in world units.
    let to_line = abs(fract(p / step + 0.5) - 0.5) * step;
    // ...expressed in pixels, so the width below means what it says at any angle.
    let px = to_line / max(duv, vec2f(1e-12));
    let half = width_px * 0.5;
    // A one-pixel ramp: full coverage inside the line, none a pixel outside it.
    let cov = vec2f(1.0) - smoothstep(vec2f(half - 0.5), vec2f(half + 0.5), px);
    return max(cov.x, cov.y);
}

/// The same for the two lines through the origin, which are single lines rather than a
/// lattice — `fract` would repeat them across the whole plane.
fn axis_coverage(p: vec2f, duv: vec2f, width_px: f32) -> f32 {
    let px = abs(p) / max(duv, vec2f(1e-12));
    let half = width_px * 0.5;
    let cov = vec2f(1.0) - smoothstep(vec2f(half - 0.5), vec2f(half + 0.5), px);
    return max(cov.x, cov.y);
}

@fragment
fn fs_grid(input: GridVertexOutput) -> @location(0) vec4f {
    // Looking up from under the ground: no lattice (#1300). Axes still draw in their own pass.
    if (uniforms.eye.z <= 0.0) {
        discard;
    }
    // World-mm per pixel along each world axis, at this fragment. Under perspective this
    // grows with distance and with grazing angle, which is exactly the correction wanted.
    let duv = fwidth(input.world_xy);

    let fine = lattice_coverage(input.world_xy, duv, uniforms.grid_steps.x, uniforms.grid_widths.x)
        * clamp(uniforms.grid_steps.z, 0.0, 1.0);
    let coarse =
        lattice_coverage(input.world_xy, duv, uniforms.grid_steps.y, uniforms.grid_widths.y);
    let axis = axis_coverage(input.world_xy, duv, uniforms.grid_widths.z);

    // Paint fine first, then coarse over it, then the origin axes on top — the same order
    // the lines used to be drawn in, so the hierarchy reads the same.
    var rgb = uniforms.grid_fine_color.rgb;
    var a = fine * uniforms.grid_fine_color.a;
    rgb = mix(rgb, uniforms.grid_coarse_color.rgb, coarse);
    a = mix(a, uniforms.grid_coarse_color.a, coarse);
    // Origin axes stay fully opaque (orientation never softens away); only the lattice fades.
    let axis_a = axis * uniforms.grid_axis_color.a;
    rgb = mix(rgb, uniforms.grid_axis_color.rgb, axis);
    a = mix(a, axis_a, axis);

    // Distance fade (#1123): as the lattice recedes from the eye's ground projection it
    // softens out instead of popping at a hard footprint edge when the camera orbits.
    // fade_start / fade_end are world mm (grid_steps.w / grid_widths.w). Origin axes skip
    // this ramp so the triad always reads.
    let fade_start = uniforms.grid_steps.w;
    let fade_end = uniforms.grid_widths.w;
    if (fade_end > fade_start && axis < 0.01) {
        let eye_ground = uniforms.eye.xy;
        let horiz = length(input.world_xy - eye_ground);
        let dist_fade = 1.0 - smoothstep(fade_start, fade_end, horiz);
        a *= dist_fade;
    }

    if (a <= 0.0) {
        discard;
    }
    // Premultiplied, matching every other pipeline's blend state.
    return vec4f(rgb * a, a);
}

struct BlitVertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs_blit(@builtin(vertex_index) vertex_index: u32) -> BlitVertexOutput {
    var positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f(3.0, -1.0),
        vec2f(-1.0, 3.0),
    );
    var out: BlitVertexOutput;
    let pos = positions[vertex_index];
    out.position = vec4f(pos, 0.0, 1.0);
    out.uv = pos * vec2f(0.5, -0.5) + vec2f(0.5, 0.5);
    return out;
}

@group(0) @binding(0) var scene_texture: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

@fragment
fn fs_blit(input: BlitVertexOutput) -> @location(0) vec4f {
    return textureSample(scene_texture, scene_sampler, input.uv);
}

// ---- Body-highlight outline (#1110) ----
//
// Selected/hovered body triangles are drawn flat into an offscreen mask (R = selected,
// G = hovered). This fullscreen pass dilates that mask in screen space and strokes a
// 9px-wide silhouette band offset 5px outside the body — blue for selected, yellow for
// hovered — so the highlight reads as an outline on the flattened camera-plane view
// rather than a fill recolour.
//
// Reuses the blit pipeline's group-0 bindings (`scene_texture` / `scene_sampler`): the
// outline pipeline has the same layout, and at draw time the bind group points at the
// mask texture instead of the resolved scene.

// Max of the mask's R/G channels inside a disc of `radius_px` around `uv`.
fn mask_max_in_radius(uv: vec2f, radius_px: f32) -> vec2f {
    let dims = vec2f(textureDimensions(scene_texture));
    let texel = 1.0 / max(dims, vec2f(1.0));
    var acc = vec2f(0.0);
    let r = i32(ceil(radius_px));
    for (var dy = -r; dy <= r; dy = dy + 1) {
        for (var dx = -r; dx <= r; dx = dx + 1) {
            let d = length(vec2f(f32(dx), f32(dy)));
            if (d > radius_px + 0.5) {
                continue;
            }
            let s = textureSampleLevel(
                scene_texture,
                scene_sampler,
                uv + vec2f(f32(dx), f32(dy)) * texel,
                0.0,
            ).rg;
            acc = max(acc, s);
        }
    }
    return acc;
}

@fragment
fn fs_outline(input: BlitVertexOutput) -> @location(0) vec4f {
    // Outer edge of the outline sits 14px outside the silhouette (5px offset + 9px width);
    // the inner edge sits 5px outside, so the band itself is 9px thick with a 5px gap
    // from the body.
    let outer = mask_max_in_radius(input.uv, 14.0);
    let inner = mask_max_in_radius(input.uv, 5.0);
    let band = clamp(outer - inner, vec2f(0.0), vec2f(1.0));

    // BODY_SILHOUETTE_COLOR / PICK_HOVER yellow — keep them matching the app's existing
    // selection/hover hues so the outline reads as the same highlight language as shading.
    let sel_rgb = vec3f(95.0 / 255.0, 165.0 / 255.0, 245.0 / 255.0);
    let hov_rgb = vec3f(255.0 / 255.0, 210.0 / 255.0, 90.0 / 255.0);

    var rgb = vec3f(0.0);
    var a = 0.0;
    // Hover first, then selected on top when both channels are live (e.g. two bodies
    // whose silhouettes cross).
    if (band.g > 0.01) {
        rgb = hov_rgb;
        a = band.g;
    }
    if (band.r > 0.01) {
        rgb = sel_rgb;
        a = band.r;
    }
    if (a <= 0.0) {
        discard;
    }
    // Premultiplied, matching every other pipeline's blend state.
    return vec4f(rgb * a, a);
}

struct TextVertexInput {
    @location(0) position: vec3f,
    @location(1) uv: vec2f,
    @location(2) color: vec4f,
}

struct TextVertexOutput {
    @builtin(position) clip_position: vec4f,
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
}

@vertex
fn vs_text(input: TextVertexInput) -> TextVertexOutput {
    var out: TextVertexOutput;
    out.clip_position = uniforms.view_proj * vec4f(input.position, 1.0);
    out.uv = input.uv;
    out.color = input.color;
    return out;
}

@group(1) @binding(0) var font_texture: texture_2d<f32>;
@group(1) @binding(1) var font_sampler: sampler;

@fragment
fn fs_text(input: TextVertexOutput) -> @location(0) vec4f {
    let glyph = textureSample(font_texture, font_sampler, input.uv);
    return vec4f(input.color.rgb * glyph.a, input.color.a * glyph.a);
}

// Tracing images (#170): full-color textured quads on construction planes. Reuses the text
// vertex layout; the vertex color's alpha carries the image opacity.
@fragment
fn fs_image(input: TextVertexOutput) -> @location(0) vec4f {
    let texel = textureSample(font_texture, font_sampler, input.uv);
    let alpha = texel.a * input.color.a;
    return vec4f(texel.rgb * alpha, alpha);
}
