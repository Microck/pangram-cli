//! Converts the approved Pangram fox GIF into deterministic terminal cells.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;

use gif::{ColorOutput, DecodeOptions};
use microck_pangram_cli::domain::Sha256Hash;

const SOURCE_PATH: &str = "assets/brand/pangram-fox-source.gif";
const OUTPUT_PATH: &str = "src/tui/intro-frames.rs";
const SOURCE_SHA256: &str = "fa806f95e5775e9bc4ffda599a540910edd2042115eae80729308b02d89a542e";
const SOURCE_WIDTH: u16 = 1_772;
const SOURCE_HEIGHT: u16 = 709;
const SOURCE_FRAME_COUNT: usize = 9;
const SOURCE_FRAME_DELAY: u16 = 7;

const ART_WIDTH: usize = 72;
const ART_HEIGHT: usize = 16;
const DRAW_HEIGHT: usize = 14;
const CYCLE_FRAME_COUNT: usize = 14;
const DISSOLVE_FRAME_COUNT: usize = 8;
const PLAYBACK_FRAME_COUNT: usize = 56;

struct SourceFrame {
    rgba: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Crop {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
}

fn main() -> Result<(), Box<dyn Error>> {
    let check_only = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => false,
        [argument] if argument == "--check" => true,
        _ => return Err("usage: generate-intro-frames [--check]".into()),
    };
    let source = fs::read(SOURCE_PATH)?;
    require(
        Sha256Hash::digest(&source).to_string() == SOURCE_SHA256,
        "approved fox GIF hash does not match metadata",
    )?;

    let frames = decode_frames(&source)?;
    let crop = alpha_bounds(&frames)?;
    let cycle = generate_cycle(&frames, crop);
    let dissolves = generate_dissolves(&cycle);
    let generated = render_module(&cycle, &dissolves);

    if check_only {
        require(
            fs::read(OUTPUT_PATH)?.as_slice() == generated.as_bytes(),
            "generated intro frames differ from the committed table",
        )?;
        println!("verified {OUTPUT_PATH} against {SOURCE_PATH}");
    } else {
        fs::write(OUTPUT_PATH, generated)?;
        println!("generated {OUTPUT_PATH} from {SOURCE_PATH}");
    }
    Ok(())
}

fn decode_frames(source: &[u8]) -> Result<Vec<SourceFrame>, Box<dyn Error>> {
    let mut options = DecodeOptions::new();
    options.set_color_output(ColorOutput::RGBA);
    options.check_frame_consistency(true);
    let mut decoder = options.read_info(source)?;

    require(
        decoder.width() == SOURCE_WIDTH && decoder.height() == SOURCE_HEIGHT,
        "approved fox GIF dimensions changed",
    )?;

    let mut frames = Vec::with_capacity(SOURCE_FRAME_COUNT);
    while let Some(frame) = decoder.read_next_frame()? {
        require(
            frame.left == 0
                && frame.top == 0
                && frame.width == SOURCE_WIDTH
                && frame.height == SOURCE_HEIGHT,
            "approved fox GIF must contain full-canvas frames",
        )?;
        require(
            frame.delay == SOURCE_FRAME_DELAY,
            "approved fox GIF frame delay changed",
        )?;
        frames.push(SourceFrame {
            rgba: frame.buffer.to_vec(),
        });
    }
    require(
        frames.len() == SOURCE_FRAME_COUNT,
        "approved fox GIF frame count changed",
    )?;
    Ok(frames)
}

fn alpha_bounds(frames: &[SourceFrame]) -> Result<Crop, Box<dyn Error>> {
    let width = usize::from(SOURCE_WIDTH);
    let height = usize::from(SOURCE_HEIGHT);
    let mut left = width;
    let mut top = height;
    let mut right = 0;
    let mut bottom = 0;

    for frame in frames {
        for y in 0..height {
            for x in 0..width {
                if frame.rgba[(y * width + x) * 4 + 3] > 8 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
    }

    require(left < right && top < bottom, "approved fox GIF is empty")?;
    Ok(Crop {
        left,
        top,
        width: right - left,
        height: bottom - top,
    })
}

fn generate_cycle(frames: &[SourceFrame], crop: Crop) -> Vec<Vec<u8>> {
    (0..CYCLE_FRAME_COUNT)
        .map(|target_index| {
            let source_index = target_index * SOURCE_FRAME_COUNT / CYCLE_FRAME_COUNT;
            resample(&frames[source_index], crop)
        })
        .collect()
}

fn resample(frame: &SourceFrame, crop: Crop) -> Vec<u8> {
    let mut cells = vec![0; ART_WIDTH * ART_HEIGHT];
    let vertical_offset = (ART_HEIGHT - DRAW_HEIGHT) / 2;

    for target_y in 0..DRAW_HEIGHT {
        let source_top = crop.top + target_y * crop.height / DRAW_HEIGHT;
        let source_bottom = crop.top + (target_y + 1) * crop.height / DRAW_HEIGHT;
        for target_x in 0..ART_WIDTH {
            let source_left = crop.left + target_x * crop.width / ART_WIDTH;
            let source_right = crop.left + (target_x + 1) * crop.width / ART_WIDTH;
            cells[(target_y + vertical_offset) * ART_WIDTH + target_x] = sample_cell(
                &frame.rgba,
                source_left,
                source_right,
                source_top,
                source_bottom,
            );
        }
    }
    cells
}

fn sample_cell(rgba: &[u8], left: usize, right: usize, top: usize, bottom: usize) -> u8 {
    let source_width = usize::from(SOURCE_WIDTH);
    let mut alpha_sum = 0_u64;
    let mut red_sum = 0_u64;
    let mut green_sum = 0_u64;
    let mut blue_sum = 0_u64;

    for y in top..bottom.max(top + 1) {
        for x in left..right.max(left + 1) {
            let pixel = (y * source_width + x) * 4;
            let alpha = u64::from(rgba[pixel + 3]);
            alpha_sum += alpha;
            red_sum += u64::from(rgba[pixel]) * alpha;
            green_sum += u64::from(rgba[pixel + 1]) * alpha;
            blue_sum += u64::from(rgba[pixel + 2]) * alpha;
        }
    }

    let pixel_count = (right.max(left + 1) - left) * (bottom.max(top + 1) - top);
    let average_alpha = alpha_sum / u64::try_from(pixel_count).expect("cell area fits u64");
    if average_alpha < 18 || alpha_sum == 0 {
        return 0;
    }

    let red = red_sum / alpha_sum;
    let green = green_sum / alpha_sum;
    let blue = blue_sum / alpha_sum;
    let palette = nearest_source_palette(red, green, blue);
    let density = if average_alpha < 76 {
        0
    } else if average_alpha < 176 {
        1
    } else {
        2
    };
    1 + palette * 3 + density
}

fn nearest_source_palette(red: u64, green: u64, blue: u64) -> u8 {
    // These are the four semantic colors in the approved source. The renderer
    // maps them to Pangram's terminal palette after geometry is fixed.
    const SOURCE_PALETTE: [[u64; 3]; 4] =
        [[24, 17, 12], [213, 91, 0], [237, 188, 137], [243, 231, 228]];
    SOURCE_PALETTE
        .iter()
        .enumerate()
        .min_by_key(|(_, color)| {
            red.abs_diff(color[0]).pow(2)
                + green.abs_diff(color[1]).pow(2)
                + blue.abs_diff(color[2]).pow(2)
        })
        .map(|(index, _)| u8::try_from(index).expect("palette index fits u8"))
        .expect("source palette is not empty")
}

fn generate_dissolves(cycle: &[Vec<u8>]) -> Vec<Vec<u8>> {
    (0..DISSOLVE_FRAME_COUNT)
        .map(|phase| {
            let mut frame = cycle
                [(PLAYBACK_FRAME_COUNT - DISSOLVE_FRAME_COUNT + phase) % CYCLE_FRAME_COUNT]
                .clone();
            for y in 0..ART_HEIGHT {
                for x in 0..ART_WIDTH {
                    let score = (x * 5 + y * 3 + (x * y) % 7) % DISSOLVE_FRAME_COUNT;
                    if score <= phase {
                        frame[y * ART_WIDTH + x] = 0;
                    }
                }
            }
            frame
        })
        .collect()
}

fn render_module(cycle: &[Vec<u8>], dissolves: &[Vec<u8>]) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "// @generated by tools/generate-intro-frames.rs; do not edit."
    )
    .unwrap();
    writeln!(output, "// Source SHA-256: {SOURCE_SHA256}").unwrap();
    writeln!(output, "pub(crate) const ART_WIDTH: usize = {ART_WIDTH};").unwrap();
    writeln!(output, "pub(crate) const ART_HEIGHT: usize = {ART_HEIGHT};").unwrap();
    writeln!(
        output,
        "pub(crate) const UNIQUE_FRAME_COUNT: usize = {};",
        cycle.len() + dissolves.len()
    )
    .unwrap();
    writeln!(
        output,
        "pub(crate) const ART_FRAMES: [[&str; ART_HEIGHT]; UNIQUE_FRAME_COUNT] = ["
    )
    .unwrap();
    for frame in cycle.iter().chain(dissolves) {
        writeln!(output, "    [").unwrap();
        for row in frame.chunks_exact(ART_WIDTH) {
            let encoded: String = row.iter().copied().map(encode_cell).collect();
            writeln!(output, "        {encoded:?},").unwrap();
        }
        writeln!(output, "    ],").unwrap();
    }
    writeln!(output, "];\n").unwrap();

    let sequence = playback_sequence();
    writeln!(
        output,
        "pub(crate) const FRAME_SEQUENCE: [usize; {PLAYBACK_FRAME_COUNT}] = {sequence:?};"
    )
    .unwrap();
    output
}

fn playback_sequence() -> [usize; PLAYBACK_FRAME_COUNT] {
    std::array::from_fn(|index| {
        if index < PLAYBACK_FRAME_COUNT - DISSOLVE_FRAME_COUNT {
            index % CYCLE_FRAME_COUNT
        } else {
            CYCLE_FRAME_COUNT + index - (PLAYBACK_FRAME_COUNT - DISSOLVE_FRAME_COUNT)
        }
    })
}

fn encode_cell(cell: u8) -> char {
    match cell {
        0 => ' ',
        1..=12 => char::from(b'a' + cell - 1),
        _ => unreachable!("generator produced an unknown cell"),
    }
}

fn require(condition: bool, message: &'static str) -> Result<(), io::Error> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_replaces_the_fourth_cycles_final_eight_frames_with_the_dissolve() {
        let sequence = playback_sequence();
        let dissolve_start = PLAYBACK_FRAME_COUNT - DISSOLVE_FRAME_COUNT;

        for (index, frame) in sequence[..dissolve_start].iter().enumerate() {
            assert_eq!(*frame, index % CYCLE_FRAME_COUNT);
        }
        assert_eq!(
            &sequence[dissolve_start..],
            &[14, 15, 16, 17, 18, 19, 20, 21]
        );
    }

    #[test]
    fn final_dissolve_frame_is_empty() {
        let cycle = vec![vec![12; ART_WIDTH * ART_HEIGHT]; CYCLE_FRAME_COUNT];
        let dissolves = generate_dissolves(&cycle);
        assert!(dissolves.last().unwrap().iter().all(|cell| *cell == 0));
        assert!(dissolves.first().unwrap().iter().any(|cell| *cell != 0));
    }

    #[test]
    fn cell_encoding_is_stable_and_ascii() {
        assert_eq!(encode_cell(0), ' ');
        assert_eq!(encode_cell(1), 'a');
        assert_eq!(encode_cell(12), 'l');
    }

    #[test]
    fn approved_source_colors_keep_four_distinct_roles() {
        assert_eq!(nearest_source_palette(20, 15, 10), 0);
        assert_eq!(nearest_source_palette(213, 91, 0), 1);
        assert_eq!(nearest_source_palette(237, 188, 137), 2);
        assert_eq!(nearest_source_palette(243, 231, 228), 3);
    }

    #[test]
    fn approved_source_hash_matches() {
        let source = fs::read(std::path::Path::new(SOURCE_PATH)).unwrap();
        assert_eq!(Sha256Hash::digest(&source).to_string(), SOURCE_SHA256);
    }
}
