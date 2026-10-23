use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorConfig {
    pub cursor_file: String,
    pub effects_enabled: bool,
    #[serde(default = "default_trail")]
    pub trail_length: u8,
    #[serde(default = "default_true")]
    pub click_ring: bool,
    #[serde(default = "default_true")]
    pub shadow: bool,
    /// Bola amarela transparente ao redor do cursor (cursor fica no meio).
    #[serde(default = "default_true")]
    pub yellow_orb: bool,
    pub sound_enabled: bool,
    pub sound_volume: f32,
    /// Arquivo .wav escolhido pelo usuario. Vazio = bipe gerado no codigo.
    #[serde(default)]
    pub sound_file: String,
}

fn default_trail() -> u8 {
    6
}

fn default_true() -> bool {
    true
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            cursor_file: String::new(),
            effects_enabled: true,
            trail_length: default_trail(),
            click_ring: true,
            shadow: true,
            yellow_orb: true,
            sound_enabled: true,
            sound_volume: 0.7,
            sound_file: String::new(),
        }
    }
}

impl CursorConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .map(|t| t.strip_prefix('\u{FEFF}').unwrap_or(&t).to_string())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).expect("config serializes");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)
    }
}

/// Aplica um cursor (.cur/.ani) como cursor de seta do sistema.
/// Original, usa apenas API publica do Windows.
#[cfg(windows)]
pub fn apply_cursor(cursor_path: &str) -> Result<(), String> {
    apply_cursor_to(cursor_path, 32512)
}

/// Aplica um arquivo a uma funcao especifica (ID OCR_*), apenas na sessao.
#[cfg(windows)]
pub fn apply_cursor_to(cursor_path: &str, ocr_id: u32) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        HCURSOR, LoadImageW, SetSystemCursor, IMAGE_CURSOR, LR_LOADFROMFILE,
        SYSTEM_CURSOR_ID,
    };

    let h = unsafe {
        LoadImageW(
            None,
            &HSTRING::from(cursor_path),
            IMAGE_CURSOR,
            0,
            0,
            LR_LOADFROMFILE,
        )
    }
    .map_err(|e| format!("LoadImageW falhou: {e}"))?;
    let cursor = HCURSOR(h.0);
    unsafe { SetSystemCursor(cursor, SYSTEM_CURSOR_ID(ocr_id)) }
        .map_err(|e| format!("SetSystemCursor falhou: {e}"))
}

pub mod cur;
pub mod roles;
pub mod syscursor;

/// Expande variaveis de ambiente estilo Windows (%SystemRoot%) no caminho.
/// O registro guarda os cursores do sistema assim; sem expandir, o arquivo
/// nao e encontrado. Nomes desconhecidos sao mantidos como estao.
pub fn expand_env(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                let value = std::env::var(name)
                    .or_else(|_| std::env::var(name.to_uppercase()))
                    .unwrap_or_else(|_| format!("%{name}%"));
                out.push_str(&rest[..start]);
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// Restaura os cursores padrao do Windows.
#[cfg(windows)]
pub fn restore_default() -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW;
    use windows::Win32::UI::WindowsAndMessaging::{
        SYSTEM_PARAMETERS_INFO_ACTION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let ok = unsafe {
        SystemParametersInfoW(
            SYSTEM_PARAMETERS_INFO_ACTION(0x0057), // SPI_SETCURSORS
            0,
            None,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    match ok {
        Ok(()) => Ok(()),
        Err(e) => Err(format!("SPI_SETCURSORS falhou: {e}")),
    }
}

#[cfg(not(windows))]
pub fn apply_cursor(_cursor_path: &str) -> Result<(), String> {
    Err("apenas Windows".to_string())
}

#[cfg(not(windows))]
pub fn restore_default() -> Result<(), String> {
    Err("apenas Windows".to_string())
}

#[cfg(test)]
mod expand_tests {
    use super::expand_env;

    #[test]
    fn plain_path_untouched() {
        assert_eq!(expand_env("C:\\a\\b.cur"), "C:\\a\\b.cur");
        assert_eq!(expand_env("%NoSuchVar_zzz%\\x"), "%NoSuchVar_zzz%\\x");
        assert_eq!(expand_env("sem porcento"), "sem porcento");
    }

    #[test]
    fn expands_custom_var() {
        std::env::set_var("CURSOR_OPEN_TEST_DIR", "C:\\t");
        assert_eq!(
            expand_env("%CURSOR_OPEN_TEST_DIR%\\a.cur"),
            "C:\\t\\a.cur"
        );
        std::env::remove_var("CURSOR_OPEN_TEST_DIR");
    }

    #[cfg(windows)]
    #[test]
    fn expands_system_root() {
        let got = expand_env("%SystemRoot%\\cursors");
        assert!(!got.contains('%'), "nao expandiu: {got}");
        assert!(std::path::Path::new(&got).is_dir(), "pasta inexistente: {got}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_bom_tolerant() {
        // Editores/ferramentas Windows gravam UTF-8 com BOM; nao pode cair no padrao.
        let dir = std::env::temp_dir().join("cursor-open-bom-test");
        let path = dir.join("config.json");
        let _ = std::fs::create_dir_all(&dir);
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(br#"{"cursor_file":"","effects_enabled":false,"trail_length":0,"click_ring":false,"shadow":false,"yellow_orb":true,"sound_enabled":true,"sound_volume":1.0,"sound_file":""}"#);
        std::fs::write(&path, raw).unwrap();
        let back = CursorConfig::load(&path);
        assert!(!back.effects_enabled);
        assert_eq!(back.trail_length, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_roundtrip() {
        let dir = std::env::temp_dir().join("cursor-open-test");
        let path = dir.join("config.json");
        let cfg = CursorConfig::default();
        cfg.save(&path).unwrap();
        let back = CursorConfig::load(&path);
        assert_eq!(back.sound_volume, 0.7);
    }
}
