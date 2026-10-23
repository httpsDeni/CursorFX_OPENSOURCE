//! cursor-ctl: controle do cursor-open (aplica, restaura, configura).
//! Uso: cursor-ctl show | apply <cur> | restore | effects on|off | sound on|off [--volume V]

use cursor_core::{apply_cursor, restore_default, CursorConfig};
use std::path::PathBuf;

fn default_config_path() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("cursor-open").join("config.json")
    } else {
        PathBuf::from("config.json")
    }
}

#[derive(Debug, PartialEq)]
enum Cmd {
    Show,
    Apply(String),
    Restore,
    Effects(bool),
    Sound(bool, Option<f32>),
    Current,
    Swap(String, Option<String>),
    Scheme(String),
    SoundFile(Option<String>),
}

fn find_role(query: &str) -> Result<cursor_core::roles::Role, String> {
    let q = query.to_lowercase();
    cursor_core::roles::ALL_ROLES
        .iter()
        .find(|r| {
            r.reg_name.to_lowercase() == q
                || r.label.to_lowercase() == q
                || r.reg_name.to_lowercase().starts_with(q.as_str())
        })
        .copied()
        .ok_or_else(|| format!("funcao desconhecida: {query} (ex.: arrow, hand, wait, ibeam)"))
}

fn parse_args(args: &[String]) -> Result<(Cmd, PathBuf), String> {
    let mut config = default_config_path();
    let mut rest: Vec<&str> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--config" {
            i += 1;
            config = PathBuf::from(args.get(i).ok_or("--config precisa de valor")?);
        } else {
            rest.push(args[i].as_str());
        }
        i += 1;
    }
    let cmd = match rest.as_slice() {
        ["show"] => Cmd::Show,
        ["apply", cur] => Cmd::Apply(cur.to_string()),
        ["restore"] => Cmd::Restore,
        ["effects", v] => Cmd::Effects(parse_on_off(v)?),
        ["sound", v] => Cmd::Sound(parse_on_off(v)?, None),
        ["sound", v, "--volume", n] => Cmd::Sound(
            parse_on_off(v)?,
            Some(n.parse::<f32>().map_err(|_| "volume invalido")?),
        ),
        ["atuais"] => Cmd::Current,
        ["trocar", role, "padrao"] => Cmd::Swap(role.to_string(), None),
        ["trocar", role, file] => Cmd::Swap(role.to_string(), Some(file.to_string())),
        ["esquema", dir] => Cmd::Scheme(dir.to_string()),
        ["som-arquivo", "padrao"] => Cmd::SoundFile(None),
        ["som-arquivo", f] => Cmd::SoundFile(Some(f.to_string())),
        _ => return Err("uso: show | apply <cur> | restore | effects on|off | sound on|off [--volume V] | atuais | trocar <funcao> <arquivo|padrao> | esquema <pasta> | som-arquivo <wav|padrao>".into()),
    };
    Ok((cmd, config))
}

fn parse_on_off(v: &str) -> Result<bool, String> {
    match v {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("use on|off".into()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (cmd, cfg_path) = parse_args(&args).unwrap_or_else(|e| {
        eprintln!("erro: {e}");
        std::process::exit(2);
    });
    let mut cfg = CursorConfig::load(&cfg_path);
    match cmd {
        Cmd::Show => {
            println!("{}", serde_json::to_string_pretty(&cfg).unwrap());
        }
        Cmd::Apply(cur) => {
            if let Err(e) = apply_cursor(&cur) {
                eprintln!("erro: {e}");
                std::process::exit(1);
            }
            cfg.cursor_file = cur;
            cfg.save(&cfg_path).unwrap();
            println!("cursor aplicado");
        }
        Cmd::Restore => {
            if let Err(e) = restore_default() {
                eprintln!("erro: {e}");
                std::process::exit(1);
            }
            cfg.cursor_file.clear();
            cfg.save(&cfg_path).unwrap();
            println!("cursor padrao restaurado");
        }
        Cmd::Effects(on) => {
            cfg.effects_enabled = on;
            cfg.save(&cfg_path).unwrap();
            println!("efeitos: {}", if on { "on" } else { "off" });
        }
        Cmd::Sound(on, vol) => {
            cfg.sound_enabled = on;
            if let Some(v) = vol {
                if !(0.0..=1.0).contains(&v) {
                    eprintln!("erro: volume fora de 0.0..=1.0");
                    std::process::exit(2);
                }
                cfg.sound_volume = v;
            }
            cfg.save(&cfg_path).unwrap();
            println!("som: {}", if on { "on" } else { "off" });
        }
        Cmd::Current => {
            for (role, file) in cursor_core::roles::read_scheme() {
                println!(
                    "{:<12} {}",
                    role.reg_name,
                    file.as_deref().unwrap_or("(padrao)")
                );
            }
        }
        Cmd::Scheme(dir) => {
            match cursor_core::roles::apply_package(std::path::Path::new(&dir)) {
                Ok(plan) => {
                    for (reg, file) in &plan {
                        println!("{reg:<12} {file}");
                    }
                }
                Err(e) => {
                    eprintln!("erro: {e}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::SoundFile(file) => {
            if let Some(f) = file.as_deref() {
                let full = cursor_core::expand_env(f);
                if !std::path::Path::new(&full).is_file() {
                    eprintln!("erro: arquivo nao encontrado: {f}");
                    std::process::exit(2);
                }
                cfg.sound_file = f.to_string();
            } else {
                cfg.sound_file.clear();
            }
            cfg.save(&cfg_path).unwrap();
            println!(
                "som: {}",
                file.as_deref().unwrap_or("proprio (bipe)")
            );
        }
        Cmd::Swap(role_q, file) => {
            let role = find_role(&role_q).unwrap_or_else(|e| {
                eprintln!("erro: {e}");
                std::process::exit(2);
            });
            if let Err(e) =
                cursor_core::roles::set_role_cursor(&role, file.as_deref())
            {
                eprintln!("erro: {e}");
                std::process::exit(1);
            }
            println!(
                "{}: {}",
                role.label,
                file.as_deref().unwrap_or("padrao restaurado")
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        std::iter::once("cursor-ctl")
            .chain(v.iter().copied())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn parses_apply_and_restore() {
        let (cmd, _) = parse_args(&a(&["apply", "C:\\c\\a.cur"])).unwrap();
        assert_eq!(cmd, Cmd::Apply("C:\\c\\a.cur".into()));
        let (cmd, _) = parse_args(&a(&["restore"])).unwrap();
        assert_eq!(cmd, Cmd::Restore);
    }

    #[test]
    fn rejects_bad_volume_flag() {
        assert!(parse_args(&a(&["sound", "on", "--volume", "abc"])).is_err());
        assert!(parse_args(&a(&["effects", "maybe"])).is_err());
    }

    #[test]
    fn parses_current_and_swap() {
        let (cmd, _) = parse_args(&a(&["atuais"])).unwrap();
        assert!(matches!(cmd, Cmd::Current));
        let (cmd, _) = parse_args(&a(&["trocar", "arrow", "padrao"])).unwrap();
        assert!(matches!(cmd, Cmd::Swap(_, None)));
        let (cmd, _) = parse_args(&a(&["trocar", "hand", "C:\\c\\a.cur"])).unwrap();
        assert!(matches!(cmd, Cmd::Swap(_, Some(_))));
        assert!(find_role("HAND").is_ok());
        assert!(find_role("seta").is_ok()); // rotulo em portugues tambem vale
        assert!(find_role("xyz").is_err());
        let (cmd, _) = parse_args(&a(&["esquema", "C:\\pacs\\x"])).unwrap();
        assert!(matches!(cmd, Cmd::Scheme(_)));
        let (cmd, _) = parse_args(&a(&["som-arquivo", "padrao"])).unwrap();
        assert!(matches!(cmd, Cmd::SoundFile(None)));
    }
}
