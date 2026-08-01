struct Uniforms {
    view_proj: mat4x4<f32>,
    // xyz: the scene's fixed light direction, normalized. w: unused.
    light_dir: vec4f,
    // xyz: camera eye in world space, for the view-dependent terms. w: unused.
    eye: vec4f,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// Lighting models, matching `ShadingModel` in scene.rs. Carried in normal.w.
const MODE_UNLIT: f32 = 0.0;
const MODE_LAMBERT: f32 = 1.0;
const MODE_REALISTIC: f32 = 2.0;

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
