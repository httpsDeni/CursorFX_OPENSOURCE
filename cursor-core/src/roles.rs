//! Funcoes de cursor do Windows lidas do esquema atual (registro).
//! Implementacao original sobre API publica: HKCU\Control Panel\Cursors.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Role {
    /// Nome do valor no registro, ex. "Arrow".
    pub reg_name: &'static str,
    /// Nome de exibicao em portugues.
    pub label: &'static str,
    /// ID OCR_* para troca imediata; None usa recarga do esquema.
    pub ocr_id: Option<u32>,
}

pub const ALL_ROLES: [Role; 15] = [
    Role { reg_name: "Arrow", label: "Seta", ocr_id: Some(32512) },
    Role { reg_name: "Help", label: "Ajuda", ocr_id: None },
    Role { reg_name: "AppStarting", label: "Ocupado (inicial)", ocr_id: Some(32650) },
    Role { reg_name: "Wait", label: "Ocupado", ocr_id: Some(32514) },
    Role { reg_name: "Crosshair", label: "Precisao", ocr_id: Some(32515) },
    Role { reg_name: "IBeam", label: "Texto", ocr_id: Some(32513) },
    Role { reg_name: "NWPen", label: "Caneta", ocr_id: None },
    Role { reg_name: "No", label: "Indisponivel", ocr_id: Some(32648) },
    Role { reg_name: "SizeNS", label: "Redim. vertical", ocr_id: Some(32645) },
    Role { reg_name: "SizeWE", label: "Redim. horizontal", ocr_id: Some(32644) },
    Role { reg_name: "SizeNWSE", label: "Redim. diag. 1", ocr_id: Some(32642) },
    Role { reg_name: "SizeNESW", label: "Redim. diag. 2", ocr_id: Some(32643) },
    Role { reg_name: "SizeAll", label: "Mover", ocr_id: Some(32646) },
    Role { reg_name: "UpArrow", label: "Alternativo", ocr_id: Some(32516) },
    Role { reg_name: "Hand", label: "Link (mao)", ocr_id: Some(32649) },
];

const CURSORS_KEY: &str = "Control Panel\\Cursors";

/// Le o esquema atual: para cada funcao, o arquivo configurado ou None (padrao).
#[cfg(windows)]
pub fn read_scheme() -> Vec<(Role, Option<String>)> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(CURSORS_KEY)
        .ok();
    ALL_ROLES
        .iter()
        .map(|r| {
            let file = key
                .as_ref()
                .and_then(|k| k.get_value::<String, _>(r.reg_name).ok())
                .filter(|s| !s.trim().is_empty());
            (*r, file)
        })
        .collect()
}

/// Troca rapida: grava o arquivo da funcao no registro e recarrega o esquema.
/// `file` None volta ao padrao do sistema.
#[cfg(windows)]
pub fn set_role_cursor(role: &Role, file: Option<&str>) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SYSTEM_PARAMETERS_INFO_ACTION,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    if let Some(f) = file {
        if !std::path::Path::new(f).is_file() {
            return Err(format!("arquivo nao encontrado: {f}"));
        }
    }
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            CURSORS_KEY,
            winreg::enums::KEY_SET_VALUE,
        )
        .map_err(|e| format!("abrir registro: {e}"))?;
    match file {
        Some(f) => key
            .set_value(role.reg_name, &f.to_string())
            .map_err(|e| format!("gravar registro: {e}"))?,
        None => {
            // Valor ausente = padrao; se nao existir, tudo bem.
            let _ = key.delete_value(role.reg_name);
        }
    }
    unsafe {
        SystemParametersInfoW(
            SYSTEM_PARAMETERS_INFO_ACTION(0x0057), // SPI_SETCURSORS
            0,
            None,
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .map_err(|e| format!("recarregar cursores: {e}"))?;
    // Troca imediata extra para funcoes com ID OCR.
    if let (Some(path), Some(id)) = (file, role.ocr_id) {
        crate::apply_cursor_to(path, id)?;
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn read_scheme() -> Vec<(Role, Option<String>)> {
    ALL_ROLES.iter().map(|r| (*r, None)).collect()
}

#[cfg(not(windows))]
pub fn set_role_cursor(_role: &Role, _file: Option<&str>) -> Result<(), String> {
    Err("apenas Windows".to_string())
}

/// Palavras-chave por funcao, na ordem de prioridade.
const KEYWORDS: &[(&str, &[&str])] = &[
    ("UpArrow", &["uparrow", "up"]),
    ("Arrow", &["arrow", "seta", "normal", "pointer", "default"]),
    ("Help", &["help", "ajuda"]),
    ("AppStarting", &["appstarting", "working"]),
    ("Wait", &["wait", "busy", "ocupado"]),
    ("Crosshair", &["crosshair", "cross", "precisao", "precis"]),
    ("IBeam", &["ibeam", "beam", "texto", "text"]),
    ("NWPen", &["nwpen", "pen", "caneta"]),
    ("No", &["unavail", "indisponivel", "indispon", "circle"]),
    ("SizeNS", &["ns", "vertical"]),
    ("SizeWE", &["ew", "horizontal"]),
    ("SizeNWSE", &["nwse"]),
    ("SizeNESW", &["nesw"]),
    ("SizeAll", &["sizeall", "move", "mover"]),
    ("Hand", &["hand", "link", "mao"]),
];

fn tokens(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

/// Descobre a funcao pelo nome do arquivo. Puro, sem tocar no sistema.
pub fn role_for_filename(file_name: &str) -> Option<&'static str> {
    let toks = tokens(file_name);
    for (reg, keys) in KEYWORDS {
        if keys.iter().any(|k| toks.iter().any(|t| t == k)) {
            return Some(reg);
        }
    }
    None
}

/// Plano de aplicacao de um pacote: pares (funcao, arquivo), sem aplicar.
pub fn plan_package(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    matches!(
                        p.extension().and_then(|e| e.to_str()),
                        Some(ext) if ext.eq_ignore_ascii_case("cur") || ext.eq_ignore_ascii_case("ani")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    let mut out = Vec::new();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if let Some(reg) = role_for_filename(name) {
            out.push((reg.to_string(), p.to_string_lossy().to_string()));
        }
    }
    out
}

/// Aplica um pacote inteiro como esquema: cada arquivo vai para sua funcao.
#[cfg(windows)]
pub fn apply_package(dir: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    if !dir.is_dir() {
        return Err(format!("pasta nao encontrada: {}", dir.display()));
    }
    let plan = plan_package(dir);
    if plan.is_empty() {
        return Err("nenhum cursor reconhecido na pasta".to_string());
    }
    let by_reg: std::collections::HashMap<&str, &Role> =
        ALL_ROLES.iter().map(|r| (r.reg_name, r)).collect();
    for (reg, file) in &plan {
        let role = by_reg
            .get(reg.as_str())
            .ok_or_else(|| format!("funcao invalida: {reg}"))?;
        set_role_cursor(role, Some(file))?;
    }
    Ok(plan)
}

#[cfg(not(windows))]
pub fn apply_package(_dir: &std::path::Path) -> Result<Vec<(String, String)>, String> {
    Err("apenas Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifteen_unique_roles() {
        let mut names: Vec<_> = ALL_ROLES.iter().map(|r| r.reg_name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 15);
    }

    #[test]
    fn scheme_covers_all_roles() {
        assert_eq!(read_scheme().len(), 15);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_missing_file() {
        assert!(set_role_cursor(&ALL_ROLES[0], Some("C:\\nao-existe\\x.cur")).is_err());
    }

    #[test]
    fn maps_filenames_to_roles() {
        assert_eq!(role_for_filename("aero_arrow_l.cur"), Some("Arrow"));
        assert_eq!(role_for_filename("aero_up_l.cur"), Some("UpArrow"));
        assert_eq!(role_for_filename("duplo-click.cur"), None);
        assert_eq!(role_for_filename("hand.ani"), Some("Hand"));
        assert_eq!(role_for_filename("aero_working_l.ani"), Some("AppStarting"));
        assert_eq!(role_for_filename("texto.cur"), Some("IBeam"));
    }

    #[test]
    fn plans_package_without_applying() {
        let base = std::env::temp_dir().join("cursor-open-plan-test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("arrow.cur"), b"fake").unwrap();
        std::fs::write(base.join("hand.cur"), b"fake").unwrap();
        std::fs::write(base.join("leia.txt"), b"x").unwrap();
        let plan = plan_package(&base);
        assert_eq!(plan.len(), 2);
        assert!(plan.iter().any(|(r, _)| r == "Arrow"));
        assert!(plan.iter().any(|(r, _)| r == "Hand"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
