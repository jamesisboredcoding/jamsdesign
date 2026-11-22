// ---- pass 1: tiles ------------------------------------------------------------------------

@group(0) @binding(1) var tile_sampler: sampler;
@group(1) @binding(0) var tile_texture: texture_2d<f32>;

const TILE_SIZE: f32 = 256.0;

struct TileOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
};

fn corner(i: u32) -> vec2<f32> {
    // Two triangles: (0,0) (1,0) (0,1) / (0,1) (1,0) (1,1)
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    return corners[i];
}

@vertex
fn tile_vs(
    @builtin(vertex_index) vi: u32,
    @location(0) tile_origin: vec2<f32>,
    @location(1) tile_size: vec2<f32>,
    @location(2) opacity: f32,
    @location(3) m0: vec2<f32>,
    @location(4) m1: vec2<f32>,
    @location(5) m2: vec2<f32>,
) -> TileOut {
    let c = corner(vi);
    let p = tile_origin + c * tile_size;
    let doc = m0 * p.x + m1 * p.y + m2;
    let local = g.origin + doc * g.zoom;
    let ndc = local / g.size * 2.0 - 1.0;
    var out: TileOut;
    out.position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
    out.uv = c * tile_size / TILE_SIZE;
    out.opacity = opacity;
    return out;
}

@fragment
fn tile_fs(in: TileOut) -> @location(0) vec4<f32> {
    // Sampling an sRGB texture gives linear values (filtered in linear light).
    var c = textureSample(tile_texture, tile_sampler, in.uv);
    if g.perceptual == 1u {
        c = vec4(linear_to_srgb(c.rgb), c.a);
    }
    return vec4(c.rgb * c.a, c.a) * in.opacity;
}

