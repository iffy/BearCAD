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

// Realistic-mode terms, matching the REALISTIC_* constants in scene.rs.
const REALISTIC_AMBIENT: f32 = 0.30;
const REALISTIC_DIFFUSE: f32 = 0.55;
const REALISTIC_SPECULAR: f32 = 0.35;
const REALISTIC_SHININESS: f32 = 24.0;
const REALISTIC_HEADLIGHT: f32 = 0.70;

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

    if (input.mode < 1.5) {
        // Lambert (Solid mode): two-sided, so a face is lit whichever way it is wound.
        let shade = 0.4 + 0.6 * abs(dot(n, light));
        return vec4f(input.color.rgb * shade, input.color.a);
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
    let intensity = min(REALISTIC_AMBIENT + REALISTIC_DIFFUSE * diffuse, 1.0);
    // Lighten toward white by the specular amount, matching `lighten_color` on the CPU.
    let lift = REALISTIC_SPECULAR * specular;
    let shaded = input.color.rgb * intensity;
    return vec4f(shaded + (vec3f(input.color.a) - shaded) * lift, input.color.a);
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
