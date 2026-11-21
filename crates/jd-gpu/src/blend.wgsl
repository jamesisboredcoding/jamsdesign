// ---- pass 1b: blend a layer onto the backdrop ----------------------------------------------
//
// Mirrors jd_raster::blend exactly (a GPU test compares them). `cb` backdrop, `cs` source,
// unpremultiplied, working space. Mode numbers follow BlendMode::ALL.

struct LayerParams {
    mode: u32,
    dissolve_seed: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(1) var backdrop: texture_2d<f32>;
@group(0) @binding(2) var source: texture_2d<f32>;
@group(1) @binding(0) var<uniform> layer: LayerParams;

@vertex
fn blend_vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let p = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

fn color_burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 { return 1.0; }
    if cs <= 0.0 { return 0.0; }
    return 1.0 - min(1.0, (1.0 - cb) / cs);
}

fn color_dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 { return 0.0; }
    if cs >= 1.0 { return 1.0; }
    return min(1.0, cb / (1.0 - cs));
}

fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 { return cb * 2.0 * cs; }
    let s = 2.0 * cs - 1.0;
    return cb + s - cb * s;
}

fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 { return cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb); }
    var d: f32;
    if cb <= 0.25 { d = ((16.0 * cb - 12.0) * cb + 4.0) * cb; } else { d = sqrt(cb); }
    return cb + (2.0 * cs - 1.0) * (d - cb);
}

fn separable(mode: u32, cb: f32, cs: f32) -> f32 {
    switch mode {
        case 2u: { return min(cb, cs); }                         // Darken
        case 3u: { return cb * cs; }                             // Multiply
        case 4u: { return color_burn(cb, cs); }
        case 5u: { return max(0.0, cb + cs - 1.0); }             // Linear Burn
        case 6u: { return max(cb, cs); }                         // Lighten
        case 7u: { return cb + cs - cb * cs; }                   // Screen
        case 8u: { return color_dodge(cb, cs); }
        case 9u: { return min(1.0, cb + cs); }                   // Linear Dodge
        case 10u: { return hard_light(cs, cb); }                 // Overlay
        case 11u: { return soft_light(cb, cs); }
        case 12u: { return hard_light(cb, cs); }
        case 13u: {                                              // Vivid Light
            if cs <= 0.5 { return color_burn(cb, 2.0 * cs); }
            return color_dodge(cb, 2.0 * (cs - 0.5));
        }
        case 14u: { return clamp(cb + 2.0 * cs - 1.0, 0.0, 1.0); } // Linear Light
        case 15u: {                                              // Pin Light
            if cs <= 0.5 { return min(cb, 2.0 * cs); }
            return max(cb, 2.0 * cs - 1.0);
        }
        case 16u: { return abs(cb - cs); }                       // Difference
        case 17u: { return cb + cs - 2.0 * cb * cs; }            // Exclusion
        case 18u: { return max(0.0, cb - cs); }                  // Subtract
        case 19u: {                                              // Divide
            if cs <= 0.0 { return select(1.0, 0.0, cb <= 0.0); }
            return min(1.0, cb / cs);
        }
        default: { return cs; }                                  // Normal, Dissolve
    }
}

fn lum(c: vec3<f32>) -> f32 {
    return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

fn clip_color(c_in: vec3<f32>) -> vec3<f32> {
    var c = c_in;
    let l = lum(c);
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    if n < 0.0 { c = l + (c - l) * l / (l - n); }
    if x > 1.0 { c = l + (c - l) * (1.0 - l) / (x - l); }
    return c;
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    return clip_color(c + (l - lum(c)));
}

fn sat(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let mx = max(c.r, max(c.g, c.b));
    let mn = min(c.r, min(c.g, c.b));
    if mx <= mn { return vec3(0.0); }
    return (c - mn) * s / (mx - mn);
}

fn blend_rgb(mode: u32, cb: vec3<f32>, cs: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 20u: { return set_lum(set_sat(cs, sat(cb)), lum(cb)); } // Hue
        case 21u: { return set_lum(set_sat(cb, sat(cs)), lum(cb)); } // Saturation
        case 22u: { return set_lum(cs, lum(cb)); }                   // Color
        case 23u: { return set_lum(cb, lum(cs)); }                   // Luminosity
        default: {
            return vec3(separable(mode, cb.r, cs.r), separable(mode, cb.g, cs.g), separable(mode, cb.b, cs.b));
        }
    }
}

fn dissolve_noise(x: u32, y: u32, seed: u32) -> f32 {
    var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

fn unpremul(c: vec4<f32>) -> vec3<f32> {
    if c.a > 0.0 { return c.rgb / c.a; }
    return vec3(0.0);
}

@fragment
fn blend_fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(floor(frag.xy));
    let b = textureLoad(backdrop, texel, 0);
    var s = textureLoad(source, texel, 0);

    if layer.mode == 1u { // Dissolve: all-or-nothing coverage with probability = alpha
        let doc = max(floor((frag.xy - g.origin) / g.zoom), vec2(0.0));
        let keep = dissolve_noise(u32(doc.x), u32(doc.y), layer.dissolve_seed) < s.a;
        if keep && s.a > 0.0 { s = vec4(s.rgb / s.a, 1.0); } else { s = vec4(0.0); }
    }

    let as_ = s.a;
    let ab = b.a;
    if as_ <= 0.0 { return b; }
    let cb = unpremul(b);
    let cs = unpremul(s);
    let bl = blend_rgb(layer.mode, cb, cs);
    let rgb = as_ * (1.0 - ab) * cs + as_ * ab * bl + (1.0 - as_) * ab * cb;
    return vec4(rgb, as_ + ab * (1.0 - as_));
}
