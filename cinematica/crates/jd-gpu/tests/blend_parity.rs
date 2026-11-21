//! Renders documents on the real GPU and checks every pixel against jd-raster's CPU compositor
//! (which `export` uses). Skips, with a message, when no GPU adapter is available.

use jd_gpu::{Renderer, Scene, View, wgpu};
use jd_raster::{BlendMode, BlendSpace, Document, TileGrid};

const W: u32 = 64;
const H: u32 = 48;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// Opaque gradient backdrop + a gradient source layer with varying alpha.
fn document(mode: BlendMode, space: BlendSpace) -> Document {
    let bottom: Vec<u8> = (0..W * H)
        .flat_map(|i| {
            let (x, y) = (i % W, i / W);
            [(x * 4) as u8, (y * 5) as u8, 255 - (x * 3) as u8, 255]
        })
        .collect();
    let top: Vec<u8> = (0..W * H)
        .flat_map(|i| {
            let (x, y) = (i % W, i / W);
            [(y * 5) as u8, 255 - (x * 4) as u8, ((x + y) * 2) as u8, (64 + x * 3) as u8]
        })
        .collect();
    let mut doc = Document::from_rgba8("t", W, H, &bottom);
    doc.blend_space = space;
    let id = doc.add_layer("top");
    let layer = doc.layer_mut(id).unwrap();
    layer.pixels = TileGrid::from_rgba8(W, H, &top);
    layer.blend = mode;
    layer.opacity = 0.8;
    doc
}

fn render(device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document) -> Vec<u8> {
    render_sized(device, queue, doc, W, H)
}

/// `w * 4` must be a multiple of 256 for the readback copy.
fn render_sized(device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, w: u32, h: u32) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut renderer = Renderer::new(device, format);
    let v = View {
        bounds: [0.0, 0.0, w as f32, h as f32],
        origin: [0.0, 0.0],
        zoom: 1.0,
        background: [0.0; 3],
        checker: [[1.0; 3], [0.8; 3]],
        checker_size: 8.0,
    };
    renderer.prepare(device, queue, &Scene::from_document(doc), &v);
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(&mut encoder, &view, [0, 0, w, h]);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (w * h * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let data = buffer.slice(..).get_mapped_range().to_vec();
    data
}

#[test]
fn gpu_matches_cpu_for_every_blend_mode() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let mut failures = Vec::new();
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        for mode in BlendMode::ALL {
            let doc = document(mode, space);
            let gpu = render(&device, &queue, &doc);
            let mut worst = 0;
            let mut mismatched = 0;
            for y in 0..H {
                for x in 0..W {
                    let cpu = jd_raster::composite::pixel_srgb8(&doc, x, y);
                    let i = ((y * W + x) * 4) as usize;
                    let diff = (0..3).map(|c| (gpu[i + c] as i32 - cpu[c] as i32).abs()).max().unwrap();
                    worst = worst.max(diff);
                    if diff > 3 {
                        mismatched += 1;
                    }
                }
            }
            // Dissolve's coin flip can land differently right at the threshold. Burn/dodge
            // divide by values near zero, so the GPU's half-float backdrop (vs the CPU's f32)
            // tips a few borderline pixels into a different result. Everything else must match.
            let sensitive = matches!(
                mode,
                BlendMode::Dissolve | BlendMode::ColorBurn | BlendMode::ColorDodge | BlendMode::VividLight
            );
            let allowed = if sensitive { (W * H) / 100 } else { 0 };
            if mismatched > allowed {
                failures.push(format!("{space:?} {mode:?}: {mismatched} px off by >3 (worst {worst})"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn gradient(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    (0..w * h).flat_map(|i| f(i % w, i / w)).collect()
}

/// Exercises every compositing path at once: a non-Normal bottom layer (nothing to blend onto
/// yet), several ping-pong swaps, Normal after a blend, hidden and zero-opacity layers in the
/// middle, multiple tiles per layer and a partial edge tile.
#[test]
fn gpu_matches_cpu_for_a_stack_of_layers() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let (w, h) = (320, 48);
    for space in [BlendSpace::Perceptual, BlendSpace::Linear] {
        let mut doc = Document::blank("stack", w, h);
        doc.blend_space = space;
        let layers: [(BlendMode, f32, bool, Vec<u8>); 6] = [
            // Opaque, so its result over the (empty) backdrop is just itself.
            (BlendMode::Multiply, 1.0, true, gradient(w, h, |x, y| [(x * 255 / w) as u8, (y * 5) as u8, 120, 255])),
            (BlendMode::Normal, 1.0, true, gradient(w, h, |x, y| [200, (x % 256) as u8, (y * 4) as u8, ((x + y) % 200) as u8])),
            (BlendMode::Screen, 0.7, true, gradient(w, h, |x, y| [(y * 5) as u8, 90, (x * 255 / w) as u8, 220])),
            (BlendMode::Overlay, 1.0, false, gradient(w, h, |_, _| [255, 0, 0, 255])),
            (BlendMode::Color, 0.0, true, gradient(w, h, |_, _| [0, 255, 0, 255])),
            (BlendMode::Difference, 1.0, true, gradient(w, h, |x, y| [(x % 97 * 2) as u8, (y * 3) as u8, 64, 180])),
        ];
        for (i, (mode, opacity, visible, px)) in layers.into_iter().enumerate() {
            let id = doc.add_layer(format!("l{i}"));
            let l = doc.layer_mut(id).unwrap();
            l.pixels = TileGrid::from_rgba8(w, h, &px);
            l.blend = mode;
            l.opacity = opacity;
            l.visible = visible;
        }
        let gpu = render_sized(&device, &queue, &doc, w, h);
        let mut bad = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let cpu = jd_raster::composite::pixel_srgb8(&doc, x, y);
                let i = ((y * w + x) * 4) as usize;
                let diff = (0..3).map(|c| (gpu[i + c] as i32 - cpu[c] as i32).abs()).max().unwrap();
                if diff > 3 && bad.len() < 5 {
                    bad.push(format!("({x},{y}) gpu {:?} cpu {:?}", &gpu[i..i + 3], &cpu[..3]));
                }
            }
        }
        assert!(bad.is_empty(), "{space:?}: {}", bad.join(", "));
    }
}

/// A whole-pixel move previewed on the GPU (layer transform) matches the committed result
/// from jd-raster's resampler.
#[test]
fn gpu_move_preview_matches_committed_move() {
    let Some((device, queue)) = gpu() else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    let (w, h) = (320, 48);
    let mut doc = Document::from_rgba8("m", w, h, &gradient(w, h, |x, _| [(x % 256) as u8, 30, 90, 255]));
    let id = doc.add_layer("top");
    let top = gradient(w, h, |x, y| if (40..120).contains(&x) && (5..30).contains(&y) { [250, 220, (x * 2 % 256) as u8, 255] } else { [0; 4] });
    doc.layer_mut(id).unwrap().pixels = TileGrid::from_rgba8(w, h, &top);

    let mut scene = Scene::from_document(&doc);
    scene.layers[1].transform = [1.0, 0.0, 0.0, 1.0, 37.0, 9.0];
    let gpu = {
        // Render the scene directly (same as render_sized but with our modified scene).
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut renderer = Renderer::new(&device, format);
        let v = View {
            bounds: [0.0, 0.0, w as f32, h as f32],
            origin: [0.0, 0.0],
            zoom: 1.0,
            background: [0.0; 3],
            checker: [[1.0; 3], [0.8; 3]],
            checker_size: 8.0,
        };
        renderer.prepare(&device, &queue, &scene, &v);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&mut encoder, &view, [0, 0, w, h]);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (w * h * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let data = buffer.slice(..).get_mapped_range().to_vec();
        data
    };

    let moved = jd_raster::transform::transform_grid(&doc.layers()[1].pixels, &jd_raster::transform::Affine::translate(37.0, 9.0));
    doc.layer_mut(id).unwrap().pixels = moved;
    for y in 0..h {
        for x in 0..w {
            let cpu = jd_raster::composite::pixel_srgb8(&doc, x, y);
            let i = ((y * w + x) * 4) as usize;
            let diff = (0..3).map(|c| (gpu[i + c] as i32 - cpu[c] as i32).abs()).max().unwrap();
            assert!(diff <= 3, "({x},{y}) gpu {:?} cpu {:?}", &gpu[i..i + 3], &cpu[..3]);
        }
    }
}
