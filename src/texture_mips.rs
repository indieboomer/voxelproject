//! Per-tile mip generation. Nearest texel filtering prevents atlas tile bleed;
//! interpolation between mip levels reduces crawling without blurring nearby pixels.
pub fn atlas(image: &image::RgbaImage, cols: u32, rows: u32) -> Vec<image::RgbaImage> {
    let mut result = vec![image.clone()];
    let mut tile = image.width() / cols;
    while tile > 1 {
        let next = tile / 2;
        let src = result.last().unwrap();
        let mut dst = image::RgbaImage::new(cols * next, rows * next);
        for ty in 0..rows {
            for tx in 0..cols {
                for y in 0..next {
                    for x in 0..next {
                        let mut rgb = [0f32; 3];
                        let mut alpha = 0.;
                        for dy in 0..2 {
                            for dx in 0..2 {
                                let p = src
                                    .get_pixel(tx * tile + x * 2 + dx, ty * tile + y * 2 + dy)
                                    .0;
                                let a = p[3] as f32 / 255.;
                                alpha += a;
                                for c in 0..3 {
                                    rgb[c] += (p[c] as f32 / 255.).powf(2.2) * a;
                                }
                            }
                        }
                        let mut p = [0u8; 4];
                        for c in 0..3 {
                            p[c] =
                                ((rgb[c] / alpha.max(0.001)).powf(1. / 2.2) * 255.).round() as u8;
                        }
                        p[3] = (alpha * 255. / 4.).round() as u8;
                        dst.put_pixel(tx * next + x, ty * next + y, image::Rgba(p));
                    }
                }
            }
        }
        result.push(dst);
        tile = next;
    }
    result
}
#[cfg(test)]
mod tests {
    #[test]
    fn adjacent_tiles_never_mix_at_any_mip() {
        let image = image::RgbaImage::from_fn(8, 4, |x, _| {
            if x < 4 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        for level in super::atlas(&image, 2, 1) {
            for (x, _, p) in level.enumerate_pixels() {
                assert_eq!(
                    p.0,
                    if x < level.width() / 2 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 0, 255, 255]
                    }
                );
            }
        }
    }
}
