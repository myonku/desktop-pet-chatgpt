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
