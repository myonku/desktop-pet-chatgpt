use super::scaled_edge;

#[test]
fn region_edges_match_shader_texels_at_supported_dpi_scales() {
    let source = 128;
    for physical in [256, 320, 384, 448, 512] {
        for x in 0..physical {
            let texel = ((x as f32 + 0.5) / physical as f32 * source as f32) as u32;
            let left = scaled_edge(texel, source, physical);
            let right = scaled_edge(texel + 1, source, physical);
            assert!(
                (left..right).contains(&x),
                "{physical}px window at pixel {x}"
            );
        }
    }
}
