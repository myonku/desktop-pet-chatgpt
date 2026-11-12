use std::{error::Error, io::Cursor};

use crate::media::frame::{Canvas, FrameView};

pub(crate) struct Placeholder {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Placeholder {
    pub fn view(&self) -> FrameView<'_> {
        FrameView {
            page_size: [self.width, self.height],
            rect: [0, 0, self.width, self.height],
            canvas: Canvas::untransformed([self.width, self.height]),
            rgba: &self.rgba,
            bc7: None,
        }
    }

    pub fn load() -> Result<Self, Box<dyn Error>> {
        let png = include_bytes!("alpha-color.png");
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
