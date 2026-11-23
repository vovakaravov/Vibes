use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};
use lofty::config::ParseOptions;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

pub const ICON_MAX_DIM: u32 = 96;
pub const COVER_MAX_DIM: u32 = 256;

#[cfg(target_env = "gnu")]
pub fn trim_allocator() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> std::ffi::c_int;
    }
    // SAFETY: glibc's `malloc_trim(0)` is thread-safe and releases unused
    // free heap/arena pages back to the OS without affecting live allocations.
    unsafe {
        let _ = malloc_trim(0);
    }
}

#[cfg(not(target_env = "gnu"))]
pub fn trim_allocator() {}

const RASTER_EXTS: &[&str] = &["jpg", "png", "webp", "gif", "bmp", "ico"];
const MAX_ART_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LOCAL_COVER_BYTES: u64 = 16 * 1024 * 1024;
const COVER_FILENAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "cover.webp",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.png",
    "album.jpg",
    "album.png",
];

pub fn art_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("vibes")
        .join("art")
}

pub fn fnv1a_hex(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub fn detect_image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("jpg");
    }
    if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1A\n" {
        return Some("png");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        return Some("gif");
    }
    if bytes.len() >= 14 && &bytes[..2] == b"BM" {
        return Some("bmp");
    }
    if bytes.len() >= 22 && &bytes[..4] == b"\x00\x00\x01\x00" {
        return Some("ico");
    }
    None
}

pub fn youtube_thumb_url(video_id: &str) -> Option<String> {
    let id = video_id.trim();
    if id.is_empty() {
        None
    } else {
        Some(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"))
    }
}

pub fn prepare_cached_thumbnail(bytes: &[u8], max_dim: u32) -> Option<(&'static str, Vec<u8>)> {
    let orig_ext = detect_image_ext(bytes)?;
    let dims = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok());
    if let Some((w, h)) = dims
        && w > 0
        && h > 0
        && w <= max_dim
        && h <= max_dim
    {
        return Some((orig_ext, bytes.to_vec()));
    }

    let img = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        image::load_from_memory(bytes).ok()
    }))
    .ok()
    .flatten()?;

    if img.width() <= max_dim && img.height() <= max_dim {
        return Some((orig_ext, bytes.to_vec()));
    }

    let resized = img.resize(max_dim, max_dim, FilterType::Triangle);
    if resized.color().has_alpha() {
        let mut out = Cursor::new(Vec::new());
        resized.write_to(&mut out, ImageFormat::Png).ok()?;
        Some(("png", out.into_inner()))
    } else {
        let rgb = resized.to_rgb8();
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 85)
            .encode_image(&rgb)
            .ok()?;
        Some(("jpg", out))
    }
}

fn cleanup_legacy_art_cache() {
    let dir = art_cache_dir();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
            && (name.starts_with("emb-") || name.starts_with("url-"))
        {
            let _ = fs::remove_file(path);
        }
    }
}

fn find_cached_file(prefix: &str, key_hash: &str) -> Option<PathBuf> {
    let dir = art_cache_dir();
    for ext in RASTER_EXTS {
        let p = dir.join(format!("{prefix}-{key_hash}.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn atomic_write_cache(prefix: &str, key_hash: &str, ext: &str, bytes: &[u8]) -> Option<PathBuf> {
    let dir = art_cache_dir();
    fs::create_dir_all(&dir).ok()?;
    let final_path = dir.join(format!("{prefix}-{key_hash}.{ext}"));
    if final_path.is_file() {
        return Some(final_path);
    }
    let tid = format!("{:?}", thread::current().id());
    let tmp_name = format!(
        ".{prefix}-{key_hash}-{}-{}.tmp",
        std::process::id(),
        fnv1a_hex(&tid)
    );
    let tmp_path = dir.join(tmp_name);
    if fs::write(&tmp_path, bytes).is_err() {
        let _ = fs::remove_file(&tmp_path);
        return None;
    }
    if fs::rename(&tmp_path, &final_path).is_err() {
        let _ = fs::remove_file(&tmp_path);
        return final_path.is_file().then_some(final_path);
    }
    Some(final_path)
}

fn file_fingerprint_hash(path: &Path, extra: &str) -> Option<(u64, String)> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let key = format!("{}|{mtime}|{}|{extra}", path.display(), meta.len());
    Some((meta.len(), fnv1a_hex(&key)))
}

pub fn find_folder_cover(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.is_file() {
                let name = p.file_name()?.to_str()?.to_lowercase();
                Some((name, p))
            } else {
                None
            }
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    for target in COVER_FILENAMES {
        if let Some((_, p)) = files.iter().find(|(name, _)| name == target) {
            return Some(p.clone());
        }
    }
    None
}

pub fn cache_local_image_file(image_path: &Path, max_dim: u32) -> Option<PathBuf> {
    let (len, hash) = file_fingerprint_hash(image_path, &max_dim.to_string())?;
    if len > MAX_LOCAL_COVER_BYTES {
        return None;
    }
    if let Some(cached) = find_cached_file("t2-cov", &hash) {
        return Some(cached);
    }
    let bytes = fs::read(image_path).ok()?;
    let (ext, out) = prepare_cached_thumbnail(&bytes, max_dim)?;
    atomic_write_cache("t2-cov", &hash, ext, &out)
}

pub fn extract_embedded_art(audio_path: &Path) -> Option<PathBuf> {
    let (_, hash) = file_fingerprint_hash(audio_path, "")?;
    if let Some(cached) = find_cached_file("t2-emb", &hash) {
        return Some(cached);
    }

    let tagged = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Probe::open(audio_path)
            .ok()
            .and_then(|p| p.guess_file_type().ok())
            .and_then(|p| {
                p.options(
                    ParseOptions::new()
                        .read_properties(false)
                        .read_cover_art(true),
                )
                .read()
                .ok()
            })
    }))
    .ok()
    .flatten()?;

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    for pic in tag.pictures() {
        if let Some((ext, out)) = prepare_cached_thumbnail(pic.data(), COVER_MAX_DIM) {
            return atomic_write_cache("t2-emb", &hash, ext, &out);
        }
    }
    None
}

pub fn resolve_local_album_art(folder: &Path, tracks: &[PathBuf]) -> Option<PathBuf> {
    if let Some(folder_cov) = find_folder_cover(folder) {
        return cache_local_image_file(&folder_cov, COVER_MAX_DIM).or(Some(folder_cov));
    }
    for track_path in tracks.iter().take(3) {
        if let Some(art) = extract_embedded_art(track_path) {
            return Some(art);
        }
    }
    None
}

pub fn resolve_local_single_art(track_path: &Path) -> Option<PathBuf> {
    extract_embedded_art(track_path)
}

pub struct ArtworkFetcher {
    url_tx: async_channel::Sender<(String, u32)>,
    submitted: Arc<Mutex<HashSet<String>>>,
}

fn fetch_one_url(agent: &ureq::Agent, url: &str, max_dim: u32) -> (Option<PathBuf>, bool) {
    let hash = fnv1a_hex(&format!("{max_dim}|{url}"));
    if let Some(cached) = find_cached_file("t2-url", &hash) {
        return (Some(cached), false);
    }
    let Ok(mut resp) = agent.get(url).header("User-Agent", "Vibes/0.1").call() else {
        return (None, true);
    };
    let Ok(bytes) = resp
        .body_mut()
        .with_config()
        .limit(MAX_ART_BYTES)
        .read_to_vec()
    else {
        return (None, true);
    };
    let Some((ext, out)) = prepare_cached_thumbnail(&bytes, max_dim) else {
        return (None, true);
    };
    (atomic_write_cache("t2-url", &hash, ext, &out), true)
}

impl ArtworkFetcher {
    pub fn spawn(on_loaded: impl Fn(String, PathBuf) + Send + Sync + 'static) -> Self {
        let (url_tx, url_rx) = async_channel::unbounded::<(String, u32)>();
        let submitted = Arc::new(Mutex::new(HashSet::new()));
        let cb = Arc::new(on_loaded);
        let shared_agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(6)))
            .max_redirects(4)
            .build()
            .into();

        for idx in 0..2 {
            let rx = url_rx.clone();
            let cb_worker = Arc::clone(&cb);
            let agent = shared_agent.clone();
            let _ = thread::Builder::new()
                .name(format!("vibes-art-{idx}"))
                .spawn(move || {
                    if idx == 0 {
                        cleanup_legacy_art_cache();
                    }

                    let mut did_work = false;
                    while let Ok((url, max_dim)) = rx.recv_blocking() {
                        let (path_opt, fetched) = fetch_one_url(&agent, &url, max_dim);
                        if fetched {
                            did_work = true;
                        }
                        if let Some(path) = path_opt {
                            cb_worker(url, path);
                        }
                        if did_work && rx.is_empty() {
                            trim_allocator();
                            did_work = false;
                        }
                    }
                });
        }

        Self { url_tx, submitted }
    }

    pub fn prefetch(&self, url: &str, max_dim: u32) {
        let clean = url.trim();
        if clean.is_empty()
            || (!clean.starts_with("http://") && !clean.starts_with("https://"))
        {
            return;
        }
        let Ok(mut guard) = self.submitted.lock() else {
            return;
        };
        if !guard.insert(clean.to_string()) {
            return;
        }
        drop(guard);
        let _ = self.url_tx.try_send((clean.to_string(), max_dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage, RgbaImage};

    #[test]
    fn detects_raster_magic_bytes_and_rejects_html() {
        assert_eq!(detect_image_ext(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(detect_image_ext(b"\x89PNG\r\n\x1A\n0000"), Some("png"));
        assert_eq!(detect_image_ext(b"RIFF1234WEBPabcd"), Some("webp"));
        assert_eq!(detect_image_ext(b"GIF89a123"), Some("gif"));
        assert_eq!(
            detect_image_ext(b"BM\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"),
            Some("bmp")
        );
        assert_eq!(detect_image_ext(b"<html><body>404</body></html>"), None);
        assert_eq!(detect_image_ext(b"<svg></svg>"), None);
        assert_eq!(detect_image_ext(b""), None);
    }

    #[test]
    fn finds_folder_cover_and_formats_youtube_thumb_url() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(find_folder_cover(tmp.path()), None);

        let cover = tmp.path().join("Folder.JPG");
        fs::write(&cover, b"dummy").unwrap();
        assert_eq!(find_folder_cover(tmp.path()), Some(cover.clone()));

        // `cover.jpg` has higher priority in COVER_FILENAMES than `folder.jpg`.
        let higher = tmp.path().join("cover.jpg");
        fs::write(&higher, b"dummy").unwrap();
        assert_eq!(find_folder_cover(tmp.path()), Some(higher));

        assert_eq!(
            youtube_thumb_url("  dQw4w9WgXcQ "),
            Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/mqdefault.jpg".to_string())
        );
        assert_eq!(youtube_thumb_url("   "), None);
    }

    #[test]
    fn downscales_oversized_images_and_preserves_alpha() {
        // 600x400 RGB JPEG -> 256x171 JPEG
        let rgb = DynamicImage::ImageRgb8(RgbImage::from_pixel(600, 400, image::Rgb([12, 80, 200])));
        let mut jpg_bytes = Cursor::new(Vec::new());
        rgb.write_to(&mut jpg_bytes, ImageFormat::Jpeg).unwrap();
        let (ext, thumb) = prepare_cached_thumbnail(jpg_bytes.get_ref(), COVER_MAX_DIM).unwrap();
        assert_eq!(ext, "jpg");
        let dims = ImageReader::new(Cursor::new(&thumb))
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .unwrap();
        assert_eq!(dims, (256, 171));

        // 512x512 RGBA PNG -> 96x96 PNG
        let rgba =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(512, 512, image::Rgba([0, 200, 90, 128])));
        let mut png_bytes = Cursor::new(Vec::new());
        rgba.write_to(&mut png_bytes, ImageFormat::Png).unwrap();
        let (ext_png, icon_thumb) =
            prepare_cached_thumbnail(png_bytes.get_ref(), ICON_MAX_DIM).unwrap();
        assert_eq!(ext_png, "png");
        let icon_dims = ImageReader::new(Cursor::new(&icon_thumb))
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .unwrap();
        assert_eq!(icon_dims, (96, 96));

        // Already-small 48x48 PNG passes through unchanged
        let small =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(48, 48, image::Rgba([255, 10, 20, 255])));
        let mut small_bytes = Cursor::new(Vec::new());
        small.write_to(&mut small_bytes, ImageFormat::Png).unwrap();
        let raw_small = small_bytes.into_inner();
        let (small_ext, out_small) = prepare_cached_thumbnail(&raw_small, ICON_MAX_DIM).unwrap();
        assert_eq!(small_ext, "png");
        assert_eq!(out_small, raw_small);
    }
}
