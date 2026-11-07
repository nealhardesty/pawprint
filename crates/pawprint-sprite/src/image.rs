//! A minimal RGBA image plus PNG encode and decode.

/// Straight (non-premultiplied) RGBA8, row-major, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// A fully transparent image.
    pub fn new(width: u32, height: u32) -> Self {
        RgbaImage {
            width,
            height,
            data: vec![0; (width * height * 4) as usize],
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    /// Set a pixel, ignoring coordinates outside the image.
    pub fn put(&mut self, x: i64, y: i64, px: [u8; 4]) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.data[i..i + 4].copy_from_slice(&px);
    }

    /// Encode as an 8-bit RGBA PNG.
    pub fn encode_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .expect("PNG header into a Vec cannot fail");
        writer
            .write_image_data(&self.data)
            .expect("PNG data into a Vec cannot fail");
        writer.finish().expect("PNG finish into a Vec cannot fail");
        out
    }

    /// Decode any PNG (palette, gray, RGB, 16-bit, ...) into RGBA8.
    pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage, String> {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec
            .read_info()
            .map_err(|e| format!("not a readable PNG: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("bad PNG data: {e}"))?;
        let (w, h) = (info.width, info.height);
        let src = &buf[..info.buffer_size()];
        let data = match info.color_type {
            png::ColorType::Rgba => src.to_vec(),
            png::ColorType::Rgb => src
                .chunks(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => src
                .chunks(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Grayscale => src.iter().flat_map(|&v| [v, v, v, 255]).collect(),
            png::ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
        };
        Ok(RgbaImage {
            width: w,
            height: h,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let mut img = RgbaImage::new(3, 2);
        img.put(0, 0, [1, 2, 3, 255]);
        img.put(2, 1, [200, 100, 50, 255]);
        img.put(-1, 5, [9, 9, 9, 255]); // ignored
        let back = RgbaImage::decode_png(&img.encode_png()).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(RgbaImage::decode_png(b"not a png").is_err());
    }
}
