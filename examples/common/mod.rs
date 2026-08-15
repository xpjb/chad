use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
pub fn run_screenshot(render: impl FnOnce(&Path) -> Result<(), String>) -> bool {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--screenshot")) {
        return false;
    }
    let Some(path) = args.next() else {
        eprintln!("--screenshot requires an output path");
        std::process::exit(2);
    };
    if let Err(error) = render(Path::new(&path)) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    true
}

pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let expected_len = width as usize * height as usize * 4;
    if rgba.len() != expected_len {
        return Err(format!(
            "expected {expected_len} tightly packed RGBA8 bytes, got {}",
            rgba.len()
        ));
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let file = File::create(path).map_err(|error| format!("create {}: {error}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("write {} header: {error}", path.display()))?;
    writer
        .write_image_data(rgba)
        .map_err(|error| format!("write {} pixels: {error}", path.display()))
}
