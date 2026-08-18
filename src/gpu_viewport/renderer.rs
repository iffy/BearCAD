//! wgpu offscreen renderer and egui paint callback.

use super::dim_labels::GpuTextVertex;
use super::scene::{GpuVertex, ViewportScene};
use eframe::egui_wgpu::wgpu::util::DeviceExt as _;
use eframe::egui_wgpu::{self, wgpu};
use egui::Rect;
use glam::Mat4;
use std::num::NonZeroU64;
use std::sync::Mutex;

/// Preferred MSAA sample count for viewport line/edge anti-aliasing.
pub const VIEWPORT_MSAA_SAMPLES: u32 = 4;

/// Depth-stencil format for the viewport. Needs a stencil aspect so coplanar
/// sketch fills can be masked to paint each pixel once (#3).
pub const VIEWPORT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// Directional shadow map for realistic-mode sun occlusion (#1535).
const SHADOW_MAP_SIZE: u32 = 2048;
const SHADOW_MAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuUniforms {
    view_proj: [[f32; 4]; 4],
    /// Directional light view-projection for the shadow map (#1535).
    light_view_proj: [[f32; 4]; 4],
    /// Scene light direction (xyz, normalized); `w` is padding for std140 alignment.
    light_dir: [f32; 4],
    /// Camera eye in world space (xyz), for the view-dependent lighting terms in
    /// `fs_main`; `w` is padding.
    eye: [f32; 4],
    /// Ground grid (#1073): fine step, coarse step, fine-level fade, distance-fade start.
    grid_steps: [f32; 4],
    /// Grid line widths in pixels: fine, coarse, origin axes; w = distance-fade end (#1123).
    grid_widths: [f32; 4],
    grid_fine_color: [f32; 4],
    grid_coarse_color: [f32; 4],
    grid_axis_color: [f32; 4],
    /// Render-target size in pixels (xy), for the screen-space line widening in `vs_axis`
    /// (#1072); zw is padding.
    viewport_px: [f32; 4],
}

impl GpuUniforms {
    /// The grid fields for a frame that is not showing one — zero width, so even a stray
    /// draw paints nothing.
    const NO_GRID: ([f32; 4], [f32; 4]) = ([1.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]);
}

/// Vertex layout shared by every scene-geometry pipeline: position, colour, and the
/// normal + lighting-model pair the fragment shader lights with (#1037).
const SCENE_VERTEX_ATTRS: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 28,
        shader_location: 2,
    },
];

pub struct ViewportGpuResources {
    target_format: wgpu::TextureFormat,
    msaa_sample_count: u32,
    scene_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    /// Depth-test-disabled pipeline so gizmo handles stay visible through bodies (#36).
    gizmo_pipeline: wgpu::RenderPipeline,
    /// Stencil-masked pipeline for coplanar sketch fills: each pixel is painted
    /// exactly once so translucent overlaps don't double-blend (#3).
    sketch_fill_pipeline: wgpu::RenderPipeline,
    /// Contact shadows on the ground (#1041): the same single-paint trick on its
    /// own stencil bit, depth-tested but never depth-writing.
    ground_shadow_pipeline: wgpu::RenderPipeline,
    /// Depth-only pass from the directional light (#1535).
    shadow_pipeline: wgpu::RenderPipeline,
    scene_transparent_pipeline: wgpu::RenderPipeline,
    /// The ground grid (#1073): one footprint quad whose fragment shader draws the lattice
    /// in pixel-measured widths. Depth-tested so bodies occlude it, but never depth-writing
    /// — the gaps between lines must not hide anything below z = 0.
    grid_pipeline: wgpu::RenderPipeline,
    /// Solid ground fill (#159/#1295/#1301): same no-depth-write footprint pass as the grid,
    /// flat colour. Keeps coplanar construction planes from z-fighting without bias.
    solid_ground_pipeline: wgpu::RenderPipeline,
    /// Shared footprint quad for grid / solid ground — rewritten each frame the footprint
    /// moves — and its two triangles, which never change.
    grid_vertex_buffer: wgpu::Buffer,
    grid_index_buffer: wgpu::Buffer,
    /// The origin axes (#1072) and screen-space sketch strokes (#1157), whose vertices carry
    /// both endpoints and a pixel half-width (`vs_axis`).
    axis_pipeline: wgpu::RenderPipeline,
    axis_vertex_buffer: wgpu::Buffer,
    axis_index_buffer: wgpu::Buffer,
    axis_vertex_capacity: u64,
    axis_index_capacity: u64,
    /// Sketch / overlay strokes (#1157): same packing as axes, drawn after the opaque base.
    stroke_vertex_buffer: wgpu::Buffer,
    stroke_index_buffer: wgpu::Buffer,
    stroke_vertex_capacity: u64,
    stroke_index_capacity: u64,
    text_pipeline: wgpu::RenderPipeline,
    /// Tracing-image quads (#170): the text pipeline's layout with a full-color fragment.
    image_pipeline: wgpu::RenderPipeline,
    /// Texture + bind group per tracing image, keyed by the scene's content id (#170).
    image_textures: Mutex<std::collections::HashMap<u64, wgpu::BindGroup>>,
    blit_pipeline: wgpu::RenderPipeline,
    /// Body-highlight outline mask pass (#1110): draws selected/hovered body triangles
    /// flat (unlit, no depth) into an offscreen R/G mask.
    mask_pipeline: wgpu::RenderPipeline,
    /// Fullscreen pass that dilates the outline mask and strokes the silhouette band
    /// onto the resolved scene colour (#1110).
    outline_pipeline: wgpu::RenderPipeline,
    /// Bind-group layout shared by the blit and outline pipelines (texture + sampler).
    blit_bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    /// Uniforms only — the light-depth pass cannot bind the shadow map it writes (#1535).
    shadow_bind_group: wgpu::BindGroup,
    text_texture_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    font_sampler: wgpu::Sampler,
    /// Single-sample resolve target sampled by the blit pass.
    color_texture: Option<wgpu::Texture>,
    color_view: Option<wgpu::TextureView>,
    msaa_color_texture: Option<wgpu::Texture>,
    msaa_color_view: Option<wgpu::TextureView>,
    depth_texture: Option<wgpu::Texture>,
    depth_view: Option<wgpu::TextureView>,
    /// Directional shadow map (#1535): depth from the light, sampled in `fs_main`.
    /// Texture/sampler are owned so the view and bind group stay valid.
    #[allow(dead_code)]
    shadow_texture: wgpu::Texture,
    shadow_view: wgpu::TextureView,
    #[allow(dead_code)]
    shadow_sampler: wgpu::Sampler,
    /// Offscreen R/G mask of selected/hovered body silhouettes (#1110).
    mask_texture: Option<wgpu::Texture>,
    mask_view: Option<wgpu::TextureView>,
    blit_bind_group: Option<wgpu::BindGroup>,
    /// Bind group pointing the outline pipeline at the current mask texture (#1110).
    outline_bind_group: Option<wgpu::BindGroup>,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    text_vertex_buffer: wgpu::Buffer,
    text_index_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    index_capacity: u64,
    text_vertex_capacity: u64,
    text_index_capacity: u64,
    texture_size: [u32; 2],
    pending_scene: Mutex<Option<ViewportScene>>,
    font_bind_group: Mutex<Option<wgpu::BindGroup>>,
}

/// Pick the highest MSAA count supported by the device, capped at [`VIEWPORT_MSAA_SAMPLES`].
pub fn clamp_msaa_sample_count(max_supported: u32) -> u32 {
    if max_supported >= VIEWPORT_MSAA_SAMPLES {
        VIEWPORT_MSAA_SAMPLES
    } else if max_supported >= 2 {
        2
    } else {
        1
    }
}

/// Pick the MSAA sample count for a render target format, or `1` when resolve is unsupported.
pub fn msaa_sample_count_for_format(features: &wgpu::TextureFormatFeatures) -> u32 {
    if !features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
    {
        return 1;
    }
    let max_supported = features
        .flags
        .supported_sample_counts()
        .into_iter()
        .max()
        .unwrap_or(1);
    clamp_msaa_sample_count(max_supported)
}

fn multisample_state(sample_count: u32) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count: sample_count,
        mask: !0,
        // MSAA resolve still anti-aliases opaque line quads; alpha-to-coverage
        // thins semi-transparent face fills to near-invisibility on dark backgrounds.
        alpha_to_coverage_enabled: false,
    }
}

impl ViewportGpuResources {
    pub fn install(render_state: &egui_wgpu::RenderState) -> Self {
        let device = &render_state.device;
        let target_format = render_state.target_format;
        let format_features = render_state
            .adapter
            .get_texture_format_features(target_format);
        let msaa_sample_count = msaa_sample_count_for_format(&format_features);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bearcad_viewport_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<GpuUniforms>() as u64),
            },
            count: None,
        };
        let shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bearcad_viewport_shadow_uniform_layout"),
                entries: &[uniform_entry],
            });
        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bearcad_viewport_uniform_layout"),
                entries: &[
                    uniform_entry,
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                ],
            });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bearcad_viewport_uniform"),
            contents: bytemuck::bytes_of(&GpuUniforms {
                view_proj: Mat4::IDENTITY.to_cols_array_2d(),
                light_view_proj: Mat4::IDENTITY.to_cols_array_2d(),
                light_dir: [0.0, 0.0, 1.0, 0.0],
                eye: [0.0, 0.0, 0.0, 0.0],
                grid_steps: GpuUniforms::NO_GRID.0,
                grid_widths: GpuUniforms::NO_GRID.1,
                grid_fine_color: [0.0; 4],
                grid_coarse_color: [0.0; 4],
                grid_axis_color: [0.0; 4],
                viewport_px: [1.0, 1.0, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bearcad_viewport_shadow_map"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_MAP_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bearcad_viewport_shadow_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bearcad_viewport_uniform_bind_group"),
            layout: &uniform_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bearcad_viewport_shadow_bind_group"),
            layout: &shadow_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let scene_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bearcad_viewport_scene_layout"),
                bind_group_layouts: &[Some(&uniform_bind_group_layout)],
                immediate_size: 0,
            });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bearcad_viewport_shadow_layout"),
                bind_group_layouts: &[Some(&shadow_bind_group_layout)],
                immediate_size: 0,
            });

        let scene_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_scene_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SCENE_VERTEX_ATTRS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        // Overlay pipeline: same as the scene pipeline but biased toward the camera with
        // a slope-scaled term, so hover fills and stroke overlays win against the faces
        // and fills they sit on at any viewing angle (see the sketch-fill bias above).
        let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_overlay_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SCENE_VERTEX_ATTRS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: -9,
                    slope_scale: -2.0,
                    clamp: 0.0,
                },
            }),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        // Gizmo pipeline: same as the scene pipeline but with the depth test disabled
        // (compare Always) and no depth writes, so manipulation handles drawn last stay
        // visible even when a body is in front of them (#36).
        let gizmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_gizmo_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SCENE_VERTEX_ATTRS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        // Coplanar sketch fills: keep the depth test, but use the stencil buffer
        // so that the first fill to cover a pixel paints it (stencil 0 -> 1) and
        // any later coplanar fill at that pixel is rejected (stencil != 0). This
        // prevents translucent overlap regions from being alpha-blended twice,
        // which previously made overlaps render darker (#3).
        let sketch_fill_stencil = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Equal,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op: wgpu::StencilOperation::IncrementClamp,
        };
        // Contact shadows (#1041) paint each pixel once for the same reason sketch fills do:
        // a body's silhouette overlaps itself, and translucent overlaps blend twice into
        // blotches. Bit 1 of the stencil, so the two single-paint passes stay independent —
        // pass where the bit is clear, and set it.
        let shadow_stencil = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::NotEqual,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op: wgpu::StencilOperation::Replace,
        };
        let ground_shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_ground_shadow_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SCENE_VERTEX_ATTRS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_contact_shadow"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                // A shadow lies *on* the ground: depth-tested so a body in front of it wins,
                // never depth-writing, because it occludes nothing.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState {
                    front: shadow_stencil,
                    back: shadow_stencil,
                    read_mask: 0b10,
                    write_mask: 0b10,
                },
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        // Depth-only from the light (#1535). Slope-scaled bias on the caster
        // plus a small comparison bias in `fs_main` keep self-shadows clean.
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_shadow_pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SCENE_VERTEX_ATTRS,
                }],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_MAP_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sketch_fill_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_sketch_fill_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SCENE_VERTEX_ATTRS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState {
                    front: sketch_fill_stencil,
                    back: sketch_fill_stencil,
                    // Bit 0 only. Contact shadows (#1041) run their own single-paint pass in
                    // the same render pass and so share the stencil buffer; giving each its
                    // own bit is what stops one pass's marks rejecting the other's fragments.
                    read_mask: 0b01,
                    write_mask: 0b01,
                },
                // Slope-scaled bias toward the camera: sketch fills are decals on the
                // face beneath them, and the fixed millimetre world-space lifts alone
                // collapse under glancing-angle depth interpolation on long thin faces
                // (stippled z-fighting). The slope term grows the bias exactly where the
                // depth gradient does.
                bias: wgpu::DepthBiasState {
                    constant: -8,
                    slope_scale: -2.0,
                    clamp: 0.0,
                },
            }),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        let scene_transparent_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bearcad_viewport_scene_transparent_pipeline"),
                layout: Some(&scene_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<GpuVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &SCENE_VERTEX_ATTRS,
                    }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: VIEWPORT_DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    // No bias of any kind (#1088/#1121): world-space lifts and GPU/frag-depth
                    // nudges both make a coplanar plane read as cutting through bodies.
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: multisample_state(msaa_sample_count),
                multiview_mask: None,
                cache: None,
            });

        let text_texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bearcad_viewport_text_texture_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let text_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bearcad_viewport_text_layout"),
                bind_group_layouts: &[Some(&uniform_bind_group_layout), Some(&text_texture_bind_group_layout)],
                immediate_size: 0,
            });

        let text_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_text_pipeline"),
            layout: Some(&text_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_text"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuTextVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 20,
                            shader_location: 2,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_text"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // Dimension labels only appear in sketch mode and must show through bodies
            // (#1280) — depth Always, no depth write (same always-on-top idea as gizmos).
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        // Tracing-image pipeline (#170): textured world quads. Depth-test on, write off —
        // bodies in front occlude images while images never occlude anything themselves.
        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_image_pipeline"),
            layout: Some(&text_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_text"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuTextVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 20,
                            shader_location: 2,
                        },
                    ],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_image"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bearcad_viewport_blit_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let blit_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bearcad_viewport_blit_layout"),
                bind_group_layouts: &[Some(&blit_bind_group_layout)],
                immediate_size: 0,
            });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_blit_pipeline"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_blit"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_blit"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Outline mask (#1110): selected/hovered body triangles, unlit, into an R/G
        // offscreen target. No depth — the silhouette is the flattened camera-plane
        // projection of the whole body, not its front faces alone.
        let mask_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_outline_mask_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SCENE_VERTEX_ATTRS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    // Rgba8Unorm so a later fullscreen pass can sample R/G as the
                    // selected/hovered channels without an sRGB decode.
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Max,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Max,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Outline stroke (#1110): dilate the mask and paint the silhouette band over the
        // resolved scene colour. Same bind-group layout as blit (texture + sampler);
        // at draw time the bind group points at the mask, not the scene.
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_outline_pipeline"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_blit"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_outline"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Ground footprint pipelines (#1073 / #159 / #1301). Alpha-blended and depth-tested
        // but not depth-writing: transparent gaps (grid) and coplanar plane fills (solid)
        // must not fight bodies or each other for depth. Hidden from below in the shader
        // (#1300).
        let ground_depth_stencil = wgpu::DepthStencilState {
            format: VIEWPORT_DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let ground_vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &SCENE_VERTEX_ATTRS,
        }];
        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_grid_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_grid"),
                buffers: &ground_vertex_buffers,
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_grid"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(ground_depth_stencil.clone()),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });
        let solid_ground_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_solid_ground_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_grid"),
                buffers: &ground_vertex_buffers,
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_solid_ground"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(ground_depth_stencil),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        // Origin-axis + sketch-stroke pipeline (#1072 / #1157): `vs_axis` widens in screen
        // space; `fs_axis` clips to a round-capped capsule so coincident joints don't show
        // square-end overshoot (#1202).
        let axis_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bearcad_viewport_axis_pipeline"),
            layout: Some(&scene_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_axis"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SCENE_VERTEX_ATTRS,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_axis"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: VIEWPORT_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: multisample_state(msaa_sample_count),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bearcad_viewport_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let font_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bearcad_viewport_font_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_indices"),
            size: 4096,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_grid_vertices"),
            size: (4 * std::mem::size_of::<GpuVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let axis_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_axis_vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let axis_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_axis_indices"),
            size: 4096,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let stroke_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_stroke_vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let stroke_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_stroke_indices"),
            size: 4096,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bearcad_viewport_grid_indices"),
            contents: bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            usage: wgpu::BufferUsages::INDEX,
        });
        let text_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_text_vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let text_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bearcad_viewport_text_indices"),
            size: 4096,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            target_format,
            msaa_sample_count,
            scene_pipeline,
            overlay_pipeline,
            gizmo_pipeline,
            sketch_fill_pipeline,
            ground_shadow_pipeline,
            shadow_pipeline,
            scene_transparent_pipeline,
            grid_pipeline,
            solid_ground_pipeline,
            grid_vertex_buffer,
            grid_index_buffer,
            axis_pipeline,
            axis_vertex_buffer,
            axis_index_buffer,
            axis_vertex_capacity: 4096,
            axis_index_capacity: 4096,
            stroke_vertex_buffer,
            stroke_index_buffer,
            stroke_vertex_capacity: 4096,
            stroke_index_capacity: 4096,
            text_pipeline,
            image_pipeline,
            image_textures: Mutex::new(std::collections::HashMap::new()),
            blit_pipeline,
            mask_pipeline,
            outline_pipeline,
            blit_bind_group_layout,
            uniform_buffer,
            uniform_bind_group,
            shadow_bind_group,
            text_texture_bind_group_layout,
            sampler,
            font_sampler,
            color_texture: None,
            color_view: None,
            msaa_color_texture: None,
            msaa_color_view: None,
            depth_texture: None,
            depth_view: None,
            shadow_texture,
            shadow_view,
            shadow_sampler,
            mask_texture: None,
            mask_view: None,
            blit_bind_group: None,
            outline_bind_group: None,
            vertex_buffer,
            index_buffer,
            text_vertex_buffer,
            text_index_buffer,
            vertex_capacity: 4096,
            index_capacity: 4096,
            text_vertex_capacity: 4096,
            text_index_capacity: 4096,
            texture_size: [0, 0],
            pending_scene: Mutex::new(None),
            font_bind_group: Mutex::new(None),
        }
    }

    fn update_font_bind_group(
        &self,
        device: &wgpu::Device,
        render_state: &egui_wgpu::RenderState,
    ) {
        let renderer = render_state.renderer.read();
        let Some(tex) = renderer.texture(&egui::TextureId::default()) else {
            return;
        };
        let Some(wgpu_tex) = tex.texture.as_ref() else {
            return;
        };
        let view = wgpu_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bearcad_viewport_font_bind_group"),
            layout: &self.text_texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.font_sampler),
                },
            ],
        });
        *self.font_bind_group.lock().unwrap() = Some(bind_group);
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.texture_size == [width, height] {
            return;
        }
        self.texture_size = [width, height];

        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let color_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bearcad_viewport_color_resolve"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let color_view = color_texture.create_view(&Default::default());

        let (msaa_color_texture, msaa_color_view) = if self.msaa_sample_count > 1 {
            let msaa_color_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bearcad_viewport_color_msaa"),
                size: extent,
                mip_level_count: 1,
                sample_count: self.msaa_sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: self.target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let msaa_color_view =
                msaa_color_texture.create_view(&wgpu::TextureViewDescriptor::default());
            (Some(msaa_color_texture), Some(msaa_color_view))
        } else {
            (None, None)
        };

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bearcad_viewport_depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: self.msaa_sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: VIEWPORT_DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_texture.create_view(&Default::default());

        // Outline mask (#1110): single-sample R/G target matching the viewport size.
        // TEXTURE_BINDING so the outline pass can sample it; RENDER_ATTACHMENT so the
        // mask pass can write selected/hovered body triangles into it.
        let mask_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bearcad_viewport_outline_mask"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let mask_view = mask_texture.create_view(&Default::default());

        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bearcad_viewport_blit_bind_group"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let outline_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bearcad_viewport_outline_bind_group"),
            layout: &self.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&mask_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    // Same filtering sampler as blit: the dilate samples land on texel
                    // centres (integer pixel offsets), so linear equals nearest there.
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        self.color_texture = Some(color_texture);
        self.color_view = Some(color_view);
        self.msaa_color_texture = msaa_color_texture;
        self.msaa_color_view = msaa_color_view;
        self.depth_texture = Some(depth_texture);
        self.depth_view = Some(depth_view);
        self.mask_texture = Some(mask_texture);
        self.mask_view = Some(mask_view);
        self.blit_bind_group = Some(blit_bind_group);
        self.outline_bind_group = Some(outline_bind_group);
    }

    fn ensure_text_buffer_capacity(
        &mut self,
        device: &wgpu::Device,
        vertex_bytes: u64,
        index_bytes: u64,
    ) {
        if vertex_bytes > self.text_vertex_capacity {
            self.text_vertex_capacity = vertex_bytes.next_power_of_two().max(4096);
            self.text_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bearcad_viewport_text_vertices"),
                size: self.text_vertex_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if index_bytes > self.text_index_capacity {
            self.text_index_capacity = index_bytes.next_power_of_two().max(4096);
            self.text_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bearcad_viewport_text_indices"),
                size: self.text_index_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }

    fn ensure_buffer_capacity(
        &mut self,
        device: &wgpu::Device,
        vertex_bytes: u64,
        index_bytes: u64,
    ) {
        if vertex_bytes > self.vertex_capacity {
            self.vertex_capacity = vertex_bytes.next_power_of_two().max(4096);
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bearcad_viewport_vertices"),
                size: self.vertex_capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if index_bytes > self.index_capacity {
            self.index_capacity = index_bytes.next_power_of_two().max(4096);
            self.index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bearcad_viewport_indices"),
                size: self.index_capacity,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }

    fn render_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &ViewportScene,
        width: u32,
        height: u32,
    ) {
        self.ensure_targets(device, width, height);
        if width == 0 || height == 0 {
            return;
        }

        let vertex_bytes = (scene.vertices.len() * std::mem::size_of::<GpuVertex>()) as u64;
        let base_index_count = scene.indices.len();
        // Contact shadows (#1041/#1461) sit between the opaque scene and the coplanar fills:
        // after the surfaces they lie on, before the decals that sit on top of everything.
        let shadow_index_count = scene.shadow_indices.len();
        let sketch_fill_index_count = scene.sketch_fill_indices.len();
        let plane_fill_index_count = scene.plane_fill_indices.len();
        // Solid faces coplanar with a construction plane, re-drawn after plane fills (#1215).
        let body_over_plane_index_count = scene.body_over_plane_indices.len();
        let overlay_index_count = scene.overlay_indices.len();
        let gizmo_index_count = scene.gizmo_indices.len();
        // Body edge-wireframe overlay (#33). Same depth-disabled pipeline as gizmos (both
        // need to stay visible "through" bodies), so it shares the final draw call below
        // rather than getting a dedicated pipeline and boundary.
        let wireframe_index_count = scene.wireframe_indices.len();
        // Outline mask indices (#1110) ride at the end of the combined index buffer and are
        // drawn in a separate pass into the offscreen R/G mask — never the main colour target.
        let mask_index_count = scene.mask_indices.len();
        let shadow_caster_index_count = scene.shadow_caster_indices.len();
        let scene_index_count = base_index_count
            + shadow_index_count
            + sketch_fill_index_count
            + plane_fill_index_count
            + body_over_plane_index_count
            + overlay_index_count
            + gizmo_index_count
            + wireframe_index_count;
        let total_index_count =
            scene_index_count + mask_index_count + shadow_caster_index_count;
        let index_bytes = (total_index_count * std::mem::size_of::<u32>()) as u64;
        let text_vertex_bytes =
            (scene.text_vertices.len() * std::mem::size_of::<GpuTextVertex>()) as u64;
        let text_index_bytes = (scene.text_indices.len() * std::mem::size_of::<u32>()) as u64;
        self.ensure_buffer_capacity(device, vertex_bytes, index_bytes);
        self.ensure_text_buffer_capacity(device, text_vertex_bytes, text_index_bytes);

        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&GpuUniforms {
                view_proj: scene.view_proj.to_cols_array_2d(),
                light_view_proj: scene.light_view_proj.to_cols_array_2d(),
                light_dir: {
                    let l = super::scene::SCENE_LIGHT_DIR.normalize_or_zero();
                    [l.x, l.y, l.z, 0.0]
                },
                eye: [scene.eye.x, scene.eye.y, scene.eye.z, 0.0],
                grid_steps: match &scene.grid {
                    Some(g) => [g.fine_step, g.coarse_step, g.fine_fade, g.fade_start_mm],
                    None => GpuUniforms::NO_GRID.0,
                },
                grid_widths: match &scene.grid {
                    Some(g) => [
                        g.fine_width_px,
                        g.coarse_width_px,
                        g.axis_width_px,
                        g.fade_end_mm,
                    ],
                    None => GpuUniforms::NO_GRID.1,
                },
                grid_fine_color: scene.grid.map(|g| g.fine_color).unwrap_or_default(),
                grid_coarse_color: scene.grid.map(|g| g.coarse_color).unwrap_or_default(),
                grid_axis_color: scene.grid.map(|g| g.axis_color).unwrap_or_default(),
                viewport_px: [width.max(1) as f32, height.max(1) as f32, 0.0, 0.0],
            }),
        );
        if !scene.vertices.is_empty() {
            queue.write_buffer(
                &self.vertex_buffer,
                0,
                bytemuck::cast_slice(&scene.vertices),
            );
        }
        if total_index_count > 0 {
            let mut combined_indices = Vec::with_capacity(total_index_count);
            combined_indices.extend_from_slice(&scene.indices);
            combined_indices.extend_from_slice(&scene.shadow_indices);
            combined_indices.extend_from_slice(&scene.sketch_fill_indices);
            combined_indices.extend_from_slice(&scene.plane_fill_indices);
            combined_indices.extend_from_slice(&scene.body_over_plane_indices);
            combined_indices.extend_from_slice(&scene.overlay_indices);
            combined_indices.extend_from_slice(&scene.gizmo_indices);
            combined_indices.extend_from_slice(&scene.wireframe_indices);
            combined_indices.extend_from_slice(&scene.mask_indices);
            combined_indices.extend_from_slice(&scene.shadow_caster_indices);
            queue.write_buffer(
                &self.index_buffer,
                0,
                bytemuck::cast_slice(&combined_indices),
            );
        }
        // The origin axes (#1072): a handful of vertices, but they grow if anything else
        // ever uses the screen-widened line, so the buffers resize like the scene's.
        if !scene.axis_vertices.is_empty() {
            let bytes = (scene.axis_vertices.len() * std::mem::size_of::<GpuVertex>()) as u64;
            if bytes > self.axis_vertex_capacity {
                self.axis_vertex_capacity = bytes.next_power_of_two().max(4096);
                self.axis_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bearcad_viewport_axis_vertices"),
                    size: self.axis_vertex_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            let index_bytes = (scene.axis_indices.len() * std::mem::size_of::<u32>()) as u64;
            if index_bytes > self.axis_index_capacity {
                self.axis_index_capacity = index_bytes.next_power_of_two().max(4096);
                self.axis_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bearcad_viewport_axis_indices"),
                    size: self.axis_index_capacity,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(
                &self.axis_vertex_buffer,
                0,
                bytemuck::cast_slice(&scene.axis_vertices),
            );
            queue.write_buffer(
                &self.axis_index_buffer,
                0,
                bytemuck::cast_slice(&scene.axis_indices),
            );
        }
        // Screen-space sketch / overlay strokes (#1157): same packing as origin axes.
        if !scene.stroke_vertices.is_empty() {
            let bytes = (scene.stroke_vertices.len() * std::mem::size_of::<GpuVertex>()) as u64;
            if bytes > self.stroke_vertex_capacity {
                self.stroke_vertex_capacity = bytes.next_power_of_two().max(4096);
                self.stroke_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bearcad_viewport_stroke_vertices"),
                    size: self.stroke_vertex_capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            let index_bytes = (scene.stroke_indices.len() * std::mem::size_of::<u32>()) as u64;
            if index_bytes > self.stroke_index_capacity {
                self.stroke_index_capacity = index_bytes.next_power_of_two().max(4096);
                self.stroke_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bearcad_viewport_stroke_indices"),
                    size: self.stroke_index_capacity,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(
                &self.stroke_vertex_buffer,
                0,
                bytemuck::cast_slice(&scene.stroke_vertices),
            );
            queue.write_buffer(
                &self.stroke_index_buffer,
                0,
                bytemuck::cast_slice(&scene.stroke_indices),
            );
        }
        // Footprint corners for grid (#1073) or solid ground (#159/#1301). Solid ground
        // carries its fill colour in the vertex; the grid lattice reads colour from uniforms.
        if let Some(grid) = &scene.grid {
            let quad: [GpuVertex; 4] = std::array::from_fn(|i| GpuVertex {
                position: grid.corners[i].to_array(),
                color: [0.0; 4],
                normal: [0.0, 0.0, 1.0, 0.0],
            });
            queue.write_buffer(&self.grid_vertex_buffer, 0, bytemuck::cast_slice(&quad));
        } else if let Some(solid) = &scene.solid_ground {
            let quad: [GpuVertex; 4] = std::array::from_fn(|i| GpuVertex {
                position: solid.corners[i].to_array(),
                color: solid.color,
                normal: [0.0, 0.0, 1.0, 0.0],
            });
            queue.write_buffer(&self.grid_vertex_buffer, 0, bytemuck::cast_slice(&quad));
        }
        if !scene.text_vertices.is_empty() {
            queue.write_buffer(
                &self.text_vertex_buffer,
                0,
                bytemuck::cast_slice(&scene.text_vertices),
            );
        }
        if !scene.text_indices.is_empty() {
            queue.write_buffer(
                &self.text_index_buffer,
                0,
                bytemuck::cast_slice(&scene.text_indices),
            );
        }

        // Tracing images (#170): upload any new textures and build this frame's quad
        // vertex/index buffers before opening the render pass.
        let mut image_draws: Vec<(u64, wgpu::Buffer, wgpu::Buffer)> = Vec::new();
        if !scene.images.is_empty() {
            let mut textures = self.image_textures.lock().expect("image texture cache");
            for quad in &scene.images {
                if !textures.contains_key(&quad.id) {
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("bearcad_tracing_image"),
                        size: wgpu::Extent3d {
                            width: quad.width_px.max(1),
                            height: quad.height_px.max(1),
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        &quad.rgba,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(4 * quad.width_px.max(1)),
                            rows_per_image: Some(quad.height_px.max(1)),
                        },
                        wgpu::Extent3d {
                            width: quad.width_px.max(1),
                            height: quad.height_px.max(1),
                            depth_or_array_layers: 1,
                        },
                    );
                    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("bearcad_tracing_image_bind"),
                        layout: &self.text_texture_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                        ],
                    });
                    textures.insert(quad.id, bind_group);
                }
                let uv = [[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
                let vertices: Vec<GpuTextVertex> = quad
                    .corners
                    .iter()
                    .zip(uv.iter())
                    .map(|(corner, uv)| GpuTextVertex {
                        position: [corner.x, corner.y, corner.z],
                        uv: *uv,
                        color: [1.0, 1.0, 1.0, quad.opacity],
                    })
                    .collect();
                let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("bearcad_tracing_image_vbuf"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("bearcad_tracing_image_ibuf"),
                    contents: bytemuck::cast_slice::<u32, u8>(&[0, 1, 2, 0, 2, 3]),
                    usage: wgpu::BufferUsages::INDEX,
                });
                image_draws.push((quad.id, vbuf, ibuf));
            }
        }

        let color_view = self.color_view.as_ref().expect("color view");
        let depth_view = self.depth_view.as_ref().expect("depth view");
        let (color_attachment_view, resolve_target, color_store) =
            if let Some(msaa_view) = self.msaa_color_view.as_ref() {
                (
                    msaa_view,
                    Some(color_view),
                    wgpu::StoreOp::Discard,
                )
            } else {
                (color_view, None, wgpu::StoreOp::Store)
            };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("bearcad_viewport_scene_encoder"),
        });
        // Light-space depth (#1535). Always clear so a scene with no casters
        // samples as fully lit rather than last frame's occluders.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bearcad_viewport_shadow_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if shadow_caster_index_count > 0 && !scene.vertices.is_empty() {
                let caster_start = (scene_index_count + mask_index_count) as u32;
                let caster_end = caster_start + shadow_caster_index_count as u32;
                pass.set_pipeline(&self.shadow_pipeline);
                pass.set_bind_group(0, &self.shadow_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(caster_start..caster_end, 0, 0..1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bearcad_viewport_scene_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_attachment_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: scene.clear_color[0] as f64,
                            g: scene.clear_color[1] as f64,
                            b: scene.clear_color[2] as f64,
                            a: scene.clear_color[3] as f64,
                        }),
                        store: color_store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // Ground footprint first, under everything (#1073 / #159 / #1301): no depth
            // write, so bodies occlude it by overwriting colour and construction planes
            // composite cleanly later. Hidden from below in the fragment shaders (#1300).
            if scene.solid_ground.is_some() {
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_pipeline(&self.solid_ground_pipeline);
                pass.set_vertex_buffer(0, self.grid_vertex_buffer.slice(..));
                pass.set_index_buffer(self.grid_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
            } else if scene.grid.is_some() {
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_pipeline(&self.grid_pipeline);
                pass.set_vertex_buffer(0, self.grid_vertex_buffer.slice(..));
                pass.set_index_buffer(self.grid_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
            }
            // The origin axes, after the grid but before the scene, so bodies occlude them
            // the way they always did (#1072).
            if !scene.axis_indices.is_empty() {
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_pipeline(&self.axis_pipeline);
                pass.set_vertex_buffer(0, self.axis_vertex_buffer.slice(..));
                pass.set_index_buffer(self.axis_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..scene.axis_indices.len() as u32, 0, 0..1);
            }
            // Opaque base first (when any), so subsequent screen-space strokes can depth-test
            // against bodies. Stroke draw is outside this block so a scene of only lines still
            // paints (#1157).
            if scene_index_count > 0 && base_index_count > 0 {
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_pipeline(&self.scene_pipeline);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..base_index_count as u32, 0, 0..1);
            }
            // Screen-space sketch strokes (#1157): after the opaque base so bodies occlude
            // them, before translucent fills so a stroke still reads under a plane wash.
            // Depth sits on the endpoints; `vs_axis` widens in pixels so a face-sketched
            // line stays painted on the face instead of a freestanding 3D ribbon. Round
            // caps via `fs_axis` keep coincident joints from looking like overshoot (#1202).
            if !scene.stroke_indices.is_empty() {
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_pipeline(&self.axis_pipeline);
                pass.set_vertex_buffer(0, self.stroke_vertex_buffer.slice(..));
                pass.set_index_buffer(
                    self.stroke_index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..scene.stroke_indices.len() as u32, 0, 0..1);
            }
            if scene_index_count > 0 {
                pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(
                    self.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                let base_end = base_index_count as u32;
                let shadow_end = base_end + shadow_index_count as u32;
                let sketch_fill_end = shadow_end + sketch_fill_index_count as u32;
                let plane_end = sketch_fill_end + plane_fill_index_count as u32;
                let body_over_end = plane_end + body_over_plane_index_count as u32;
                let overlay_end = body_over_end + overlay_index_count as u32;
                let scene_end = scene_index_count as u32;
                if shadow_end > base_end {
                    // Reference 0b10 against bit 1: a fragment passes while that bit is
                    // clear and then sets it, so a silhouette's self-overlap paints once
                    // rather than blending into blotches (#1041).
                    pass.set_pipeline(&self.ground_shadow_pipeline);
                    pass.set_stencil_reference(0b10);
                    pass.draw_indexed(base_end..shadow_end, 0, 0..1);
                }
                if sketch_fill_end > shadow_end {
                    // Stencil ref 0 against bit 0: only fragments where that bit is still
                    // clear pass, and each one sets it, so coplanar sketch fills paint each
                    // pixel exactly once instead of double-blending overlaps (#3).
                    pass.set_pipeline(&self.sketch_fill_pipeline);
                    pass.set_stencil_reference(0);
                    pass.draw_indexed(shadow_end..sketch_fill_end, 0, 0..1);
                }
                if plane_end > sketch_fill_end {
                    pass.set_pipeline(&self.scene_transparent_pipeline);
                    pass.draw_indexed(sketch_fill_end..plane_end, 0, 0..1);
                }
                // Solid faces that share a construction plane's surface, re-drawn after the
                // translucent plane wash so they win coplanar depth ties without bias (#1215).
                // Sketch fills wrote a closer depth with their own bias, so LessEqual keeps
                // them in front of this pass.
                if body_over_end > plane_end {
                    pass.set_pipeline(&self.scene_pipeline);
                    pass.draw_indexed(plane_end..body_over_end, 0, 0..1);
                }
                if !image_draws.is_empty() {
                    // Tracing images (#170): depth-tested, no depth write — under all
                    // overlay/gizmo geometry.
                    let textures = self.image_textures.lock().expect("image texture cache");
                    pass.set_pipeline(&self.image_pipeline);
                    pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                    for (id, vbuf, ibuf) in &image_draws {
                        let Some(bind_group) = textures.get(id) else { continue };
                        pass.set_bind_group(1, bind_group, &[]);
                        pass.set_vertex_buffer(0, vbuf.slice(..));
                        pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..6, 0, 0..1);
                    }
                    // Restore state for the ranges below.
                    pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                }
                if overlay_end > body_over_end {
                    pass.set_pipeline(&self.overlay_pipeline);
                    pass.draw_indexed(body_over_end..overlay_end, 0, 0..1);
                }
                if scene_end > overlay_end {
                    // Gizmos, then the body edge-wireframe overlay (#33): both use the
                    // depth-test-disabled pipeline so they show through bodies (#36), so
                    // they share this one draw call over their combined index range.
                    pass.set_pipeline(&self.gizmo_pipeline);
                    pass.draw_indexed(overlay_end..scene_end, 0, 0..1);
                }
            }
            if !scene.text_indices.is_empty() {
                if let Some(font_bind_group) = self.font_bind_group.lock().unwrap().as_ref() {
                    pass.set_pipeline(&self.text_pipeline);
                    pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                    pass.set_bind_group(1, font_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.text_vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.text_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..scene.text_indices.len() as u32, 0, 0..1);
                }
            }
        }

        // Body-highlight outline (#1110/#1155): when the scene built a mask of selected/
        // hovered body silhouettes, paint it into the offscreen R/G target and stroke the
        // dilated band over the resolved scene colour (which already has the fill recolour).
        // Two extra passes — only when any body is selected/hovered.
        if mask_index_count > 0 {
            if let (Some(mask_view), Some(color_view), Some(outline_bind_group)) = (
                self.mask_view.as_ref(),
                self.color_view.as_ref(),
                self.outline_bind_group.as_ref(),
            ) {
                {
                    let mut mask_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("bearcad_viewport_outline_mask_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: mask_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    mask_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
                    mask_pass.set_pipeline(&self.mask_pipeline);
                    mask_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    mask_pass.set_index_buffer(
                        self.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    let mask_start = scene_index_count as u32;
                    let mask_end = total_index_count as u32;
                    mask_pass.draw_indexed(mask_start..mask_end, 0, 0..1);
                }
                {
                    // Load the resolved scene colour and alpha-blend the outline band on top.
                    let mut outline_pass =
                        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("bearcad_viewport_outline_pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: color_view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                    outline_pass.set_pipeline(&self.outline_pipeline);
                    outline_pass.set_bind_group(0, outline_bind_group, &[]);
                    outline_pass.draw(0..3, 0..1);
                }
            }
        }

        queue.submit(std::iter::once(encoder.finish()));
    }
}

pub struct ViewportPaintCallback {
    rect: Rect,
}

impl egui_wgpu::CallbackTrait for ViewportPaintCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &mut ViewportGpuResources = callback_resources.get_mut().unwrap();
        let scene = resources.pending_scene.lock().unwrap().take();
        let Some(scene) = scene else {
            return Vec::new();
        };
        let width = (self.rect.width() * screen_descriptor.pixels_per_point).round() as u32;
        let height = (self.rect.height() * screen_descriptor.pixels_per_point).round() as u32;
        resources.render_scene(device, queue, &scene, width, height);
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &ViewportGpuResources = callback_resources.get().unwrap();
        let Some(blit_bind_group) = resources.blit_bind_group.as_ref() else {
            return;
        };
        let viewport = info.viewport_in_pixels();
        if viewport.width_px == 0 || viewport.height_px == 0 {
            return;
        }
        render_pass.set_viewport(
            viewport.left_px as f32,
            viewport.top_px as f32,
            viewport.width_px as f32,
            viewport.height_px as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&resources.blit_pipeline);
        render_pass.set_bind_group(0, blit_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

pub fn paint_viewport(
    resources: &ViewportGpuResources,
    render_state: &egui_wgpu::RenderState,
    painter: &egui::Painter,
    rect: Rect,
    scene: ViewportScene,
) {
    resources.update_font_bind_group(&render_state.device, render_state);
    *resources.pending_scene.lock().unwrap() = Some(scene);
    painter.add(egui_wgpu::Callback::new_paint_callback(
        rect,
        ViewportPaintCallback { rect },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_msaa_prefers_four_samples_when_supported() {
        assert_eq!(clamp_msaa_sample_count(8), VIEWPORT_MSAA_SAMPLES);
        assert_eq!(clamp_msaa_sample_count(4), VIEWPORT_MSAA_SAMPLES);
    }

    #[test]
    fn clamp_msaa_falls_back_to_two_or_one() {
        assert_eq!(clamp_msaa_sample_count(3), 2);
        assert_eq!(clamp_msaa_sample_count(2), 2);
        assert_eq!(clamp_msaa_sample_count(1), 1);
        assert_eq!(clamp_msaa_sample_count(0), 1);
    }

    #[test]
    fn multisample_state_keeps_alpha_to_coverage_off_for_transparent_fills() {
        let msaa = multisample_state(4);
        assert_eq!(msaa.count, 4);
        assert!(!msaa.alpha_to_coverage_enabled);
        let single = multisample_state(1);
        assert_eq!(single.count, 1);
        assert!(!single.alpha_to_coverage_enabled);
    }

    #[test]
    fn msaa_sample_count_for_format_requires_resolve_support() {
        let no_resolve = wgpu::TextureFormatFeatures {
            allowed_usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
            flags: wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4,
        };
        assert_eq!(msaa_sample_count_for_format(&no_resolve), 1);

        let with_resolve = wgpu::TextureFormatFeatures {
            allowed_usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
            flags: wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
        };
        assert_eq!(
            msaa_sample_count_for_format(&with_resolve),
            VIEWPORT_MSAA_SAMPLES
        );
    }
}