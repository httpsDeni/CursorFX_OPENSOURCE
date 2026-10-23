//! Cursor Open — app desktop de configuracao.
//! Design original em egui: pacotes, preview animado, efeitos e som.

// Sem console junto no release (instalado); no debug mantem para diagnóstico.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use cursor_core::roles::{read_scheme, set_role_cursor, Role};
use cursor_core::{
    cur::{parse_ani_first_frame, parse_cur_file},
    syscursor::system_cursor_image,
    apply_cursor, expand_env, restore_default, CursorConfig,
};
use cursor_fx::{Effects, Overlay, TrailBuffer};
use cursor_sound::{ensure_bank, play_click, BankEntry, SoundHook};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Current,
    Packages,
    Effects,
    Sound,
}

#[derive(Debug, Clone)]
struct Package {
    name: String,
    cursors: Vec<PathBuf>,
}

/// Cada subpasta do diretorio de pacotes vira um pacote; .cur/.ani entram na lista.
fn scan_packages(dir: &Path) -> Vec<Package> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    dirs.sort();
    for d in dirs {
        if !d.is_dir() {
            continue;
        }
        let mut cursors: Vec<PathBuf> = std::fs::read_dir(&d)
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
        cursors.sort();
        let name = d
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("sem nome")
            .to_string();
        out.push(Package { name, cursors });
    }
    out
}

fn is_ani(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .map(|e| e.eq_ignore_ascii_case("ani"))
        .unwrap_or(false)
}

fn app_dirs() -> (PathBuf, PathBuf) {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("cursor-open");
    (base.join("config.json"), base.join("packages"))
}

struct CursorApp {
    cfg: CursorConfig,
    cfg_path: PathBuf,
    pkg_dir: PathBuf,
    packages: Vec<Package>,
    selected_pkg: usize,
    scheme: Vec<(Role, Option<String>)>,
    selected_role: usize,
    selected_scheme: Option<usize>,
    selected_sound: Option<usize>,
    sounds: Vec<BankEntry>,
    sounds_dir: PathBuf,
    pending: Option<(Role, String)>,
    textures: HashMap<String, egui::TextureHandle>,
    texture_dims: HashMap<String, (u32, u32)>,
    tab: Tab,
    status: String,
    trail: TrailBuffer,
    angle: f32,
    ring_t: Option<f32>,
    overlay: Option<Overlay>,
    sound_hook: Option<SoundHook>,
    tray: Option<TrayIcon>,
    tray_open: Option<MenuItem>,
    tray_quit: Option<MenuItem>,
}

/// Icone da bandeja: mini bola amarela, gerado em codigo (sem assets).
fn tray_orb_icon() -> Option<tray_icon::Icon> {
    let s = 32u32;
    let mut rgba = Vec::with_capacity((s * s * 4) as usize);
    for y in 0..s {
        for x in 0..s {
            let dx = x as f32 + 0.5 - s as f32 / 2.0;
            let dy = y as f32 + 0.5 - s as f32 / 2.0;
            let d = (dx * dx + dy * dy).sqrt();
            let px = if d <= 11.0 {
                [255, 213, 0, 255]
            } else if d <= 13.0 {
                [255, 225, 80, 200]
            } else if d <= 14.5 {
                [255, 213, 0, 90]
            } else {
                [0, 0, 0, 0]
            };
            rgba.extend_from_slice(&px);
        }
    }
    tray_icon::Icon::from_rgba(rgba, s, s).ok()
}

impl CursorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = egui::Color32::from_rgb(0x14, 0x17, 0x1c);
        visuals.panel_fill = egui::Color32::from_rgb(0x1a, 0x1e, 0x25);
        visuals.selection.bg_fill = egui::Color32::from_rgb(0x2d, 0xd4, 0xbf);
        visuals.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2d, 0xd4, 0xbf));
        cc.egui_ctx.set_visuals(visuals);

        let (cfg_path, pkg_dir) = app_dirs();
        let _ = std::fs::create_dir_all(&pkg_dir);
        let sounds_dir = cursor_sound::default_bank_dir();
        let sounds = ensure_bank(&sounds_dir);
        let loaded = CursorConfig::load(&cfg_path);
        let selected_sound = if loaded.sound_file.trim().is_empty() {
            None
        } else {
            sounds
                .iter()
                .position(|e| e.path == loaded.sound_file)
        };
        let cfg = loaded;
        let trail_len = 6;
        let packages = scan_packages(&pkg_dir);
        let status = if packages.is_empty() {
            format!("nenhum pacote em {} — crie subpastas com .cur/.ani", pkg_dir.display())
        } else {
            format!("{} pacote(s) encontrado(s)", packages.len())
        };
        let mut app = Self {
            cfg,
            cfg_path,
            pkg_dir,
            packages,
            selected_pkg: 0,
            scheme: read_scheme(),
            selected_role: 0,
            selected_scheme: None,
            selected_sound,
            sounds,
            sounds_dir,
            pending: None,
            textures: HashMap::new(),
            texture_dims: HashMap::new(),
            tab: Tab::Current,
            status,
            trail: TrailBuffer::new(trail_len),
            angle: 0.0,
            ring_t: None,
            overlay: None,
            sound_hook: None,
            tray: None,
            tray_open: None,
            tray_quit: None,
        };
        app.sync_background();
        // Bandeja: clique esquerdo abre, botao direito tem menu Abrir/Sair.
        // Minimizar esconde a janela; efeitos e som seguem nos threads.
        let mut tray_status = "bandeja indisponivel".to_string();
        if let Some(icon) = tray_orb_icon() {
            let open = MenuItem::new("Abrir", true, None);
            let quit = MenuItem::new("Sair", true, None);
            let menu = Menu::new();
            if menu.append_items(&[&open, &quit]).is_ok() {
                match TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_tooltip("Cursor Open — efeitos e som ativos")
                    .with_title("Cursor Open")
                    .with_icon(icon)
                    .with_menu_on_left_click(false)
                    .build()
                {
                    Ok(tray) => {
                        app.tray = Some(tray);
                        app.tray_open = Some(open);
                        app.tray_quit = Some(quit);
                        tray_status = "bandeja ativa: minimizar esconde, efeitos continuam".to_string();
                    }
                    Err(e) => tray_status = format!("bandeja falhou: {e}"),
                }
            }
        }
        app.status = format!("{} | {tray_status}", app.status);
        #[cfg(windows)]
        Self::spawn_minimize_watch();
        // Acorda a UI quando chega evento da bandeja (clique/menu),
        // senao o app escondido nao repintaria para restaurar.
        let repaint_ctx = cc.egui_ctx.clone();
        let _ = std::thread::Builder::new()
            .name("cursor-open-tray-watch".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(150));
                if !TrayIconEvent::receiver().is_empty()
                    || !MenuEvent::receiver().is_empty()
                {
                    repaint_ctx.request_repaint();
                }
            });
        app
    }

    fn restore_window(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// Vigia o minimizar num thread proprio: quando a janela principal
    /// esta minimizada, esconde sozinho (vai para a bandeja).
    /// Nao depende do repintar do egui, que nem sempre roda apos minimizar.
    #[cfg(windows)]
    fn spawn_minimize_watch() {
        use windows::core::BOOL;
        use windows::Win32::Foundation::{HWND, LPARAM};
        use windows::Win32::UI::WindowsAndMessaging::*;
        struct Find {
            pid: u32,
            hwnd: HWND,
        }
        unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let st = &mut *(lparam.0 as *mut Find);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == st.pid {
                let mut buf = [0u16; 256];
                let len = GetWindowTextW(hwnd, &mut buf);
                if String::from_utf16_lossy(&buf[..len.max(0) as usize]) == "Cursor Open" {
                    st.hwnd = hwnd;
                }
            }
            true.into()
        }
        let _ = std::thread::Builder::new()
            .name("cursor-open-min-watch".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                unsafe {
                    static mut MAIN: HWND = HWND(std::ptr::null_mut());
                    if MAIN.0.is_null() {
                        let mut st = Find {
                            pid: std::process::id(),
                            hwnd: HWND(std::ptr::null_mut()),
                        };
                        let _ = EnumWindows(
                            Some(enum_cb),
                            LPARAM(&mut st as *mut Find as isize),
                        );
                        MAIN = st.hwnd;
                    }
                    if !MAIN.0.is_null() {
                        if !IsWindow(Some(MAIN)).as_bool() {
                            MAIN = HWND(std::ptr::null_mut());
                        } else if IsIconic(MAIN).as_bool() {
                            let _ = ShowWindow(MAIN, SW_HIDE);
                        }
                    }
                }
            });
    }

    /// Mantem overlay global + som no clique de acordo com o config atual.
    /// Efeito segue o cursor fora do app; som toca em todo clique esquerdo.
    /// O app precisa ficar aberto (pode minimizar) para os ganchos viverem.
    fn sync_background(&mut self) {
        let want_fx = Effects::from(&self.cfg);
        let need_restart = match self.overlay.as_ref() {
            Some(o) => !o.running() || o.effects() != want_fx,
            None => want_fx.enabled,
        };
        if need_restart {
            self.overlay = None;
            if want_fx.enabled {
                let mut o = Overlay::new(want_fx);
                match o.start() {
                    Ok(()) => self.overlay = Some(o),
                    Err(e) => {
                        self.status = format!("overlay: {e}");
                        Self::note_error(&self.cfg_path, &format!("overlay: {e}"));
                    }
                }
            }
        }
        if !want_fx.enabled {
            self.overlay = None;
        }
        let need_sound = match self.sound_hook.as_ref() {
            Some(h) => !h.running() || !h.config_matches(&self.cfg),
            None => self.cfg.sound_enabled,
        };
        if need_sound {
            self.sound_hook = None;
            if self.cfg.sound_enabled {
                let mut h = SoundHook::new(&self.cfg);
                match h.start() {
                    Ok(()) => self.sound_hook = Some(h),
                    Err(e) => {
                        self.status = format!("som: {e}");
                        Self::note_error(&self.cfg_path, &format!("som: {e}"));
                    }
                }
            }
        }
        if !self.cfg.sound_enabled {
            self.sound_hook = None;
        }
    }

    fn refresh_scheme(&mut self) {
        self.scheme = read_scheme();
        self.textures.clear();
        let custom = self.scheme.iter().filter(|(_, f)| f.is_some()).count();
        self.status = format!("{custom} de {} funcoes personalizadas", self.scheme.len());
    }

    fn store_texture(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        img: &cursor_core::cur::CurImage,
    ) -> egui::TextureHandle {
        if let Some(t) = self.textures.get(key) {
            return t.clone();
        }
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [img.width as usize, img.height as usize],
            &img.rgba,
        );
        let tex = ctx.load_texture(key.to_string(), color, egui::TextureOptions::NEAREST);
        self.textures.insert(key.to_string(), tex.clone());
        self.texture_dims
            .insert(key.to_string(), (img.width, img.height));
        tex
    }

    fn texture_dims(&self, key: &str) -> Option<(u32, u32)> {
        self.texture_dims.get(key).copied()
    }

    /// Previa de uma funcao: arquivo (.cur/.ani) ou cursor padrao do sistema.
    /// Retorna textura e dimensoes.
    fn preview_texture(
        &mut self,
        ctx: &egui::Context,
        role: &Role,
        file: Option<&str>,
    ) -> Option<(egui::TextureHandle, u32, u32)> {
        match file {
            Some(path) if is_ani(path) => {
                let full = expand_env(path);
                let key = format!("ani:{full}");
                if let Some(t) = self.textures.get(&key).cloned() {
                    let (w, h) = self.texture_dims(&key).unwrap_or((0, 0));
                    return Some((t, w, h));
                }
                let data = std::fs::read(&full).ok()?;
                let img = parse_ani_first_frame(&data)?;
                let t = self.store_texture(ctx, &key, &img);
                Some((t, img.width, img.height))
            }
            Some(path) => {
                let full = expand_env(path);
                if let Some(t) = self.textures.get(&full).cloned() {
                    let (w, h) = self.texture_dims(&full).unwrap_or((0, 0));
                    return Some((t, w, h));
                }
                let img = parse_cur_file(Path::new(&full))?;
                let t = self.store_texture(ctx, &full, &img);
                Some((t, img.width, img.height))
            }
            None => {
                let key = format!("sys:{}", role.reg_name);
                if let Some(t) = self.textures.get(&key).cloned() {
                    let (w, h) = self.texture_dims(&key).unwrap_or((0, 0));
                    return Some((t, w, h));
                }
                let img = system_cursor_image(role.reg_name)?;
                let t = self.store_texture(ctx, &key, &img);
                Some((t, img.width, img.height))
            }
        }
    }

    /// Registra a ultima falha de fundo para diagnostico.
    fn note_error(cfg_path: &std::path::Path, msg: &str) {
        if let Some(dir) = cfg_path.parent() {
            let _ = std::fs::write(dir.join("ultimo-erro.txt"), msg);
        }
    }

    fn save(&mut self) {
        if let Err(e) = self.cfg.save(&self.cfg_path) {
            self.status = format!("erro ao salvar: {e}");
            return;
        }
        self.sync_background();
    }

    fn main_cursor(&self) -> Option<&Path> {
        let pkg = self.packages.get(self.selected_pkg)?;
        pkg.cursors
            .iter()
            .find(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| {
                        let s = s.to_lowercase();
                        s.contains("arrow") || s.contains("seta") || s.contains("normal")
                    })
                    .unwrap_or(false)
            })
            .or_else(|| pkg.cursors.first())
            .map(|p| p.as_path())
    }

    fn apply_selected(&mut self) {
        let Some(cur) = self.main_cursor().map(|p| p.to_string_lossy().to_string()) else {
            self.status = "nada para aplicar".to_string();
            return;
        };
        match apply_cursor(&cur) {
            Ok(()) => {
                self.cfg.cursor_file = cur;
                self.save();
                self.status = "cursor aplicado".to_string();
            }
            Err(e) => self.status = format!("erro: {e}"),
        }
    }

    fn restore(&mut self) {
        match restore_default() {
            Ok(()) => {
                self.cfg.cursor_file.clear();
                self.save();
                self.status = "cursor padrao restaurado".to_string();
            }
            Err(e) => self.status = format!("erro: {e}"),
        }
    }
}

impl eframe::App for CursorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Minimizar joga para a bandeja (efeitos/som continuam nos threads).
        if ui.ctx().input(|i| i.viewport().minimized.unwrap_or(false)) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            match ev {
                TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    ..
                } => Self::restore_window(ui.ctx()),
                TrayIconEvent::DoubleClick { .. } => Self::restore_window(ui.ctx()),
                _ => {}
            }
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if self.tray_open.as_ref().is_some_and(|it| ev.id == *it.id()) {
                Self::restore_window(ui.ctx());
            } else if self.tray_quit.as_ref().is_some_and(|it| ev.id == *it.id()) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Cursor Open");
                ui.label(&self.status);
            });
        });

        egui::Panel::left("nav")
            .resizable(false)
            .default_size(190.0)
            .show(ui, |ui| {
                ui.heading("Configurar");
                ui.selectable_value(&mut self.tab, Tab::Current, "Atuais");
                ui.selectable_value(&mut self.tab, Tab::Packages, "Pacotes");
                ui.selectable_value(&mut self.tab, Tab::Effects, "Efeitos");
                ui.selectable_value(&mut self.tab, Tab::Sound, "Som");
                ui.separator();
                if ui.button("Aplicar cursor").clicked() {
                    self.apply_selected();
                }
                if ui.button("Restaurar padrao").clicked() {
                    self.restore();
                }
                ui.separator();
                ui.label(if self.tray.is_some() {
                    "● bandeja ativa (minimizar esconde)"
                } else {
                    "○ sem bandeja"
                });
            });

        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Current => {
                ui.horizontal(|ui| {
                    ui.heading("Cursores em uso");
                    if ui.button("Atualizar").clicked() {
                        self.refresh_scheme();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Esquema:");
                    let mut names: Vec<String> =
                        vec!["(manter atual)".to_string()];
                    names.extend(self.packages.iter().map(|p| p.name.clone()));
                    let mut sel = self.selected_scheme.map(|i| i + 1).unwrap_or(0);
                    egui::ComboBox::from_id_salt("scheme")
                        .selected_text(&names[sel])
                        .show_ui(ui, |ui| {
                            for (i, n) in names.iter().enumerate() {
                                ui.selectable_value(&mut sel, i, n);
                            }
                        });
                    self.selected_scheme = if sel == 0 { None } else { Some(sel - 1) };
                    if ui.button("Aplicar esquema").clicked() {
                        match self.selected_scheme {
                            Some(i) => {
                                let dir = self.packages[i].cursors.first()
                                    .and_then(|p| p.parent().map(|d| d.to_path_buf()));
                                match dir {
                                    Some(d) => match cursor_core::roles::apply_package(&d) {
                                        Ok(plan) => {
                                            self.status = format!(
                                                "esquema {} aplicado ({} funcoes)",
                                                self.packages[i].name,
                                                plan.len()
                                            );
                                            self.refresh_scheme();
                                        }
                                        Err(e) => {
                                            self.status = format!("erro: {e}")
                                        }
                                    },
                                    None => {
                                        self.status = "pacote vazio".to_string();
                                    }
                                }
                            }
                            None => {
                                self.status = "escolha um pacote".to_string();
                            }
                        }
                    }
                    if ui.button("Restaurar tudo").clicked() {
                        self.restore();
                        self.refresh_scheme();
                    }
                });
                ui.separator();
                ui.columns(2, |cols| {
                    let mut picked: Option<usize> = None;
                    egui::ScrollArea::vertical().show(&mut cols[0], |ui| {
                        for i in 0..self.scheme.len() {
                            let (role, file) = self.scheme[i].clone();
                            ui.horizontal(|ui| {
                                match self.preview_texture(ui.ctx(), &role, file.as_deref()) {
                                Some((t, _, _)) => {
                                    ui.image((t.id(), egui::vec2(40.0, 40.0)));
                                }
                                None => {
                                    ui.label("?");
                                }
                                }
                                if ui
                                    .selectable_label(
                                        self.selected_role == i,
                                        role.label,
                                    )
                                    .clicked()
                                {
                                    picked = Some(i);
                                }
                            });
                        }
                    });
                    if let Some(i) = picked {
                        self.selected_role = i;
                    }
                    let idx = self.selected_role.min(self.scheme.len().saturating_sub(1));
                    let (role, file) = self.scheme[idx].clone();
                    cols[1].vertical_centered(|ui| {
                        ui.heading(role.label);
                        match self.preview_texture(ui.ctx(), &role, file.as_deref()) {
                            Some((t, w, h)) => {
                                ui.image((t.id(), egui::vec2(128.0, 128.0)));
                                ui.label(format!("{w} x {h}"));
                            }
                            None => {
                                ui.label("Previa indisponivel.");
                            }
                        }
                        if let Some(path) = file.as_deref() {
                            if is_ani(path) {
                                ui.label("Cursor animado: primeiro frame.");
                            }
                            ui.label(
                                Path::new(path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(path),
                            );
                        } else {
                            ui.label("Padrao do sistema.");
                        }
                        if ui.button("Procurar...").clicked() {
                            if let Some(p) = rfd::FileDialog::new()
                                .add_filter("Cursor", &["cur", "ani"])
                                .pick_file()
                            {
                                self.pending = Some((
                                    role,
                                    p.to_string_lossy().to_string(),
                                ));
                            }
                        }
                        if ui.button("Usar Padrao").clicked() {
                            match set_role_cursor(&role, None) {
                                Ok(()) => self.refresh_scheme(),
                                Err(e) => self.status = format!("erro: {e}"),
                            }
                        }
                    });
                });
                if let Some((role, path)) = self.pending.clone() {
                    ui.separator();
                    ui.heading(format!("Trocar: {}", role.label));
                    ui.columns(2, |cols| {
                        cols[0].vertical_centered(|ui| {
                            ui.label("Atual");
                            let current = self
                                .scheme
                                .iter()
                                .find(|(r, _)| r.reg_name == role.reg_name)
                                .and_then(|(_, f)| f.clone());
                            match self.preview_texture(
                                ui.ctx(),
                                &role,
                                current.as_deref(),
                            ) {
                                Some((t, _, _)) => {
                                    ui.image((t.id(), egui::vec2(96.0, 96.0)));
                                }
                                None => {
                                    ui.label("Previa indisponivel.");
                                }
                            }
                            ui.label(
                                current
                                    .as_deref()
                                    .and_then(|c| {
                                        Path::new(c)
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                    })
                                    .unwrap_or("padrao"),
                            );
                        });
                        cols[1].vertical_centered(|ui| {
                            ui.label("Novo");
                            match self.preview_texture(ui.ctx(), &role, Some(&path)) {
                                Some((t, w, h)) => {
                                    ui.image((t.id(), egui::vec2(96.0, 96.0)));
                                    ui.label(format!("{w} x {h}"));
                                    if is_ani(&path) {
                                        ui.label("Animado: primeiro frame.");
                                    }
                                }
                                None => {
                                    ui.label("Previa indisponivel.");
                                }
                            }
                            ui.label(
                                Path::new(&path)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or(&path),
                            );
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Confirmar troca").clicked() {
                            match set_role_cursor(&role, Some(&path)) {
                                Ok(()) => {
                                    self.status =
                                        format!("{} trocado", role.label);
                                    self.pending = None;
                                    self.refresh_scheme();
                                }
                                Err(e) => {
                                    self.status = format!("erro: {e}")
                                }
                            }
                        }
                        if ui.button("Cancelar").clicked() {
                            self.pending = None;
                            self.status = "troca cancelada".to_string();
                        }
                    });
                }
            }
            Tab::Packages => {
                ui.heading("Pacotes de cursor");
                if self.packages.is_empty() {
                    ui.label(format!(
                        "Coloque pastas com .cur/.ani em {}",
                        self.pkg_dir.display()
                    ));
                }
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    for (i, pkg) in self.packages.iter().enumerate() {
                        let label = format!("{} ({} cursores)", pkg.name, pkg.cursors.len());
                        if ui.selectable_label(self.selected_pkg == i, label).clicked() {
                            self.selected_pkg = i;
                        }
                    }
                });
                ui.separator();
                self.preview(ui);
            }
            Tab::Effects => {
                ui.heading("Efeitos — seguem o cursor na tela (app minimizado ok)");
                let live = self.overlay.as_ref().is_some_and(|o| o.running());
                ui.label(if live {
                    "● efeitos ATIVOS no cursor global"
                } else {
                    "○ efeitos desligados (ative para ver no cursor de verdade)"
                });
                let mut fx = self.cfg.effects_enabled;
                if ui.checkbox(&mut fx, "Ativar efeitos").changed() {
                    self.cfg.effects_enabled = fx;
                    self.save();
                }
                ui.horizontal(|ui| {
                    ui.label("Predefinicao:");
                    let presets: [(&str, u8, bool, bool, bool); 5] = [
                        ("Bola amarela", 0, false, false, true),
                        ("Cometa", 10, true, true, false),
                        ("Anel", 0, true, false, false),
                        ("Sombra", 4, false, true, false),
                        ("Limpo", 0, false, false, false),
                    ];
                    for (name, trail, ring, shadow, orb) in presets {
                        if ui.button(name).clicked() {
                            self.cfg.trail_length = trail;
                            self.cfg.click_ring = ring;
                            self.cfg.shadow = shadow;
                            self.cfg.yellow_orb = orb;
                            self.cfg.effects_enabled = true;
                            self.save();
                            self.status =
                                format!("efeito: {name}");
                        }
                    }
                });
                let mut trail = self.cfg.trail_length;
                if ui
                    .add(egui::Slider::new(&mut trail, 0..=12).text("Rastro"))
                    .changed()
                {
                    self.cfg.trail_length = trail;
                    self.save();
                }
                let mut ring = self.cfg.click_ring;
                if ui.checkbox(&mut ring, "Anel no clique").changed() {
                    self.cfg.click_ring = ring;
                    self.save();
                }
                let mut shadow = self.cfg.shadow;
                if ui.checkbox(&mut shadow, "Sombra").changed() {
                    self.cfg.shadow = shadow;
                    self.save();
                }
                let mut orb = self.cfg.yellow_orb;
                if ui
                    .checkbox(&mut orb, "Bola amarela ao redor do cursor")
                    .changed()
                {
                    self.cfg.yellow_orb = orb;
                    if orb {
                        // Uma bola so, bem no meio: zera rastro e anel.
                        self.cfg.effects_enabled = true;
                        self.cfg.trail_length = 0;
                        self.cfg.click_ring = false;
                    }
                    self.save();
                }
                ui.separator();
                self.preview(ui);
            }
            Tab::Sound => {
                ui.heading("Som do clique");
                let mut on = self.cfg.sound_enabled;
                if ui.checkbox(&mut on, "Ativar som").changed() {
                    self.cfg.sound_enabled = on;
                    self.save();
                }
                let mut vol = self.cfg.sound_volume;
                if ui
                    .add(egui::Slider::new(&mut vol, 0.0..=1.0).text("Volume"))
                    .changed()
                {
                    self.cfg.sound_volume = vol;
                    self.save();
                }
                ui.horizontal(|ui| {
                    ui.label("Banco de sons:");
                    let mut sel = self.selected_sound.map(|i| i + 1).unwrap_or(0);
                    let mut names = vec!["Clique de mouse (padrao)".to_string()];
                    names.extend(self.sounds.iter().map(|e| e.name.clone()));
                    egui::ComboBox::from_id_salt("soundbank")
                        .selected_text(&names[sel])
                        .show_ui(ui, |ui| {
                            for (i, n) in names.iter().enumerate() {
                                ui.selectable_value(&mut sel, i, n);
                            }
                        });
                    let new_sel = if sel == 0 { None } else { Some(sel - 1) };
                    if new_sel != self.selected_sound {
                        self.selected_sound = new_sel;
                        self.cfg.sound_file = new_sel
                            .and_then(|i| self.sounds.get(i))
                            .map(|e| e.path.clone())
                            .unwrap_or_default();
                        self.save();
                    }
                });
                ui.label(format!(
                    "Arquivo: {}",
                    if self.cfg.sound_file.trim().is_empty() {
                        "clique de mouse (padrao)".to_string()
                    } else {
                        self.cfg.sound_file.clone()
                    }
                ));
                ui.horizontal(|ui| {
                    if ui.button("Recarregar lista").clicked() {
                        let cur = self.cfg.sound_file.clone();
                        self.sounds = ensure_bank(&self.sounds_dir);
                        self.selected_sound =
                            self.sounds.iter().position(|e| e.path == cur);
                        self.status = format!(
                            "{} som(ns) na pasta",
                            self.sounds.len()
                        );
                    }
                    if ui.button("Abrir pasta de sons").clicked() {
                        #[cfg(windows)]
                        let _ = std::process::Command::new("explorer")
                            .arg(&self.sounds_dir)
                            .spawn();
                        self.status = "jogue o .wav na pasta e clique Recarregar".to_string();
                    }
                    if ui.button("Escolher arquivo...").clicked() {
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "mp3"])
                            .pick_file()
                        {
                            match cursor_sound::import_sound_to_bank(
                                &p,
                                &self.sounds_dir,
                            ) {
                                Ok(entry) => {
                                    self.sounds =
                                        ensure_bank(&self.sounds_dir);
                                    self.selected_sound = self
                                        .sounds
                                        .iter()
                                        .position(|e| e.path == entry.path);
                                    self.cfg.sound_file = entry.path.clone();
                                    self.save();
                                    self.status = format!(
                                        "som importado: {}",
                                        entry.name
                                    );
                                }
                                Err(e) => {
                                    self.status = format!("erro: {e}")
                                }
                            }
                        }
                    }
                    if ui.button("Usar clique de mouse").clicked() {
                        self.cfg.sound_file.clear();
                        self.save();
                    }
                });
                if ui.button("Testar som").clicked() {
                    match play_click(&self.cfg) {
                        Ok(()) => self.status = "som tocado".to_string(),
                        Err(e) => self.status = format!("erro: {e}"),
                    }
                }
            }
        });

        if matches!(self.tab, Tab::Packages | Tab::Effects) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

impl CursorApp {
    fn preview(&mut self, ui: &mut egui::Ui) {
        self.trail.set_capacity(self.cfg.trail_length);
        ui.label("Previa (clique no quadro para o anel)");
        let size = egui::vec2(ui.available_width().min(560.0), 260.0);
        let (resp, painter) = ui.allocate_painter(size, egui::Sense::click());
        let rect = resp.rect;
        painter.rect_filled(rect, 10.0, egui::Color32::from_rgb(0x0f, 0x12, 0x17));

        let dt = ui.ctx().input(|i| i.stable_dt).min(0.05);
        let center = rect.center();
        let radius = (rect.width().min(rect.height()) / 2.0 - 34.0).max(20.0);
        self.angle += dt * 1.6;
        let pos = center + radius * egui::vec2(self.angle.cos(), self.angle.sin());
        if self.cfg.trail_length > 0 {
            self.trail.push((pos.x as i32, pos.y as i32));
        }

        if resp.clicked() && self.cfg.click_ring {
            self.ring_t = Some(0.0);
        }

        let pts = self.trail.points();
        for (i, (x, y)) in pts.iter().enumerate() {
            let a = ((i + 1) as f32 / pts.len().max(1) as f32 * 160.0) as u8;
            painter.circle_filled(
                egui::pos2(*x as f32, *y as f32),
                9.0,
                egui::Color32::from_rgba_unmultiplied(0x2d, 0xd4, 0xbf, a),
            );
        }
        // Bola amarela como na foto: circulo amarelo meio transparente,
        // seta/cursor no meio. Desenha antes do ponto para ficar atras.
        // Desloca igual ao overlay (ponta da seta -> centro visual).
        if self.cfg.yellow_orb {
            let ball = pos + egui::vec2(9.0, 13.0);
            painter.circle_filled(
                ball,
                34.0,
                egui::Color32::from_rgba_unmultiplied(255, 213, 0, 150),
            );
            painter.circle_filled(
                ball + egui::vec2(0.0, -4.0),
                20.0,
                egui::Color32::from_rgba_unmultiplied(255, 235, 100, 170),
            );
        }
        // Sombra + seta estilizada no ponto atual.
        if self.cfg.shadow {
            painter.circle_filled(pos + egui::vec2(3.0, 4.0), 11.0, egui::Color32::from_black_alpha(110));
        }
        painter.circle_filled(pos, 10.0, egui::Color32::WHITE);
        painter.circle_filled(pos, 5.0, egui::Color32::from_rgb(0x2d, 0xd4, 0xbf));

        if let Some(t) = self.ring_t.as_mut() {
            *t += dt * 1.4;
            if *t >= 1.0 {
                self.ring_t = None;
            } else {
                painter.circle_stroke(
                    center,
                    12.0 + *t * 60.0,
                    egui::Stroke::new(
                        2.5,
                        egui::Color32::from_rgba_unmultiplied(
                            0x2d,
                            0xd4,
                            0xbf,
                            (200.0 * (1.0 - *t)) as u8,
                        ),
                    ),
                );
            }
        }

        if !self.cfg.effects_enabled {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "efeitos desligados",
                egui::FontId::proportional(14.0),
                egui::Color32::GRAY,
            );
        }
    }
}

/// Uma instancia so: dois apps = dois overlays (duas bolas) + som duplo.
/// Quem chega depois espera o anterior terminar de sair (kill + relaunch
/// imediato); so desiste se o mutex continuar preso (outro app vivo).
#[cfg(windows)]
fn ensure_single_instance() {
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    unsafe {
        for _ in 0..10 {
            match CreateMutexW(None, false, w!("Local\\CursorOpenSingleInstance")) {
                // HANDLE nao tem Drop: soltar o valor nao fecha o mutex,
                // ele vive ate o processo sair.
                Ok(_) if GetLastError().0 != 183 => return,
                Ok(m) => {
                    let _ = CloseHandle(m);
                }
                Err(_) => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        std::process::exit(0);
    }
}

fn main() -> eframe::Result<()> {
    #[cfg(windows)]
    ensure_single_instance();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([980.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Cursor Open",
        options,
        Box::new(|cc| Ok(Box::new(CursorApp::new(cc)) as Box<dyn eframe::App>)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_lists_subfolders_with_cursors() {
        let base = std::env::temp_dir().join("cursor-open-pkg-test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("Neon")).unwrap();
        std::fs::write(base.join("Neon").join("arrow.cur"), b"fake").unwrap();
        std::fs::write(base.join("Neon").join("leia.txt"), b"x").unwrap();
        std::fs::create_dir_all(base.join("Vazio")).unwrap();
        let pkgs = scan_packages(&base);
        assert_eq!(pkgs.len(), 2);
        let neon = pkgs.iter().find(|p| p.name == "Neon").unwrap();
        assert_eq!(neon.cursors.len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }
}
