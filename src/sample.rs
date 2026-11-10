use std::{error::Error, io::Cursor};

pub struct Sample {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
pub struct FrameView<'a> {
    pub page_size: [u32; 2],
    pub rect: [u32; 4],
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

impl Sample {
    pub fn view(&self) -> FrameView<'_> {
        FrameView {
            page_size: [self.width, self.height],
            rect: [0, 0, self.width, self.height],
            rgba: &self.rgba,
            bc7: None,
        }
    }

    pub fn load() -> Result<Self, Box<dyn Error>> {
        let png = include_bytes!("render/alpha-color.png");
        let mut reader = png::Decoder::new(Cursor::new(png)).read_info()?;
        let mut rgba = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or("PNG output size overflow")?
        ];
        let info = reader.next_frame(&mut rgba)?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return Err("the prototype sample must be an RGBA8 PNG".into());
        }
        rgba.truncate(info.buffer_size());
        Ok(Self {
            width: info.width,
            height: info.height,
            rgba,
        })
    }
}
