//! GPU renderer for raster documents.
//!
//! Toolkit-agnostic: the host (iced today) hands us a wgpu device/queue, a target and where in
//! it to draw. Per frame: [`Renderer::prepare`] uploads any new tiles and writes uniforms, then
//! [`Renderer::render`] records the passes described in `common.wgsl`.

mod mips;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use jd_raster::{BlendMode, BlendSpace, Document, TILE_SIZE, Tile, TileId, composite};

pub use wgpu;

const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const COMPOSITE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The part of a tile to draw. Edge tiles are cut at the canvas edge (and edge-padded for
/// mipmaps) unless they hold content past it (moved off the canvas), which must stay visible
/// while it's being moved. Tiles wholly off the canvas are drawn in full.
fn valid_area(pos: [i32; 2], doc_size: [u32; 2], tile: &Tile) -> [u32; 2] {
    let ts = TILE_SIZE as i64;
    let clip = |p: i32, size: u32| (size as i64 - p as i64 * ts).clamp(0, ts) as u32;
    let v = [clip(pos[0], doc_size[0]), clip(pos[1], doc_size[1])];
    let full = [TILE_SIZE, TILE_SIZE];
    if v == full || v[0] == 0 || v[1] == 0 || pos[0] < 0 || pos[1] < 0 {
        return full;
    }
    let beyond = tile.data().chunks_exact(4).enumerate().any(|(i, p)| {
        p[3] != 0 && ((i as u32 % TILE_SIZE) >= v[0] || (i as u32 / TILE_SIZE) >= v[1])
    });
    if beyond { [TILE_SIZE, TILE_SIZE] } else { v }
}

/// Uniform offsets must be 256-byte aligned on every backend.
const PARAMS_STRIDE: u64 = 256;

/// An immutable snapshot of what to draw. Cheap to build: tiles are `Arc`s.
#[derive(Debug, Clone)]
pub struct Scene {
    pub doc_size: [u32; 2],
    pub blend_space: BlendSpace,
    /// Bottom to top.
    pub layers: Vec<SceneLayer>,
    /// Tiles not drawn this frame that should stay cached on the GPU if already uploaded
    /// (e.g. other open documents), so switching tabs doesn't re-upload everything.
    pub retain: Vec<TileId>,
}

#[derive(Debug, Clone)]
pub struct SceneLayer {
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub dissolve_seed: u32,
    /// Affine map applied to the layer's tiles (doc px to doc px), `[a, b, c, d, e, f]` with
    /// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`. Used to preview moves and transforms.
    pub transform: [f32; 6],
    /// Tile grid positions; negative or past the canvas for content moved off it.
    pub tiles: Vec<([i32; 2], Arc<Tile>)>,
}

pub const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

impl Scene {
    pub fn from_document(doc: &Document) -> Self {
        Self {
            doc_size: [doc.width, doc.height],
            blend_space: doc.blend_space,
            layers: doc
                .layers()
                .iter()
                .enumerate()
                .map(|(i, l)| SceneLayer {
                    visible: l.visible,
                    opacity: l.opacity,
                    blend: l.blend,
                    dissolve_seed: composite::dissolve_seed(i),
                    transform: IDENTITY,
                    tiles: l.pixels.iter_all().map(|(x, y, t)| ([x, y], t.clone())).collect(),
                })
                .collect(),
            retain: Vec::new(),
        }
    }
}

/// Where and how to draw, all in **physical** pixels.
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// The viewport rectangle inside the target: x, y, width, height.
    pub bounds: [f32; 4],
    /// Document (0,0) relative to the viewport's top-left.
    pub origin: [f32; 2],
    /// Physical pixels per document pixel.
    pub zoom: f32,
    /// sRGB colours (0..1).
    pub background: [f32; 3],
    pub checker: [[f32; 3]; 2],
    pub checker_size: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    size: [f32; 2],
    origin: [f32; 2],
    bounds_origin: [f32; 2],
    doc_size: [f32; 2],
    background: [f32; 4],
    checker_a: [f32; 4],
    checker_b: [f32; 4],
    zoom: f32,
    checker_size: f32,
    encode_srgb: u32,
    perceptual: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct TileInstance {
    origin: [f32; 2],
    /// Part of the tile inside the document (edge tiles are partial), doc px.
    size: [f32; 2],
    opacity: f32,
    /// Layer transform columns: (a, b), (c, d), (e, f).
    m0: [f32; 2],
    m1: [f32; 2],
    m2: [f32; 2],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LayerParams {
    mode: u32,
    dissolve_seed: u32,
    _pad: [u32; 2],
}

struct GpuTile {
    bind_group: wgpu::BindGroup,
    _texture: wgpu::Texture,
}

/// Consecutive tiles drawn the same way.
enum Batch {
    /// Normal layers: drawn straight onto the accumulated image.
    Normal(Range<u32>),
    /// One layer with another blend mode; `slot` indexes its `LayerParams`.
    Blend { tiles: Range<u32>, slot: u32 },
}

struct Targets {
    size: [u32; 2],
    accum: [wgpu::TextureView; 2],
    layer: wgpu::TextureView,
    /// Blend pass reading accum[i] as backdrop (writes accum[1 - i]).
    blend_groups: [wgpu::BindGroup; 2],
    /// Present pass reading accum[i].
    present_groups: [wgpu::BindGroup; 2],
}

pub struct Renderer {
    encode_srgb: bool,
    globals: wgpu::Buffer,
    tile_pipeline: wgpu::RenderPipeline,
    tile_globals_bind_group: wgpu::BindGroup,
    tile_texture_layout: wgpu::BindGroupLayout,
    blend_pipeline: wgpu::RenderPipeline,
    blend_layout: wgpu::BindGroupLayout,
    params_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    params_bind_group: wgpu::BindGroup,
    params_capacity: u64,
    present_pipeline: wgpu::RenderPipeline,
    present_layout: wgpu::BindGroupLayout,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    tiles: HashMap<TileId, GpuTile>,
    targets: Option<Targets>,
    /// This frame's draw list: tile per instance index, grouped into batches.
    draws: Vec<TileId>,
    batches: Vec<Batch>,
    bounds: [f32; 4],
}

impl Renderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let common = include_str!("common.wgsl");
        let module = |label, src: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{common}\n{src}").into()),
            })
        };
        let tile_module = module("jd-gpu tile", include_str!("tile.wgsl"));
        let blend_module = module("jd-gpu blend", include_str!("blend.wgsl"));
        let present_module = module("jd-gpu present", include_str!("present.wgsl"));

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("jd-gpu globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_entry = |binding, dynamic| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: None,
            },
            count: None,
        };
        let texture_entry = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        // Declared before `pipeline` so they outlive every call (the closure's borrow is shared).
        let tile_attributes = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32x2, 4 => Float32x2, 5 => Float32x2
        ];
        let tile_buffers = [wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TileInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &tile_attributes,
        }];
        #[allow(clippy::too_many_arguments)]
        let pipeline = |label,
                        layouts: &[&wgpu::BindGroupLayout],
                        module,
                        vs,
                        fs,
                        buffers,
                        format,
                        blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: layouts,
                    push_constant_ranges: &[],
                })),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };

        // --- tiles
        let tile_globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("jd-gpu tile globals"),
            entries: &[
                uniform_entry(0, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let tile_texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("jd-gpu tile texture"),
            entries: &[texture_entry(0, true)],
        });
        // Smooth when zoomed out (trilinear over the CPU-built mips), crisp pixels when zoomed in.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("jd-gpu tile sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let tile_globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("jd-gpu tile globals"),
            layout: &tile_globals_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let tile_pipeline = pipeline(
            "jd-gpu tile",
            &[&tile_globals_layout, &tile_texture_layout],
            &tile_module,
            "tile_vs",
            "tile_fs",
            &tile_buffers,
            COMPOSITE_FORMAT,
            Some(wgpu::BlendState { color: over, alpha: over }),
        );

        // --- blend modes
        let blend_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("jd-gpu blend"),
            entries: &[uniform_entry(0, false), texture_entry(1, false), texture_entry(2, false)],
        });
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("jd-gpu layer params"),
            entries: &[uniform_entry(0, true)],
        });
        let blend_pipeline = pipeline(
            "jd-gpu blend",
            &[&blend_layout, &params_layout],
            &blend_module,
            "blend_vs",
            "blend_fs",
            &[],
            COMPOSITE_FORMAT,
            None,
        );
        let params_capacity = 16;
        let (params, params_bind_group) = Self::params_buffer(device, &params_layout, params_capacity);

        // --- present
        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("jd-gpu present"),
            entries: &[uniform_entry(0, false), texture_entry(1, false)],
        });
        let present_pipeline = pipeline(
            "jd-gpu present",
            &[&present_layout],
            &present_module,
            "present_vs",
            "present_fs",
            &[],
            target_format,
            None,
        );

        let instance_capacity = 256;
        Self {
            encode_srgb: !target_format.is_srgb(),
            instances: Self::instance_buffer(device, instance_capacity),
            instance_capacity,
            globals,
            tile_pipeline,
            tile_globals_bind_group,
            tile_texture_layout,
            blend_pipeline,
            blend_layout,
            params_layout,
            params,
            params_bind_group,
            params_capacity,
            present_pipeline,
            present_layout,
            tiles: HashMap::new(),
            targets: None,
            draws: Vec::new(),
            batches: Vec::new(),
            bounds: [0.0; 4],
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("jd-gpu tile instances"),
            size: (capacity * std::mem::size_of::<TileInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn params_buffer(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        capacity: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("jd-gpu layer params"),
            size: capacity * PARAMS_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("jd-gpu layer params"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<LayerParams>() as u64),
                }),
            }],
        });
        (buffer, group)
    }

    /// Uploads new tiles, drops tiles no longer needed, and writes this frame's uniforms.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: &Scene, view: &View) {
        let live: HashSet<TileId> = scene
            .layers
            .iter()
            .flat_map(|l| l.tiles.iter().map(|(_, t)| t.id()))
            .chain(scene.retain.iter().copied())
            .collect();
        self.tiles.retain(|id, _| live.contains(id));

        let mut instances = Vec::new();
        let mut params = Vec::new();
        self.draws.clear();
        self.batches.clear();
        for layer in &scene.layers {
            let start = instances.len() as u32;
            for (pos, tile) in &layer.tiles {
                let valid = valid_area(*pos, scene.doc_size, tile);
                if !self.tiles.contains_key(&tile.id()) {
                    let gpu = self.upload(device, queue, tile, valid);
                    self.tiles.insert(tile.id(), gpu);
                }
                if layer.visible && layer.opacity > 0.0 {
                    instances.push(TileInstance {
                        origin: [(pos[0] * TILE_SIZE as i32) as f32, (pos[1] * TILE_SIZE as i32) as f32],
                        size: [valid[0] as f32, valid[1] as f32],
                        opacity: layer.opacity,
                        m0: [layer.transform[0], layer.transform[1]],
                        m1: [layer.transform[2], layer.transform[3]],
                        m2: [layer.transform[4], layer.transform[5]],
                    });
                    self.draws.push(tile.id());
                }
            }
            let range = start..instances.len() as u32;
            if range.is_empty() {
                continue;
            }
            if layer.blend == BlendMode::Normal {
                match self.batches.last_mut() {
                    Some(Batch::Normal(r)) if r.end == range.start => r.end = range.end,
                    _ => self.batches.push(Batch::Normal(range)),
                }
            } else {
                self.batches.push(Batch::Blend { tiles: range, slot: params.len() as u32 });
                params.push(LayerParams {
                    mode: layer.blend.index(),
                    dissolve_seed: layer.dissolve_seed,
                    _pad: [0; 2],
                });
            }
        }

        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instances = Self::instance_buffer(device, self.instance_capacity);
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&instances));
        }
        if params.len() as u64 > self.params_capacity {
            self.params_capacity = (params.len() as u64).next_power_of_two();
            (self.params, self.params_bind_group) =
                Self::params_buffer(device, &self.params_layout, self.params_capacity);
        }
        for (i, p) in params.iter().enumerate() {
            queue.write_buffer(&self.params, i as u64 * PARAMS_STRIDE, bytemuck::bytes_of(p));
        }

        let size = [view.bounds[2].ceil().max(1.0) as u32, view.bounds[3].ceil().max(1.0) as u32];
        if self.targets.as_ref().is_none_or(|t| t.size != size) {
            self.targets = Some(self.create_targets(device, size));
        }
        self.bounds = view.bounds;

        let perceptual = scene.blend_space == BlendSpace::Perceptual;
        let linear = |c: [f32; 3]| c.map(|v| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) });
        let working = |c: [f32; 3]| {
            let c = if perceptual { c } else { linear(c) };
            [c[0], c[1], c[2], 1.0]
        };
        let bg = linear(view.background);
        let globals = Globals {
            size: [view.bounds[2], view.bounds[3]],
            origin: view.origin,
            bounds_origin: [view.bounds[0], view.bounds[1]],
            doc_size: [scene.doc_size[0] as f32, scene.doc_size[1] as f32],
            background: [bg[0], bg[1], bg[2], 1.0],
            checker_a: working(view.checker[0]),
            checker_b: working(view.checker[1]),
            zoom: view.zoom,
            checker_size: view.checker_size,
            encode_srgb: self.encode_srgb as u32,
            perceptual: perceptual as u32,
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
    }

    /// Records all passes. `clip` is the scissor rect in target pixels: x, y, width, height.
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, clip: [u32; 4]) {
        let Some(t) = &self.targets else { return };
        if clip[2] == 0 || clip[3] == 0 {
            return;
        }

        let pass = |encoder: &mut wgpu::CommandEncoder, view, clear: bool| {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("jd-gpu composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            })
            .forget_lifetime()
        };
        let draw_tiles = |pass: &mut wgpu::RenderPass<'_>, range: &Range<u32>| {
            pass.set_pipeline(&self.tile_pipeline);
            pass.set_bind_group(0, &self.tile_globals_bind_group, &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            for i in range.clone() {
                let Some(tile) = self.tiles.get(&self.draws[i as usize]) else { continue };
                pass.set_bind_group(1, &tile.bind_group, &[]);
                pass.draw(0..6, i..i + 1);
            }
        };

        let mut cur = 0;
        let mut cleared = false;
        for batch in &self.batches {
            match batch {
                Batch::Normal(range) => {
                    let mut p = pass(encoder, &t.accum[cur], !cleared);
                    cleared = true;
                    draw_tiles(&mut p, range);
                }
                Batch::Blend { tiles, slot } => {
                    if !cleared {
                        drop(pass(encoder, &t.accum[cur], true));
                        cleared = true;
                    }
                    {
                        let mut p = pass(encoder, &t.layer, true);
                        draw_tiles(&mut p, tiles);
                    }
                    let mut p = pass(encoder, &t.accum[1 - cur], false);
                    p.set_pipeline(&self.blend_pipeline);
                    p.set_bind_group(0, &t.blend_groups[cur], &[]);
                    p.set_bind_group(1, &self.params_bind_group, &[(*slot as u64 * PARAMS_STRIDE) as u32]);
                    p.draw(0..3, 0..1);
                    cur = 1 - cur;
                }
            }
        }
        if !cleared {
            drop(pass(encoder, &t.accum[cur], true));
        }

        let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("jd-gpu present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        let [x, y, w, h] = self.bounds;
        p.set_viewport(x, y, w, h, 0.0, 1.0);
        p.set_scissor_rect(clip[0], clip[1], clip[2], clip[3]);
        p.set_pipeline(&self.present_pipeline);
        p.set_bind_group(0, &t.present_groups[cur], &[]);
        p.draw(0..3, 0..1);
    }

    fn create_targets(&self, device: &wgpu::Device, size: [u32; 2]) -> Targets {
        let texture = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: COMPOSITE_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let accum = [texture("jd-gpu accum a"), texture("jd-gpu accum b")];
        let layer = texture("jd-gpu layer");
        let globals = || wgpu::BindGroupEntry { binding: 0, resource: self.globals.as_entire_binding() };
        let view = |binding, v| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(v) };
        let blend_group = |i: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("jd-gpu blend"),
                layout: &self.blend_layout,
                entries: &[globals(), view(1, &accum[i]), view(2, &layer)],
            })
        };
        let present_group = |i: usize| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("jd-gpu present"),
                layout: &self.present_layout,
                entries: &[globals(), view(1, &accum[i])],
            })
        };
        Targets {
            size,
            blend_groups: [blend_group(0), blend_group(1)],
            present_groups: [present_group(0), present_group(1)],
            accum,
            layer,
        }
    }

    /// `valid` is the part of the tile inside the document. For edge tiles the rest is padded
    /// with copies of the last row/column so filtering never blends in transparent pixels.
    fn upload(&self, device: &wgpu::Device, queue: &wgpu::Queue, tile: &Tile, valid: [u32; 2]) -> GpuTile {
        let padded;
        let base: &[u8] = if valid == [TILE_SIZE, TILE_SIZE] {
            tile.data()
        } else {
            padded = mips::pad_edges(tile.data(), TILE_SIZE, valid);
            &padded
        };
        let levels = mips::chain(base, TILE_SIZE);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("jd-gpu tile"),
            size: wgpu::Extent3d { width: TILE_SIZE, height: TILE_SIZE, depth_or_array_layers: 1 },
            mip_level_count: levels.len() as u32 + 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TILE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let write = |level: u32, data: &[u8], size: u32| {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 4), rows_per_image: Some(size) },
                wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            );
        };
        write(0, base, TILE_SIZE);
        for (i, level) in levels.iter().enumerate() {
            write(i as u32 + 1, level, TILE_SIZE >> (i + 1));
        }
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("jd-gpu tile"),
            layout: &self.tile_texture_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
        });
        GpuTile { bind_group, _texture: texture }
    }
}
