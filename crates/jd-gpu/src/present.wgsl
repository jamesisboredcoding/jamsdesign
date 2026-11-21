// ---- pass 2: present ----------------------------------------------------------------------

@group(0) @binding(1) var composite: texture_2d<f32>;

@vertex
fn present_vs(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // Fullscreen triangle.
    let p = vec2(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn present_fs(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let local = frag.xy - g.bounds_origin;
    let doc = (local - g.origin) / g.zoom;

    var color = g.background.rgb;
    if all(doc >= vec2(0.0)) && all(doc < g.doc_size) {
        let cell = floor((local - g.origin) / g.checker_size);
        let odd = (i32(cell.x) + i32(cell.y)) & 1;
        let checker = select(g.checker_a.rgb, g.checker_b.rgb, odd == 1);
        let size = vec2<i32>(textureDimensions(composite));
        let texel = clamp(vec2<i32>(floor(local)), vec2(0), size - 1);
        let src = textureLoad(composite, texel, 0);
        // Composite over the checkerboard in working space, then go to linear for output.
        color = src.rgb + checker * (1.0 - src.a);
        if g.perceptual == 1u {
            color = srgb_to_linear(clamp(color, vec3(0.0), vec3(1.0)));
        }
    }

    if g.encode_srgb == 1u {
        color = linear_to_srgb(clamp(color, vec3(0.0), vec3(1.0)));
    }
    return vec4(color, 1.0);
}
