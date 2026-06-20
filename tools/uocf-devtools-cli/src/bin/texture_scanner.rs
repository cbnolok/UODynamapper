use byteorder::{LittleEndian, ReadBytesExt};
use clap::Parser;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use uocf::uop_container::hash::hash_file_name_single;

/// Scan unpacked DDS textures and write a CSV inventory.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Root directory containing unpacked DDS files.
    #[arg(value_name = "INPUT_DIR")]
    input_dir: PathBuf,

    /// CSV inventory output path.
    #[arg(short, long, value_name = "CSV")]
    output: PathBuf,

    /// Optional directory where square opaque land candidates are copied.
    #[arg(long, value_name = "DIR")]
    land_candidates_dir: Option<PathBuf>,
}

#[derive(Debug)]
struct TextureInfo {
    path: String,
    vpath_hash: String,
    width: u32,
    height: u32,
    is_square: u8,
    is_opaque: u8,
    is_land_candidate: u8,
    size: u64,
    unused1: u8,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let root = cli.input_dir;
    let land_dir = cli.land_candidates_dir;
    let mut infos = Vec::new();

    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("input directory does not exist: {}", root.display()),
        ));
    }

    println!("Scanning {}...", root.display());
    if let Some(land_dir) = &land_dir {
        fs::create_dir_all(land_dir)?;
    }

    for entry in walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "dds") {
                match process_dds(path, &root) {
                    Ok(info) => {
                        if info.is_land_candidate == 1 {
                            // Copy to land candidates folder
                            if let Some(land_dir) = &land_dir {
                                let target_path = land_dir.join(info.path.replace('/', "_"));
                                if let Some(parent) = target_path.parent() {
                                    fs::create_dir_all(parent)?;
                                }
                                fs::copy(path, target_path)?;
                            }
                        }
                        infos.push(info);
                    }
                    Err(e) => eprintln!("Warning: Failed to process {}: {}", path.display(), e),
                }
            }
        }
    }

    println!("Found {} DDS files. Sorting...", infos.len());

    // Sort by width, then height
    infos.sort_by(|a, b| a.width.cmp(&b.width).then(a.height.cmp(&b.height)));

    if let Some(parent) = cli.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut wtr = csv::Writer::from_path(&cli.output)?;

    wtr.write_record(&[
        "path",
        "vpath_hash",
        "width",
        "height",
        "is_square",
        "is_opaque",
        "is_land_candidate",
        "unused1",
        "size",
    ])?;

    for info in infos {
        wtr.write_record(&[
            info.path,
            info.vpath_hash,
            info.width.to_string(),
            info.height.to_string(),
            info.is_square.to_string(),
            info.is_opaque.to_string(),
            info.is_land_candidate.to_string(),
            info.unused1.to_string(),
            info.size.to_string(),
        ])?;
    }

    wtr.flush()?;

    println!(
        "Done. CSV saved to {}. Land candidates copied to {}",
        cli.output.display(),
        land_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "disabled".to_string())
    );

    Ok(())
}

fn process_dds(path: &Path, root: &Path) -> io::Result<TextureInfo> {
    let mut file = fs::File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;

    let size = data.len() as u64;

    let rel_path = path
        .strip_prefix(root)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
        .to_string_lossy()
        .into_owned();
    let vpath_hash = if rel_path.starts_with("0x") {
        rel_path
            .split('.')
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty texture path"))?
            .to_string()
    } else {
        format!("0x{:016x}", hash_file_name_single(&rel_path))
    };

    let mut cursor = Cursor::new(&data);
    let mut magic = [0u8; 4];
    cursor.read_exact(&mut magic)?;

    let mut unused1 = 0u8;
    if &magic != b"DDS " {
        cursor.set_position(0);
        let present = cursor.read_u8()? != 0;
        if present {
            unused1 = cursor.read_u8()?;
            let _name_index = cursor.read_i32::<LittleEndian>()?;
            let count1 = cursor.read_u8()?;
            for _ in 0..count1 {
                cursor.set_position(cursor.position() + 17);
            }
            let count2 = cursor.read_u32::<LittleEndian>()?;
            cursor.set_position(cursor.position() + (count2 as u64 * 4));
            let count3 = cursor.read_u32::<LittleEndian>()?;
            cursor.set_position(cursor.position() + (count3 as u64 * 4));

            let mut magic2 = [0u8; 4];
            if cursor.read_exact(&mut magic2).is_ok() && &magic2 == b"DDS " {
                // OK
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Expected DDS magic after EC header",
                ));
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Texture not present in EC header",
            ));
        }
    }

    let dds_data_start = cursor.position() as usize - 4;
    let dds_data = &data[dds_data_start..];

    let mut dds_cursor = Cursor::new(dds_data);
    let dds = ddsfile::Dds::read(&mut dds_cursor).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("DDS read error: {}", e))
    })?;

    let width = dds.get_width();
    let height = dds.get_height();

    let is_square = if width == height { 1 } else { 0 };
    let mut is_opaque = 1u8;

    // Check for opacity if it's a square texture
    if is_square == 1 && width >= 4 && height >= 4 {
        let cursor = Cursor::new(dds_data);
        if let Ok(mut decoder) = dds::Decoder::new(cursor) {
            let size = decoder.main_size();
            let rgba_len = (size.width * size.height * 4) as usize;
            let mut rgba = vec![0u8; rgba_len];
            if let Some(image) =
                dds::ImageViewMut::new(&mut rgba, size, dds::ColorFormat::RGBA_U8)
            {
                if decoder.read_surface(image).is_ok() {
                    for i in 0..(rgba_len / 4) {
                        if rgba[i * 4 + 3] < 255 {
                            is_opaque = 0;
                            break;
                        }
                    }
                }
            }
        }
    } else if is_square == 1 {
        // Very small textures (1x1, 2x2) are hard to decode with some libraries,
        // but they are unlikely to be land textures anyway.
        is_opaque = 1;
    }

    let is_land_candidate = if is_square == 1 && is_opaque == 1 {
        1
    } else {
        0
    };

    Ok(TextureInfo {
        path: rel_path,
        vpath_hash,
        width,
        height,
        is_square,
        is_opaque,
        is_land_candidate,
        size,
        unused1,
    })
}
