#[derive(Clone, Copy)]
pub struct Canvas {
    pub size: [u32; 2],
    pub anchor: [f32; 2],
    pub offset: [f32; 2],
    pub scale: f32,
}

impl Canvas {
    pub fn untransformed(size: [u32; 2]) -> Self {
        Self {
            size,
            anchor: [0.5; 2],
            offset: [0.0; 2],
            scale: 1.0,
        }
    }

    pub fn destination(self, viewport: [u32; 2]) -> [i32; 4] {
        let canvas = self.size.map(f64::from);
        let viewport = viewport.map(f64::from);
        let fit = (viewport[0] / canvas[0]).min(viewport[1] / canvas[1]);
        let mut bounds = [0; 4];
        for axis in 0..2 {
            let fitted = canvas[axis] * fit;
            let scaled = fitted * f64::from(self.scale);
            let origin = (viewport[axis] - fitted) * 0.5
                + (fitted - scaled) * f64::from(self.anchor[axis])
                + fitted * f64::from(self.offset[axis]);
            // Keep pixel-center arithmetic representable for out-of-view metadata.
            let quantize = |value: f64| value.round().clamp(-250_000.0, 250_000.0) as i32;
            bounds[axis] = quantize(origin);
            bounds[axis + 2] = quantize(origin + scaled).max(bounds[axis] + 1);
        }
        bounds
    }
}
#[derive(Clone, Copy)]
pub struct FrameView<'a> {
    pub page_size: [u32; 2],
    pub rect: [u32; 4],
    pub canvas: Canvas,
    pub rgba: &'a [u8],
    pub bc7: Option<&'a [u8]>,
}

impl<'a> FrameView<'a> {
    pub fn rows(self) -> impl Iterator<Item = &'a [u8]> {
        let [x, y, width, height] = self.rect.map(|value| value as usize);
        self.rgba
            .chunks_exact(self.page_size[0] as usize * 4)
            .skip(y)
            .take(height)
            .map(move |row| &row[x * 4..(x + width) * 4])
    }
}
