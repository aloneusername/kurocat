//! "Character from a photo": the user picks one photo of their pet, Lovart's
//! image AI draws two 3×3 sheets of it (nine base poses, then nine idle
//! moments drawn against the first sheet so it stays the same character), and
//! we cut them into a character pack (see `characters.rs`) and switch to it.
//!
//! Drawing goes through the Lovart skill script on this machine
//! (`~/.claude/skills/lovart-api/scripts/agent_skill.py`, overridable with
//! `KUROCAT_LOVART_SCRIPT`), run in the user's login shell so it gets the
//! Lovart keys and the usual `python3`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use image::{Rgba, RgbaImage};
use tauri::{AppHandle, Emitter};

use crate::characters;

pub const PROGRESS_EVENT: &str = "kuro-character-progress";
const PHOTO_EXTENSIONS: [&str; 6] = ["jpg", "jpeg", "png", "heic", "webp", "tiff"];

/// The base poses, in sheet order (3 × 3), with their pack state.
const POSES: [(&str, &str); 9] = [
    ("idle", "idle.png"),
    ("blink", "blink.png"),
    ("typingLeft", "type-left.png"),
    ("typingRight", "type-right.png"),
    ("mouse", "mouse.png"),
    ("mouseClick", "click.png"),
    ("pet", "pet.png"),
    ("sleep", "sleep.png"),
    ("privacy", "privacy.png"),
];

/// The idle moments of the second sheet, in sheet order (3 × 3).
const MOMENTS: [&str; 9] = [
    "look-left.png",
    "look-right.png",
    "look-up.png",
    "yawn-half.png",
    "yawn-wide.png",
    "ear-twitch.png",
    "head-tilt.png",
    "lick-paw.png",
    "smile.png",
];

const STYLE: &str = "画风统一的可爱 Q 版卡通插画，角色要有清晰的深色描边。每格都是正面半身，两只前爪搭在格子下方边缘（像趴在键盘后面），角色在每格里的位置和大小一致，都不要超出自己的格子。整张图是 1:1 的正方形，排成 3 行 3 列，每格大小相同；背景是同一种纯绿色（#00FF00 绿幕绿），不要阴影、渐变、地面线、文字、编号和边框。";

fn base_prompt() -> String {
    format!(
        "把附件照片里的宠物变成一个桌面宠物的卡通角色，要一眼能认出是这只宠物：保留它的毛色和花纹、脸部特征和眼睛颜色；表情和眼神要照着照片画（照片里凶就画凶、温和就画温和），不要改成千篇一律的萌系大眼。画成一张角色设定图，同一个角色画 9 个状态。{STYLE}9 个状态依次是：1 待机；2 眨眼；3 画面左边那只前爪按下、右边那只抬起（打字）；4 画面右边那只前爪按下、左边那只抬起（打字）；5 眼睛看向画面左下方，画面左边那只前爪向左下伸出、像搭在鼠标上；6 和第 5 格一样但那只爪子用力按下、嘴微微张开（点鼠标）；7 被摸头时眯着眼享受、表情放松满足；8 闭眼睡觉；9 两只前爪捂住眼睛。"
    )
}

fn moments_prompt() -> String {
    format!(
        "参照第二张附件（刚画好的角色设定图）再画一张图，必须是同一个角色：外形、毛色花纹、眼睛、描边、画风、角色大小和在格子里的位置都和它完全一致，只改动作和表情。{STYLE}这次画 9 个待机时的小动作，依次是：1 头和眼睛转向画面左边；2 头和眼睛转向画面右边；3 抬头往上看；4 打哈欠刚开始、嘴张开一半；5 打哈欠嘴张到最大、眼睛眯起；6 一只耳朵抖动向外撇；7 歪着头好奇地看；8 抬起一只前爪舔爪子；9 满足地微笑、眼睛弯弯。"
    )
}

static RUNNING: AtomicBool = AtomicBool::new(false);

fn lovart_script() -> PathBuf {
    if let Ok(p) = std::env::var("KUROCAT_LOVART_SCRIPT") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".claude/skills/lovart-api/scripts/agent_skill.py")
}

/// Runs the Lovart script with `args` and parses the JSON it prints last.
fn run_lovart(args: &[&str]) -> Result<serde_json::Value, String> {
    let script = lovart_script();
    if !script.is_file() {
        return Err("这台电脑上没有 Lovart 画图工具".to_string());
    }
    #[cfg(unix)]
    let output = {
        let mut cmd = Command::new("/bin/zsh");
        cmd.arg("-lc").arg("python3 \"$0\" \"$@\"").arg(&script).args(args);
        cmd.output()
    };
    #[cfg(not(unix))]
    let output = Command::new("python").arg(&script).args(args).output();
    let output = output.map_err(|e| format!("无法启动 Lovart 画图工具：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json_start = stdout.find('{').ok_or_else(|| {
        let err = String::from_utf8_lossy(&output.stderr);
        let first = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("没有返回结果");
        format!("Lovart 出错：{}", first.chars().take(60).collect::<String>())
    })?;
    serde_json::from_str(&stdout[json_start..]).map_err(|_| "Lovart 返回的结果看不懂".to_string())
}

/// Copies the photo to `dest` as a PNG no larger than 1600 px (macOS `sips`
/// also reads HEIC from iPhones); elsewhere the file is copied as is.
fn prepare_photo(src: &Path, dest: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let ok = Command::new("/usr/bin/sips")
            .args(["-s", "format", "png", "-Z", "1600"])
            .arg(src)
            .arg("--out")
            .arg(dest)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok && dest.is_file() {
            return Ok(());
        }
        return Err("这张照片打不开，换一张试试".to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::fs::copy(src, dest).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// One finished Lovart drawing: where it was saved and its URL.
struct Drawing {
    path: PathBuf,
    url: String,
    thread_id: String,
}

fn upload_photo(photo: &Path) -> Result<String, String> {
    let photo_str = photo.to_str().ok_or("照片路径有特殊字符")?;
    let uploaded = run_lovart(&["upload", "--file", photo_str])?;
    uploaded["url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "照片上传失败".to_string())
}

fn draw(prompt: &str, attachments: &[&str], thread: Option<&str>, out_dir: &Path) -> Result<Drawing, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let out_str = out_dir.to_str().ok_or("临时目录路径有特殊字符")?;
    let mut args = vec!["chat", "--prompt", prompt, "--json", "--download", "--output-dir", out_str];
    if let Some(t) = thread {
        args.extend(["--thread-id", t]);
    }
    args.push("--attachments");
    args.extend(attachments.iter().copied());
    let result = run_lovart(&args)?;
    match result["final_status"].as_str() {
        Some("done") => {}
        Some("pending_confirmation") => {
            return Err("这次画图要额外确认费用，已取消".to_string());
        }
        Some(other) => return Err(format!("AI 画图没完成（{other}），请重试")),
        None => return Err("AI 画图没有返回结果".to_string()),
    }
    let thread_id = result["thread_id"].as_str().unwrap_or_default().to_string();
    // A reused thread lists its earlier pictures too; the newest one is last.
    result["downloaded"]
        .as_array()
        .and_then(|list| {
            list.iter()
                .rev()
                .filter(|d| d["type"].as_str() == Some("image"))
                .find_map(|d| {
                    let path = PathBuf::from(d["local_path"].as_str()?);
                    let url = d["url"].as_str()?.to_string();
                    path.is_file().then(|| Drawing { path, url, thread_id: thread_id.clone() })
                })
        })
        .ok_or_else(|| "AI 没画出图，请重试".to_string())
}

// ---- cutting the sheet ----

/// The cell's background colour: the median of its border pixels.
fn border_colour(cell: &RgbaImage) -> [u8; 3] {
    let (w, h) = cell.dimensions();
    let mut channels: [Vec<u8>; 3] = [vec![], vec![], vec![]];
    let mut add = |x: u32, y: u32| {
        let p = cell.get_pixel(x, y);
        for c in 0..3 {
            channels[c].push(p[c]);
        }
    };
    for x in 0..w {
        add(x, 0);
        add(x, h - 1);
    }
    for y in 0..h {
        add(0, y);
        add(w - 1, y);
    }
    let mut out = [0u8; 3];
    for c in 0..3 {
        channels[c].sort_unstable();
        out[c] = channels[c][channels[c].len() / 2];
    }
    out
}

fn colour_distance(p: &Rgba<u8>, bg: [u8; 3]) -> f32 {
    let d: f32 = (0..3).map(|c| (p[c] as f32 - bg[c] as f32).powi(2)).sum();
    d.sqrt()
}

/// A saturated background (the green screen we ask for) cannot occur in the
/// pet, so every pixel of that colour is keyed out, even gaps enclosed by
/// paws. A white or grey background could be white fur too, so then only the
/// background connected to the cell border is removed.
fn remove_background(cell: &mut RgbaImage) {
    let (w, h) = cell.dimensions();
    let bg = border_colour(cell);
    let (max, min) = (*bg.iter().max().unwrap(), *bg.iter().min().unwrap());
    let chroma = max as i32 - min as i32 > 90;
    let green_screen = chroma && bg[1] > bg[0] && bg[1] > bg[2];
    let (cut, soft) = if chroma { (90.0, 150.0) } else { (40.0, 80.0) };

    let mut is_bg = vec![false; (w * h) as usize];
    if chroma {
        for (x, y, p) in cell.enumerate_pixels() {
            is_bg[(y * w + x) as usize] = colour_distance(p, bg) < cut;
        }
    } else {
        let mut stack: Vec<(u32, u32)> = Vec::new();
        for x in 0..w {
            stack.push((x, 0));
            stack.push((x, h - 1));
        }
        for y in 0..h {
            stack.push((0, y));
            stack.push((w - 1, y));
        }
        while let Some((x, y)) = stack.pop() {
            let i = (y * w + x) as usize;
            if is_bg[i] || colour_distance(cell.get_pixel(x, y), bg) >= cut {
                continue;
            }
            is_bg[i] = true;
            if x > 0 {
                stack.push((x - 1, y));
            }
            if x + 1 < w {
                stack.push((x + 1, y));
            }
            if y > 0 {
                stack.push((x, y - 1));
            }
            if y + 1 < h {
                stack.push((x, y + 1));
            }
        }
    }

    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if is_bg[i] {
                // Clear the colour too: resizing later blends transparent
                // neighbours in, and a green one would tint the edge.
                *cell.get_pixel_mut(x, y) = Rgba([0, 0, 0, 0]);
                continue;
            }
            let touches_bg = [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 && is_bg[(ny as u32 * w + nx as u32) as usize]
            });
            let px = cell.get_pixel_mut(x, y);
            if touches_bg {
                // Edge pixels that still look like the background fade out.
                let d = colour_distance(px, bg);
                px[3] = (((d - cut) / (soft - cut)).clamp(0.35, 1.0) * 255.0) as u8;
            }
            if green_screen {
                // Remove the green cast the screen leaves on fur edges.
                let limit = px[0].max(px[2]).saturating_add(12);
                if px[1] > limit {
                    px[1] = limit;
                }
            }
        }
    }
}

/// Clears stray bits (e.g. fur of the neighbouring pose that crossed the cell
/// line): only blobs at least 3% the size of the largest one are kept.
fn keep_main_figure(img: &mut RgbaImage) {
    let (w, h) = img.dimensions();
    let mut label = vec![0u32; (w * h) as usize];
    let mut sizes = vec![0usize];
    for start in 0..(w * h) {
        let (sx, sy) = (start % w, start / w);
        if label[start as usize] != 0 || img.get_pixel(sx, sy)[3] <= 40 {
            continue;
        }
        let id = sizes.len() as u32;
        let mut size = 0;
        let mut stack = vec![(sx, sy)];
        label[start as usize] = id;
        while let Some((x, y)) = stack.pop() {
            size += 1;
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let i = (ny as u32 * w + nx as u32) as usize;
                if label[i] == 0 && img.get_pixel(nx as u32, ny as u32)[3] > 40 {
                    label[i] = id;
                    stack.push((nx as u32, ny as u32));
                }
            }
        }
        sizes.push(size);
    }
    let largest = sizes.iter().copied().max().unwrap_or(0);
    for (i, px) in img.pixels_mut().enumerate() {
        let l = label[i] as usize;
        if l == 0 || sizes[l] * 100 < largest * 3 {
            px[3] = 0;
        }
    }
}

/// Opaque pixels in the bottom band of the left / right half: a raised paw
/// leaves its half emptier there than the idle pose.
fn bottom_band_halves(img: &RgbaImage) -> (i64, i64) {
    let (w, h) = img.dimensions();
    let (mut left, mut right) = (0i64, 0i64);
    for y in POSE_BASELINE.saturating_sub(70).min(h)..POSE_BASELINE.min(h) {
        for x in 0..w {
            if img.get_pixel(x, y)[3] > 40 {
                if x < w / 2 {
                    left += 1;
                } else {
                    right += 1;
                }
            }
        }
    }
    (left, right)
}

/// True when this typing pose has its screen-left paw down (right one raised).
fn left_paw_down(idle: &RgbaImage, pose: &RgbaImage) -> bool {
    let (il, ir) = bottom_band_halves(idle);
    let (pl, pr) = bottom_band_halves(pose);
    (il - pl) < (ir - pr)
}

fn opaque_bbox(img: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut l, mut t, mut r, mut b) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 40 {
            l = l.min(x);
            t = t.min(y);
            r = r.max(x);
            b = b.max(y);
        }
    }
    (l <= r && t <= b).then_some((l, t, r + 1, b + 1))
}

const POSE_SIZE: u32 = 512;
const POSE_BASELINE: u32 = 480;

/// One figure cut out of a sheet cell, before scaling and placing.
pub struct Figure(RgbaImage);

/// Cuts a `cols` × `rows` sheet into transparent figures (row by row).
pub fn cut_figures(sheet: &RgbaImage, cols: u32, rows: u32) -> Result<Vec<Figure>, String> {
    let (w, h) = sheet.dimensions();
    if w < 150 * cols || h < 150 * rows {
        return Err("AI 画的图太小了，请重试".to_string());
    }
    let (cw, ch) = (w / cols, h / rows);
    let mut figures = Vec::new();
    for i in 0..cols * rows {
        let mut cell = image::imageops::crop_imm(sheet, (i % cols) * cw, (i / cols) * ch, cw, ch).to_image();
        remove_background(&mut cell);
        keep_main_figure(&mut cell);
        let (l, t, r, b) = opaque_bbox(&cell).ok_or("AI 画的图里有一格是空的，请重试")?;
        if (r - l) * (b - t) < cw * ch / 10 {
            return Err("AI 画的图里有一格太小，请重试".to_string());
        }
        figures.push(Figure(image::imageops::crop_imm(&cell, l, t, r - l, b - t).to_image()));
    }
    Ok(figures)
}

/// Largest scale at which every figure fits the pose canvas.
fn fitting_scale(figures: &[Figure]) -> f32 {
    figures
        .iter()
        .map(|f| {
            let (w, h) = f.0.dimensions();
            ((POSE_SIZE - 32) as f32 / w as f32).min((POSE_BASELINE - 16) as f32 / h as f32)
        })
        .fold(f32::INFINITY, f32::min)
}

fn median_width(figures: &[Figure]) -> f32 {
    let mut widths: Vec<u32> = figures.iter().map(|f| f.0.width()).collect();
    widths.sort_unstable();
    widths[widths.len() / 2] as f32
}

/// Scales every figure by `k` (one scale for a whole sheet, so the character
/// keeps its size) and places it on the shared baseline and centre line.
fn place(figures: &[Figure], k: f32) -> Vec<RgbaImage> {
    figures
        .iter()
        .map(|Figure(fig)| {
            let (w, h) = fig.dimensions();
            let (nw, nh) = (((w as f32 * k) as u32).max(1), ((h as f32 * k) as u32).max(1));
            let scaled = image::imageops::resize(fig, nw, nh, image::imageops::FilterType::Lanczos3);
            let mut canvas = RgbaImage::new(POSE_SIZE, POSE_SIZE);
            let x = POSE_SIZE.saturating_sub(nw) / 2;
            let y = POSE_BASELINE.saturating_sub(nh);
            image::imageops::overlay(&mut canvas, &scaled, x as i64, y as i64);
            canvas
        })
        .collect()
}

/// The base poses: one scale for the sheet, typing poses put on the side of
/// the paw that is actually down (the AI often mixes up left and right).
pub fn base_poses(figures: &[Figure]) -> Vec<RgbaImage> {
    let k = fitting_scale(figures).min(1.6);
    let mut poses = place(figures, k);
    let third_left = left_paw_down(&poses[0], &poses[2]);
    let fourth_left = left_paw_down(&poses[0], &poses[3]);
    if !third_left && fourth_left {
        poses.swap(2, 3);
    }
    poses
}

/// The idle moments, scaled so the character matches the base poses' size.
pub fn moment_poses(base: &[Figure], moments: &[Figure]) -> Vec<RgbaImage> {
    let base_k = fitting_scale(base).min(1.6);
    let k = (base_k * median_width(base) / median_width(moments)).min(fitting_scale(moments));
    place(moments, k)
}

// ---- building the pack ----

fn pack_name() -> String {
    let taken: Vec<String> = characters::load_characters().into_iter().map(|c| c.name).collect();
    let mut n = 1;
    loop {
        let name = if n == 1 { "我的宠物".to_string() } else { format!("我的宠物 {n}") };
        if !taken.contains(&name) {
            return name;
        }
        n += 1;
    }
}

fn write_poses(dir: &Path, files: &[&str], poses: &[RgbaImage]) -> Result<(), String> {
    for (file, img) in files.iter().zip(poses) {
        img.save(dir.join(file)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The idle acts built from the moments; `idle.png` links them back to rest.
fn idle_acts() -> serde_json::Value {
    serde_json::json!([
        { "frames": ["look-left.png", "look-left.png", "idle.png", "look-right.png", "look-right.png", "idle.png"], "fps": 2 },
        { "frames": ["yawn-half.png", "yawn-wide.png", "yawn-wide.png", "yawn-wide.png", "yawn-half.png"], "fps": 4 },
        { "frames": ["ear-twitch.png", "idle.png", "ear-twitch.png"], "fps": 6 },
        { "frames": ["head-tilt.png", "head-tilt.png", "head-tilt.png", "head-tilt.png"], "fps": 2 },
        { "frames": ["lick-paw.png", "idle.png", "lick-paw.png", "idle.png", "lick-paw.png"], "fps": 3 },
        { "frames": ["look-up.png", "look-up.png", "look-up.png"], "fps": 2 },
        { "frames": ["smile.png", "smile.png", "smile.png"], "fps": 2 }
    ])
}

fn write_manifest(dir: &Path, with_moments: bool) -> Result<String, String> {
    let name = pack_name();
    let mut states = serde_json::Map::new();
    for (state, file) in POSES {
        states.insert(state.to_string(), serde_json::Value::String(file.to_string()));
    }
    let mut manifest = serde_json::json!({
        "formatVersion": 1,
        "name": name,
        "emoji": "🐾",
        "author": "照片生成",
        "scale": 1.05,
        "showKeyboard": true,
        "purr": true,
        "states": states,
    });
    if with_moments {
        manifest["idleActs"] = idle_acts();
    }
    std::fs::write(
        dir.join(characters::MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(name)
}

fn open_sheet(path: &Path) -> Result<RgbaImage, String> {
    Ok(image::open(path).map_err(|_| "AI 画的图打不开，请重试".to_string())?.to_rgba8())
}

fn generate(app: &AppHandle, photo_src: &Path) -> Result<(String, String), String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let work = std::env::temp_dir().join(format!("kurocat-photo-{stamp}"));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = (|| {
        let photo = work.join("photo.png");
        prepare_photo(photo_src, &photo)?;
        let photo_url = upload_photo(&photo)?;

        let _ = app.emit(PROGRESS_EVENT, "🎨 AI 正在画基础动作（1/2），大约一分钟…");
        let base = draw(&base_prompt(), &[&photo_url], None, &work.join("base"))?;
        let base_figures = cut_figures(&open_sheet(&base.path)?, 3, 3)?;

        let pack_dir = work.join("pack");
        std::fs::create_dir_all(&pack_dir).map_err(|e| e.to_string())?;
        let pose_files: Vec<&str> = POSES.iter().map(|(_, f)| *f).collect();
        write_poses(&pack_dir, &pose_files, &base_poses(&base_figures))?;
        let _ = std::fs::copy(&photo, pack_dir.join("source-photo.png"));
        let _ = std::fs::copy(&base.path, pack_dir.join("source-sheet.png"));

        // The idle moments make the pet feel alive but are optional: if this
        // second drawing fails, the pack still works without them.
        let _ = app.emit(PROGRESS_EVENT, "🎨 AI 正在画小动作（2/2），大约一分钟…");
        let thread = (!base.thread_id.is_empty()).then_some(base.thread_id.as_str());
        let moments = draw(&moments_prompt(), &[&photo_url, &base.url], thread, &work.join("moments"))
            .and_then(|m| Ok((cut_figures(&open_sheet(&m.path)?, 3, 3)?, m.path)));
        let with_moments = match moments {
            Ok((figures, sheet)) => {
                write_poses(&pack_dir, &MOMENTS, &moment_poses(&base_figures, &figures))?;
                let _ = std::fs::copy(sheet, pack_dir.join("source-sheet-2.png"));
                true
            }
            Err(err) => {
                eprintln!("[kurocat] idle moments skipped: {err}");
                false
            }
        };

        let _ = app.emit(PROGRESS_EVENT, "✂️ 正在整理动作…");
        let name = write_manifest(&pack_dir, with_moments)?;
        let id = format!("photo-{stamp}");
        characters::install_dir(&id, &pack_dir, &["source-photo.png", "source-sheet.png", "source-sheet-2.png"])?;
        Ok((id, name))
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Menu action: pick a pet photo, draw it, install and switch to it.
#[tauri::command]
pub async fn generate_character_from_photo(app: AppHandle) -> Result<Option<String>, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        let msg = "上一张照片还在画，请稍等".to_string();
        let _ = app.emit(characters::ERROR_EVENT, msg.clone());
        return Err(msg);
    }
    let result: Result<Option<String>, String> = async {
        let picked = crate::file_dialog::pick_file(&app, "选一张宠物照片", "照片", &PHOTO_EXTENSIONS).await?;
        let Some(photo) = picked else {
            return Ok(None);
        };
        let _ = app.emit(PROGRESS_EVENT, "📤 正在上传照片…");
        let worker = app.clone();
        let (id, name) = tauri::async_runtime::spawn_blocking(move || generate(&worker, &photo))
            .await
            .map_err(|e| e.to_string())??;
        characters::activate_character(&app, &id);
        let _ = app.emit(characters::IMPORTED_EVENT, name);
        Ok(Some(id))
    }
    .await;
    RUNNING.store(false, Ordering::SeqCst);
    if let Err(msg) = &result {
        let _ = app.emit(characters::ERROR_EVENT, msg.clone());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A green 3×3 sheet with one dark blob per cell, offset differently;
    /// `size` scales the blobs.
    fn fake_sheet(size: u32) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(1536, 1536, Rgba([0, 255, 0, 255]));
        for i in 0..9u32 {
            let (cx, cy) = ((i % 3) * 512 + 230 + i * 5, (i / 3) * 512 + 240 + i * 3);
            for y in cy - size..cy + size {
                for x in cx - size..cx + size - 10 {
                    img.put_pixel(x, y, Rgba([120, 110, 100, 255]));
                }
            }
        }
        img
    }

    #[test]
    fn base_poses_share_baseline_centre_and_size() {
        let poses = base_poses(&cut_figures(&fake_sheet(150), 3, 3).unwrap());
        assert_eq!(poses.len(), 9);
        let widths: Vec<u32> = poses
            .iter()
            .map(|pose| {
                assert_eq!(pose.dimensions(), (POSE_SIZE, POSE_SIZE));
                let (l, _, r, b) = opaque_bbox(pose).unwrap();
                assert_eq!(b, POSE_BASELINE);
                assert!(((l + r) as i32 - POSE_SIZE as i32).abs() <= 1);
                assert_eq!(pose.get_pixel(5, 5)[3], 0, "background must be transparent");
                r - l
            })
            .collect();
        assert!(widths.iter().all(|w| w.abs_diff(widths[0]) <= 2), "one scale per sheet: {widths:?}");
    }

    #[test]
    fn moments_are_scaled_to_the_base_size() {
        let base = cut_figures(&fake_sheet(150), 3, 3).unwrap();
        let small = cut_figures(&fake_sheet(110), 3, 3).unwrap();
        let base_w = {
            let p = &base_poses(&base)[0];
            let (l, _, r, _) = opaque_bbox(p).unwrap();
            r - l
        };
        let moment_w = {
            let p = &moment_poses(&base, &small)[0];
            let (l, _, r, _) = opaque_bbox(p).unwrap();
            r - l
        };
        // Within 2%: the moments may be capped slightly to fit the canvas.
        assert!(base_w.abs_diff(moment_w) * 50 <= base_w, "{base_w} vs {moment_w}");
    }

    #[test]
    fn an_empty_cell_is_rejected() {
        let mut sheet = fake_sheet(150);
        for y in 1024..1536 {
            for x in 1024..1536 {
                sheet.put_pixel(x, y, Rgba([0, 255, 0, 255]));
            }
        }
        assert!(cut_figures(&sheet, 3, 3).is_err());
    }

    #[test]
    fn stray_bits_from_the_neighbouring_cell_are_removed() {
        let mut img = RgbaImage::new(100, 100);
        for y in 30..90 {
            for x in 30..90 {
                img.put_pixel(x, y, Rgba([90, 80, 70, 255]));
            }
        }
        img.put_pixel(2, 50, Rgba([90, 80, 70, 255]));
        keep_main_figure(&mut img);
        assert_eq!(img.get_pixel(2, 50)[3], 0);
        assert_eq!(opaque_bbox(&img), Some((30, 30, 90, 90)));
    }

    fn pose_with_paws(left_down: bool, right_down: bool) -> RgbaImage {
        let mut img = RgbaImage::new(POSE_SIZE, POSE_SIZE);
        for y in 200..POSE_BASELINE - 70 {
            for x in 120..392 {
                img.put_pixel(x, y, Rgba([90, 80, 70, 255]));
            }
        }
        for (down, x0) in [(left_down, 130u32), (right_down, 300u32)] {
            let (top, bottom) = if down { (POSE_BASELINE - 70, POSE_BASELINE) } else { (POSE_BASELINE - 180, POSE_BASELINE - 110) };
            for y in top..bottom {
                for x in x0..x0 + 80 {
                    img.put_pixel(x, y, Rgba([90, 80, 70, 255]));
                }
            }
        }
        img
    }

    #[test]
    fn typing_poses_are_told_apart_by_the_raised_paw() {
        let idle = pose_with_paws(true, true);
        assert!(left_paw_down(&idle, &pose_with_paws(true, false)));
        assert!(!left_paw_down(&idle, &pose_with_paws(false, true)));
    }

    /// Re-cuts a real AI sheet:
    /// KUROCAT_SHEET=<png> KUROCAT_POSES=<dir> [KUROCAT_COLS=3 KUROCAT_ROWS=3] cargo test -- --ignored recut
    #[test]
    #[ignore]
    fn recut_real_sheet() {
        let sheet = image::open(std::env::var("KUROCAT_SHEET").unwrap()).unwrap().to_rgba8();
        let out = PathBuf::from(std::env::var("KUROCAT_POSES").unwrap());
        std::fs::create_dir_all(&out).unwrap();
        let cols: u32 = std::env::var("KUROCAT_COLS").map(|v| v.parse().unwrap()).unwrap_or(3);
        let rows: u32 = std::env::var("KUROCAT_ROWS").map(|v| v.parse().unwrap()).unwrap_or(3);
        for (i, img) in place(&cut_figures(&sheet, cols, rows).unwrap(), 1.0).iter().enumerate() {
            img.save(out.join(format!("pose-{i}.png"))).unwrap();
        }
    }

    #[test]
    fn green_screen_is_keyed_out_everywhere_but_white_fur_stays() {
        let mut cell = RgbaImage::from_pixel(100, 100, Rgba([0, 255, 0, 255]));
        for y in 20..80 {
            for x in 20..80 {
                cell.put_pixel(x, y, Rgba([252, 252, 250, 255]));
            }
        }
        // A green gap fully enclosed by the white body (e.g. between paws).
        for y in 45..55 {
            for x in 45..55 {
                cell.put_pixel(x, y, Rgba([0, 250, 10, 255]));
            }
        }
        remove_background(&mut cell);
        assert_eq!(*cell.get_pixel(5, 5), Rgba([0, 0, 0, 0]), "no green left in removed pixels");
        assert_eq!(cell.get_pixel(50, 50)[3], 0, "enclosed green gap is removed");
        assert_eq!(cell.get_pixel(30, 30)[3], 255, "white fur stays");
    }

    /// Rewrites the idle moments of an installed photo pack from its two sheets:
    /// KUROCAT_SHEET=<base png> KUROCAT_SHEET2=<moments png> KUROCAT_POSES=<pack dir> cargo test -- --ignored rebuild
    #[test]
    #[ignore]
    fn rebuild_moments_from_sheets() {
        let base = cut_figures(&open_sheet(Path::new(&std::env::var("KUROCAT_SHEET").unwrap())).unwrap(), 3, 3).unwrap();
        let moments = cut_figures(&open_sheet(Path::new(&std::env::var("KUROCAT_SHEET2").unwrap())).unwrap(), 3, 3).unwrap();
        let dir = PathBuf::from(std::env::var("KUROCAT_POSES").unwrap());
        write_poses(&dir, &MOMENTS, &moment_poses(&base, &moments)).unwrap();
    }

    #[test]
    fn white_inside_the_outline_is_kept() {
        let mut cell = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        for i in 20..80 {
            for (x, y) in [(i, 20), (i, 79), (20, i), (79, i)] {
                cell.put_pixel(x, y, Rgba([90, 80, 70, 255]));
            }
        }
        remove_background(&mut cell);
        assert_eq!(cell.get_pixel(2, 2)[3], 0);
        assert_eq!(cell.get_pixel(50, 50)[3], 255, "white fur inside the outline stays");
    }
}
