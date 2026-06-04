use image::{ImageBuffer, ImageFormat, Rgb};

pub fn generate_test_image(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let mut img = ImageBuffer::new(width, height);

    for (x, y, pixel) in img.enumerate_pixels_mut() {
        *pixel = Rgb([ (x % 256) as u8, (y % 256) as u8, 128 ]);
    }

    let mut bytes: Vec<u8> = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut bytes);
    img.write_to(&mut cursor, format).unwrap();

    bytes
}
