use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

// ── Executor list ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExcVersion {
    pub filename: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExcEntry {
    pub name: String,
    pub os: Vec<String>,
    pub versions: Vec<ExcVersion>,
}

#[derive(Debug, Deserialize)]
struct ExcConfig {
    exc: Vec<ExcEntry>,
}

// ── Android SDK constants ──────────────────────────────────────────────────────

const SYSTEM_IMAGE_PKG: &str = "system-images;android-35;google_apis;x86_64";

// ── Path helpers ───────────────────────────────────────────────────────────────

fn base_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("app_data_dir failed")
}

fn exc_folder(app: &AppHandle, name: &str) -> PathBuf {
    base_dir(app).join("exc").join(name)
}

fn sdk_dir() -> Result<PathBuf, String> {
    for var in &["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(p) = std::env::var(var) {
            let path = PathBuf::from(&p);
            if path.exists() {
                return Ok(path);
            }
        }
    }
    if let Some(local) = dirs::data_local_dir() {
        let path = local.join("Android").join("Sdk");
        if path.exists() {
            return Ok(path);
        }
    }
    Err("Android SDK not found. Install Android Studio to continue.".to_string())
}

fn sdk_tool(sdk: &Path, name: &str) -> PathBuf {
    let bat = if cfg!(windows) { format!("{}.bat", name) } else { name.to_string() };
    let v1 = sdk.join("cmdline-tools").join("latest").join("bin").join(&bat);
    if v1.exists() { return v1; }
    sdk.join("tools").join("bin").join(&bat)
}

fn image_installed(sdk: &Path) -> bool {
    sdk.join("system-images")
        .join("android-35")
        .join("google_apis")
        .join("x86_64")
        .exists()
}

fn avd_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".android")
        .join("avd")
}

// ── Executor commands ──────────────────────────────────────────────────────────

#[tauri::command]
fn get_exc_list(app: AppHandle) -> Result<Vec<ExcEntry>, String> {
    let dir = base_dir(&app);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    for entry in &[] as &[ExcEntry] {
        let _ = fs::create_dir_all(exc_folder(&app, &entry.name));
    }

    // Just return empty — exc list is fetched by frontend from GitHub
    Ok(vec![])
}

#[tauri::command]
fn get_installed_apk(app: AppHandle, exc_name: String) -> Option<String> {
    let dir = exc_folder(&app, &exc_name);
    if !dir.exists() {
        let _ = fs::create_dir_all(&dir);
        return None;
    }
    let mut apks: Vec<_> = fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x.to_ascii_lowercase() == "apk")
                .unwrap_or(false)
        })
        .collect();
    apks.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    apks.last().map(|e| e.file_name().to_string_lossy().to_string())
}

#[tauri::command]
fn open_download_in_browser(url: String) -> Result<(), String> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", &url])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn check_and_move_download(app: AppHandle, exc_name: String, filename: String) -> bool {
    let downloads = dirs::download_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads"));
    let source = downloads.join(&filename);
    if !source.exists() { return false; }
    if downloads.join(format!("{}.crdownload", &filename)).exists()
        || downloads.join(format!("{}.part", &filename)).exists()
    {
        return false;
    }
    let dest_dir = exc_folder(&app, &exc_name);
    let _ = fs::create_dir_all(&dest_dir);
    let dest = dest_dir.join(&filename);
    if fs::rename(&source, &dest).is_ok() { return true; }
    if fs::copy(&source, &dest).is_ok() {
        let _ = fs::remove_file(&source);
        return true;
    }
    false
}

// ── Android SDK commands ───────────────────────────────────────────────────────

#[tauri::command]
fn check_android_sdk() -> Result<String, String> {
    sdk_dir().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn check_system_image() -> bool {
    sdk_dir().map(|p| image_installed(&p)).unwrap_or(false)
}

#[tauri::command]
async fn download_system_image() -> Result<(), String> {
    let sdk = sdk_dir()?;
    let mgr = sdk_tool(&sdk, "sdkmanager");

    if !mgr.exists() {
        return Err("sdkmanager not found. Install Android Studio command-line tools.".to_string());
    }

    // Accept licenses first
    let _ = std::process::Command::new(&mgr)
        .args(["--licenses"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut c| {
            if let Some(mut stdin) = c.stdin.take() {
                use std::io::Write;
                for _ in 0..20 { let _ = stdin.write_all(b"y\n"); }
            }
            c.wait()
        });

    let mut child = std::process::Command::new(&mgr)
        .args(["--install", SYSTEM_IMAGE_PKG])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        for _ in 0..10 { let _ = stdin.write_all(b"y\n"); }
    }

    let output = child.wait_with_output().map_err(|e| e.to_string())?;

    if output.status.success() || image_installed(&sdk) {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        Err(if err.trim().is_empty() {
            "Download failed (unknown error)".to_string()
        } else {
            err
        })
    }
}

// ── AVD management ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AvdInfo {
    pub name: String,
    pub display_name: String,
    pub exc_name: String,
    pub target: String,
}

#[tauri::command]
fn list_avds() -> Result<Vec<AvdInfo>, String> {
    let sdk = match sdk_dir() {
        Ok(s) => s,
        Err(_) => return Ok(vec![]),
    };
    let mgr = sdk_tool(&sdk, "avdmanager");
    if !mgr.exists() { return Ok(vec![]); }

    let output = std::process::Command::new(&mgr)
        .args(["list", "avd"])
        .output()
        .map_err(|e| e.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut avds = Vec::new();
    let mut cur_name: Option<String> = None;
    let mut cur_target: Option<String> = None;

    for line in stdout.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("Name: ") {
            if let (Some(n), t) = (cur_name.take(), cur_target.take()) {
                if n.starts_with("psychex_") {
                    let mut parts = n.splitn(3, '_');
                    parts.next(); // "psychex"
                    let exc = parts.next().unwrap_or("unknown").to_string();
                    let disp = parts.next().unwrap_or(&n).to_string();
                    avds.push(AvdInfo {
                        name: n.clone(),
                        display_name: disp,
                        exc_name: exc,
                        target: t.unwrap_or_default(),
                    });
                }
            }
            cur_name = Some(name.to_string());
        } else if line.starts_with("Based on:") || line.starts_with("Target:") {
            let val = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
            cur_target = Some(val);
        }
    }
    if let (Some(n), t) = (cur_name, cur_target) {
        if n.starts_with("psychex_") {
            let mut parts = n.splitn(3, '_');
            parts.next();
            let exc = parts.next().unwrap_or("unknown").to_string();
            let disp = parts.next().unwrap_or(&n).to_string();
            avds.push(AvdInfo {
                name: n,
                display_name: disp,
                exc_name: exc,
                target: t.unwrap_or_default(),
            });
        }
    }

    Ok(avds)
}

#[tauri::command]
fn create_avd(name: String, exc_name: String, ram: u32, storage: u32) -> Result<(), String> {
    let sdk = sdk_dir()?;
    let mgr = sdk_tool(&sdk, "avdmanager");
    if !mgr.exists() { return Err("avdmanager not found".to_string()); }

    let avd_name = format!("psychex_{}_{}", exc_name, name);

    let output = std::process::Command::new(&mgr)
        .args([
            "create", "avd",
            "--name", &avd_name,
            "--package", SYSTEM_IMAGE_PKG,
            "--device", "pixel_7",
            "--force",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?
        .wait_with_output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).to_string();
        if !err.trim().is_empty() { return Err(err); }
    }

    // Apply RAM and storage to config.ini
    let config_path = avd_home()
        .join(format!("{}.avd", avd_name))
        .join("config.ini");

    if config_path.exists() {
        let content = fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
        let updated = content
            .lines()
            .map(|line| {
                if line.starts_with("hw.ramSize=") {
                    format!("hw.ramSize={}", ram)
                } else if line.starts_with("disk.dataPartition.size=") {
                    format!("disk.dataPartition.size={}M", storage)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&config_path, updated).map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
fn delete_avd(name: String) -> Result<(), String> {
    let sdk = sdk_dir()?;
    let mgr = sdk_tool(&sdk, "avdmanager");

    let output = std::process::Command::new(&mgr)
        .args(["delete", "avd", "--name", &name])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

#[tauri::command]
fn start_avd(name: String) -> Result<(), String> {
    let sdk = sdk_dir()?;
    let emu = sdk.join("emulator").join(if cfg!(windows) { "emulator.exe" } else { "emulator" });

    if !emu.exists() {
        return Err("Android Emulator not found in SDK".to_string());
    }

    std::process::Command::new(&emu)
        .args(["-avd", &name])
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ── Tauri entry ────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_exc_list,
            get_installed_apk,
            open_download_in_browser,
            check_and_move_download,
            check_android_sdk,
            check_system_image,
            download_system_image,
            list_avds,
            create_avd,
            delete_avd,
            start_avd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
