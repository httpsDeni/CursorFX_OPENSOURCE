//! Sons de clique do cursor.
//! Implementacao original: gera um bipe WAV em memoria e toca via PlaySoundW.

use cursor_core::CursorConfig;
use std::sync::Arc;

pub const RATE: u32 = 22050;

/// Monta um WAV 16-bit PCM com os canais/taxa dados (originais preservados).
fn samples_to_wav(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let channels = channels.max(1);
    let rate = rate.max(1);
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = rate * channels as u32 * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&(channels * 2).to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

#[cfg(test)]
fn click_like_wav_for_test(volume: f32) -> Vec<u8> {
    samples_to_wav(&tone(880.0, 880.0, 50, volume, 0.0), 1, RATE)
}

/// Tom com varredura de frequencia, ataque rapido e decaimento exponencial.
fn tone(f0: f32, f1: f32, ms: u32, volume: f32, noise_mix: f32) -> Vec<i16> {
    let n = ((RATE * ms / 1000).max(1)) as usize;
    let amp = volume.clamp(0.0, 1.0) * i16::MAX as f32;
    let mut lcg = 0x12345678u32;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let f = f0 + (f1 - f0) * (i as f32 / n as f32);
            let env = (-4.0 * i as f32 / n as f32).exp() * (i as f32 / (0.005 * RATE as f32)).min(1.0);
            lcg = lcg.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = ((lcg >> 16) as f32 / 32768.0 - 1.0) * noise_mix;
            let s = (2.0 * std::f32::consts::PI * f * t).sin() * (1.0 - noise_mix) + noise;
            (amp * env * s) as i16
        })
        .collect()
}

/// Se o WAV estiver no nosso formato (22050 Hz, mono, 16 bits),
/// devolve os bytes com o volume aplicado para tocar da memoria.
/// Qualquer outro formato retorna None (tocar via arquivo).
/// Aplica o volume escalando as amostras 16-bit, seja qual for a
/// taxa/canais (cabecalho original intacto: o tom nao muda).
/// Qualquer outro formato retorna None (tocar via arquivo).
fn scale_wav_file(data: &[u8], volume: f32) -> Option<Vec<u8>> {
    if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    // Localiza os blocos 'fmt ' e 'data' (o data pode nao estar no byte 44).
    let mut fmt: Option<(u16, u16)> = None; // (canais, bits)
    let mut data_off = 0usize;
    let mut data_len = 0usize;
    let mut off = 12usize;
    while off + 8 <= data.len() {
        let id = &data[off..off + 4];
        let size = u32::from_le_bytes([data[off + 4], data[off + 5], data[off + 6], data[off + 7]]) as usize;
        let body = off + 8;
        if id == b"fmt " && size >= 16 && body + 16 <= data.len() {
            let audio = u16::from_le_bytes([data[body], data[body + 1]]);
            let channels = u16::from_le_bytes([data[body + 2], data[body + 3]]);
            let bits = u16::from_le_bytes([data[body + 14], data[body + 15]]);
            if audio == 1 {
                fmt = Some((channels, bits));
            }
        } else if id == b"data" && body + size <= data.len() {
            data_off = body;
            data_len = size;
        }
        off = body + size + (size % 2);
    }
    let (channels, bits) = fmt?;
    if channels == 0 || bits != 16 || data_len == 0 || data_len % 2 != 0 {
        return None;
    }
    if data_off + data_len > data.len() {
        return None;
    }
    let mut out = data.to_vec();
    let v = volume.clamp(0.0, 1.0);
    for i in (0..data_len).step_by(2) {
        let s = i16::from_le_bytes([data[data_off + i], data[data_off + i + 1]]);
        let q = ((s as f32 * v) as i16).to_le_bytes();
        out[data_off + i] = q[0];
        out[data_off + i + 1] = q[1];
    }
    Some(out)
}

fn mix(a: &[i16], b: &[i16]) -> Vec<i16> {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| {
            let x = *a.get(i).unwrap_or(&0) as i32 + *b.get(i).unwrap_or(&0) as i32;
            x.clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

/// CLICK mecanico realista: estalo curto de banda larga + ressonancia.
/// Imita o "tec" do micro-switch: ataque instantaneo, 2 transientes.
fn mouse_click(volume: f32) -> Vec<i16> {
    let ms = 70u32;
    let n = ((RATE * ms / 1000).max(1)) as usize;
    let amp = volume.clamp(0.0, 1.0) * i16::MAX as f32;
    let mut lcg = 0x9E3779B9u32;
    let mut next_noise = || {
        lcg = lcg.wrapping_mul(1664525).wrapping_add(1013904223);
        ((lcg >> 16) as f32 / 32768.0) - 1.0
    };
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            // Transiente principal: ruido que apaga em ~3ms.
            let snap = next_noise() * (-t * 380.0).exp();
            // Corpo: ressonancia plastica ~2.8kHz + grave curto.
            let body = (2.0 * std::f32::consts::PI * 2800.0 * t).sin()
                * (-t * 210.0).exp()
                * 0.55
                + (2.0 * std::f32::consts::PI * 320.0 * t).sin()
                    * (-t * 160.0).exp()
                    * 0.30;
            // Segundo micro-estalo do switch (~14ms depois).
            let t2 = t - 0.014;
            let snap2 = if t2 > 0.0 {
                next_noise() * (-t2 * 460.0).exp() * 0.7
            } else {
                0.0
            };
            ((snap * 0.9 + body + snap2) * amp * 0.9) as i16
        })
        .collect()
}

fn click_wav(volume: f32) -> Vec<u8> {
    samples_to_wav(&mouse_click(volume), 1, RATE)
}

fn bank_sounds(volume: f32) -> Vec<(&'static str, Vec<i16>)> {
    vec![
        ("clique-mouse", mouse_click(volume)),
        ("clique-seco", tone(2000.0, 1200.0, 60, volume, 0.5)),
        ("pop", tone(700.0, 350.0, 90, volume, 0.0)),
        ("bipe", tone(880.0, 880.0, 90, volume, 0.0)),
        ("madeira", tone(180.0, 120.0, 120, volume, 0.25)),
        ("sino", mix(&tone(1318.0, 1318.0, 400, volume, 0.0), &tone(1976.0, 1976.0, 400, volume * 0.5, 0.0))),
        ("bolha", tone(300.0, 1000.0, 140, volume, 0.0)),
    ]
}

#[derive(Debug, Clone)]
pub struct BankEntry {
    pub name: String,
    pub path: String,
}

/// Decodifica um MP3 preservando taxa/canais originais (sem mudar o tom).
fn decode_mp3_to_bank(src: &std::path::Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(src).map_err(|_| "nao consegui ler o arquivo".to_string())?;
    let mut dec = minimp3::Decoder::new(file);
    let mut pcm = Vec::new();
    let (mut channels, mut rate) = (1u16, RATE);
    loop {
        match dec.next_frame() {
            Ok(frame) => {
                if pcm.is_empty() {
                    channels = frame.channels.max(1) as u16;
                    rate = frame.sample_rate.max(1) as u32;
                }
                pcm.extend_from_slice(&frame.data);
                if pcm.len() / channels.max(1) as usize > RATE as usize * 15 {
                    break;
                }
            }
            Err(minimp3::Error::Eof) => break,
            Err(e) => return Err(format!("mp3 invalido: {e:?}")),
        }
    }
    if pcm.is_empty() {
        return Err("mp3 vazio ou sem audio".to_string());
    }
    Ok(samples_to_wav(&pcm, channels, rate))
}

/// Importa um som de fora (.wav ou .mp3) para dentro do banco.
/// MP3 e convertido para o formato do banco; .wav valido e copiado.
/// Sem isso o som escolhido some da lista ao reiniciar o app.
pub fn import_sound_to_bank(src: &std::path::Path, dir: &std::path::Path) -> Result<BankEntry, String> {
    let _ = std::fs::create_dir_all(dir).map_err(|e| format!("pasta de sons: {e}"))?;
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    let data = match ext.as_str() {
        "wav" => {
            let d = std::fs::read(src).map_err(|_| "nao consegui ler o arquivo".to_string())?;
            if d.len() < 44 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
                return Err("arquivo nao parece um .wav valido".to_string());
            }
            d
        }
        "mp3" => decode_mp3_to_bank(src)?,
        _ => return Err("escolha um arquivo .wav ou .mp3".to_string()),
    };
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("som")
        .to_string();
    let mut dest = dir.join(format!("{stem}.wav"));
    if let Ok(existing) = std::fs::read(&dest) {
        if existing == data {
            return Ok(BankEntry {
                name: stem,
                path: dest.to_string_lossy().to_string(),
            });
        }
    }
    let mut n = 2;
    while dest.exists() {
        dest = dir.join(format!("{stem} {n}.wav"));
        n += 1;
        if n > 100 {
            return Err("muitos arquivos com esse nome".to_string());
        }
    }
    std::fs::write(&dest, &data).map_err(|e| format!("copiar som: {e}"))?;
    let name = dest
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&stem)
        .to_string();
    Ok(BankEntry {
        name,
        path: dest.to_string_lossy().to_string(),
    })
}

/// Pasta padrao do banco de sons (%APPDATA%/cursor-open/sounds).
pub fn default_bank_dir() -> std::path::PathBuf {
    std::env::var("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("cursor-open")
        .join("sounds")
}

/// Garante o banco de sons proprios na pasta (gera o que faltar) e lista.
/// Tambem inclui .wav extras (importados pelo usuario), senao eles
/// somem da lista ao reiniciar o app.
pub fn ensure_bank(dir: &std::path::Path) -> Vec<BankEntry> {
    let _ = std::fs::create_dir_all(dir);
    let mut out = Vec::new();
    for (name, samples) in bank_sounds(0.8) {
        let path = dir.join(format!("{name}.wav"));
        if !path.is_file() {
            let _ = std::fs::write(&path, samples_to_wav(&samples, 1, RATE));
        }
        out.push(BankEntry {
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
        });
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            let is_wav = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("wav"))
                .unwrap_or(false);
            if !is_wav || !p.is_file() {
                continue;
            }
            let ps = p.to_string_lossy().to_string();
            if out.iter().any(|b| b.path == ps) {
                continue;
            }
            let name = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("som")
                .to_string();
            out.push(BankEntry { name, path: ps });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn play_click(cfg: &CursorConfig) -> Result<(), String> {
    if !cfg.sound_enabled {
        return Ok(());
    }
    if !(0.0..=1.0).contains(&cfg.sound_volume) {
        return Err("volume fora de 0.0..=1.0".to_string());
    }
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_MEMORY};
        if cfg.sound_file.trim().is_empty() {
            let wav = click_wav(cfg.sound_volume);
            let ok = unsafe {
                PlaySoundW(
                    windows::core::PCWSTR(wav.as_ptr() as *const u16),
                    None,
                    SND_ASYNC | SND_MEMORY,
                )
            };
            return ok
                .as_bool()
                .then_some(())
                .ok_or_else(|| "Windows recusou o audio".to_string());
        }
        let full = cursor_core::expand_env(&cfg.sound_file);
        let data = std::fs::read(&full).map_err(|_| {
            format!("arquivo de som nao encontrado: {}", cfg.sound_file)
        })?;
        // Arquivos no nosso formato tocam da memoria com o volume aplicado.
        let (bytes, from_memory) = match scale_wav_file(&data, cfg.sound_volume) {
            Some(wav) => (wav, true),
            None => (data, false),
        };
        let ok = unsafe {
            if from_memory {
                PlaySoundW(
                    windows::core::PCWSTR(bytes.as_ptr() as *const u16),
                    None,
                    SND_ASYNC | SND_MEMORY,
                )
            } else {
                PlaySoundW(
                    &HSTRING::from(full),
                    None,
                    SND_ASYNC | SND_FILENAME,
                )
            }
        };
        ok.as_bool()
            .then_some(())
            .ok_or_else(|| "Windows recusou o audio".to_string())
    }
    #[cfg(not(windows))]
    {
        if !cfg.sound_file.trim().is_empty()
            && !std::path::Path::new(&cfg.sound_file).is_file()
        {
            return Err(format!(
                "arquivo de som nao encontrado: {}",
                cfg.sound_file
            ));
        }
        Ok(())
    }
}

/// Toca o som configurado a cada clique esquerdo global enquanto ativo.
/// Polling com GetAsyncKeyState (mesma tecnica do overlay): barato e sem hook DLL.
/// O app mantem uma instancia viva; parar o app para o som (sem daemon separado).
pub struct SoundHook {
    cfg: CursorConfig,
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SoundHook {
    pub fn new(cfg: &CursorConfig) -> Self {
        Self {
            cfg: cfg.clone(),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            handle: None,
        }
    }

    pub fn config_matches(&self, cfg: &CursorConfig) -> bool {
        self.cfg.sound_enabled == cfg.sound_enabled
            && (self.cfg.sound_volume - cfg.sound_volume).abs() < f32::EPSILON
            && self.cfg.sound_file == cfg.sound_file
    }

    pub fn running(&self) -> bool {
        self.handle.is_some()
    }

    pub fn start(&mut self) -> Result<(), String> {
        if !self.cfg.sound_enabled {
            return Ok(());
        }
        if self.handle.is_some() {
            return Ok(());
        }
        self.stop
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let stop = Arc::clone(&self.stop);
        let cfg = self.cfg.clone();
        self.handle = Some(
            std::thread::Builder::new()
                .name("cursor-sound-hook".into())
                .spawn(move || run_click_loop(stop, cfg))
                .map_err(|e| format!("spawn sound hook: {e}"))?,
        );
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for SoundHook {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
fn run_click_loop(stop: Arc<std::sync::atomic::AtomicBool>, cfg: CursorConfig) {
    use std::time::Duration;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    let mut was_down = false;
    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        unsafe {
            let down = GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
            if down && !was_down {
                let _ = play_click(&cfg);
            }
            was_down = down;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(windows))]
fn run_click_loop(stop: Arc<std::sync::atomic::AtomicBool>, _cfg: CursorConfig) {
    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_ok() {
        let wav = click_like_wav_for_test(0.5);
        assert!(wav.starts_with(b"RIFF"));
        assert!(wav.len() > 44);
    }

    #[test]
    fn missing_file_errors() {
        let cfg = CursorConfig {
            sound_file: "C:\\nao-existe\\x.wav".into(),
            ..CursorConfig::default()
        };
        assert!(play_click(&cfg).is_err());
    }

    #[test]
    fn default_beep_ok() {
        assert!(play_click(&CursorConfig::default()).is_ok());
    }

    #[test]
    fn click_sounds_like_switch_not_beep() {
        // CLICK real: pico logo no inicio e apaga rapido (mecanico),
        // nada de tom continuo de bipe.
        let s = mouse_click(0.8);
        assert!(s.len() > 500, "curto demais");
        let peak0 = s[..100].iter().map(|v| v.abs() as i32).max().unwrap_or(0);
        let tail = s[s.len() - 100..]
            .iter()
            .map(|v| v.abs() as i32)
            .max()
            .unwrap_or(0);
        assert!(peak0 > 8000, "sem estalo inicial: {peak0}");
        assert!(tail < peak0 / 8, "nao apagou como clique: {tail} vs {peak0}");
    }

    #[test]
    fn bank_generates_six_valid_wavs() {
        let dir = std::env::temp_dir().join("cursor-open-bank-test");
        let _ = std::fs::remove_dir_all(&dir);
        let entries = ensure_bank(&dir);
        assert_eq!(entries.len(), 7);
        for e in &entries {
            let d = std::fs::read(&e.path).unwrap();
            assert!(d.starts_with(b"RIFF"), "sem cabecalho: {}", e.name);
            assert!(d.len() > 100, "curto demais: {}", e.name);
            let body = &d[44..];
            let peak = body
                .chunks_exact(2)
                .map(|c| (i16::from_le_bytes([c[0], c[1]]) as i32).abs())
                .max()
                .unwrap_or(0);
            assert!(peak > 1000, "som mudo: {}", e.name);
        }
        // Segunda chamada nao reescreve nem duplica.
        assert_eq!(ensure_bank(&dir).len(), 7);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_format_keeps_tone() {
        // Estereo 44100 Hz passa intacto: cabecalho declara 2ch/44100,
        // amostras bit a bit iguais (tom original preservado).
        let pcm: Vec<i16> = (0..4410).flat_map(|_| vec![1000i16, 3000i16]).collect();
        let wav = samples_to_wav(&pcm, 2, 44100);
        let ch = u16::from_le_bytes([wav[22], wav[23]]);
        let rate = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
        assert_eq!((ch, rate), (2, 44100));
        let back: Vec<i16> = wav[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(back, pcm);
        // Volume em arquivo estereofonico: cabecalho igual, amostras escaladas.
        let half = scale_wav_file(&wav, 0.5).unwrap();
        assert_eq!(&half[..44], &wav[..44]);
        let s: Vec<i16> = half[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(s, vec![500i16, 1500i16].repeat(4410));
    }

    #[test]
    fn import_copies_wav_into_bank_and_lists() {
        let dir = std::env::temp_dir().join("cursor-open-import-test");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("orig").join("meu-click.wav");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, samples_to_wav(&mouse_click(0.8), 1, RATE)).unwrap();

        let e1 = import_sound_to_bank(&src, &dir).expect("importa");
        assert_eq!(e1.name, "meu-click");
        assert!(std::path::Path::new(&e1.path).is_file());
        assert!(ensure_bank(&dir).iter().any(|e| e.path == e1.path));

        // Reimportar o mesmo arquivo nao duplica.
        let e2 = import_sound_to_bank(&src, &dir).expect("reimporta");
        assert_eq!(e1.path, e2.path);

        // Conteudo diferente com mesmo nome ganha sufixo.
        std::fs::write(&src, samples_to_wav(&tone(440.0, 440.0, 50, 0.8, 0.0), 1, RATE)).unwrap();
        let e3 = import_sound_to_bank(&src, &dir).expect("importa variante");
        assert_ne!(e1.path, e3.path);
        assert!(e3.name.starts_with("meu-click"));

        // Rejeita extensao errada e conteudo invalido.
        let txt = dir.join("nota.txt");
        std::fs::write(&txt, b"oi").unwrap();
        assert!(import_sound_to_bank(&txt, &dir).is_err());
        let fake = dir.join("falso.wav");
        std::fs::write(&fake, b"nao e wav de verdade...............").unwrap();
        assert!(import_sound_to_bank(&fake, &dir).is_err());
        // MP3 falso (lixo) e rejeitado, nao copiado.
        let badmp3 = dir.join("lixo.mp3");
        std::fs::write(&badmp3, b"isso nao e audio...................").unwrap();
        assert!(import_sound_to_bank(&badmp3, &dir).is_err());
        assert!(!dir.join("lixo.wav").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scaling_applies_volume() {
        let wav = samples_to_wav(&[1000, -2000, 3000], 1, RATE);
        let half = scale_wav_file(&wav, 0.5).unwrap();
        let s: Vec<i16> = half[44..]
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(s, vec![500, -1000, 1500]);
        let mute = scale_wav_file(&wav, 0.0).unwrap();
        assert!(mute[44..].iter().all(|&b| b == 0));
        assert!(scale_wav_file(b"RIFF....lixo", 1.0).is_none());
    }
}
