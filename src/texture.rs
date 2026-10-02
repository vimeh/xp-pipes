//! Bounded decoding for the four formats offered by XP's texture picker.
//! Original artwork is supplied by the user, never embedded in this package.
use crate::Result;
use image::{ImageFormat, ImageReader, Limits, imageops::FilterType};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_WORK: u64 = 256 * 1024 * 1024;

pub struct Texture {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
}
impl Texture {
    pub fn load(custom: Option<&Path>, fallback: Option<&Path>) -> Result<Option<Self>> {
        if let Some(path) = custom {
            match Self::read(path) {
                Ok(texture) => return Ok(Some(texture)),
                Err(error) if fallback.is_some() => eprintln!(
                    "wlpipes: texture {} failed ({error}); using supplied default",
                    path.display()
                ),
                Err(error) => return Err(format!("texture {}: {error}", path.display()).into()),
            }
        }
        fallback
            .map(|path| {
                Self::read(path)
                    .map_err(|error| format!("default texture {}: {error}", path.display()).into())
            })
            .transpose()
    }

    fn read(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len())? > MAX_BYTES {
            return Err("encoded texture exceeds 64 MiB".into());
        }
        let format = image::guess_format(&bytes).or_else(|_| ImageFormat::from_path(path))?;
        Self::decode(&bytes, format)
    }

    fn decode(bytes: &[u8], format: ImageFormat) -> Result<Self> {
        if !matches!(
            format,
            ImageFormat::Bmp | ImageFormat::Jpeg | ImageFormat::Tga | ImageFormat::Png
        ) {
            return Err("texture must be BMP, JPEG, TGA or PNG".into());
        }
        let mut limits = Limits::default();
        limits.max_image_width = Some(16384);
        limits.max_image_height = Some(16384);
        limits.max_alloc = Some(MAX_BYTES);
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits.clone());
        let (width, height) = reader.into_dimensions()?;
        let size = checked_size([width, height])?;
        let output_bytes = u64::from(size[0]) * u64::from(size[1]) * 4;
        let decoded_bytes = u64::from(width) * u64::from(height) * 4;
        let intermediate = if size == [width, height] {
            0
        } else {
            u64::from(width) * u64::from(size[1]) * 16
        };
        if decoded_bytes + output_bytes + intermediate + u64::try_from(bytes.len())? > MAX_WORK {
            return Err("texture conversion exceeds 256 MiB work budget".into());
        }
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits);
        let rgba = reader.decode()?.into_rgba8();
        let rgba = if size == [width, height] {
            rgba
        } else {
            image::imageops::resize(&rgba, size[0], size[1], FilterType::Triangle)
        };
        Ok(Self {
            size,
            rgba: rgba.into_raw(),
        })
    }
}

fn checked_size(size: [u32; 2]) -> Result<[u32; 2]> {
    if size.into_iter().any(|n| n == 0 || n > 16384) {
        return Err("texture dimension outside 1..16384".into());
    }
    let size = size.map(u32::next_power_of_two);
    if u64::from(size[0]) * u64::from(size[1]) * 4 > MAX_BYTES {
        return Err("decoded/POT texture exceeds 64 MiB".into());
    }
    Ok(size)
}

/// 0100e977: integer viewport/image quotient, multiply 1/8, add 1/2 and truncate.
pub fn repeats(viewport: [u32; 2], texture: [u32; 2]) -> [u16; 2] {
    std::array::from_fn(|i| {
        u16::try_from(((viewport[i] / texture[i] + 4) / 8).max(1))
            .expect("viewport bounded to 16384")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_is_explicit_and_alpha_is_not_premultiplied() {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 10, 30, 0]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        let decoded = Texture::decode(bytes.get_ref(), ImageFormat::Png).unwrap();
        assert_eq!(&decoded.rgba[..4], &[200, 10, 30, 0]);
        let path = std::env::temp_dir().join(format!(
            "wlpipes-texture-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, bytes.get_ref()).unwrap();
        let missing = path.with_extension("missing");
        assert!(Texture::load(Some(&missing), None).is_err());
        assert_eq!(
            Texture::load(Some(&missing), Some(&path))
                .unwrap()
                .unwrap()
                .rgba,
            decoded.rgba
        );
        assert!(Texture::load(None, None).unwrap().is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn decode_formats_and_power_of_two_bounds() {
        let image = image::RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([
                u8::try_from(x * 80).unwrap(),
                u8::try_from(y * 200).unwrap(),
                50,
                255,
            ])
        });
        for format in [
            ImageFormat::Bmp,
            ImageFormat::Png,
            ImageFormat::Tga,
            ImageFormat::Jpeg,
        ] {
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image.clone())
                .into_rgb8()
                .write_to(&mut bytes, format)
                .unwrap();
            let decoded = Texture::decode(bytes.get_ref(), format).unwrap();
            assert_eq!(decoded.size, [4, 2]);
            assert_eq!(decoded.rgba.len(), 32);
            assert!(decoded.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        }
        assert!(Texture::decode(&[0; 32], ImageFormat::Png).is_err());
        for size in [[0, 1], [u32::MAX, 1], [8192, 8192]] {
            assert!(checked_size(size).is_err());
        }
        assert_eq!(checked_size([65, 33]).unwrap(), [128, 64]);
        assert_eq!(repeats([1920, 1080], [64, 64]), [4, 2]);
        assert_eq!(repeats([5120, 2160], [64, 64]), [10, 4]);
        assert_eq!(repeats([640, 480], [64, 64]), [1, 1]);
        assert_eq!(repeats([1280, 800], [64, 64]), [3, 2]);
    }
}
