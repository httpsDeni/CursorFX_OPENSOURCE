//! Efeitos de cursor: rastro logico + overlay no Windows.
//! Implementacao original com API publica (janela layered que segue o cursor).

use cursor_core::CursorConfig;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effects {
    pub enabled: bool,
    pub trail_length: u8,
    pub click_ring: bool,
    pub shadow: bool,
    pub orb: bool,
}

impl From<&CursorConfig> for Effects {
    fn from(cfg: &CursorConfig) -> Self {
        Self {
            enabled: cfg.effects_enabled,
            trail_length: cfg.trail_length.min(12),
            click_ring: cfg.click_ring,
            shadow: cfg.shadow,
            orb: cfg.yellow_orb,
        }
    }
}

/// Rastro como logica pura: guarda as ultimas posicoes, sem depender do Windows.
#[derive(Default)]
pub struct TrailBuffer {
    points: VecDeque<(i32, i32)>,
    capacity: usize,
}

impl TrailBuffer {
    pub fn new(capacity: u8) -> Self {
        Self {
            points: VecDeque::new(),
            capacity: capacity.max(1) as usize,
        }
    }

    pub fn push(&mut self, p: (i32, i32)) {
        if self.points.back() == Some(&p) {
            return;
        }
        self.points.push_back(p);
        while self.points.len() > self.capacity {
            self.points.pop_front();
        }
    }

    pub fn points(&self) -> Vec<(i32, i32)> {
        self.points.iter().copied().collect()
    }

    pub fn set_capacity(&mut self, capacity: u8) {
        self.capacity = capacity.max(1) as usize;
        while self.points.len() > self.capacity {
            self.points.pop_front();
        }
    }
}

pub struct Overlay {
    effects: Effects,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    #[allow(dead_code)]
    last_click: Option<Instant>,
}

impl Overlay {
    pub fn new(effects: Effects) -> Self {
        Self {
            effects,
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
            last_click: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.effects.enabled
    }

    pub fn effects(&self) -> Effects {
        self.effects
    }

    pub fn running(&self) -> bool {
        self.handle.is_some()
    }

    pub fn start(&mut self) -> Result<(), String> {
        if !self.effects.enabled {
            return Ok(());
        }
        if self.handle.is_some() {
            return Ok(());
        }
        self.stop.store(false, Ordering::SeqCst);
        let stop = Arc::clone(&self.stop);
        let fx = self.effects;
        self.handle = Some(
            std::thread::Builder::new()
                .name("cursor-fx-overlay".into())
                .spawn(move || run_overlay_loop(stop, fx))
                .map_err(|e| format!("spawn overlay: {e}"))?,
        );
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn apply_effects(effects: &Effects) -> Result<(), String> {
    if !effects.enabled {
        return Ok(());
    }
    let mut overlay = Overlay::new(Effects {
        enabled: true,
        trail_length: effects.trail_length,
        click_ring: effects.click_ring,
        shadow: effects.shadow,
        orb: effects.orb,
    });
    overlay.start()?;
    overlay.stop();
    Ok(())
}

/// Tamanho da janela do overlay (rastro precisa de area ao redor do cursor).
#[cfg(windows)]
const OVERLAY_SIZE: i32 = 192;

#[cfg(windows)]
fn run_overlay_loop(stop: Arc<AtomicBool>, fx: Effects) {
    use windows::core::w;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::*;

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    // Magenta como cor transparente (COLORKEY) para limpar o quadro.
    const KEY: COLORREF = COLORREF(0x00FF00FF);
    const TEAL: COLORREF = COLORREF(45 | (212 << 8) | (191 << 16));
    const GRAY: COLORREF = COLORREF(70 | (70 << 8) | (70 << 16));
    const YELLOW: COLORREF = COLORREF(255 | (213 << 8) | (0 << 16));
    const YELLOW_HI: COLORREF = COLORREF(255 | (235 << 8) | (100 << 16));

    unsafe {
        use windows::Win32::Foundation::HWND as RawHwnd;
        let null_hwnd = RawHwnd(std::ptr::null_mut());
        let hinstance: windows::Win32::Foundation::HINSTANCE =
            GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        let cls = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: w!("CursorOpenFx"),
            ..Default::default()
        };
        // A classe vive por processo: no segundo start ela ja existe
        // (toda troca de config recria o overlay). So aborta em erro real.
        if RegisterClassExW(&cls) == 0 {
            let err = windows::Win32::Foundation::GetLastError().0;
            const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
            if err != ERROR_CLASS_ALREADY_EXISTS {
                return;
            }
        }
        let hwnd = match CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("CursorOpenFx"),
            w!("CursorOpenFx"),
            WS_POPUP,
            0,
            0,
            OVERLAY_SIZE,
            OVERLAY_SIZE,
            Some(null_hwnd),
            Some(windows::Win32::UI::WindowsAndMessaging::HMENU(std::ptr::null_mut())),
            Some(hinstance),
            None,
        ) {
            Ok(h) => h,
            Err(_) => return,
        };
        // Bola amarela pede mais transparencia (como na foto: ~50%).
        let alpha: u8 = if fx.orb { 150 } else { 220 };
        let _ = SetLayeredWindowAttributes(
            hwnd,
            KEY,
            alpha,
            LWA_ALPHA | LWA_COLORKEY,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let half = OVERLAY_SIZE / 2;
        let mut trail = TrailBuffer::new(fx.trail_length);
        let mut rings: Vec<f32> = Vec::new();
        let mut was_down = false;
        let mut last = Instant::now();
        while !stop.load(Ordering::SeqCst) {
            let dt = last.elapsed().as_secs_f32().min(0.1);
            last = Instant::now();
            let mut pt = windows::Win32::Foundation::POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt).is_ok() {
                if fx.trail_length > 0 {
                    trail.push((pt.x, pt.y));
                }
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    pt.x - half,
                    pt.y - half,
                    OVERLAY_SIZE,
                    OVERLAY_SIZE,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                if fx.click_ring {
                    let down = GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
                    if down && !was_down && rings.len() < 4 {
                        rings.push(0.0);
                    }
                    was_down = down;
                }
                let hdc = GetDC(Some(hwnd));
                if !hdc.is_invalid() {
                    // Limpa com a cor transparente.
                    let bg = CreateSolidBrush(KEY);
                    let rc = RECT { left: 0, top: 0, right: OVERLAY_SIZE, bottom: OVERLAY_SIZE };
                    FillRect(hdc, &rc, bg);
                    let _ = DeleteObject(bg.into());
                    // Bola amarela com a seta bem no meio: o ponto do
                    // Windows e a ponta da seta, entao desloca a bola para
                    // o centro visual da seta (corpo fica centralizado).
                    if fx.orb {
                        let cx = half + 9;
                        let cy = half + 13;
                        let brush = CreateSolidBrush(YELLOW);
                        let pen = CreatePen(PS_NULL, 0, COLORREF(0));
                        let _ = SelectObject(hdc, brush.into());
                        let _ = SelectObject(hdc, pen.into());
                        let r = 34;
                        let _ = Ellipse(hdc, cx - r, cy - r, cx + r, cy + r);
                        let _ = DeleteObject(brush.into());
                        let _ = DeleteObject(pen.into());
                        // Brilho interno para volume de bola.
                        let brush2 = CreateSolidBrush(YELLOW_HI);
                        let pen2 = CreatePen(PS_NULL, 0, COLORREF(0));
                        let _ = SelectObject(hdc, brush2.into());
                        let _ = SelectObject(hdc, pen2.into());
                        let r2 = 22;
                        let _ = Ellipse(
                            hdc,
                            cx - r2,
                            cy - r2 - 4,
                            cx + r2,
                            cy + r2 - 4,
                        );
                        let _ = DeleteObject(brush2.into());
                        let _ = DeleteObject(pen2.into());
                    }
                    // Sombra atras do cursor.
                    if fx.shadow {
                        let brush = CreateSolidBrush(GRAY);
                        let pen = CreatePen(PS_NULL, 0, COLORREF(0));
                        let _ = SelectObject(hdc, brush.into());
                        let _ = SelectObject(hdc, pen.into());
                        let _ = Ellipse(hdc, half - 8 + 4, half - 8 + 6, half + 8 + 4, half + 8 + 6);
                        let _ = DeleteObject(brush.into());
                        let _ = DeleteObject(pen.into());
                    }
                    // Rastro: circulos que encolhem.
                    let pts = trail.points();
                    let n = pts.len().max(1);
                    for (i, (x, y)) in pts.iter().enumerate() {
                        let r = 3 + 8 * (i + 1) / n;
                        let brush = CreateSolidBrush(TEAL);
                        let pen = CreatePen(PS_NULL, 0, COLORREF(0));
                        let _ = SelectObject(hdc, brush.into());
                        let _ = SelectObject(hdc, pen.into());
                        let rx = x - (pt.x - half);
                        let ry = y - (pt.y - half);
                        let _ = Ellipse(hdc, rx - r as i32, ry - r as i32, rx + r as i32, ry + r as i32);
                        let _ = DeleteObject(brush.into());
                        let _ = DeleteObject(pen.into());
                    }
                    // Aneis de clique em expansao.
                    if fx.click_ring {
                        let pen = CreatePen(PS_SOLID, 3, TEAL);
                        let hollow = GetStockObject(HOLLOW_BRUSH);
                        let _ = SelectObject(hdc, pen.into());
                        let _ = SelectObject(hdc, hollow);
                        for t in &rings {
                            let r = (14.0 + *t * 130.0) as i32;
                            let _ = Ellipse(hdc, half - r, half - r, half + r, half + r);
                        }
                        let _ = DeleteObject(pen.into());
                    }
                    ReleaseDC(Some(hwnd), hdc);
                }
                for t in rings.iter_mut() {
                    *t += dt;
                }
                rings.retain(|t| *t < 0.45);
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        let _ = DestroyWindow(hwnd);
    }
}

#[cfg(not(windows))]
fn run_overlay_loop(stop: Arc<AtomicBool>, _fx: Effects) {
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_is_noop() {
        let fx = Effects {
            enabled: false,
            trail_length: 0,
            click_ring: false,
            shadow: false,
            orb: false,
        };
        assert!(apply_effects(&fx).is_ok());
    }

    /// Regressao: desligar e ligar de novo precisa recriar a janela
    /// no MESMO processo (a classe ja esta registrada; abortar ali
    /// deixava o efeito morto apos qualquer troca de config).
    #[cfg(windows)]
    #[test]
    fn restart_after_stop_recreates_window() {
        use std::time::{Duration, Instant};
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

        fn overlay_visible() -> bool {
            unsafe { FindWindowW(w!("CursorOpenFx"), w!("CursorOpenFx")).is_ok() }
        }
        fn wait_for(visible: bool) -> bool {
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                if overlay_visible() == visible {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            overlay_visible() == visible
        }

        // A janela e global (classe + titulo): se o app estiver aberto,
        // o overlay dele aparece aqui e o teste nao consegue provar nada.
        if overlay_visible() {
            eprintln!("SKIP: app aberto (overlay dele visivel); feche o app e rode de novo");
            return;
        }
        let fx = Effects {
            enabled: true,
            trail_length: 0,
            click_ring: false,
            shadow: false,
            orb: true,
        };
        let mut first = Overlay::new(fx);
        first.start().expect("primeiro start");
        assert!(wait_for(true), "overlay nao apareceu no primeiro start");
        first.stop();
        assert!(wait_for(false), "overlay nao sumiu apos stop");
        let mut second = Overlay::new(fx);
        second.start().expect("segundo start");
        assert!(
            wait_for(true),
            "overlay nao voltou no segundo start (classe ja existe?)"
        );
        second.stop();
        assert!(wait_for(false), "overlay nao sumiu no fim");
    }

    #[test]
    fn trail_keeps_capacity_and_dedups() {
        let mut t = TrailBuffer::new(3);
        t.push((0, 0));
        t.push((0, 0));
        t.push((1, 1));
        t.push((2, 2));
        t.push((3, 3));
        assert_eq!(t.points(), vec![(1, 1), (2, 2), (3, 3)]);
    }
}
