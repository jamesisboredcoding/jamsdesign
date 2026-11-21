// Passes, per frame:
//  1. `tile_*` (tile.wgsl): draws a layer's tiles, premultiplied, into an offscreen Rgba16Float
//     texture the size of the viewport. Values are in the document's WORKING space: sRGB-encoded
//     ("perceptual", Photoshop-like) or linear light. Normal layers go straight onto the
//     accumulated image with hardware blending.
//  1b. `blend_*` (blend.wgsl): other blend modes draw the layer into its own texture first, then
//     combine it with the accumulated image into a second accumulation texture (ping-pong).
//  2. `present_*` (present.wgsl): canvas background, transparency checkerboard inside the
//     document, the accumulated image over it, then conversion to the target's encoding.

struct Globals {
    size: vec2<f32>,          // viewport size, physical px
    origin: vec2<f32>,        // document (0,0) in viewport-local physical px
    bounds_origin: vec2<f32>, // viewport top-left in framebuffer px
    doc_size: vec2<f32>,      // document size in doc px
    background: vec4<f32>,    // linear
    checker_a: vec4<f32>,     // working space
    checker_b: vec4<f32>,     // working space
    zoom: f32,                // physical px per doc px
    checker_size: f32,        // physical px
    encode_srgb: u32,         // target isn't sRGB: encode ourselves
    perceptual: u32,          // working space is sRGB-encoded (else linear)
};

@group(0) @binding(0) var<uniform> g: Globals;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3(0.0031308));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3(0.0)) + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}
