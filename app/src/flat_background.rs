//! Single-colour keying for artwork, screenshots, and simple studio backdrops.

use cutout_core::image::{Plane, Rgba};

const BACKGROUND_TOLERANCE: u8 = 4;
const EDGE_TOLERANCE: u8 = 16;

fn distance(pixel: &[u8], background: &[u8]) -> u8 {
    pixel[0].abs_diff(background[0])
        .max(pixel[1].abs_diff(background[1]))
        .max(pixel[2].abs_diff(background[2]))
}

pub fn mask(image: &Rgba) -> Result<Plane, String> {
    let (width, height) = (image.w as usize, image.h as usize);
    if width == 0 || height == 0 {
        return Err("The layer has no pixels.".into());
    }
    let pixel = |x, y| {
        let start = (y * width + x) * 4;
        &image.data[start..start + 4]
    };
    // Sample each edge at a bounded cost even for a very large layer.
    let edge = (0..width).step_by((width / 512).max(1))
        .flat_map(|x| [pixel(x, 0), pixel(x, height - 1)])
        .chain((1..height - 1).step_by((height / 512).max(1))
            .flat_map(|y| [pixel(0, y), pixel(width - 1, y)]));
    let mut background: &[u8] = &[0, 0, 0];
    let mut votes = 0;
    for sample in edge.clone() {
        if sample[3] != 255 {
            continue;
        }
        if votes == 0 {
            background = sample;
        }
        if distance(sample, background) <= BACKGROUND_TOLERANCE {
            votes += 1;
        } else {
            votes -= 1;
        }
    }
    let (matching, total) = edge.fold((0, 0), |(matching, total), sample| {
        let matches = sample[3] == 255 && distance(sample, background) <= BACKGROUND_TOLERANCE;
        (matching + usize::from(matches), total + 1)
    });
    if matching * 3 < total * 2 {
        return Err("No uniform backdrop found. Try AI Remove Background.".into());
    }

    // A tight colour key includes enclosed gaps in lettering and logos. Keep the
    // transition narrow to preserve similar foreground shades and existing alpha.
    let mut has_background = false;
    let mut has_subject = false;
    let alpha = image.data.chunks_exact(4).enumerate().map(|(index, sample)| {
        if sample[3] != 255 {
            has_subject |= sample[3] > 0;
            return 255;
        }
        let delta = distance(sample, background);
        if delta <= BACKGROUND_TOLERANCE {
            has_background = true;
            return 0;
        }
        if delta < EDGE_TOLERANCE {
            let x = index % width;
            let y = index / width;
            let keyed = |x, y| {
                let neighbor = pixel(x, y);
                neighbor[3] == 255 && distance(neighbor, background) <= BACKGROUND_TOLERANCE
            };
            if (x > 0 && keyed(x - 1, y))
                || (x + 1 < width && keyed(x + 1, y))
                || (y > 0 && keyed(x, y - 1))
                || (y + 1 < height && keyed(x, y + 1))
            {
                let edge = u16::from(delta - BACKGROUND_TOLERANCE);
                return (edge * 255 / u16::from(EDGE_TOLERANCE - BACKGROUND_TOLERANCE)) as u8;
            }
        }
        has_subject = true;
        255
    }).collect();
    if !has_background || !has_subject {
        return Err("No foreground separates from the backdrop. Try AI Remove Background.".into());
    }
    Plane::from_vec(image.w, image.h, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(image: &mut Rgba, x: u32, y: u32, color: [u8; 4]) {
        let index = image.idx(x, y);
        image.make_mut()[index..index + 4].copy_from_slice(&color);
    }

    #[test]
    fn keys_white_black_and_colored_backdrops_including_enclosed_holes() {
        for (backdrop, ink) in [
            ([255, 255, 255, 255], [0, 0, 0, 255]),
            ([0, 0, 0, 255], [255, 255, 255, 255]),
            ([35, 112, 200, 255], [220, 60, 35, 255]),
        ] {
            let mut image = Rgba::filled(12, 10, backdrop);
            for y in 2..8 {
                for x in 2..8 {
                    if x == 2 || x == 7 || y == 2 || y == 7 {
                        set(&mut image, x, y, ink);
                    }
                }
            }
            set(&mut image, 10, 6, ink);
            let result = mask(&image).unwrap();
            assert_eq!((result.w, result.h), (12, 10));
            assert_eq!(result.get(0, 0), 0);
            assert_eq!(result.get(4, 4), 0);
            assert_eq!(result.get(2, 4), 255);
            assert_eq!(result.get(10, 6), 255);
        }
    }

    #[test]
    fn retains_existing_alpha_and_softens_only_near_the_backdrop() {
        let mut image = Rgba::filled(9, 9, [255; 4]);
        for y in 2..7 {
            for x in 2..7 {
                set(&mut image, x, y, [10, 10, 10, 255]);
            }
        }
        set(&mut image, 1, 4, [246, 246, 246, 255]);
        set(&mut image, 4, 4, [246, 246, 246, 255]);
        set(&mut image, 1, 3, [255, 255, 255, 127]);
        set(&mut image, 0, 0, [255, 255, 255, 0]);
        let result = mask(&image).unwrap();
        assert!((1..255).contains(&result.get(1, 4)));
        assert_eq!(result.get(4, 4), 255);
        assert_eq!(result.get(1, 3), 255);
        assert_eq!(result.get(0, 0), 255);
    }

    #[test]
    fn rejects_uniform_images_and_multicolored_borders() {
        let mut image = Rgba::filled(8, 8, [255; 4]);
        assert!(mask(&image).is_err());
        for y in 0..8 {
            for x in 4..8 {
                set(&mut image, x, y, [0, 0, 0, 255]);
            }
        }
        assert!(mask(&image).is_err());
    }
}
