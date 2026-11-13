use super::{RECT, Region, scaled_edge};
use windows::Win32::Graphics::Gdi::PtInRegion;

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

#[test]
fn batched_region_preserves_fragmented_alpha() {
    let side = 224;
    let runs: Vec<_> = (0..side)
        .flat_map(|y| {
            (y % 2..side).step_by(2).map(move |x| RECT {
                left: x,
                top: y,
                right: x + 1,
                bottom: y + 1,
            })
        })
        .collect();
    let start = std::time::Instant::now();
    let region = Region::from_runs(&runs).unwrap();
    eprintln!("{} alpha runs in {:?}", runs.len(), start.elapsed());
    for y in 0..side {
        for x in 0..side {
            assert_eq!(
                unsafe { PtInRegion(region.0, x, y) }.as_bool(),
                (x + y) % 2 == 0
            );
        }
    }
    assert!(!unsafe { PtInRegion(Region::from_runs(&[]).unwrap().0, 0, 0) }.as_bool());
}
