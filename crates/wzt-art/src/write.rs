//! Indexed PNG output with a streamed nearest-neighbour upscale.
//!
//! A layer is generated at logical resolution and written at device
//! resolution. The device-size buffer never exists: each logical row is
//! expanded horizontally once into a single output row (and bit-packed when
//! the palette is small), and that row is handed to the PNG encoder `scale`
//! times. A 6016x3384 layer at scale 4 holds a 1504x846 buffer in memory and
//! streams 3384 rows out.
//!
//! The file is an 8-bit-or-smaller indexed PNG with `PLTE` and `tRNS`
//! chunks, so one byte (or fewer bits) per pixel and the flat runs of an
//! upscaled image compress very well. Entry 0 is transparent.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use png::{BitDepth, ColorType, Compression, Filter};

use crate::error::{ArtError, Result};
use crate::palette::Layer;

/// Bits per index: the smallest PNG depth that holds the palette.
fn depth_for(colors: usize) -> (BitDepth, u32) {
    match colors {
        0..=2 => (BitDepth::One, 1),
        3..=4 => (BitDepth::Two, 2),
        5..=16 => (BitDepth::Four, 4),
        _ => (BitDepth::Eight, 8),
    }
}

/// Pack one row of indices MSB-first into `bits`-wide fields.
fn pack_row(indices: &[u8], bits: u32, out: &mut Vec<u8>) {
    out.clear();
    if bits == 8 {
        out.extend_from_slice(indices);
        return;
    }
    let per_byte = (8 / bits) as usize;
    for chunk in indices.chunks(per_byte) {
        let mut byte = 0u8;
        for (i, &v) in chunk.iter().enumerate() {
            byte |= v << (8 - bits as usize * (i + 1));
        }
        out.push(byte);
    }
}

/// Dimensions of the written image.
pub fn output_size(layer: &Layer, scale: u32) -> (u32, u32) {
    (layer.width * scale, layer.height * scale)
}

/// Write `layer` as an indexed PNG at `scale` times its size.
pub fn write_png<W: Write>(out: W, layer: &Layer, scale: u32) -> Result<()> {
    assert!(scale >= 1, "scale is validated before writing");
    let (out_w, out_h) = output_size(layer, scale);

    let plte: Vec<u8> = layer
        .palette
        .iter()
        .flat_map(|c| [c[0], c[1], c[2]])
        .collect();
    // tRNS may stop early: entries past its end are opaque.
    let last_translucent = layer.palette.iter().rposition(|c| c[3] != 255);
    let trns: Vec<u8> = layer.palette[..last_translucent.map_or(0, |i| i + 1)]
        .iter()
        .map(|c| c[3])
        .collect();
    let (depth, bits) = depth_for(layer.palette.len());

    let mut encoder = png::Encoder::new(out, out_w, out_h);
    encoder.set_color(ColorType::Indexed);
    encoder.set_depth(depth);
    encoder.set_palette(plte);
    if !trns.is_empty() {
        encoder.set_trns(trns);
    }
    // Duplicated rows become runs of zeros under `Up`, which deflate loves.
    encoder.set_compression(Compression::Fast);
    encoder.set_filter(if scale > 1 {
        Filter::Up
    } else {
        Filter::Adaptive
    });

    let mut writer = encoder.write_header()?;
    let mut stream = writer.stream_writer()?;
    let mut expanded = vec![0u8; out_w as usize];
    let mut packed = Vec::new();
    for y in 0..layer.height {
        let row = &layer.pixels[y as usize * layer.width as usize..][..layer.width as usize];
        for (x, &index) in row.iter().enumerate() {
            expanded[x * scale as usize..(x + 1) * scale as usize].fill(index);
        }
        pack_row(&expanded, bits, &mut packed);
        for _ in 0..scale {
            stream
                .write_all(&packed)
                .map_err(png::EncodingError::from)?;
        }
    }
    stream.finish()?;
    writer.finish()?;
    Ok(())
}

/// Write to `path` through a temporary file, so a reader (WezTerm watching
/// the directory) sees the old image or the new one, never half of one.
pub fn write_png_file(path: &Path, layer: &Layer, scale: u32) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| ArtError::io(parent, e))?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .map_or_else(|| "layer".into(), |n| n.to_string_lossy())
    ));
    {
        let file = fs::File::create(&tmp).map_err(|e| ArtError::io(&tmp, e))?;
        let mut buffered = BufWriter::new(file);
        write_png(&mut buffered, layer, scale)?;
        buffered.flush().map_err(|e| ArtError::io(&tmp, e))?;
    }
    fs::rename(&tmp, path).map_err(|e| ArtError::io(path, e))
}

/// A decoded PNG, for tests and for reading golden images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPng {
    pub width: u32,
    pub height: u32,
    /// `width * height` RGBA pixels, row-major, from the palette.
    pub rgba: Vec<[u8; 4]>,
    /// Whether the file was indexed colour.
    pub indexed: bool,
}

/// Decode any PNG to RGBA, expanding the palette and `tRNS`.
pub fn read_png(path: &Path) -> Result<DecodedPng> {
    let file = fs::File::open(path).map_err(|e| ArtError::io(path, e))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND);
    let png_err = |e: png::DecodingError| ArtError::Image {
        path: path.to_path_buf(),
        message: e.to_string(),
    };
    let mut reader = decoder.read_info().map_err(png_err)?;
    let indexed = reader.info().color_type == ColorType::Indexed;
    let mut buf = vec![
        0;
        reader.output_buffer_size().ok_or_else(|| ArtError::Image {
            path: path.to_path_buf(),
            message: "image too large".into(),
        })?
    ];
    let info = reader.next_frame(&mut buf).map_err(png_err)?;
    let (w, h) = (info.width, info.height);
    let bytes = &buf[..info.buffer_size()];
    let rgba: Vec<[u8; 4]> = match info.color_type {
        ColorType::Rgba => bytes
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect(),
        ColorType::Rgb => bytes
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        other => {
            return Err(ArtError::Image {
                path: path.to_path_buf(),
                message: format!("unexpected colour type {other:?} after expansion"),
            });
        }
    };
    Ok(DecodedPng {
        width: w,
        height: h,
        rgba,
        indexed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer_with(colors: usize) -> Layer {
        let palette: Vec<[u8; 4]> = (0..colors)
            .map(|i| {
                if i == 0 {
                    [0, 0, 0, 0]
                } else {
                    [(i * 3 % 256) as u8, 100, 200, 255 - (i % 7) as u8]
                }
            })
            .collect();
        let (w, h) = (7u32, 5u32);
        let pixels = (0..w * h).map(|i| (i as usize % colors) as u8).collect();
        Layer {
            width: w,
            height: h,
            palette,
            pixels,
        }
    }

    fn roundtrip(layer: &Layer, scale: u32) -> DecodedPng {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("l.png");
        write_png_file(&path, layer, scale).unwrap();
        read_png(&path).unwrap()
    }

    #[test]
    fn round_trips_at_every_bit_depth_and_scale() {
        for colors in [2, 3, 4, 5, 16, 17, 200] {
            for scale in [1, 3] {
                let layer = layer_with(colors);
                let png = roundtrip(&layer, scale);
                assert!(png.indexed);
                assert_eq!((png.width, png.height), (7 * scale, 5 * scale));
                for y in 0..png.height {
                    for x in 0..png.width {
                        let want = layer.rgba_at(x / scale, y / scale);
                        let got = png.rgba[(y * png.width + x) as usize];
                        // Fully transparent entries decode with any rgb.
                        if want[3] == 0 {
                            assert_eq!(got[3], 0);
                        } else {
                            assert_eq!(got, want, "colors={colors} scale={scale} ({x},{y})");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn output_dimensions_are_logical_times_scale() {
        let layer = layer_with(4);
        assert_eq!(output_size(&layer, 5), (35, 25));
    }

    #[test]
    fn packing_is_msb_first() {
        let mut out = Vec::new();
        pack_row(&[1, 0, 1, 1, 0, 0, 1, 0, 1], 1, &mut out);
        assert_eq!(out, vec![0b1011_0010, 0b1000_0000]);
        pack_row(&[3, 1, 2], 2, &mut out);
        assert_eq!(out, vec![0b11_01_10_00]);
        pack_row(&[0xA, 0x5], 4, &mut out);
        assert_eq!(out, vec![0xA5]);
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        write_png_file(&dir.path().join("a.png"), &layer_with(4), 2).unwrap();
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.png".to_string()]);
    }
}
