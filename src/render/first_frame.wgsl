@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

struct Frame {
    source: vec4<u32>,
    destination: vec4<i32>,
}

@group(0) @binding(0) var sprite: texture_2d<f32>;
@group(0) @binding(1) var<uniform> frame: Frame;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let origin = frame.destination.xy;
    let end = frame.destination.zw;
    if any(pixel < origin) || any(pixel >= end) {
        return vec4<f32>(0.0);
    }
    // Map destination pixel centers to the same source texels as the hit region.
    let extent = frame.source.zw;
    let center = vec2<u32>(2 * (pixel - origin) + vec2<i32>(1));
    let local = center * extent / vec2<u32>(2 * (end - origin));
    let rgba = textureLoad(sprite, vec2<i32>(frame.source.xy + local), 0);
    return vec4<f32>(rgba.rgb * rgba.a, rgba.a);
}
