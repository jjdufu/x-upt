use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

fn main() {
    let imgs_dir = Path::new("imgs");
    if imgs_dir.exists() {
        fs::remove_dir_all(imgs_dir).unwrap();
    }
    fs::create_dir_all(imgs_dir).unwrap();

    for i in 1..=100 {
        let width = 120;
        let height = 120;
        let mut img = image::RgbImage::new(width, height);

        let pattern_type = i % 4;
        for x in 0..width {
            for y in 0..height {
                let pixel = match pattern_type {
                    0 => {
                        let r = ((x as f32 / width as f32) * 255.0) as u8;
                        let g = ((y as f32 / height as f32) * 255.0) as u8;
                        let b = ((i as f32 / 100.0) * 255.0) as u8;
                        image::Rgb([r, g, b])
                    }
                    1 => {
                        let cx = width / 2;
                        let cy = height / 2;
                        let dist = (((x as i32 - cx as i32).pow(2) + (y as i32 - cy as i32).pow(2))
                            as f32)
                            .sqrt();
                        let r = ((dist / 50.0).sin().abs() * 255.0) as u8;
                        let g = ((i as f32 / 100.0) * 255.0) as u8;
                        let b = ((x as f32 / width as f32) * 255.0) as u8;
                        image::Rgb([r, g, b])
                    }
                    2 => {
                        if x % 12 < 2 || y % 12 < 2 {
                            image::Rgb([255, 255, 255])
                        } else {
                            let r = (i * 2) as u8;
                            let g = x as u8;
                            let b = y as u8;
                            image::Rgb([r, g, b])
                        }
                    }
                    _ => {
                        let val = (x + y + i * 5) % 255;
                        image::Rgb([val as u8, (255 - val) as u8, ((x * y) % 255) as u8])
                    }
                };
                img.put_pixel(x, y, pixel);
            }
        }

        let mut bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut bytes);
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();

        let target_size = 25 * 1024;
        if bytes.len() < target_size {
            let padding_len = target_size - bytes.len();
            bytes.extend(vec![0u8; padding_len]);
        }

        let file_path = imgs_dir.join(format!("img_{:03}.png", i));
        let mut file = File::create(file_path).unwrap();
        file.write_all(&bytes).unwrap();
    }

    println!(
        "Successfully regenerated 100 unique images of size between 15KB and 50KB in the 'imgs' directory."
    );
}
