struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return Vertex(vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0), uv);
}

@group(0) @binding(0) var sprite: texture_2d<f32>;

@fragment
fn fs_main(vertex: Vertex) -> @location(0) vec4<f32> {
    // Nearest texel preserves the test patches and avoids filtering straight alpha.
    let pixel = vec2<i32>(vertex.uv * vec2<f32>(textureDimensions(sprite)));
    let rgba = textureLoad(sprite, pixel, 0);
    // The UNORM composition surface receives premultiplied display-encoded RGB.
    return vec4<f32>(rgba.rgb * rgba.a, rgba.a);
}
